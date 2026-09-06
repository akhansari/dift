use crate::{
    config::{Config, Icons},
    input::Scope,
    render::Palette,
    state::{PromptKind, State, TreeRow},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
};
use unicode_width::UnicodeWidthStr;

pub(super) fn draw(
    frame: &mut Frame,
    state: &State,
    config: &Config,
    area: Rect,
    list_state: &mut ListState,
) {
    let palette = Palette::new(config.light);
    let focused = state.focus() == Scope::Tree;
    let count = state.matching_paths().len();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused {
            palette.accent
        } else {
            palette.muted
        }));
    let mut inner = block.inner(area);
    frame.render_widget(block, area);
    let editing = state
        .prompt()
        .filter(|prompt| prompt.kind() == PromptKind::Filter);
    if (!state.filter().is_empty() || editing.is_some()) && inner.height > 1 {
        let query = editing.map_or(state.filter(), |prompt| prompt.editor().text());
        let label = format!("Filter: {query}");
        frame.render_widget(
            Paragraph::new(Line::from(name_spans(
                &label,
                &[],
                inner.width as usize,
                Style::default().fg(palette.accent),
            )))
            .style(palette.base()),
            Rect { height: 1, ..inner },
        );
        inner.y += 1;
        inner.height -= 1;
    }
    let rows = state.tree(false);
    let selection = rows.iter().position(|row| row.path == state.selection());
    let active_directory = selection.and_then(|index| {
        let row = &rows[index];
        if row.directory {
            Some((row.path.as_path(), row.depth))
        } else {
            row.path
                .parent()
                .map(|parent| (parent, row.depth.saturating_sub(1)))
        }
    });
    list_state.select(selection);
    let width = inner.width.saturating_sub(2) as usize; // selection gutter + right padding
    let items = rows.iter().map(|row| {
        ListItem::new(row_line(
            row,
            width,
            state,
            config,
            palette,
            active_directory
                .filter(|(path, depth)| row.depth > *depth && row.path.starts_with(path))
                .map(|(_, depth)| depth),
        ))
    });
    let selected_background = if focused {
        if config.light {
            Color::Rgb(218, 229, 247)
        } else {
            Color::Rgb(35, 48, 68)
        }
    } else {
        palette.panel
    };
    let list = List::new(items)
        .style(palette.base())
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_symbol(Span::styled(
            if focused { "▎" } else { " " },
            Style::default().fg(palette.accent),
        ))
        .highlight_style(Style::default().bg(selected_background))
        .scroll_padding(2);
    frame.render_stateful_widget(list, inner, list_state);
    if count == 0 && inner.height > 1 {
        frame.render_widget(
            Paragraph::new(if state.filter().is_empty() {
                "No changes"
            } else {
                "No matching files"
            })
            .style(palette.base().fg(palette.muted)),
            Rect {
                x: inner.x + 1,
                y: inner.y + 1,
                width: inner.width.saturating_sub(1),
                height: 1,
            },
        );
    }
    let max_scroll = rows.len().saturating_sub(inner.height as usize);
    if max_scroll > 0 && !inner.is_empty() {
        let mut scroll = ScrollbarState::new(max_scroll + 1)
            .position(list_state.offset())
            .viewport_content_length(inner.height as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("┃")
                .track_style(palette.base().fg(palette.muted))
                .thumb_style(palette.base().fg(if focused {
                    palette.accent
                } else {
                    palette.muted
                })),
            Rect {
                x: area.x,
                width: area.width,
                ..inner
            },
            &mut scroll,
        );
    }
}

