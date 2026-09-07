use crate::{
    config::{Config, Icons},
    input::{Action, Bindings, Scope},
    render::Palette,
    state::{Effects, State, TreeRow},
    text::name_spans,
};
use ratatui::{
    Frame,
    crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
};
use std::{path::PathBuf, time::Instant};
use unicode_width::UnicodeWidthStr;

/// Tree scrolling is independent of both the selected path and the diff viewport.
#[derive(Default)]
pub(super) struct Viewport {
    offset: usize,
    selection: PathBuf,
    reveal_selection: bool,
    find_query: Option<String>,
}

impl Viewport {
    fn reconcile(&mut self, state: &State, height: usize) {
        let rows = state.tree(false);
        let maximum = rows.len().saturating_sub(height);
        self.offset = self.offset.min(maximum);
        if height == 0 {
            return;
        }
        let find_query = state
            .prompt()
            .filter(|prompt| prompt.kind() == crate::state::PromptKind::Find)
            .map(|prompt| prompt.editor().text().to_owned());
        let find_edited = self.find_query != find_query;
        self.find_query = find_query;
        if (self.reveal_selection || self.selection != state.selection() || find_edited)
            && let Some(index) = rows.iter().position(|row| row.path == state.selection())
        {
            let padding = 2.min(height.saturating_sub(1) / 2);
            if index < self.offset + padding {
                self.offset = index.saturating_sub(padding);
            } else if index >= self.offset + height - padding {
                self.offset = (index + padding + 1).saturating_sub(height).min(maximum);
            }
        }
        state.selection().clone_into(&mut self.selection);
        self.reveal_selection = false;
    }

    pub(super) fn handle_key(
        &mut self,
        state: &mut State,
        key: KeyEvent,
        bindings: &Bindings,
        height: usize,
        now: Instant,
    ) -> Effects {
        if state.prompt().is_none() && !state.help() {
            let action = bindings.action(state.focus(), key);
            self.reveal_selection |=
                matches!(action, Some(Action::NextFile | Action::PreviousFile))
                    || (state.focus() == Scope::Tree
                        && matches!(
                            action,
                            Some(
                                Action::Up
                                    | Action::Down
                                    | Action::Left
                                    | Action::Right
                                    | Action::First
                                    | Action::Last
                                    | Action::Accept
                            )
                        ));
        }
        state.handle_key(key, bindings, height, now)
    }

    pub(super) fn handle_mouse(
        &mut self,
        state: &mut State,
        config: &Config,
        area: Rect,
        mouse: MouseEvent,
        now: Instant,
    ) -> Effects {
        let inner = content_area(area);
        if !state.sidebar()
            || state.help()
            || state.prompt().is_some()
            || !inner.contains((mouse.column, mouse.row).into())
            || !mouse.modifiers.is_empty()
        {
            return Effects::default();
        }
        self.reconcile(state, inner.height as usize);
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let maximum = state
                    .tree(false)
                    .len()
                    .saturating_sub(inner.height as usize);
                let offset = if mouse.kind == MouseEventKind::ScrollUp {
                    self.offset.saturating_sub(3)
                } else {
                    self.offset.saturating_add(3).min(maximum)
                };
                let redraw = self.offset != offset;
                self.offset = offset;
                Effects {
                    redraw,
                    ..Effects::default()
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let index = self.offset + usize::from(mouse.row - inner.y);
                let Some(row) = state.tree(false).get(index) else {
                    return Effects::default();
                };
                let layout =
                    RowLayout::new(row, inner.width.saturating_sub(2) as usize, state, config);
                let column = usize::from(mouse.column - inner.x);
                let toggle_column = 1 + layout.indent_levels * 2;
                let toggle = row.directory
                    && layout.prefix_width > 0
                    && (toggle_column..toggle_column + layout.prefix.width()).contains(&column);
                let effects = if toggle {
                    state.toggle_tree_directory(index)
                } else {
                    state.select_tree_row(index, now)
                };
                // A click operates on the viewport the user is looking at.
                state.selection().clone_into(&mut self.selection);
                effects
            }
            _ => Effects::default(),
        }
    }
}

fn content_area(area: Rect) -> Rect {
    Block::default().borders(Borders::RIGHT).inner(area)
}

