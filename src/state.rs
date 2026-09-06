mod tree;
use tree::TreeIndex;
pub use tree::TreeRow;
mod update;
use update::Displayed;
pub use update::Effects;

use crate::{
    input::{Action, Editor, Scope},
    model::Snapshot,
    render::Document,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Filter,
    Search,
}
/// An active prompt and the values restored when it is cancelled.
#[derive(Debug, Clone)]
pub struct Prompt {
    /// Whether the prompt edits the file filter or diff search.
    kind: PromptKind,
    /// Text and cursor being edited.
    editor: Editor,
    /// Filter or search text from before the prompt opened.
    previous: String,
    /// Selected path from before the prompt opened.
    previous_selection: PathBuf,
}

impl Prompt {
    pub fn kind(&self) -> PromptKind {
        self.kind
    }
    pub fn editor(&self) -> &Editor {
        &self.editor
    }
}

/// Repository data and interactive state for the terminal UI.
pub struct State {
    tree: TreeIndex,
    displayed: Displayed,
    help_scroll: usize,
    /// Most recently collected repository changes.
    snapshot: Arc<Snapshot>,
    /// Selected file or directory; an empty path selects the root.
    selection: PathBuf,
    /// Directories whose children are hidden in the tree.
    collapsed: BTreeSet<PathBuf>,
    /// Fuzzy query used to filter file paths.
    filter: String,
    /// Query used to find matches in the diff.
    search: String,
    /// Active filter or search prompt, if any.
    prompt: Option<Prompt>,
    /// Input scope currently receiving keyboard actions.
    focus: Scope,
    /// Whether the file sidebar is visible.
    sidebar: bool,
    /// Whether the diff uses a side-by-side layout.
    split: bool,
    /// Whether the help overlay is visible.
    help: bool,
    /// Zero-based document row at the top of the diff viewport.
    scroll: usize,
    /// Rendered diff currently displayed.
    document: Arc<Document>,
    /// Paths explicitly allowed to load beyond size limits.
    loaded: BTreeSet<PathBuf>,
    /// Latest render request identifier used to reject stale results.
    generation: u64,
    /// Most recent repository collection error, if any.
    error: Option<String>,
    /// Whether a repository scan is in progress.
    collecting: bool,
    /// Whether the latest render request is still in progress.
    rendering: bool,
    /// Index of the active search hit in the document.
    match_index: Option<usize>,
}