fn row_line<'a>(
    row: &'a TreeRow,
    width: usize,
    state: &State,
    config: &Config,
    palette: Palette,
    guide_depth: Option<usize>,
) -> Line<'a> {
    let collapsed = state.is_collapsed(&row.path);
    let selected = state.focus() == Scope::Tree && row.path == state.selection();
    let muted = Style::default().fg(palette.muted);
    let mut metadata = Vec::new();
    if row.directory {
        let count = if row.depth == 0 && !state.filter().is_empty() {
            format!("{}/{}", row.count, state.visible_files())
        } else {
            row.count.to_string()
        };
        metadata.push(Span::styled(count, muted));
        if !row.badges.is_empty() {
            metadata.push(Span::raw(" "));
        }
    }
    metadata.extend(row.badges.chars().map(|badge| {
        let color = match (badge, config.light) {
            ('U', true) => Color::Rgb(139, 91, 16),
            ('U', false) => Color::Rgb(205, 166, 92),
            ('S', true) => Color::Rgb(39, 115, 69),
            ('S', false) => Color::Rgb(124, 181, 141),
            ('?', true) => Color::Rgb(27, 111, 126),
            ('?', false) => Color::Rgb(112, 180, 194),
            _ => palette.muted,
        };
        Span::styled(badge.to_string(), Style::default().fg(color))
    }));
    let metadata_width: usize = metadata.iter().map(Span::width).sum();
    let available = width.saturating_sub(metadata_width + usize::from(metadata_width > 0));
    let prefix = match config.icons {
        Icons::Nerd => icon(row, collapsed),
        Icons::None if row.directory => {
            if collapsed {
                "▸"
            } else {
                "▾"
            }
        }
        Icons::None => " ",
    };
    let prefix_width = 2;
    // Prefer at least eight columns of basename over deeply nested indentation.
    let indent_levels = row
        .depth
        .min(available.saturating_sub(prefix_width + row.name.width().min(8)) / 2);
    let mut spans = Vec::new();
    for depth in row.depth - indent_levels..row.depth {
        spans.push(Span::styled(
            if guide_depth == Some(depth) {
                "│ "
            } else {
                "  "
            },
            muted,
        ));
    }
    let prefix_width = if available > prefix_width {
        spans.push(Span::styled(prefix, muted));
        spans.push(Span::raw(" "));
        prefix_width
    } else {
        0
    };
    let name_style = Style::default()
        .fg(if selected {
            palette.accent
        } else {
            palette.foreground
        })
        .add_modifier(if selected || row.directory {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });
    spans.extend(name_spans(
        &row.name,
        &row.match_indices,
        available.saturating_sub(indent_levels * 2 + prefix_width),
        name_style,
    ));
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + metadata_width)),
    ));
    spans.extend(metadata);
    Line::from(spans)
}

/// Nerd Fonts glyphs from https://www.nerdfonts.com/cheat-sheet.
fn icon(row: &TreeRow, collapsed: bool) -> &'static str {
    if row.directory {
        return if collapsed { "\u{f07b}" } else { "\u{f07c}" };
    }
    match row
        .path
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("rs") => "\u{e7a8}",
        Some("js" | "mjs" | "cjs") => "\u{e74e}",
        Some("ts" | "mts" | "cts") => "\u{e628}",
        Some("jsx" | "tsx") => "\u{e7ba}",
        Some("py" | "pyi") => "\u{e73c}",
        Some("md" | "mdx" | "markdown") => "\u{e73e}",
        Some("json" | "jsonc" | "toml" | "yaml" | "yml" | "kdl" | "ini" | "conf") => "\u{e615}",
        _ => "\u{f15c}",
    }
}

/// Fit a name without splitting graphemes, keeping the extension where possible.
/// Match indices remain relative to the original name after middle truncation.
fn name_spans<'a>(name: &'a str, matches: &[usize], width: usize, style: Style) -> Vec<Span<'a>> {
    if width == 0 {
        return Vec::new();
    }
    let source = Span::raw(name);
    let mut byte = 0;
    let graphemes: Vec<_> = source
        .styled_graphemes(Style::default())
        .enumerate()
        .map(|(index, grapheme)| {
            let start = byte;
            byte += grapheme.symbol.len();
            (start..byte, index, grapheme.symbol.width())
        })
        .collect();
    let truncated = name.width() > width;
    let (head, tail) = if truncated {
        let remaining = width - 1;
        let extension = name
            .rfind('.')
            .filter(|index| *index > 0)
            .map_or(0, |index| name[index..].width());
        let tail_budget = (remaining / 2)
            .max(extension)
            .min(remaining.saturating_sub(1));
        let mut tail_width = 0;
        let tail = graphemes
            .iter()
            .rev()
            .take_while(|(_, _, cells)| {
                if tail_width + cells > tail_budget {
                    return false;
                }
                tail_width += cells;
                true
            })
            .count();
        let mut head_width = 0;
        let head = graphemes
            .iter()
            .take_while(|(_, _, cells)| {
                if head_width + cells > remaining - tail_width {
                    return false;
                }
                head_width += cells;
                true
            })
            .count();
        (head, graphemes.len() - tail)
    } else {
        (graphemes.len(), graphemes.len())
    };
    let styled = |(bytes, index, _): &(std::ops::Range<usize>, usize, usize)| {
        Span::styled(
            &name[bytes.clone()],
            if matches.binary_search(index).is_ok() {
                style.add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            } else {
                style
            },
        )
    };
    let mut spans: Vec<_> = graphemes[..head].iter().map(styled).collect();
    if truncated {
        spans.push(Span::styled("…", style));
        spans.extend(graphemes[tail..].iter().map(styled));
    }
    spans
}
