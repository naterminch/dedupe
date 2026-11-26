//! Putting a recorded run back.
//!
//! A run from the clean-up history ([`crate::history`]) comes back file by
//! file where that is still possible: a trashed file from the system trash,
//! a moved or gathered one from the folder it went to. Nothing is ever
//! overwritten: a file whose place is taken now stays where it is, and so
//! does one that is gone from the trash or from its folder, or a moved copy
//! that changed since. Deleted-for-good files, hard links and stripped
//! metadata are listed with the reason they cannot come back.

use super::fileops::move_back;
use crate::cli::HashAlgo;
use crate::hashing::HashEngine;
use crate::history::{self, Item, Run, Went};
use std::path::{Path, PathBuf};

/// Whether this platform lets an app put files back from the trash: the
/// `trash` crate's restore exists on Windows and on Freedesktop-trash Unix
/// only. Elsewhere (macOS) trashed files are listed as restorable by hand,
/// while moved files still come back.
pub const CAN_RESTORE_TRASH: bool = cfg!(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
));

/// What happens, or happened, to one file of a run on restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Comes back from the trash.
    FromTrash,
    /// Moves back from where the run put it.
    FromFolder(PathBuf),
    /// This restore put it back.
    Restored,
    /// An earlier restore already put it back.
    AlreadyBack,
    /// Something is at its original place now; it is never overwritten.
    Taken,
    /// The trash no longer holds it (emptied, or restored by hand).
    NotInTrash,
    /// The folder it was moved to no longer holds it.
    NotInFolder(PathBuf),
    /// The moved copy no longer holds the bytes it left with.
    Changed(PathBuf),
    /// In the trash, but this platform gives apps no way to restore it.
    NoTrashRestore,
    /// Deleted for good.
    DeletedForGood,
    /// Replaced by a hard link to this keeper, so its path still shows the
    /// same content.
    Linked(PathBuf),
    /// Its metadata was stripped in place.
    Rewritten,
    /// Putting it back was tried and failed.
    Failed(String),
}

impl Verdict {
    /// Whether a restore would bring this file back.
    #[must_use]
    pub fn comes_back(&self) -> bool {
        matches!(self, Self::FromTrash | Self::FromFolder(_))
    }

    /// The verdict in plain words, short enough to follow a file's name.
    #[must_use]
    pub fn says(&self) -> String {
        match self {
            Self::FromTrash => "Comes back from the trash".to_string(),
            Self::FromFolder(at) => format!("Moves back from {}", at.display()),
            Self::Restored => "Back where it was".to_string(),
            Self::AlreadyBack => "Already back where it was".to_string(),
            Self::Taken => {
                "Another file is at its place now, and it is never overwritten".to_string()
            }
            Self::NotInTrash => "No longer in the trash".to_string(),
            Self::NotInFolder(at) => format!("No longer at {}", at.display()),
            Self::Changed(at) => format!("The copy at {} changed after it was moved", at.display()),
            Self::NoTrashRestore => {
                "In the trash: this system lets you put it back from the Trash yourself".to_string()
            }
            Self::DeletedForGood => "Deleted for good".to_string(),
            Self::Linked(keeper) => format!(
                "Now a hard link to {}, with the same content",
                keeper.display()
            ),
            Self::Rewritten => "Its metadata was removed for good".to_string(),
            Self::Failed(reason) => format!("Could not be put back: {reason}"),
        }
    }
}

/// One file of a run with its [`Verdict`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    /// Where the file was before the run, and goes back to.
    pub path: PathBuf,
    pub size: u64,
    pub verdict: Verdict,
}

/// What [`restore_run`] did.
#[derive(Debug)]
pub struct RestoreOutcome {
    /// The run as it was before this restore.
    pub run: Run,
    /// Every file the restore was asked about, with what happened to it.
    pub items: Vec<PlannedItem>,
    /// Files put back.
    pub restored: Vec<PathBuf>,
    /// Their bytes.
    pub bytes: u64,
}

/// Every file of `run` with what a restore would do with it now. Reads the
/// disk and the trash, so callers run it off the UI thread.
#[must_use]
pub fn plan_restore(run: &Run) -> Vec<PlannedItem> {
    let trash = list_trash(run);
    run.items
        .iter()
        .map(|item| PlannedItem {
            path: item.path.clone(),
            size: item.size,
            verdict: verdict(run, item, trash.as_ref()),
        })
        .collect()
}

/// Put back the files of run `id` that `wanted` picks and that can still
/// come back, and record them as back in the history. `None` when the
/// history holds no run `id`.
pub fn restore_run(id: u64, wanted: impl Fn(&Path) -> bool) -> Option<RestoreOutcome> {
    let run = history::find(id)?;
    let trash = list_trash(&run);
    let engine = HashEngine::new(hash_algo(&run.hash_algo));
    let mut items = Vec::new();
    let mut restored = Vec::new();
    let mut bytes = 0;
    for item in run.items.iter().filter(|item| wanted(&item.path)) {
        let verdict = match verdict(&run, item, trash.as_ref()) {
            Verdict::FromTrash => from_trash(&run, item, trash.as_ref()),
            Verdict::FromFolder(at) => from_folder(item, &at, &engine),
            other => other,
        };
        if verdict == Verdict::Restored {
            restored.push(item.path.clone());
            bytes += item.size;
        }
        items.push(PlannedItem {
            path: item.path.clone(),
            size: item.size,
            verdict,
        });
    }
    // The files are back either way; a history that cannot say so only
    // offers them again, and the next restore finds their place taken.
    if let Err(err) = history::mark_restored(id, &restored) {
        eprintln!("dedupe: could not record the restore in the history: {err}");
    }
    Some(RestoreOutcome {
        run,
        items,
        restored,
        bytes,
    })
}

