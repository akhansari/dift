mod update;
use update::Displayed;
pub use update::Effects;

use crate::{
    input::{Action, Editor, Scope},
    model::{Category, Snapshot, display_path},
    render::Document,
};
use nucleo_matcher::{
    Matcher, Utf32Str,
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

/// A file or directory displayed in the sidebar tree.
#[derive(Debug, Clone)]
pub struct TreeRow {
    /// Path relative to the repository root.
    pub path: PathBuf,
    /// Whether this row represents a directory.
    pub directory: bool,
    /// Nesting depth used for indentation.
    pub depth: usize,
    /// Compact change category markers shown beside the path.
    pub badges: String,
}
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
        Self {
            displayed: Displayed::default(),
            help_scroll: 0,
            snapshot: Arc::new(Snapshot::default()),
            selection: PathBuf::new(),
            collapsed: BTreeSet::new(),
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
        self.snapshot = Arc::new(snapshot);
        self.reconcile_selection();
    }
    fn reconcile_selection(&mut self) {
        let paths = self.matching_paths();
        while !self.selection.as_os_str().is_empty()
            && !paths
                .iter()
                .any(|p| p == &self.selection || p.starts_with(&self.selection))
        {
            self.selection.pop();
        }
    }
    pub fn matching_paths(&self) -> BTreeSet<PathBuf> {
        let atom = Atom::new(
            &self.filter,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
            false,
        );
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT.match_paths());
        let mut buffer = Vec::new();
        self.snapshot
            .changes
            .iter()
            .map(|c| &c.path)
            .filter(|path| {
                self.filter.is_empty()
                    || atom
                        .score(
                            Utf32Str::new(&display_path(path), &mut buffer),
                            &mut matcher,
                        )
                        .is_some()
            })
            .cloned()
            .collect()
    }
    pub fn selected_changes(&self) -> Vec<usize> {
        let paths = self.matching_paths();
        self.snapshot
            .changes
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                paths.contains(&c.path)
                    && (self.selection.as_os_str().is_empty()
                        || c.path == self.selection
                        || c.path.starts_with(&self.selection))
            })
            .map(|(i, _)| i)
            .collect()
    }
    pub fn tree(&self, include_collapsed: bool) -> Vec<TreeRow> {
        let paths = self.matching_paths();
        let mut nodes: BTreeMap<PathBuf, bool> = BTreeMap::new();
        for path in &paths {
            nodes.insert(path.clone(), false);
            let mut parent = path.parent();
            while let Some(p) = parent.filter(|p| !p.as_os_str().is_empty()) {
                nodes.insert(p.to_owned(), true);
                parent = p.parent();
            }
        }
        let mut rows = vec![TreeRow {
            path: PathBuf::new(),
            directory: true,
            depth: 0,
            badges: String::new(),
        }];
        self.tree_children(Path::new(""), &nodes, &mut rows, include_collapsed, 1);
        rows
    }
    fn tree_children(
        &self,
        parent: &Path,
        nodes: &BTreeMap<PathBuf, bool>,
        rows: &mut Vec<TreeRow>,
        include_collapsed: bool,
        depth: usize,
    ) {
        if !include_collapsed && self.collapsed.contains(parent) {
            return;
        }
        let mut children: Vec<_> = nodes
            .iter()
            .filter(|(p, _)| p.parent() == Some(parent))
            .collect();
        children.sort_by(|(a, a_dir), (b, b_dir)| {
            b_dir
                .cmp(a_dir)
                .then_with(|| a.file_name().cmp(&b.file_name()))
        });
        for (path, directory) in children {
            let categories: BTreeSet<_> = self
                .snapshot
                .changes
                .iter()
                .filter(|c| c.path == *path)
                .map(|c| c.category)
                .collect();
            let badges = categories
                .into_iter()
                .map(|c| match c {
                    Category::Unstaged => 'U',
                    Category::Untracked => '?',
                    Category::Staged => 'S',
                    Category::Comparison => 'M',
                })
                .collect();
            rows.push(TreeRow {
                path: path.clone(),
                directory: *directory,
                depth,
                badges,
            });
            if *directory {
                self.tree_children(path, nodes, rows, include_collapsed, depth + 1);
            }
        }
    }
    fn next_file(&mut self, forward: bool) {
        let all = self.tree(true);
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
            for ancestor in row.path.ancestors().skip(1) {
                self.collapsed.remove(ancestor);
            }
            self.scroll = 0;
        }
    }
    fn move_tree(&mut self, forward: bool) {
        let rows = self.tree(false);
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
        } else if self.selection.pop() {
            self.scroll = 0;
        }
    }
    fn expand(&mut self) {
        self.collapsed.remove(&self.selection);
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
            prompt.editor.text().clone_into(query);
        }
        if !self.selection.as_os_str().is_empty()
            && !self
                .matching_paths()
                .iter()
                .any(|p| p == &self.selection || p.starts_with(&self.selection))
        {
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
                self.filter = prompt.previous;
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
    use crate::model::Change;
    fn snapshot() -> Snapshot {
        let file = |path: &str, category| Change {
            path: path.into(),
            old_path: None,
            category,
            status: 'M',
            hunks: Vec::new(),
            summary: None,
            oversized: false,
            old_source: None,
            new_source: None,
        };
        Snapshot {
            branch: "main".into(),
            changes: vec![
                file("src/a.rs", Category::Unstaged),
                file("src/a.rs", Category::Staged),
                file("src/b.rs", Category::Untracked),
                file("tests/c.rs", Category::Staged),
            ],
        }
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
}
