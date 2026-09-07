mod activity;
mod collection;
mod diff;
mod tree;
mod update;

pub use collection::CollectionRequest;
use collection::CollectionState;
use diff::DiffState;
use tree::TreeIndex;
pub use tree::TreeRow;
pub use update::{EditorPosition, Effects};

use crate::{
    config::Whitespace,
    input::{Action, Editor, Scope},
    model::Snapshot,
    render::Document,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const FILTER_DEBOUNCE: Duration = Duration::from_millis(300);
const TOAST_DURATION: Duration = Duration::from_secs(3);

struct Toast {
    message: String,
    expires_at: Instant,
}

/// A working-tree file and an optional one-based editor destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorTarget<'a> {
    pub path: &'a Path,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Filter,
    Find,
    Search,
}
/// An active prompt and the values restored when it is cancelled.
#[derive(Debug, Clone)]
pub struct Prompt {
    /// Whether the prompt filters files, finds a file, or searches the diff.
    kind: PromptKind,
    /// Text and cursor being edited.
    editor: Editor,
    /// Filter or search text from before the prompt opened.
    previous: String,
    /// Selected path from before the prompt opened.
    previous_selection: PathBuf,
    previous_collapsed: BTreeSet<PathBuf>,
    previous_unfiltered_collapsed: Option<BTreeSet<PathBuf>>,
    /// When the latest filter edit should be applied.
    filter_deadline: Option<Instant>,
    find_matches: Vec<PathBuf>,
    find_index: usize,
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
    activity: activity::ActivityState,
    collection: CollectionState,
    diff: DiffState,
    tree: TreeIndex,
    help_scroll: usize,
    /// Most recently collected repository changes.
    snapshot: Arc<Snapshot>,
    /// Selected file or directory; an empty path selects the root.
    selection: PathBuf,
    /// Directories whose children are hidden in the tree.
    collapsed: BTreeSet<PathBuf>,
    /// Normal tree collapse state saved while a nonempty filter is active.
    unfiltered_collapsed: Option<BTreeSet<PathBuf>>,
    /// Applied fuzzy query used to filter file paths; draft text lives in the prompt.
    filter: String,
    /// Active filter or search prompt, if any.
    prompt: Option<Prompt>,
    /// Input scope currently receiving keyboard actions.
    focus: Scope,
    /// Whether the file sidebar is visible.
    sidebar: bool,
    /// Whether the help overlay is visible.
    help: bool,
    /// Latest setting notification, removed when its deadline passes.
    toast: Option<Toast>,
    visible_files: usize,
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
        &self.diff.search
    }
    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }
    pub fn document(&self) -> &Document {
        &self.diff.document
    }
    /// Working-tree path selected in the tree or identified by the visible diff header.
    pub fn active_file(&self, height: usize) -> Option<&Path> {
        if self.focus == Scope::Tree {
            self.tree
                .rows(false)
                .iter()
                .find(|row| row.path == self.selection && !row.directory)
                .map(|row| row.path.as_path())
        } else {
            self.document()
                .current_file_section(self.scroll(), height)
                .map(|section| section.path.as_path())
        }
    }
    pub fn editor_target(
        &self,
        height: usize,
        position: EditorPosition,
    ) -> Option<EditorTarget<'_>> {
        let path = self.active_file(height)?;
        let line = (position == EditorPosition::VisibleHunk)
            .then(|| {
                self.document()
                    .visible_hunk_line(path, self.scroll(), height)
            })
            .flatten();
        Some(EditorTarget { path, line })
    }
    pub(crate) fn shared_document(&self) -> &Arc<Document> {
        &self.diff.document
    }
    pub(crate) fn focus_diff(&mut self) {
        self.focus = Scope::Diff;
    }
    pub fn loaded(&self) -> &BTreeSet<PathBuf> {
        &self.collection.loaded
    }
    pub fn error(&self) -> Option<&str> {
        self.collection.error.as_deref()
    }
    pub fn toast(&self) -> Option<&str> {
        self.toast.as_ref().map(|toast| toast.message.as_str())
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
        self.diff.split
    }
    pub fn help(&self) -> bool {
        self.help
    }
    pub fn scroll(&self) -> usize {
        self.diff.scroll
    }
    pub fn collecting(&self) -> bool {
        self.collection.collecting
    }
    pub fn rendering(&self) -> bool {
        self.diff.rendering
    }
    pub fn match_index(&self) -> Option<usize> {
        self.diff.match_index
    }
    pub fn new(split: bool) -> Self {
        Self::with_whitespace(split, Whitespace::Exact)
    }

    pub fn with_whitespace(split: bool, whitespace: Whitespace) -> Self {
        let snapshot = Arc::new(Snapshot::default());
        let collapsed = BTreeSet::new();
        let tree = TreeIndex::new(&snapshot, "", &collapsed);
        Self {
            activity: activity::ActivityState::default(),
            diff: DiffState::new(split),
            collection: CollectionState::new(whitespace),
            tree,
            help_scroll: 0,
            snapshot,
            selection: PathBuf::new(),
            collapsed,
            unfiltered_collapsed: None,
            filter: String::new(),
            prompt: None,
            focus: Scope::Tree,
            sidebar: true,
            help: false,
            toast: None,
            visible_files: 0,
        }
    }

    pub fn update_snapshot(&mut self, snapshot: Snapshot) {
        self.visible_files = snapshot
            .changes
            .iter()
            .map(|change| &change.path)
            .collect::<BTreeSet<_>>()
            .len();
        self.tree = TreeIndex::new(&snapshot, &self.filter, &self.collapsed);
        self.snapshot = Arc::new(snapshot);
        if let Some(prompt) = &mut self.prompt
            && prompt.kind == PromptKind::Find
            && !prompt.editor.text().is_empty()
        {
            prompt.find_matches = self.tree.find_files(prompt.editor.text());
            prompt.find_index = prompt
                .find_matches
                .iter()
                .position(|path| *path == self.selection)
                .unwrap_or(0);
            self.select_find_match();
        }
        self.reconcile_selection();
    }

    /// Activity changes only the tree's appearance, never the rendered diff.
    pub fn advance_activity(&mut self, now: Instant) -> Effects {
        Effects {
            redraw: self.activity.advance(now),
            ..Effects::default()
        }
    }

    pub fn activity_intensity(&self, path: &Path, directory: bool) -> f32 {
        self.activity
            .intensity(path, directory, self.matching_paths())
    }

    pub fn visible_files(&self) -> usize {
        self.visible_files
    }

    pub fn requested_whitespace(&self) -> Whitespace {
        self.collection.whitespace
    }

    pub fn collection_request(&mut self) -> CollectionRequest {
        self.collection.request()
    }

    fn reconcile_selection(&mut self) {
        let previous = self.selection.clone();
        while !self
            .tree
            .rows(false)
            .iter()
            .any(|row| row.path == self.selection)
        {
            if !self.selection.pop() {
                break;
            }
        }
        if self.selection != previous {
            self.diff.scroll = 0;
        }
    }

    fn refilter(&mut self) {
        if self.filter.is_empty() {
            if let Some(collapsed) = self.unfiltered_collapsed.take() {
                self.collapsed = collapsed;
            }
        } else if self.unfiltered_collapsed.is_none() {
            self.unfiltered_collapsed = Some(self.collapsed.clone());
        }
        self.tree.refilter(&self.filter, &self.collapsed);
        if !self.filter.is_empty() {
            let mut revealed = false;
            for row in self.tree.rows(true).iter().filter(|row| row.directory) {
                revealed |= self.collapsed.remove(&row.path);
            }
            if revealed {
                self.tree.reveal(&self.collapsed);
            }
        }
        self.reconcile_selection();
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
            self.diff.scroll = 0;
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
            self.diff.scroll = 0;
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
            self.diff.scroll = 0;
        }
    }

    fn expand(&mut self) {
        if self.collapsed.remove(&self.selection) {
            self.tree.reveal(&self.collapsed);
        }
    }

    fn enter_directory(&mut self) {
        if self.is_collapsed(&self.selection) {
            self.expand();
        } else if let Some(child) = self.tree.rows(false).iter().find(|row| {
            !row.path.as_os_str().is_empty() && row.path.parent() == Some(&self.selection)
        }) {
            self.selection.clone_from(&child.path);
            self.diff.scroll = 0;
        }
    }

    fn begin_prompt(&mut self, kind: PromptKind) {
        let text = match kind {
            PromptKind::Filter => &self.filter,
            PromptKind::Find => "",
            PromptKind::Search => &self.diff.search,
        };
        self.prompt = Some(Prompt {
            kind,
            editor: Editor::new(text.to_owned()),
            previous: text.to_owned(),
            previous_selection: self.selection.clone(),
            previous_collapsed: self.collapsed.clone(),
            previous_unfiltered_collapsed: self.unfiltered_collapsed.clone(),
            filter_deadline: None,
            find_matches: Vec::new(),
            find_index: 0,
        });
    }

    fn prompt_edited(&mut self, now: Instant) -> bool {
        if let Some(prompt) = &mut self.prompt
            && prompt.kind == PromptKind::Filter
        {
            prompt.filter_deadline =
                (prompt.editor.text() != self.filter).then_some(now + FILTER_DEBOUNCE);
            return false;
        }
        self.apply_prompt()
    }

    fn apply_prompt(&mut self) -> bool {
        let mut changed = false;
        if let Some(prompt) = &self.prompt {
            let query = match prompt.kind {
                PromptKind::Filter => &mut self.filter,
                PromptKind::Find => return self.apply_find(),
                PromptKind::Search => &mut self.diff.search,
            };
            changed = query != prompt.editor.text();
            if changed {
                prompt.editor.text().clone_into(query);
                if prompt.kind == PromptKind::Filter {
                    self.refilter();
                }
            }
        }
        changed
    }

    fn apply_find(&mut self) -> bool {
        let Some(prompt) = &mut self.prompt else {
            return false;
        };
        prompt.find_matches.clear();
        prompt.find_index = 0;
        let query = prompt.editor.text();
        if query.is_empty() {
            let changed = self.selection != prompt.previous_selection;
            self.selection.clone_from(&prompt.previous_selection);
            self.collapsed.clone_from(&prompt.previous_collapsed);
            self.unfiltered_collapsed
                .clone_from(&prompt.previous_unfiltered_collapsed);
            self.tree.reveal(&self.collapsed);
            if changed {
                self.diff.scroll = 0;
            }
            self.reconcile_selection();
            return changed;
        }
        prompt.find_matches = self.tree.find_files(query);
        self.select_find_match()
    }

    fn next_find_match(&mut self) -> bool {
        let Some(prompt) = &mut self.prompt else {
            return false;
        };
        if prompt.kind != PromptKind::Find || prompt.find_matches.is_empty() {
            return false;
        }
        prompt.find_index = (prompt.find_index + 1) % prompt.find_matches.len();
        self.select_find_match()
    }

    fn select_find_match(&mut self) -> bool {
        let Some(path) = self
            .prompt
            .as_ref()
            .and_then(|prompt| prompt.find_matches.get(prompt.find_index))
        else {
            return false;
        };
        let changed = self.selection != *path;
        self.selection.clone_from(path);
        let mut revealed = false;
        for ancestor in self.selection.ancestors().skip(1) {
            revealed |= self.collapsed.remove(ancestor);
        }
        if revealed {
            self.tree.reveal(&self.collapsed);
        }
        if changed {
            self.diff.scroll = 0;
        }
        changed
    }

    fn finish_prompt(&mut self, accept: bool) -> bool {
        let mut changed = accept
            && self
                .prompt
                .as_ref()
                .is_some_and(|prompt| prompt.kind != PromptKind::Find)
            && self.apply_prompt();
        if let Some(prompt) = self.prompt.take()
            && !accept
        {
            match prompt.kind {
                PromptKind::Filter => {
                    changed = self.filter != prompt.previous
                        || self.selection != prompt.previous_selection;
                    self.filter = prompt.previous;
                    self.collapsed = prompt.previous_collapsed;
                    self.unfiltered_collapsed = prompt.previous_unfiltered_collapsed;
                    self.tree.refilter(&self.filter, &self.collapsed);
                    self.selection = prompt.previous_selection;
                }
                PromptKind::Find => {
                    changed = self.selection != prompt.previous_selection;
                    self.collapsed = prompt.previous_collapsed;
                    self.unfiltered_collapsed = prompt.previous_unfiltered_collapsed;
                    self.tree.reveal(&self.collapsed);
                    self.selection = prompt.previous_selection;
                }
                PromptKind::Search => {
                    changed = self.diff.search != prompt.previous;
                    self.diff.search = prompt.previous;
                }
            }
        }
        let selection = self.selection.clone();
        self.reconcile_selection();
        changed || self.selection != selection
    }

    pub fn scroll_by(&mut self, amount: usize, down: bool, height: usize) {
        self.diff.scroll_by(amount, down, height);
    }

    pub fn jump_match(&mut self, forward: bool, height: usize) {
        self.diff.jump_match(forward, height);
    }

    fn input_action(&mut self, action: Action, now: Instant) -> bool {
        let changed = self
            .prompt
            .as_mut()
            .is_some_and(|prompt| prompt.editor.edit(action));
        changed && self.prompt_edited(now)
    }
}

