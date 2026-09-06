//! Bounded alignment and inline emphasis, independent of syntax and layout.
use super::{Cancellation, Cancelled};
use crate::{config::Emphasis, model::SourceLine};
use similar::{
    Algorithm, ChangeTag, DiffTag, InlineChangeMode, InlineChangeOptions, TextDiff,
    capture_diff_slices_by_key_deadline,
};
use std::{
    ops::Range,
    time::{Duration, Instant},
};

/// Algorithms are explicit so the standalone refinement benchmark uses the same code.
#[derive(Clone, Copy)]
pub(super) struct Algorithms {
    pub block: Algorithm,
    pub similarity: Algorithm,
    pub inline: Algorithm,
}

pub(super) const ALGORITHMS: Algorithms = Algorithms {
    // Selected by the quality-gated refine_bench corpus; see docs/refinement.md.
    block: Algorithm::RawMyers,
    similarity: Algorithm::Myers,
    inline: Algorithm::RawMyers,
};

const MAX_BLOCK_BYTES: usize = 64 * 1024;
const MAX_CANDIDATES: usize = 4096;
const MAX_SIMILARITY_BYTES: usize = 2048;
const MIN_SIMILARITY: f32 = 0.65;

/// Hunk indices on each side of a displayed row, in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pair {
    pub old: Option<usize>,
    pub new: Option<usize>,
}

pub(super) fn positional(
    old: &[usize],
    new: &[usize],
    cancel: &Cancellation,
) -> Result<Vec<Pair>, Cancelled> {
    (0..old.len().max(new.len()))
        .map(|i| {
            cancel.check()?;
            Ok(Pair {
                old: old.get(i).copied(),
                new: new.get(i).copied(),
            })
        })
        .collect()
}

pub(super) fn align(
    lines: &[SourceLine],
    old: &[usize],
    new: &[usize],
    algorithms: Algorithms,
    deadline: Instant,
    cancel: &Cancellation,
) -> Result<Vec<Pair>, Cancelled> {
    cancel.check()?;
    // Count a line terminator too, so empty lines cannot bypass the input limit.
    let bytes = old.iter().chain(new).try_fold(0usize, |bytes, i| {
        cancel.check()?;
        Ok(bytes.saturating_add(lines[*i].text.len()).saturating_add(1))
    })?;
    if old.is_empty() || new.is_empty() || bytes > MAX_BLOCK_BYTES || Instant::now() >= deadline {
        return positional(old, new, cancel);
    }
    let ops = capture_diff_slices_by_key_deadline(
        algorithms.block,
        old,
        new,
        |i| lines[*i].text.trim(),
        Some(deadline),
    );
    let mut pairs = Vec::with_capacity(old.len().max(new.len()));
    for op in ops {
        cancel.check()?;
        let (tag, a, b) = op.as_tag_tuple();
        let (old, new) = (&old[a], &new[b]);
        // Equal keys anchor indentation-only changes as well as exact matches.
        // Only replacements need fuzzy matching; timed-out regions use positional rows.
        let matched = if tag == DiffTag::Replace {
            fuzzy(lines, old, new, algorithms.similarity, deadline, cancel)?
        } else {
            None
        };
        pairs.extend(match matched {
            Some(matched) => matched,
            None => positional(old, new, cancel)?,
        });
    }
    Ok(pairs)
}

#[derive(Clone, Copy, Default)]
enum Step {
    Pair,
    #[default]
    Remove,
    Insert,
}

#[derive(Clone, Copy, Default)]
struct Cell {
    score: f32,
    step: Step,
}

