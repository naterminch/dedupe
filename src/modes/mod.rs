mod exif;
mod extensions;
mod findings;
mod folders;
mod glob;
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
use std::path::PathBuf;

pub use exif::{ExifReport, strip_exif};
pub use glob::glob_match;
pub use names::{MatchBucket, match_entries, name_similarity};

pub const AUDIO_EXTS: [&str; 12] = [
    "mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav", "wma", "aiff", "aif", "ape",
];

#[derive(Debug, Clone)]
pub struct Finding {
    pub path: PathBuf,
    pub size: u64,
    pub kind: &'static str,
    pub detail: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct FindingGroup {
    pub label: String,
    pub detail: String,
    pub members: Vec<PathBuf>,
    pub sizes: Vec<u64>,
    pub bytes: u64,
    pub reclaimable: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ModeResult {
    pub mode: FindMode,
    pub findings: Vec<Finding>,
    pub groups: Vec<FindingGroup>,
    pub files_scanned: u64,
    pub dirs_scanned: u64,
    pub bytes_scanned: u64,
    pub files_skipped: u64,
    pub dirs_skipped: u64,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
    pub cancelled: bool,
}

impl ModeResult {
    #[must_use]
    pub fn finding_count(&self) -> usize {
        self.findings.len()
    }

    #[must_use]
    pub fn reclaimable(&self) -> u64 {
        self.findings
            .iter()
            .filter(|f| !f.is_dir)
            .map(|f| f.size)
            .sum::<u64>()
            + self.groups.iter().map(|g| g.reclaimable).sum::<u64>()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.findings.is_empty() && self.groups.is_empty()
    }
}

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