#[cfg(test)]
mod tests {
    mod debounce;
    mod tree_ux;

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
            activity: None,
            branch: "main".into(),
            whitespace: Default::default(),
            whitespace_hidden_files: 0,
            tracked_stats: None,
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
        state.action(Action::Down, 10, Instant::now());
        // When
        let selected = state.selected_changes();
        // Then
        assert_eq!(selected, vec![0, 1, 2]);
    }

    #[test]
    fn next_file_reveals_a_collapsed_path() {
        // Given
        let mut state = state();
        state.action(Action::Down, 10, Instant::now());
        state.action(Action::Left, 10, Instant::now());
        // When
        state.action(Action::NextFile, 10, Instant::now());
        // Then
        assert_eq!(state.selection(), Path::new("src/a.rs"));
        assert!(!state.is_collapsed(Path::new("src")));
        assert!(paths(&state).contains(&Path::new("src/a.rs")));
    }

    #[test]
    fn cancel_filter_restores_query_and_selection() {
        // Given
        let mut state = state();
        state.action(Action::NextFile, 10, Instant::now());
        state.action(Action::Filter, 10, Instant::now());
        state.paste("tests", Instant::now());
        // When
        state.action(Action::Cancel, 10, Instant::now());
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
        state.action(Action::NextFile, 10, Instant::now());
        // When
        state.update_snapshot(snapshot());
        // Then
        assert_eq!(state.selection(), Path::new("src"));
    }

    #[test]
    fn refreshing_keeps_selection_queries_and_collapsed_directories() {
        // Given
        let mut state = state();
        state.action(Action::Last, 10, Instant::now());
        state.action(Action::Left, 10, Instant::now());
        state.action(Action::Left, 10, Instant::now());
        state.action(Action::First, 10, Instant::now());
        state.action(Action::NextFile, 10, Instant::now());
        state.action(Action::Filter, 10, Instant::now());
        state.paste("src", Instant::now());
        state.action(Action::Accept, 10, Instant::now());
        state.action(Action::Search, 10, Instant::now());
        state.paste("needle", Instant::now());
        state.action(Action::Accept, 10, Instant::now());
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
        let now = Instant::now();
        state.action(Action::Filter, 10, now);
        state.paste("tests", now);
        // When
        state.apply_pending_filter(now + Duration::from_millis(300));
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
        state.action(Action::Filter, 10, Instant::now());
        state.paste("tests", Instant::now());
        state.action(Action::Accept, 10, Instant::now());
        // When
        state.action(Action::Cancel, 10, Instant::now());
        // Then
        assert_eq!(state.matching_paths().len(), 3);
        assert_eq!(state.tree(false).len(), 6);
        assert_eq!(state.selected_changes(), [0, 1, 2, 3]);
    }

    #[test]
    fn collapsing_a_directory_keeps_its_changes_selected() {
        // Given
        let mut state = state();
        state.action(Action::Down, 10, Instant::now());
        // When
        state.action(Action::Left, 10, Instant::now());
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
        state.action(Action::Down, 10, Instant::now());
        state.action(Action::Left, 10, Instant::now());
        // When
        state.action(Action::Right, 10, Instant::now());
        // Then
        assert_eq!(state.tree(false).len(), 6);
        assert_eq!(state.selection(), Path::new("src"));
        assert!(paths(&state).contains(&Path::new("src/b.rs")));
    }

    #[test]
    fn replacing_a_filtered_snapshot_removes_old_paths_and_falls_back_to_the_parent() {
        // Given
        let mut state = state();
        state.action(Action::NextFile, 10, Instant::now());
        state.action(Action::Filter, 10, Instant::now());
        state.paste("src", Instant::now());
        state.action(Action::Accept, 10, Instant::now());
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
        state.action(Action::NextFile, 10, Instant::now());
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
        state.action(Action::Down, 10, Instant::now());
        // Then
        assert_eq!(state.selection(), Path::new("src"));
        assert_eq!(state.selected_changes(), [0]);
    }

    #[test]
    fn a_filter_without_matches_keeps_the_root_available() {
        // Given
        let mut state = state();
        let now = Instant::now();
        state.action(Action::NextFile, 10, now);
        state.action(Action::Filter, 10, now);
        state.paste("missing-file", now);
        // When
        state.apply_pending_filter(now + Duration::from_millis(300));
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
        state.action(Action::Down, 10, Instant::now());
        state.action(Action::Left, 10, Instant::now());
        let replacement = snapshot_with(&[("src/new.rs", Category::Untracked)]);
        // When
        state.update_snapshot(replacement);
        // Then
        assert_eq!(paths(&state), [Path::new(""), Path::new("src")]);
        assert_eq!(state.tree(true).len(), 3);
        assert_eq!(state.selected_changes(), [0]);
    }
}
