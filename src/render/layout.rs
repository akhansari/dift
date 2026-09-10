use super::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn push_header(document: &mut Document, text: &str, style: Style) {
    let text = crate::text::escape_controls(text.chars());
    document.rows.push(Arc::new(Row {
        line: Line::styled(text, style),
        anchors: Vec::new(),
        code: Vec::new(),
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
    hits: Vec<(usize, Range<usize>)>,
    width: usize,
    code_start: usize,
    source: Vec<SourceCell>,
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

/// Inputs shared by every source row in one file layout.
#[derive(Clone, Copy)]
struct LayoutContext<'a> {
    category: Category,
    path: &'a Arc<PathBuf>,
    width: usize,
    query: &'a str,
    show_line_numbers: bool,
    palette: Palette,
    cancel: &'a Cancellation,
}

fn cells(
    context: &LayoutContext<'_>,
    spec: CellSpec<'_>,
    width: usize,
    unified: bool,
    limit: Option<usize>,
) -> Result<Vec<CellRow>, Cancelled> {
    let LayoutContext {
        category,
        path,
        palette,
        cancel,
        ..
    } = *context;
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
    let prefix = if !context.show_line_numbers {
        format!("{sign} ")
    } else if unified && width >= 2 * digits + 10 {
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
            category,
            old: spec.old,
            number,
            byte,
            text: Arc::clone(&source_text),
        }),
        hits: Vec::new(),
        width: prefix_width,
        code_start: prefix_width,
        source: Vec::new(),
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
        for glyph in text.chars() {
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
            for (index, _) in matches
                .iter()
                .enumerate()
                .filter(|(_, range)| range.start < end && range.end > byte)
            {
                let start = if glyph_width == 0 {
                    row.source
                        .last()
                        .map_or(row.width, |cell| cell.columns.start)
                } else {
                    row.width
                };
                if let Some((_, columns)) = row.hits.iter_mut().find(|(hit, _)| *hit == index) {
                    columns.end = row.width + glyph_width;
                } else {
                    row.hits.push((index, start..row.width + glyph_width));
                }
            }
            if let Some(previous) = row.spans.last_mut().filter(|s| s.style == style) {
                previous.content.to_mut().push(glyph);
            } else {
                row.spans.push(Span::styled(glyph.to_string(), style));
            }
            if let Some(previous) = row
                .source
                .last_mut()
                .filter(|previous| glyph_width == 0 || previous.bytes == (byte..end))
            {
                previous.columns.end = row.width + glyph_width;
                previous.bytes.end = end;
            } else {
                row.source.push(SourceCell {
                    columns: row.width..row.width + glyph_width,
                    bytes: byte..end,
                });
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
                    hits: Vec::new(),
                    width,
                    code_start: width,
                    source: Vec::new(),
                }),
        );
    }
    Ok(rows)
}

/// Keep one navigation entry per occurrence while collecting its wrapped fragments.
fn append_hits(
    document: &mut Document,
    hits: Vec<(usize, Range<usize>)>,
    indices: &mut std::collections::BTreeMap<usize, usize>,
    column_offset: usize,
) {
    for (occurrence, columns) in hits {
        let index = *indices.entry(occurrence).or_insert_with(|| {
            let index = document.hits.len();
            document.hits.push(Hit {
                row: document.rows.len(),
                fragments: Vec::new(),
            });
            index
        });
        document.hits[index].fragments.push(HitFragment {
            row: document.rows.len(),
            columns: column_offset + columns.start..column_offset + columns.end,
        });
    }
}

fn append_unified(
    context: &LayoutContext<'_>,
    document: &mut Document,
    source: &SourceLine,
    colors: &SyntaxColors,
    emphasis: &[Range<usize>],
    limit: Option<usize>,
) -> Result<(), Cancelled> {
    let LayoutContext {
        width,
        query,
        cancel,
        ..
    } = *context;
    let old = source.new.is_none();
    let mut hits = std::collections::BTreeMap::new();
    for row in cells(
        context,
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
        limit,
    )? {
        cancel.check()?;
        append_hits(document, row.hits, &mut hits, 0);
        document.rows.push(Arc::new(Row {
            code: row
                .anchor
                .as_ref()
                .map(|_| CodeRegion {
                    columns: row.code_start..row.width,
                    anchor: 0,
                    side: None,
                    source: row.source,
                })
                .into_iter()
                .collect(),
            line: Line::from(row.spans),
            anchors: row.anchor.into_iter().collect(),
        }));
    }
    Ok(())
}

