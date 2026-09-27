use super::{GitError, Repository, path_from_bytes};
use crate::model::IndexStage;
use gix::bstr::ByteSlice;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

const OBJECT_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Git state owned by the collection thread. No repository or index writes.
pub struct Collector {
    pub(super) paths: Repository,
    repo: gix::Repository,
    config_files: BTreeMap<PathBuf, Option<u64>>,
    tracking: Option<Tracking>,
    pub(super) historical_tree: Option<super::history::HistoricalTree>,
}

#[derive(Default, Debug)]
struct Tracking {
    paths: BTreeSet<PathBuf>,
    submodules: BTreeSet<PathBuf>,
    ignore_case: bool,
    uncertain: bool,
}
impl Tracking {
    fn new(ignore_case: bool) -> Self {
        Self {
            ignore_case,
            ..Self::default()
        }
    }

    fn insert(&mut self, path: PathBuf, submodule: bool) {
        let path = if self.ignore_case {
            let Some(path) = path.to_str() else {
                self.uncertain = true;
                return;
            };
            PathBuf::from(path.to_lowercase())
        } else {
            path
        };
        if submodule {
            self.submodules.insert(path.clone());
        }
        self.paths.insert(path);
    }

    fn may_contain(&self, path: &Path) -> bool {
        if self.uncertain {
            return true;
        }
        let path = if self.ignore_case {
            let Some(path) = path.to_str() else {
                return true;
            };
            Cow::Owned(PathBuf::from(path.to_lowercase()))
        } else {
            Cow::Borrowed(path)
        };
        self.paths
            .range::<Path, _>((
                std::ops::Bound::Included(path.as_ref()),
                std::ops::Bound::Unbounded,
            ))
            .next()
            .is_some_and(|entry| entry.starts_with(&path))
            || path
                .ancestors()
                .any(|parent| self.submodules.contains(parent))
    }
}

pub(super) struct Status {
    pub branch: String,
    pub conflicts: Vec<PathBuf>,
    pub conflict_stages: BTreeMap<PathBuf, Vec<IndexStage>>,
    pub untracked: Vec<PathBuf>,
}

/// Resolved resources used by event filtering and external watches.
pub(crate) struct WatchContext {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub resources: BTreeSet<PathBuf>,
    /// Lowest to highest priority, below worktree .gitignore files.
    pub excludes: Vec<PathBuf>,
    pub ignore_case: bool,
}

impl WatchContext {
    pub fn resource_changed(&self, path: &Path) -> bool {
        self.resources
            .iter()
            .any(|resource| resource.starts_with(path))
    }
}

