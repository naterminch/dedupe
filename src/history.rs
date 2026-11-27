//! The clean-up history: one record per run that changed files on disk.
//!
//! Every delete, move, hard link, consolidate and metadata strip — from the
//! desktop app and the command line alike — leaves a [`Run`] here: when it
//! happened, what it did, the folders it scanned, and for every file where
//! it went. The GUI's History page and `dedupe --history` list them, and
//! [`crate::actions::restore_run`] puts back what can still come back,
//! marking each file it returns so a run shows what is already back.
//!
//! Stored as JSON in `~/.dedupe/history.json` (`%USERPROFILE%\.dedupe\...` on
//! Windows), overridable with the `DEDUPE_HISTORY` env var. Like
//! [`crate::prefs`] and [`crate::cache`], it never blocks the work it
//! records: writes are atomic (temp file + rename), a write that fails is
//! reported and skipped, and a file that cannot be read is set aside as
//! `history.json.bad` and a fresh history started. Only the newest
//! [`MAX_RUNS`] runs are kept.

use crate::actions::Disposition;
use crate::ui_helpers::counted;
use crate::util::human_bytes;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Schema version; a file with another version is set aside, not misread.
const HISTORY_VERSION: u32 = 1;

/// How many runs the history keeps; older ones are dropped as new ones land.
pub const MAX_RUNS: usize = 200;

/// Serializes this process's read-modify-write cycles: the GUI records and
/// restores from worker threads.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// What a run set out to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunKind {
    /// Duplicate copies deleted (to the trash or for good).
    Duplicates,
    /// A cleanup scan's findings deleted.
    Findings,
    /// Duplicate copies moved into a folder (`--move-dir`).
    MoveAside,
    /// Duplicate copies replaced by hard links to their keeper.
    HardLink,
    /// Keepers gathered into one folder (`--consolidate-dir`).
    Consolidate,
    /// EXIF/XMP metadata removed from photos in place.
    StripMetadata,
}

/// How a run's files left their place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    Trash,
    Permanent,
    Moved,
    Linked,
    Rewritten,
}

impl From<Disposition> for Method {
    fn from(disposition: Disposition) -> Self {
        match disposition {
            Disposition::Trash => Self::Trash,
            Disposition::Permanent => Self::Permanent,
        }
    }
}

/// Where one file went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "kebab-case")]
pub enum Went {
    /// Into the system trash.
    Trash,
    /// Deleted for good.
    Deleted,
    /// Moved to `path`.
    Moved { path: PathBuf },
    /// Replaced by a hard link to `keeper`.
    Linked { keeper: PathBuf },
    /// Rewritten in place without its metadata.
    Rewritten,
}

impl From<Disposition> for Went {
    fn from(disposition: Disposition) -> Self {
        match disposition {
            Disposition::Trash => Self::Trash,
            Disposition::Permanent => Self::Deleted,
        }
    }
}

/// One file a run changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Where the file was before the run.
    pub path: PathBuf,
    /// The bytes this file accounts for in the run's total: its size, or for
    /// a metadata strip the bytes the strip removed.
    pub size: u64,
    pub went: Went,
    /// The file's content hash at the time of the run, when one was known
    /// (in the run's [`Run::hash_algo`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// Put back by a restore since.
    #[serde(default)]
    pub restored: bool,
}

impl Item {
    #[must_use]
    pub fn new(path: PathBuf, size: u64, went: Went, hash: Option<String>) -> Self {
        Self {
            path,
            size,
            went,
            hash,
            restored: false,
        }
    }
}

/// One recorded run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Unique and never reused, also after old runs are dropped.
    pub id: u64,
    /// When the run finished, in seconds since the Unix epoch.
    pub when: i64,
    pub kind: RunKind,
    pub method: Method,
    /// The folders the scan behind the run looked at.
    pub roots: Vec<PathBuf>,
    /// The algorithm of the items' hashes ("blake3", "sha256", "md5").
    pub hash_algo: String,
    /// Bytes the run freed (moved, for a move or a consolidate).
    pub bytes: u64,
    pub items: Vec<Item>,
}

impl Run {
    /// Files of this run that a restore has put back.
    #[must_use]
    pub fn back(&self) -> usize {
        self.items.iter().filter(|item| item.restored).count()
    }

    /// What the run did, in a short plain sentence.
    #[must_use]
    pub fn describe(&self) -> String {
        let n = self.items.len() as u64;
        let copies = counted(n, "duplicate copy", "duplicate copies");
        let found = counted(n, "found item", "found items");
        let folder = self
            .destination()
            .map_or_else(|| "a folder".to_string(), |dir| dir.display().to_string());
        match (self.kind, self.method) {
            (RunKind::Findings, Method::Trash) => format!("Moved {found} to the trash"),
            (RunKind::Findings, _) => format!("Deleted {found} for good"),
            (RunKind::MoveAside, _) => format!("Moved {copies} to {folder}"),
            (RunKind::HardLink, _) => format!("Replaced {copies} with hard links"),
            (RunKind::Consolidate, _) => {
                format!("Gathered {} into {folder}", counted(n, "keeper", "keepers"))
            }
            (RunKind::StripMetadata, _) => format!(
                "Removed the metadata from {}",
                counted(n, "photo", "photos")
            ),
            (RunKind::Duplicates, Method::Trash) => format!("Moved {copies} to the trash"),
            (RunKind::Duplicates, _) => format!("Deleted {copies} for good"),
        }
    }