fn append_split(
    context: &LayoutContext<'_>,
    document: &mut Document,
    left: CellSpec<'_>,
    right: CellSpec<'_>,
    limit: Option<usize>,
) -> Result<(), Cancelled> {
    let LayoutContext {
        width,
        palette,
        cancel,
        ..
    } = *context;
    let a_width = (width - 1) / 2;
    let b_width = width - 1 - a_width;
    let a = cells(context, left, a_width, false, limit)?;
    let b = cells(context, right, b_width, false, limit)?;
    let count = a.len().max(b.len());
    let mut a = a.into_iter();
    let mut b = b.into_iter();
    let mut left_hits = std::collections::BTreeMap::new();
    let mut right_hits = std::collections::BTreeMap::new();
    for _ in 0..count {
        cancel.check()?;
        let mut spans = Vec::new();
        let mut anchors = Vec::new();
        let mut code = Vec::new();
        if let Some(row) = a.next() {
            if row.anchor.is_some() {
                code.push(CodeRegion {
                    columns: row.code_start..row.width,
                    anchor: anchors.len(),
                    side: Some(DiffSide::Old),
                    source: row.source,
                });
            }
            spans.extend(row.spans);
            anchors.extend(row.anchor);
            append_hits(document, row.hits, &mut left_hits, 0);
        } else {
            spans.push(Span::styled(" ".repeat(a_width), palette.base()));
        }
        spans.push(Span::styled("│", palette.base().fg(palette.muted)));
        if let Some(row) = b.next() {
            if row.anchor.is_some() {
                code.push(CodeRegion {
                    columns: a_width + 1 + row.code_start..a_width + 1 + row.width,
                    anchor: anchors.len(),
                    side: Some(DiffSide::New),
                    source: row
                        .source
                        .into_iter()
                        .map(|cell| SourceCell {
                            columns: a_width + 1 + cell.columns.start
                                ..a_width + 1 + cell.columns.end,
                            bytes: cell.bytes,
                        })
                        .collect(),
                });
            }
            spans.extend(row.spans);
            anchors.extend(row.anchor);
            append_hits(document, row.hits, &mut right_hits, a_width + 1);
        } else {
            spans.push(Span::styled(" ".repeat(b_width), palette.base()));
        }
        document.rows.push(Arc::new(Row {
            line: Line::from(spans),
            anchors,
            code,
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
    let split = spec.split && width >= MIN_SPLIT_WIDTH;
    let mut old_context = ContextSource::new(change.old_source.as_deref());
    let mut new_context = ContextSource::new(change.new_source.as_deref());
    let query = spec.query;
    let palette = Palette::new(config.light);
    let mut document = Document::default();
    let path = Arc::new(change.path.clone());
    let context = LayoutContext {
        category: change.category,
        path: &path,
        width,
        query,
        show_line_numbers: spec.show_line_numbers,
        palette,
        cancel,
    };
    document.rows.push(Arc::new(Row {
        line: header::line(change, width, palette),
        anchors: Vec::new(),
        code: Vec::new(),
    }));
    push_header(
        &mut document,
        &"─".repeat(width),
        palette.base().fg(palette.muted),
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
        finish_hunk(&mut document);
        document.hunk_sections.push(HunkSection {
            rows: document.rows.len()..document.rows.len(),
            first_change_line: first_change_line(hunk),
        });
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
                        append_split(&context, &mut document, left, right, remaining)?;
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
                        append_unified(&context, &mut document, line, colors, emphasis, remaining)?;
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
                    append_split(&context, &mut document, left, right, remaining)?;
                } else {
                    append_unified(
                        &context,
                        &mut document,
                        source,
                        &visuals.new,
                        &[],
                        remaining,
                    )?;
                }
                position += 1;
            }
        }
    }
    if reached_limit(&mut document, limit) {
        return Ok(document);
    }
    finish_hunk(&mut document);
    push_header(&mut document, "", palette.base());
    Ok(document)
}

fn reached_limit(document: &mut Document, limit: Option<usize>) -> bool {
    if limit.is_some_and(|limit| document.rows.len() >= limit) {
        finish_hunk(document);
        document.partial = true;
        true
    } else {
        false
    }
}

fn finish_hunk(document: &mut Document) {
    if let Some(hunk) = document.hunk_sections.last_mut() {
        hunk.rows.end = document.rows.len();
    }
}

fn first_change_line(hunk: &crate::model::Hunk) -> Option<usize> {
    let first = hunk
        .lines
        .iter()
        .position(|line| line.kind != LineKind::Context)?;
    Some(
        hunk.lines[first..]
            .iter()
            .find_map(|line| line.new)
            .or_else(|| hunk.lines[..first].iter().rev().find_map(|line| line.new))
            .unwrap_or(1)
            .max(1),
    )
}
