//! Persisted GUI preferences (sidebar options).
//!
//! The GUI starts with hardcoded defaults; without persistence the user
//! re-enters folders and options on every launch. This module stores those
//! selections as human-readable JSON so they survive restarts:
//! `~/.dedupe/gui-prefs.json` (`%USERPROFILE%\.dedupe\...` on Windows),
//! overridable with the `DEDUPE_PREFS` env var.
//!
//! Design notes (mirrors [`crate::cache`]):
//! - Best-effort by design: a missing, corrupt, or version-mismatched file
//!   yields [`GuiPrefs::default()`], never an error at startup.
//! - Writes are atomic (temp file + rename) so a crash mid-save cannot
//!   leave a half-written file.
//! - Only plain option values are stored, never GPUI entity state.

use crate::ui_helpers::NO_LIMIT_LABEL;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{OnceLock, mpsc};

/// Serializes every test that mutates the process-global `DEDUPE_PREFS`
/// env var: this module's override test plus the view tests in
/// [`crate::ui_tests`], which point prefs at a temp file so runs never
/// touch the user's real `~/.dedupe/gui-prefs.json`.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Schema version; bump to invalidate previously stored preferences.
const PREFS_VERSION: u32 = 1;

/// One scan folder with its protection flag (mirrors the sidebar Ref toggle).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FolderPref {
    pub path: String,
    pub reference: bool,
}

/// All sidebar selections worth restoring on next launch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuiPrefs {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub folders: Vec<FolderPref>,
    #[serde(default)]
    pub types: String,
    #[serde(default = "default_no_limit")]
    pub min_size: String,
    #[serde(default = "default_no_limit")]
    pub max_size: String,
    #[serde(default)]
    pub max_depth: String,
    #[serde(default)]
    pub exclude_dir: String,
    #[serde(default)]
    pub exclude_path: String,
    #[serde(default = "default_similarity")]
    pub similarity: String,
    #[serde(default)]
    pub jobs: String,
    #[serde(default)]
    pub hash_index: usize,
    #[serde(default)]
    pub keep_index: usize,
    /// Index into the scan-type list (0 = duplicates).
    #[serde(default)]
    pub find_index: usize,
    /// Index into content / size / name / fuzzy duplicate matching.
    #[serde(default)]
    pub match_index: usize,
    /// Index into auto / dHash / pHash / aHash / color image fingerprints.
    #[serde(default)]
    pub image_hash_index: usize,
    /// Also find shorter clips contained inside longer videos.
    #[serde(default)]
    pub partial_clips: bool,
    #[serde(default = "default_partial_similarity")]
    pub partial_similarity: String,
    #[serde(default = "default_fuzzy_threshold")]
    pub fuzzy_threshold: String,
    /// `big-files`: how many of the largest files to report.
    #[serde(default = "default_top")]
    pub top: String,
    /// Cleanup-mode ignore globs (one per line or `;` separated).
    #[serde(default)]
    pub ignore: String,
    /// Music matching: true = audio fingerprint, false = embedded tags.
    #[serde(default)]
    pub music_by_content: bool,
    #[serde(default)]
    pub exact: bool,
    #[serde(default)]
    pub no_cache: bool,
    #[serde(default = "default_true")]
    pub trash: bool,
    #[serde(default)]
    pub sort_biggest: bool,
    /// Remember scan folders between runs. Defaults to true so existing
    /// installs keep their current behavior; off starts every launch empty.
    #[serde(default = "default_true")]
    pub remember_folders: bool,
    /// True = dark mode. Defaults to light (kit default).
    #[serde(default)]
    pub dark: bool,
}

fn default_version() -> u32 {
    PREFS_VERSION
}

fn default_similarity() -> String {
    "97".to_string()
}

fn default_partial_similarity() -> String {
    "85".to_string()
}

fn default_fuzzy_threshold() -> String {
    "85".to_string()
}

fn default_top() -> String {
    "50".to_string()
}

fn default_no_limit() -> String {
    NO_LIMIT_LABEL.to_string()
}

fn default_true() -> bool {
    true
}

impl Default for GuiPrefs {
    fn default() -> Self {
        Self {
            version: PREFS_VERSION,
            folders: Vec::new(),
            types: String::new(),
            min_size: default_no_limit(),
            max_size: default_no_limit(),
            max_depth: String::new(),
            exclude_dir: String::new(),
            exclude_path: String::new(),
            similarity: default_similarity(),
            jobs: String::new(),
            hash_index: 0,
            keep_index: 0,
            find_index: 0,
            match_index: 0,
            image_hash_index: 0,
            partial_clips: false,
            partial_similarity: default_partial_similarity(),
            fuzzy_threshold: default_fuzzy_threshold(),
            top: default_top(),
            ignore: String::new(),
            music_by_content: false,
            exact: false,
            no_cache: false,
            trash: true,
            sort_biggest: false,
            remember_folders: true,
            dark: false,
        }
    }
}

impl GuiPrefs {
    /// Load from the default location. Never fails: any problem yields defaults.
    pub fn load() -> Self {
        Self::load_from(default_prefs_path())
    }