impl State {
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub fn selection(&self) -> &Path {
        &self.selection
    }
    pub fn is_collapsed(&self, path: &Path) -> bool {
        self.collapsed.contains(path)
    }
    pub fn filter(&self) -> &str {
        &self.filter
    }
    pub fn search(&self) -> &str {
        &self.search
    }
    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn loaded(&self) -> &BTreeSet<PathBuf> {
        &self.loaded
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn help_scroll(&self) -> usize {
        self.help_scroll
    }
    pub fn focus(&self) -> Scope {
        self.focus
    }
    pub fn sidebar(&self) -> bool {
        self.sidebar
    }
    pub fn split(&self) -> bool {
        self.split
    }
    pub fn help(&self) -> bool {
        self.help
    }
    pub fn scroll(&self) -> usize {
        self.scroll
    }
    pub fn collecting(&self) -> bool {
        self.collecting
    }
    pub fn rendering(&self) -> bool {
        self.rendering
    }
    pub fn match_index(&self) -> Option<usize> {
        self.match_index
    }
    pub fn new(split: bool) -> Self {
        let snapshot = Arc::new(Snapshot::default());
        let collapsed = BTreeSet::new();
        let tree = TreeIndex::new(&snapshot, "", &collapsed);
        Self {
            tree,
            displayed: Displayed::default(),
            help_scroll: 0,
            snapshot,
            selection: PathBuf::new(),
            collapsed,
            filter: String::new(),
            search: String::new(),
            prompt: None,
            focus: Scope::Tree,
            sidebar: true,
            split,
            help: false,
            scroll: 0,
            document: Arc::new(Document::default()),
            loaded: BTreeSet::new(),
            generation: 0,
            error: None,
            collecting: false,
            rendering: false,
            match_index: None,
        }
    }
    pub fn update_snapshot(&mut self, snapshot: Snapshot) {
        self.tree = TreeIndex::new(&snapshot, &self.filter, &self.collapsed);
        self.snapshot = Arc::new(snapshot);
        self.reconcile_selection();
    }
    fn reconcile_selection(&mut self) {
        while !self.tree.contains_selection(&self.selection) {
            self.selection.pop();
        }
    }
    pub fn matching_paths(&self) -> &BTreeSet<PathBuf> {
        self.tree.matching_paths()
    }
    pub fn selected_changes(&self) -> Vec<usize> {
        self.tree.selected_changes(&self.selection)
    }
    pub fn tree(&self, include_collapsed: bool) -> &[TreeRow] {
        self.tree.rows(include_collapsed)
    }
    fn next_file(&mut self, forward: bool) {
        let all = self.tree.rows(true);
        let current = all
            .iter()
            .position(|r| r.path == self.selection)
            .unwrap_or(0);
        let candidate = if forward {
            all.iter().skip(current + 1).find(|r| !r.directory)
        } else {
            all[..current].iter().rev().find(|r| !r.directory)
        };
        if let Some(row) = candidate {
            self.selection.clone_from(&row.path);
            let mut revealed = false;
            for ancestor in row.path.ancestors().skip(1) {
                revealed |= self.collapsed.remove(ancestor);
            }
            self.scroll = 0;
            if revealed {
                self.tree.reveal(&self.collapsed);
            }
        }
    }
    fn move_tree(&mut self, forward: bool) {
        let rows = self.tree.rows(false);
        let index = rows
            .iter()
            .position(|r| r.path == self.selection)
            .unwrap_or(0);
        let next = if forward {
            (index + 1).min(rows.len().saturating_sub(1))
        } else {
            index.saturating_sub(1)
        };
        if let Some(row) = rows.get(next) {
            self.selection.clone_from(&row.path);
            self.scroll = 0;
        }
    }
    fn collapse(&mut self) {
        let is_directory = self
            .tree(false)
            .iter()
            .any(|r| r.path == self.selection && r.directory);
        if is_directory && !self.collapsed.contains(&self.selection) {
            self.collapsed.insert(self.selection.clone());
            self.tree.reveal(&self.collapsed);
        } else if self.selection.pop() {
            self.scroll = 0;
        }
    }
    fn expand(&mut self) {
        if self.collapsed.remove(&self.selection) {
            self.tree.reveal(&self.collapsed);
        }
    }
    fn begin_prompt(&mut self, kind: PromptKind) {
        let text = if kind == PromptKind::Filter {
            &self.filter
        } else {
            &self.search
        };
        self.prompt = Some(Prompt {
            kind,
            editor: Editor::new(text.clone()),
            previous: text.clone(),
            previous_selection: self.selection.clone(),
        });
    }
    fn sync_prompt(&mut self) -> bool {
        let mut changed = false;
        if let Some(prompt) = &self.prompt {
            let query = match prompt.kind {
                PromptKind::Filter => &mut self.filter,
                PromptKind::Search => &mut self.search,
            };
            changed = query != prompt.editor.text();
            if changed {
                prompt.editor.text().clone_into(query);
                if prompt.kind == PromptKind::Filter {
                    self.tree.refilter(&self.filter, &self.collapsed);
                }
            }
        }
        if !self.tree.contains_selection(&self.selection) {
            self.selection.clear();
            changed = true;
        }
        changed
    }
    fn finish_prompt(&mut self, accept: bool) -> bool {
        let mut changed = false;
        if let Some(prompt) = self.prompt.take()
            && !accept
        {
            if prompt.kind == PromptKind::Filter {
                changed =
                    self.filter != prompt.previous || self.selection != prompt.previous_selection;
                if self.filter != prompt.previous {
                    self.filter = prompt.previous;
                    self.tree.refilter(&self.filter, &self.collapsed);
                }
                self.selection = prompt.previous_selection;
            } else {
                changed = self.search != prompt.previous;
                self.search = prompt.previous;
            }
        }
        let selection = self.selection.clone();
        self.reconcile_selection();
        changed || self.selection != selection
    }
    pub fn scroll_by(&mut self, amount: usize, down: bool, height: usize) {
        self.scroll = if down {
            self.scroll.saturating_add(amount)
        } else {
            self.scroll.saturating_sub(amount)
        }
        .min(self.document.rows.len().saturating_sub(height));
    }
    pub fn jump_match(&mut self, forward: bool, height: usize) {
        let hits = &self.document.hits;
        if hits.is_empty() {
            return;
        }
        let index = match self.match_index {
            Some(i) if forward => (i + 1) % hits.len(),
            Some(i) => (i + hits.len() - 1) % hits.len(),
            None if forward => hits.iter().position(|h| h.row >= self.scroll).unwrap_or(0),
            None => hits
                .iter()
                .rposition(|h| h.row < self.scroll)
                .unwrap_or(hits.len() - 1),
        };
        self.match_index = Some(index);
        self.scroll = hits[index]
            .row
            .saturating_sub(height / 3)
            .min(self.document.rows.len().saturating_sub(height));
    }
    fn input_action(&mut self, action: Action) -> bool {
        if let Some(prompt) = &mut self.prompt {
            prompt.editor.edit(action);
        }
        self.sync_prompt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Category, Change};
    fn snapshot() -> Snapshot {
        snapshot_with(&[
            ("src/a.rs", Category::Unstaged),
            ("src/a.rs", Category::Staged),
            ("src/b.rs", Category::Untracked),
            ("tests/c.rs", Category::Staged),
        ])
    }
    fn snapshot_with(files: &[(&str, Category)]) -> Snapshot {
        Snapshot {
            branch: "main".into(),
            changes: files
                .iter()
                .map(|(path, category)| Change {
                    path: (*path).into(),
                    old_path: None,
                    category: *category,
                    status: 'M',
                    hunks: Vec::new(),
                    summary: None,
                    oversized: false,
                    old_source: None,
                    new_source: None,
                })
                .collect(),
        }
    }
    fn paths(state: &State) -> Vec<&Path> {
        state
            .tree(false)
            .iter()
            .map(|row| row.path.as_path())
            .collect()
    }
    fn state() -> State {
        let mut state = State::new(false);
        state.update_snapshot(snapshot());
        state
    }
    #[test]
    fn a_path_with_two_statuses_has_one_tree_entry() {
        // Given
        let state = state();
        // When
        let rows = state.tree(false);
        // Then
        assert_eq!(
            rows.iter()
                .filter(|r| r.path == Path::new("src/a.rs"))
                .count(),
            1
        );
    }
    #[test]
    fn selecting_a_directory_includes_every_status() {
        // Given
        let mut state = state();
        state.action(Action::Down, 10);
        // When
        let selected = state.selected_changes();
        // Then
        assert_eq!(selected, vec![0, 1, 2]);
    }
    #[test]
    fn next_file_reveals_a_collapsed_path() {
        // Given
        let mut state = state();
        state.action(Action::Down, 10);
        state.action(Action::Left, 10);
        // When
        state.action(Action::NextFile, 10);
        // Then
        assert_eq!(state.selection(), Path::new("src/a.rs"));
        assert!(!state.is_collapsed(Path::new("src")));
        assert!(paths(&state).contains(&Path::new("src/a.rs")));
    }
    #[test]
    fn cancel_filter_restores_query_and_selection() {
        // Given
        let mut state = state();
        state.action(Action::NextFile, 10);
        state.action(Action::Filter, 10);
        state.paste("tests");
        // When
        state.action(Action::Cancel, 10);
        // Then
        assert_eq!(state.filter(), "");
        assert_eq!(state.selection(), Path::new("src/a.rs"));
        assert_eq!(state.matching_paths().len(), 3);
        assert_eq!(state.tree(false).len(), 6);
    }
    #[test]
    fn vanished_file_selects_its_surviving_parent() {
        // Given
        let mut initial = snapshot();
        initial.changes[0].path = "src/0-gone.rs".into();
        let mut state = State::new(false);
        state.update_snapshot(initial);
        state.action(Action::NextFile, 10);
        // When
        state.update_snapshot(snapshot());
        // Then
        assert_eq!(state.selection(), Path::new("src"));
    }

    #[test]
    fn refreshing_keeps_selection_queries_and_collapsed_directories() {
        // Given
        let mut state = state();
        state.action(Action::Last, 10);
        state.action(Action::Left, 10);
        state.action(Action::Left, 10);
        state.action(Action::First, 10);
        state.action(Action::NextFile, 10);
        state.action(Action::Filter, 10);
        state.paste("src");
        state.action(Action::Accept, 10);
        state.action(Action::Search, 10);
        state.paste("needle");
        state.action(Action::Accept, 10);
        let mut changed = snapshot();
        changed.changes[0].summary = Some("updated content".into());
        // When
        state.update_snapshot(changed);
        // Then
        assert_eq!(state.selection(), Path::new("src/a.rs"));
        assert_eq!(state.filter(), "src");
        assert_eq!(state.search(), "needle");
        assert!(state.is_collapsed(Path::new("tests")));
        assert_eq!(state.selected_changes(), vec![0, 1]);
        assert_eq!(
            state.snapshot().changes[0].summary.as_deref(),
            Some("updated content")
        );
    }

    #[test]
    fn filtering_updates_the_tree_count_and_selected_changes_together() {
        // Given
        let mut state = state();
        state.action(Action::Filter, 10);
        // When
        state.paste("tests");
        // Then
        assert_eq!(
            paths(&state),
            [Path::new(""), Path::new("tests"), Path::new("tests/c.rs")]
        );
        assert_eq!(state.matching_paths().len(), 1);
        assert_eq!(state.selected_changes(), [3]);
    }

    #[test]
    fn clearing_an_accepted_filter_restores_all_paths_and_categories() {
        // Given
        let mut state = state();
        state.action(Action::Filter, 10);
        state.paste("tests");
        state.action(Action::Accept, 10);
        // When
        state.action(Action::Cancel, 10);
        // Then
        assert_eq!(state.matching_paths().len(), 3);
        assert_eq!(state.tree(false).len(), 6);
        assert_eq!(state.selected_changes(), [0, 1, 2, 3]);
    }

    #[test]
    fn collapsing_a_directory_keeps_its_changes_selected() {
        // Given
        let mut state = state();
        state.action(Action::Down, 10);
        // When
        state.action(Action::Left, 10);
        // Then
        assert_eq!(
            paths(&state),
            [
                Path::new(""),
                Path::new("src"),
                Path::new("tests"),
                Path::new("tests/c.rs")
            ]
        );
        assert_eq!(state.tree(true).len(), 6);
        assert_eq!(state.selected_changes(), [0, 1, 2]);
        assert_eq!(state.matching_paths().len(), 3);
    }

    #[test]
    fn expanding_a_directory_restores_its_visible_descendants() {
        // Given
        let mut state = state();
        state.action(Action::Down, 10);
        state.action(Action::Left, 10);
        // When
        state.action(Action::Right, 10);
        // Then
        assert_eq!(state.tree(false).len(), 6);
        assert_eq!(state.selection(), Path::new("src"));
        assert!(paths(&state).contains(&Path::new("src/b.rs")));
    }

    #[test]
    fn replacing_a_filtered_snapshot_removes_old_paths_and_falls_back_to_the_parent() {
        // Given
        let mut state = state();
        state.action(Action::NextFile, 10);
        state.action(Action::Filter, 10);
        state.paste("src");
        state.action(Action::Accept, 10);
        let replacement = snapshot_with(&[
            ("docs/guide.md", Category::Untracked),
            ("src/new.rs", Category::Staged),
        ]);
        // When
        state.update_snapshot(replacement);
        // Then
        assert_eq!(state.selection(), Path::new("src"));
        assert_eq!(
            paths(&state),
            [Path::new(""), Path::new("src"), Path::new("src/new.rs")]
        );
        assert_eq!(state.matching_paths().len(), 1);
        assert_eq!(state.selected_changes(), [1]);
    }

    #[test]
    fn a_category_change_updates_badges_and_change_indices() {
        // Given
        let mut state = state();
        state.action(Action::NextFile, 10);
        let replacement = snapshot_with(&[
            ("src/b.rs", Category::Untracked),
            ("src/a.rs", Category::Staged),
        ]);
        // When
        state.update_snapshot(replacement);
        // Then
        assert_eq!(state.selected_changes(), [1]);
        assert_eq!(state.tree(false)[2].badges, "S");
        assert_eq!(state.tree(false)[2].path, Path::new("src/a.rs"));
    }

    #[test]
    fn tree_order_places_directories_before_files_without_reordering_diff_categories() {
        // Given
        let mut state = State::new(false);
        let snapshot = snapshot_with(&[
            ("a.rs", Category::Unstaged),
            ("z/main.rs", Category::Staged),
        ]);
        // When
        state.update_snapshot(snapshot);
        // Then
        assert_eq!(
            paths(&state),
            [
                Path::new(""),
                Path::new("z"),
                Path::new("z/main.rs"),
                Path::new("a.rs")
            ]
        );
        assert_eq!(state.selected_changes(), [0, 1]);
    }

    #[test]
    fn selecting_a_directory_excludes_similarly_named_siblings() {
        // Given
        let mut state = State::new(false);
        state.update_snapshot(snapshot_with(&[
            ("src/a.rs", Category::Unstaged),
            ("src-extra/b.rs", Category::Unstaged),
            ("src.rs", Category::Staged),
        ]));
        // When
        state.action(Action::Down, 10);
        // Then
        assert_eq!(state.selection(), Path::new("src"));
        assert_eq!(state.selected_changes(), [0]);
    }

    #[test]
    fn a_filter_without_matches_keeps_the_root_available() {
        // Given
        let mut state = state();
        state.action(Action::NextFile, 10);
        state.action(Action::Filter, 10);
        // When
        state.paste("missing-file");
        // Then
        assert_eq!(paths(&state), [Path::new("")]);
        assert_eq!(state.selection(), Path::new(""));
        assert!(state.matching_paths().is_empty());
        assert!(state.selected_changes().is_empty());
    }

    #[test]
    fn a_snapshot_replacement_preserves_collapsed_ancestors_of_new_files() {
        // Given
        let mut state = state();
        state.action(Action::Down, 10);
        state.action(Action::Left, 10);
        let replacement = snapshot_with(&[("src/new.rs", Category::Untracked)]);
        // When
        state.update_snapshot(replacement);
        // Then
        assert_eq!(paths(&state), [Path::new(""), Path::new("src")]);
        assert_eq!(state.tree(true).len(), 3);
        assert_eq!(state.selected_changes(), [0]);
    }
}
