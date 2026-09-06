use super::{Cancellation, Cancelled};
use crate::model::{Change, LineKind, SourceLine};
use similar::{ChangeTag, TextDiff};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

/// Aligned old and new line indices within a hunk.
#[derive(Clone, Copy)]
pub(super) struct Pair {
    /// Zero-based hunk index of the removed line, when present.
    pub old: Option<usize>,
    /// Zero-based hunk index of the added line, when present.
    pub new: Option<usize>,
}

/// Alignment and inline emphasis for a contiguous block of changed lines.
pub(super) struct Block {
    /// Exclusive end index of the block in the hunk's lines.
    pub end: usize,
    /// Old and new line pairings in display order.
    pub pairs: Vec<Pair>,
    /// Emphasized byte ranges keyed by hunk line index.
    pub emphasis: BTreeMap<usize, Vec<Range<usize>>>,
}

/// A file change prepared for diff layout.
pub(super) struct Prepared {
    /// Content fingerprint used to look up cached preparation results.
    pub hash: u64,
    /// Source change shared with cached render results.
    pub change: Arc<Change>,
    /// Blocks for each hunk, keyed by their starting line index.
    pub blocks: Vec<BTreeMap<usize, Block>>,
    /// Whether preparation attempted detailed alignment and inline emphasis.
    pub refined: bool,
}

impl Prepared {
    pub fn bytes(&self) -> usize {
        self.change.old_source.as_ref().map_or(0, |s| s.len())
            + self.change.new_source.as_ref().map_or(0, |s| s.len())
            + self
                .change
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .map(|l| l.text.len() + 160)
                .sum::<usize>()
            + self
                .blocks
                .iter()
                .flat_map(|h| h.values())
                .map(|block| {
                    block.pairs.len() * size_of::<Pair>()
                        + block
                            .emphasis
                            .values()
                            .map(|r| r.len() * size_of::<Range<usize>>())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}

/// One allowance shared by every file and hunk in a rendering operation.
pub(super) struct WorkBudget {
    remaining: Duration,
    /// Number of comparison operations started within this budget.
    pub comparisons: u64,
}
impl WorkBudget {
    pub fn new(remaining: Duration) -> Self {
        Self {
            remaining,
            comparisons: 0,
        }
    }
    fn run<T>(&mut self, maximum: Duration, operation: impl FnOnce(Duration) -> T) -> Option<T> {
        if self.remaining.is_zero() {
            return None;
        }
        let start = Instant::now();
        self.comparisons += 1;
        let result = operation(self.remaining.min(maximum));
        self.remaining = self.remaining.saturating_sub(start.elapsed());
        Some(result)
    }
}

pub(super) fn prepare(
    hash: u64,
    change: Arc<Change>,
    refined: bool,
    budget: &mut WorkBudget,
    cancel: &Cancellation,
) -> Result<Prepared, Cancelled> {
    let mut hunks = Vec::new();
    for hunk in &change.hunks {
        cancel.check()?;
        let mut blocks = BTreeMap::new();
        let mut start = 0;
        while start < hunk.lines.len() {
            cancel.check()?;
            if matches!(
                hunk.lines[start].kind,
                LineKind::Context | LineKind::Conflict
            ) {
                start += 1;
                continue;
            }
            let end = hunk.lines[start..]
                .iter()
                .position(|l| matches!(l.kind, LineKind::Context | LineKind::Conflict))
                .map(|i| start + i)
                .unwrap_or(hunk.lines.len());
            let old: Vec<_> = (start..end)
                .filter(|i| hunk.lines[*i].kind == LineKind::Removed)
                .collect();
            let new: Vec<_> = (start..end)
                .filter(|i| hunk.lines[*i].kind == LineKind::Added)
                .collect();
            let pairs = align(&hunk.lines, &old, &new, refined, budget, cancel)?;
            let mut emphasis = BTreeMap::new();
            if refined {
                for pair in &pairs {
                    cancel.check()?;
                    if let (Some(a), Some(b)) = (pair.old, pair.new) {
                        let old = &hunk.lines[a].text;
                        let new = &hunk.lines[b].text;
                        if old.len().max(new.len()) <= 4096
                            && let Some((left, right)) = budget
                                .run(Duration::from_millis(3), |timeout| {
                                    emphasize(old, new, timeout)
                                })
                        {
                            emphasis.insert(a, left);
                            emphasis.insert(b, right);
                        }
                    }
                }
            }
            blocks.insert(
                start,
                Block {
                    end,
                    pairs,
                    emphasis,
                },
            );
            start = end;
        }
        hunks.push(blocks);
    }
    Ok(Prepared {
        hash,
        change,
        blocks: hunks,
        refined,
    })
}

fn align(
    lines: &[SourceLine],
    old: &[usize],
    new: &[usize],
    refined: bool,
    budget: &mut WorkBudget,
    cancel: &Cancellation,
) -> Result<Vec<Pair>, Cancelled> {
    let (mut i, mut j) = (0, 0);
    let mut pairs = Vec::new();
    while i < old.len() && j < new.len() {
        cancel.check()?;
        let mut ratio = |a: usize, b: usize| {
            let (a, b) = (&lines[a].text, &lines[b].text);
            // Reserve at least half the shared allowance for word emphasis.
            if !refined
                || budget.remaining <= Duration::from_millis(10)
                || a.len().max(b.len()) > 2048
            {
                return 1.0;
            }
            budget
                .run(Duration::from_millis(1), |timeout| {
                    TextDiff::configure()
                        .timeout(timeout)
                        .diff_chars(a, b)
                        .ratio()
                })
                .unwrap_or(1.0)
        };
        let current = ratio(old[i], new[j]);
        if current < 0.65 {
            let next_old = old.get(i + 1).map(|a| ratio(*a, new[j])).unwrap_or(0.0);
            let next_new = new.get(j + 1).map(|b| ratio(old[i], *b)).unwrap_or(0.0);
            if next_old > current.max(0.65) && next_old >= next_new {
                pairs.push(Pair {
                    old: Some(old[i]),
                    new: None,
                });
                i += 1;
                continue;
            }
            if next_new > current.max(0.65) {
                pairs.push(Pair {
                    old: None,
                    new: Some(new[j]),
                });
                j += 1;
                continue;
            }
        }
        pairs.push(Pair {
            old: Some(old[i]),
            new: Some(new[j]),
        });
        i += 1;
        j += 1;
    }
    pairs.extend(old[i..].iter().map(|i| Pair {
        old: Some(*i),
        new: None,
    }));
    pairs.extend(new[j..].iter().map(|i| Pair {
        old: None,
        new: Some(*i),
    }));
    Ok(pairs)
}

fn emphasize(old: &str, new: &str, timeout: Duration) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let diff = TextDiff::configure()
        .timeout(timeout)
        .diff_unicode_words(old, new);
    if diff.ratio() < 0.25 {
        return (Vec::new(), Vec::new());
    }
    let (mut a, mut b) = (0, 0);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                a += len;
                b += len;
            }
            ChangeTag::Delete => {
                left.push(a..a + len);
                a += len;
            }
            ChangeTag::Insert => {
                right.push(b..b + len);
                b += len;
            }
        }
    }
    (left, right)
}
