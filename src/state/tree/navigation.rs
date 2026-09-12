use crate::{
    input::Scope,
    state::{Effects, State},
};
use std::time::Instant;

impl State {
    /// Fall back to a surviving ancestor, reporting whether selection changed.
    pub(in crate::state) fn reconcile_selection(&mut self) -> bool {
        let mut changed = false;
        while !self
            .tree
            .rows(false)
            .iter()
            .any(|row| row.path == self.selection)
        {
            if !self.selection.pop() {
                break;
            }
            changed = true;
        }
        if changed {
            self.diff.scroll = 0;
        }
        changed
    }

    pub(in crate::state) fn reveal_selection(&mut self) {
        let mut revealed = false;
        for ancestor in self.selection.ancestors().skip(1) {
            revealed |= self.collapsed.remove(ancestor);
        }
        if revealed {
            self.tree.reveal(&self.collapsed);
        }
    }

    pub(in crate::state) fn next_file(&mut self, forward: bool) {
        let all = self.tree.rows(true);
        let current = all
            .iter()
            .position(|row| row.path == self.selection)
            .unwrap_or(0);
        let candidate = if forward {
            all.iter().skip(current + 1).find(|row| !row.directory)
        } else {
            all[..current].iter().rev().find(|row| !row.directory)
        };
        if let Some(row) = candidate {
            self.selection.clone_from(&row.path);
            self.reveal_selection();
            self.diff.scroll = 0;
        }
    }

    pub(in crate::state) fn move_tree(&mut self, forward: bool) {
        let rows = self.tree.rows(false);
        let index = rows
            .iter()
            .position(|row| row.path == self.selection)
            .unwrap_or(0);
        let next = if forward {
            (index + 1).min(rows.len().saturating_sub(1))
        } else {
            index.saturating_sub(1)
        };
        if let Some(row) = rows.get(next) {
            self.selection.clone_from(&row.path);
            self.diff.scroll = 0;
        }
    }

    pub(in crate::state) fn collapse(&mut self) {
        let is_directory = self
            .tree(false)
            .iter()
            .any(|row| row.path == self.selection && row.directory);
        if is_directory && !self.collapsed.contains(&self.selection) {
            self.collapsed.insert(self.selection.clone());
            self.tree.reveal(&self.collapsed);
        } else if self.selection.pop() {
            self.diff.scroll = 0;
        }
    }

    pub(in crate::state) fn expand(&mut self) {
        if self.collapsed.remove(&self.selection) {
            self.tree.reveal(&self.collapsed);
        }
    }

    pub(in crate::state) fn enter_directory(&mut self) {
        if self.is_collapsed(&self.selection) {
            self.expand();
        } else if let Some(child) = self.tree.rows(false).iter().find(|row| {
            !row.path.as_os_str().is_empty() && row.path.parent() == Some(&self.selection)
        }) {
            self.selection.clone_from(&child.path);
            self.diff.scroll = 0;
        }
    }

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
}
