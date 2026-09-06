use crate::model::{Category, Snapshot, display_path};
use nucleo_matcher::{
    Matcher, Utf32Str,
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
    path::{Path, PathBuf},
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

struct Node {
    path: PathBuf,
    searchable: String,
    parent: Option<usize>,
    children: Vec<usize>,
    changes: Vec<usize>,
    badges: String,
}
impl Node {
    fn new(path: &Path, parent: Option<usize>) -> Self {
        Self {
            path: path.to_owned(),
            searchable: display_path(path),
            parent,
            children: Vec::new(),
            changes: Vec::new(),
            badges: String::new(),
        }
    }
}

/// Snapshot-owned hierarchy and cached projections for filtering and collapse.
pub(super) struct TreeIndex {
    nodes: Vec<Node>,
    by_path: BTreeMap<PathBuf, usize>,
    matching: BTreeSet<PathBuf>,
    all: Vec<TreeRow>,
    visible: Vec<TreeRow>,
}

impl TreeIndex {
    pub(super) fn new(snapshot: &Snapshot, filter: &str, collapsed: &BTreeSet<PathBuf>) -> Self {
        let mut tree = Self {
            nodes: vec![Node::new(Path::new(""), None)],
            by_path: BTreeMap::from([(PathBuf::new(), 0)]),
            matching: BTreeSet::new(),
            all: Vec::new(),
            visible: Vec::new(),
        };
        for (index, change) in snapshot.changes.iter().enumerate() {
            let id = tree.insert_path(&change.path);
            tree.nodes[id].changes.push(index);
        }
        for node in &mut tree.nodes {
            node.badges = node
                .changes
                .iter()
                .map(|index| snapshot.changes[*index].category)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|category| match category {
                    Category::Unstaged => 'U',
                    Category::Untracked => '?',
                    Category::Staged => 'S',
                    Category::Comparison => 'M',
                })
                .collect();
        }
        tree.refilter(filter, collapsed);
        tree
    }

    fn insert_path(&mut self, path: &Path) -> usize {
        if let Some(id) = self.by_path.get(path) {
            return *id;
        }
        let missing: Vec<_> = path
            .ancestors()
            .take_while(|ancestor| !self.by_path.contains_key(*ancestor))
            .collect();
        let mut parent = missing
            .last()
            .and_then(|path| path.parent())
            .and_then(|path| self.by_path.get(path))
            .copied()
            .unwrap_or(0);
        for path in missing.into_iter().rev() {
            let id = self.nodes.len();
            self.nodes.push(Node::new(path, Some(parent)));
            self.nodes[parent].children.push(id);
            self.by_path.insert(path.to_owned(), id);
            parent = id;
        }
        parent
    }

    pub(super) fn refilter(&mut self, filter: &str, collapsed: &BTreeSet<PathBuf>) {
        let atom = Atom::new(
            filter,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
            false,
        );
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT.match_paths());
        let mut buffer = Vec::new();
        let mut included = vec![false; self.nodes.len()];
        self.matching.clear();
        for (id, node) in self.nodes.iter().enumerate() {
            if node.changes.is_empty()
                || (!filter.is_empty()
                    && atom
                        .score(Utf32Str::new(&node.searchable, &mut buffer), &mut matcher)
                        .is_none())
            {
                continue;
            }
            self.matching.insert(node.path.clone());
            let mut ancestor = Some(id);
            while let Some(id) = ancestor {
                if included[id] {
                    break;
                }
                included[id] = true;
                ancestor = self.nodes[id].parent;
            }
        }
        // A path can be a file in one category and an ancestor in another.
        // Its directory presentation follows the currently matching descendants.
        let directories: Vec<_> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(id, node)| id == 0 || node.children.iter().any(|child| included[*child]))
            .collect();
        self.all.clear();
        let mut pending = vec![(0, 0)];
        while let Some((id, depth)) = pending.pop() {
            let node = &self.nodes[id];
            self.all.push(TreeRow {
                path: node.path.clone(),
                directory: directories[id],
                depth,
                badges: node.badges.clone(),
            });
            let mut children: Vec<_> = node
                .children
                .iter()
                .copied()
                .filter(|child| included[*child])
                .collect();
            children.sort_by(|a, b| {
                directories[*b].cmp(&directories[*a]).then_with(|| {
                    self.nodes[*a]
                        .path
                        .file_name()
                        .cmp(&self.nodes[*b].path.file_name())
                })
            });
            pending.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
        }
        self.reveal(collapsed);
    }

    pub(super) fn reveal(&mut self, collapsed: &BTreeSet<PathBuf>) {
        self.visible.clear();
        let mut hidden_below = None;
        for row in &self.all {
            if hidden_below.is_some_and(|depth| row.depth > depth) {
                continue;
            }
            hidden_below = (row.directory && collapsed.contains(&row.path)).then_some(row.depth);
            self.visible.push(row.clone());
        }
    }

    pub(super) fn rows(&self, include_collapsed: bool) -> &[TreeRow] {
        if include_collapsed {
            &self.all
        } else {
            &self.visible
        }
    }

    pub(super) fn matching_paths(&self) -> &BTreeSet<PathBuf> {
        &self.matching
    }

    pub(super) fn contains_selection(&self, selection: &Path) -> bool {
        selection.as_os_str().is_empty() || self.subtree_paths(selection).next().is_some()
    }

    pub(super) fn selected_changes(&self, selection: &Path) -> Vec<usize> {
        let mut indices: Vec<_> = self
            .subtree_paths(selection)
            .filter_map(|path| self.by_path.get(path))
            .flat_map(|id| self.nodes[*id].changes.iter().copied())
            .collect();
        // Snapshot order defines diff category order, independently of tree order.
        indices.sort_unstable();
        indices
    }

    fn subtree_paths<'a>(&'a self, selection: &'a Path) -> impl Iterator<Item = &'a PathBuf> {
        self.matching
            .range::<Path, _>((Bound::Included(selection), Bound::Unbounded))
            .take_while(move |path| path.starts_with(selection))
    }
}
