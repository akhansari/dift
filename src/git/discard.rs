use super::mutation::{IndexMutation, IndexObservation, MutationAttribution, MutationResult};
use super::{ChangeTarget, GitError, Repository, path_from_bytes};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// Preview results shared by discard preparation and its interactive menu.
pub(crate) type DiscardOption = Result<Arc<DiscardPreview>, String>;
pub(crate) const DISCARD_MODES: [DiscardMode; 2] = [DiscardMode::Unstaged, DiscardMode::All];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardMode {
    Unstaged,
    All,
}

impl DiscardMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unstaged => "Discard unstaged changes",
            Self::All => "Discard all changes",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardOutcome {
    Discarded,
    Unchanged,
    Stale,
}

/// A frozen, mode-specific review. Callers cannot alter its execution scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardPreview {
    target: ChangeTarget,
    mode: DiscardMode,
    head: Vec<u8>,
    index: BTreeMap<PathBuf, Vec<Vec<u8>>>,
    worktree: BTreeMap<PathBuf, Fingerprint>,
    restore: BTreeSet<PathBuf>,
    remove: BTreeSet<PathBuf>,
    deletions: BTreeSet<PathBuf>,
}

impl DiscardPreview {
    pub fn mode(&self) -> DiscardMode {
        self.mode
    }
    pub fn files(&self) -> usize {
        self.worktree.len()
    }
    pub fn deletions(&self) -> usize {
        self.deletions.len()
    }
    pub fn outside_paths(&self) -> impl Iterator<Item = &Path> {
        self.worktree
            .keys()
            .filter(|path| !self.target.contains(path))
            .map(PathBuf::as_path)
    }
}

/// Content is hashed in bounded memory, without invoking Git filters or following links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Fingerprint {
    pub(super) metadata: crate::model::WorktreeMetadata,
    content: Option<gix::hash::ObjectId>,
}

impl Fingerprint {
    pub(super) fn capture(path: &Path) -> Result<Self, GitError> {
        let metadata = super::source::activity_metadata(path)
            .ok_or_else(|| GitError::Command(format!("Cannot inspect {}", path.display())))?;
        let content = if matches!(
            metadata,
            crate::model::WorktreeMetadata::Present { regular: true, .. }
        ) {
            let mut file = fs::File::open(path)?;
            let mut hash = gix::hash::hasher(gix::hash::Kind::Sha256);
            let mut buffer = [0; 64 * 1024];
            loop {
                let len = file.read(&mut buffer)?;
                if len == 0 {
                    break;
                }
                hash.update(&buffer[..len]);
            }
            Some(
                hash.try_finalize()
                    .map_err(|error| GitError::access("Fingerprint file", error.into_error()))?,
            )
        } else {
            None
        };
        if super::source::activity_metadata(path).as_ref() != Some(&metadata) {
            return Err(GitError::Command(
                "Files changed while preparing discard; review again".into(),
            ));
        }
        Ok(Self { metadata, content })
    }
}

