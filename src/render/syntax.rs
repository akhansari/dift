use super::{
    Cancellation, Cancelled,
    cache::{Cache, fingerprint},
};
use ratatui::style::{Color, Style};
use std::{ops::Range, path::Path, sync::Arc};
use syntect::{
    easy::HighlightLines,
    highlighting::{HighlightState, Theme},
    parsing::{ParseState, SyntaxReference, SyntaxSet},
};

pub(super) type LineColors = Arc<[(Range<usize>, Style)]>;
pub(super) type SyntaxColors = Arc<[LineColors]>;
const CHECKPOINT_LINES: usize = 128;
const MAX_SYNTAX_LINE_BYTES: usize = 16 * 1024;

#[derive(Clone)]
struct Checkpoint {
    line: usize,
    byte: usize,
    highlight: HighlightState,
    parse: ParseState,
}

struct Entry {
    hash: u64,
    syntax: String,
    source: Arc<str>,
    colors: Vec<LineColors>,
    checkpoints: Vec<Checkpoint>,
}
impl Entry {
    fn bytes(&self) -> usize {
        self.source.len()
            + self
                .colors
                .iter()
                .map(|line| line.len() * size_of::<(Range<usize>, Style)>() + 32)
                .sum::<usize>()
            + self.checkpoints.len() * 4096
    }
}

/// Cached syntax colors and parser checkpoints for source files.
pub(super) struct SyntaxCache {
    cache: Cache<Entry>,
    /// Total source lines parsed for syntax highlighting.
    pub parsed_lines: u64,
    /// Total syntax-highlighted lines reused from cached entries.
    pub reused_lines: u64,
}
impl SyntaxCache {
    pub fn new() -> Self {
        Self {
            cache: Cache::new(64 * 1024 * 1024),
            parsed_lines: 0,
            reused_lines: 0,
        }
    }

    pub fn bytes(&self) -> usize {
        self.cache.bytes()
    }

    /// Parse through the last required source line, retaining states so cancelled
    /// jobs and edits can resume without reparsing the unchanged prefix.
    pub fn highlight(
        &mut self,
        syntaxes: &SyntaxSet,
        theme: &Theme,
        path: &Path,
        source: Option<&Arc<str>>,
        through: usize,
        cancel: &Cancellation,
    ) -> Result<SyntaxColors, Cancelled> {
        cancel.check()?;
        let Some(source) = source.filter(|_| through > 0) else {
            return Ok(Arc::from([]));
        };
        let syntax = syntax_for(syntaxes, path);
        if std::ptr::eq(syntax, syntaxes.find_syntax_plain_text()) {
            return Ok(Arc::from([]));
        }
        let hash = fingerprint(source);
        let entry = self.cache.take(|entry| {
            entry.hash == hash && entry.syntax == syntax.name && entry.source == *source
        });
        let mut entry = entry
            .map(|(entry, _)| entry)
            .unwrap_or_else(|| self.seed(syntax, source, hash, theme));
        let target = through.min(source.split_inclusive('\n').count());
        if entry.colors.len() >= target {
            self.reused_lines += target as u64;
            let colors = entry.colors[..target].to_vec().into();
            let bytes = entry.bytes();
            self.cache.insert(entry, bytes);
            return Ok(colors);
        }
        let checkpoint = entry.checkpoints.last().cloned();
        let (mut highlighter, mut byte) = if let Some(checkpoint) = checkpoint {
            entry.colors.truncate(checkpoint.line);
            self.reused_lines += checkpoint.line as u64;
            (
                HighlightLines::from_state(theme, checkpoint.highlight, checkpoint.parse),
                checkpoint.byte,
            )
        } else {
            (HighlightLines::new(syntax, theme), 0)
        };
        let mut cancelled = false;
        let mut plain_remainder = false;
        while entry.colors.len() < target {
            if cancel.check().is_err() {
                cancelled = true;
                break;
            }
            let tail = &source[byte..];
            let len = tail.find('\n').map(|i| i + 1).unwrap_or(tail.len());
            let line = &tail[..len];
            // Regex parsing cannot be interrupted within one line. Very long or
            // invalid lines retain diff styling; don't resume from an invalid state.
            plain_remainder |= line.len() > MAX_SYNTAX_LINE_BYTES;
            let colors = if plain_remainder {
                Arc::from([])
            } else {
                self.parsed_lines += 1;
                match highlighter.highlight_line(line, syntaxes) {
                    Ok(spans) => {
                        let mut offset = 0;
                        spans
                            .into_iter()
                            .map(|(style, text)| {
                                let range = offset..offset + text.len();
                                offset += text.len();
                                (
                                    range,
                                    Style::default().fg(Color::Rgb(
                                        style.foreground.r,
                                        style.foreground.g,
                                        style.foreground.b,
                                    )),
                                )
                            })
                            .collect::<Vec<_>>()
                            .into()
                    }
                    Err(_) => {
                        plain_remainder = true;
                        Arc::from([])
                    }
                }
            };
            entry.colors.push(colors);
            byte += len;
            if !plain_remainder && entry.colors.len().is_multiple_of(CHECKPOINT_LINES) {
                let (highlight, parse) = highlighter.state();
                entry.checkpoints.push(Checkpoint {
                    line: entry.colors.len(),
                    byte,
                    highlight: highlight.clone(),
                    parse: parse.clone(),
                });
                highlighter = HighlightLines::from_state(theme, highlight, parse);
            }
        }
        if !plain_remainder
            && entry
                .checkpoints
                .last()
                .is_none_or(|last| last.line != entry.colors.len())
        {
            let (highlight, parse) = highlighter.state();
            entry.checkpoints.push(Checkpoint {
                line: entry.colors.len(),
                byte,
                highlight,
                parse,
            });
        }
        let colors = entry.colors.to_vec().into();
        let bytes = entry.bytes();
        self.cache.insert(entry, bytes);
        if cancelled {
            Err(Cancelled)
        } else {
            cancel.check()?;
            Ok(colors)
        }
    }

