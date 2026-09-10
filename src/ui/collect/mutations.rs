use super::Update;
use crate::{
    git::{GitError, IndexMutation, MutationAttribution, Repository},
    model::{ActivityObservations, Snapshot},
};
use std::{collections::BTreeSet, path::PathBuf};

/// Owns attribution until a successful snapshot consumes it. Collection failures
/// retain evidence, but can never turn an uncertain command into a confirmed one.
#[derive(Default)]
pub(super) struct Mutations {
    pending: Vec<IndexMutation>,
    ambiguous: BTreeSet<PathBuf>,
}

impl Mutations {
    pub(super) fn record(
        &mut self,
        attribution: MutationAttribution,
        observed: Option<&ActivityObservations>,
        head: Option<&[u8]>,
    ) {
        if let MutationAttribution::Confirmed(mut evidence) = attribution {
            if let (Some(observed), Some(head)) = (observed, head) {
                evidence.retain_observed(observed, head);
            }
            self.pending.push(*evidence);
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.pending.clear();
        self.ambiguous.clear();
    }

    pub(super) fn before_refresh(&mut self, repo: &Repository) -> BTreeSet<PathBuf> {
        let mut quiet = BTreeSet::new();
        for evidence in &self.pending {
            let candidates = evidence.changed_paths();
            let matching = evidence.quiet_paths(repo);
            self.ambiguous
                .extend(candidates.difference(&matching).cloned());
            quiet.extend(matching);
        }
        quiet.difference(&self.ambiguous).cloned().collect()
    }

    pub(super) fn finish(
        &mut self,
        repo: &Repository,
        before: &BTreeSet<PathBuf>,
        result: &Result<Snapshot, GitError>,
    ) -> Option<Update> {
        if self.pending.is_empty() {
            return None;
        }
        let observations = result.as_ref().ok()?.activity.as_ref()?;
        let quiet_paths: BTreeSet<_> = self
            .pending
            .iter()
            .flat_map(|evidence| {
                evidence.quiet_paths(repo).into_iter().filter(|path| {
                    before.contains(path)
                        && !self.ambiguous.contains(path)
                        && evidence.matches_snapshot(path, observations)
                })
            })
            .collect();
        let external = self
            .pending
            .iter()
            .flat_map(IndexMutation::changed_paths)
            .filter(|path| !quiet_paths.contains(path) && observations.contains_key(path))
            .collect();
        let quiet = observations
            .iter()
            .filter(|(path, _)| quiet_paths.contains(*path))
            .map(|(path, observation)| (path.clone(), observation.clone()))
            .collect();
        self.pending.clear();
        self.ambiguous.clear();
        Some(Update::InternalActivity(quiet_paths, quiet, external))
    }
}

#[cfg(test)]
mod tests;
