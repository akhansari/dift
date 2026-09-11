use super::{PromptKind, State, TOAST_DURATION, Toast};
use crate::{
    git::GitError,
    input::{Action, Bindings, Scope},
    model::{ActivityObservations, Snapshot},
    render::worker::{RenderRequest, RenderResult},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// Whether an external editor should receive a position from the displayed diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorPosition {
    VisibleHunk,
    Default,
}

/// Work the terminal loop must perform after an interactive transition.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    pub redraw: bool,
    pub render: bool,
    pub collect: bool,
    pub toggle_stage: bool,
    pub open_discard: bool,
    pub confirm_discard: bool,
    pub quit: bool,
    pub open_editor: Option<EditorPosition>,
}
impl Effects {
    fn draw(render: bool) -> Self {
        Self {
            redraw: true,
            render,
            ..Self::default()
        }
    }
}

impl State {
    /// Select a visible tree row without resetting an already selected diff.
    pub(crate) fn select_tree_row(&mut self, index: usize, now: Instant) -> Effects {
        let Some(row) = self.tree.rows(false).get(index) else {
            return Effects::default();
        };
        let changed = self.selection != row.path;
        if changed {
            self.selection.clone_from(&row.path);
            self.diff.scroll = 0;
        }
        if !row.directory {
            self.activity.visit(&self.selection, now);
        }
        self.focus = Scope::Tree;
        Effects::draw(changed)
    }

    /// Toggle a visible directory, retaining selection unless it becomes hidden.
    pub(crate) fn toggle_tree_directory(&mut self, index: usize) -> Effects {
        let Some(row) = self.tree.rows(false).get(index).filter(|row| row.directory) else {
            return Effects::default();
        };
        if !self.collapsed.remove(&row.path) {
            self.collapsed.insert(row.path.clone());
        }
        self.tree.reveal(&self.collapsed);
        let hidden = !self
            .tree
            .rows(false)
            .iter()
            .any(|row| row.path == self.selection);
        self.reconcile_selection();
        self.focus = Scope::Tree;
        Effects::draw(hidden)
    }

    pub(crate) fn show_toast(&mut self, message: String, now: Instant) {
        self.toast = Some(Toast {
            message,
            expires_at: now + TOAST_DURATION,
        });
    }

    pub fn toast_delay(&self, now: Instant) -> Option<Duration> {
        self.toast
            .as_ref()
            .map(|toast| toast.expires_at.saturating_duration_since(now))
    }

