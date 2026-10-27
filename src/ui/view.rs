//! View state for the GUI: everything a scan or render pass reads and writes.

use crate::{cli, report};
use gpui_kit::component::input::InputState;
use gpui_kit::component::slider::SliderState;
use gpui_kit::{Entity, Subscription};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Slider bounds for the size filters (log scale): 1 KB … 1 TB.
pub(crate) const SIZE_SLIDER_MIN: f32 = 1024.0;
pub(crate) const SIZE_SLIDER_MAX: f32 = 1_000_000_000_000.0;

/// Image formats the UI can thumbnail directly from disk.
pub(crate) const THUMB_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "bmp", "webp"];

/// Groups rendered before a "Show more" button takes over; keeps huge
/// result sets fast and navigable.
pub(crate) const GROUP_PAGE: usize = 100;

pub(crate) struct DedupeView {
    pub(crate) view: AppView,
    pub(crate) settings_tab: SettingsTab,
    pub(crate) folder_input: Entity<InputState>,
    pub(crate) folders: Vec<FolderEntry>,
    pub(crate) types_input: Entity<InputState>,
    pub(crate) min_size_input: Entity<InputState>,
    pub(crate) max_size_input: Entity<InputState>,
    pub(crate) max_depth_input: Entity<InputState>,
    pub(crate) exclude_dir_input: Entity<InputState>,
    pub(crate) exclude_path_input: Entity<InputState>,
    pub(crate) similarity_input: Entity<InputState>,
    pub(crate) partial_similarity_input: Entity<InputState>,
    pub(crate) fuzzy_threshold_input: Entity<InputState>,
    pub(crate) top_input: Entity<InputState>,
    pub(crate) ignore_input: Entity<InputState>,
    pub(crate) jobs_input: Entity<InputState>,
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
    pub(crate) last_stats: Option<report::ScanStats>,
    pub(crate) last_paths: Vec<String>,
    pub(crate) last_hash_algo: String,
    pub(crate) last_ffprobe: bool,
    pub(crate) last_ref_dirs: Vec<PathBuf>,
    pub(crate) min_size_slider: Entity<SliderState>,
    pub(crate) max_size_slider: Entity<SliderState>,
    pub(crate) ffmpeg_ok: bool,
    pub(crate) ffprobe_ok: bool,
    pub(crate) media_checked: bool,
    pub(crate) posters: HashMap<PathBuf, PathBuf>,
    /// Inline video preview strips (frame PNGs) by source path. Filled
    /// lazily when a video card first renders; cached on disk too.
    pub(crate) strips: HashMap<PathBuf, Vec<PathBuf>>,
    /// Videos with a strip extraction currently in flight (dedupes spawns).
    pub(crate) preview_loading: HashSet<PathBuf>,
    pub(crate) dup_files: u64,
    pub(crate) reclaim_bytes: u64,
    pub(crate) scanning: bool,
    pub(crate) scan_done: u64,
    pub(crate) scan_total: u64,
    pub(crate) scan_determinate: bool,
    pub(crate) status: String,
    /// Toast text queued by a worker or dialog; delivered on the next frame
    /// because those callbacks have no `Window` to push it to directly.
    pub(crate) pending_toast: Option<(String, bool)>,
    pub(crate) files_scanned: usize,
    pub(crate) bytes_scanned: u64,
    pub(crate) has_scanned: bool,
    pub(crate) ffprobe_note: bool,
    pub(crate) groups: Vec<crate::matching::Group>,
    /// Non-duplicate cleanup scan output (`--find` modes). Mutually
    /// exclusive with `groups`: a mode scan never fills the duplicate list.
    pub(crate) mode_result: Option<crate::modes::ModeResult>,
    /// Pristine scan results: `groups` is a live view over this,
    /// re-derived whenever the similarity floor changes. Keep decisions
    /// are mirrored back here so tightening never loses keeper choices.
    pub(crate) base_groups: Vec<crate::matching::Group>,
    /// Similarity floor the scan ran with (fraction 0..=1).
    pub(crate) scan_similarity: f64,
    /// Live similarity floor (fraction); always >= `scan_similarity`.
    /// Lowering below the scan floor needs discarded pairs — rescan.
    pub(crate) min_sim: f64,
    pub(crate) min_sim_input: Entity<InputState>,
    /// How many groups render at once; the rest sit behind a "Show more"
    /// button so huge result sets stay navigable (and fast).
    pub(crate) group_limit: usize,
    pub(crate) last_hash: cli::HashAlgo,
    pub(crate) expanded: HashSet<usize>,
    pub(crate) selected: HashSet<PathBuf>,
    /// Previous keeper at the last single-click pick (group, path): a
    /// double-click reverts that flip so "just looking" never moves KEEP.
    pub(crate) last_pick: Option<(usize, PathBuf)>,
    /// Original paths from the last trash run: enables one-step undo.
    /// Cleared on permanent deletes and after a successful restore.
    pub(crate) last_trashed: Vec<PathBuf>,
    /// Armed consolidate destination (two-click flow): the first click
    /// picks the folder and previews the moves in the status line, the
    /// second click moves. Disarmed by any keeper/group change, since the
    /// previewed paths would be stale.
    pub(crate) consolidate_dest: Option<PathBuf>,
    /// Cooperative cancellation for the running scan; Stop sets it and the
    /// pipeline polls it at phase boundaries.
    pub(crate) scan_cancel: crate::cancel::CancelToken,
    /// Scan generation: bumped by every start and by Stop, so a scan task
    /// finishing late (after a Stop or a newer scan) discards its output.
    pub(crate) scan_gen: u64,
    pub(crate) _subs: Vec<Subscription>,
}

/// One folder to scan, with an optional protection flag (Krokiet's Ref
/// checkbox pattern: protected folders are never deleted from).
#[derive(Debug, Clone)]
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
    pub(crate) stats: report::ScanStats,
    pub(crate) groups: Vec<crate::matching::Group>,
}
