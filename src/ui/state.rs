//! Application state for the GUI, shared by every component.
//!
//! Native Shell components are plain functions that re-run on demand, so the
//! view state lives in one [`AppState`] signal instead of a graph of entities:
//! components read it with [`app`] and write it with
//! `app().with(|s| ...)`. Every write re-renders the components that read it.
//!
//! Text fields are plain `String`s here; the widgets keep their own caret and
//! selection state, so the value is all the view model has to own.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use nsh_core::prelude::*;

use crate::cancel::CancelToken;
use crate::cli::HashAlgo;
use crate::matching::Group;
use crate::modes::ModeResult;
use crate::prefs::GuiPrefs;
use crate::report::ScanStats;
use crate::ui_helpers::NO_LIMIT_LABEL;

/// Groups rendered before a "Show more" button takes over; keeps huge
/// result sets fast and navigable.
pub(crate) const GROUP_PAGE: usize = 100;

/// One folder to scan, with an optional protection flag (Krokiet's Ref
/// checkbox pattern: protected folders are never deleted from).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderEntry {
    pub(crate) path: String,
    pub(crate) reference: bool,
}

/// Top-level view: main scan/results flow vs the full Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AppView {
    #[default]
    Main,
    Settings,
}

/// How the duplicate groups are laid out: the original two-column preview
/// cards for media groups, or one compact row list for everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Layout {
    #[default]
    Grid,
    List,
}

/// Active tab inside the Settings page side nav.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SettingsTab {
    #[default]
    General,
    Preferences,
}

/// Owned report data carried into the async save task.
pub(crate) struct ReportSnapshot {
    pub(crate) paths: Vec<String>,
    pub(crate) hash_algo: String,
    pub(crate) ffprobe_used: bool,
    pub(crate) stats: ScanStats,
    pub(crate) groups: Vec<Group>,
}

