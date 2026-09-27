mod activity;
mod clipboard;
mod collect;
mod commands;
mod commit;
mod discard;
mod editor;
mod filters;
#[cfg(test)]
pub(crate) mod fixtures;
mod help;
mod history;
mod modal;
mod selection;
mod stats;
mod terminal;
mod tree;
mod watch;

use crate::{
    AppResult,
    config::Config,
    git::{Mode, Repository},
    input::{Action, Bindings, Scope},
    render::{Palette, worker::RenderWorker},
    state::{Effects, PromptKind, State},
};
use activity::{Activity, ActivityStatus};
use ratatui::{
    Frame,
    crossterm::{
        event::{
            self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
            EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind,
        },
        execute,
        style::Colored,
    },
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use std::{
    io::{self, IsTerminal},
    ops::ControlFlow,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

struct TerminalGuard;
#[derive(Default)]
struct Interaction {
    tree: tree::Viewport,
    selection: selection::Selection,
}

impl Interaction {
    fn handle_key(
        &mut self,
        state: &mut State,
        key: KeyEvent,
        bindings: &Bindings,
        height: usize,
        now: Instant,
    ) -> Effects {
        if state.discard_menu().is_some()
            || state.commit_dialog().is_some()
            || state.filter_menu().is_some()
        {
            self.selection.clear();
            return state.handle_key(key, bindings, height, now);
        }
        if state.prompt().is_none()
            && !state.help()
            && bindings.action(state.focus(), key) == Some(Action::Cancel)
            && self.selection.clear()
        {
            return Effects {
                redraw: true,
                ..Effects::default()
            };
        }
        let mut effects = self.tree.handle_key(state, key, bindings, height, now);
        if effects.render
            || state.help()
            || state.prompt().is_some()
            || state.filter_menu().is_some()
        {
            effects.redraw |= self.selection.clear();
        }
        effects
    }
}

/// The input and selection lifecycle shared by the worktree and revision viewers.
struct ViewerInput<'a> {
    state: &'a mut State,
    interaction: &'a mut Interaction,
}

impl<'a> ViewerInput<'a> {
    fn new(state: &'a mut State, interaction: &'a mut Interaction) -> Self {
        Self { state, interaction }
    }

    fn paste(&mut self, text: &str, now: Instant) -> Effects {
        self.state.paste(text, now)
    }

    fn key(&mut self, key: KeyEvent, bindings: &Bindings, height: usize, now: Instant) -> Effects {
        self.interaction
            .handle_key(self.state, key, bindings, height, now)
    }

    fn mouse(
        &mut self,
        view: &ViewAreas,
        config: &Config,
        mouse: MouseEvent,
        now: Instant,
    ) -> Effects {
        view.handle_mouse(self.state, config, self.interaction, mouse, now)
    }

    fn pending_filter(&mut self, now: Instant) -> Effects {
        self.state.apply_pending_filter(now)
    }

    fn clear_selection(&mut self) -> bool {
        self.interaction.selection.clear()
    }

    fn take_copy(&mut self) -> Option<String> {
        self.interaction.selection.take_copy()
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
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
    let root = repo.root.clone();
    let mut history = history::History::new(repo.clone(), config.clone());
    let collecting = collect::Worker::spawn(repo, mode, config.limits, config.reconcile)?;
    let degraded = &collecting.degraded;
    let mut terminal = ratatui::try_init()?;
    let monochrome = Colored::ansi_color_disabled_memoized();
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
    let mut state = State::with_whitespace(config.split, config.whitespace);
    state.set_comparison(!watching);
    let mut commands = commands::Commands::new(root, watching);
    let mut interaction = Interaction::default();
    let mut render_needed = true;
    let mut needs_draw = true;
    let mut viewport = (0usize, 1usize);
    collecting.request(state.collection_request())?;
    let mut activity = ActivityStatus::new(Activity::current(
        &state,
        watching,
        degraded.load(Ordering::Relaxed),
    ));
    loop {
        needs_draw |= history.tick(Instant::now())?;
        needs_draw |= commands.update_commit(&mut terminal, &mut state, &collecting)?;
        while let Some(update) = worker_update(collecting.updates.try_recv(), "Git collection")? {
            let effects = commands.update(update, &mut state, &collecting)?;
            needs_draw |= effects.redraw;
            render_needed |= effects.render;
        }
        let size = terminal.size()?;
        commands.resize(size)?;
        let full_area = Rect::new(0, 0, size.width, size.height);
        let view = ViewAreas::new(full_area, state.sidebar());
        if state.help() {
            help::Help::new(view.area, &config.bindings, &Palette::new(config.light))
                .sync(&mut state);
        }
        let new_viewport = view.viewport();
        if viewport != new_viewport {
            viewport = new_viewport;
            render_needed = true;
        }
        let historical_view = history
            .detail_state()
            .map(|detail| ViewAreas::new(full_area, detail.sidebar()));
        let history_effects = history.apply_pending_filter(Instant::now())?;
        needs_draw |= history_effects.redraw;
        if let Some(detail) = history.detail_mut() {
            needs_draw |= detail.expire_toast(Instant::now()).redraw;
        }
        if history_effects.render {
            history.request_render()?;
        }
        let effects = ViewerInput::new(&mut state, &mut interaction).pending_filter(Instant::now());
        needs_draw |= effects.redraw;
        render_needed |= effects.render;
        needs_draw |= state.expire_toast(Instant::now()).redraw;
        let activity_redraw = state.advance_activity(Instant::now()).redraw;
        needs_draw |= activity_redraw && !monochrome && state.sidebar() && !state.help();
        if render_needed {
            needs_draw = true;
            render_worker.request(state.render_request(viewport.0, viewport.1))?;
            render_needed = false;
        }
        while let Some(result) = worker_update(render_worker.try_recv(), "Rendering")? {
            needs_draw |= state.apply_render(result, viewport.1);
        }
        if let Some(view) = &historical_view {
            needs_draw |= history.update_render(view.viewport().1)?;
        }
        needs_draw |= activity.update(
            Activity::current(&state, watching, degraded.load(Ordering::Relaxed)),
            Instant::now(),
        );
        if needs_draw {
            terminal.draw(|frame| {
                match history.screen() {
                    Some(history::Screen::List) => history.draw_list(frame),
                    Some(history::Screen::Detail(_)) => history.draw_detail(frame),
                    None => draw(
                        frame,
                        &state,
                        &config,
                        activity.displayed(),
                        &mut interaction,
                        state.help_scroll(),
                    ),
                }
                if monochrome {
                    suppress_colors(frame.buffer_mut());
                }
            })?;
            needs_draw = false;
        }
        let poll_interval = Duration::from_millis(33);
        let now = Instant::now();
        let timeout = state
            .filter_delay(now)
            .into_iter()
            .chain(state.toast_delay(now))
            .chain(activity.delay(now))
            .min()
            .map_or(poll_interval, |delay| delay.min(poll_interval));
        if !event::poll(timeout)? {
            continue;
        }
        let effects = match event::read()? {
            Event::Resize(_, _) => {
                ViewerInput::new(&mut state, &mut interaction).clear_selection();
                history.clear_selection();
                Effects {
                    redraw: true,
                    render: true,
                    ..Effects::default()
                }
            }
            Event::Paste(text) => match history.screen() {
                Some(history::Screen::List) => Effects {
                    redraw: history.paste(&text, Instant::now()),
                    ..Effects::default()
                },
                Some(history::Screen::Detail(_)) => history.detail_paste(&text, Instant::now())?,
                None => ViewerInput::new(&mut state, &mut interaction).paste(&text, Instant::now()),
            },
            Event::Key(key) if key.kind != KeyEventKind::Release => match history.screen() {
                Some(history::Screen::List) => {
                    if !history.key(key, size.height.saturating_sub(4) as usize, Instant::now())? {
                        break;
                    }
                    Effects {
                        redraw: true,
                        render: false,
                        ..Effects::default()
                    }
                }
                Some(history::Screen::Detail(_)) => history.detail_key(
                    key,
                    historical_view.as_ref().map_or(1, |view| view.viewport().1),
                    Instant::now(),
                )?,
                None if state.prompt().is_none()
                    && !state.help()
                    && state.commit_dialog().is_none()
                    && state.discard_menu().is_none()
                    && state.filter_menu().is_none()
                    && config.bindings.action(state.focus(), key) == Some(Action::History) =>
                {
                    history.toggle()?;
                    Effects {
                        redraw: true,
                        ..Effects::default()
                    }
                }
                None => ViewerInput::new(&mut state, &mut interaction).key(
                    key,
                    &config.bindings,
                    viewport.1,
                    Instant::now(),
                ),
            },
            Event::Mouse(mouse) if size.width >= 12 && size.height >= 6 => match history.screen() {
                Some(history::Screen::List) => Effects {
                    redraw: history.mouse(mouse, full_area)?,
                    ..Effects::default()
                },
                Some(history::Screen::Detail(_)) => {
                    history.detail_mouse(mouse, full_area, Instant::now())?
                }
                None => ViewerInput::new(&mut state, &mut interaction).mouse(
                    &view,
                    &config,
                    mouse,
                    Instant::now(),
                ),
            },
            _ => Effects::default(),
        };
        let copy = if history.detail_state().is_some() {
            history.take_copy()
        } else {
            ViewerInput::new(&mut state, &mut interaction).take_copy()
        };
        if let Some(text) = copy {
            clipboard::copy(&mut io::stdout().lock(), &text)?;
        }
        needs_draw |= effects.redraw;
        if history.detail_state().is_some() {
            if effects.render {
                history.request_render()?;
            }
        } else if history.screen().is_none() {
            render_needed |= effects.render;
        }
        if let Some(command) = effects.command {
            if history.detail_state().is_some() {
                match command {
                    crate::state::Command::Quit => break,
                    crate::state::Command::OpenEditor(position) => history.open_editor(
                        &mut terminal,
                        historical_view.as_ref().map_or(1, |view| view.viewport().1),
                        position,
                    )?,
                    crate::state::Command::ToggleStage
                    | crate::state::Command::OpenCommit
                    | crate::state::Command::OpenDiscard
                    | crate::state::Command::ConfirmDiscard
                    | crate::state::Command::SubmitCommit => {
                        if let Some(detail) = history.detail_mut() {
                            detail.show_toast(
                                "Git changes are unavailable while viewing history".into(),
                                Instant::now(),
                            );
                        }
                    }
                    _ => {}
                }
                needs_draw = true;
                continue;
            }
            match commands.dispatch(
                command,
                commands::Context {
                    terminal: &mut terminal,
                    state: &mut state,
                    collecting: &collecting,
                    interaction: &mut interaction,
                    size,
                    height: viewport.1,
                },
            )? {
                ControlFlow::Break(()) => break,
                ControlFlow::Continue(redraw) => needs_draw |= redraw,
            }
        }
    }
    Ok(())
}

fn suppress_colors(buffer: &mut ratatui::buffer::Buffer) {
    // Suppressed color commands can reset text attributes in Crossterm.
    // Keep backend colors at their defaults so reverse-video selection survives.
    for cell in &mut buffer.content {
        cell.set_fg(Color::Reset).set_bg(Color::Reset);
    }
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
            Constraint::Length((body.width / 3).min(34)),
            Constraint::Min(1),
        ])
        .split(body);
        (pieces[0], pieces[1])
    } else {
        (Rect::default(), body)
    }
}

