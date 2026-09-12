use super::{Command, EditorPosition, Effects, PromptKind, State};
use crate::{
    git::GitError,
    input::{Action, Bindings, Scope},
    model::Snapshot,
    render::worker::{RenderRequest, RenderResult},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{sync::Arc, time::Instant};

impl State {
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
        if self.filter_menu.is_some() {
            return self.change_filter_key(key);
        }
        if self.commit_dialog().is_some() {
            return self.commit_key(key, bindings);
        }
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
        if self.filter_menu.is_some() {
            return Effects::default();
        }
        if self.commit_dialog().is_some() {
            return self.commit_paste(text);
        }
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
        if self.commit_dialog().is_some() {
            return self.commit_action(action);
        }
        if self.discard.is_some() {
            return self.discard_action(action);
        }
        if action == Action::Quit {
            return Effects {
                command: Some(Command::Quit),
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
                effects.command = Some(Command::Collect);
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
            Action::ChangeFilter => self.begin_change_filter(),
            Action::Find => self.begin_prompt(PromptKind::Find),
            Action::Search => self.begin_prompt(PromptKind::Search),
            Action::NextMatch => self.jump_match(true, height),
            Action::PreviousMatch => self.jump_match(false, height),
            Action::Cancel => {
                if !self.diff.search.is_empty() {
                    self.diff.search.clear();
                } else if !self.filter.is_empty() || self.change_filter.selected() {
                    self.filter.clear();
                    self.change_filter = super::filters::ChangeFilter::default();
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
                    effects.command = Some(Command::Collect);
                } else {
                    self.expand();
                }
            }
            Action::ToggleStage => effects.command = Some(Command::ToggleStage),
            Action::Discard => effects.command = Some(Command::OpenDiscard),
            Action::Commit => effects.command = Some(Command::OpenCommit),
            Action::Refresh => effects.command = Some(Command::Collect),
            Action::OpenEditor => {
                effects.command = Some(Command::OpenEditor(EditorPosition::VisibleHunk))
            }
            Action::OpenEditorPlain => {
                effects.command = Some(Command::OpenEditor(EditorPosition::Default))
            }
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
