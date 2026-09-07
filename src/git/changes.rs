use std::{collections::BTreeSet, ffi::OsString, path::PathBuf, sync::Arc};

use crate::config::Whitespace;
use crate::model::{Category, Change, Hunk, LineKind, LineStats, Snapshot, SourceLine};

use super::{
    Collector, GitError, Limits, Mode,
    patch::{RawPatch, parse_raw_patch},
    path_from_bytes,
    source::{self, Entry, TextSource},
};

struct TrackedChanges {
    changes: Vec<Change>,
    exact_paths: BTreeSet<PathBuf>,
    stats: LineStats,
}

impl Collector {
    pub fn snapshot(
        &mut self,
        mode: &Mode,
        limits: Limits,
        loaded: &BTreeSet<PathBuf>,
    ) -> Result<Snapshot, GitError> {
        self.snapshot_with_whitespace(mode, limits, loaded, Whitespace::Exact)
    }

    /// Collect Git patches and counts using the selected whitespace policy.
    pub fn snapshot_with_whitespace(
        &mut self,
        mode: &Mode,
        limits: Limits,
        loaded: &BTreeSet<PathBuf>,
        whitespace: Whitespace,
    ) -> Result<Snapshot, GitError> {
        self.refresh_configuration(false)?;
        let mut result = Snapshot {
            whitespace,
            ..Snapshot::default()
        };
        let mut exact_paths = BTreeSet::new();
        let mut stats = LineStats::default();
        match mode {
            Mode::Comparison { base, head, label } => {
                result.branch.clone_from(label);
                let tracked = self.tracked(
                    Category::Comparison,
                    &[base.as_str(), head.as_str()],
                    limits,
                    loaded,
                    &[],
                    whitespace,
                )?;
                result.changes = tracked.changes;
                exact_paths.extend(tracked.exact_paths);
                stats = tracked.stats;
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
                for (category, extra) in [
                    (Category::Unstaged, &[][..]),
                    (Category::Staged, &["--cached"][..]),
                ] {
                    let tracked =
                        self.tracked(category, extra, limits, loaded, &conflicts, whitespace)?;
                    result.changes.extend(tracked.changes);
                    exact_paths.extend(tracked.exact_paths);
                    stats = stats.checked_add(tracked.stats).ok_or_else(|| {
                        GitError::InvalidOutput("Git line statistics overflow".into())
                    })?;
                }
            }
        }
        for change in &result.changes {
            exact_paths.remove(&change.path);
        }
        result.whitespace_hidden_files = exact_paths.len();
        result.tracked_stats = Some(stats);
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
        whitespace: Whitespace,
    ) -> Result<TrackedChanges, GitError> {
        let exact_paths = if whitespace == Whitespace::Exact {
            BTreeSet::new()
        } else {
            self.paths
                .run(diff_args(extra, conflicts, Whitespace::Exact, true))?
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
                .map(path_from_bytes)
                .collect()
        };
        let output = self
            .paths
            .run(diff_args(extra, conflicts, whitespace, false))?;
        let (records, stats) = parse_raw_patch(&output)?;
        let changes = records
            .into_iter()
            .map(|record| {
                let explicitly_loaded = loaded.contains(&record.path);
                self.tracked_change(record, category, limits, explicitly_loaded)
            })
            .collect::<Result<_, _>>()?;
        Ok(TrackedChanges {
            changes,
            exact_paths,
            stats,
        })
    }

