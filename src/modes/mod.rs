mod exif;
mod extensions;
mod findings;
mod folders;
mod music;
mod names;
mod walk;

use self::extensions::{bad_extensions, bad_names};
use self::findings::{broken_files, empty_dirs, finding, invalid_symlinks, temp_reason};
use self::folders::duplicate_folders;
use self::music::music_groups;
use self::walk::walk_tree;
use crate::cancel::CancelToken;
use crate::cli::{Cli, FindMode};
use crate::hashing::HashEngine;
use crate::scan::FileEntry;
use std::path::{Path, PathBuf};

pub use crate::glob::glob_match;
pub use exif::{ExifReport, strip_exif};
pub use names::{MatchBucket, match_entries, name_similarity};

/// Extensions the music mode treats as audio, lowercase and without the dot.
pub const AUDIO_EXTS: [&str; 12] = [
    "mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav", "wma", "aiff", "aif", "ape",
];

/// One item a cleanup mode reported.
///
/// `#[non_exhaustive]`: fields grow as new safety checks appear (the content
/// hash was the last one), and callers outside the crate should build these
/// with [`Finding::default`] and field assignment rather than a literal.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Finding {
    /// The item the mode reported.
    pub path: PathBuf,
    /// Size in bytes; 0 for directories.
    pub size: u64,
    /// Short machine-readable reason ("empty", "temp", "broken", …).
    pub kind: &'static str,
    /// Human-readable explanation shown next to the path.
    pub detail: String,
    /// A folder rather than a file, which deletion treats differently.
    pub is_dir: bool,
    /// Content hash recorded when the scan ran, so a deletion can prove the
    /// file has not changed since. `None` when the content could not be read,
    /// which deletion treats as unverified and keeps.
    pub hash: Option<String>,
}

/// A set of items a cleanup mode treats as copies of each other.
///
/// `#[non_exhaustive]` for the same reason as [`Finding`]: build with
/// [`FindingGroup::default`] plus field assignment outside the crate.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FindingGroup {
    /// Short heading, e.g. "Identical folder tree (2 copies)".
    pub label: String,
    /// Supporting line of text for the heading.
    pub detail: String,
    /// The copies, in display order.
    pub members: Vec<PathBuf>,
    /// Size per member, parallel to `members`.
    pub sizes: Vec<u64>,
    /// Content hash per member, parallel to `sizes`, recorded at scan time
    /// (see [`attach_content_hashes`]).
    pub hashes: Vec<Option<String>>,
    /// Total size of one copy of the group's content.
    pub bytes: u64,
    /// Space freed if every member but one is removed.
    pub reclaimable: u64,
}

/// Everything one cleanup-mode run produced.
#[derive(Debug, Clone, Default)]
pub struct ModeResult {
    /// Which mode ran.
    pub mode: FindMode,
    /// Individual items, one per finding.
    pub findings: Vec<Finding>,
    /// Items the mode treated as copies of each other.
    pub groups: Vec<FindingGroup>,
    /// Files the walker handed to the mode.
    pub files_scanned: u64,
    /// Directories the walker entered.
    pub dirs_scanned: u64,
    /// Total size of those files.
    pub bytes_scanned: u64,
    /// Files skipped by size, type or ignore rules.
    pub files_skipped: u64,
    /// Directories skipped by depth, exclusion or ignore rules.
    pub dirs_skipped: u64,
    /// Problems that stopped a file or directory from being checked.
    pub errors: Vec<String>,
    /// Informational asides, e.g. why a check could not run.
    pub notes: Vec<String>,
    /// The run stopped early because the user cancelled.
    pub cancelled: bool,
}

impl ModeResult {
    /// Number of individual findings, not counting group members.
    #[must_use]
    pub fn finding_count(&self) -> usize {
        self.findings.len()
    }

    /// Bytes freed if every finding and every non-keeper group member were
    /// removed; folders contribute nothing.
    #[must_use]
    pub fn reclaimable(&self) -> u64 {
        self.findings
            .iter()
            .filter(|f| !f.is_dir)
            .map(|f| f.size)
            .sum::<u64>()
            + self.groups.iter().map(|g| g.reclaimable).sum::<u64>()
    }

    /// True when the mode found nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty() && self.groups.is_empty()
    }
}

/// What a candidate holds: file bytes, or a symlink's target so a broken link
/// can still be verified before it is unlinked. `None` for directories and for
/// anything unreadable, which deletion treats as "cannot prove it unchanged"
/// and keeps.
#[must_use]
pub fn content_hash(path: &Path, engine: &HashEngine) -> Option<String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::read_link(path)
            .ok()
            .map(|target| engine.bytes(target.to_string_lossy().as_bytes())),
        Ok(meta) if meta.is_dir() => None,
        Ok(_) => engine.full(path).ok(),
        Err(_) => None,
    }
}

/// Record the content of every deletable item, so deletion can re-hash it and
/// skip the ones that changed in between.
///
/// Only a run that is about to delete pays for this: hashing is per finding,
/// and a report-only scan (or `--dry-run`) must not read the files it merely
/// lists. A cancelled scan stops early, leaving the rest unhashed so they are
/// kept rather than removed unverified.
pub fn attach_content_hashes(result: &mut ModeResult, engine: &HashEngine, cancel: &CancelToken) {
    for finding in &mut result.findings {
        if cancel.is_cancelled() {
            return;
        }
        finding.hash = if still_same_size(&finding.path, finding.size) {
            content_hash(&finding.path, engine)
        } else {
            // The file already changed since it was classified: hashing it now
            // would bless the new contents, so record nothing and let the
            // deletion keep it.
            None
        };
    }
    for group in &mut result.groups {
        if cancel.is_cancelled() {
            return;
        }
        group.hashes = group
            .members
            .iter()
            .zip(&group.sizes)
            .map(|(path, size)| {
                if still_same_size(path, *size) {
                    content_hash(path, engine)
                } else {
                    None
                }
            })
            .collect();
    }
}