impl Repository {
    pub fn prepare_discard(
        &self,
        target: &ChangeTarget,
        mode: DiscardMode,
    ) -> Result<DiscardPreview, GitError> {
        let head = self.activity_head()?;
        let index = self.index_entries()?;
        let baseline = if mode == DiscardMode::All && !head.is_empty() {
            self.head_modes(&head)?
        } else {
            BTreeMap::new()
        };
        let status = self.run([
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
            "--renames",
        ])?;
        let mut records = status
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty());
        let mut paths = BTreeSet::new();
        while let Some(record) = records.next() {
            if record.len() < 4 || record[2] != b' ' {
                return Err(GitError::InvalidOutput("Invalid Git status record".into()));
            }
            let path = path_from_bytes(&record[3..]);
            let source = if matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C') {
                Some(path_from_bytes(records.next().ok_or_else(|| {
                    GitError::InvalidOutput("Missing rename source".into())
                })?))
            } else {
                None
            };
            if !target.contains(&path) && !source.as_ref().is_some_and(|path| target.contains(path))
            {
                continue;
            }
            if mode == DiscardMode::Unstaged && (record[1] == b' ' || record[0] == b'?') {
                continue;
            }
            paths.insert(path);
            if mode == DiscardMode::All
                && record[0] == b'R'
                && let Some(source) = source
            {
                paths.insert(source);
            }
        }
        let mut preview = DiscardPreview {
            target: target.clone(),
            mode,
            head,
            index: BTreeMap::new(),
            worktree: BTreeMap::new(),
            restore: BTreeSet::new(),
            remove: BTreeSet::new(),
            deletions: BTreeSet::new(),
        };
        for path in paths {
            self.check_discard_path(&path)?;
            let entries = index.get(&path).cloned().unwrap_or_default();
            if entries.iter().any(|entry| !entry.ends_with(b" 0")) {
                return Err(GitError::Command(format!(
                    "Unresolved conflict: {}",
                    path.display()
                )));
            }
            if entries.iter().any(|entry| entry.starts_with(b"160000 "))
                || baseline.get(&path).is_some_and(|mode| mode == b"160000")
            {
                return Err(GitError::Command(format!(
                    "Submodule changes are unsupported: {}",
                    path.display()
                )));
            }
            let fingerprint = Fingerprint::capture(&self.root.join(&path))?;
            let exists = !matches!(
                fingerprint.metadata,
                crate::model::WorktreeMetadata::Missing
            );
            let in_baseline = match mode {
                DiscardMode::Unstaged => !entries.is_empty(),
                DiscardMode::All => baseline.contains_key(&path),
            };
            if in_baseline || !entries.is_empty() {
                preview.restore.insert(path.clone());
            } else if mode == DiscardMode::All && exists {
                preview.remove.insert(path.clone());
            }
            if !in_baseline && exists {
                preview.deletions.insert(path.clone());
            }
            preview.index.insert(path.clone(), entries);
            preview.worktree.insert(path, fingerprint);
        }
        // Reject a preview collected across an index/HEAD transition.
        let current_index = self.index_entries()?;
        if preview.head != self.activity_head()?
            || preview.index.iter().any(|(path, entries)| {
                current_index
                    .get(path)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    != entries
            })
        {
            return Err(GitError::Command(
                "Git state changed while preparing discard; review again".into(),
            ));
        }
        Ok(preview)
    }

    pub fn discard(&self, preview: &DiscardPreview) -> Result<DiscardOutcome, GitError> {
        self.discard_tracked(preview).outcome
    }

    pub(crate) fn discard_tracked(
        &self,
        preview: &DiscardPreview,
    ) -> MutationResult<DiscardOutcome> {
        let result = self.prepare_discard(&preview.target, preview.mode);
        let current = match result {
            Ok(current) => current,
            // The target becoming unsupported also invalidates the review.
            Err(_) => {
                return MutationResult {
                    outcome: Ok(DiscardOutcome::Stale),
                    attribution: MutationAttribution::Uncertain,
                };
            }
        };
        if &current != preview {
            return MutationResult {
                outcome: Ok(DiscardOutcome::Stale),
                attribution: MutationAttribution::Uncertain,
            };
        }
        if preview.files() == 0 {
            return MutationResult {
                outcome: Ok(DiscardOutcome::Unchanged),
                attribution: MutationAttribution::Uncertain,
            };
        }
        let paths = preview.worktree.keys().cloned().collect();
        let before = IndexObservation::capture(self, &paths);
        let outcome = self
            .execute_discard(preview)
            .map(|()| DiscardOutcome::Discarded)
            .map_err(|error| GitError::Command(format!("Discard failed; may be partial: {error}")));
        let after = IndexObservation::capture(self, &paths);
        let attribution = match (before, after, outcome.is_ok()) {
            (Ok(before), Ok(after), true) => MutationAttribution::Confirmed(Box::new(
                IndexMutation::new(before, after, paths).restoring(self),
            )),
            _ => MutationAttribution::Uncertain,
        };
        MutationResult {
            outcome,
            attribution,
        }
    }

    fn execute_discard(&self, preview: &DiscardPreview) -> Result<(), GitError> {
        match preview.mode {
            DiscardMode::Unstaged => self.run_paths(
                &["restore", "--worktree", "--no-recurse-submodules"],
                &preview.restore,
            )?,
            DiscardMode::All if preview.head.is_empty() => {
                self.run_paths(&["rm", "-f", "--ignore-unmatch"], &preview.restore)?
            }
            DiscardMode::All => {
                let source = format!("--source={}", String::from_utf8_lossy(&preview.head).trim());
                self.run_paths(
                    &[
                        "restore",
                        &source,
                        "--staged",
                        "--worktree",
                        "--no-recurse-submodules",
                    ],
                    &preview.restore,
                )?;
            }
        }
        for path in &preview.remove {
            self.check_discard_path(path)?;
            // Restoring tracked files can run user-configured smudge filters.
            // Do not delete new content written by a filter or another process
            // since the full preflight, even if earlier restores succeeded.
            let current = Fingerprint::capture(&self.root.join(path))?;
            if preview.worktree.get(path) != Some(&current) {
                return Err(GitError::Command(format!(
                    "File changed before deletion: {}",
                    path.display()
                )));
            }
            fs::remove_file(self.root.join(path))
                .map_err(|error| GitError::file("Delete untracked file", path, error))?;
        }
        Ok(())
    }

    fn head_modes(&self, head: &[u8]) -> Result<BTreeMap<PathBuf, Vec<u8>>, GitError> {
        let output = self.run(["ls-tree", "-r", "-z", String::from_utf8_lossy(head).trim()])?;
        output
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .map(|record| {
                let tab = record
                    .iter()
                    .position(|byte| *byte == b'\t')
                    .ok_or_else(|| GitError::InvalidOutput("Invalid tree entry".into()))?;
                let mode = record
                    .split(|byte| *byte == b' ')
                    .next()
                    .unwrap_or_default();
                Ok((path_from_bytes(&record[tab + 1..]), mode.to_vec()))
            })
            .collect()
    }

    fn check_discard_path(&self, path: &Path) -> Result<(), GitError> {
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(GitError::Command(format!(
                "Unsupported discard path: {}",
                path.display()
            )));
        }
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| !directory.as_os_str().is_empty()) {
            let absolute = self.root.join(directory);
            if absolute.join(".git").exists()
                || fs::symlink_metadata(&absolute).is_ok_and(|metadata| metadata.is_symlink())
            {
                return Err(GitError::Command(format!(
                    "Nested repository or symlink parent: {}",
                    directory.display()
                )));
            }
            parent = directory.parent();
        }
        match fs::symlink_metadata(self.root.join(path)) {
            Ok(metadata) if !metadata.is_file() && !metadata.is_symlink() => {
                Err(GitError::Command(format!(
                    "Directory, submodule, or nested repository is unsupported: {}",
                    path.display()
                )))
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(GitError::file("Inspect discard path", path, error)),
        }
    }
}