fn fuzzy(
    lines: &[SourceLine],
    old: &[usize],
    new: &[usize],
    algorithm: Algorithm,
    deadline: Instant,
    cancel: &Cancellation,
) -> Result<Option<Vec<Pair>>, Cancelled> {
    if old.len().saturating_mul(new.len()) > MAX_CANDIDATES
        || old
            .iter()
            .chain(new)
            .any(|i| lines[*i].text.len() > MAX_SIMILARITY_BYTES)
        || Instant::now() >= deadline
    {
        return Ok(None);
    }
    let columns = new.len() + 1;
    let mut cells = vec![Cell::default(); (old.len() + 1) * columns];
    // Suffix scores allow forward reconstruction with stable ties: pair, remove, insert.
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            cancel.check()?;
            if Instant::now() >= deadline {
                return Ok(None);
            }
            let (a, b) = (lines[old[i]].text.trim(), lines[new[j]].text.trim());
            let comparison_deadline = deadline.min(Instant::now() + Duration::from_millis(1));
            let ratio = TextDiff::configure()
                .algorithm(algorithm)
                .deadline(comparison_deadline)
                .diff_chars(a, b)
                .ratio();
            if Instant::now() >= comparison_deadline {
                return Ok(None);
            }
            let remove = cells[(i + 1) * columns + j].score;
            let insert = cells[i * columns + j + 1].score;
            let paired = ratio + cells[(i + 1) * columns + j + 1].score;
            cells[i * columns + j] = if ratio >= MIN_SIMILARITY && paired >= remove.max(insert) {
                Cell {
                    score: paired,
                    step: Step::Pair,
                }
            } else if remove >= insert {
                Cell {
                    score: remove,
                    step: Step::Remove,
                }
            } else {
                Cell {
                    score: insert,
                    step: Step::Insert,
                }
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut pairs = Vec::with_capacity(old.len().max(new.len()));
    while i < old.len() && j < new.len() {
        cancel.check()?;
        let step = cells[i * columns + j].step;
        pairs.push(Pair {
            old: (!matches!(step, Step::Insert)).then_some(old[i]),
            new: (!matches!(step, Step::Remove)).then_some(new[j]),
        });
        i += usize::from(!matches!(step, Step::Insert));
        j += usize::from(!matches!(step, Step::Remove));
    }
    pairs.extend(positional(&old[i..], &new[j..], cancel)?);
    Ok(Some(pairs))
}

/// Emphasized UTF-8 byte ranges on the old and new sides.
pub(super) type EmphasisRanges = (Vec<Range<usize>>, Vec<Range<usize>>);

pub(super) fn emphasize(
    old: &str,
    new: &str,
    algorithm: Algorithm,
    emphasis: Emphasis,
    deadline: Instant,
    cancel: &Cancellation,
) -> Result<EmphasisRanges, Cancelled> {
    cancel.check()?;
    let mode = match emphasis {
        Emphasis::Word => InlineChangeMode::UnicodeWords,
        Emphasis::Grapheme => InlineChangeMode::Graphemes,
        Emphasis::Off => return Ok((Vec::new(), Vec::new())),
    };
    if old == new || old.len().max(new.len()) > 4096 || Instant::now() >= deadline {
        return Ok((Vec::new(), Vec::new()));
    }
    let diff = TextDiff::configure()
        .deadline(deadline)
        .diff_slices(&[old], &[new]);
    let mut options = InlineChangeOptions::new();
    options
        .algorithm(algorithm)
        .mode(mode)
        .min_ratio(0.25)
        .semantic_cleanup(true);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for change in diff.iter_all_inline_changes_with_options_deadline(options, Some(deadline)) {
        cancel.check()?;
        let ranges = match change.tag() {
            ChangeTag::Delete => &mut left,
            ChangeTag::Insert => &mut right,
            ChangeTag::Equal => continue,
        };
        let mut offset = 0;
        for &(emphasized, value) in change.values() {
            cancel.check()?;
            let end = offset + value.len();
            if emphasized && end > offset {
                append_range(ranges, offset..end);
            }
            offset = end;
        }
    }
    // Refinement and semantic cleanup are cooperative; never retain a partial result.
    if Instant::now() >= deadline {
        Ok((Vec::new(), Vec::new()))
    } else {
        Ok((left, right))
    }
}

fn append_range(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    if let Some(last) = ranges.last_mut().filter(|last| last.end == range.start) {
        last.end = range.end;
    } else {
        ranges.push(range);
    }
}
