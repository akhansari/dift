use super::super::{help, worker_update};
use super::{DetailKind, History, safe, shortcut, worker::Request};
use crate::{
    AppResult,
    input::{Action, Editor, Scope},
    render::Palette,
};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};
use std::{
    io,
    time::{Duration, Instant},
};

impl History {
    pub(super) fn load(&mut self, refresh: bool, append: bool) -> AppResult<()> {
        self.list.generation += 1;
        self.list.loading = true;
        self.list.error = None;
        if refresh {
            self.list.base = None;
        }
        let worker = self
            .list
            .worker
            .as_ref()
            .ok_or_else(|| io::Error::other("History worker stopped"))?;
        worker.sender.send(Request {
            generation: self.list.generation,
            refresh,
            skip: if append { self.list.commits.len() } else { 0 },
            query: self.list.query.clone(),
        })?;
        Ok(())
    }

    pub(super) fn poll_list(&mut self, now: Instant) -> AppResult<bool> {
        if self
            .list
            .search_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.list.search_deadline = None;
            self.list.query = self
                .list
                .search
                .as_ref()
                .map_or("", |(editor, _)| editor.text())
                .to_owned();
            self.load(false, false)?;
        }
        let mut redraw = false;
        if let Some(worker) = &self.list.worker {
            while let Some(page) = worker_update(worker.updates.try_recv(), "Git history")? {
                if page.generation != self.list.generation {
                    continue;
                }
                self.list.loading = false;
                match page.result {
                    Ok((tip, branch, data)) => {
                        self.list.tip = tip;
                        self.list.branch = branch;
                        self.list.more = data.more;
                        if page.append {
                            self.list.commits.extend(data.commits);
                        } else {
                            let selected = self
                                .list
                                .commits
                                .get(self.list.cursor)
                                .map(|commit| commit.id.as_str());
                            self.list.cursor = selected
                                .and_then(|id| {
                                    data.commits.iter().position(|commit| commit.id == id)
                                })
                                .unwrap_or(0);
                            self.list.offset = 0;
                            self.list.commits = data.commits;
                        }
                    }
                    Err(error) => self.list.error = Some(error.to_string()),
                }
                redraw = true;
            }
        }
        Ok(redraw)
    }

    pub fn key(&mut self, key: KeyEvent, height: usize, now: Instant) -> AppResult<bool> {
        let scope = if self.list.search.is_some() {
            Scope::Input
        } else {
            Scope::History
        };
        let action = self.config.bindings.action(scope, key);
        if action == Some(Action::Quit) {
            return Ok(false);
        }
        if self.list.help {
            match action {
                Some(Action::Help | Action::Cancel) => self.list.help = false,
                Some(Action::Down) => {
                    self.list.help_scroll = self.list.help_scroll.saturating_add(1)
                }
                Some(Action::Up) => self.list.help_scroll = self.list.help_scroll.saturating_sub(1),
                Some(Action::PageDown) => {
                    self.list.help_scroll = self.list.help_scroll.saturating_add(height)
                }
                Some(Action::PageUp) => {
                    self.list.help_scroll = self.list.help_scroll.saturating_sub(height)
                }
                _ => {}
            }
            return Ok(true);
        }
        if let Some((editor, previous)) = &mut self.list.search {
            match action {
                Some(Action::Accept) => {
                    self.list.query = editor.text().to_owned();
                    self.list.search = None;
                    self.list.search_deadline = None;
                    self.load(false, false)?;
                }
                Some(Action::Cancel) => {
                    self.list.query = previous.clone();
                    self.list.search = None;
                    self.list.search_deadline = None;
                    self.load(false, false)?;
                }
                Some(action) => {
                    if editor.edit(action) {
                        self.list.search_deadline = Some(now + Duration::from_millis(250));
                    }
                }
                None if let KeyCode::Char(ch) = key.code
                    && editor.insert(ch.encode_utf8(&mut [0; 4])) =>
                {
                    self.list.search_deadline = Some(now + Duration::from_millis(250));
                }
                _ => {}
            }
            return Ok(true);
        }
        match action {
            Some(Action::History | Action::Cancel) => {
                self.toggle()?;
            }
            Some(Action::Down) => {
                self.list.cursor =
                    (self.list.cursor + 1).min(self.list.commits.len().saturating_sub(1))
            }
            Some(Action::Up) => self.list.cursor = self.list.cursor.saturating_sub(1),
            Some(Action::HalfDown | Action::PageDown) => {
                self.list.cursor = (self.list.cursor + height.max(1))
                    .min(self.list.commits.len().saturating_sub(1))
            }
            Some(Action::HalfUp | Action::PageUp) => {
                self.list.cursor = self.list.cursor.saturating_sub(height.max(1))
            }
            Some(Action::First) => self.list.cursor = 0,
            Some(Action::Last) => self.list.cursor = self.list.commits.len().saturating_sub(1),
            Some(Action::Accept) => {
                if let Some(commit) = self.list.commits.get(self.list.cursor).cloned() {
                    self.open(DetailKind::Commit {
                        commit,
                        presentation: crate::model::Presentation::Changes,
                    })?;
                }
            }
            Some(Action::MarkBase) => {
                if let Some(commit) = self.list.commits.get(self.list.cursor) {
                    if self
                        .list
                        .base
                        .as_ref()
                        .is_some_and(|base| base.id == commit.id)
                    {
                        self.list.base = None;
                    } else {
                        self.list.base = Some(commit.clone());
                    }
                }
            }
            Some(Action::Compare) => {
                if let (Some(base), Some(target)) =
                    (&self.list.base, self.list.commits.get(self.list.cursor))
                    && base.id != target.id
                {
                    self.open(DetailKind::Comparison {
                        base: base.clone(),
                        target: Box::new(target.clone()),
                    })?;
                }
            }
            Some(Action::Search) => {
                self.list.search = Some((
                    Editor::new(self.list.query.clone()),
                    self.list.query.clone(),
                ))
            }
            Some(Action::Help) => {
                self.list.help = true;
                self.list.help_scroll = 0;
            }
            Some(Action::Refresh) => {
                self.list.commits.clear();
                self.list.cursor = 0;
                self.load(true, false)?;
            }
            _ => {}
        }
        if self.list.more
            && !self.list.loading
            && self.list.cursor + height + 10 >= self.list.commits.len()
        {
            self.load(false, true)?;
        }
        Ok(true)
    }

    pub fn paste(&mut self, text: &str, now: Instant) -> bool {
        if let Some((editor, _)) = &mut self.list.search {
            if editor.insert(text) {
                self.list.search_deadline = Some(now + Duration::from_millis(250));
            }
            return true;
        }
        false
    }

    pub fn mouse(&mut self, mouse: MouseEvent, area: Rect) -> AppResult<bool> {
        if self.list.search.is_some() || self.list.help {
            return Ok(false);
        }
        let body = list_body(area);
        if !body.contains((mouse.column, mouse.row).into()) {
            return Ok(false);
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.list.cursor =
                    (self.list.cursor + 3).min(self.list.commits.len().saturating_sub(1))
            }
            MouseEventKind::ScrollUp => self.list.cursor = self.list.cursor.saturating_sub(3),
            MouseEventKind::Down(MouseButton::Left) => {
                self.list.cursor = (self.list.offset + usize::from(mouse.row - body.y))
                    .min(self.list.commits.len().saturating_sub(1));
            }
            _ => return Ok(false),
        }
        if self.list.more
            && !self.list.loading
            && self.list.cursor + usize::from(body.height) + 10 >= self.list.commits.len()
        {
            self.load(false, true)?;
        }
        Ok(true)
    }

    pub fn draw_list(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let palette = Palette::new(self.config.light);
        frame.render_widget(Block::default().style(palette.base()), area);
        if area.height < 5 {
            return;
        }
        let body = list_body(area);
        let base = self
            .list
            .base
            .as_ref()
            .map(|commit| format!(" · base {} {}", commit.short_id(), safe(commit.subject())))
            .unwrap_or_default();
        let search = if self.list.query.is_empty() {
            String::new()
        } else {
            format!(" · search: {}", self.list.query)
        };
        let title = format!(" History · {}{}{}", safe(&self.list.branch), base, search);
        frame.render_widget(
            Paragraph::new(title).style(
                palette
                    .base()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect { height: 1, ..area },
        );
        let maximum = self
            .list
            .commits
            .len()
            .saturating_sub(usize::from(body.height));
        if self.list.cursor < self.list.offset {
            self.list.offset = self.list.cursor;
        } else if self.list.cursor >= self.list.offset + usize::from(body.height) {
            self.list.offset = self.list.cursor + 1 - usize::from(body.height);
        }
        self.list.offset = self.list.offset.min(maximum);
        let items: Vec<_> = self
            .list
            .commits
            .iter()
            .skip(self.list.offset)
            .take(usize::from(body.height))
            .map(|commit| {
                let mark = if self
                    .list
                    .base
                    .as_ref()
                    .is_some_and(|base| base.id == commit.id)
                {
                    "◆"
                } else {
                    " "
                };
                let refs = if commit.refs.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", safe(&commit.refs.join(", ")))
                };
                let head = if self.list.tip.as_deref() == Some(commit.id.as_str()) {
                    " (HEAD)"
                } else {
                    ""
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{mark} {} ", commit.short_id()),
                        palette.base().fg(palette.accent),
                    ),
                    Span::raw(format!("{}{}", safe(commit.subject()), head)),
                    Span::styled(refs, palette.base().fg(palette.accent)),
                    Span::styled(
                        format!(
                            "  · {} · {}",
                            safe(&commit.author),
                            safe(&commit.relative_date)
                        ),
                        palette.base().fg(palette.muted),
                    ),
                ]))
            })
            .collect();
        let mut state = ListState::default()
            .with_selected(Some(self.list.cursor.saturating_sub(self.list.offset)));
        frame.render_stateful_widget(
            List::new(items)
                .style(palette.base())
                .highlight_symbol("▎")
                .highlight_style(Style::default().bg(palette.focused)),
            body,
            &mut state,
        );
        if self.list.commits.is_empty() {
            frame.render_widget(
                Paragraph::new(self.list.error.as_deref().unwrap_or(if self.list.loading {
                    "Loading history…"
                } else if self.list.tip.is_none() {
                    "No commits yet"
                } else {
                    "No matching commits"
                }))
                .style(palette.base().fg(palette.muted)),
                body,
            );
        }
        let status = if self.list.loading {
            "Loading…"
        } else if self.list.more {
            "More commits below"
        } else {
            "End of history"
        };
        frame.render_widget(
            Paragraph::new(format!(" {status} · {} shown", self.list.commits.len()))
                .style(palette.base().fg(palette.muted)),
            Rect::new(area.x, body.bottom(), area.width, 1),
        );
        if let Some((editor, _)) = &self.list.search {
            let accept = self.config.bindings.labels(Scope::Input, Action::Accept);
            let cancel = self.config.bindings.labels(Scope::Input, Action::Cancel);
            frame.render_widget(
                Paragraph::new(format!(
                    " Search › {}  ({}: keep · {}: cancel)",
                    editor.text(),
                    accept,
                    cancel
                ))
                .style(palette.base().fg(palette.accent)),
                Rect::new(area.x, area.bottom() - 1, area.width, 1),
            );
        } else {
            let keys = |action| shortcut(&self.config.bindings, action);
            let hint = format!(
                " {} view · {} mark base · {} compare · {} search · {} refresh · {} back · {} toggle · {} help · {} quit",
                keys(Action::Accept),
                keys(Action::MarkBase),
                keys(Action::Compare),
                keys(Action::Search),
                keys(Action::Refresh),
                keys(Action::Cancel),
                keys(Action::History),
                keys(Action::Help),
                keys(Action::Quit),
            );
            frame.render_widget(
                Paragraph::new(hint).style(palette.base().fg(palette.muted)),
                Rect::new(area.x, area.bottom() - 1, area.width, 1),
            );
        }
        if self.list.help {
            help::Help::new(area, &self.config.bindings, &palette).draw(
                frame,
                self.list.help_scroll,
                &palette,
            );
        }
    }
}

fn list_body(area: Rect) -> Rect {
    Rect {
        y: area.y.saturating_add(2),
        height: area.height.saturating_sub(4),
        ..area
    }
}
