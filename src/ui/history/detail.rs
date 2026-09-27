use super::super::{
    Interaction, StatusLabel, ViewAreas, ViewerInput, collect, draw_in, editor, modal,
    worker_update,
};
use super::{
    Detail, DetailKind, DetailScreen, History, InfoOverlay, Screen, safe, safe_message, shortcut,
};
use crate::{
    AppResult,
    git::{CommitInfo, Mode},
    input::{Action, Scope},
    model::{Presentation, Snapshot},
    render::{Palette, worker::RenderWorker},
    state::{EditorPosition, Effects, State},
};
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{KeyEvent, MouseEvent},
    layout::Rect,
    widgets::{Paragraph, Wrap},
};
use std::{path::Path, time::Instant};

impl Detail {
    fn input(
        &mut self,
        commit: Option<&CommitInfo>,
        action: impl FnOnce(&mut ViewerInput<'_>) -> Effects,
    ) -> AppResult<Effects> {
        let before = self.state.selection().to_owned();
        let effects = action(&mut ViewerInput::new(
            &mut self.state,
            &mut self.interaction,
        ));
        self.select_file_if_changed(&before, commit)?;
        Ok(effects)
    }

    fn select_file(&mut self, commit: &CommitInfo) -> AppResult<()> {
        self.collecting.mode(
            Mode::Files {
                id: commit.id.clone(),
                selected: Some(self.state.selection().to_owned()),
            },
            self.state.collection_request(),
        )?;
        Ok(())
    }

    fn select_file_if_changed(
        &mut self,
        before: &Path,
        commit: Option<&CommitInfo>,
    ) -> AppResult<()> {
        if let Some(commit) = commit
            && before != self.state.selection()
            && self
                .state
                .tree(false)
                .iter()
                .any(|row| row.path == self.state.selection() && !row.directory)
        {
            self.select_file(commit)?;
        }
        Ok(())
    }
}

impl History {
    pub(crate) fn sync_detail_help(&mut self, area: Rect) {
        if let (true, Screen::Detail(screen)) = (self.visible, &mut self.screen) {
            let detail = &mut screen.viewer;
            ViewerInput::new(&mut detail.state, &mut detail.interaction)
                .sync_help(area, &self.config);
        }
    }

    pub fn apply_pending_filter(&mut self, now: Instant) -> AppResult<Effects> {
        if !self.visible {
            return Ok(Effects::default());
        }
        let Screen::Detail(screen) = &mut self.screen else {
            return Ok(Effects::default());
        };
        let detail = &mut screen.viewer;
        detail.input(files_commit(&screen.kind), |viewer| {
            viewer.pending_filter(now)
        })
    }

    pub fn take_copy(&mut self) -> Option<String> {
        if !self.visible {
            return None;
        }
        match &mut self.screen {
            Screen::Detail(screen) => {
                ViewerInput::new(&mut screen.viewer.state, &mut screen.viewer.interaction)
                    .take_copy()
            }
            _ => None,
        }
    }

    pub fn clear_selection(&mut self) {
        if let Screen::Detail(screen) = &mut self.screen {
            ViewerInput::new(&mut screen.viewer.state, &mut screen.viewer.interaction)
                .clear_selection();
        }
    }

    pub(super) fn poll_detail(&mut self, now: Instant) -> AppResult<bool> {
        if !self.visible {
            return Ok(false);
        }
        let mut redraw = false;
        let mut render = false;
        if let Screen::Detail(screen) = &mut self.screen {
            let detail = &mut screen.viewer;
            let files_commit = files_commit(&screen.kind);
            while let Some(update) = worker_update(
                detail.collecting.updates.try_recv(),
                "Git history collection",
            )? {
                let effects = match update {
                    collect::Update::Started(generation) => {
                        detail.state.collection_started(generation);
                        crate::state::Effects::default()
                    }
                    collect::Update::Finished(generation, result) => {
                        let before = detail.state.selection().to_owned();
                        let effects = detail.state.collection_finished(generation, result, now);
                        detail.select_file_if_changed(&before, files_commit)?;
                        effects
                    }
                    _ => crate::state::Effects::default(),
                };
                render |= effects.render;
                redraw |= effects.redraw;
            }
        }
        if render {
            self.request_render()?;
        }
        Ok(redraw)
    }