/// Whether a candidate still looks like the entry the scan classified. Symlinks
/// report their own size, so they count as unchanged; a regular file that grew
/// or shrank between detection and hashing does not.
fn still_same_size(path: &Path, expected: u64) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => true,
        Ok(meta) if meta.is_file() => meta.len() == expected,
        _ => false,
    }
}

/// Stable machine name for a mode (`empty-files`), used in JSON and in the
/// report's mode column.
#[must_use]
pub fn mode_name(mode: FindMode) -> &'static str {
    match mode {
        FindMode::Duplicates => "duplicates",
        FindMode::EmptyFiles => "empty-files",
        FindMode::EmptyDirs => "empty-dirs",
        FindMode::BigFiles => "big-files",
        FindMode::TempFiles => "temp-files",
        FindMode::InvalidSymlinks => "invalid-symlinks",
        FindMode::BrokenFiles => "broken-files",
        FindMode::BadExtensions => "bad-extensions",
        FindMode::BadNames => "bad-names",
        FindMode::DuplicateFolders => "duplicate-folders",
        FindMode::Music => "music",
    }
}

/// Human title for a mode, used as the report heading and window title.
#[must_use]
pub fn mode_title(mode: FindMode) -> &'static str {
    match mode {
        FindMode::Duplicates => "Duplicate files",
        FindMode::EmptyFiles => "Empty files",
        FindMode::EmptyDirs => "Empty folders",
        FindMode::BigFiles => "Largest files",
        FindMode::TempFiles => "Temporary and backup files",
        FindMode::InvalidSymlinks => "Broken symlinks",
        FindMode::BrokenFiles => "Corrupt or unreadable media",
        FindMode::BadExtensions => "Files with the wrong extension",
        FindMode::BadNames => "Problematic file names",
        FindMode::DuplicateFolders => "Duplicate folder trees",
        FindMode::Music => "Duplicate music",
    }
}

/// Walk the scan roots and run the cleanup mode `cli.find` selects.
///
/// Stops early when `cancel` fires, returning what had been collected; the
/// caller can tell by [`ModeResult::cancelled`].
#[must_use]
pub fn run_mode(cli: &Cli, cancel: &CancelToken) -> ModeResult {
    let tree = walk_tree(cli, cancel);
    let mut result = ModeResult {
        mode: cli.find,
        files_scanned: tree.files.len() as u64,
        dirs_scanned: tree.dirs.len() as u64,
        bytes_scanned: tree.files.iter().map(|f| f.size).sum(),
        files_skipped: tree.files_skipped,
        dirs_skipped: tree.dirs_skipped,
        errors: tree.errors.clone(),
        cancelled: cancel.is_cancelled(),
        ..ModeResult::default()
    };
    if cancel.is_cancelled() {
        return result;
    }
    match cli.find {
        FindMode::Duplicates => {}
        FindMode::EmptyFiles => {
            result.findings = tree
                .files
                .iter()
                .filter(|f| f.size == 0)
                .map(|f| finding(f.path.clone(), 0, "empty", "zero-byte file", false))
                .collect();
        }
        FindMode::EmptyDirs => {
            result.findings = empty_dirs(&tree);
        }
        FindMode::BigFiles => {
            let mut files: Vec<&FileEntry> = tree.files.iter().collect();
            files.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
            let total = files.len();
            let take = cli.top.max(1);
            result.findings = files
                .into_iter()
                .take(take)
                .enumerate()
                .map(|(i, f)| {
                    finding(
                        f.path.clone(),
                        f.size,
                        "big",
                        format!("#{} of {} scanned files", i + 1, total),
                        false,
                    )
                })
                .collect();
        }
        FindMode::TempFiles => {
            result.findings = tree
                .files
                .iter()
                .filter_map(|f| {
                    temp_reason(&f.path)
                        .map(|reason| finding(f.path.clone(), f.size, "temp", reason, false))
                })
                .collect();
        }
        FindMode::InvalidSymlinks => {
            result.findings = invalid_symlinks(&tree);
        }
        FindMode::BrokenFiles => {
            result.findings = broken_files(&tree, &mut result.notes, cancel);
        }
        FindMode::BadExtensions => {
            result.findings = bad_extensions(&tree);
        }
        FindMode::BadNames => {
            result.findings = bad_names(&tree);
        }
        FindMode::DuplicateFolders => {
            let engine = HashEngine::new(cli.hash);
            let (groups, notes) = duplicate_folders(&tree, &engine, cancel);
            result.groups = groups;
            result.notes.extend(notes);
        }
        FindMode::Music => {
            result.groups = music_groups(cli, &tree, &mut result.notes, cancel);
        }
    }
    result.findings.sort_by(|a, b| a.path.cmp(&b.path));
    result.cancelled = cancel.is_cancelled();
    result
}

#[cfg(test)]
mod tests;