/// Everything a scan or render pass reads and writes.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) view: AppView,
    pub(crate) settings_tab: SettingsTab,
    pub(crate) folder_input: String,
    pub(crate) folder_filter: Option<String>,
    pub(crate) folders: Vec<FolderEntry>,
    pub(crate) types_input: String,
    pub(crate) min_size_input: String,
    pub(crate) max_size_input: String,
    pub(crate) max_depth_input: String,
    pub(crate) exclude_dir_input: String,
    pub(crate) exclude_path_input: String,
    pub(crate) similarity_input: String,
    pub(crate) partial_similarity_input: String,
    pub(crate) fuzzy_threshold_input: String,
    pub(crate) top_input: String,
    pub(crate) ignore_input: String,
    pub(crate) jobs_input: String,
    pub(crate) min_sim_input: String,
    pub(crate) hash_index: Option<usize>,
    pub(crate) keep_index: Option<usize>,
    pub(crate) find_index: Option<usize>,
    pub(crate) match_index: Option<usize>,
    pub(crate) image_hash_index: Option<usize>,
    pub(crate) partial_clips: bool,
    pub(crate) music_by_content: bool,
    pub(crate) exact: bool,
    pub(crate) no_cache: bool,
    pub(crate) cache_msg: String,
    pub(crate) trash: bool,
    pub(crate) sort_biggest: bool,
    pub(crate) remember_folders: bool,
    pub(crate) last_stats: Option<ScanStats>,
    pub(crate) last_paths: Vec<String>,
    pub(crate) last_hash_algo: String,
    pub(crate) last_ffprobe: bool,
    pub(crate) last_ref_dirs: Vec<PathBuf>,
    pub(crate) ffmpeg_ok: bool,
    pub(crate) ffprobe_ok: bool,
    pub(crate) media_checked: bool,
    pub(crate) posters: HashMap<PathBuf, PathBuf>,
    /// Inline video preview strips (frame PNGs) by source path. Filled
    /// lazily when a video card first renders; cached on disk too.
    pub(crate) strips: HashMap<PathBuf, Vec<PathBuf>>,
    /// Videos whose poster extraction was requested this scan: in flight or
    /// already attempted, so a failure is not retried on every re-render.
    pub(crate) poster_jobs: HashSet<PathBuf>,
    pub(crate) dup_files: u64,
    pub(crate) reclaim_bytes: u64,
    pub(crate) scanning: bool,
    pub(crate) scan_done: u64,
    pub(crate) scan_total: u64,
    pub(crate) scan_determinate: bool,
    /// Current pipeline phase ("Hashing files…", "Comparing media…"), shown
    /// as the progress card's heading: a scan is two counting passes, so the
    /// bar legitimately restarts with a different total when the phase
    /// changes, and the heading is what makes that legible.
    pub(crate) scan_phase: String,
    pub(crate) status: String,
    pub(crate) files_scanned: usize,
    pub(crate) bytes_scanned: u64,
    pub(crate) has_scanned: bool,
    pub(crate) ffprobe_note: bool,
    pub(crate) groups: Vec<Group>,
    /// Bumped whenever the shape of `groups` changes (a scan, a threshold, a
    /// delete, a consolidate, a re-sort). The results list keys its rebuilds
    /// on it, so keeper picks and ticks — which change no structure — never
    /// rebuild or reorder the list.
    pub(crate) results_rev: u64,
    /// Non-duplicate cleanup scan output (`--find` modes). Mutually
    /// exclusive with `groups`: a mode scan never fills the duplicate list.
    pub(crate) mode_result: Option<ModeResult>,
    /// Pristine scan results: `groups` is a live view over this,
    /// re-derived whenever the similarity floor changes. Keep decisions
    /// are mirrored back here so tightening never loses keeper choices.
    pub(crate) base_groups: Vec<Group>,
    /// Similarity floor the scan ran with (fraction 0..=1).
    pub(crate) scan_similarity: f64,
    /// Live similarity floor (fraction); always >= `scan_similarity`.
    /// Lowering below the scan floor needs discarded pairs — rescan.
    pub(crate) min_sim: f64,
    /// How many groups render at once; the rest sit behind a "Show more"
    /// button so huge result sets stay navigable (and fast).
    pub(crate) group_limit: usize,
    pub(crate) last_hash: HashAlgo,
    /// Groups the user closed; every other group is open. Storing the closed
    /// ones instead of the open ones is what makes expansion the default: a
    /// scan, a re-threshold, a delete and "Show more" all start fully open
    /// without having to list every group.
    pub(crate) collapsed: HashSet<usize>,
    /// The video playing in its card's preview, if any. One at a time: media
    /// engines and audio are heavy, and a click elsewhere stops this one.
    pub(crate) playing: Option<PathBuf>,
    /// Results layout picked in the toolbar.
    pub(crate) layout: Layout,
    pub(crate) selected: HashSet<PathBuf>,
    /// Original paths from the last trash run: enables one-step undo.
    /// Cleared on permanent deletes and after a successful restore.
    pub(crate) last_trashed: Vec<PathBuf>,
    /// Armed consolidate destination (two-click flow): the first click
    /// picks the folder and previews the moves in the status line, the
    /// second click moves. Disarmed by any keeper/group change, since the
    /// previewed paths would be stale.
    pub(crate) consolidate_dest: Option<PathBuf>,
    /// The delete confirmation dialog is open: the ticked duplicates are
    /// shown as a dry run and nothing is removed until it is confirmed.
    pub(crate) confirm_delete: bool,
    /// Cooperative cancellation for the running scan; Stop sets it and the
    /// pipeline polls it at phase boundaries.
    pub(crate) scan_cancel: CancelToken,
    /// Scan generation: bumped by every start and by Stop, so a scan task
    /// finishing late (after a Stop or a newer scan) discards its output.
    pub(crate) scan_gen: u64,
}

