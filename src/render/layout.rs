use super::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn push_header(document: &mut Document, text: &str, style: Style) {
    let text: String = text
        .chars()
        .flat_map(|ch| {
            if ch.is_control() {
                ch.escape_default().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect();
    document.rows.push(Arc::new(Row {
        line: Line::styled(text, style),
        anchors: Vec::new(),
    }));
}

pub fn search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if query.chars().any(char::is_uppercase) {
        return text
            .match_indices(query)
            .map(|(i, value)| i..i + value.len())
            .collect();
    }
    let mut folded = String::new();
    let mut offsets = Vec::new();
    for (start, ch) in text.char_indices() {
        for lower in ch.to_lowercase() {
            folded.push(lower);
            offsets.extend(std::iter::repeat_n(
                (start, start + ch.len_utf8()),
                lower.len_utf8(),
            ));
        }
    }
    let needle = query.to_lowercase();
    folded
        .match_indices(&needle)
        .filter_map(|(i, value)| Some(offsets.get(i)?.0..offsets.get(i + value.len() - 1)?.1))
        .collect()
}

struct CellSpec<'a> {
    source: Option<&'a SourceLine>,
    /// Original side-specific context, which may differ under whitespace filtering.
    content: Option<(&'a str, bool)>,
    old: bool,
    colors: &'a SyntaxColors,
    emphasis: &'a [Range<usize>],
    query: &'a str,
}
struct CellRow {
    spans: Vec<Span<'static>>,
    anchor: Option<Anchor>,
    hits: usize,
    width: usize,
}

/// Hunk context visits source lines in order; scan each source at most once.
struct ContextSource<'a> {
    lines: std::str::SplitInclusive<'a, char>,
    number: usize,
}

impl<'a> ContextSource<'a> {
    fn new(source: Option<&'a str>) -> Self {
        Self {
            lines: source.unwrap_or("").split_inclusive('\n'),
            number: 0,
        }
    }

    fn at(&mut self, number: Option<usize>) -> Option<(&'a str, bool)> {
        let number = number?;
        let skip = number.checked_sub(self.number + 1)?;
        let line = self.lines.nth(skip)?;
        self.number = number;
        Some((
            line.strip_suffix('\n').unwrap_or(line),
            !line.ends_with('\n'),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn cells(
    change: &Change,
    path: &Arc<PathBuf>,
    spec: CellSpec<'_>,
    width: usize,
    unified: bool,
    palette: Palette,
    limit: Option<usize>,
    cancel: &Cancellation,
) -> Result<Vec<CellRow>, Cancelled> {
    let Some(source) = spec.source else {
        return Ok(Vec::new());
    };
    let (text, no_newline) = spec.content.unwrap_or((&source.text, source.no_newline));
    let number = if spec.old { source.old } else { source.new }.unwrap_or(0);
    let colors = spec
        .colors
        .get(number.saturating_sub(1))
        .map(|line| line.as_ref())
        .unwrap_or(&[]);
    let matches = search_ranges(text, spec.query);
    let base = palette.line(source.kind);
    let sign = match source.kind {
        LineKind::Added => '+',
        LineKind::Removed => '-',
        LineKind::Conflict => '!',
        LineKind::Context => ' ',
    };
    let number_text = |n: Option<usize>| n.map(|n| n.to_string()).unwrap_or_default();
    let digits = number_text(source.old)
        .len()
        .max(number_text(source.new).len())
        .max(5);
    let prefix = if unified && width >= 2 * digits + 10 {
        format!(
            "{:>digits$} {:>digits$} {sign} ",
            number_text(source.old),
            number_text(source.new)
        )
    } else if width >= digits + 7 {
        format!(
            "{:>digits$} {sign} ",
            if spec.old {
                number_text(source.old)
            } else {
                number_text(source.new)
            }
        )
    } else {
        format!("{sign} ")
    };
    let prefix_width = prefix.len();
    let source_text: Arc<str> = Arc::from(text);
    let make_row = |byte, first| CellRow {
        spans: vec![Span::styled(
            if first {
                prefix.clone()
            } else {
                " ".repeat(prefix_width)
            },
            base.fg(palette.muted),
        )],
        anchor: Some(Anchor {
            path: Arc::clone(path),
            category: change.category,
            old: spec.old,
            number,
            byte,
            text: Arc::clone(&source_text),
        }),
        hits: 0,
        width: prefix_width,
    };
    let mut rows = Vec::new();
    let mut row = make_row(0, true);
    let mut logical_column = 0;
    for (byte, ch) in text.char_indices() {
        if byte.is_multiple_of(256) {
            cancel.check()?;
        }
        let end = byte + ch.len_utf8();
        let text = if ch == '\t' {
            " ".repeat(4 - logical_column % 4)
        } else if ch.is_control() {
            ch.escape_default().to_string()
        } else {
            ch.to_string()
        };
        logical_column += UnicodeWidthStr::width(text.as_str());
        let mut style = base;
        if let Some((_, syntax)) = colors.iter().find(|(r, _)| r.contains(&byte)) {
            style = style.patch(*syntax);
        }
        if spec.emphasis.iter().any(|r| r.start < end && r.end > byte) {
            style = style
                .bg(if spec.old {
                    palette.removed_emphasis
                } else {
                    palette.added_emphasis
                })
                .add_modifier(Modifier::BOLD);
        }
        if matches.iter().any(|r| r.start < end && r.end > byte) {
            style = style
                .bg(Color::Rgb(237, 197, 102))
                .fg(Color::Rgb(25, 28, 35))
                .add_modifier(Modifier::BOLD);
        }
        for (part, glyph) in text.chars().enumerate() {
            let glyph_width = UnicodeWidthChar::width(glyph).unwrap_or(0);
            if row.width > prefix_width && row.width + glyph_width > width {
                row.spans.push(Span::styled(
                    " ".repeat(width.saturating_sub(row.width)),
                    base,
                ));
                rows.push(row);
                if limit.is_some_and(|limit| rows.len() >= limit) {
                    return Ok(rows);
                }
                row = make_row(byte, false);
            }
            if part == 0 {
                row.hits += matches
                    .iter()
                    .filter(|r| r.start >= byte && r.start < end)
                    .count();
            }
            if let Some(previous) = row.spans.last_mut().filter(|s| s.style == style) {
                previous.content.to_mut().push(glyph);
            } else {
                row.spans.push(Span::styled(glyph.to_string(), style));
            }
            row.width += glyph_width;
        }
    }
    row.spans.push(Span::styled(
        " ".repeat(width.saturating_sub(row.width)),
        base,
    ));
    rows.push(row);
    if no_newline {
        rows.extend(
            b"\\ No newline at end of file"
                .chunks(width)
                .map(|part| CellRow {
                    spans: vec![Span::styled(
                        format!("{:<width$}", String::from_utf8_lossy(part)),
                        base.fg(palette.muted),
                    )],
                    anchor: None,
                    hits: 0,
                    width,
                }),
        );
    }
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn append_unified(
    document: &mut Document,
    change: &Change,
    path: &Arc<PathBuf>,
    source: &SourceLine,
    colors: &SyntaxColors,
    emphasis: &[Range<usize>],
    query: &str,
    width: usize,
    palette: Palette,
    limit: Option<usize>,
    cancel: &Cancellation,
) -> Result<(), Cancelled> {
    let old = source.new.is_none();
    for row in cells(
        change,
        path,
        CellSpec {
            source: Some(source),
            content: None,
            old,
            colors,
            emphasis,
            query,
        },
        width,
        true,
        palette,
        limit,
        cancel,
    )? {
        cancel.check()?;
        document.hits.extend(std::iter::repeat_n(
            Hit {
                row: document.rows.len(),
            },
            row.hits,
        ));
        document.rows.push(Arc::new(Row {
            line: Line::from(row.spans),
            anchors: row.anchor.into_iter().collect(),
        }));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_split(
    document: &mut Document,
    change: &Change,
    path: &Arc<PathBuf>,
    left: CellSpec<'_>,
    right: CellSpec<'_>,
    width: usize,
    palette: Palette,
    limit: Option<usize>,
    cancel: &Cancellation,
) -> Result<(), Cancelled> {
    let a_width = (width - 1) / 2;
    let b_width = width - 1 - a_width;
    let a = cells(change, path, left, a_width, false, palette, limit, cancel)?;
    let b = cells(change, path, right, b_width, false, palette, limit, cancel)?;
    let count = a.len().max(b.len());
    let mut a = a.into_iter();
    let mut b = b.into_iter();
    for _ in 0..count {
        cancel.check()?;
        let mut spans = Vec::new();
        let mut anchors = Vec::new();
        let mut hits = 0;
        if let Some(row) = a.next() {
            spans.extend(row.spans);
            anchors.extend(row.anchor);
            hits += row.hits;
        } else {
            spans.push(Span::styled(" ".repeat(a_width), palette.base()));
        }
        spans.push(Span::styled("│", palette.base().fg(palette.muted)));
        if let Some(row) = b.next() {
            spans.extend(row.spans);
            anchors.extend(row.anchor);
            hits += row.hits;
        } else {
            spans.push(Span::styled(" ".repeat(b_width), palette.base()));
        }
        document.hits.extend(std::iter::repeat_n(
            Hit {
                row: document.rows.len(),
            },
            hits,
        ));
        document.rows.push(Arc::new(Row {
            line: Line::from(spans),
            anchors,
        }));
    }
    Ok(())
}

pub(super) fn file(
    prepared: &Prepared,
    visuals: &Visuals,
    config: &Config,
    spec: RenderSpec<'_>,
    limit: Option<usize>,
    cancel: &Cancellation,
) -> Result<Document, Cancelled> {
    let change = &prepared.change;
    let width = spec.width.max(4);
    let split = spec.split && width >= 32;
    let mut old_context = ContextSource::new(change.old_source.as_deref());
    let mut new_context = ContextSource::new(change.new_source.as_deref());
    let query = spec.query;
    let palette = Palette::new(config.light);
    let mut document = Document::default();
    let path = Arc::new(change.path.clone());
    let name = if let Some(old) = &change.old_path {
        format!(
            " {}  {} → {}",
            change.status,
            display_path(old),
            display_path(&change.path)
        )
    } else {
        format!(" {}  {}", change.status, display_path(&change.path))
    };
    push_header(
        &mut document,
        &name,
        palette
            .base()
            .bg(palette.panel)
            .add_modifier(Modifier::BOLD),
    );
    if let Some(summary) = &change.summary {
        for line in summary.lines() {
            push_header(
                &mut document,
                &format!("    {line}"),
                palette.base().fg(palette.muted),
            );
        }
        if change.oversized {
            document.excluded += 1;
            let key = config
                .bindings
                .keys(crate::input::Scope::Global, crate::input::Action::Accept);
            push_header(
                &mut document,
                &format!("    Select the file and press {key} to load."),
                palette.base().fg(palette.accent),
            );
        }
    }
    for (hunk_index, hunk) in change.hunks.iter().enumerate() {
        cancel.check()?;
        push_header(
            &mut document,
            &format!(" {}", hunk.header),
            palette.base().fg(palette.muted),
        );
        let mut position = 0;
        while position < hunk.lines.len() {
            cancel.check()?;
            if reached_limit(&mut document, limit) {
                return Ok(document);
            }
            let source = &hunk.lines[position];
            if let Some(block) = prepared.blocks[hunk_index].get(&position) {
                if split {
                    for pair in &block.pairs {
                        if reached_limit(&mut document, limit) {
                            return Ok(document);
                        }
                        let left = CellSpec {
                            source: pair.old.map(|i| &hunk.lines[i]),
                            content: None,
                            old: true,
                            colors: &visuals.old,
                            emphasis: pair
                                .old
                                .and_then(|i| block.emphasis.get(&i))
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            query,
                        };
                        let right = CellSpec {
                            source: pair.new.map(|i| &hunk.lines[i]),
                            content: None,
                            old: false,
                            colors: &visuals.new,
                            emphasis: pair
                                .new
                                .and_then(|i| block.emphasis.get(&i))
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            query,
                        };
                        let remaining = limit.map(|n| n.saturating_sub(document.rows.len()));
                        append_split(
                            &mut document,
                            change,
                            &path,
                            left,
                            right,
                            width,
                            palette,
                            remaining,
                            cancel,
                        )?;
                    }
                } else {
                    for (index, line) in
                        hunk.lines.iter().enumerate().take(block.end).skip(position)
                    {
                        if reached_limit(&mut document, limit) {
                            return Ok(document);
                        }
                        let colors = if line.kind == LineKind::Removed {
                            &visuals.old
                        } else {
                            &visuals.new
                        };
                        let emphasis = block.emphasis.get(&index).map(Vec::as_slice).unwrap_or(&[]);
                        let remaining = limit.map(|n| n.saturating_sub(document.rows.len()));
                        append_unified(
                            &mut document,
                            change,
                            &path,
                            line,
                            colors,
                            emphasis,
                            query,
                            width,
                            palette,
                            remaining,
                            cancel,
                        )?;
                    }
                }
                position = block.end;
            } else {
                let remaining = limit.map(|n| n.saturating_sub(document.rows.len()));
                if split && source.kind == LineKind::Context {
                    let left = CellSpec {
                        source: Some(source),
                        content: old_context.at(source.old),
                        old: true,
                        colors: &visuals.old,
                        emphasis: &[],
                        query: "",
                    };
                    let right = CellSpec {
                        source: Some(source),
                        content: new_context.at(source.new),
                        old: false,
                        colors: &visuals.new,
                        emphasis: &[],
                        query,
                    };
                    append_split(
                        &mut document,
                        change,
                        &path,
                        left,
                        right,
                        width,
                        palette,
                        remaining,
                        cancel,
                    )?;
                } else {
                    append_unified(
                        &mut document,
                        change,
                        &path,
                        source,
                        &visuals.new,
                        &[],
                        query,
                        width,
                        palette,
                        remaining,
                        cancel,
                    )?;
                }
                position += 1;
            }
        }
    }
    if reached_limit(&mut document, limit) {
        return Ok(document);
    }
    push_header(&mut document, "", palette.base());
    Ok(document)
}

fn reached_limit(document: &mut Document, limit: Option<usize>) -> bool {
    if limit.is_some_and(|limit| document.rows.len() >= limit) {
        document.partial = true;
        true
    } else {
        false
    }
}
