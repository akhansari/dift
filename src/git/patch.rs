use std::path::PathBuf;

use crate::model::{Hunk, LineKind, LineStats, SourceLine};

use super::{GitError, path_from_bytes};

pub(super) struct RawPatch {
    pub(super) path: PathBuf,
    pub(super) old_path: Option<PathBuf>,
    pub(super) status: char,
    pub(super) old_oid: String,
    pub(super) new_oid: String,
    pub(super) old_mode: String,
    pub(super) new_mode: String,
    pub(super) patch: String,
}

pub(super) struct ParsedPatch {
    pub(super) hunks: Vec<Hunk>,
    pub(super) line_count: usize,
    pub(super) kind: PatchKind,
}

pub(super) enum PatchKind {
    Text,
    Metadata(String),
    Binary,
    Submodule(String),
}

impl PatchKind {
    pub(super) fn allows_text(&self) -> bool {
        matches!(self, Self::Text | Self::Metadata(_))
    }

    pub(super) fn into_summary(self) -> Option<String> {
        match self {
            Self::Text => None,
            Self::Metadata(summary) | Self::Submodule(summary) => Some(summary),
            Self::Binary => Some("Binary file changed".into()),
        }
    }
}

impl RawPatch {
    pub(super) fn is_submodule(&self) -> bool {
        self.old_mode == "160000" || self.new_mode == "160000"
    }

    pub(super) fn parse(&self) -> ParsedPatch {
        let (hunks, metadata) = parse_patch(&self.patch);
        let line_count = hunks.iter().map(|hunk| hunk.lines.len()).sum();
        let binary = self
            .patch
            .lines()
            .take_while(|line| !line.starts_with("@@ "))
            .any(|line| line.starts_with("Binary files ") || line == "GIT binary patch");
        let kind = if binary {
            PatchKind::Binary
        } else if self.is_submodule() {
            let commits = self
                .patch
                .lines()
                .filter(|line| line.contains("Subproject commit"))
                .collect::<Vec<_>>()
                .join("\n");
            PatchKind::Submodule(format!("Submodule change\n{commits}"))
        } else if hunks.is_empty() {
            PatchKind::Metadata(if metadata.is_empty() {
                "File metadata changed".into()
            } else {
                metadata
            })
        } else {
            PatchKind::Text
        };
        ParsedPatch {
            hunks,
            line_count,
            kind,
        }
    }
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

pub(super) fn parse_raw_patch(mut input: &[u8]) -> Result<(Vec<RawPatch>, LineStats), GitError> {
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
    // Git emits raw records, matching numstat records, then a NUL and patch text.
    let stats = records
        .iter()
        .try_fold(LineStats::default(), |total, record| {
            let entry = take_nul(&mut input)?;
            let mut fields = entry.splitn(3, |byte| *byte == b'\t');
            let invalid = || GitError::InvalidOutput("Invalid Git numstat record".into());
            let added = fields.next().ok_or_else(invalid)?;
            let removed = fields.next().ok_or_else(invalid)?;
            let path = fields.next().ok_or_else(invalid)?;
            let (path, old_path) = if path.is_empty() {
                let old = path_from_bytes(take_nul(&mut input)?);
                (path_from_bytes(take_nul(&mut input)?), Some(old))
            } else {
                (path_from_bytes(path), None)
            };
            if path != record.path || old_path != record.old_path {
                return Err(GitError::InvalidOutput(
                    "Git numstat paths do not match raw records".into(),
                ));
            }
            let counts = if added == b"-" && removed == b"-" {
                LineStats::default()
            } else {
                LineStats {
                    added: parse_count(added)?,
                    removed: parse_count(removed)?,
                }
            };
            total
                .checked_add(counts)
                .ok_or_else(|| GitError::InvalidOutput("Git line statistics overflow".into()))
        })?;
    if !records.is_empty() {
        input = input
            .strip_prefix(b"\0")
            .ok_or_else(|| GitError::InvalidOutput("Missing Git patch separator".into()))?;
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
    Ok((records, stats))
}

fn parse_count(input: &[u8]) -> Result<u64, GitError> {
    let invalid = || GitError::InvalidOutput("Invalid Git numstat count".into());
    if input.is_empty() || !input.iter().all(u8::is_ascii_digit) {
        return Err(invalid());
    }
    std::str::from_utf8(input)
        .map_err(|_| invalid())?
        .parse()
        .map_err(|_| invalid())
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

    fn output_with_numstat(entry: &str) -> String {
        format!(":100644 100644 aaaa bbbb M\0file\0{entry}\0\0diff --git a/file b/file\n")
    }

    macro_rules! malformed_numstat {
        ($name:ident, $entry:expr) => {
            #[test]
            fn $name() {
                // Given
                let output = output_with_numstat($entry);
                // When
                let result = parse_raw_patch(output.as_bytes());
                // Then
                assert!(matches!(result, Err(GitError::InvalidOutput(_))));
            }
        };
    }

    malformed_numstat!(missing_counts_are_rejected, "file");
    malformed_numstat!(nonnumeric_counts_are_rejected, "bad\t1\tfile");
    malformed_numstat!(signed_counts_are_rejected, "+1\t1\tfile");
    malformed_numstat!(half_binary_counts_are_rejected, "-\t1\tfile");
    malformed_numstat!(
        overflowing_counts_are_rejected,
        "18446744073709551616\t0\tfile"
    );
    malformed_numstat!(mismatched_paths_are_rejected, "1\t1\tother");

    #[test]
    fn truncated_numstat_records_are_rejected() {
        // Given
        let output = b":100644 100644 aaaa bbbb M\0file\0";
        // When
        let result = parse_raw_patch(output);
        // Then
        assert!(matches!(result, Err(GitError::InvalidOutput(_))));
    }

    #[test]
    fn overflowing_totals_are_rejected() {
        // Given
        let output = b":100644 100644 aaaa bbbb M\0one\0:100644 100644 aaaa bbbb M\0two\0\
            18446744073709551615\t0\tone\0\
            1\t0\ttwo\0\0diff --git a/one b/one\ndiff --git a/two b/two\n";
        // When
        let result = parse_raw_patch(output);
        // Then
        assert!(matches!(result, Err(GitError::InvalidOutput(_))));
    }

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
