use super::{Effects, State};
use crate::input::{Action, Editor};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    time::{Duration, Instant},
};

const FILTER_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Filter,
    Find,
    Search,
}

/// Tree state restored when a navigation prompt is cancelled.
#[derive(Debug, Clone)]
struct TreeRollback {
    selection: PathBuf,
    collapsed: BTreeSet<PathBuf>,
    unfiltered_collapsed: Option<BTreeSet<PathBuf>>,
}

impl TreeRollback {
    fn capture(state: &State) -> Self {
        Self {
            selection: state.selection.clone(),
            collapsed: state.collapsed.clone(),
            unfiltered_collapsed: state.unfiltered_collapsed.clone(),
        }
    }

    fn restore(self, state: &mut State) {
        state.selection = self.selection;
        state.collapsed = self.collapsed;
        state.unfiltered_collapsed = self.unfiltered_collapsed;
    }
}

#[derive(Debug, Clone)]
enum PromptState {
    Filter {
        previous: String,
        tree: TreeRollback,
        deadline: Option<Instant>,
    },
    Find {
        tree: TreeRollback,
        matches: Vec<PathBuf>,
        index: usize,
    },
    Search {
        previous: String,
    },
}

/// An editor and the rollback/navigation data specific to its prompt kind.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub(super) editor: Editor,
    state: PromptState,
}

impl Prompt {
    pub fn kind(&self) -> PromptKind {
        match self.state {
            PromptState::Filter { .. } => PromptKind::Filter,
            PromptState::Find { .. } => PromptKind::Find,
            PromptState::Search { .. } => PromptKind::Search,
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }
}

impl State {
    pub(super) fn begin_prompt(&mut self, kind: PromptKind) {
        let (text, state) = match kind {
            PromptKind::Filter => (
                &self.filter[..],
                PromptState::Filter {
                    previous: self.filter.clone(),
                    tree: TreeRollback::capture(self),
                    deadline: None,
                },
            ),
            PromptKind::Find => (
                "",
                PromptState::Find {
                    tree: TreeRollback::capture(self),
                    matches: Vec::new(),
                    index: 0,
                },
            ),
            PromptKind::Search => (
                &self.diff.search[..],
                PromptState::Search {
                    previous: self.diff.search.clone(),
                },
            ),
        };
        self.prompt = Some(Prompt {
            editor: Editor::new(text.to_owned()),
            state,
        });
    }

    pub(super) fn prompt_edited(&mut self, now: Instant) -> bool {
        if let Some(Prompt {
            editor,
            state: PromptState::Filter { deadline, .. },
        }) = &mut self.prompt
        {
            *deadline = (editor.text() != self.filter).then_some(now + FILTER_DEBOUNCE);
            return false;
        }
        self.apply_prompt()
    }

    fn apply_prompt(&mut self) -> bool {
        let Some(prompt) = &self.prompt else {
            return false;
        };
        let filter = match prompt.state {
            PromptState::Find { .. } => return self.apply_find(),
            PromptState::Filter { .. } => true,
            PromptState::Search { .. } => false,
        };
        let query = if filter {
            &mut self.filter
        } else {
            &mut self.diff.search
        };
        let changed = query != prompt.editor.text();
        if changed {
            prompt.editor.text().clone_into(query);
            if filter {
                self.refilter();
            }
        }
        changed
    }

    fn apply_find(&mut self) -> bool {
        let Some(Prompt {
            editor,
            state:
                PromptState::Find {
                    tree,
                    matches,
                    index,
                },
        }) = &mut self.prompt
        else {
            return false;
        };
        matches.clear();
        *index = 0;
        let query = editor.text();
        if query.is_empty() {
            let changed = self.selection != tree.selection;
            self.selection.clone_from(&tree.selection);
            self.collapsed.clone_from(&tree.collapsed);
            self.unfiltered_collapsed
                .clone_from(&tree.unfiltered_collapsed);
            self.tree.reveal(&self.collapsed);
            if changed {
                self.diff.scroll = 0;
            }
            self.reconcile_selection();
            return changed;
        }
        *matches = self.tree.find_files(query);
        self.select_find_match()
    }

    pub(super) fn refresh_find_matches(&mut self) {
        if let Some(Prompt {
            editor,
            state: PromptState::Find { matches, index, .. },
        }) = &mut self.prompt
            && !editor.text().is_empty()
        {
            *matches = self.tree.find_files(editor.text());
            *index = matches
                .iter()
                .position(|path| *path == self.selection)
                .unwrap_or(0);
            self.select_find_match();
        }
    }

    pub(super) fn next_find_match(&mut self) -> bool {
        let Some(Prompt {
            state: PromptState::Find { matches, index, .. },
            ..
        }) = &mut self.prompt
        else {
            return false;
        };
        if matches.is_empty() {
            return false;
        }
        *index = (*index + 1) % matches.len();
        self.select_find_match()
    }

    fn select_find_match(&mut self) -> bool {
        let Some(Prompt {
            state: PromptState::Find { matches, index, .. },
            ..
        }) = &self.prompt
        else {
            return false;
        };
        let Some(path) = matches.get(*index) else {
            return false;
        };
        let changed = self.selection != *path;
        self.selection.clone_from(path);
        self.reveal_selection();
        if changed {
            self.diff.scroll = 0;
        }
        changed
    }

    pub(super) fn finish_prompt(&mut self, accept: bool) -> bool {
        let mut changed = accept
            && self
                .prompt
                .as_ref()
                .is_some_and(|prompt| prompt.kind() != PromptKind::Find)
            && self.apply_prompt();
        if let Some(prompt) = self.prompt.take()
            && !accept
        {
            match prompt.state {
                PromptState::Filter { previous, tree, .. } => {
                    changed = self.filter != previous || self.selection != tree.selection;
                    self.filter = previous;
                    tree.restore(self);
                    self.tree.refilter(&self.filter, &self.collapsed);
                }
                PromptState::Find { tree, .. } => {
                    changed = self.selection != tree.selection;
                    tree.restore(self);
                    self.tree.reveal(&self.collapsed);
                }
                PromptState::Search { previous } => {
                    changed = self.diff.search != previous;
                    self.diff.search = previous;
                }
            }
        }
        let reconciled = self.reconcile_selection();
        changed || reconciled
    }

    pub(super) fn input_action(&mut self, action: Action, now: Instant) -> bool {
        let changed = self
            .prompt
            .as_mut()
            .is_some_and(|prompt| prompt.editor.edit(action));
        changed && self.prompt_edited(now)
    }

    /// Time until a pending filter edit should be applied.
    pub fn filter_delay(&self, now: Instant) -> Option<Duration> {
        match &self.prompt.as_ref()?.state {
            PromptState::Filter { deadline, .. } => {
                deadline.map(|deadline| deadline.saturating_duration_since(now))
            }
            _ => None,
        }
    }

    /// Apply the latest filter edit once its debounce delay has elapsed.
    pub fn apply_pending_filter(&mut self, now: Instant) -> Effects {
        let Some(Prompt {
            state: PromptState::Filter { deadline, .. },
            ..
        }) = &mut self.prompt
        else {
            return Effects::default();
        };
        if !deadline.is_some_and(|deadline| now >= deadline) {
            return Effects::default();
        }
        *deadline = None;
        Effects::draw(self.apply_prompt())
    }
}
