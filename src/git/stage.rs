use super::mutation::{IndexMutation, IndexObservation, MutationAttribution, MutationResult};
use super::{GitError, Repository, path_from_bytes, successful};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// A repository-relative whole file or recursive directory (empty for the root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeTarget {
    File(PathBuf),
    Subtree(PathBuf),
}
pub type StageTarget = ChangeTarget;
impl ChangeTarget {
    pub(crate) fn contains(&self, path: &Path) -> bool {
        match self {
            Self::File(file) => path == file,
            Self::Subtree(directory) => path.starts_with(directory),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageOutcome {
    Staged,
    Unstaged,
    Unchanged,
}

struct StageOperation {
    outcome: StageOutcome,
    paths: BTreeSet<PathBuf>,
    args: [&'static str; 2],
}

impl Repository {
    /// Choose from fresh index/worktree status, independent of displayed diffs.
    pub fn toggle_stage(&self, target: &StageTarget) -> Result<StageOutcome, GitError> {
        self.toggle_stage_tracked(target).outcome
    }

    pub(crate) fn toggle_stage_tracked(&self, target: &StageTarget) -> MutationResult {
        let operation = match self.prepare_stage(target) {
            Ok(Some(operation)) => operation,
            result => {
                return MutationResult {
                    outcome: result.map(|_| StageOutcome::Unchanged),
                    attribution: MutationAttribution::Uncertain,
                };
            }
        };
        // Evidence failures do not block the user's Git operation.
        let before = IndexObservation::capture(self, &operation.paths);
        let outcome = self.execute_stage(&operation).map(|()| operation.outcome);
        let after = IndexObservation::capture(self, &operation.paths);
        let attribution = match (before, after, outcome.is_ok()) {
            (Ok(before), Ok(after), true) => MutationAttribution::Confirmed(Box::new(
                IndexMutation::new(before, after, operation.paths),
            )),
            _ => MutationAttribution::Uncertain,
        };
        MutationResult {
            outcome,
            attribution,
        }
    }

    fn prepare_stage(&self, target: &StageTarget) -> Result<Option<StageOperation>, GitError> {
        let status = self.run([
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=dirty",
            "--renames",
        ])?;
        let mut records = status
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty());
        let mut unstaged = BTreeSet::new();
        let mut staged = BTreeSet::new();
        while let Some(record) = records.next() {
            if record.len() < 4 || record[2] != b' ' {
                return Err(GitError::InvalidOutput("Invalid Git status record".into()));
            }
            let path = path_from_bytes(&record[3..]);
            let renamed = matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C');
            let source = if renamed {
                Some(path_from_bytes(records.next().ok_or_else(|| {
                    GitError::InvalidOutput("Missing rename source".into())
                })?))
            } else {
                None
            };
            if !target.contains(&path)
                && !source
                    .as_ref()
                    .is_some_and(|source| target.contains(source))
            {
                continue;
            }
            if record[1] != b' ' && record[1] != b'!' {
                unstaged.insert(path.clone());
                // An index rename's source no longer exists in the index.
                if matches!(record[1], b'R' | b'C')
                    && let Some(source) = &source
                {
                    unstaged.insert(source.clone());
                }
            }
            if !matches!(record[0], b' ' | b'?' | b'!') {
                staged.insert(path);
                if record[0] == b'R'
                    && let Some(source) = source
                {
                    staged.insert(source);
                }
            }
        }
        let (outcome, paths, args) = if !unstaged.is_empty() {
            (StageOutcome::Staged, unstaged, ["add", "--all"])
        } else if !staged.is_empty() {
            (StageOutcome::Unstaged, staged, ["reset", "--quiet"])
        } else {
            return Ok(None);
        };
        Ok(Some(StageOperation {
            outcome,
            paths,
            args,
        }))
    }

    fn execute_stage(&self, operation: &StageOperation) -> Result<(), GitError> {
        self.run_paths(&operation.args, &operation.paths)
    }

    pub(super) fn run_paths(
        &self,
        args: &[&str],
        paths: &BTreeSet<PathBuf>,
    ) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut child = Command::new("git")
            .env("LC_ALL", "C")
            .args(["--literal-pathspecs", "--no-pager", "-C"])
            .arg(&self.root)
            .args(args)
            .args(["--pathspec-from-file=-", "--pathspec-file-nul"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let write_result = (|| -> std::io::Result<()> {
            let mut input = child
                .stdin
                .take()
                .ok_or_else(|| std::io::Error::other("Missing Git stdin"))?;
            for path in paths {
                input.write_all(path.as_os_str().as_encoded_bytes())?;
                input.write_all(&[0])?;
            }
            Ok(())
        })();
        // Reap even on a broken pipe, preferring Git's diagnostic.
        successful(child.wait_with_output()?)?;
        write_result?;
        Ok(())
    }
}