/// Shared geometry for drawing, wrapping, and input routing.
struct ViewAreas {
    area: Rect,
    tree: Rect,
    diff: Rect,
    diff_content: Rect,
    status_border: Rect,
    status: Rect,
    footer: Rect,
}

impl ViewAreas {
    fn handle_mouse(
        &self,
        state: &mut State,
        config: &Config,
        interaction: &mut Interaction,
        mouse: MouseEvent,
        now: Instant,
    ) -> Effects {
        let mut redraw = interaction.selection.reconcile(state, self);
        if state.help() {
            let help = help::Help::new(self.area, &config.bindings, &Palette::new(config.light));
            help.sync(state);
            if help.rect().contains((mouse.column, mouse.row).into())
                && mouse.modifiers.is_empty()
                && matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                )
            {
                redraw |= state.scroll_help(3, mouse.kind == MouseEventKind::ScrollDown);
            }
            return Effects {
                redraw,
                ..Effects::default()
            };
        }
        if state.prompt().is_some()
            || state.discard_menu().is_some()
            || state.commit_dialog().is_some()
            || state.filter_menu().is_some()
        {
            return Effects {
                redraw,
                ..Effects::default()
            };
        }
        let position = (mouse.column, mouse.row).into();
        if self.diff.contains(position)
            && mouse.modifiers.is_empty()
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            let previous = state.scroll();
            state.scroll_by(
                3,
                mouse.kind == MouseEventKind::ScrollDown,
                self.viewport().1,
            );
            redraw |= state.scroll() != previous;
        }
        // Update drag endpoints after wheel scrolling has changed the viewport.
        if self.diff.contains(position)
            || !matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            redraw |= interaction.selection.handle_mouse(state, self, mouse);
        }
        let mut effects = if self.tree.contains(position) {
            interaction
                .tree
                .handle_mouse(state, config, self.tree, mouse, now)
        } else {
            Effects::default()
        };
        effects.redraw |= redraw;
        effects
    }

    /// The exact header rectangle drawn over the document, also used for hit testing.
    fn header(&self, state: &State) -> Option<(usize, Rect)> {
        let inner = self.diff_content;
        let (section, header) = state
            .document()
            .visible_file_header(state.scroll(), inner.height as usize)?;
        let y = inner.y + header.start.saturating_sub(state.scroll()) as u16;
        let height = header.len();
        Some((
            section.rows.start,
            Rect::new(inner.x, y, inner.width, height as u16),
        ))
    }

    fn new(area: Rect, sidebar: bool) -> Self {
        let rows = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
        let (tree, diff) = panes(rows[0], sidebar);
        Self {
            area,
            tree,
            diff,
            // Reserve the scrollbar column even when the document fits.
            diff_content: Rect {
                width: diff.width.saturating_sub(1),
                ..diff
            },
            status_border: rows[1],
            status: rows[2],
            footer: rows[3],
        }
    }

    fn viewport(&self) -> (usize, usize) {
        (
            self.diff_content.width as usize,
            self.diff_content.height.max(1) as usize,
        )
    }
}

