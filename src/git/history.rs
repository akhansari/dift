use super::{GitError, Limits, Repository, path_from_bytes};
use crate::model::{Category, Change, Hunk, LineKind, SourceLine};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_email: String,
    pub authored: String,
    pub committer: String,
    pub committer_email: String,
    pub committed: String,
    pub relative_date: String,
    pub message: String,
    pub refs: Vec<String>,
}

impl CommitInfo {
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or("")
    }
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(12)]
    }
    pub fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        [&self.id, &self.message, &self.author, &self.author_email]
            .into_iter()
            .any(|value| value.to_lowercase().contains(&query))
    }
}

pub struct HistoryPage {
    pub commits: Vec<CommitInfo>,
    pub more: bool,
}

impl Repository {
    pub fn history_head(&self) -> Result<(Option<String>, String), GitError> {
        let branch = self
            .run(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .unwrap_or_else(|_| "Detached HEAD".into());
        match self.resolve("HEAD") {
            Ok(id) => Ok((Some(id), branch)),
            Err(error) if branch != "Detached HEAD" => {
                let reference = format!("refs/heads/{branch}");
                if self
                    .run(["show-ref", "--verify", "--quiet", &reference])
                    .is_err()
                {
                    Ok((None, branch))
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    pub fn history_page(
        &self,
        tip: &str,
        skip: usize,
        count: usize,
        query: &str,
    ) -> Result<HistoryPage, GitError> {
        let mut args: Vec<OsString> = vec![
            "log".into(),
            "--topo-order".into(),
            "--date-order".into(),
            "-z".into(),
            "--format=format:%H%x00%P%x00%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI%x00%cr%x00%B%x00"
                .into(),
        ];
        if query.is_empty() {
            args.push(format!("--skip={skip}").into());
            args.push(format!("--max-count={}", count.saturating_add(1)).into());
        }
        args.push(tip.into());
        args.push("--".into());
        let output = self.run(args)?;
        let refs = self.history_refs()?;
        let fields: Vec<_> = output.split(|byte| *byte == 0).collect();
        let mut commits = Vec::new();
        let mut position: usize = 0;
        while let Some(record) = fields.get(position..position.saturating_add(10)) {
            if let Some(commit) = parse_commit(record, &refs)
                && (query.is_empty() || commit.matches(query))
            {
                commits.push(commit);
            }
            position += 10;
            // The -z record separator follows the final field's %x00.
            if fields.get(position).is_some_and(|field| field.is_empty()) {
                position += 1;
            }
        }
        let more = query.is_empty() && commits.len() > count;
        if more {
            commits.truncate(count);
        }
        Ok(HistoryPage { commits, more })
    }

    fn history_refs(&self) -> Result<BTreeMap<String, Vec<String>>, GitError> {
        let output = self.run([
            "for-each-ref",
            "--format=%(objectname)%00%(refname)%00%(*objectname)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        let mut refs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for line in output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let fields: Vec<_> = line.split(|byte| *byte == 0).collect();
            let [id, name, peeled] = fields.as_slice() else {
                continue;
            };
            let id = if peeled.is_empty() { id } else { peeled };
            let name = String::from_utf8_lossy(name);
            let display = if let Some(branch) = name.strip_prefix("refs/heads/") {
                branch.to_owned()
            } else if let Some(remote) = name.strip_prefix("refs/remotes/") {
                format!("remote:{remote}")
            } else if let Some(tag) = name.strip_prefix("refs/tags/") {
                format!("tag:{tag}")
            } else {
                name.into_owned()
            };
            refs.entry(String::from_utf8_lossy(id).into_owned())
                .or_default()
                .push(display);
        }
        Ok(refs)
    }

    /// Resolve a path through the immutable commit tree rather than the worktree.
    pub fn historical_blob(&self, commit: &str, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let mut literal = OsString::from(":(literal)");
        literal.push(path);
        let entries = self.run([
            OsString::from("ls-tree"),
            "-z".into(),
            commit.into(),
            "--".into(),
            literal,
        ])?;
        let entry = entries
            .split(|byte| *byte == 0)
            .filter_map(|entry| entry.split_once_byte(b'\t'))
            .find(|(_, name)| path_from_bytes(name) == path);
        let Some((header, _)) = entry else {
            return Ok(None);
        };
        let parts: Vec<_> = header.split(|b| *b == b' ').collect();
        let [_, kind, id] = parts.as_slice() else {
            return Err(GitError::InvalidOutput("Invalid tree entry".into()));
        };
        if *kind != b"blob" {
            return Ok(None);
        }
        self.run(["cat-file", "blob", String::from_utf8_lossy(id).as_ref()])
            .map(Some)
    }
}

fn parse_commit(fields: &[&[u8]], refs: &BTreeMap<String, Vec<String>>) -> Option<CommitInfo> {
    let [
        id,
        parents,
        author,
        author_email,
        authored,
        committer,
        committer_email,
        committed,
        relative_date,
        message,
    ] = fields
    else {
        return None;
    };
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let id = text(id);
    Some(CommitInfo {
        refs: refs.get(&id).cloned().unwrap_or_default(),
        id,
        parents: text(parents)
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
        author: text(author),
        author_email: text(author_email),
        authored: text(authored),
        committer: text(committer),
        committer_email: text(committer_email),
        committed: text(committed),
        relative_date: text(relative_date),
        message: text(message),
    })
}

trait SplitOnceByte {
    fn split_once_byte(&self, byte: u8) -> Option<(&[u8], &[u8])>;
}
impl SplitOnceByte for [u8] {
    fn split_once_byte(&self, byte: u8) -> Option<(&[u8], &[u8])> {
        let pos = self.iter().position(|b| *b == byte)?;
        Some((&self[..pos], &self[pos + 1..]))
    }
}

impl super::Collector {
    pub(super) fn commit_files(
        &self,
        commit: &str,
        selected: Option<&Path>,
        limits: Limits,
        loaded: &BTreeSet<PathBuf>,
    ) -> Result<Vec<Change>, GitError> {
        let entries = self
            .paths
            .run(["ls-tree", "-r", "-z", "--full-tree", commit])?;
        entries
            .split(|b| *b == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let (header, path) = entry
                    .split_once_byte(b'\t')
                    .ok_or_else(|| GitError::InvalidOutput("Invalid tree entry".into()))?;
                let parts: Vec<_> = header.split(|b| *b == b' ').collect();
                let [mode, kind, id] = parts.as_slice() else {
                    return Err(GitError::InvalidOutput("Invalid tree entry".into()));
                };
                let path = path_from_bytes(path);
                let explicit = loaded.contains(&path);
                let selected = selected == Some(path.as_path()) || explicit;
                let id = std::str::from_utf8(id)
                    .map_err(|_| GitError::InvalidOutput("Invalid blob ID".into()))?;
                let mut change = Change {
                    path,
                    old_path: None,
                    category: Category::Comparison,
                    status: ' ',
                    hunks: Vec::new(),
                    summary: None,
                    oversized: false,
                    old_source: None,
                    new_source: None,
                };
                if *kind == b"commit" {
                    change.summary = Some(format!("Submodule at {id}"));
                } else if *mode == b"120000" {
                    let target = self.object_text(id)?;
                    change.summary = Some(format!(
                        "Symbolic link → {}",
                        target.as_deref().unwrap_or("(non-UTF-8 target)")
                    ));
                } else {
                    let bytes = self.object_size(id)?;
                    if selected && (bytes <= limits.bytes || explicit) {
                        let text = self.object_text(id)?;
                        if let Some(text) = text.filter(|text| !text.contains('\0')) {
                            let count = text.split_terminator('\n').count();
                            if count <= limits.lines || explicit {
                                change.hunks.push(Hunk {
                                    header: "Contents at commit".into(),
                                    lines: text
                                        .split_terminator('\n')
                                        .enumerate()
                                        .map(|(i, line)| SourceLine {
                                            kind: LineKind::Context,
                                            old: None,
                                            new: Some(i + 1),
                                            text: line.into(),
                                            no_newline: !text.ends_with('\n') && i + 1 == count,
                                        })
                                        .collect(),
                                });
                                change.new_source = Some(text);
                            } else {
                                change.oversized = true;
                                change.summary =
                                    Some("Large file. Press Enter to load its contents.".into());
                            }
                        } else {
                            change.summary =
                                Some(format!("Binary or non-UTF-8 file ({bytes} bytes)"));
                        }
                    } else {
                        change.oversized = selected && bytes > limits.bytes;
                        change.summary = Some(if change.oversized {
                            format!("Large file ({bytes} bytes). Select and press Enter to load.")
                        } else {
                            "Select a file to read its contents.".into()
                        });
                    }
                }
                Ok(change)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppResult, git::Mode, ui::fixtures::Fixture};

    #[test]
    fn history_is_pinned_after_head_advances_and_pages_without_repeating_commits() -> AppResult<()>
    {
        // Given: a branch with two commits.
        let fixture = Fixture::new("")?;
        fixture.write("tracked.txt", "second\n")?;
        fixture.git(&["add", "tracked.txt"])?;
        fixture.git(&["commit", "-m", "second"])?;
        let repo = fixture.repo()?;
        let (tip, branch) = repo.history_head()?;
        let tip = tip.ok_or("Missing branch tip")?;
        // When: the branch advances before requesting successive pages from the fixed tip.
        fixture.write("tracked.txt", "third\n")?;
        fixture.git(&["add", "tracked.txt"])?;
        fixture.git(&["commit", "-m", "third"])?;
        let first = repo.history_page(&tip, 0, 1, "")?;
        let second = repo.history_page(&tip, 1, 1, "")?;
        // Then: neither page includes the new commit, and the pinned pages are disjoint.
        assert_eq!(branch, "main");
        assert_ne!(Some(tip), repo.history_head()?.0);
        assert_eq!(first.commits[0].subject(), "second");
        assert_eq!(second.commits[0].subject(), "initial");
        assert!(first.more);
        assert!(!second.more);
        Ok(())
    }

    #[test]
    fn search_finds_a_commit_beyond_the_first_page() -> AppResult<()> {
        // Given: an older commit with a matching message.
        let fixture = Fixture::new("")?;
        fixture.write("tracked.txt", "second\n")?;
        fixture.git(&["add", "tracked.txt"])?;
        fixture.git(&["commit", "-m", "second"])?;
        let repo = fixture.repo()?;
        let (Some(tip), _) = repo.history_head()? else {
            return Err("Missing tip".into());
        };
        // When: searching the whole pinned history.
        let result = repo.history_page(&tip, 0, 1, "initial")?;
        // Then: the older commit is found independently of pagination.
        assert_eq!(result.commits.len(), 1);
        assert_eq!(result.commits[0].subject(), "initial");
        Ok(())
    }

    #[test]
    fn root_commit_shows_added_files() -> AppResult<()> {
        // Given: a commit with no parent.
        let fixture = Fixture::new("")?;
        let repo = fixture.repo()?;
        let (Some(tip), _) = repo.history_head()? else {
            return Err("Missing tip".into());
        };
        // When: collecting its patch against the empty tree.
        let snapshot = repo.snapshot(
            &Mode::Commit {
                id: tip,
                parent: None,
            },
            Limits::default(),
            &BTreeSet::new(),
        )?;
        // Then: tracked files are additions with readable content.
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| change.path == Path::new("tracked.txt")
                    && change.status == 'A'
                    && change.new_source.as_deref() == Some("original\n"))
        );
        Ok(())
    }

    #[test]
    fn full_tree_reads_an_unchanged_file_from_the_commit() -> AppResult<()> {
        // Given: a commit whose source differs from the worktree.
        let fixture = Fixture::new("")?;
        let repo = fixture.repo()?;
        let (Some(tip), _) = repo.history_head()? else {
            return Err("Missing tip".into());
        };
        fixture.write("tracked.txt", "uncommitted\n")?;
        // When: selecting that path in the commit tree.
        let snapshot = repo.snapshot(
            &Mode::Files {
                id: tip.clone(),
                selected: Some("tracked.txt".into()),
            },
            Limits::default(),
            &BTreeSet::new(),
        )?;
        // Then: both the preview and editor blob contain historical bytes.
        assert_eq!(
            snapshot
                .changes
                .iter()
                .find(|change| change.path == Path::new("tracked.txt"))
                .and_then(|change| change.new_source.as_deref()),
            Some("original\n")
        );
        assert_eq!(
            repo.historical_blob(&tip, Path::new("tracked.txt"))?,
            Some(b"original\n".to_vec())
        );
        Ok(())
    }

    #[test]
    fn full_tree_contents_have_only_new_side_positions_and_no_diff_totals() -> AppResult<()> {
        // Given: a commit with a tracked text file.
        let fixture = Fixture::new("")?;
        let repo = fixture.repo()?;
        let (Some(tip), _) = repo.history_head()? else {
            return Err("Missing tip".into());
        };
        // When: collecting the full file at that commit.
        let snapshot = repo.snapshot(
            &Mode::Files {
                id: tip,
                selected: Some("tracked.txt".into()),
            },
            Limits::default(),
            &BTreeSet::new(),
        )?;
        // Then: its contents are not modeled or counted as a diff.
        let file = snapshot
            .changes
            .iter()
            .find(|change| change.path == Path::new("tracked.txt"))
            .ok_or("Missing tracked file")?;
        assert_eq!(snapshot.tracked_stats, None);
        assert_eq!(file.hunks[0].lines[0].old, None);
        assert_eq!(file.hunks[0].lines[0].new, Some(1));
        Ok(())
    }

    #[test]
    fn refs_label_annotated_tags_and_remote_tracking_branches_at_exact_commit() -> AppResult<()> {
        // Given: two reference kinds pointing to the branch tip.
        let fixture = Fixture::new("")?;
        fixture.git(&["tag", "-a", "v1", "-m", "release"])?;
        fixture.git(&["update-ref", "refs/remotes/origin/main", "HEAD"])?;
        let repo = fixture.repo()?;
        let (Some(tip), _) = repo.history_head()? else {
            return Err("Missing tip".into());
        };
        // When: listing commits with their exact references.
        let page = repo.history_page(&tip, 0, 200, "")?;
        // Then: both the tag's peeled commit and the remote-tracking branch appear.
        assert!(page.commits[0].refs.contains(&"main".to_owned()));
        assert!(
            page.commits[0]
                .refs
                .contains(&"remote:origin/main".to_owned())
        );
        assert!(page.commits[0].refs.contains(&"tag:v1".to_owned()));
        Ok(())
    }

    #[test]
    fn merge_changes_are_compared_with_the_first_parent() -> AppResult<()> {
        // Given: a merge with independent files changed on each parent.
        let fixture = Fixture::new("")?;
        fixture.git(&["checkout", "-b", "topic"])?;
        fixture.write("topic.txt", "topic\n")?;
        fixture.git(&["add", "topic.txt"])?;
        fixture.git(&["commit", "-m", "topic"])?;
        fixture.git(&["checkout", "main"])?;
        fixture.write("main.txt", "main\n")?;
        fixture.git(&["add", "main.txt"])?;
        fixture.git(&["commit", "-m", "main change"])?;
        fixture.git(&["merge", "--no-ff", "-m", "merge topic", "topic"])?;
        let repo = fixture.repo()?;
        let (Some(tip), _) = repo.history_head()? else {
            return Err("Missing tip".into());
        };
        let page = repo.history_page(&tip, 0, 1, "")?;
        let merge = &page.commits[0];
        // When: viewing its first-parent patch.
        let snapshot = repo.snapshot(
            &Mode::Commit {
                id: merge.id.clone(),
                parent: merge.parents.first().cloned(),
            },
            Limits::default(),
            &BTreeSet::new(),
        )?;
        // Then: only the topic-side file was introduced relative to main.
        assert_eq!(merge.parents.len(), 2);
        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(snapshot.changes[0].path, Path::new("topic.txt"));
        Ok(())
    }

    #[test]
    fn unborn_branch_has_an_empty_history() -> AppResult<()> {
        // Given: a repository with a named branch but no commits.
        let directory = tempfile::tempdir()?;
        let output = std::process::Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(directory.path())
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let repo = Repository::discover(directory.path())?;
        // When: resolving the current history tip.
        let (tip, branch) = repo.history_head()?;
        // Then: the branch is shown with no commits to list.
        assert_eq!(tip, None);
        assert_eq!(branch, "main");
        Ok(())
    }

    #[test]
    fn detached_head_uses_its_own_commit_history() -> AppResult<()> {
        // Given: an existing commit checked out without a branch.
        let fixture = Fixture::new("")?;
        fixture.git(&["checkout", "--detach"])?;
        let repo = fixture.repo()?;
        // When: resolving the history without a symbolic branch.
        let (tip, branch) = repo.history_head()?;
        // Then: its exact HEAD commit remains available.
        assert_eq!(tip, Some(repo.resolve("HEAD")?));
        assert_eq!(branch, "Detached HEAD");
        Ok(())
    }

    #[test]
    fn large_historical_source_waits_for_explicit_load() -> AppResult<()> {
        // Given: a file larger than the automatic preview limit.
        let fixture = Fixture::new("")?;
        let repo = fixture.repo()?;
        let (Some(tip), _) = repo.history_head()? else {
            return Err("Missing tip".into());
        };
        let mode = Mode::Files {
            id: tip,
            selected: Some("tracked.txt".into()),
        };
        let limits = Limits {
            bytes: 4,
            lines: 20,
        };
        // When: the same file is collected without and with explicit loading.
        let preview = repo.snapshot(&mode, limits, &BTreeSet::new())?;
        let full = repo.snapshot(&mode, limits, &BTreeSet::from(["tracked.txt".into()]))?;
        // Then: the large blob is not read until requested.
        assert!(
            preview
                .changes
                .iter()
                .any(|change| change.path == Path::new("tracked.txt")
                    && change.oversized
                    && change.new_source.is_none())
        );
        assert!(
            full.changes
                .iter()
                .any(|change| change.path == Path::new("tracked.txt")
                    && !change.oversized
                    && change.new_source.as_deref() == Some("original\n"))
        );
        Ok(())
    }
}