    pub fn update_render(&mut self, height: usize) -> AppResult<bool> {
        let Screen::Detail(screen) = &mut self.screen else {
            return Ok(false);
        };
        let detail = &mut screen.viewer;
        let mut redraw = false;
        while let Some(result) = worker_update(detail.rendering.try_recv(), "History rendering")? {
            redraw |= detail.state.apply_render(result, height);
        }
        Ok(redraw)
    }

    pub fn request_render(&mut self) -> AppResult<()> {
        let Screen::Detail(screen) = &mut self.screen else {
            return Ok(());
        };
        let detail = &mut screen.viewer;
        let size = ratatui::crossterm::terminal::size()?;
        let area = Rect::new(0, 0, size.0, size.1);
        let view = ViewAreas::new(area, detail.state.sidebar());
        detail.rendering.request(
            detail
                .state
                .render_request(view.viewport().0, view.viewport().1),
        )?;
        Ok(())
    }

    pub(super) fn open(&mut self, kind: DetailKind) -> AppResult<()> {
        let mode = match &kind {
            DetailKind::Commit {
                commit,
                presentation: Presentation::Files,
            } => Mode::Files {
                id: commit.id.clone(),
                selected: None,
            },
            DetailKind::Commit { commit, .. } => Mode::Commit {
                id: commit.id.clone(),
                parent: commit.parents.first().cloned(),
            },
            DetailKind::Comparison { base, target } => comparison(base, target),
        };
        let mut state = State::with_whitespace(self.config.split, self.config.whitespace);
        state.set_comparison(true);
        state.set_presentation(match &kind {
            DetailKind::Commit { presentation, .. } => *presentation,
            DetailKind::Comparison { .. } => Presentation::Changes,
        });
        let collecting = collect::Worker::spawn(
            self.repo.clone(),
            mode,
            self.config.limits,
            self.config.reconcile,
        )?;
        collecting.request(state.collection_request())?;
        let rendering = RenderWorker::spawn(self.config.clone())?;
        self.screen = Screen::Detail(Box::new(DetailScreen {
            kind,
            viewer: Detail {
                state,
                interaction: Interaction::default(),
                collecting,
                rendering,
            },
            info: None,
        }));
        self.request_render()?;
        Ok(())
    }

    fn change_mode(&mut self, mode: Mode) -> AppResult<()> {
        let Screen::Detail(screen) = &mut self.screen else {
            return Ok(());
        };
        let detail = &mut screen.viewer;
        detail.state.replace_presentation(
            if matches!(mode, Mode::Files { .. }) {
                Presentation::Files
            } else {
                Presentation::Changes
            },
            Snapshot::default(),
        );
        detail
            .collecting
            .mode(mode, detail.state.collection_request())?;
        self.request_render()
    }

    pub fn detail_paste(&mut self, text: &str, now: Instant) -> AppResult<Effects> {
        let Screen::Detail(screen) = &mut self.screen else {
            return Ok(Effects::default());
        };
        if screen.info.is_some() {
            return Ok(Effects::default());
        }
        let detail = &mut screen.viewer;
        detail.input(files_commit(&screen.kind), |viewer| viewer.paste(text, now))
    }

