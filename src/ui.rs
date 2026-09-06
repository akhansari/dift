mod collect;
#[cfg(test)]
mod fixtures;
mod watch;

use crate::{
    AppResult,
    config::Config,
    git::{Mode, Repository},
    input::{Action, Scope},
    model::display_path,
    render::{
        Palette,
        worker::{RenderRequest, RenderResult, RenderWorker},
    },
    state::{PromptKind, State},
};
use ratatui::{
    Frame,
    crossterm::{
        event::{
            self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
            KeyModifiers,
        },
        execute,
    },
    layout::{Constraint, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
struct Displayed {
    selection: PathBuf,
    query: String,
    pending_search_jump: bool,
}

impl Displayed {
    fn apply(&mut self, state: &mut State, result: RenderResult, height: usize) -> bool {
        if result.generation != state.generation
            || (result.document.partial && self.selection == state.selection && state.scroll > 0)
        {
            // Scrolling can happen after requesting a prefix preview. Keep the
            // current source position until a complete document is available.
            return false;
        }
        state.scroll = if self.selection == state.selection {
            result
                .document
                .reanchored_scroll(&state.document, state.scroll)
        } else {
            0
        }
        .min(result.document.rows.len().saturating_sub(height));
        state.document = result.document;
        state.rendering = !result.finished;
        if self.query != state.search || self.selection != state.selection {
            state.match_index = None;
        } else if let Some(index) = state.match_index {
            state.match_index = (!state.document.hits.is_empty())
                .then_some(index.min(state.document.hits.len().saturating_sub(1)));
        }
        if self.pending_search_jump && !state.search.is_empty() && !state.document.hits.is_empty() {
            state.match_index = None;
            state.jump_match(true, height);
            self.pending_search_jump = false;
        }
        if result.finished || state.search.is_empty() {
            self.pending_search_jump = false;
        }
        self.selection.clone_from(&state.selection);
        self.query.clone_from(&state.search);
        true
    }
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}
pub fn run(repo: Repository, mode: Mode, config: Config) -> AppResult<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "Interactive viewing requires a terminal. Use --help for invocation examples.",
        )
        .into());
    }
    let render_worker = RenderWorker::spawn(config.clone())?;
    let watching = matches!(mode, Mode::Watch);
    let collecting =
        collect::Worker::spawn(repo.clone(), mode.clone(), config.limits, config.reconcile)?;
    let degraded = &collecting.degraded;
    let mut terminal = ratatui::try_init()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnableBracketedPaste)?;
    let mut state = State::new(config.split);
    let mut list_state = ListState::default();
    let mut help_scroll = 0usize;
    let mut render_needed = true;
    let mut needs_draw = true;
    let mut last_degraded = degraded.load(Ordering::Relaxed);
    let mut viewport = (0usize, 1usize);
    let mut displayed = Displayed::default();
    state.collecting = true;
    collecting.request(state.loaded.clone())?;
    loop {
        let degraded_now = degraded.load(Ordering::Relaxed);
        needs_draw |= degraded_now != last_degraded;
        last_degraded = degraded_now;
        while let Ok(update) = collecting.updates.try_recv() {
            needs_draw = true;
            let result = match update {
                collect::Update::Started => {
                    state.collecting = true;
                    continue;
                }
                collect::Update::Finished(result) => result,
            };
            state.collecting = false;
            match result {
                Ok(snapshot) => {
                    if state.snapshot.as_ref() != &snapshot {
                        state.update_snapshot(snapshot);
                        render_needed = true;
                    }
                    state.error = None;
                }
                Err(error) => state.error = Some(error.to_string()),
            }
        }
        let size = terminal.size()?;
        let (_, diff) = panes(
            Rect::new(0, 1, size.width, size.height.saturating_sub(3)),
            state.sidebar,
        );
        let new_viewport = (
            diff.width.saturating_sub(2) as usize,
            diff.height.saturating_sub(2).max(1) as usize,
        );
        if viewport != new_viewport {
            viewport = new_viewport;
            render_needed = true;
        }
        if render_needed {
            state.generation += 1;
            state.rendering = true;
            needs_draw = true;
            displayed.pending_search_jump |=
                displayed.query != state.search && !state.search.is_empty();
            render_worker.request(RenderRequest {
                generation: state.generation,
                snapshot: Arc::clone(&state.snapshot),
                selected: state.selected_changes(),
                width: viewport.0,
                split: state.split,
                query: state.search.clone(),
                // A prefix-only preview cannot preserve a source anchor farther
                // down the current document. Reflow/refresh there uses a full
                // basic diff before replacing the existing view.
                preview_rows: if displayed.selection == state.selection && state.scroll > 0 {
                    0
                } else {
                    viewport.1.saturating_mul(2).max(16)
                },
            })?;
            render_needed = false;
        }
        while let Ok(result) = render_worker.try_recv() {
            needs_draw |= displayed.apply(&mut state, result, viewport.1);
        }
        if needs_draw {
            terminal.draw(|frame| {
                draw(
                    frame,
                    &state,
                    &config,
                    &repo,
                    watching,
                    degraded.load(Ordering::Relaxed),
                    &mut list_state,
                    help_scroll,
                )
            })?;
            needs_draw = false;
        }
        if !event::poll(Duration::from_millis(33))? {
            continue;
        }
        match event::read()? {
            Event::Resize(_, _) => {
                render_needed = true;
                needs_draw = true;
            }
            Event::Paste(text) => {
                if let Some(prompt) = &mut state.prompt {
                    needs_draw = true;
                    prompt.editor.insert(&text);
                    state.sync_prompt();
                    render_needed = true;
                }
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let scope = if state.prompt.is_some() {
                    Scope::Input
                } else {
                    state.focus
                };
                let action = config.bindings.action(scope, key);
                if action == Some(Action::Quit) {
                    break;
                }
                if state.prompt.is_some() {
                    needs_draw = true;
                    let before = (
                        state.filter.clone(),
                        state.search.clone(),
                        state.selection.clone(),
                    );
                    match action {
                        Some(Action::Accept) => state.finish_prompt(true),
                        Some(Action::Cancel) => state.finish_prompt(false),
                        Some(action) => state.input_action(action),
                        None if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            if let KeyCode::Char(ch) = key.code {
                                if let Some(prompt) = &mut state.prompt {
                                    prompt.editor.insert(&ch.to_string());
                                }
                                state.sync_prompt();
                            }
                        }
                        _ => {}
                    }
                    render_needed |= before
                        != (
                            state.filter.clone(),
                            state.search.clone(),
                            state.selection.clone(),
                        );
                    continue;
                }
                needs_draw |= action.is_some();
                if state.help {
                    match action {
                        Some(Action::Cancel | Action::Help) => state.help = false,
                        Some(Action::Down) => help_scroll += 1,
                        Some(Action::Up) => help_scroll = help_scroll.saturating_sub(1),
                        Some(Action::PageDown) => help_scroll += viewport.1 / 2,
                        Some(Action::PageUp) => {
                            help_scroll = help_scroll.saturating_sub(viewport.1 / 2)
                        }
                        _ => {}
                    }
                    continue;
                }
                let selection_before = state.selection.clone();
                match action {
                    Some(Action::Sidebar) => {
                        state.sidebar = !state.sidebar;
                        state.focus = if state.sidebar {
                            Scope::Tree
                        } else {
                            Scope::Diff
                        };
                        render_needed = true;
                    }
                    Some(Action::Layout) => {
                        state.split = !state.split;
                        render_needed = true;
                    }
                    Some(Action::Focus) => {
                        if state.sidebar {
                            state.focus = if state.focus == Scope::Tree {
                                Scope::Diff
                            } else {
                                Scope::Tree
                            };
                        }
                    }
                    Some(Action::NextFile) => state.next_file(true),
                    Some(Action::PreviousFile) => state.next_file(false),
                    Some(Action::Down) => {
                        if state.focus == Scope::Tree {
                            state.move_tree(true);
                        } else {
                            state.scroll_by(1, true, viewport.1);
                        }
                    }
                    Some(Action::Up) => {
                        if state.focus == Scope::Tree {
                            state.move_tree(false);
                        } else {
                            state.scroll_by(1, false, viewport.1);
                        }
                    }
                    Some(Action::Left) => state.collapse(),
                    Some(Action::Right) => state.expand(),
                    Some(Action::HalfDown) => {
                        state.scroll_by((viewport.1 / 2).max(1), true, viewport.1)
                    }
                    Some(Action::HalfUp) => {
                        state.scroll_by((viewport.1 / 2).max(1), false, viewport.1)
                    }
                    Some(Action::PageDown) => state.scroll_by(viewport.1, true, viewport.1),
                    Some(Action::PageUp) => state.scroll_by(viewport.1, false, viewport.1),
                    Some(Action::First) => {
                        if state.focus == Scope::Tree {
                            state.selection.clear();
                        } else {
                            state.scroll = 0;
                        }
                    }
                    Some(Action::Last) => {
                        if state.focus == Scope::Tree {
                            if let Some(row) = state.tree(false).last() {
                                state.selection.clone_from(&row.path);
                            }
                        } else {
                            state.scroll = state.document.rows.len().saturating_sub(viewport.1);
                        }
                    }
                    Some(Action::Filter) => state.begin_prompt(PromptKind::Filter),
                    Some(Action::Search) => state.begin_prompt(PromptKind::Search),
                    Some(Action::NextMatch) => state.jump_match(true, viewport.1),
                    Some(Action::PreviousMatch) => state.jump_match(false, viewport.1),
                    Some(Action::Cancel) => {
                        if !state.search.is_empty() {
                            state.search.clear();
                        } else {
                            state.filter.clear();
                        }
                        render_needed = true;
                    }
                    Some(Action::Accept) => {
                        if state
                            .snapshot
                            .changes
                            .iter()
                            .any(|c| c.path == state.selection && c.oversized)
                        {
                            state.loaded.insert(state.selection.clone());
                            collecting.request(state.loaded.clone())?;
                        } else {
                            state.expand();
                        }
                    }
                    Some(Action::Refresh) => collecting.request(state.loaded.clone())?,
                    Some(Action::Help) => {
                        state.help = true;
                        help_scroll = 0;
                    }
                    _ => {}
                }
                if state.selection != selection_before {
                    render_needed = true;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn panes(body: Rect, sidebar: bool) -> (Rect, Rect) {
    if sidebar && body.width >= 24 {
        let pieces = Layout::horizontal([
            Constraint::Length((body.width / 3).min(38)),
            Constraint::Min(1),
        ])
        .split(body);
        (pieces[0], pieces[1])
    } else {
        (Rect::default(), body)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    frame: &mut Frame,
    state: &State,
    config: &Config,
    repo: &Repository,
    watching: bool,
    degraded: bool,
    list_state: &mut ListState,
    help_scroll: usize,
) {
    let area = frame.area();
    let palette = Palette::new(config.light);
    frame.render_widget(Block::default().style(palette.base()), area);
    if area.width < 12 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("Terminal too small").style(palette.base()),
            area,
        );
        return;
    }
    let layout = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    let title = format!(
        " Δ WATCH  {}  ·  {}",
        repo.root.file_name().unwrap_or_default().to_string_lossy(),
        state.snapshot.branch
    );
    frame.render_widget(
        Paragraph::new(title).style(
            palette
                .base()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        layout[0],
    );
    let (tree_area, diff_area) = panes(layout[1], state.sidebar);
    if tree_area.width > 0 {
        let rows = state.tree(false);
        list_state.select(rows.iter().position(|r| r.path == state.selection));
        let items: Vec<_> = rows
            .iter()
            .map(|row| {
                let prefix = if row.directory {
                    if state.collapsed.contains(&row.path) {
                        "▸"
                    } else {
                        "▾"
                    }
                } else {
                    " "
                };
                let name = if row.path.as_os_str().is_empty() {
                    "/".into()
                } else {
                    display_path(std::path::Path::new(
                        row.path.file_name().unwrap_or_default(),
                    ))
                };
                let indent = "  ".repeat(row.depth.saturating_sub(1));
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{indent}{prefix} {name}"), palette.base()),
                    Span::styled(
                        format!("  {}", row.badges),
                        palette.base().fg(palette.muted),
                    ),
                ]))
            })
            .collect();
        let count = state.matching_paths().len();
        let tree = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Changes · {count} "))
                    .border_style(Style::default().fg(if state.focus == Scope::Tree {
                        palette.accent
                    } else {
                        palette.muted
                    })),
            )
            .highlight_style(
                palette
                    .base()
                    .bg(palette.panel)
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▎");
        frame.render_stateful_widget(tree, tree_area, list_state);
    }
    let scope = if state.selection.as_os_str().is_empty() {
        "Repository".into()
    } else {
        display_path(&state.selection)
    };
    let mode = if state.split && diff_area.width >= 34 {
        "Split"
    } else {
        "Unified"
    };
    let diff_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " {scope} · {mode}{} ",
            if state.rendering { " · updating" } else { "" }
        ))
        .border_style(Style::default().fg(if state.focus == Scope::Diff {
            palette.accent
        } else {
            palette.muted
        }));
    let inner = diff_block.inner(diff_area);
    frame.render_widget(diff_block, diff_area);
    let rows: Vec<Line<'_>> = state
        .document
        .rows
        .iter()
        .skip(state.scroll)
        .take(inner.height as usize)
        .map(|r| r.line.clone())
        .collect();
    frame.render_widget(Paragraph::new(rows).style(palette.base()), inner);
    let max_scroll = state
        .document
        .rows
        .len()
        .saturating_sub(inner.height as usize);
    if !inner.is_empty() && max_scroll > 0 {
        // Ratatui counts scroll positions, including the last full viewport.
        let mut scrollbar_state = ScrollbarState::new(max_scroll + 1)
            .position(state.scroll)
            .viewport_content_length(inner.height as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(palette.base().fg(palette.muted))
                .thumb_style(palette.base().fg(palette.accent)),
            diff_area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
    let status = if let Some(error) = &state.error {
        format!(" Error: {error}")
    } else {
        let activity = if state.collecting {
            "Refreshing"
        } else if state.rendering {
            "Rendering"
        } else if !watching {
            "Snapshot"
        } else if degraded {
            "Polling"
        } else {
            "Watching"
        };
        let filter = if state.filter.is_empty() {
            String::new()
        } else {
            format!(" · filter: {}", state.filter)
        };
        let search = if state.search.is_empty() {
            String::new()
        } else {
            format!(
                " · /{}: {}{} matches",
                state.search,
                state.document.hits.len(),
                if state.rendering { " (updating)" } else { "" }
            )
        };
        let excluded = if state.document.excluded == 0 {
            String::new()
        } else {
            format!(" · {} oversized excluded", state.document.excluded)
        };
        format!(
            " {activity}{filter}{search}{excluded} · {}/{}",
            state.scroll + 1,
            state.document.rows.len()
        )
    };
    frame.render_widget(
        Paragraph::new(status).style(palette.base().fg(if state.error.is_some() {
            palette.removed_emphasis
        } else {
            palette.muted
        })),
        layout[2],
    );
    if let Some(prompt) = &state.prompt {
        let label = if prompt.kind == PromptKind::Filter {
            "Filter › "
        } else {
            "Search › "
        };
        let accept = config.bindings.keys(Scope::Input, Action::Accept);
        let cancel = config.bindings.keys(Scope::Input, Action::Cancel);
        let suffix = format!("  ({accept}: keep · {cancel}: cancel)");
        let available = layout[3].width.saturating_sub(label.len() as u16 + 1) as usize;
        let mut start = 0;
        while UnicodeWidthStr::width(&prompt.editor.text[start..prompt.editor.cursor])
            >= available.max(1)
        {
            start += prompt.editor.text[start..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
        }
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label, palette.base().fg(palette.accent)),
                Span::raw(prompt.editor.text[start..].to_owned()),
                Span::styled(suffix, palette.base().fg(palette.muted)),
            ]))
            .style(palette.base()),
            layout[3],
        );
        let x = layout[3].x
            + (UnicodeWidthStr::width(label)
                + UnicodeWidthStr::width(&prompt.editor.text[start..prompt.editor.cursor]))
                as u16;
        frame.set_cursor_position((x.min(area.right().saturating_sub(1)), layout[3].y));
    } else {
        let keys = |action| config.bindings.keys(Scope::Global, action);
        let footer = format!(
            " {} files  {} filter  {} search  {} layout  {} help  {} quit",
            format_args!("{}/{}", keys(Action::NextFile), keys(Action::PreviousFile)),
            keys(Action::Filter),
            keys(Action::Search),
            keys(Action::Layout),
            keys(Action::Help),
            keys(Action::Quit)
        );
        frame.render_widget(
            Paragraph::new(footer).style(palette.base().fg(palette.muted)),
            layout[3],
        );
    }
    if state.help {
        let width = area.width.saturating_sub(4).min(78);
        let height = area.height.saturating_sub(2);
        let rect = Rect::new(area.x + (area.width - width) / 2, area.y + 1, width, height);
        frame.render_widget(Clear, rect);
        let mut lines = Vec::new();
        for scope in Scope::ALL {
            lines.push(Line::styled(
                format!(" {}", scope.name().to_uppercase()),
                palette
                    .base()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ));
            if let Some(actions) = config.bindings.0.get(&scope) {
                for action in actions.keys() {
                    lines.push(Line::raw(format!(
                        " {:<24} {}",
                        config.bindings.keys(scope, *action),
                        action.name()
                    )));
                }
            }
        }
        let maximum = lines
            .len()
            .saturating_sub(height.saturating_sub(2) as usize);
        let content: Vec<_> = lines.into_iter().skip(help_scroll.min(maximum)).collect();
        frame.render_widget(
            Paragraph::new(content).style(palette.base()).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Keybindings · scroll to explore ")
                    .border_style(Style::default().fg(palette.accent)),
            ),
            rect,
        );
    }
}
#[cfg(test)]
mod tests;
