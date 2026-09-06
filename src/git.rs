use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use crate::model::Snapshot;

mod changes;
mod collector;
mod patch;
mod source;
pub use collector::Collector;
pub(crate) use collector::WatchContext;
pub use patch::parse_patch;

#[derive(Debug)]
pub enum GitError {
    Io(std::io::Error),
    Command(String),
    InvalidOutput(String),
    Access {
        operation: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    File {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
}
impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Access { operation, source } => write!(f, "{operation}: {source}"),
            Self::File {
                operation,
                path,
                source,
            } => write!(f, "{operation} {}: {source}", path.display()),
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
            Self::File { source, .. } => Some(source),
            _ => None,
        }
    }
}
impl GitError {
    fn file(operation: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::File {
            operation,
            path: path.to_owned(),
            source,
        }
    }

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

    /// Collect with an explicit whitespace comparison policy.
    pub fn snapshot_with_whitespace(
        &self,
        mode: &Mode,
        limits: Limits,
        loaded: &BTreeSet<PathBuf>,
        whitespace: crate::config::Whitespace,
    ) -> Result<Snapshot, GitError> {
        Collector::new(self.clone())?.snapshot_with_whitespace(mode, limits, loaded, whitespace)
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