/// The trash's contents, read once per plan, when the run left anything in
/// it that may still come back; `Err` holds why the trash could not be read.
fn list_trash(run: &Run) -> Option<Result<bin::Listing, String>> {
    let wanted = run
        .items
        .iter()
        .any(|item| !item.restored && item.went == Went::Trash);
    (CAN_RESTORE_TRASH && wanted).then(bin::list)
}

fn verdict(run: &Run, item: &Item, trash: Option<&Result<bin::Listing, String>>) -> Verdict {
    if item.restored {
        return Verdict::AlreadyBack;
    }
    match &item.went {
        Went::Deleted => Verdict::DeletedForGood,
        Went::Linked { keeper } => Verdict::Linked(keeper.clone()),
        Went::Rewritten => Verdict::Rewritten,
        Went::Trash if !CAN_RESTORE_TRASH => Verdict::NoTrashRestore,
        Went::Trash => match trash {
            Some(Ok(listing)) => {
                if bin::find(listing, &item.path, run.when).is_none() {
                    Verdict::NotInTrash
                } else if occupied(&item.path) {
                    Verdict::Taken
                } else {
                    Verdict::FromTrash
                }
            }
            Some(Err(reason)) => Verdict::Failed(format!("could not read the trash: {reason}")),
            None => Verdict::NotInTrash,
        },
        Went::Moved { path: at } => {
            if !occupied(at) {
                Verdict::NotInFolder(at.clone())
            } else if occupied(&item.path) {
                Verdict::Taken
            } else {
                Verdict::FromFolder(at.clone())
            }
        }
    }
}

/// Whether anything — a file, a folder, a dangling link — is at `path`.
fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn from_trash(run: &Run, item: &Item, trash: Option<&Result<bin::Listing, String>>) -> Verdict {
    let Some(Ok(listing)) = trash else {
        return Verdict::NotInTrash;
    };
    match bin::find(listing, &item.path, run.when) {
        Some(entry) => bin::restore(entry),
        None => Verdict::NotInTrash,
    }
}

/// Move a moved file back, after proving it still holds the bytes it left
/// with (when the run knew them).
fn from_folder(item: &Item, at: &Path, engine: &HashEngine) -> Verdict {
    if let Some(expected) = &item.hash {
        match engine.full(at) {
            Ok(hash) if &hash == expected => {}
            Ok(_) => return Verdict::Changed(at.to_path_buf()),
            Err(err) => return Verdict::Failed(format!("could not read {}: {err}", at.display())),
        }
    }
    match move_back(at, &item.path) {
        Ok(true) => Verdict::Restored,
        Ok(false) => Verdict::Taken,
        Err(err) => Verdict::Failed(err.to_string()),
    }
}

/// The run's hash algorithm, by the name the history stores.
fn hash_algo(name: &str) -> HashAlgo {
    match name {
        "sha256" => HashAlgo::Sha256,
        "md5" => HashAlgo::Md5,
        _ => HashAlgo::Blake3,
    }
}

/// The trash, where an app can read and restore it.
#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
mod bin {
    use super::Verdict;
    use std::path::Path;
    use trash::TrashItem;

    pub(super) type Listing = Vec<TrashItem>;

    pub(super) fn list() -> Result<Listing, String> {
        trash::os_limited::list().map_err(|e| e.to_string())
    }

    /// The trash entry for `path` that a run at `when` left: of several
    /// (the same path trashed by more than one run), the one deleted
    /// closest to the run.
    pub(super) fn find<'a>(listing: &'a Listing, path: &Path, when: i64) -> Option<&'a TrashItem> {
        listing
            .iter()
            .filter(|entry| same_path(&entry.original_path(), path))
            .min_by_key(|entry| {
                // Some Linux trashes report no deletion time (negative).
                if entry.time_deleted < 0 {
                    i64::MAX
                } else {
                    (entry.time_deleted - when).abs()
                }
            })
    }

    pub(super) fn restore(entry: &TrashItem) -> Verdict {
        match trash::os_limited::restore_all([entry.clone()]) {
            Ok(()) => Verdict::Restored,
            Err(trash::Error::RestoreCollision { .. }) => Verdict::Taken,
            Err(err) => Verdict::Failed(err.to_string()),
        }
    }

    /// Paths compared as the platform does: Windows ignores case and the
    /// `\\?\` prefix.
    fn same_path(a: &Path, b: &Path) -> bool {
        if cfg!(windows) {
            let plain = |p: &Path| {
                let text = p.to_string_lossy().to_lowercase();
                text.strip_prefix(r"\\?\")
                    .map_or(text.clone(), str::to_string)
            };
            plain(a) == plain(b)
        } else {
            a == b
        }
    }
}

/// No trash an app can restore from: nothing is ever listed.
#[cfg(not(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
mod bin {
    use super::Verdict;
    use std::path::Path;

    #[derive(Debug)]
    pub(super) struct Entry;

    pub(super) type Listing = Vec<Entry>;

    pub(super) fn list() -> Result<Listing, String> {
        Ok(Vec::new())
    }

    pub(super) fn find<'a>(_: &'a Listing, _: &Path, _: i64) -> Option<&'a Entry> {
        None
    }

    pub(super) fn restore(_: &Entry) -> Verdict {
        Verdict::NoTrashRestore
    }
}
