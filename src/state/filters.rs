use super::{Effects, State};
use crate::model::{Category, Change};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{collections::BTreeSet, path::PathBuf};

pub(crate) const STATUSES: [(char, &str); 8] = [
    ('M', "Modified"),
    ('A', "Added"),
    ('D', "Deleted"),
    ('R', "Renamed"),
    ('C', "Copied"),
    ('T', "Type changed"),
    ('U', "Unmerged/conflict"),
    ('?', "Untracked"),
];

#[derive(Clone, Default)]
pub(crate) struct ChangeFilter {
    pub categories: [bool; 2],
    pub statuses: [bool; 8],
}

impl ChangeFilter {
    pub fn matches(&self, change: &Change) -> bool {
        let category = match change.category {
            Category::Unstaged | Category::Untracked => self.categories[0],
            Category::Staged => self.categories[1],
            Category::Comparison => true,
        };
        (!self.categories.contains(&true) || category)
            && (!self.statuses.contains(&true)
                || !self.statuses.contains(&false)
                || STATUSES
                    .iter()
                    .zip(self.statuses)
                    .any(|((status, _), selected)| selected && *status == change.status))
    }

    pub fn selected(&self) -> bool {
        self.categories.contains(&true) || self.statuses.contains(&true)
    }

    pub fn restricted(&self) -> bool {
        (self.categories.contains(&true) && self.categories.contains(&false))
            || (self.statuses.contains(&true) && self.statuses.contains(&false))
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        match self.categories {
            [true, true] => parts.push("Both categories".into()),
            [true, false] => parts.push("Unstaged/Untracked".into()),
            [false, true] => parts.push("Staged".into()),
            _ => {}
        }
        if !self.statuses.contains(&false) {
            parts.push("All statuses".into());
        } else if self.statuses.contains(&true) {
            parts.push(
                STATUSES
                    .iter()
                    .zip(self.statuses)
                    .filter(|(_, selected)| *selected)
                    .map(|((status, _), _)| status.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        parts.join(" · ")
    }
}

pub(crate) struct FilterMenu {
    pub selected: usize,
    previous: ChangeFilter,
    selection: PathBuf,
    collapsed: BTreeSet<PathBuf>,
    unfiltered_collapsed: Option<BTreeSet<PathBuf>>,
}

impl State {
    pub(super) fn refilter(&mut self) {
        let active = !self.filter.is_empty() || self.change_filter.restricted();
        if !active {
            if let Some(collapsed) = self.views.unfiltered_mut().take() {
                *self.views.collapsed_mut() = collapsed;
            }
        } else if self.views.unfiltered().is_none() {
            *self.views.unfiltered_mut() = Some(self.views.collapsed().clone());
        }
        self.tree
            .filter_changes(&self.snapshot, &self.change_filter);
        self.tree.refilter(&self.filter, self.views.collapsed());
        if active {
            let mut revealed = false;
            for row in self.tree.rows(true).iter().filter(|row| row.directory) {
                for ancestor in row.path.ancestors() {
                    revealed |= self.views.collapsed_mut().remove(ancestor);
                }
            }
            if revealed {
                self.tree.reveal(self.views.collapsed());
            }
        }
        self.reconcile_selection();
    }

    pub(crate) fn set_comparison(&mut self, comparison: bool) {
        self.comparison = comparison;
    }

    pub(crate) fn change_filter(&self) -> &ChangeFilter {
        &self.change_filter
    }
    pub(crate) fn filter_menu(&self) -> Option<&FilterMenu> {
        self.filter_menu.as_ref()
    }
    pub(crate) fn comparison(&self) -> bool {
        self.comparison
    }

    pub(super) fn begin_change_filter(&mut self) {
        self.filter_menu = Some(FilterMenu {
            selected: 0,
            previous: self.change_filter.clone(),
            selection: self.selection.clone(),
            collapsed: self.views.collapsed().clone(),
            unfiltered_collapsed: self.views.unfiltered().clone(),
        });
    }

    pub(super) fn change_filter_key(&mut self, key: KeyEvent) -> Effects {
        if key.kind != KeyEventKind::Press {
            return Effects::default();
        }
        let count = if self.comparison { 8 } else { 10 };
        let Some(menu) = &mut self.filter_menu else {
            return Effects::default();
        };
        match key.code {
            KeyCode::Esc => {
                if let Some(menu) = self.filter_menu.take() {
                    self.change_filter = menu.previous;
                    self.selection = menu.selection;
                    *self.views.collapsed_mut() = menu.collapsed;
                    *self.views.unfiltered_mut() = menu.unfiltered_collapsed;
                    self.tree
                        .filter_changes(&self.snapshot, &self.change_filter);
                    self.tree.refilter(&self.filter, self.views.collapsed());
                    self.reconcile_selection();
                }
                Effects::draw(true)
            }
            KeyCode::Enter => {
                self.filter_menu = None;
                Effects::draw(false)
            }
            KeyCode::Up => {
                menu.selected = (menu.selected + count - 1) % count;
                Effects::draw(false)
            }
            KeyCode::Down => {
                menu.selected = (menu.selected + 1) % count;
                Effects::draw(false)
            }
            KeyCode::Tab | KeyCode::BackTab => {
                menu.selected = if self.comparison || menu.selected >= 2 {
                    0
                } else {
                    2
                };
                Effects::draw(false)
            }
            KeyCode::Char(' ') if key.modifiers.is_empty() => {
                let selected = if !self.comparison && menu.selected < 2 {
                    &mut self.change_filter.categories[menu.selected]
                } else {
                    &mut self.change_filter.statuses
                        [menu.selected - if self.comparison { 0 } else { 2 }]
                };
                *selected = !*selected;
                self.refilter();
                Effects::draw(true)
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                self.change_filter = ChangeFilter::default();
                self.refilter();
                Effects::draw(true)
            }
            _ => Effects::default(),
        }
    }
}
