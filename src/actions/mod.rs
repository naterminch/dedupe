mod consolidate;
mod delete;
mod error;
mod fileops;
mod findings;
mod guard;
mod hardlink;
mod quarantine;
mod restore;

use std::path::PathBuf;

use crate::history;

pub use consolidate::{consolidate_keepers, move_targets, plan_consolidation};
pub use delete::{delete_duplicates, delete_targets};
pub use error::ActionError;
pub use findings::{delete_finding_groups, delete_findings, is_protected, remove_findings};
pub use hardlink::{can_hardlink, hardlink_duplicates, link_targets};
pub use quarantine::{move_duplicates, plan_move_aside};
pub use restore::{
    CAN_RESTORE_TRASH, PlannedItem, RestoreOutcome, Verdict, plan_restore, restore_run,
};

#[derive(Debug, Default)]
pub struct DeletionStats {
    pub files_deleted: u64,
    pub bytes_freed: u64,
    pub files_skipped: u64,
    /// Removals that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every file that went, as the clean-up history records it.
    pub items: Vec<history::Item>,
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
    /// Removals that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every file that went, as the clean-up history records it.
    pub items: Vec<history::Item>,
    pub bytes_freed: u64,
}

/// One file the caller wants moved — a keeper into the consolidate
/// directory, or a duplicate set aside — with the hash it must still match.
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

/// Per-file outcome of [`move_targets`], so callers (e.g. the GUI) can
/// update their own state for exactly the files that were moved.
#[derive(Debug, Default)]
pub struct MoveOutcome {
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub skipped: Vec<PathBuf>,
    /// Moves that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every file that moved, as the clean-up history records it.
    pub items: Vec<history::Item>,
    pub bytes_moved: u64,
}

/// One duplicate the caller wants replaced by a hard link to its keeper.
///
/// Both files carry the hash they must still match: the duplicate because it
/// is about to be replaced, the keeper because its bytes are what the
/// duplicate's path will show from then on.
#[derive(Debug, Clone)]
pub struct LinkTarget {
    pub path: PathBuf,
    pub expected_hash: String,
    pub size: u64,
    pub keeper: PathBuf,
    pub keeper_hash: String,
    pub keeper_size: u64,
}

/// Per-file outcome of [`link_targets`], so callers (e.g. the GUI) can
/// update their own state for exactly the files that were replaced.
#[derive(Debug, Default)]
pub struct LinkOutcome {
    pub linked: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    /// Links that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every file that was linked, as the clean-up history records it.
    pub items: Vec<history::Item>,
    pub bytes_saved: u64,
}

/// Per-item outcome of [`remove_findings`], so callers (e.g. the GUI) can
/// update their own state for exactly the items that were removed.
#[derive(Debug, Default)]
pub struct FindingOutcome {
    pub removed: Vec<PathBuf>,
    /// Items that changed since the scan, or whose removal failed.
    pub skipped: Vec<PathBuf>,
    /// Items inside a protected folder, never candidates.
    pub protected: Vec<PathBuf>,
    /// Removals that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every item that went, as the clean-up history records it.
    pub items: Vec<history::Item>,
    pub bytes_freed: u64,
}

#[derive(Debug, Default)]
pub struct ConsolidateStats {
    pub files_moved: u64,
    pub bytes_moved: u64,
    pub files_skipped: u64,
    /// Moves that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every keeper that moved, as the clean-up history records it.
    pub items: Vec<history::Item>,
}

#[derive(Debug, Default)]
pub struct FindingDeleteStats {
    pub deleted: u64,
    pub bytes_freed: u64,
    pub skipped: u64,
    /// Removals that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every item that went, as the clean-up history records it.
    pub items: Vec<history::Item>,
}

#[derive(Debug, Default)]
pub struct MoveDuplicateStats {
    pub files_moved: u64,
    pub bytes_moved: u64,
    pub files_skipped: u64,
    /// Moves that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every copy that moved, as the clean-up history records it.
    pub items: Vec<history::Item>,
}

#[derive(Debug, Default)]
pub struct HardlinkStats {
    pub linked: u64,
    pub bytes_saved: u64,
    pub files_skipped: u64,
    /// Links that failed, with the reason, so a front end can explain them.
    pub failures: Vec<(PathBuf, ActionError)>,
    /// Every copy that was linked, as the clean-up history records it.
    pub items: Vec<history::Item>,
}

#[cfg(test)]
mod tests;