    /// The run's size in words: what it freed, or what it moved.
    #[must_use]
    pub fn size_label(&self) -> String {
        let bytes = human_bytes(self.bytes);
        match self.method {
            Method::Trash => format!("{bytes} in the trash"),
            Method::Moved => format!("{bytes} moved"),
            Method::Permanent | Method::Linked | Method::Rewritten => format!("{bytes} freed"),
        }
    }

    /// The folder a move or consolidate put its files into: the first moved
    /// file's folder.
    fn destination(&self) -> Option<&Path> {
        self.items.iter().find_map(|item| match &item.went {
            Went::Moved { path } => path.parent(),
            _ => None,
        })
    }
}

/// A run about to be recorded; [`record`] gives it its id and time.
#[derive(Debug)]
pub struct NewRun<'a, R: AsRef<Path>> {
    pub kind: RunKind,
    pub method: Method,
    pub roots: &'a [R],
    pub hash_algo: &'a str,
    pub bytes: u64,
    pub items: Vec<Item>,
}

/// The file on disk.
#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    version: u32,
    /// The id the next run gets.
    next_id: u64,
    /// Newest first.
    runs: Vec<Run>,
}

impl Default for Journal {
    fn default() -> Self {
        Self {
            version: HISTORY_VERSION,
            next_id: 1,
            runs: Vec::new(),
        }
    }
}

/// Default history location: `DEDUPE_HISTORY` if set (non-empty), otherwise
/// under the user's home directory, otherwise the current directory.
#[must_use]
pub fn default_history_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DEDUPE_HISTORY")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".dedupe").join("history.json");
    }
    PathBuf::from("history.json")
}

/// Every recorded run, newest first. Never fails: a missing file is an empty
/// history, an unreadable one is set aside (see the module docs).
#[must_use]
pub fn load() -> Vec<Run> {
    load_from(&default_history_path())
}

/// [`load`] from an explicit path.
#[must_use]
pub fn load_from(path: &Path) -> Vec<Run> {
    read_journal(path).runs
}

/// The run with `id`, if the history still holds it.
#[must_use]
pub fn find(id: u64) -> Option<Run> {
    load().into_iter().find(|run| run.id == id)
}

/// Record a run and return its id; `None` when it changed nothing (no
/// record is kept) or the history could not be written, which is reported
/// on stderr and never stops the caller.
pub fn record<R: AsRef<Path>>(run: NewRun<'_, R>) -> Option<u64> {
    let path = default_history_path();
    match record_to(&path, run) {
        Ok(id) => id,
        Err(err) => {
            eprintln!(
                "dedupe: could not record this clean-up in {}: {err}",
                path.display()
            );
            None
        }
    }
}

/// [`record`] into an explicit history file.
///
/// # Errors
///
/// Returns an error when the history cannot be written.
pub fn record_to<R: AsRef<Path>>(path: &Path, run: NewRun<'_, R>) -> io::Result<Option<u64>> {
    if run.items.is_empty() {
        return Ok(None);
    }
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut journal = read_journal(path);
    let id = journal.next_id;
    journal.next_id += 1;
    journal.runs.insert(
        0,
        Run {
            id,
            when: now(),
            kind: run.kind,
            method: run.method,
            roots: run.roots.iter().map(|r| absolute(r.as_ref())).collect(),
            hash_algo: run.hash_algo.to_string(),
            bytes: run.bytes,
            items: run.items.into_iter().map(Item::absolute).collect(),
        },
    );
    journal.runs.truncate(MAX_RUNS);
    write_journal(path, &journal)?;
    Ok(Some(id))
}

/// `path` made absolute against the working directory, so a run recorded by
/// `dedupe . --delete` can be put back from anywhere. Nothing is resolved on
/// disk: the file is usually gone from there.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl Item {
    fn absolute(mut self) -> Self {
        self.path = absolute(&self.path);
        match &mut self.went {
            Went::Moved { path } => *path = absolute(path),
            Went::Linked { keeper } => *keeper = absolute(keeper),
            Went::Trash | Went::Deleted | Went::Rewritten => {}
        }
        self
    }
}

/// Mark `paths` of run `id` as put back.
///
/// # Errors
///
/// Returns an error when the history cannot be written.
pub fn mark_restored(id: u64, paths: &[PathBuf]) -> io::Result<()> {
    mark_restored_in(&default_history_path(), id, paths)
}

