mod navigation;

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

struct FuzzyPathMatcher {
    atom: Atom,
    matcher: Matcher,
    graphemes: Vec<char>,
}

impl FuzzyPathMatcher {
    fn new(query: &str) -> Self {
        Self {
            atom: Atom::new(
                query,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
                false,
            ),
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT.match_paths()),
            graphemes: Vec::new(),
        }
    }

    fn haystack<'a>(path: &'a str, graphemes: &'a mut Vec<char>) -> Utf32Str<'a> {
        if path.is_ascii() {
            Utf32Str::Ascii(path.as_bytes())
        } else {
            // Grapheme offsets keep highlighting aligned with displayed paths.
            graphemes.clear();
            graphemes.extend(nucleo_matcher::chars::graphemes(path));
            Utf32Str::Unicode(graphemes)
        }
    }

    fn score(&mut self, path: &str) -> Option<u16> {
        let haystack = Self::haystack(path, &mut self.graphemes);
        self.atom.score(haystack, &mut self.matcher)
    }

    fn indices(&mut self, path: &str, indices: &mut Vec<u32>) -> Option<u16> {
        indices.clear();
        let haystack = Self::haystack(path, &mut self.graphemes);
        self.atom.indices(haystack, &mut self.matcher, indices)
    }
}

/// A Git status paired with the category that determines its tree color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TreeBadge {
    pub category: Category,
    pub status: char,
}

/// A file or directory displayed in the sidebar tree.
#[derive(Debug, Clone)]
pub struct TreeRow {
    /// Path relative to the repository root.
    pub path: PathBuf,
    /// Whether this row represents a directory.
    pub directory: bool,
    /// Nesting depth used for indentation.
    pub depth: usize,
    /// Git status markers shown beside the path, ordered by category.
    pub badges: Vec<TreeBadge>,
    /// Escaped basename, compact directory chain, or repository root label.
    pub name: String,
    /// Distinct matching paths in this subtree, including this path if changed.
    pub count: usize,
    /// Grapheme positions within the displayed name matched by the fuzzy query.
    pub match_indices: Vec<usize>,
}