    fn tracked_change(
        &self,
        record: RawPatch,
        category: Category,
        limits: Limits,
        explicitly_loaded: bool,
    ) -> Result<Change, GitError> {
        let patch = record.parse();
        let submodule = record.is_submodule();
        let full = self.paths.root.join(&record.path);
        let worktree =
            (category == Category::Unstaged && !submodule).then(|| source::inspect(&full));
        let sizes = SourceSizes {
            old: if submodule {
                0
            } else {
                self.object_size(&record.old_oid)?
            },
            new: match &worktree {
                Some(Ok(entry)) => Some(entry.size()),
                // Unknown size must not be treated as an empty file.
                Some(Err(_)) => None,
                None if submodule => Some(0),
                None => Some(self.object_size(&record.new_oid)?),
            },
        };
        let oversized =
            !explicitly_loaded && (sizes.exceeds(limits.bytes) || patch.line_count > limits.lines);
        let text_allowed = !oversized && patch.kind.allows_text();
        let summary = if oversized {
            Some(sizes.oversized_summary(patch.line_count))
        } else {
            patch.kind.into_summary()
        };
        let old_source = if text_allowed {
            self.object_text(&record.old_oid)?
        } else {
            None
        };
        let mut change = Change {
            path: record.path,
            old_path: record.old_path,
            category,
            status: record.status,
            hunks: if text_allowed {
                patch.hunks
            } else {
                Vec::new()
            },
            summary,
            oversized,
            old_source,
            new_source: None,
        };
        match worktree {
            Some(Err(error)) => attach_worktree_source(&mut change, Err(error)),
            Some(Ok(entry)) if text_allowed => {
                attach_worktree_source(&mut change, source::read_text(&full, entry));
            }
            None if text_allowed => change.new_source = self.object_text(&record.new_oid)?,
            _ => {}
        }
        Ok(change)
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
        let explicitly_loaded = loaded.contains(&path);
        let mut change = Change {
            path,
            old_path: None,
            category,
            status: if conflict { 'U' } else { '?' },
            hunks: Vec::new(),
            summary: None,
            oversized: false,
            old_source: None,
            new_source: None,
        };
        let bytes = match source::inspect(&full)? {
            Entry::Missing => {
                change.summary = Some(missing_file_summary(conflict).into());
                return Ok(change);
            }
            Entry::Symlink { .. } => {
                change.summary = Some(match source::read_link(&full)? {
                    Some(target) => format!("Symbolic link → {}", target.to_string_lossy()),
                    None => missing_file_summary(conflict).into(),
                });
                return Ok(change);
            }
            Entry::Special { .. } => {
                change.summary = Some("Special file (contents not read)".into());
                return Ok(change);
            }
            Entry::Regular { bytes } => bytes,
        };
        if !explicitly_loaded && bytes > limits.bytes {
            change.oversized = true;
            change.summary = Some(format!(
                "Oversized file ({bytes} bytes). Select this file to load its contents."
            ));
            return Ok(change);
        }
        let Some(bytes) = source::read_file(&full)? else {
            change.summary = Some(missing_file_summary(conflict).into());
            return Ok(change);
        };
        if bytes.contains(&0) {
            change.summary = Some("Binary file".into());
            return Ok(change);
        }
        let Ok(source) = String::from_utf8(bytes) else {
            change.summary = Some("Non-UTF-8 file (text preview unavailable)".into());
            return Ok(change);
        };
        let line_count = source.split_terminator('\n').count();
        if !explicitly_loaded && line_count > limits.lines {
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

struct SourceSizes {
    old: u64,
    new: Option<u64>,
}

impl SourceSizes {
    fn exceeds(&self, limit: u64) -> bool {
        self.old > limit || self.new.is_some_and(|bytes| bytes > limit)
    }

    fn oversized_summary(&self, line_count: usize) -> String {
        let size = match self.new {
            Some(new) => format!("{} bytes", self.old.max(new)),
            None => format!("{} old-source bytes; new size unavailable", self.old),
        };
        format!(
            "Oversized file ({size}, {line_count} diff lines). Select this file to load its contents."
        )
    }
}

/// Full source enables expansion, but a read failure must not discard the parsed patch.
fn attach_worktree_source(change: &mut Change, source: Result<TextSource, GitError>) {
    match source {
        Ok(TextSource::Text(text)) => change.new_source = Some(text),
        Ok(TextSource::Missing | TextSource::NonUtf8 | TextSource::Special) => {}
        Err(error) => {
            let diagnostic = format!("Full working-tree source unavailable: {error}");
            match &mut change.summary {
                Some(summary) => {
                    summary.push('\n');
                    summary.push_str(&diagnostic);
                }
                None => change.summary = Some(diagnostic),
            }
        }
    }
}

fn missing_file_summary(conflict: bool) -> &'static str {
    if conflict {
        "Unmerged conflict — no working-tree file. Resolve the deletion with Git."
    } else {
        "File disappeared during collection; awaiting refresh."
    }
}

fn diff_args(
    extra: &[&str],
    conflicts: &[PathBuf],
    whitespace: Whitespace,
    names_only: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-c",
        "diff.suppressBlankEmpty=false",
        "diff",
        "-z",
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
    args.extend(
        if names_only {
            &["--name-only"][..]
        } else {
            &["--raw", "--numstat", "--patch"][..]
        }
        .iter()
        .map(OsString::from),
    );
    args.extend(whitespace.git_flag().map(OsString::from));
    args.extend(extra.iter().map(OsString::from));
    args.push("--".into());
    for path in conflicts {
        let mut exclude = OsString::from(":(top,literal,exclude)");
        exclude.push(path);
        args.push(exclude);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, path::Path};

    fn parsed_change(summary: Option<&str>) -> Change {
        Change {
            path: "file.txt".into(),
            old_path: None,
            category: Category::Unstaged,
            status: 'M',
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".into(),
                lines: vec![SourceLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(1),
                    text: "after".into(),
                    no_newline: false,
                }],
            }],
            summary: summary.map(str::to_owned),
            oversized: false,
            old_source: Some(Arc::from("before\n")),
            new_source: None,
        }
    }

    fn read_failure() -> GitError {
        GitError::file(
            "Read working-tree file",
            Path::new("file.txt"),
            io::Error::new(io::ErrorKind::PermissionDenied, "read denied"),
        )
    }

    #[test]
    fn unavailable_full_source_keeps_the_patch_and_old_source() {
        // Given
        let mut change = parsed_change(None);
        // When
        attach_worktree_source(&mut change, Err(read_failure()));
        // Then
        assert_eq!(change.hunks.len(), 1);
        assert_eq!(change.hunks[0].lines[0].text, "after");
        assert_eq!(change.old_source.as_deref(), Some("before\n"));
        assert!(change.new_source.is_none());
        assert_eq!(
            change.summary.as_deref(),
            Some(
                "Full working-tree source unavailable: Read working-tree file file.txt: read denied"
            )
        );
    }

    #[test]
    fn source_diagnostics_preserve_existing_file_summaries() {
        // Given
        let mut change = parsed_change(Some("old mode 100644\nnew mode 100755"));
        // When
        attach_worktree_source(&mut change, Err(read_failure()));
        // Then
        assert_eq!(
            change.summary.as_deref(),
            Some(
                "old mode 100644\nnew mode 100755\nFull working-tree source unavailable: Read working-tree file file.txt: read denied"
            )
        );
    }
}