    /// Load from an explicit path (also used by tests).
    pub fn load_from(path: PathBuf) -> Self {
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => return Self::default(),
        };
        match serde_json::from_slice::<GuiPrefs>(&bytes) {
            Ok(p) if p.version == PREFS_VERSION => p,
            _ => Self::default(),
        }
    }

    /// Persist atomically (temp file + rename). Best-effort: callers ignore
    /// errors so a read-only home dir never breaks the GUI.
    pub fn save(&self) -> io::Result<()> {
        self.save_to(default_prefs_path())
    }

    /// Persist to an explicit path (also used by tests).
    pub fn save_to(&self, path: PathBuf) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        if let Some(parent) = path.parent() {
            // create_dir_all on "" errors; skip it (relative file in cwd).
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Persist off the UI thread. Bursts are coalesced so only the newest
    /// snapshot reaches disk, and callers never wait on the filesystem.
    pub fn save_async(self) {
        type Job = (GuiPrefs, PathBuf);
        static WRITER: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();
        let path = default_prefs_path();
        let tx = WRITER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("dedupe-prefs-writer".into())
                .spawn(move || {
                    while let Ok((mut prefs, mut path)) = rx.recv() {
                        while let Ok((next, next_path)) = rx.try_recv() {
                            prefs = next;
                            path = next_path;
                        }
                        let _ = prefs.save_to(path);
                    }
                })
                .ok()
                .map(|_| tx)
        });
        if let Some(tx) = tx {
            let _ = tx.send((self, path));
        }
    }
}

/// Default prefs file location: `DEDUPE_PREFS` if set (non-empty), otherwise
/// under the user's home directory, otherwise the current directory.
pub fn default_prefs_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DEDUPE_PREFS")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".dedupe").join("gui-prefs.json");
    }
    PathBuf::from("gui-prefs.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dedupe-prefs-{tag}-{}.json", std::process::id()))
    }

    #[test]
    fn roundtrip_preserves_all_fields() {
        let path = tmp_path("rt");
        let _ = fs::remove_file(&path);
        let prefs = GuiPrefs {
            folders: vec![
                FolderPref {
                    path: "C:\\media".to_string(),
                    reference: true,
                },
                FolderPref {
                    path: "D:\\incoming".to_string(),
                    reference: false,
                },
            ],
            types: "jpg,png".to_string(),
            min_size: "100KB".to_string(),
            hash_index: 1,
            keep_index: 2,
            find_index: 4,
            match_index: 3,
            image_hash_index: 2,
            partial_clips: true,
            partial_similarity: "80".to_string(),
            fuzzy_threshold: "90".to_string(),
            top: "25".to_string(),
            ignore: "*.tmp".to_string(),
            music_by_content: true,
            exact: true,
            trash: false,
            sort_biggest: true,
            remember_folders: false,
            ..GuiPrefs::default()
        };
        prefs.save_to(path.clone()).unwrap();

        let loaded = GuiPrefs::load_from(path.clone());
        assert_eq!(loaded, prefs);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn legacy_prefs_without_remember_flag_keep_remembering() {
        // Files written before the flag existed have no field: serde
        // defaults it to true so upgrades keep their folders.
        let path = tmp_path("legacy");
        fs::write(
            &path,
            serde_json::json!({"version": PREFS_VERSION, "types": "mp4"}).to_string(),
        )
        .unwrap();
        let loaded = GuiPrefs::load_from(path.clone());
        assert!(loaded.remember_folders);
        assert_eq!(loaded.types, "mp4");
        assert!(loaded.folders.is_empty());
        assert_eq!(loaded.find_index, 0);
        assert_eq!(loaded.match_index, 0);
        assert_eq!(loaded.image_hash_index, 0);
        assert!(!loaded.partial_clips);
        assert_eq!(loaded.partial_similarity, "85");
        assert_eq!(loaded.fuzzy_threshold, "85");
        assert_eq!(loaded.top, "50");
        assert!(loaded.ignore.is_empty());
        assert!(!loaded.music_by_content);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_or_corrupt_file_yields_defaults() {
        let path = tmp_path("missing");
        let _ = fs::remove_file(&path);
        assert_eq!(GuiPrefs::load_from(path.clone()), GuiPrefs::default());

        fs::write(&path, b"not json").unwrap();
        assert_eq!(GuiPrefs::load_from(path.clone()), GuiPrefs::default());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn version_mismatch_yields_defaults() {
        let path = tmp_path("version");
        let prefs = GuiPrefs {
            version: PREFS_VERSION + 1,
            ..GuiPrefs::default()
        };
        fs::write(
            &path,
            serde_json::to_vec(&prefs).expect("serialize test prefs"),
        )
        .unwrap();
        assert_eq!(GuiPrefs::load_from(path.clone()), GuiPrefs::default());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn minimal_json_falls_back_to_field_defaults() {
        // `{}` exercises every `#[serde(default)]` fn: version, similarity,
        // trash, and both size labels.
        let path = tmp_path("minimal");
        fs::write(&path, b"{}").unwrap();
        let loaded = GuiPrefs::load_from(path.clone());
        // `{}` has no version (defaults to 0) → version mismatch → defaults.
        assert_eq!(loaded, GuiPrefs::default());
        assert_eq!(loaded.min_size, NO_LIMIT_LABEL);
        assert!(loaded.trash);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_and_load_use_the_env_override_path() {
        // The env var is process-global: hold the shared lock so parallel
        // tests (notably the UI tests in ui_tests) cannot race on it.
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = tmp_path("env");
        let _ = fs::remove_file(&path);
        unsafe {
            std::env::set_var("DEDUPE_PREFS", &path);
        }
        assert_eq!(default_prefs_path(), path);
        let prefs = GuiPrefs {
            types: "mp4".to_string(),
            ..GuiPrefs::default()
        };
        prefs.save().unwrap();
        assert_eq!(GuiPrefs::load(), prefs);
        unsafe {
            std::env::remove_var("DEDUPE_PREFS");
        }
        let _ = fs::remove_file(&path);
    }
}
