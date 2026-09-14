mod activity;
mod collection;
mod command;
mod commit;
mod diff;
mod discard;
mod filters;
mod prompt;
mod toast;
pub(crate) use filters::STATUSES;
mod tree;
mod update;

pub use collection::CollectionRequest;
use collection::CollectionState;
pub use command::{Command, EditorPosition, Effects};
pub(crate) use commit::{CommitDialog, CommitField};
use diff::DiffState;
pub use prompt::{Prompt, PromptKind};
use toast::Toast;
use tree::TreeIndex;
pub use tree::{TreeBadge, TreeRow};

use crate::{config::Whitespace, input::Scope, model::Snapshot, render::Document};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

/// A working-tree file and an optional one-based editor destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorTarget<'a> {
    pub path: &'a Path,
    pub line: Option<usize>,
}

/// Repository data and interactive state for the terminal UI.
pub struct State {
    activity: activity::ActivityState,
    collection: CollectionState,
    diff: DiffState,
    tree: TreeIndex,
    help_scroll: usize,
    help_height: usize,
    help_max_scroll: usize,
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
    change_filter: filters::ChangeFilter,
    filter_menu: Option<filters::FilterMenu>,
    comparison: bool,
    /// Active filter or search prompt, if any.
    prompt: Option<Prompt>,
    /// Input scope currently receiving keyboard actions.
    focus: Scope,
    /// Whether the file sidebar is visible.
    sidebar: bool,
    /// Whether the help overlay is visible.
    help: bool,
    discard: Option<discard::DiscardMenu>,
    commit: CommitDialog,
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
        path.ancestors().any(|path| self.collapsed.contains(path))
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
    pub fn stage_target(&self, height: usize) -> Option<crate::git::StageTarget> {
        self.change_target(height)
    }

    pub fn change_target(&self, height: usize) -> Option<crate::git::ChangeTarget> {
        use crate::git::StageTarget;
        if self.focus == Scope::Tree {
            self.tree
                .rows(false)
                .iter()
                .find(|row| row.path == self.selection)
                .map(|row| {
                    if row.directory {
                        StageTarget::Subtree(row.path.clone())
                    } else {
                        StageTarget::File(row.path.clone())
                    }
                })
        } else {
            self.active_file(height)
                .map(|path| StageTarget::File(path.to_owned()))
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
    pub fn help_scroll(&self) -> usize {
        self.help_scroll
    }
    pub(crate) fn resize_help(&mut self, rows: usize, height: usize) {
        self.help_height = height;
        self.help_max_scroll = rows.saturating_sub(height);
        self.help_scroll = self.help_scroll.min(self.help_max_scroll);
    }

    pub(crate) fn scroll_help(&mut self, amount: usize, down: bool) -> bool {
        let previous = self.help_scroll;
        self.help_scroll = if down {
            previous.saturating_add(amount).min(self.help_max_scroll)
        } else {
            previous.saturating_sub(amount)
        };
        previous != self.help_scroll
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
            help_height: 0,
            help_max_scroll: 0,
            snapshot,
            selection: PathBuf::new(),
            collapsed,
            unfiltered_collapsed: None,
            filter: String::new(),
            change_filter: filters::ChangeFilter::default(),
            filter_menu: None,
            comparison: false,
            prompt: None,
            focus: Scope::Tree,
            sidebar: true,
            help: false,
            discard: None,
            commit: CommitDialog::default(),
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
        self.tree
            .filter_changes(&self.snapshot, &self.change_filter);
        self.tree.refilter(&self.filter, &self.collapsed);
        self.refresh_find_matches();
        self.reconcile_selection();
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

    pub fn matching_paths(&self) -> &BTreeSet<PathBuf> {
        self.tree.matching_paths()
    }

    pub fn selected_changes(&self) -> Vec<usize> {
        self.tree.selected_changes(&self.selection)
    }

    pub fn tree(&self, include_collapsed: bool) -> &[TreeRow] {
        self.tree.rows(include_collapsed)
    }

    pub fn scroll_by(&mut self, amount: usize, down: bool, height: usize) {
        self.diff.scroll_by(amount, down, height);
    }

    pub fn jump_match(&mut self, forward: bool, height: usize) {
        self.diff.jump_match(forward, height);
    }
}

#[cfg(test)]
mod tests {
    mod debounce;
    mod filters;
    mod prompt;
    mod tree_ux;

    use super::*;
    use crate::{
        input::Action,
        model::{Category, Change},
    };
    use std::time::{Duration, Instant};
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
        assert_eq!(
            state.tree(false)[2].badges,
            [TreeBadge {
                category: Category::Staged,
                status: 'M'
            }]
        );
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