impl Collector {
    pub(crate) fn watch_context(&self) -> Result<WatchContext, GitError> {
        let config = self.repo.config_snapshot();
        let global = config
            .trusted_path("core.excludesFile")
            .map_err(|e| GitError::access("Resolve Git excludes file", e))?
            .or_else(|| git_config_home().map(|base| base.join("ignore")));
        let excludes: Vec<_> = global
            .into_iter()
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| self.paths.root.join(path))
            .chain([self.paths.common_dir.join("info/exclude")])
            .collect();
        let mut config_files: BTreeSet<_> = self.config_files.keys().cloned().collect();
        if let Some(attributes) = config
            .trusted_path("core.attributesFile")
            .map_err(|e| GitError::access("Resolve Git attributes file", e))?
        {
            config_files.insert(self.paths.root.join(attributes));
        }
        let resources = config_files
            .iter()
            .chain(&excludes)
            .cloned()
            .chain([self.repo.index_path()])
            .flat_map(|path| std::iter::once(path.clone()).chain(gix::path::realpath(&path).ok()))
            .collect();
        Ok(WatchContext {
            root: self.paths.root.clone(),
            git_dir: self.paths.git_dir.clone(),
            common_dir: self.paths.common_dir.clone(),
            resources,
            excludes,
            ignore_case: config.boolean("core.ignoreCase").unwrap_or(false),
        })
    }

    pub(crate) fn may_be_tracked(&self, path: &Path) -> bool {
        self.tracking
            .as_ref()
            .is_none_or(|tracking| tracking.may_contain(path))
    }

    pub fn new(paths: Repository) -> Result<Self, GitError> {
        let mut repo =
            gix::open(&paths.git_dir).map_err(|e| GitError::access("Open repository", e))?;
        repo.object_cache_size(OBJECT_CACHE_BYTES);
        let mut collector = Self {
            paths,
            repo,
            config_files: BTreeMap::new(),
            tracking: None,
            historical_tree: None,
        };
        collector.normalize_resource_paths()?;
        collector.remember_configuration();
        Ok(collector)
    }

    /// Reload on explicit invalidation as well as changes to resolved configuration sources.
    pub fn refresh_configuration(&mut self, force: bool) -> Result<(), GitError> {
        if force
            || self
                .config_files
                .iter()
                .any(|(path, before)| fingerprint(path) != *before)
        {
            self.repo
                .reload()
                .map_err(|e| GitError::access("Reload repository configuration", e))?;
            self.repo.object_cache_size(OBJECT_CACHE_BYTES);
            self.normalize_resource_paths()?;
            self.remember_configuration();
        }
        Ok(())
    }

    fn normalize_resource_paths(&mut self) -> Result<(), GitError> {
        // Git runs with -C at the worktree root; gix otherwise opens these relative to the process cwd.
        for key in ["core.excludesFile", "core.attributesFile"] {
            if let Some(path) = self
                .repo
                .config_snapshot()
                .trusted_path(key)
                .map_err(|e| GitError::access("Resolve Git resource path", e))?
                && path.is_relative()
                && !path.as_os_str().is_empty()
            {
                let path = gix::path::into_bstr(self.paths.root.join(path));
                let mut config = self.repo.config_snapshot_mut();
                config
                    .set_raw_value(key, path)
                    .map_err(|e| GitError::access("Normalize Git resource path", e.into_error()))?;
                config
                    .commit()
                    .map_err(|e| GitError::access("Apply in-memory Git configuration", e))?;
            }
        }
        Ok(())
    }

    fn remember_configuration(&mut self) {
        let config = self.repo.config_snapshot();
        let mut paths: BTreeSet<_> = config
            .plumbing()
            .sections()
            .filter_map(|section| section.meta().path.clone())
            .collect();
        paths.insert(self.paths.common_dir.join("config"));
        paths.insert(self.paths.git_dir.join("config.worktree"));
        if let Some(home) = std::env::home_dir() {
            paths.insert(home.join(".gitconfig"));
        }
        if let Some(base) = git_config_home() {
            paths.insert(base.join("config"));
        }
        self.config_files = paths
            .into_iter()
            .map(|path| {
                let stamp = fingerprint(&path);
                (path, stamp)
            })
            .collect();
    }

    pub(super) fn resolve(&self, revision: &str) -> Result<String, GitError> {
        self.repo
            .rev_parse_single(format!("{revision}^{{commit}}").as_bytes().as_bstr())
            .map(|id| id.to_string())
            .map_err(|e| GitError::access("Resolve commit", e))
    }

    pub(super) fn merge_base(&self, a: &str, b: &str) -> Result<String, GitError> {
        self.repo
            .merge_base(oid(a)?, oid(b)?)
            .map(|id| id.to_string())
            .map_err(|e| GitError::access("Find merge base", e))
    }

    pub(super) fn object_size(&self, id: &str) -> Result<u64, GitError> {
        let id = oid(id)?;
        if id.is_null() {
            return Ok(0);
        }
        self.repo
            .find_header(id)
            .map(|header| header.size())
            .map_err(|e| GitError::access("Read object size", e))
    }

    pub(super) fn object_text(&self, id: &str) -> Result<Option<Arc<str>>, GitError> {
        let id = oid(id)?;
        if id.is_null() {
            return Ok(Some(Arc::from("")));
        }
        let blob = self
            .repo
            .find_blob(id)
            .map_err(|e| GitError::access("Read blob", e))?;
        Ok(std::str::from_utf8(&blob.data).ok().map(Arc::from))
    }

    pub(super) fn status(&mut self) -> Result<Status, GitError> {
        // Decode afresh: an external tool can replace the index without advancing its mtime.
        let index = match self.repo.open_index() {
            Ok(index) if index.is_sparse() => return self.cli_status(),
            Ok(index) => index,
            Err(e)
                if e.downcast_any_ref::<gix::index::file::init::OpenError>()
                    .is_some_and(|open| {
                        open.path == self.repo.index_path()
                            && open.source.kind() == std::io::ErrorKind::NotFound
                    }) =>
            {
                gix::index::File::from_state(
                    gix::index::State::new(self.repo.object_hash()),
                    self.repo.index_path(),
                )
            }
            Err(e) if unsupported_index(&e) => return self.cli_status(),
            Err(e) => return Err(GitError::access("Read index", e)),
        };
        let mut tracking = Tracking::new(
            self.repo
                .config_snapshot()
                .boolean("core.ignoreCase")
                .unwrap_or(false),
        );
        let mut conflicts = BTreeSet::new();
        let mut conflict_stages: BTreeMap<PathBuf, Vec<IndexStage>> = BTreeMap::new();
        for entry in index.entries() {
            let path = path_from_bytes(entry.path(&index));
            if entry.stage() != gix::index::entry::Stage::Unconflicted {
                conflicts.insert(path.clone());
                conflict_stages
                    .entry(path.clone())
                    .or_default()
                    .push(IndexStage {
                        stage: entry.stage() as u32,
                        mode: entry.mode.bits(),
                        oid: entry.id.to_string(),
                    });
            }
            tracking.insert(path, entry.mode == gix::index::entry::Mode::COMMIT);
        }
        let branch = self
            .repo
            .head_name()
            .map_err(|e| GitError::access("Read HEAD", e))?
            .map(|name| name.shorten().to_str_lossy().into_owned())
            .unwrap_or_else(|| "(detached)".into());
        let status = self
            .repo
            .status(gix::progress::Discard)
            .map_err(|e| GitError::access("Read status", e))?
            .index(gix::worktree::IndexPersistedOrInMemory::InMemory(index))
            .untracked_files(gix::status::UntrackedFiles::Files)
            // Git diff is authoritative for tracked paths, including dirty submodules.
            .index_worktree_submodules(None)
            .into_index_worktree_iter(Vec::<gix::bstr::BString>::new())
            .map_err(|e| GitError::access("Collect worktree status", e))?;
        let mut untracked = Vec::new();
        for item in status {
            let item = item.map_err(|e| GitError::access("Collect worktree entry", e))?;
            if let gix::status::index_worktree::Item::DirectoryContents { entry, .. } = item
                && entry.status == gix::dir::entry::Status::Untracked
            {
                untracked.push(path_from_bytes(&entry.rela_path));
            }
        }
        self.tracking = Some(tracking);
        Ok(Status {
            branch,
            conflicts: conflicts.into_iter().collect(),
            conflict_stages,
            untracked,
        })
    }

    // Keep compatibility with index extensions gix cannot interpret. Never use this for I/O/corruption errors.
    fn cli_status(&mut self) -> Result<Status, GitError> {
        let status = self.paths.run([
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--no-ahead-behind",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ])?;
        let mut result = Status {
            branch: String::new(),
            conflicts: Vec::new(),
            conflict_stages: BTreeMap::new(),
            untracked: Vec::new(),
        };
        let mut records = status.split(|b| *b == 0);
        while let Some(record) = records.next() {
            if let Some(branch) = record.strip_prefix(b"# branch.head ") {
                result.branch = String::from_utf8_lossy(branch).into_owned();
            }
            if let Some(path) = record.strip_prefix(b"? ") {
                result.untracked.push(path_from_bytes(path));
            }
            if record.starts_with(b"u ")
                && let Some(path) = record.splitn(11, |b| *b == b' ').nth(10)
            {
                result.conflicts.push(path_from_bytes(path));
            }
            if record.starts_with(b"2 ") {
                records.next();
            }
        }
        let entries = self.paths.run(["ls-files", "--stage", "-z"])?;
        let mut tracking = Tracking::new(
            self.repo
                .config_snapshot()
                .boolean("core.ignoreCase")
                .unwrap_or(false),
        );
        for entry in entries.split(|b| *b == 0).filter(|entry| !entry.is_empty()) {
            let Some(tab) = entry.iter().position(|b| *b == b'\t') else {
                return Err(GitError::InvalidOutput("Invalid index listing".into()));
            };
            let path = path_from_bytes(&entry[tab + 1..]);
            let header = std::str::from_utf8(&entry[..tab])
                .map_err(|e| GitError::access("Read index entry", e))?;
            let fields: Vec<_> = header.split(' ').collect();
            let [mode, oid, stage] = fields.as_slice() else {
                return Err(GitError::InvalidOutput("Invalid index listing".into()));
            };
            let stage: u32 = stage
                .parse()
                .map_err(|e| GitError::access("Read index stage", e))?;
            if stage != 0 {
                result
                    .conflict_stages
                    .entry(path.clone())
                    .or_default()
                    .push(IndexStage {
                        stage,
                        mode: u32::from_str_radix(mode, 8)
                            .map_err(|e| GitError::access("Read index mode", e))?,
                        oid: (*oid).to_owned(),
                    });
            }
            tracking.insert(path, entry.starts_with(b"160000 "));
        }
        self.tracking = Some(tracking);
        Ok(result)
    }
}

fn oid(value: &str) -> Result<gix::ObjectId, GitError> {
    gix::ObjectId::from_hex(value.as_bytes())
        .map_err(|e| GitError::access("Parse object ID", e.into_error()))
}

fn unsupported_index(error: &gix::Error) -> bool {
    // gix 0.88 reports unknown mandatory extensions as messages rather than typed errors.
    error
        .downcast_any_ref::<gix::error::Message>()
        .is_some_and(|message| {
            message
                .message
                .starts_with("Encountered mandatory extension '")
                && message.message.ends_with("' which isn't implemented yet")
        })
}

fn fingerprint(path: &Path) -> Option<u64> {
    let bytes = fs::read(path).ok()?;
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    Some(hash.finish())
}

fn git_config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".config")))
        .map(|base| base.join("git"))
}
