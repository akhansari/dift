use super::{GitError, Repository, StageOutcome, path_from_bytes};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Result and attribution travel together, including on command failure.
pub(crate) struct MutationResult {
    pub outcome: Result<StageOutcome, GitError>,
    pub attribution: MutationAttribution,
}

pub(crate) enum MutationAttribution {
    Confirmed(Box<IndexMutation>),
    Uncertain,
}

type IndexEntries = BTreeMap<PathBuf, Vec<Vec<u8>>>;

/// Evidence is frozen at command completion; retries never infer ownership anew.
/// This is sampled state, not process provenance: an indistinguishable update or
/// a change restored between samples cannot be attributed with certainty.
pub(crate) struct IndexMutation {
    before: IndexObservation,
    after: IndexObservation,
    paths: BTreeSet<PathBuf>,
    observed: BTreeSet<PathBuf>,
    working_sources: BTreeMap<PathBuf, std::sync::Arc<str>>,
}

pub(super) struct IndexObservation {
    entries: IndexEntries,
    head: Vec<u8>,
    worktree: BTreeMap<PathBuf, Option<crate::model::WorktreeMetadata>>,
}

impl IndexObservation {
    pub(super) fn capture(repo: &Repository, paths: &BTreeSet<PathBuf>) -> Result<Self, GitError> {
        Ok(Self {
            entries: repo.index_entries()?,
            head: repo.activity_head()?,
            worktree: paths
                .iter()
                .map(|path| {
                    (
                        path.clone(),
                        super::source::activity_metadata(&repo.root.join(path)),
                    )
                })
                .collect(),
        })
    }
}

impl IndexMutation {
    pub(super) fn new(
        before: IndexObservation,
        after: IndexObservation,
        paths: BTreeSet<PathBuf>,
    ) -> Self {
        Self {
            before,
            after,
            paths,
            observed: BTreeSet::new(),
            working_sources: BTreeMap::new(),
        }
    }

    /// Only paths whose pre-operation working state was actually observed may
    /// be quieted. Rename sources can be absent from the displayed path map.
    pub(crate) fn retain_observed(
        &mut self,
        observations: &crate::model::ActivityObservations,
        head: &[u8],
    ) {
        if head != self.before.head {
            self.observed.clear();
            return;
        }
        self.working_sources = observations
            .iter()
            .filter_map(|(path, categories)| {
                working_source(categories).map(|source| (path.clone(), source.clone()))
            })
            .collect();
        self.observed = self
            .paths
            .iter()
            .filter(|path| {
                let Some(Some(metadata)) = self.before.worktree.get(*path) else {
                    return false;
                };
                let index_matches = observed_index(path, observations).is_some_and(|expected| {
                    self.before
                        .entries
                        .get(*path)
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                        == expected.as_slice()
                });
                index_matches
                    && (observations.get(*path).is_some_and(|categories| {
                        categories
                            .values()
                            .any(|observation| observation.metadata.as_ref() == Some(metadata))
                    }) || matches!(metadata, crate::model::WorktreeMetadata::Missing)
                        && observations
                            .values()
                            .flat_map(|categories| categories.values())
                            .any(|observation| observation.git.old_path.as_ref() == Some(*path)))
            })
            .cloned()
            .collect();
    }

    pub(crate) fn changed_paths(&self) -> BTreeSet<PathBuf> {
        self.paths
            .iter()
            .filter(|path| self.before.entries.get(*path) != self.after.entries.get(*path))
            .cloned()
            .collect()
    }

    pub(crate) fn quiet_paths(&self, repo: &Repository) -> BTreeSet<PathBuf> {
        let Ok(current) = IndexObservation::capture(repo, &self.paths) else {
            return BTreeSet::new();
        };
        if current.head != self.after.head || self.before.head != self.after.head {
            return BTreeSet::new();
        }
        self.paths
            .iter()
            .filter(|path| {
                let before = self.before.worktree.get(*path).and_then(Option::as_ref);
                let after = self.after.worktree.get(*path).and_then(Option::as_ref);
                // Directory metadata cannot identify a submodule's checked-out commit.
                let reliable = matches!(
                    before,
                    Some(crate::model::WorktreeMetadata::Missing)
                        | Some(crate::model::WorktreeMetadata::Present {
                            regular: true,
                            modified: Some(_),
                            ..
                        })
                        | Some(crate::model::WorktreeMetadata::Present {
                            symlink: true,
                            modified: Some(_),
                            link_target: Some(_),
                            ..
                        })
                );
                self.observed.contains(*path)
                    && reliable
                    && before == after
                    && current.worktree.get(*path) == self.after.worktree.get(*path)
                    && self.before.entries.get(*path) != self.after.entries.get(*path)
                    && current.entries.get(*path) == self.after.entries.get(*path)
            })
            .cloned()
            .collect()
    }

