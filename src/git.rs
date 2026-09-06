use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fmt, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use crate::model::{Category, Change, Hunk, LineKind, Snapshot, SourceLine};

mod collector;
pub use collector::Collector;
pub(crate) use collector::WatchContext;

#[derive(Debug)]
pub enum GitError {
    Io(std::io::Error),
    Command(String),
    InvalidOutput(String),
    Access {
        operation: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Access { operation, source } => write!(f, "{operation}: {source}"),
            Self::Io(e) => write!(f, "Git I/O: {e}"),
            Self::Command(s) | Self::InvalidOutput(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for GitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Access { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}
impl GitError {
    fn access(
        operation: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Access {
            operation,
            source: Box::new(source),
        }
    }
}
impl From<std::io::Error> for GitError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Repository paths used for Git commands and filesystem watching.
#[derive(Debug, Clone)]
pub struct Repository {
    /// Absolute path to the working tree root.
    pub root: PathBuf,
    /// Absolute path to this working tree's Git metadata.
    pub git_dir: PathBuf,
    /// Absolute path to Git metadata shared across worktrees.
    pub common_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub enum Mode {
    Watch,
    Comparison {
        base: String,
        head: String,
        label: String,
    },
}

/// Size thresholds for automatically loading a file's diff.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum source size in bytes on either side of a change.
    pub bytes: u64,
    /// Maximum number of diff lines per file change.
    pub lines: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 2 * 1024 * 1024,
            lines: 20_000,
        }
    }
}

fn successful(output: Output) -> Result<Vec<u8>, GitError> {
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(GitError::Command(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

impl Repository {
    pub fn discover(cwd: &Path) -> Result<Self, GitError> {
        let repo = gix::discover(cwd).map_err(|e| GitError::access("Discover repository", e))?;
        let root = repo
            .workdir()
            .ok_or_else(|| GitError::InvalidOutput("A working tree is required".into()))?;
        Ok(Self {
            root: absolute(root)?,
            git_dir: absolute(repo.git_dir())?,
            common_dir: absolute(repo.common_dir())?,
        })
    }

    pub fn run<I, S>(&self, args: I) -> Result<Vec<u8>, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        successful(
            Command::new("git")
                .env("LC_ALL", "C")
                .args(["--no-optional-locks", "--no-pager", "-C"])
                .arg(&self.root)
                .args(args)
                .output()?,
        )
    }

    pub fn resolve(&self, revision: &str) -> Result<String, GitError> {
        Collector::new(self.clone())?.resolve(revision)
    }

    pub fn comparison(&self, base: &str, head: &str, review: bool) -> Result<Mode, GitError> {
        let collector = Collector::new(self.clone())?;
        let a = collector.resolve(base)?;
        let b = collector.resolve(head)?;
        let resolved_base = if review {
            collector.merge_base(&a, &b)?
        } else {
            a
        };
        Ok(Mode::Comparison {
            base: resolved_base,
            head: b,
            label: format!("{base} {} {head}", if review { "…" } else { "→" }),
        })
    }

    /// Collect once. Interactive callers should reuse a `Collector` instead.
    pub fn snapshot(
        &self,
        mode: &Mode,
        limits: Limits,
        loaded: &BTreeSet<PathBuf>,
    ) -> Result<Snapshot, GitError> {
        Collector::new(self.clone())?.snapshot(mode, limits, loaded)
    }
}

impl Collector {
    pub fn snapshot(
        &mut self,
        mode: &Mode,
        limits: Limits,
        loaded: &BTreeSet<PathBuf>,
    ) -> Result<Snapshot, GitError> {
        self.refresh_configuration(false)?;
        let mut result = Snapshot::default();
        match mode {
            Mode::Comparison { base, head, label } => {
                result.branch.clone_from(label);
                result.changes = self.tracked(
                    Category::Comparison,
                    &[base.as_str(), head.as_str()],
                    limits,
                    loaded,
                    &[],
                )?;
            }
            Mode::Watch => {
                let status = self.status()?;
                result.branch = status.branch;
                let conflicts = status.conflicts;
                for path in status.untracked {
                    result.changes.push(self.worktree_file(
                        path,
                        Category::Untracked,
                        limits,
                        loaded,
                    )?);
                }
                for path in &conflicts {
                    result.changes.push(self.worktree_file(
                        path.clone(),
                        Category::Unstaged,
                        limits,
                        loaded,
                    )?);
                }
                result.changes.extend(self.tracked(
                    Category::Unstaged,
                    &[],
                    limits,
                    loaded,
                    &conflicts,
                )?);
                result.changes.extend(self.tracked(
                    Category::Staged,
                    &["--cached"],
                    limits,
                    loaded,
                    &conflicts,
                )?);
            }
        }
        result.sort();
        Ok(result)
    }

    fn tracked(
        &self,
        category: Category,
        extra: &[&str],
        limits: Limits,
        loaded: &BTreeSet<PathBuf>,
        conflicts: &[PathBuf],
    ) -> Result<Vec<Change>, GitError> {
        let mut args: Vec<OsString> = [
            "-c",
            "diff.suppressBlankEmpty=false",
            "diff",
            "--raw",
            "-z",
            "--patch",
            "--full-index",
            "--no-abbrev",
            "--no-color",
            "--output-indicator-new=+",
            "--output-indicator-old=-",
            "--output-indicator-context= ",
            "--no-ext-diff",
            "--no-textconv",
            "--no-relative",
            "--find-renames",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--unified=3",
            "--submodule=short",
            "--ignore-submodules=none",
            "--diff-filter=ACDMRTB",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        args.extend(extra.iter().map(OsString::from));
        args.push("--".into());
        for path in conflicts {
            let mut exclude = OsString::from(":(top,literal,exclude)");
            exclude.push(path);
            args.push(exclude);
        }
        let output = self.paths.run(args)?;
        let records = parse_raw_patch(&output)?;
        records.into_iter().map(|record| {
            let (hunks, metadata) = parse_patch(&record.patch);
            let submodule = record.old_mode == "160000" || record.new_mode == "160000";
            let old_size = if submodule { 0 } else { self.object_size(&record.old_oid)? };
            let new_size = if submodule { 0 } else if category == Category::Unstaged { fs::symlink_metadata(self.paths.root.join(&record.path)).map(|m| m.len()).unwrap_or(0) } else { self.object_size(&record.new_oid)? };
            let oversized = !loaded.contains(&record.path) && (old_size.max(new_size) > limits.bytes || hunks.iter().map(|h| h.lines.len()).sum::<usize>() > limits.lines);
            let binary = record.patch.lines().take_while(|line| !line.starts_with("@@ ")).any(|line| line.starts_with("Binary files ") || line == "GIT binary patch");
            let text_allowed = !oversized && !binary && !submodule;
            let old_source = if text_allowed { self.object_text(&record.old_oid)? } else { None };
            let new_source = if text_allowed && category == Category::Unstaged {
                read_regular_text(&self.paths.root.join(&record.path))
            } else if text_allowed { self.object_text(&record.new_oid)? } else { None };
            let summary = if oversized { Some(format!("Oversized file ({} bytes, {} diff lines). Select this file to load its contents.", old_size.max(new_size), hunks.iter().map(|h| h.lines.len()).sum::<usize>())) }
                else if binary { Some("Binary file changed".into()) }
                else if submodule { Some(format!("Submodule change\n{}", record.patch.lines().filter(|l| l.contains("Subproject commit")).collect::<Vec<_>>().join("\n"))) }
                else if hunks.is_empty() { Some(if metadata.is_empty() { "File metadata changed".into() } else { metadata }) }
                else { None };
            Ok(Change { path: record.path, old_path: record.old_path, category, status: record.status, hunks: if text_allowed { hunks } else { Vec::new() }, summary, oversized, old_source, new_source })
        }).collect()
    }

    fn worktree_file(
        &self,
        path: PathBuf,
        category: Category,
        limits: Limits,
        loaded: &BTreeSet<PathBuf>,
    ) -> Result<Change, GitError> {
        let full = self.paths.root.join(&path);
        let conflict = category == Category::Unstaged;
        let mut change = Change {
            path: path.clone(),
            old_path: None,
            category,
            status: if conflict { 'U' } else { '?' },
            hunks: Vec::new(),
            summary: None,
            oversized: false,
            old_source: None,
            new_source: None,
        };
        let metadata = match fs::symlink_metadata(&full) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                change.summary = Some(if conflict {
                    "Unmerged conflict — no working-tree file. Resolve the deletion with Git."
                        .into()
                } else {
                    "File disappeared during collection; awaiting refresh.".into()
                });
                return Ok(change);
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.is_symlink() {
            change.summary = Some(format!(
                "Symbolic link → {}",
                fs::read_link(&full)?.to_string_lossy()
            ));
            return Ok(change);
        }
        if !metadata.is_file() {
            change.summary = Some("Special file (contents not read)".into());
            return Ok(change);
        }
        if !loaded.contains(&path) && metadata.len() > limits.bytes {
            change.oversized = true;
            change.summary = Some(format!(
                "Oversized file ({} bytes). Select this file to load its contents.",
                metadata.len()
            ));
            return Ok(change);
        }
        let bytes = fs::read(&full)?;
        if bytes.contains(&0) {
            change.summary = Some("Binary file".into());
            return Ok(change);
        }
        let Ok(source) = String::from_utf8(bytes) else {
            change.summary = Some("Non-UTF-8 file (text preview unavailable)".into());
            return Ok(change);
        };
        let line_count = source.split_terminator('\n').count();
        if !loaded.contains(&path) && line_count > limits.lines {
            change.oversized = true;
            change.summary = Some("Oversized diff. Select this file to load its contents.".into());
            return Ok(change);
        }
        let lines = source
            .split_terminator('\n')
            .enumerate()
            .map(|(i, text)| SourceLine {
                kind: if conflict {
                    LineKind::Conflict
                } else {
                    LineKind::Added
                },
                old: None,
                new: Some(i + 1),
                text: text.into(),
                no_newline: !source.ends_with('\n') && i + 1 == line_count,
            })
            .collect();
        change.new_source = Some(Arc::from(source));
        change.hunks.push(Hunk {
            header: if conflict {
                "Unmerged — working-tree conflict view".into()
            } else {
                "New untracked file".into()
            },
            lines,
        });
        Ok(change)
    }
}

fn read_regular_text(path: &Path) -> Option<Arc<str>> {
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.is_file() {
        fs::read_to_string(path).ok().map(Arc::from)
    } else if meta.is_symlink() {
        fs::read_link(path)
            .ok()
            .map(|p| Arc::from(p.to_string_lossy().into_owned()))
    } else {
        None
    }
}
fn absolute(path: &Path) -> Result<PathBuf, GitError> {
    gix::path::realpath(path).map_err(|error| GitError::access("Resolve repository path", error))
}
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(bytes))
}
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).as_ref())
}