/// Draw the tree and report whether its shared pane divider is visible.
pub(super) fn draw(
    frame: &mut Frame,
    state: &State,
    config: &Config,
    area: Rect,
    viewport: &mut Viewport,
) -> bool {
    let palette = Palette::new(config.light);
    let focused = state.focus() == Scope::Tree;
    let count = state.matching_paths().len();
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(palette.base().fg(palette.muted));
    let inner = content_area(area);
    viewport.reconcile(state, inner.height as usize);
    let rows = state.tree(false);
    let max_scroll = rows.len().saturating_sub(inner.height as usize);
    let scrollbar_visible = max_scroll > 0 && !inner.is_empty();
    if !scrollbar_visible {
        frame.render_widget(block, area);
    }
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
    let visible_selection = selection
        .and_then(|index| index.checked_sub(viewport.offset))
        .filter(|index| *index < inner.height as usize);
    let mut list_state = ListState::default().with_selected(visible_selection);
    let width = inner.width.saturating_sub(2) as usize; // selection gutter + right padding
    let items = rows
        .iter()
        .skip(viewport.offset)
        .take(inner.height as usize)
        .map(|row| {
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
        palette.focused
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
        .highlight_style(Style::default().bg(selected_background));
    frame.render_stateful_widget(list, inner, &mut list_state);
    // Apply after List's selected-row styling so external edits remain visible
    // on the selected file. Foregrounds, badges, and the focus marker survive.
    for (index, row) in rows
        .iter()
        .skip(viewport.offset)
        .take(inner.height as usize)
        .enumerate()
    {
        let intensity = state.activity_intensity(&row.path, row.directory);
        if intensity > 0.0 {
            let background = if row.path == state.selection() {
                selected_background
            } else if config.light {
                Color::Rgb(255, 255, 255)
            } else {
                Color::Rgb(20, 23, 30)
            };
            let amber = if config.light {
                Color::Rgb(248, 228, 190)
            } else {
                Color::Rgb(65, 49, 27)
            };
            let color = blend(background, amber, intensity);
            let y = inner.y + index as u16;
            for x in inner.x..inner.right() {
                frame.buffer_mut()[(x, y)].set_bg(color);
            }
        }
    }
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
    if scrollbar_visible {
        let mut scroll = ScrollbarState::new(max_scroll + 1)
            .position(viewport.offset)
            .viewport_content_length(inner.height as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("┃")
                .track_style(palette.base().fg(palette.muted))
                .thumb_style(palette.base().fg(palette.accent)),
            Rect {
                x: area.x,
                width: area.width,
                ..inner
            },
            &mut scroll,
        );
    }
    !scrollbar_visible
}

fn blend(background: Color, tint: Color, intensity: f32) -> Color {
    match (background, tint) {
        (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) => {
            let channel = |base: u8, target: u8| {
                (f32::from(base) + (f32::from(target) - f32::from(base)) * intensity).round() as u8
            };
            Color::Rgb(channel(r, tr), channel(g, tg), channel(b, tb))
        }
        _ => background,
    }
}

/// Shared cell measurements keep folder hit targets aligned with rendered glyphs.
struct RowLayout {
    count: Option<String>,
    available: usize,
    indent_levels: usize,
    prefix: &'static str,
    prefix_width: usize,
}

impl RowLayout {
    fn new(row: &TreeRow, width: usize, state: &State, config: &Config) -> Self {
        let count = row.directory.then(|| {
            if row.depth == 0 && !state.filter().is_empty() {
                format!("{}/{}", row.count, state.visible_files())
            } else {
                row.count.to_string()
            }
        });
        let metadata_width = count.as_ref().map_or(0, |count| count.width())
            + usize::from(count.is_some() && !row.badges.is_empty())
            + row.badges.width();
        let available = width.saturating_sub(metadata_width + usize::from(metadata_width > 0));
        let collapsed = state.is_collapsed(&row.path);
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
        // Prefer at least eight columns of basename over deeply nested indentation.
        let indent_levels = row
            .depth
            .min(available.saturating_sub(2 + row.name.width().min(8)) / 2);
        Self {
            count,
            available,
            indent_levels,
            prefix,
            prefix_width: if available > 2 { 2 } else { 0 },
        }
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
    let layout = RowLayout::new(row, width, state, config);
    let selected = state.focus() == Scope::Tree && row.path == state.selection();
    let muted = Style::default().fg(palette.muted);
    let mut metadata = Vec::new();
    if let Some(count) = layout.count {
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
    let mut spans = Vec::new();
    for depth in row.depth - layout.indent_levels..row.depth {
        spans.push(Span::styled(
            if guide_depth == Some(depth) {
                "│ "
            } else {
                "  "
            },
            muted,
        ));
    }
    if layout.prefix_width > 0 {
        spans.push(Span::styled(layout.prefix, muted));
        spans.push(Span::raw(" "));
    }
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
        layout
            .available
            .saturating_sub(layout.indent_levels * 2 + layout.prefix_width),
        name_style,
    ));
    let used: usize = spans.iter().map(Span::width).sum();
    let metadata_width: usize = metadata.iter().map(Span::width).sum();
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