    fn seed(&self, syntax: &SyntaxReference, source: &Arc<str>, hash: u64, theme: &Theme) -> Entry {
        let reusable = self
            .cache
            .iter()
            .filter(|entry| entry.syntax == syntax.name)
            .filter_map(|entry| {
                let common = source
                    .bytes()
                    .zip(entry.source.bytes())
                    .take_while(|(a, b)| a == b)
                    .count();
                entry
                    .checkpoints
                    .iter()
                    .rfind(|checkpoint| {
                        // A completed source may end midway through a line. Its
                        // EOF state is reusable for that exact version only;
                        // appending text must reparse the entire final line.
                        checkpoint.byte <= common
                            && (checkpoint.byte == 0
                                || source.as_bytes()[checkpoint.byte - 1] == b'\n')
                    })
                    .map(|checkpoint| (entry, checkpoint))
            })
            .max_by_key(|(_, checkpoint)| checkpoint.byte);
        if let Some((entry, checkpoint)) = reusable {
            Entry {
                hash,
                syntax: syntax.name.clone(),
                source: Arc::clone(source),
                colors: entry.colors[..checkpoint.line].to_vec(),
                checkpoints: entry
                    .checkpoints
                    .iter()
                    .take_while(|state| state.line <= checkpoint.line)
                    .cloned()
                    .collect(),
            }
        } else {
            let (highlight, parse) = HighlightLines::new(syntax, theme).state();
            Entry {
                hash,
                syntax: syntax.name.clone(),
                source: Arc::clone(source),
                colors: Vec::new(),
                checkpoints: vec![Checkpoint {
                    line: 0,
                    byte: 0,
                    highlight,
                    parse,
                }],
            }
        }
    }
}

fn syntax_for<'a>(syntaxes: &'a SyntaxSet, path: &Path) -> &'a SyntaxReference {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(|e| syntaxes.find_syntax_by_extension(e))
        .or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| syntaxes.find_syntax_by_token(n))
        })
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text())
}
