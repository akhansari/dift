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
    render::{Palette, worker::RenderWorker},
    state::{Effects, PromptKind, State},
};
use ratatui::{
    Frame,
    crossterm::{
        event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
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
    sync::{atomic::Ordering, mpsc},
    time::Duration,
};
use unicode_width::UnicodeWidthStr;

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
    let mut render_needed = true;
    let mut needs_draw = true;
    let mut last_degraded = degraded.load(Ordering::Relaxed);
    let mut viewport = (0usize, 1usize);
    state.collection_started();
    collecting.request(state.loaded().clone())?;
    loop {
        let degraded_now = degraded.load(Ordering::Relaxed);
        needs_draw |= degraded_now != last_degraded;
        last_degraded = degraded_now;
        while let Some(update) = worker_update(collecting.updates.try_recv(), "Git collection")? {
            needs_draw = true;
            match update {
                collect::Update::Started => state.collection_started(),
                collect::Update::Finished(result) => {
                    render_needed |= state.collection_finished(result).render;
                }
            }
        }
        let size = terminal.size()?;
        let (_, diff) = panes(
            Rect::new(0, 1, size.width, size.height.saturating_sub(3)),
            state.sidebar(),
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
            needs_draw = true;
            render_worker.request(state.render_request(viewport.0, viewport.1))?;
            render_needed = false;
        }
        while let Some(result) = worker_update(render_worker.try_recv(), "Rendering")? {
            needs_draw |= state.apply_render(result, viewport.1);
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
                    state.help_scroll(),
                )
            })?;
            needs_draw = false;
        }
        if !event::poll(Duration::from_millis(33))? {
            continue;
        }
        let effects = match event::read()? {
            Event::Resize(_, _) => Effects {
                redraw: true,
                render: true,
                ..Effects::default()
            },
            Event::Paste(text) => state.paste(&text),
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                state.handle_key(key, &config.bindings, viewport.1)
            }
            _ => Effects::default(),
        };
        if effects.quit {
            break;
        }
        needs_draw |= effects.redraw;
        render_needed |= effects.render;
        if effects.collect {
            collecting.request(state.loaded().clone())?;
        }
    }
    Ok(())
}

fn worker_update<T>(result: Result<T, mpsc::TryRecvError>, worker: &str) -> io::Result<Option<T>> {
    match result {
        Ok(update) => Ok(Some(update)),
        Err(mpsc::TryRecvError::Empty) => Ok(None),
        Err(mpsc::TryRecvError::Disconnected) => {
            Err(io::Error::other(format!("{worker} worker disconnected")))
        }
    }
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
        state.snapshot().branch
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
    let (tree_area, diff_area) = panes(layout[1], state.sidebar());
    if tree_area.width > 0 {
        let rows = state.tree(false);
        list_state.select(rows.iter().position(|r| r.path == state.selection()));
        let items: Vec<_> = rows
            .iter()
            .map(|row| {
                let prefix = if row.directory {
                    if state.is_collapsed(&row.path) {
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
                    .border_style(Style::default().fg(if state.focus() == Scope::Tree {
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
    let scope = if state.selection().as_os_str().is_empty() {
        "Repository".into()
    } else {
        display_path(state.selection())
    };
    let mode = if state.split() && diff_area.width >= 34 {
        "Split"
    } else {
        "Unified"
    };
    let diff_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " {scope} · {mode}{} ",
            if state.rendering() {
                " · updating"
            } else {
                ""
            }
        ))
        .border_style(Style::default().fg(if state.focus() == Scope::Diff {
            palette.accent
        } else {
            palette.muted
        }));
    let inner = diff_block.inner(diff_area);
    frame.render_widget(diff_block, diff_area);
    let rows: Vec<Line<'_>> = state
        .document()
        .rows
        .iter()
        .skip(state.scroll())
        .take(inner.height as usize)
        .map(|r| r.line.clone())
        .collect();
    frame.render_widget(Paragraph::new(rows).style(palette.base()), inner);
    let max_scroll = state
        .document()
        .rows
        .len()
        .saturating_sub(inner.height as usize);
    if !inner.is_empty() && max_scroll > 0 {
        // Ratatui counts scroll positions, including the last full viewport.
        let mut scrollbar_state = ScrollbarState::new(max_scroll + 1)
            .position(state.scroll())
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
    let status = if let Some(error) = state.error() {
        format!(" Error: {error}")
    } else {
        let activity = if state.collecting() {
            "Refreshing"
        } else if state.rendering() {
            "Rendering"
        } else if !watching {
            "Snapshot"
        } else if degraded {
            "Polling"
        } else {
            "Watching"
        };
        let filter = if state.filter().is_empty() {
            String::new()
        } else {
            format!(" · filter: {}", state.filter())
        };
        let search = if state.search().is_empty() {
            String::new()
        } else {
            format!(
                " · /{}: {}{} matches",
                state.search(),
                state.document().hits.len(),
                if state.rendering() { " (updating)" } else { "" }
            )
        };
        let excluded = if state.document().excluded == 0 {
            String::new()
        } else {
            format!(" · {} oversized excluded", state.document().excluded)
        };
        format!(
            " {activity}{filter}{search}{excluded} · {}/{}",
            state.scroll() + 1,
            state.document().rows.len()
        )
    };
    frame.render_widget(
        Paragraph::new(status).style(palette.base().fg(if state.error().is_some() {
            palette.removed_emphasis
        } else {
            palette.muted
        })),
        layout[2],
    );
    if let Some(prompt) = state.prompt() {
        let label = if prompt.kind() == PromptKind::Filter {
            "Filter › "
        } else {
            "Search › "
        };
        let accept = config.bindings.keys(Scope::Input, Action::Accept);
        let cancel = config.bindings.keys(Scope::Input, Action::Cancel);
        let suffix = format!("  ({accept}: keep · {cancel}: cancel)");
        let available = layout[3].width.saturating_sub(label.len() as u16 + 1) as usize;
        let mut start = 0;
        while UnicodeWidthStr::width(&prompt.editor().text()[start..prompt.editor().cursor()])
            >= available.max(1)
        {
            start += prompt.editor().text()[start..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
        }
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label, palette.base().fg(palette.accent)),
                Span::raw(prompt.editor().text()[start..].to_owned()),
                Span::styled(suffix, palette.base().fg(palette.muted)),
            ]))
            .style(palette.base()),
            layout[3],
        );
        let x = layout[3].x
            + (UnicodeWidthStr::width(label)
                + UnicodeWidthStr::width(&prompt.editor().text()[start..prompt.editor().cursor()]))
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
    if state.help() {
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