impl Default for AppState {
    fn default() -> Self {
        let similarity = crate::similar::DEFAULT_SIMILARITY_PCT / 100.0;
        Self {
            view: AppView::Main,
            settings_tab: SettingsTab::General,
            folder_input: String::new(),
            folder_filter: None,
            folders: Vec::new(),
            types_input: String::new(),
            min_size_input: NO_LIMIT_LABEL.to_string(),
            max_size_input: NO_LIMIT_LABEL.to_string(),
            max_depth_input: String::new(),
            exclude_dir_input: String::new(),
            exclude_path_input: String::new(),
            similarity_input: "97".to_string(),
            partial_similarity_input: "85".to_string(),
            fuzzy_threshold_input: "85".to_string(),
            top_input: "50".to_string(),
            ignore_input: String::new(),
            jobs_input: String::new(),
            min_sim_input: String::new(),
            hash_index: Some(0),
            keep_index: Some(0),
            find_index: Some(0),
            match_index: Some(0),
            image_hash_index: Some(0),
            partial_clips: false,
            music_by_content: false,
            exact: false,
            no_cache: false,
            cache_msg: String::new(),
            trash: true,
            sort_biggest: false,
            remember_folders: true,
            last_stats: None,
            last_paths: Vec::new(),
            last_hash_algo: "blake3".to_string(),
            last_ffprobe: false,
            last_ref_dirs: Vec::new(),
            ffmpeg_ok: false,
            ffprobe_ok: false,
            media_checked: false,
            posters: HashMap::new(),
            strips: HashMap::new(),
            poster_jobs: HashSet::new(),
            dup_files: 0,
            reclaim_bytes: 0,
            scanning: false,
            scan_done: 0,
            scan_total: 0,
            scan_determinate: false,
            scan_phase: crate::pipeline::phase_label(crate::pipeline::ScanPhase::Scanning)
                .to_string(),
            status: "Choose a folder and press Scan.".to_string(),
            files_scanned: 0,
            bytes_scanned: 0,
            has_scanned: false,
            ffprobe_note: false,
            groups: Vec::new(),
            results_rev: 0,
            mode_result: None,
            base_groups: Vec::new(),
            scan_similarity: similarity,
            min_sim: similarity,
            group_limit: GROUP_PAGE,
            last_hash: HashAlgo::Blake3,
            collapsed: HashSet::new(),
            playing: None,
            layout: Layout::Grid,
            selected: HashSet::new(),
            last_trashed: Vec::new(),
            consolidate_dest: None,
            confirm_delete: false,
            scan_cancel: CancelToken::new(),
            scan_gen: 0,
        }
    }
}

impl AppState {
    /// Restore the sidebar selections from persisted preferences.
    ///
    /// The remember toggle gates restore: off starts every launch with an
    /// empty folder list (the session list itself is untouched).
    pub(crate) fn from_prefs(saved: &GuiPrefs) -> Self {
        let mut state = Self {
            folders: if saved.remember_folders {
                saved
                    .folders
                    .iter()
                    .map(|f| FolderEntry {
                        path: f.path.clone(),
                        reference: f.reference,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            ..Self::default()
        };
        state.apply_prefs(saved);
        state
    }

    /// Copy the persisted preferences onto this state.
    ///
    /// Only settings are touched: the live folder list, results, selection,
    /// scan progress and status are session data, so the Settings page can
    /// reset preferences in place without discarding what is on screen.
    pub(crate) fn apply_prefs(&mut self, saved: &GuiPrefs) {
        self.types_input = saved.types.clone();
        // Legacy prefs stored a blank box for "no limit"; display the
        // explicit `unlimited` label instead so the box never looks broken.
        self.min_size_input = if saved.min_size.trim().is_empty() {
            NO_LIMIT_LABEL.to_string()
        } else {
            saved.min_size.clone()
        };
        self.max_size_input = if saved.max_size.trim().is_empty() {
            NO_LIMIT_LABEL.to_string()
        } else {
            saved.max_size.clone()
        };
        self.max_depth_input = saved.max_depth.clone();
        self.exclude_dir_input = saved.exclude_dir.clone();
        self.exclude_path_input = saved.exclude_path.clone();
        self.similarity_input = saved.similarity.clone();
        self.partial_similarity_input = saved.partial_similarity.clone();
        self.fuzzy_threshold_input = saved.fuzzy_threshold.clone();
        self.top_input = saved.top.clone();
        self.ignore_input = saved.ignore.clone();
        self.jobs_input = saved.jobs.clone();
        self.hash_index = Some(saved.hash_index.min(2));
        self.keep_index = Some(saved.keep_index.min(4));
        self.find_index = Some(saved.find_index.min(10));
        self.match_index = Some(saved.match_index.min(3));
        self.image_hash_index = Some(saved.image_hash_index.min(4));
        self.partial_clips = saved.partial_clips;
        self.music_by_content = saved.music_by_content;
        self.exact = saved.exact;
        self.no_cache = saved.no_cache;
        self.trash = saved.trash;
        self.sort_biggest = saved.sort_biggest;
        self.remember_folders = saved.remember_folders;
    }
}

/// The one shared state signal. Created on first use from [`AppState::default`].
pub(crate) fn app() -> Signal<AppState> {
    global::<AppState>()
}