    pub(crate) fn matches_snapshot(
        &self,
        path: &Path,
        observations: &crate::model::ActivityObservations,
    ) -> bool {
        let expected = self.after.worktree.get(path).and_then(Option::as_ref);
        match observations.get(path) {
            Some(categories) => {
                let source_matches =
                    match (self.working_sources.get(path), working_source(categories)) {
                        (Some(before), Some(current)) => before == current,
                        _ => true,
                    };
                source_matches
                    && observed_index(path, observations).is_some_and(|index| {
                        self.after
                            .entries
                            .get(path)
                            .map(Vec::as_slice)
                            .unwrap_or_default()
                            == index.as_slice()
                    })
                    && categories
                        .values()
                        .all(|observation| observation.metadata.as_ref() == expected)
            }
            // A path removed from the change list cannot itself create a pulse.
            None => true,
        }
    }
}

fn working_source(
    categories: &BTreeMap<crate::model::Category, crate::model::ActivityObservation>,
) -> Option<&std::sync::Arc<str>> {
    use crate::model::Category;
    categories
        .get(&Category::Unstaged)
        .or_else(|| categories.get(&Category::Untracked))
        .and_then(|observation| observation.source.as_ref())
}

/// Reconstruct the index side of a status observation, never its working source.
fn observed_index(
    path: &Path,
    observations: &crate::model::ActivityObservations,
) -> Option<Vec<Vec<u8>>> {
    use crate::model::Category;
    let Some(categories) = observations.get(path) else {
        return observations
            .values()
            .any(|categories| {
                categories
                    .get(&Category::Staged)
                    .is_some_and(|observation| observation.git.old_path.as_deref() == Some(path))
            })
            .then(Vec::new);
    };
    if let Some(conflict) = categories
        .get(&Category::Unstaged)
        .filter(|observation| !observation.git.conflict_stages.is_empty())
    {
        return Some(
            conflict
                .git
                .conflict_stages
                .iter()
                .map(|entry| {
                    format!("{:06o} {} {}", entry.mode, entry.oid, entry.stage).into_bytes()
                })
                .collect(),
        );
    }
    let (mode, oid) = if let Some(staged) = categories.get(&Category::Staged) {
        (staged.git.new_mode.as_str(), staged.git.new_oid.as_deref()?)
    } else if let Some(unstaged) = categories.get(&Category::Unstaged) {
        (
            unstaged.git.old_mode.as_str(),
            unstaged.git.old_oid.as_str(),
        )
    } else if categories.contains_key(&Category::Untracked) {
        return Some(Vec::new());
    } else {
        return None;
    };
    if mode.is_empty() || oid.is_empty() {
        return None;
    }
    if mode.bytes().all(|byte| byte == b'0') {
        Some(Vec::new())
    } else {
        Some(vec![format!("{mode} {oid} 0").into_bytes()])
    }
}

impl Repository {
    pub(crate) fn activity_head(&self) -> Result<Vec<u8>, GitError> {
        self.run(["rev-parse", "--revs-only", "HEAD"])
    }

    fn index_entries(&self) -> Result<BTreeMap<PathBuf, Vec<Vec<u8>>>, GitError> {
        let output = self.run(["ls-files", "--stage", "-z"])?;
        let mut entries = BTreeMap::<PathBuf, Vec<Vec<u8>>>::new();
        for record in output
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            let tab = record
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| GitError::InvalidOutput("Invalid index entry".into()))?;
            entries
                .entry(path_from_bytes(&record[tab + 1..]))
                .or_default()
                .push(record[..tab].to_vec());
        }
        Ok(entries)
    }
}
