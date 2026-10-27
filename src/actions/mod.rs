mod consolidate;
mod delete;
mod fileops;
mod findings;
mod guard;
mod hardlink;
mod quarantine;

use std::path::PathBuf;

pub use consolidate::{consolidate_keepers, consolidate_targets, plan_consolidation};
pub use delete::{delete_duplicates, delete_targets};
pub use findings::{delete_finding_groups, delete_findings};
pub use hardlink::hardlink_duplicates;
pub use quarantine::move_duplicates;

#[derive(Debug, Default)]
pub struct DeletionStats {
    pub files_deleted: u64,
    pub bytes_freed: u64,
    pub files_skipped: u64,
}

/// Where deleted files go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Remove permanently (previous behavior).
    Permanent,
    /// Move to the system trash (recoverable).
    Trash,
}

/// One file the caller wants deleted, with the hash it must still match.
///
/// The hash is verified immediately before deletion: a file that changed
/// since the scan is never removed.
#[derive(Debug, Clone)]
pub struct DeleteTarget {
    pub path: PathBuf,
    pub expected_hash: String,
    pub size: u64,
}

/// Per-file outcome of [`delete_targets`], so callers (e.g. the GUI) can
/// update their own state for exactly the files that were removed.
#[derive(Debug, Default)]
pub struct DeleteOutcome {
    pub deleted: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub bytes_freed: u64,
}

/// One keeper the caller wants moved into the consolidate directory, with
/// the hash it must still match.
///
/// The hash is verified immediately before moving: a file that changed
/// since the scan is never moved.
#[derive(Debug, Clone)]
pub struct MoveTarget {
    pub from: PathBuf,
    pub to: PathBuf,
    pub expected_hash: String,
    pub size: u64,
}

/// A keeper move planned by [`plan_consolidation`], tagged with its group
/// so callers can report per-group progress.
#[derive(Debug, Clone)]
pub struct PlannedMove {
    pub group_index: usize,
    pub target: MoveTarget,
}

/// Per-file outcome of [`consolidate_targets`], so callers (e.g. the GUI)
/// can update their own state for exactly the files that were moved.
#[derive(Debug, Default)]
pub struct ConsolidateOutcome {
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub skipped: Vec<PathBuf>,
    pub bytes_moved: u64,
}

#[derive(Debug, Default)]
pub struct ConsolidateStats {
    pub files_moved: u64,
    pub bytes_moved: u64,
    pub files_skipped: u64,
}

#[derive(Debug, Default)]
pub struct FindingDeleteStats {
    pub deleted: u64,
    pub bytes_freed: u64,
    pub skipped: u64,
}

#[derive(Debug, Default)]
pub struct MoveDuplicateStats {
    pub files_moved: u64,
    pub bytes_moved: u64,
    pub files_skipped: u64,
}

#[derive(Debug, Default)]
pub struct HardlinkStats {
    pub linked: u64,
    pub bytes_saved: u64,
    pub files_skipped: u64,
}

#[cfg(test)]
mod tests;