/// [`mark_restored`] in an explicit history file.
///
/// # Errors
///
/// Returns an error when the history cannot be written.
pub fn mark_restored_in(path: &Path, id: u64, paths: &[PathBuf]) -> io::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut journal = read_journal(path);
    let Some(run) = journal.runs.iter_mut().find(|run| run.id == id) else {
        return Ok(());
    };
    for item in &mut run.items {
        if paths.contains(&item.path) {
            item.restored = true;
        }
    }
    write_journal(path, &journal)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The time of a run in local time, as the command line prints it.
#[must_use]
pub fn local_stamp(when: i64) -> String {
    chrono::DateTime::from_timestamp(when, 0).map_or_else(String::new, |utc| {
        utc.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    })
}

/// The time of a run as the History page shows it: "Today, 14:03",
/// "Yesterday, 09:12", or "3 Nov 2026, 18:40".
#[must_use]
pub fn friendly_stamp(when: i64) -> String {
    let Some(at) = chrono::DateTime::from_timestamp(when, 0) else {
        return String::new();
    };
    let at = at.with_timezone(&chrono::Local);
    let today = chrono::Local::now().date_naive();
    let day = at.date_naive();
    let time = at.format("%H:%M");
    if day == today {
        format!("Today, {time}")
    } else if today.pred_opt() == Some(day) {
        format!("Yesterday, {time}")
    } else {
        at.format("%-d %b %Y, %H:%M").to_string()
    }
}

/// Read the journal; a missing file is an empty one, a corrupt or foreign
/// one is moved aside to `<name>.bad` so the next write starts fresh without
/// destroying it.
fn read_journal(path: &Path) -> Journal {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Journal::default(),
        Err(err) => {
            eprintln!(
                "dedupe: could not read the clean-up history at {}: {err}",
                path.display()
            );
            return Journal::default();
        }
    };
    let problem = match serde_json::from_slice::<Journal>(&bytes) {
        Ok(journal) if journal.version == HISTORY_VERSION => return journal,
        Ok(journal) => format!("it is version {}", journal.version),
        Err(err) => err.to_string(),
    };
    let aside = path.with_extension("json.bad");
    let kept = fs::rename(path, &aside).is_ok();
    eprintln!(
        "dedupe: the clean-up history at {} could not be read ({problem}); starting a new one{}",
        path.display(),
        if kept {
            format!(", the old file is kept as {}", aside.display())
        } else {
            String::new()
        }
    );
    Journal::default()
}

fn write_journal(path: &Path, journal: &Journal) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(journal).map_err(io::Error::other)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    // Per process: the desktop app and a command-line run may both record.
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dedupe-history-{tag}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn new_run(paths: &[&str]) -> NewRun<'static, &'static str> {
        NewRun {
            kind: RunKind::Duplicates,
            method: Method::Trash,
            roots: &["C:/photos"],
            hash_algo: "blake3",
            bytes: 10 * paths.len() as u64,
            items: paths
                .iter()
                .map(|p| Item::new(std::env::temp_dir().join(p), 10, Went::Trash, None))
                .collect(),
        }
    }

    #[test]
    fn runs_are_listed_newest_first_and_marked_back() {
        let path = tmp_path("roundtrip");
        let first = record_to(&path, new_run(&["a"])).unwrap().unwrap();
        let second = record_to(&path, new_run(&["b", "c"])).unwrap().unwrap();
        assert!(second > first);
        assert_eq!(record_to(&path, new_run(&[])).unwrap(), None);

        mark_restored_in(&path, second, &[std::env::temp_dir().join("c")]).unwrap();
        let runs = load_from(&path);
        assert_eq!(
            runs.iter().map(|r| r.id).collect::<Vec<_>>(),
            [second, first]
        );
        assert_eq!(runs[0].back(), 1);
        assert!(runs[0].items[1].restored && !runs[0].items[0].restored);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn only_the_newest_runs_are_kept_and_ids_are_never_reused() {
        let path = tmp_path("cap");
        let mut last = 0;
        for _ in 0..MAX_RUNS + 5 {
            last = record_to(&path, new_run(&["a"])).unwrap().unwrap();
        }
        let runs = load_from(&path);
        assert_eq!(runs.len(), MAX_RUNS);
        assert_eq!(runs[0].id, last);
        assert_eq!(runs[MAX_RUNS - 1].id, last - MAX_RUNS as u64 + 1);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_corrupt_history_is_set_aside_and_a_new_one_started() {
        let path = tmp_path("corrupt");
        fs::write(&path, b"{ not json").unwrap();
        assert!(load_from(&path).is_empty());
        let aside = path.with_extension("json.bad");
        assert_eq!(fs::read(&aside).unwrap(), b"{ not json");

        let id = record_to(&path, new_run(&["a"])).unwrap();
        assert_eq!(id, Some(1));
        assert_eq!(load_from(&path).len(), 1);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&aside);
    }
}
