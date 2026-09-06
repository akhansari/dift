use super::{PromptKind, State};
use crate::{
    config::Whitespace,
    git::GitError,
    input::{Action, Bindings, Scope},
    model::Snapshot,
    render::worker::{RenderRequest, RenderResult},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// Work the terminal loop must perform after an interactive transition.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    pub redraw: bool,
    pub render: bool,
    pub collect: bool,
    pub quit: bool,
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

#[derive(Default)]
pub(super) struct Displayed {
    selection: PathBuf,
    query: String,
    pending_search_jump: bool,
}

impl State {
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

    pub fn collection_started(&mut self, generation: u64) {
        if generation == self.collection_generation {
            self.collecting = true;
        }
    }

    pub fn collection_finished(
        &mut self,
        generation: u64,
        result: Result<Snapshot, GitError>,
    ) -> Effects {
        if generation != self.collection_generation {
            return Effects::default();
        }
        self.collecting = false;
        let mut render = false;
        match result {
            Ok(snapshot) => {
                render = self.snapshot.as_ref() != &snapshot;
                if render {
                    if self.snapshot.whitespace != snapshot.whitespace {
                        // Do not label the old diff with the newly applied mode. Keep
                        // its anchors until the first matching preview can reanchor.
                        if self.transition_document.is_none() {
                            self.transition_document = Some(Arc::clone(&self.document));
                        }
                        self.document = Arc::new(crate::render::Document::default());
                        self.match_index = None;
                    }
                    // Prevent a render from the previous snapshot being accepted while
                    // the terminal loop arranges the new render request.
                    self.generation += 1;
                    self.update_snapshot(snapshot);
                }
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        Effects::draw(render)
    }

    pub fn render_request(&mut self, width: usize, height: usize) -> RenderRequest {
        self.generation += 1;
        self.rendering = true;
        self.displayed.pending_search_jump |=
            self.displayed.query != self.search && !self.search.is_empty();
        RenderRequest {
            generation: self.generation,
            snapshot: Arc::clone(&self.snapshot),
            selected: self.selected_changes(),
            width,
            split: self.split,
            query: self.search.clone(),
            // A prefix preview cannot preserve an anchor beyond that prefix.
            preview_rows: if self.displayed.selection == self.selection && self.scroll > 0 {
                0
            } else {
                height.saturating_mul(2).max(16)
            },
        }
    }

    pub fn apply_render(&mut self, result: RenderResult, height: usize) -> bool {
        if result.generation != self.generation
            || (result.document.partial
                && self.displayed.selection == self.selection
                && self.scroll > 0)
        {
            // Scrolling can happen after requesting a prefix preview. Keep the
            // current source position until a complete document is available.
            return false;
        }
        self.scroll = if self.displayed.selection == self.selection {
            result.document.reanchored_scroll(
                self.transition_document.as_ref().unwrap_or(&self.document),
                self.scroll,
            )
        } else {
            0
        }
        .min(result.document.rows.len().saturating_sub(height));
        self.document = result.document;
        self.transition_document = None;
        self.rendering = !result.finished;
        if self.displayed.query != self.search || self.displayed.selection != self.selection {
            self.match_index = None;
        } else if let Some(index) = self.match_index {
            self.match_index = (!self.document.hits.is_empty())
                .then_some(index.min(self.document.hits.len().saturating_sub(1)));
        }
        if self.displayed.pending_search_jump
            && !self.search.is_empty()
            && !self.document.hits.is_empty()
        {
            self.match_index = None;
            self.jump_match(true, height);
            self.displayed.pending_search_jump = false;
        }
        if result.finished || self.search.is_empty() {
            self.displayed.pending_search_jump = false;
        }
        self.displayed.selection.clone_from(&self.selection);
        self.displayed.query.clone_from(&self.search);
        true
    }
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        bindings: &Bindings,
        height: usize,
        now: Instant,
    ) -> Effects {
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
        let Some(prompt) = &mut self.prompt else {
            return Effects::default();
        };
        let changed = prompt.editor.insert(text);
        Effects::draw(changed && self.prompt_edited(now))
    }

    pub fn action(&mut self, action: Action, height: usize, now: Instant) -> Effects {
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
                action => self.input_action(action, now),
            };
            return Effects::draw(changed);
        }
        if self.help {
            match action {
                Action::Cancel | Action::Help => self.help = false,
                Action::Down => self.help_scroll = self.help_scroll.saturating_add(1),
                Action::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                Action::PageDown => self.help_scroll = self.help_scroll.saturating_add(height / 2),
                Action::PageUp => self.help_scroll = self.help_scroll.saturating_sub(height / 2),
                _ => {}
            }
            return Effects::draw(false);
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
                self.split = !self.split;
                effects.render = true;
            }
            Action::Whitespace => {
                self.whitespace = if self.whitespace == Whitespace::Exact {
                    self.ignore_whitespace
                } else {
                    Whitespace::Exact
                };
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
                self.scroll_by(height, action == Action::PageDown, height)
            }
            Action::First => {
                if self.focus == Scope::Tree {
                    self.selection.clear();
                } else {
                    self.scroll = 0;
                }
            }
            Action::Last => {
                if self.focus == Scope::Tree {
                    if let Some(row) = self.tree.rows(false).last() {
                        self.selection.clone_from(&row.path);
                    }
                } else {
                    self.scroll = self.document.rows.len().saturating_sub(height);
                }
            }
            Action::Filter => self.begin_prompt(PromptKind::Filter),
            Action::Search => self.begin_prompt(PromptKind::Search),
            Action::NextMatch => self.jump_match(true, height),
            Action::PreviousMatch => self.jump_match(false, height),
            Action::Cancel => {
                if !self.search.is_empty() {
                    self.search.clear();
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
                    self.loaded.insert(self.selection.clone());
                    effects.collect = true;
                } else {
                    self.expand();
                }
            }
            Action::Refresh => effects.collect = true,
            Action::Help => {
                self.help = true;
                self.help_scroll = 0;
            }
            _ => {}
        }
        effects.render |= self.selection != selection;
        effects
    }
}

#[cfg(test)]
mod tests;