fn draw(
    frame: &mut Frame,
    state: &State,
    config: &Config,
    activity: Activity,
    interaction: &mut Interaction,
    help_scroll: usize,
) {
    draw_in(
        frame,
        state,
        config,
        StatusLabel::Activity(activity),
        interaction,
        help_scroll,
        frame.area(),
    );
}

enum StatusLabel<'a> {
    Activity(Activity),
    Title { text: &'a str, footer: &'a str },
}

fn draw_in(
    frame: &mut Frame,
    state: &State,
    config: &Config,
    status: StatusLabel<'_>,
    interaction: &mut Interaction,
    help_scroll: usize,
    area: Rect,
) {
    let palette = Palette::new(config.light);
    frame.render_widget(Block::default().style(palette.base()), area);
    if area.width < 12 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("Terminal too small").style(palette.base()),
            area,
        );
        return;
    }
    let view = ViewAreas::new(area, state.sidebar());
    interaction.selection.reconcile(state, &view);
    let tree_divider_visible =
        view.tree.width > 0 && tree::draw(frame, state, config, view.tree, &mut interaction.tree);
    let inner = view.diff_content;
    let rows: Vec<Line<'_>> = state
        .document()
        .rows
        .iter()
        .skip(state.scroll())
        .take(inner.height as usize)
        .map(|r| r.line.clone())
        .collect();
    frame.render_widget(Paragraph::new(rows).style(palette.base()), inner);
    if let Some(hit) = state
        .match_index()
        .and_then(|index| state.document().hits.get(index))
    {
        let style = Style::default()
            .bg(Color::Rgb(184, 92, 31))
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        for fragment in &hit.fragments {
            let Some(row) = fragment.row.checked_sub(state.scroll()) else {
                continue;
            };
            if row >= usize::from(inner.height) {
                continue;
            }
            let start = fragment.columns.start.min(usize::from(inner.width));
            let end = fragment.columns.end.min(usize::from(inner.width));
            frame.buffer_mut().set_style(
                Rect::new(
                    inner.x + start as u16,
                    inner.y + row as u16,
                    (end - start) as u16,
                    1,
                ),
                style,
            );
        }
    }

    if let Some((start, header_area)) = view.header(state) {
        let header: Vec<_> = state
            .document()
            .rows
            .iter()
            .skip(start)
            .take(header_area.height as usize)
            .map(|row| row.line.clone())
            .collect();
        frame.render_widget(Paragraph::new(header).style(palette.base()), header_area);
        if state.focus() == Scope::Diff && !header_area.is_empty() {
            frame.buffer_mut().set_style(
                Rect {
                    height: 1,
                    ..header_area
                },
                Style::default().bg(palette.focused),
            );
            frame.render_widget(
                Paragraph::new("▎").style(palette.base().bg(palette.focused).fg(palette.accent)),
                Rect {
                    width: 1,
                    height: 1,
                    ..header_area
                },
            );
        }
    }
    interaction
        .selection
        .draw(frame.buffer_mut(), state, &view, palette);
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
                .track_symbol(Some(" "))
                .thumb_symbol("┃")
                .track_style(palette.base())
                .thumb_style(palette.base().fg(palette.accent)),
            view.diff,
            &mut scrollbar_state,
        );
    }
    let separator_style = palette.base().fg(palette.muted);
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(separator_style),
        view.status_border,
    );
    if tree_divider_visible {
        frame.render_widget(
            Paragraph::new("┴").style(separator_style),
            Rect::new(view.tree.right() - 1, view.status_border.y, 1, 1),
        );
    }
    let status_spans = if let Some(error) = state.error() {
        vec![Span::styled(
            format!(" Error: {error}"),
            palette.base().fg(palette.removed_emphasis),
        )]
    } else {
        let muted = palette.base().fg(palette.muted);
        let accent = palette.base().fg(palette.accent);
        let mut spans = vec![match &status {
            StatusLabel::Title { text, .. } => {
                Span::styled(format!(" {text}"), accent.add_modifier(Modifier::BOLD))
            }
            StatusLabel::Activity(activity) => {
                Span::styled(format!(" {}", activity.label()), muted)
            }
        }];
        if !state.filter().is_empty() {
            spans.extend([
                Span::styled(" · ", muted),
                Span::styled("filter: ", accent),
                Span::styled(state.filter(), accent),
            ]);
        }
        if state.change_filter().selected() {
            spans.push(Span::styled(
                format!(" · {}", state.change_filter().summary()),
                accent,
            ));
        }
        if !state.search().is_empty() {
            spans.extend([
                Span::styled(" · ", muted),
                Span::styled("/", accent),
                Span::styled(state.search(), accent),
                Span::styled(
                    format!(
                        ": {}{} matches",
                        state.document().hits.len(),
                        if state.rendering() { " (updating)" } else { "" }
                    ),
                    muted,
                ),
            ]);
        }
        if state.document().excluded > 0 {
            spans.push(Span::styled(
                format!(" · {} oversized excluded", state.document().excluded),
                muted,
            ));
        }
        if let Some(totals) = state
            .snapshot()
            .tracked_stats
            .filter(|_| !state.presentation().is_files())
        {
            stats::append(&mut spans, totals, usize::from(view.status.width), palette);
        }
        spans
    };
    frame.render_widget(
        Paragraph::new(Line::from(status_spans)).style(palette.base()),
        view.status,
    );
    if let Some(prompt) = state.prompt() {
        let label = match prompt.kind() {
            PromptKind::Filter => "Filter › ",
            PromptKind::Find => "Find › ",
            PromptKind::Search => "Search › ",
        };
        let accept = config.bindings.labels(Scope::Input, Action::Accept);
        let cancel = config.bindings.labels(Scope::Input, Action::Cancel);
        let next = config.bindings.labels(Scope::Input, Action::NextFindMatch);
        let suffix = if prompt.kind() == PromptKind::Find && !next.is_empty() {
            format!("  ({next}: next match · {accept}: keep · {cancel}: cancel)")
        } else {
            format!("  ({accept}: keep · {cancel}: cancel)")
        };
        let available = view.footer.width.saturating_sub(label.len() as u16 + 1) as usize;
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
            view.footer,
        );
        let x = view.footer.x
            + (UnicodeWidthStr::width(label)
                + UnicodeWidthStr::width(&prompt.editor().text()[start..prompt.editor().cursor()]))
                as u16;
        frame.set_cursor_position((x.min(area.right().saturating_sub(1)), view.footer.y));
    } else {
        let footer = match status {
            StatusLabel::Title { footer, .. } => std::borrow::Cow::Borrowed(footer),
            StatusLabel::Activity(_) => {
                let keys = |action| config.bindings.labels(Scope::Global, action);
                std::borrow::Cow::Owned(format!(
                    " {} files  {} view  {} history  {} sidebar  {} split  {} filter  {} find  {} search  {} help  {} quit",
                    format_args!("{}/{}", keys(Action::NextFile), keys(Action::PreviousFile)),
                    format_args!("{}/{}", keys(Action::HalfDown), keys(Action::HalfUp)),
                    keys(Action::History),
                    keys(Action::Sidebar),
                    keys(Action::Layout),
                    keys(Action::Filter),
                    keys(Action::Find),
                    keys(Action::Search),
                    keys(Action::Help),
                    keys(Action::Quit)
                ))
            }
        };
        frame.render_widget(
            Paragraph::new(footer.as_ref()).style(palette.base().fg(palette.muted)),
            view.footer,
        );
    }
    if let Some(message) = state.toast().filter(|_| !state.help()) {
        let body = Rect::new(area.x, area.y, area.width, view.status_border.y - area.y);
        draw_toast(frame, message, body, &palette);
    }
    if state.help() {
        help::Help::new(area, &config.bindings, &palette).draw(frame, help_scroll, &palette);
    }
    discard::draw(frame, state, &config.bindings, &palette);
    commit::draw(frame, state, &palette);
    filters::draw(frame, state, &palette);
}

fn draw_toast(frame: &mut Frame, message: &str, body: Rect, palette: &Palette) {
    if body.width < 4 || body.height < 3 {
        return;
    }
    let width = (message.width() + 4).min(body.width as usize) as u16;
    let rect = Rect::new(body.right() - width, body.bottom() - 3, width, 3);
    let style = palette.base().bg(palette.panel);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style.fg(palette.accent))
        .style(style);
    let inner = block.inner(rect);
    let text_width = inner.width.saturating_sub(2) as usize;
    let text = if message.width() > text_width {
        format!(
            "{}…",
            message
                .chars()
                .take(text_width.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        message.to_owned()
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(format!(" {text} ")).style(style), inner);
}
#[cfg(test)]
mod tests;
