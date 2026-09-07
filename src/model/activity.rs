use super::Category;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::SystemTime};

pub type ActivityObservations = BTreeMap<PathBuf, BTreeMap<Category, ActivityObservation>>;

/// Repository identity excludes preview limits, whitespace policy, and UI settings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GitIdentity {
    pub status: char,
    pub old_path: Option<PathBuf>,
    pub old_mode: String,
    pub new_mode: String,
    pub old_oid: String,
    /// Unstaged files use their working-tree observation instead of Git's raw new OID.
    pub new_oid: Option<String>,
    pub conflict_stages: Vec<IndexStage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStage {
    pub stage: u32,
    pub mode: u32,
    pub oid: String,
}

/// Cheap fallback for files whose full source is unavailable. Missing is distinct
/// from an unsuccessful inspection (represented by an absent observation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeMetadata {
    Missing,
    Present {
        regular: bool,
        symlink: bool,
        bytes: u64,
        modified: Option<SystemTime>,
        readonly: bool,
        link_target: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityObservation {
    pub git: GitIdentity,
    pub metadata: Option<WorktreeMetadata>,
    /// Share already loaded raw text; never load more content just for activity.
    pub source: Option<Arc<str>>,
}

impl ActivityObservation {
    pub fn changed_since(&self, previous: &Self) -> bool {
        self.git != previous.git
            || match (&self.source, &previous.source) {
                (Some(current), Some(previous)) => current != previous,
                _ => matches!((&self.metadata, &previous.metadata),
                    (Some(current), Some(previous)) if current != previous),
            }
    }
}