    pub fn detail_key(&mut self, key: KeyEvent, height: usize, now: Instant) -> AppResult<Effects> {
        let Screen::Detail(screen) = &mut self.screen else {
            return Ok(Effects::default());
        };
        let detail = &mut screen.viewer;
        let history_action = self.config.bindings.action(Scope::History, key);
        let action = history_action
            .filter(|action| {
                matches!(
                    action,
                    Action::History | Action::ToggleFiles | Action::Swap | Action::CommitInfo
                )
            })
            .or_else(|| self.config.bindings.action(detail.state.focus(), key));
        if let Some(info) = &mut screen.info {
            match action {
                Some(Action::Cancel | Action::CommitInfo) => screen.info = None,
                Some(Action::Down) => info.scroll = info.scroll.saturating_add(1),
                Some(Action::Up) => info.scroll = info.scroll.saturating_sub(1),
                Some(Action::PageDown) => {
                    info.scroll = info.scroll.saturating_add(info.viewport.max(1))
                }
                Some(Action::PageUp) => {
                    info.scroll = info.scroll.saturating_sub(info.viewport.max(1))
                }
                Some(Action::First) => info.scroll = 0,
                Some(Action::Quit) => {
                    return Ok(Effects {
                        command: Some(crate::state::Command::Quit),
                        ..Effects::default()
                    });
                }
                _ => {}
            }
            return Ok(Effects {
                redraw: true,
                ..Effects::default()
            });
        }
        if detail.state.prompt().is_none()
            && !detail.state.help()
            && detail.state.filter_menu().is_none()
            && action == Some(Action::CommitInfo)
        {
            screen.info = Some(InfoOverlay::default());
            return Ok(Effects {
                redraw: true,
                ..Effects::default()
            });
        }
        if detail.state.prompt().is_none()
            && !detail.state.help()
            && detail.state.filter_menu().is_none()
            && detail.state.discard_menu().is_none()
            && detail.state.commit_dialog().is_none()
            && action == Some(Action::Cancel)
        {
            if detail.interaction.selection.clear() {
                return Ok(Effects {
                    redraw: true,
                    ..Effects::default()
                });
            }
            self.screen = Screen::List;
            return Ok(Effects {
                redraw: true,
                ..Effects::default()
            });
        }
        if detail.state.prompt().is_none()
            && !detail.state.help()
            && detail.state.filter_menu().is_none()
            && detail.state.discard_menu().is_none()
            && detail.state.commit_dialog().is_none()
        {
            if action == Some(Action::History) {
                self.toggle()?;
                return Ok(Effects {
                    redraw: true,
                    ..Effects::default()
                });
            }
            let mode = match (action, &mut screen.kind) {
                (
                    Some(Action::ToggleFiles),
                    DetailKind::Commit {
                        commit,
                        presentation,
                    },
                ) => {
                    *presentation = if presentation.is_files() {
                        Presentation::Changes
                    } else {
                        Presentation::Files
                    };
                    Some(if presentation.is_files() {
                        Mode::Files {
                            id: commit.id.clone(),
                            selected: None,
                        }
                    } else {
                        Mode::Commit {
                            id: commit.id.clone(),
                            parent: commit.parents.first().cloned(),
                        }
                    })
                }
                (Some(Action::Swap), DetailKind::Comparison { base, target }) => {
                    std::mem::swap(base, target.as_mut());
                    Some(comparison(base, target))
                }
                _ => None,
            };
            if let Some(mode) = mode {
                self.change_mode(mode)?;
                return Ok(Effects {
                    redraw: true,
                    render: true,
                    ..Effects::default()
                });
            }
        }
        let mut effects = detail.input(files_commit(&screen.kind), |viewer| {
            viewer.key(key, &self.config.bindings, height, now)
        })?;
        if let Some(command) = effects.command.take() {
            match command {
                crate::state::Command::Collect => {
                    if let Some(commit) = files_commit(&screen.kind) {
                        detail.select_file(commit)?;
                    } else {
                        detail
                            .collecting
                            .request(detail.state.collection_request())?;
                    }
                }
                _ => effects.command = Some(command),
            }
        }
        Ok(effects)
    }

    pub fn open_editor(
        &mut self,
        terminal: &mut DefaultTerminal,
        height: usize,
        position: EditorPosition,
    ) -> AppResult<()> {
        let Screen::Detail(screen) = &mut self.screen else {
            return Ok(());
        };
        let detail = &mut screen.viewer;
        let Some(target) = detail.state.editor_target(height, position) else {
            detail
                .state
                .show_toast("Select a file to open in the editor".into(), Instant::now());
            return Ok(());
        };
        let deleted = detail
            .state
            .snapshot()
            .changes
            .iter()
            .any(|change| change.path == target.path && change.status == 'D');
        let revision = match &screen.kind {
            DetailKind::Commit {
                commit,
                presentation: Presentation::Changes,
            } if deleted => commit.parents.first().unwrap_or(&commit.id),
            DetailKind::Commit { commit, .. } => &commit.id,
            DetailKind::Comparison { base, .. } if deleted => &base.id,
            DetailKind::Comparison { target: end, .. } => &end.id,
        };
        let result = editor::open_historical(terminal, &self.repo, revision, target);
        if let Err(error) = result {
            detail.state.show_toast(error.to_string(), Instant::now());
        }
        Ok(())
    }

    pub fn detail_mouse(
        &mut self,
        mouse: MouseEvent,
        area: Rect,
        now: Instant,
    ) -> AppResult<Effects> {
        let Screen::Detail(screen) = &mut self.screen else {
            return Ok(Effects::default());
        };
        if screen.info.is_some() {
            return Ok(Effects::default());
        }
        let detail = &mut screen.viewer;
        let view = ViewAreas::new(area, detail.state.sidebar());
        detail.input(files_commit(&screen.kind), |viewer| {
            viewer.mouse(&view, &self.config, mouse, now)
        })
    }