    /// Clear an expired notification without recollecting or rendering the diff.
    pub fn expire_toast(&mut self, now: Instant) -> Effects {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| now >= toast.expires_at)
        {
            self.toast = None;
            Effects::draw(false)
        } else {
            Effects::default()
        }
    }

    /// Time until a pending filter edit should be applied.
    pub fn filter_delay(&self, now: Instant) -> Option<Duration> {
        self.prompt
            .as_ref()
            .and_then(|prompt| prompt.filter_deadline)
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    /// Apply the latest filter edit once its debounce delay has elapsed.
    pub fn apply_pending_filter(&mut self, now: Instant) -> Effects {
        let Some(prompt) = &mut self.prompt else {
            return Effects::default();
        };
        if !prompt
            .filter_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            return Effects::default();
        }
        prompt.filter_deadline = None;
        Effects::draw(self.apply_prompt())
    }

    pub(crate) fn highlight_activity(&mut self, paths: &BTreeSet<PathBuf>, now: Instant) {
        self.activity.highlight(paths, now);
    }

    pub(crate) fn observe_activity(&mut self, observations: ActivityObservations, now: Instant) {
        self.activity.observe(Some(observations), now);
    }

    pub(crate) fn internal_activity(
        &mut self,
        paths: &BTreeSet<PathBuf>,
        observations: ActivityObservations,
    ) {
        self.activity.rebase(paths, observations);
    }

    pub fn collection_started(&mut self, generation: u64) {
        self.collection.started(generation);
    }

    pub fn collection_finished(
        &mut self,
        generation: u64,
        result: Result<Snapshot, GitError>,
        now: Instant,
    ) -> Effects {
        let Some(result) = self.collection.finish(generation, result) else {
            return Effects::default();
        };
        let Ok(mut snapshot) = result else {
            return Effects::draw(false);
        };
        self.activity.observe(snapshot.activity.take(), now);
        if self.collection.take_whitespace_notification()
            && self.snapshot.whitespace != snapshot.whitespace
        {
            self.show_toast(
                format!(
                    "Whitespace: {} · {} hidden by whitespace",
                    snapshot.whitespace.name(),
                    snapshot.whitespace_hidden_files,
                ),
                now,
            );
        }
        let render = self.snapshot.as_ref() != &snapshot;
        if render {
            self.diff
                .invalidate(self.snapshot.whitespace != snapshot.whitespace);
            self.update_snapshot(snapshot);
        }
        Effects::draw(render)
    }

    pub fn render_request(&mut self, width: usize, height: usize) -> RenderRequest {
        self.diff.request(
            Arc::clone(&self.snapshot),
            self.selected_changes(),
            &self.selection,
            width,
            height,
        )
    }

    pub fn apply_render(&mut self, result: RenderResult, height: usize) -> bool {
        self.diff.apply_render(result, height, &self.selection)
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        bindings: &Bindings,
        height: usize,
        now: Instant,
    ) -> Effects {
        if self.discard.is_some() {
            if key.kind != ratatui::crossterm::event::KeyEventKind::Press {
                return Effects::default();
            }
            let input = bindings.action(Scope::Input, key);
            let action = input
                .filter(|action| matches!(action, Action::Accept | Action::Cancel))
                .or_else(|| bindings.action(self.focus, key));
            return action
                .map(|action| self.discard_action(action))
                .unwrap_or_default();
        }
        let scope = if self.prompt.is_some() {
            Scope::Input
        } else {
            self.focus
        };
        match bindings.action(scope, key) {
            Some(action) => self.action(action, height, now),
            None if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                match key.code {
                    KeyCode::Char(ch) => self.paste(ch.encode_utf8(&mut [0; 4]), now),
                    _ => Effects::default(),
                }
            }
            None => Effects::default(),
        }
    }

    pub fn paste(&mut self, text: &str, now: Instant) -> Effects {
        if self.discard.is_some() {
            return Effects::default();
        }
        let Some(prompt) = &mut self.prompt else {
            return Effects::default();
        };
        let changed = prompt.editor.insert(text);
        Effects::draw(changed && self.prompt_edited(now))
    }

    pub fn action(&mut self, action: Action, height: usize, now: Instant) -> Effects {
        if self.discard.is_some() {
            return self.discard_action(action);
        }
        if action == Action::Quit {
            return Effects {
                quit: true,
                ..Effects::default()
            };
        }
        if self.prompt.is_some() {
            let changed = match action {
                Action::Accept => self.finish_prompt(true),
                Action::Cancel => self.finish_prompt(false),
                Action::NextFindMatch => self.next_find_match(),
                action => self.input_action(action, now),
            };
            return Effects::draw(changed);
        }
        if self.help {
            let redraw = match action {
                Action::Cancel | Action::Help => {
                    self.help = false;
                    true
                }
                Action::Down => self.scroll_help(1, true),
                Action::Up => self.scroll_help(1, false),
                Action::PageDown => self.scroll_help(self.help_height.max(1), true),
                Action::PageUp => self.scroll_help(self.help_height.max(1), false),
                Action::HalfDown => self.scroll_help((self.help_height / 2).max(1), true),
                Action::HalfUp => self.scroll_help((self.help_height / 2).max(1), false),
                Action::First => self.scroll_help(usize::MAX, false),
                Action::Last => self.scroll_help(usize::MAX, true),
                _ => false,
            };
            return Effects {
                redraw,
                ..Effects::default()
            };
        }
        let selection = self.selection.clone();
        let mut effects = Effects::draw(false);
        match action {
            Action::Sidebar => {
                self.sidebar = !self.sidebar;
                self.focus = if self.sidebar {
                    Scope::Tree
                } else {
                    Scope::Diff
                };
                effects.render = true;
            }
            Action::Layout => {
                self.diff.split = !self.diff.split;
                effects.render = true;
            }
            Action::LineNumbers => {
                self.diff.show_line_numbers = !self.diff.show_line_numbers;
                effects.render = true;
            }
            Action::Whitespace => {
                self.collection.toggle_whitespace();
                effects.collect = true;
            }
            Action::Focus if self.sidebar => {
                self.focus = if self.focus == Scope::Tree {
                    Scope::Diff
                } else {
                    Scope::Tree
                };
            }
            Action::NextFile => self.next_file(true),
            Action::PreviousFile => self.next_file(false),
            Action::Down | Action::Up => {
                let forward = action == Action::Down;
                if self.focus == Scope::Tree {
                    self.move_tree(forward);
                } else {
                    self.scroll_by(1, forward, height);
                }
            }
            Action::Left => self.collapse(),
            Action::Right => self.enter_directory(),
            Action::HalfDown | Action::HalfUp => {
                self.scroll_by((height / 2).max(1), action == Action::HalfDown, height)
            }
            Action::PageDown | Action::PageUp => {
                // Leave room for the sticky header on the destination page.
                let page = height
                    .saturating_sub(crate::render::FILE_HEADER_ROWS)
                    .max(1);
                self.scroll_by(page, action == Action::PageDown, height)
            }
            Action::First => {
                if self.focus == Scope::Tree {
                    self.selection.clear();
                } else {
                    self.diff.scroll = 0;
                }
            }
            Action::Last => {
                if self.focus == Scope::Tree {
                    if let Some(row) = self.tree.rows(false).last() {
                        self.selection.clone_from(&row.path);
                    }
                } else {
                    self.diff.scroll = self.diff.document.rows.len().saturating_sub(height);
                }
            }
            Action::Filter => self.begin_prompt(PromptKind::Filter),
            Action::Find => self.begin_prompt(PromptKind::Find),
            Action::Search => self.begin_prompt(PromptKind::Search),
            Action::NextMatch => self.jump_match(true, height),
            Action::PreviousMatch => self.jump_match(false, height),
            Action::Cancel => {
                if !self.diff.search.is_empty() {
                    self.diff.search.clear();
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.refilter();
                }
                effects.render = true;
            }
            Action::Accept => {
                if self
                    .snapshot
                    .changes
                    .iter()
                    .any(|c| c.path == self.selection && c.oversized)
                {
                    self.collection.loaded.insert(self.selection.clone());
                    effects.collect = true;
                } else {
                    self.expand();
                }
            }
            Action::ToggleStage => effects.toggle_stage = true,
            Action::Discard => effects.open_discard = true,
            Action::Refresh => effects.collect = true,
            Action::OpenEditor => effects.open_editor = Some(EditorPosition::VisibleHunk),
            Action::OpenEditorPlain => effects.open_editor = Some(EditorPosition::Default),
            Action::Help => {
                self.help = true;
                self.help_scroll = 0;
            }
            _ => {}
        }
        let navigated_to_file = self.selection != selection
            && matches!(
                action,
                Action::NextFile
                    | Action::PreviousFile
                    | Action::Up
                    | Action::Down
                    | Action::Left
                    | Action::Right
                    | Action::First
                    | Action::Last
            );
        if (navigated_to_file || action == Action::Accept)
            && self
                .tree
                .rows(false)
                .iter()
                .any(|row| row.path == self.selection && !row.directory)
        {
            self.activity.visit(&self.selection, now);
        }
        effects.render |= self.selection != selection;
        effects
    }
}

#[cfg(test)]
mod tests;
