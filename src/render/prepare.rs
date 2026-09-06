use super::{
    Cancellation, Cancelled,
    refine::{self, ALGORITHMS, Pair},
};
use crate::{
    config::Emphasis,
    model::{Change, LineKind, SourceLine},
};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

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
    /// Number of refinement operations started within this budget.
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
        if self.remaining.is_zero() || maximum.is_zero() {
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
    emphasis_mode: Emphasis,
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
            if refined && emphasis_mode != Emphasis::Off {
                for pair in &pairs {
                    cancel.check()?;
                    if let (Some(a), Some(b)) = (pair.old, pair.new) {
                        let old = &hunk.lines[a].text;
                        let new = &hunk.lines[b].text;
                        if old != new && old.len().max(new.len()) <= 4096 {
                            let ranges = budget
                                .run(Duration::from_millis(3), |timeout| {
                                    refine::emphasize(
                                        old,
                                        new,
                                        ALGORITHMS.inline,
                                        emphasis_mode,
                                        Instant::now() + timeout,
                                        cancel,
                                    )
                                })
                                .transpose()?;
                            if let Some((left, right)) = ranges {
                                if !left.is_empty() {
                                    emphasis.insert(a, left);
                                }
                                if !right.is_empty() {
                                    emphasis.insert(b, right);
                                }
                            }
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
    // Keep alignment's allowance consistent across emphasis modes.
    // Preserve ten milliseconds of the shared work allowance for inline emphasis.
    let maximum = budget
        .remaining
        .saturating_sub(Duration::from_millis(10))
        .min(Duration::from_millis(3));
    let pairs = if refined {
        budget
            .run(maximum, |timeout| {
                refine::align(
                    lines,
                    old,
                    new,
                    ALGORITHMS,
                    Instant::now() + timeout,
                    cancel,
                )
            })
            .transpose()?
    } else {
        None
    };
    match pairs {
        Some(pairs) => Ok(pairs),
        None => refine::positional(old, new, cancel),
    }
}