    pub fn draw_detail(&mut self, frame: &mut Frame) {
        let Screen::Detail(screen) = &mut self.screen else {
            return;
        };
        let detail = &mut screen.viewer;
        let palette = Palette::new(self.config.light);
        let area = frame.area();
        let summary = match &screen.kind {
            DetailKind::Commit {
                commit,
                presentation,
            } => {
                let view = if presentation.is_files() {
                    "FILES"
                } else {
                    "CHANGES"
                };
                format!(
                    "{} · {} · {}",
                    commit.short_id(),
                    view,
                    safe(commit.subject())
                )
            }
            DetailKind::Comparison { base, target } => {
                format!("Compare {} → {}", base.short_id(), target.short_id())
            }
        };
        let keys = |action| shortcut(&self.config.bindings, action);
        let footer = match &screen.kind {
            DetailKind::Commit { presentation, .. } => format!(
                " {} history · {} {} · {} commit details · {} original · {} help · {} quit",
                keys(Action::Cancel),
                keys(Action::ToggleFiles),
                if presentation.is_files() {
                    "diff view"
                } else {
                    "timecapsule"
                },
                keys(Action::CommitInfo),
                keys(Action::History),
                keys(Action::Help),
                keys(Action::Quit),
            ),
            DetailKind::Comparison { .. } => format!(
                " {} history · {} swap · {} endpoint details · {} original · {} help · {} quit",
                keys(Action::Cancel),
                keys(Action::Swap),
                keys(Action::CommitInfo),
                keys(Action::History),
                keys(Action::Help),
                keys(Action::Quit),
            ),
        };
        draw_in(
            frame,
            &detail.state,
            &self.config,
            StatusLabel::Title {
                text: &summary,
                footer: &footer,
            },
            &mut detail.interaction,
            detail.state.help_scroll(),
            area,
        );
        if let Some(info) = &mut screen.info {
            let text = match &screen.kind {
                DetailKind::Commit { commit, .. } => format!(
                    "Commit: {}\nParents: {}\nAuthor: {} <{}> · {}\nCommitter: {} <{}> · {}\nRefs: {}\n\n{}",
                    commit.id,
                    commit.parents.join(", "),
                    safe(&commit.author),
                    safe(&commit.author_email),
                    safe(&commit.authored),
                    safe(&commit.committer),
                    safe(&commit.committer_email),
                    safe(&commit.committed),
                    safe(&commit.refs.join(", ")),
                    safe_message(&commit.message)
                ),
                DetailKind::Comparison { base, target } => format!(
                    "BASE: {}\n{}\n\nTARGET: {}\n{}",
                    base.id,
                    safe_message(&base.message),
                    target.id,
                    safe_message(&target.message)
                ),
            };
            let title = " Commit details ";
            let keys = |action| shortcut(&self.config.bindings, action);
            let hint = modal::hint(modal::scroll_hint(keys), &palette);
            let width = modal::widest(&text)
                .max(modal::widest(title))
                .max(hint.width());
            let modal = modal::Modal::new(area, width, 0, Vec::new());
            let columns = usize::from(modal.inner().width.max(1));
            let rows = modal::wrapped_rows(&text, columns);
            let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
            let modal = modal::Modal::new(area, width, rows, vec![hint]);
            let inner = modal.draw(frame, title, &palette);
            info.viewport = inner.height;
            info.scroll = info.scroll.min(
                rows.saturating_sub(usize::from(inner.height))
                    .min(usize::from(u16::MAX)) as u16,
            );
            frame.render_widget(
                paragraph
                    .scroll((info.scroll, 0))
                    .style(palette.base().bg(palette.panel)),
                inner,
            );
        }
    }
}

fn comparison(base: &CommitInfo, target: &CommitInfo) -> Mode {
    Mode::Comparison {
        base: base.id.clone(),
        head: target.id.clone(),
        label: format!("{} → {}", base.short_id(), target.short_id()),
    }
}

fn files_commit(kind: &DetailKind) -> Option<&CommitInfo> {
    match kind {
        DetailKind::Commit {
            commit,
            presentation: Presentation::Files,
        } => Some(commit),
        _ => None,
    }
}
