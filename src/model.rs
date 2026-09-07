use crate::config::Whitespace;
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Unstaged,
    Untracked,
    Staged,
    Comparison,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unstaged => "Unstaged",
            Self::Untracked => "Untracked",
            Self::Staged => "Staged",
            Self::Comparison => "Changes",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    Conflict,
}

/// A source line and its position on each side of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceLine {
    /// The line's role in the diff.
    pub kind: LineKind,
    /// One-based line number in the old source, when present.
    pub old: Option<usize>,
    /// One-based line number in the new source, when present.
    pub new: Option<usize>,
    /// Line content without the diff prefix or trailing newline.
    pub text: String,
    /// Whether this line lacks a terminating newline in the source.
    pub no_newline: bool,
}

/// A contiguous group of diff lines with its patch header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hunk {
    /// The patch header describing the affected line ranges.
    pub header: String,
    /// Source lines in patch order.
    pub lines: Vec<SourceLine>,
}

/// A file change within one diff category.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Change {
    /// Current file path relative to the repository root.
    pub path: PathBuf,
    /// Previous path for a renamed or copied file.
    pub old_path: Option<PathBuf>,
    /// The diff category containing this change.
    pub category: Category,
    /// Git status code describing the change.
    pub status: char,
    /// Parsed text hunks available for display.
    pub hunks: Vec<Hunk>,
    /// Optional description for special or omitted content.
    pub summary: Option<String>,
    /// Whether content was omitted because it exceeds loading limits.
    pub oversized: bool,
    /// Complete old source for syntax highlighting, when available.
    pub old_source: Option<Arc<str>>,
    /// Complete new source for syntax highlighting, when available.
    pub new_source: Option<Arc<str>>,
}

/// Added and removed lines reported by Git, independently of preview limits.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LineStats {
    /// Lines added across the counted diffs.
    pub added: u64,
    /// Lines removed across the counted diffs.
    pub removed: u64,
}

impl LineStats {
    pub(crate) fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            added: self.added.checked_add(other.added)?,
            removed: self.removed.checked_add(other.removed)?,
        })
    }
}

/// Collected repository changes and their display label.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// File changes collected for the active mode.
    pub changes: Vec<Change>,
    /// Branch name or revision comparison label.
    pub branch: String,
    /// Whitespace policy applied to this snapshot's Git patches.
    pub whitespace: Whitespace,
    /// Distinct paths hidden in every category by whitespace comparison.
    pub whitespace_hidden_files: usize,
    /// Whole-repository tracked totals for the active comparison and whitespace policy.
    /// Untracked files and unresolved conflicts are excluded. `None` means uncollected.
    pub tracked_stats: Option<LineStats>,
}

impl Snapshot {
    pub fn sort(&mut self) {
        self.changes.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.path.cmp(&b.path))
        });
    }
}

pub fn display_path(path: &std::path::Path) -> String {
    crate::text::escape_controls(
        path.to_string_lossy()
            .chars()
            .map(|ch| if cfg!(windows) && ch == '\\' { '/' } else { ch }),
    )
}
