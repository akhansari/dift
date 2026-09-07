use crate::{config::Whitespace, git::GitError, model::Snapshot};
use std::{collections::BTreeSet, path::PathBuf};

/// Inputs for an explicit collection request; watch refreshes reuse the latest request.
#[derive(Debug, Default)]
pub struct CollectionRequest {
    pub generation: u64,
    pub loaded: BTreeSet<PathBuf>,
    pub whitespace: Whitespace,
}

/// Owns requested collection settings and rejects superseded worker updates.
/// Applied settings remain part of the snapshot until collection succeeds.
pub(super) struct CollectionState {
    pub(super) loaded: BTreeSet<PathBuf>,
    pub(super) error: Option<String>,
    pub(super) collecting: bool,
    pub(super) whitespace: Whitespace,
    generation: u64,
    ignore_whitespace: Whitespace,
    whitespace_toast_pending: bool,
}

impl CollectionState {
    pub(super) fn new(whitespace: Whitespace) -> Self {
        Self {
            loaded: BTreeSet::new(),
            error: None,
            collecting: false,
            whitespace,
            generation: 0,
            ignore_whitespace: if whitespace == Whitespace::Exact {
                Whitespace::IgnoreChanges
            } else {
                whitespace
            },
            whitespace_toast_pending: false,
        }
    }

    pub(super) fn request(&mut self) -> CollectionRequest {
        self.generation += 1;
        self.collecting = true;
        self.error = None;
        CollectionRequest {
            generation: self.generation,
            loaded: self.loaded.clone(),
            whitespace: self.whitespace,
        }
    }

    pub(super) fn started(&mut self, generation: u64) {
        if generation == self.generation {
            self.collecting = true;
        }
    }

    pub(super) fn finish(
        &mut self,
        generation: u64,
        result: Result<Snapshot, GitError>,
    ) -> Option<Result<Snapshot, GitError>> {
        if generation != self.generation {
            return None;
        }
        self.collecting = false;
        self.error = result.as_ref().err().map(ToString::to_string);
        Some(result)
    }

    pub(super) fn toggle_whitespace(&mut self) {
        self.whitespace = if self.whitespace == Whitespace::Exact {
            self.ignore_whitespace
        } else {
            Whitespace::Exact
        };
        self.whitespace_toast_pending = true;
    }

    pub(super) fn take_whitespace_notification(&mut self) -> bool {
        std::mem::take(&mut self.whitespace_toast_pending)
    }
}