struct RawPatch {
    path: PathBuf,
    old_path: Option<PathBuf>,
    status: char,
    old_oid: String,
    new_oid: String,
    old_mode: String,
    new_mode: String,
    patch: String,
}

fn take_nul<'a>(input: &mut &'a [u8]) -> Result<&'a [u8], GitError> {
    let end = input
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| GitError::InvalidOutput("Truncated Git raw record".into()))?;
    let value = &input[..end];
    *input = &input[end + 1..];
    Ok(value)
}

fn parse_raw_patch(mut input: &[u8]) -> Result<Vec<RawPatch>, GitError> {
    let mut records = Vec::new();
    while input.first() == Some(&b':') {
        let header = String::from_utf8_lossy(take_nul(&mut input)?);
        let fields: Vec<_> = header.trim_start_matches(':').split(' ').collect();
        if fields.len() != 5 {
            return Err(GitError::InvalidOutput("Invalid Git raw header".into()));
        }
        let status = fields[4].chars().next().unwrap_or('M');
        let first_path = path_from_bytes(take_nul(&mut input)?);
        let (path, old_path) = if matches!(status, 'R' | 'C') {
            (path_from_bytes(take_nul(&mut input)?), Some(first_path))
        } else {
            (first_path, None)
        };
        records.push(RawPatch {
            path,
            old_path,
            status,
            old_oid: fields[2].into(),
            new_oid: fields[3].into(),
            old_mode: fields[0].into(),
            new_mode: fields[1].into(),
            patch: String::new(),
        });
    }
    let text = String::from_utf8_lossy(input);
    let mut patches = Vec::new();
    let mut current = String::new();
    for line in text.trim_start_matches(['\0', '\n']).split_terminator('\n') {
        if line.starts_with("diff --git ") && !current.is_empty() {
            patches.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        patches.push(current);
    }
    if records.len() != patches.len() {
        return Err(GitError::InvalidOutput(format!(
            "Git returned {} file records and {} patches",
            records.len(),
            patches.len()
        )));
    }
    for (record, patch) in records.iter_mut().zip(patches) {
        record.patch = patch;
    }
    Ok(records)
}

pub fn parse_patch(patch: &str) -> (Vec<Hunk>, String) {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut metadata = Vec::new();
    let (mut old, mut new) = (0, 0);
    for line in patch.split_terminator('\n') {
        if line.starts_with("@@ ") {
            let fields: Vec<_> = line.split_whitespace().collect();
            old = fields
                .get(1)
                .and_then(|s| s.trim_start_matches('-').split(',').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            new = fields
                .get(2)
                .and_then(|s| s.trim_start_matches('+').split(',').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            hunks.push(Hunk {
                header: line.into(),
                lines: Vec::new(),
            });
        } else if let Some(hunk) = hunks.last_mut() {
            if line == "\\ No newline at end of file" {
                if let Some(previous) = hunk.lines.last_mut() {
                    previous.no_newline = true;
                }
                continue;
            }
            let kind = match line.as_bytes().first() {
                Some(b' ') => LineKind::Context,
                Some(b'+') => LineKind::Added,
                Some(b'-') => LineKind::Removed,
                _ => continue,
            };
            let old_no = if kind != LineKind::Added {
                let n = old;
                old += 1;
                Some(n)
            } else {
                None
            };
            let new_no = if kind != LineKind::Removed {
                let n = new;
                new += 1;
                Some(n)
            } else {
                None
            };
            hunk.lines.push(SourceLine {
                kind,
                old: old_no,
                new: new_no,
                text: line[1..].into(),
                no_newline: false,
            });
        } else if !line.starts_with("diff --git ")
            && !line.starts_with("index ")
            && !line.starts_with("--- ")
            && !line.starts_with("+++ ")
        {
            metadata.push(line);
        }
    }
    (hunks, metadata.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_preserves_line_numbers_and_content_that_looks_like_a_header() {
        // Given
        let patch = "@@ -4,2 +4,2 @@ function\n same\n-before\n+diff --git literal text\n\\ No newline at end of file\n";
        // When
        let (hunks, _) = parse_patch(patch);
        // Then
        assert_eq!(hunks[0].lines.len(), 3);
        assert_eq!(hunks[0].lines[1].old, Some(5));
        assert_eq!(hunks[0].lines[2].new, Some(5));
        assert_eq!(hunks[0].lines[2].text, "diff --git literal text");
        assert!(hunks[0].lines[2].no_newline);
    }
}