struct Node {
    path: PathBuf,
    searchable: String,
    name: String,
    name_start: usize,
    name_end: usize,
    parent: Option<usize>,
    children: Vec<usize>,
    changes: Vec<usize>,
    badges: Vec<TreeBadge>,
}
impl Node {
    fn new(path: &Path, parent: Option<usize>) -> Self {
        let searchable = display_path(path);
        let name = if path.as_os_str().is_empty() {
            "All changes".into()
        } else {
            display_path(Path::new(path.file_name().unwrap_or_default()))
        };
        let name_end = nucleo_matcher::chars::graphemes(&searchable).count();
        let name_start = name_end.saturating_sub(nucleo_matcher::chars::graphemes(&name).count());
        Self {
            path: path.to_owned(),
            searchable,
            name,
            name_start,
            name_end,
            parent,
            children: Vec::new(),
            changes: Vec::new(),
            badges: Vec::new(),
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
    eligible: BTreeSet<usize>,
}

impl TreeIndex {
    pub(super) fn new(snapshot: &Snapshot, filter: &str, collapsed: &BTreeSet<PathBuf>) -> Self {
        let mut tree = Self {
            nodes: vec![Node::new(Path::new(""), None)],
            by_path: BTreeMap::from([(PathBuf::new(), 0)]),
            matching: BTreeSet::new(),
            all: Vec::new(),
            visible: Vec::new(),
            eligible: (0..snapshot.changes.len()).collect(),
        };
        for (index, change) in snapshot.changes.iter().enumerate() {
            let id = tree.insert_path(&change.path);
            tree.nodes[id].changes.push(index);
        }
        for node in &mut tree.nodes {
            node.badges = node
                .changes
                .iter()
                .map(|index| {
                    let change = &snapshot.changes[*index];
                    TreeBadge {
                        category: change.category,
                        status: change.status,
                    }
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
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
        let mut matcher = FuzzyPathMatcher::new(filter);
        let mut counts = vec![0; self.nodes.len()];
        let mut matches = vec![BTreeSet::new(); self.nodes.len()];
        let mut indices = Vec::new();
        self.matching.clear();
        for (id, node) in self.nodes.iter().enumerate() {
            indices.clear();
            if !node
                .changes
                .iter()
                .any(|index| self.eligible.contains(index))
            {
                continue;
            }
            if !filter.is_empty() && matcher.indices(&node.searchable, &mut indices).is_none() {
                continue;
            }
            self.matching.insert(node.path.clone());
            counts[id] = 1;
            let mut ancestor = (!indices.is_empty()).then_some(id);
            while let Some(id) = ancestor {
                let node = &self.nodes[id];
                matches[id].extend(indices.iter().filter_map(|index| {
                    let index = *index as usize;
                    (node.name_start..node.name_end)
                        .contains(&index)
                        .then(|| index - node.name_start)
                }));
                ancestor = node.parent;
            }
        }
        // Parents are inserted before children, so one reverse pass counts paths
        // without counting the same file again for each change category.
        for id in (1..self.nodes.len()).rev() {
            if let Some(parent) = self.nodes[id].parent {
                counts[parent] += counts[id];
            }
        }
        // A path can be a file in one category and an ancestor in another.
        // Its directory presentation follows the currently matching descendants.
        let directories: Vec<_> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(id, node)| id == 0 || node.children.iter().any(|child| counts[*child] > 0))
            .collect();
        self.all.clear();
        let mut pending = vec![(0, 0)];
        while let Some((mut id, depth)) = pending.pop() {
            let start = id;
            let mut name = self.nodes[id].name.clone();
            let mut match_indices: Vec<_> = matches[id].iter().copied().collect();
            // Only directories without their own matching change may disappear.
            // A path can itself be a changed file in another Git category.
            while id != 0 && !self.matching.contains(&self.nodes[id].path) {
                let mut children = self.nodes[id]
                    .children
                    .iter()
                    .copied()
                    .filter(|child| counts[*child] > 0);
                let Some(child) = children.next() else {
                    break;
                };
                if children.next().is_some() || !directories[child] {
                    break;
                }
                let offset = self.nodes[child].name_start - self.nodes[start].name_start;
                name.push('/');
                name.push_str(&self.nodes[child].name);
                match_indices.extend(matches[child].iter().map(|index| offset + index));
                id = child;
            }
            let node = &self.nodes[id];
            self.all.push(TreeRow {
                path: node.path.clone(),
                directory: directories[id],
                depth,
                badges: node.badges.clone(),
                name,
                count: counts[id],
                match_indices,
            });
            let mut children: Vec<_> = node
                .children
                .iter()
                .copied()
                .filter(|child| counts[*child] > 0)
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
            hidden_below = (row.directory
                && row.path.ancestors().any(|path| collapsed.contains(path)))
            .then_some(row.depth);
            self.visible.push(row.clone());
        }
    }

    pub(super) fn filter_changes(
        &mut self,
        snapshot: &Snapshot,
        filter: &super::filters::ChangeFilter,
    ) {
        self.eligible = snapshot
            .changes
            .iter()
            .enumerate()
            .filter_map(|(index, change)| filter.matches(change).then_some(index))
            .collect();
        for node in &mut self.nodes {
            node.badges = node
                .changes
                .iter()
                .filter(|index| self.eligible.contains(index))
                .map(|index| {
                    let change = &snapshot.changes[*index];
                    TreeBadge {
                        category: change.category,
                        status: change.status,
                    }
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
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

    pub(super) fn find_files(&self, query: &str) -> Vec<PathBuf> {
        let mut matcher = FuzzyPathMatcher::new(query);
        let mut matches = Vec::new();
        for row in self.all.iter().filter(|row| !row.directory) {
            let Some(score) = self
                .by_path
                .get(&row.path)
                .and_then(|id| matcher.score(&self.nodes[*id].searchable))
            else {
                continue;
            };
            matches.push((&row.path, score));
        }
        // Stable sorting preserves tree order for equal scores.
        matches.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
        matches.into_iter().map(|(path, _)| path.clone()).collect()
    }

    pub(super) fn selected_changes(&self, selection: &Path) -> Vec<usize> {
        let mut indices: Vec<_> = self
            .subtree_paths(selection)
            .filter_map(|path| self.by_path.get(path))
            .flat_map(|id| self.nodes[*id].changes.iter().copied())
            .filter(|index| self.eligible.contains(index))
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
