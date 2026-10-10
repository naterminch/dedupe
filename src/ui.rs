//! Desktop GUI for dedupe, built on gpui-kit.
//!
//! Opens when the binary runs without a scan path (or with `--gui`). The GUI
//! drives the same [`crate::pipeline::run_scan`] as the CLI, so results are
//! identical; only the presentation differs.
//!
//! Layout: a side nav (folders + per-run scope + Scan), a main results
//! pane with one collapsible section per duplicate group, a Settings page
//! for global defaults (matching + system), and a status bar. Pure helpers
//! live in [`crate::ui_helpers`]; group/member rows live in
//! [`crate::ui_results`].
//!
//! Native-feel boundaries (do not reimplement inside the view layer):
//! folder picking and report saving go through native `rfd` dialogs off the
//! GPUI thread, deletions honor the OS trash, and theme mode persists to
//! `gui-prefs.json` and is restored before first paint.

use crate::ui_helpers::{
    NO_LIMIT_LABEL, bar_pos, fit_text, hash_caption, is_no_limit, parse_size_limit,
    prefs_size_to_slider, size_slider_to_text, size_text_to_slider,
};
use crate::ui_results::{SnapshotGroup, is_grid_preview, render_media_card, render_member};
use crate::{actions, cli, hashing, media, pipeline, poster, prefs, report, util};
use gpui_kit::base::{Disableable as _, Selectable as _, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderScale, SliderState};
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::component::{IconName, Theme, ThemeMode};
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, FontWeight, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString, Size, Styled as _,
    Subscription, TitlebarOptions, Window, WindowOptions, div, px,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Slider bounds for the size filters (log scale): 1 KB … 1 TB.
pub(crate) const SIZE_SLIDER_MIN: f32 = 1024.0;
pub(crate) const SIZE_SLIDER_MAX: f32 = 1_000_000_000_000.0;

/// Image formats the UI can thumbnail directly from disk.
pub(crate) const THUMB_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "bmp", "webp"];

/// Groups rendered before a "Show more" button takes over; keeps huge
/// result sets fast and navigable.
pub(crate) const GROUP_PAGE: usize = 100;

/// Boot the GUI application. Returns when the window is closed.
pub fn run() {
    // Instant is Copy; both framework callbacks are 'static, so each gets
    // its own copy of the start time.
    let t_start = std::time::Instant::now();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            eprintln!("[startup] application ready in {:?}", t_start.elapsed());
            gpui_kit::init(cx);
            eprintln!("[startup] kit init done in {:?}", t_start.elapsed());
            gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("dedupe".into()),
                        ..Default::default()
                    }),
                    window_min_size: Some(Size {
                        width: px(1120.),
                        height: px(640.),
                    }),
                    ..WindowOptions::default()
                },
                cx,
                move |window, cx| {
                    let view = cx.new(|cx| DedupeView::new(window, cx));
                    eprintln!("[startup] window + view built in {:?}", t_start.elapsed());
                    view
                },
            )
            .expect("failed to open dedupe window");
        });
}

pub(crate) struct DedupeView {
    pub(crate) view: AppView,
    pub(crate) folder_input: Entity<InputState>,
    pub(crate) folders: Vec<FolderEntry>,
    pub(crate) types_input: Entity<InputState>,
    pub(crate) min_size_input: Entity<InputState>,
    pub(crate) max_size_input: Entity<InputState>,
    pub(crate) max_depth_input: Entity<InputState>,
    pub(crate) exclude_dir_input: Entity<InputState>,
    pub(crate) exclude_path_input: Entity<InputState>,
    pub(crate) similarity_input: Entity<InputState>,
    pub(crate) jobs_input: Entity<InputState>,
    pub(crate) hash_index: Option<usize>,
    pub(crate) keep_index: Option<usize>,
    pub(crate) exact: bool,
    pub(crate) no_cache: bool,
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
    pub(crate) files_scanned: usize,
    pub(crate) bytes_scanned: u64,
    pub(crate) has_scanned: bool,
    pub(crate) ffprobe_note: bool,
    pub(crate) groups: Vec<crate::matching::Group>,
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

/// Owned report data carried into the async save task.
struct ReportSnapshot {
    paths: Vec<String>,
    hash_algo: String,
    ffprobe_used: bool,
    stats: report::ScanStats,
    groups: Vec<crate::matching::Group>,
}

impl DedupeView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = prefs::GuiPrefs::load();
        let mk = |placeholder: &str, default: &str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(default)
            })
        };
        let folder_input = mk("Type a folder, then Add (or Browse…)", "", window, cx);
        let types_input = mk("jpg,png,mp4 (empty = all)", &saved.types, window, cx);
        // Legacy prefs stored a blank box for "no limit"; display the
        // explicit `unlimited` label instead so the box never looks broken.
        let min_size_default = if saved.min_size.trim().is_empty() {
            NO_LIMIT_LABEL
        } else {
            saved.min_size.as_str()
        };
        let max_size_default = if saved.max_size.trim().is_empty() {
            NO_LIMIT_LABEL
        } else {
            saved.max_size.as_str()
        };
        let min_size_input = mk("100KB", min_size_default, window, cx);
        let max_size_input = mk("500MB", max_size_default, window, cx);
        let max_depth_input = mk("0 = top level only", &saved.max_depth, window, cx);
        let exclude_dir_input = mk("node_modules", &saved.exclude_dir, window, cx);
        let exclude_path_input = mk("substring", &saved.exclude_path, window, cx);
        let similarity_input = mk("0–100", &saved.similarity, window, cx);
        let jobs_input = mk("0", &saved.jobs, window, cx);
        // Live similarity floor for the current results (blank = scan
        // default). Lowering below the scan floor needs a rescan.
        let min_sim_input = mk("e.g. 98", "", window, cx);
        let mk_size_slider = |default: f32, cx: &mut Context<Self>| {
            // NOTE: max() must come before min(): the state clamps its
            // value on every builder call, and min(1024) with the default
            // max(100) panics.
            cx.new(|_| {
                SliderState::new()
                    .max(SIZE_SLIDER_MAX)
                    .min(SIZE_SLIDER_MIN)
                    .scale(SliderScale::Logarithmic)
                    .default_value(default)
            })
        };
        let min_size_slider = mk_size_slider(prefs_size_to_slider(&saved.min_size, true), cx);
        let max_size_slider = mk_size_slider(prefs_size_to_slider(&saved.max_size, false), cx);
        let mut view = Self {
            view: AppView::Main,
            folder_input,
            // The remember toggle gates restore: off starts every launch
            // with an empty folder list (session list itself is untouched).
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
            types_input,
            min_size_input,
            max_size_input,
            max_depth_input,
            exclude_dir_input,
            exclude_path_input,
            similarity_input,
            jobs_input,
            hash_index: Some(saved.hash_index.min(2)),
            keep_index: Some(saved.keep_index.min(3)),
            exact: saved.exact,
            no_cache: saved.no_cache,
            trash: saved.trash,
            sort_biggest: saved.sort_biggest,
            remember_folders: saved.remember_folders,
            last_stats: None,
            last_paths: Vec::new(),
            last_hash_algo: "blake3".to_string(),
            last_ffprobe: false,
            last_ref_dirs: Vec::new(),
            min_size_slider,
            max_size_slider,
            ffmpeg_ok: false,
            ffprobe_ok: false,
            media_checked: false,
            posters: HashMap::new(),
            strips: HashMap::new(),
            preview_loading: HashSet::new(),
            dup_files: 0,
            reclaim_bytes: 0,
            scanning: false,
            scan_done: 0,
            scan_total: 0,
            scan_determinate: false,
            status: "Choose a folder and press Scan.".to_string(),
            files_scanned: 0,
            bytes_scanned: 0,
            has_scanned: false,
            ffprobe_note: false,
            groups: Vec::new(),
            base_groups: Vec::new(),
            scan_similarity: crate::similar::DEFAULT_SIMILARITY_PCT / 100.0,
            min_sim: crate::similar::DEFAULT_SIMILARITY_PCT / 100.0,
            min_sim_input,
            group_limit: GROUP_PAGE,
            last_hash: cli::HashAlgo::Blake3,
            expanded: HashSet::new(),
            selected: HashSet::new(),
            last_pick: None,
            last_trashed: Vec::new(),
            _subs: Vec::new(),
        };
        // Slider — text box (one way; set_value emits no Change event, so
        // this cannot echo back).
        view._subs.push(cx.subscribe_in(
            &view.min_size_slider,
            window,
            |this, _, event, window, cx| {
                if let SliderEvent::Change(v) = event {
                    size_slider_to_text(v.start(), true, &this.min_size_input, window, cx);
                }
            },
        ));
        view._subs.push(cx.subscribe_in(
            &view.max_size_slider,
            window,
            |this, _, event, window, cx| {
                if let SliderEvent::Change(v) = event {
                    size_slider_to_text(v.start(), false, &this.max_size_input, window, cx);
                }
            },
        ));
        // Text box — slider.
        view._subs.push(cx.subscribe_in(
            &view.min_size_input,
            window,
            |this, state, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    size_text_to_slider(
                        &state.read(cx).value(),
                        true,
                        &this.min_size_slider,
                        window,
                        cx,
                    );
                }
            },
        ));
        view._subs.push(cx.subscribe_in(
            &view.max_size_input,
            window,
            |this, state, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    size_text_to_slider(
                        &state.read(cx).value(),
                        false,
                        &this.max_size_slider,
                        window,
                        cx,
                    );
                }
            },
        ));
        // Live similarity floor: every keystroke re-derives the groups from
        // the pristine scan results — no rescan, no I/O.
        view._subs.push(cx.subscribe_in(
            &view.min_sim_input,
            window,
            |this, state, event, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let raw = state.read(cx).value().trim().to_string();
                    this.apply_sim_input(&raw, cx);
                }
            },
        ));
        // Probe ffmpeg/ffprobe off the critical path: two process spawns
        // can cost hundreds of milliseconds and must not block first paint.
        cx.spawn(async move |this, cx: &mut gpui_kit::AsyncApp| {
            let (ffmpeg_ok, ffprobe_ok) = cx
                .background_executor()
                .spawn(async move { (media::ffmpeg_available(), media::ffprobe_available()) })
                .await;
            this.update(cx, |view: &mut Self, cx| {
                view.ffmpeg_ok = ffmpeg_ok;
                view.ffprobe_ok = ffprobe_ok;
                view.media_checked = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
        // Restore the saved theme before first paint to avoid a light-flash
        // when the user left the app in dark mode.
        if saved.dark {
            Theme::change(ThemeMode::Dark, None, cx);
        }
        // Checklist A4: initial focus lands in the folder input so the user
        // can type immediately after launch.
        let initial_focus = view.folder_input.read(cx).focus_handle(cx);
        window.focus(&initial_focus, cx);
        view
    }

    /// Read the sidebar form into CLI options. This is the GUI equivalent of
    /// clap parsing: every field maps 1:1 to a `dedupe` flag.
    fn collect_options(&self, cx: &mut Context<Self>) -> Result<cli::Cli, String> {
        let val = |e: &Entity<InputState>| e.read(cx).value().to_string();
        let split_multi = |s: &str| {
            s.split([',', ';', '\n'])
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };

        let paths: Vec<String> = self.folders.iter().map(|f| f.path.clone()).collect();
        if paths.is_empty() {
            return Err("Add a folder to scan first (type one and Add, or Browse…).".to_string());
        }
        let similarity_raw = val(&self.similarity_input).trim().to_string();
        let similarity: f64 = if similarity_raw.is_empty() {
            crate::similar::DEFAULT_SIMILARITY_PCT
        } else {
            similarity_raw
                .parse()
                .map_err(|_| "Similarity must be a number between 0 and 100.".to_string())?
        };
        if !(0.0..=100.0).contains(&similarity) {
            return Err(format!(
                "Similarity must be between 0 and 100 (got {similarity})."
            ));
        }
        let jobs_raw = val(&self.jobs_input).trim().to_string();
        let jobs: usize = if jobs_raw.is_empty() {
            0
        } else {
            jobs_raw
                .parse()
                .map_err(|_| "Worker threads must be a whole number (0 = auto).".to_string())?
        };
        let max_depth_raw = val(&self.max_depth_input).trim().to_string();
        let max_depth: Option<usize> = if max_depth_raw.is_empty() {
            None
        } else {
            Some(
                max_depth_raw
                    .parse()
                    .map_err(|_| "Max depth must be a whole number (e.g. 0, 2).".to_string())?,
            )
        };
        let opt = |e: &Entity<InputState>| {
            let v = val(e).trim().to_string();
            if is_no_limit(&v) { None } else { Some(v) }
        };
        // Validate sizes now so typos surface before the (slow) scan starts.
        // Blank or `unlimited` means unbounded.
        for e in [&self.min_size_input, &self.max_size_input] {
            let raw = val(e);
            if let Err(err) = parse_size_limit(&raw) {
                return Err(format!("Bad size “{raw}”: {err:#}"));
            }
        }
        Ok(cli::Cli {
            paths,
            max_depth,
            types: split_multi(&val(&self.types_input)),
            keep_smaller: self.keep_index.unwrap_or(0) == 1,
            keep_newest: self.keep_index.unwrap_or(0) == 2,
            keep_oldest: self.keep_index.unwrap_or(0) == 3,
            reference_dir: self
                .folders
                .iter()
                .filter(|f| f.reference)
                .map(|f| f.path.clone())
                .collect(),
            trash: self.trash,
            dry_run: false,
            delete: false,
            yes: false,
            hash: match self.hash_index.unwrap_or(0) {
                1 => cli::HashAlgo::Sha256,
                2 => cli::HashAlgo::Md5,
                _ => cli::HashAlgo::Blake3,
            },
            exact: self.exact,
            similarity,
            no_cache: self.no_cache,
            min_size: opt(&self.min_size_input),
            max_size: opt(&self.max_size_input),
            exclude_dir: split_multi(&val(&self.exclude_dir_input)),
            exclude_path: split_multi(&val(&self.exclude_path_input)),
            json: false,
            verbose: false,
            quiet: true,
            jobs,
            gui: false,
        })
    }

    /// Snapshot the current sidebar into `gui-prefs.json`. Best-effort:
    /// failures are ignored so a read-only home dir never breaks the GUI.
    /// Text fields are saved here (at scan time) plus on every discrete
    /// toggle, which avoids a disk write per keystroke.
    fn save_prefs(&self, cx: &mut Context<Self>) {
        let val = |e: &Entity<InputState>| e.read(cx).value().to_string();
        let out = prefs::GuiPrefs {
            version: 1,
            // Off means "don't keep": persist an empty list so stale
            // folders never resurface; the live session list is untouched.
            folders: if self.remember_folders {
                self.folders
                    .iter()
                    .map(|f| prefs::FolderPref {
                        path: f.path.clone(),
                        reference: f.reference,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            types: val(&self.types_input),
            min_size: val(&self.min_size_input),
            max_size: val(&self.max_size_input),
            max_depth: val(&self.max_depth_input),
            exclude_dir: val(&self.exclude_dir_input),
            exclude_path: val(&self.exclude_path_input),
            similarity: val(&self.similarity_input),
            jobs: val(&self.jobs_input),
            hash_index: self.hash_index.unwrap_or(0),
            keep_index: self.keep_index.unwrap_or(0),
            exact: self.exact,
            no_cache: self.no_cache,
            trash: self.trash,
            sort_biggest: self.sort_biggest,
            remember_folders: self.remember_folders,
            dark: cx.theme().is_dark(),
        };
        let _ = out.save();
    }

    fn start_scan(&mut self, cx: &mut Context<Self>) {
        self.save_prefs(cx);
        let opts = match self.collect_options(cx) {
            Ok(o) => o,
            Err(msg) => {
                self.status = msg;
                cx.notify();
                return;
            }
        };
        self.last_hash = opts.hash;
        self.scan_similarity = (opts.similarity / 100.0).clamp(0.0, 1.0);
        self.scanning = true;
        self.scan_done = 0;
        self.scan_total = 0;
        self.scan_determinate = false;
        self.has_scanned = false;
        self.groups.clear();
        self.selected.clear();
        self.expanded.clear();
        self.status = pipeline::phase_label(pipeline::ScanPhase::Scanning).to_string();
        cx.notify();

        // Progress crosses threads through shared state; a timer below
        // polls it into the view (GPUI views may only update on the main
        // thread). The bars are headless indicatif meters — polled for
        // position/length, never drawn — so the pipeline needs no
        // GUI-specific progress plumbing.
        let phase: Arc<Mutex<pipeline::ScanPhase>> =
            Arc::new(Mutex::new(pipeline::ScanPhase::Scanning));
        let phase_write = phase.clone();
        let phase_read = phase.clone();
        let hash_bar = indicatif::ProgressBar::hidden();
        let sim_bar = indicatif::ProgressBar::hidden();
        let hash_bar_poll = hash_bar.clone();
        let sim_bar_poll = sim_bar.clone();
        cx.spawn(async move |this, cx: &mut gpui_kit::AsyncApp| {
            let out = cx
                .background_executor()
                .spawn(async move {
                    pipeline::run_scan(&opts, Some(&hash_bar), Some(&sim_bar), &|p| {
                        if let Ok(mut s) = phase_write.lock() {
                            *s = p;
                        }
                    })
                })
                .await;
            this.update(cx, |view: &mut Self, cx| {
                match out {
                    Ok(output) => view.apply_output(output, cx),
                    Err(e) => {
                        view.scanning = false;
                        view.status = format!("Scan failed: {e:#}");
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.spawn(async move |this, cx: &mut gpui_kit::AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let alive = this
                    .update(cx, |view: &mut Self, cx| {
                        if view.scanning {
                            let ph = phase_read
                                .lock()
                                .map(|s| *s)
                                .unwrap_or(pipeline::ScanPhase::Scanning);
                            let (label, done, total, determinate) = match ph {
                                pipeline::ScanPhase::Scanning => {
                                    (pipeline::phase_label(ph).to_string(), 0, 0, false)
                                }
                                pipeline::ScanPhase::Hashing => {
                                    let (d, t) = bar_pos(&hash_bar_poll);
                                    (format!("Hashing {d} of {t}"), d, t, true)
                                }
                                pipeline::ScanPhase::ComparingMedia => {
                                    let (d, t) = bar_pos(&sim_bar_poll);
                                    (format!("Comparing media {d} of {t}"), d, t, true)
                                }
                                pipeline::ScanPhase::Finishing => {
                                    (pipeline::phase_label(ph).to_string(), 0, 0, false)
                                }
                            };
                            view.status = label;
                            view.scan_done = done;
                            view.scan_total = total;
                            view.scan_determinate = determinate && total > 0;
                            cx.notify();
                        }
                        view.scanning
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_output(&mut self, out: pipeline::ScanOutput, cx: &mut Context<Self>) {
        self.scanning = false;
        self.scan_done = 0;
        self.scan_total = 0;
        self.scan_determinate = false;
        self.has_scanned = true;
        self.files_scanned = out.stats.files_scanned;
        self.bytes_scanned = out.stats.bytes_scanned;
        self.ffprobe_note = out.needs_ffprobe_note;
        self.last_stats = Some(out.stats);
        self.last_paths = out.paths.clone();
        self.last_hash_algo = out.hash_algo.to_string();
        self.last_ffprobe = out.ffprobe_used;
        self.last_ref_dirs = out.reference_dirs;
        self.groups = out.groups;
        // Pristine copy for live re-thresholding; the floor starts at the
        // scan threshold (everything qualifies).
        self.base_groups = self.groups.clone();
        self.min_sim = self.scan_similarity;
        self.group_limit = GROUP_PAGE;
        // One display ordering for the whole scan (honors the sort toggle).
        self.sort_groups();
        // Pre-select every duplicate for deletion (KEEP members are never
        // selectable); the user unchecks what should survive.
        self.selected = self
            .groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| !m.keep)
            .map(|m| m.path.clone())
            .collect();
        // Expand the first few groups; huge result sets stay navigable.
        self.expanded = self.groups.iter().take(10).map(|g| g.index).collect();
        let dups: u64 = self
            .groups
            .iter()
            .map(|g| (g.members.len() - g.keep_count()) as u64)
            .sum();
        let reclaim: u64 = self.groups.iter().map(|g| g.dup_bytes()).sum();
        self.dup_files = dups;
        self.reclaim_bytes = reclaim;
        if self.groups.is_empty() {
            self.status = format!(
                "No duplicates — scanned {} file(s) ({}).",
                self.files_scanned,
                util::human_bytes(self.bytes_scanned)
            );
        } else {
            self.status = format!(
                "{} group(s) · {} duplicate file(s) · {} reclaimable.",
                self.groups.len(),
                dups,
                util::human_bytes(reclaim)
            );
        }
        for e in &out.cache_save_errors {
            self.status.push_str(&format!(" (cache write failed: {e})"));
        }
        // Generate video posters off the UI thread; each completion
        // re-renders its row's thumbnail.
        let jobs: Vec<(PathBuf, u64, Option<i64>)> = self
            .groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| media::classify(&m.path) == media::MediaKind::Video)
            .take(120)
            .map(|m| (m.path.clone(), m.size, m.mtime_secs))
            .collect();
        self.posters.clear();
        // Preview strips are per-scan too: paths are gone, so drop frames
        // and in-flight flags (their async tasks no-op on update when the
        // view is gone).
        self.strips.clear();
        self.preview_loading.clear();
        if !jobs.is_empty() {
            cx.spawn(async move |this, cx| {
                for (path, size, mtime) in jobs {
                    let done = cx
                        .background_executor()
                        .spawn(async move {
                            poster::video_poster(&path, size, mtime).map(|p| (path, p))
                        })
                        .await;
                    let alive = this
                        .update(cx, |view: &mut Self, cx| {
                            if let Some((orig, thumb)) = done {
                                view.posters.insert(orig, thumb);
                                cx.notify();
                            }
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            })
            .detach();
        }
    }

    /// Add folders to the scan list (deduplicated, in order).
    fn add_folders(&mut self, raw: &str, cx: &mut Context<Self>) {
        for p in raw
            .split([';', '\n'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            if !self.folders.iter().any(|f| f.path == p) {
                self.folders.push(FolderEntry {
                    path: p.to_string(),
                    reference: false,
                });
            }
        }
        self.save_prefs(cx);
        cx.notify();
    }

    fn browse(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .folders
            .last()
            .map(|f| f.path.clone())
            .unwrap_or_default();
        // Async dialog on rfd's own thread: a blocking native dialog must
        // never run inside a GPUI event handler — its modal message loop
        // re-enters GPUI dispatch and aborts on the borrowed app context.
        cx.spawn(async move |this, _cx| {
            let mut dlg = rfd::AsyncFileDialog::new().set_title("Choose a folder to scan");
            if !start.is_empty() && std::path::Path::new(&start).exists() {
                dlg = dlg.set_directory(&start);
            }
            let picked = dlg.pick_folder().await;
            if let Some(handle) = picked {
                let path = handle.path().display().to_string();
                this.update(_cx, |view, cx| {
                    view.add_folders(&path, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// Mark every non-keep member for deletion (fresh intent after a
    /// keeper/mode change).
    fn reselect_all_dups(&mut self) {
        self.selected = self.all_dup_paths();
    }

    /// Promote one member to keeper of its group (per-group decision).
    /// Member order never changes here: the grid keeps every card in place
    /// and only the highlight (KEEP/DUP badge + border) moves.
    pub(crate) fn set_keeper(
        &mut self,
        group_index: usize,
        path: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.index == group_index) {
            for m in g.members.iter_mut() {
                m.keep = m.path == path;
            }
        }
        self.reselect_all_dups();
        self.sync_keep_to_base();
        // Keeper swap changes which bytes count as reclaimable.
        self.recount();
        cx.notify();
    }

    /// Apply a new global keep mode to the current results (no rescan).
    fn apply_keep_mode(&mut self, index: usize, cx: &mut Context<Self>) {
        self.keep_index = Some(index);
        let keep = match index {
            1 => crate::matching::KeepMode::Smallest,
            2 => crate::matching::KeepMode::Newest,
            3 => crate::matching::KeepMode::Oldest,
            _ => crate::matching::KeepMode::First,
        };
        crate::matching::reassign_keepers(&mut self.groups, keep);
        self.reselect_all_dups();
        let name = ["First found", "Smallest", "Newest", "Oldest"][index.min(3)];
        self.status = format!(
            "Keep mode: {name} applied to {} group(s).",
            self.groups.len()
        );
        self.sync_keep_to_base();
        // New keepers everywhere: re-apply the display order once, here.
        self.sort_groups();
        self.save_prefs(cx);
        cx.notify();
    }

    /// Recompute the summary counters from the current groups.
    fn recount(&mut self) {
        let dups: u64 = self
            .groups
            .iter()
            .map(|g| (g.members.len() - g.keep_count()) as u64)
            .sum();
        self.dup_files = dups;
        self.reclaim_bytes = self.groups.iter().map(|g| g.dup_bytes()).sum();
    }

    /// Order `groups` for display: biggest-reclaimable-first while the sort
    /// toggle is on, otherwise the scan's discovery order (via base_groups).
    /// Stable sort, so ties never jitter.
    ///
    /// Called ONLY on explicit order-changing events (new scan, sort toggle,
    /// similarity filter, global keep mode) — never on keeper picks or
    /// renders, so clicking a card can't reshuffle the list underneath.
    fn sort_groups(&mut self) {
        if self.sort_biggest {
            self.groups
                .sort_by_key(|b| std::cmp::Reverse(b.dup_bytes()));
        } else {
            let pos: HashMap<usize, usize> = self
                .base_groups
                .iter()
                .enumerate()
                .map(|(i, g)| (g.index, i))
                .collect();
            self.groups
                .sort_by_key(|g| pos.get(&g.index).copied().unwrap_or(usize::MAX));
        }
    }

    /// Mirror keep flags and member order from the live view back into the
    /// pristine scan results, so re-thresholding never loses keeper choices.
    /// Members hidden by the floor keep their stored flags.
    fn sync_keep_to_base(&mut self) {
        for g in &self.groups {
            if let Some(base) = self.base_groups.iter_mut().find(|b| b.index == g.index) {
                for m in &g.members {
                    if let Some(bm) = base.members.iter_mut().find(|bm| bm.path == m.path) {
                        bm.keep = m.keep;
                    }
                }
                let order: HashMap<PathBuf, usize> = g
                    .members
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (m.path.clone(), i))
                    .collect();
                base.members
                    .sort_by_key(|m| order.get(&m.path).copied().unwrap_or(usize::MAX));
            }
        }
    }

    /// Re-derive the live groups from the pristine scan results at floor
    /// `t` (fraction). Exact groups always qualify; similar members need
    /// similarity-to-keeper >= `t`. Groups left with fewer than 2 members
    /// drop out (restorable by lowering the floor — no rescan).
    fn apply_threshold(&mut self, t: f64, cx: &mut Context<Self>) {
        let t = t.clamp(self.scan_similarity, 1.0);
        self.min_sim = t;
        self.groups = self
            .base_groups
            .iter()
            .filter_map(|g| {
                let members: Vec<_> = g
                    .members
                    .iter()
                    .filter(|m| m.similarity.is_none_or(|s| s >= t))
                    .cloned()
                    .collect();
                if members.len() >= 2 {
                    let mut g = g.clone();
                    g.members = members;
                    Some(g)
                } else {
                    None
                }
            })
            .collect();
        self.group_limit = GROUP_PAGE;
        self.reselect_all_dups();
        self.recount();
        // Fresh member set: re-apply the display order once, here.
        self.sort_groups();
        self.status = format!(
            "Similarity ≥ {:.0}% — {} group(s), {} duplicate file(s).",
            t * 100.0,
            self.groups.len(),
            self.dup_files,
        );
        cx.notify();
    }

    /// Parse the live floor box: blank restores the scan floor, a number
    /// tightens it, anything below the scan floor is clamped (loosening
    /// needs discarded pairs — rescan instead).
    fn apply_sim_input(&mut self, raw: &str, cx: &mut Context<Self>) {
        let raw = raw.trim();
        if raw.is_empty() {
            if (self.min_sim - self.scan_similarity).abs() > f64::EPSILON {
                let floor = self.scan_similarity;
                self.apply_threshold(floor, cx);
            }
            return;
        }
        match raw.parse::<f64>() {
            Ok(v) if (0.0..=100.0).contains(&v) => {
                let t = v / 100.0;
                if t < self.scan_similarity {
                    self.status = format!(
                        "Floor {:.0}% is below the scan threshold {:.0}% — rescan to loosen.",
                        v,
                        self.scan_similarity * 100.0
                    );
                    cx.notify();
                    return;
                }
                self.apply_threshold(t, cx);
            }
            _ => {}
        }
    }

    /// Files currently marked for deletion (selected checkboxes that are
    /// still non-keep members of a group).
    fn pending_targets(&self) -> Vec<actions::DeleteTarget> {
        let mut out = Vec::new();
        for g in &self.groups {
            for m in &g.members {
                if !m.keep && self.selected.contains(&m.path) {
                    out.push(actions::DeleteTarget {
                        path: m.path.clone(),
                        expected_hash: m.content_hash.clone().unwrap_or_else(|| g.hash.clone()),
                        size: m.size,
                    });
                }
            }
        }
        out
    }

    fn perform_delete(&mut self, cx: &mut Context<Self>) {
        let targets = self.pending_targets();
        if targets.is_empty() {
            self.status = "Nothing selected for deletion.".to_string();
            cx.notify();
            return;
        }
        let engine = hashing::HashEngine::new(self.last_hash);
        let disposition = if self.trash {
            actions::Disposition::Trash
        } else {
            actions::Disposition::Permanent
        };
        let outcome = actions::delete_targets(&targets, &engine, disposition);
        let gone: HashSet<PathBuf> = outcome.deleted.iter().cloned().collect();
        for g in &mut self.groups {
            g.members.retain(|m| !gone.contains(&m.path));
        }
        self.groups.retain(|g| g.members.len() >= 2);
        // Prune the pristine copy too so a later floor change cannot
        // resurrect deleted files.
        for g in &mut self.base_groups {
            g.members.retain(|m| !gone.contains(&m.path));
        }
        self.base_groups.retain(|g| g.members.len() >= 2);
        for p in &outcome.deleted {
            self.selected.remove(p);
        }
        let verb = if self.trash {
            "Moved to trash"
        } else {
            "Deleted"
        };
        // Trash receipts enable one-step undo; permanent deletes cannot
        // be undone, so any stale receipt is dropped.
        if self.trash {
            self.last_trashed = outcome.deleted.clone();
        } else {
            self.last_trashed.clear();
        }
        self.status = format!(
            "{verb} {} file(s) ({} freed); {} skipped.",
            outcome.deleted.len(),
            util::human_bytes(outcome.bytes_freed),
            outcome.skipped.len()
        );
        cx.notify();
    }

    /// Restore the last trash run (undo). Permanent deletes have no
    /// receipt and cannot be restored. Restored files reappear on disk;
    /// press Scan to refresh the groups.
    fn undo_delete(&mut self, cx: &mut Context<Self>) {
        if self.last_trashed.is_empty() {
            self.status = "Nothing to undo.".to_string();
            cx.notify();
            return;
        }
        // `trash::os_limited` (list/restore) only exists on Windows and
        // Freedesktop-trash Unix — there is no public macOS API for it, so
        // the module is cfg-gated out there. Mirror the crate's gate here.
        #[cfg(any(
            target_os = "windows",
            all(
                unix,
                not(target_os = "macos"),
                not(target_os = "ios"),
                not(target_os = "android")
            )
        ))]
        {
            let wanted: HashSet<PathBuf> = self.last_trashed.iter().cloned().collect();
            let items = match trash::os_limited::list() {
                Ok(items) => items,
                Err(e) => {
                    self.status = format!("Could not read the trash: {e}");
                    cx.notify();
                    return;
                }
            };
            let mine: Vec<_> = items
                .into_iter()
                .filter(|it| wanted.contains(&it.original_path()))
                .collect();
            if mine.is_empty() {
                self.status = "Nothing to restore — the trash was already emptied.".to_string();
                self.last_trashed.clear();
                cx.notify();
                return;
            }
            let n = mine.len();
            match trash::os_limited::restore_all(mine) {
                Ok(()) => {
                    self.status = format!("Restored {n} file(s). Press Scan to refresh.");
                    self.last_trashed.clear();
                }
                Err(e) => {
                    self.status = format!("Restore failed: {e}");
                }
            }
            cx.notify();
        }
        #[cfg(not(any(
            target_os = "windows",
            all(
                unix,
                not(target_os = "macos"),
                not(target_os = "ios"),
                not(target_os = "android")
            )
        )))]
        {
            self.last_trashed.clear();
            self.status =
                "Undo is not supported on this platform (trash restore needs Windows/Linux)."
                    .to_string();
            cx.notify();
        }
    }

    /// Included-folders list with per-row Ref toggles (Krokiet pattern).
    fn render_folder_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex().gap_1();
        if self.folders.is_empty() {
            list = list.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No folders yet — add one below."),
            );
        }
        for (i, folder) in self.folders.iter().enumerate() {
            list = list.child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .child(fit_text(&folder.path, 120)),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                Checkbox::new(SharedString::from(format!("ref-{i}")))
                                    .label("Ref")
                                    .checked(folder.reference)
                                    .on_change(cx.listener(move |this, value, _, cx| {
                                        if let Some(f) = this.folders.get_mut(i) {
                                            f.reference = *value;
                                            this.save_prefs(cx);
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("rm-folder-{i}")))
                                    .label("×")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if i < this.folders.len() {
                                            this.folders.remove(i);
                                            this.save_prefs(cx);
                                            cx.notify();
                                        }
                                    })),
                            ),
                    ),
            );
        }
        list
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Side nav: scan folders + per-run scope. Global tuning lives on the
        // Settings page — the main flow stays a 3-step scan.
        v_flex()
            .w(px(320.))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .gap_1()
                    .p_4()
                    .pb_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "SCAN FOLDERS ({}) — TICK REF TO PROTECT",
                                self.folders.len()
                            )),
                    )
                    .child(self.render_folder_list(cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.folder_input)))
                            .child(Button::new("add-folder").label("Add").on_click(cx.listener(
                                |this, _, window, cx| {
                                    let raw = this.folder_input.read(cx).value().trim().to_string();
                                    this.add_folders(&raw, cx);
                                    this.folder_input.update(cx, |input, cx| {
                                        input.set_value(
                                            SharedString::from(String::new()),
                                            window,
                                            cx,
                                        );
                                    });
                                },
                            )))
                            .child(
                                Button::new("browse")
                                    .icon(IconName::FolderOpen)
                                    .label("Browse…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.browse(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div().flex_1().w_full().overflow_y_scrollbar().child(
                    v_flex()
                        .gap_3()
                        .px_4()
                        .py_2()
                        .child(self.sidebar_section("SCOPE (THIS SCAN)", cx))
                        .child(self.sidebar_field(
                            "FILE TYPES (EMPTY = ALL)",
                            Input::new(&self.types_input),
                            cx,
                        ))
                        .child(self.render_size_row(
                            "MIN SIZE",
                            &self.min_size_slider,
                            &self.min_size_input,
                            cx,
                        ))
                        .child(self.render_size_row(
                            "MAX SIZE",
                            &self.max_size_slider,
                            &self.max_size_input,
                            cx,
                        ))
                        .child(self.sidebar_field(
                            "MAX DEPTH (EMPTY = UNLIMITED)",
                            Input::new(&self.max_depth_input),
                            cx,
                        ))
                        .child(self.sidebar_field(
                            "SKIP DIRECTORIES (; SEPARATED)",
                            Input::new(&self.exclude_dir_input),
                            cx,
                        ))
                        .child(self.sidebar_field(
                            "SKIP PATHS CONTAINING (; SEPARATED)",
                            Input::new(&self.exclude_path_input),
                            cx,
                        )),
                ),
            )
            .child(
                div()
                    .p_4()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex().gap_2().child(self.render_media_status(cx)).child(
                            Button::new("scan")
                                .primary()
                                .icon(IconName::Search)
                                .w_full()
                                .disabled(self.scanning)
                                .label(if self.scanning {
                                    "Scanning…"
                                } else {
                                    "Scan for duplicates"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_scan(cx);
                                })),
                        ),
                    ),
            )
    }

    /// Settings page: global defaults (matching + system). Per-run scope
    /// lives in the side nav. Everything auto-saves to `gui-prefs.json`.
    fn render_settings(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let section_title = |label: &'static str, cx: &mut Context<Self>| {
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(cx.theme().foreground)
                .child(label)
        };
        div()
            .flex_1()
            .h_full()
            .w_full()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .gap_4()
                    .p_6()
                    .max_w(px(640.))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("settings-back")
                                    .label("← Back")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.view = AppView::Main;
                                        cx.notify();
                                    })),
                            )
                            .child(section_title("Settings", cx)),
                    )
                    .child(
                        v_flex().gap_2().child(section_title("MATCHING", cx)).child(
                            v_flex()
                                .gap_3()
                                .child(self.sidebar_section("HASH ALGORITHM", cx))
                                .child(
                                    RadioGroup::new("hash")
                                        .children(["blake3", "sha256", "md5"])
                                        .selected_index(self.hash_index)
                                        .on_change(cx.listener(|this, value, _, cx| {
                                            this.hash_index = Some(*value);
                                            this.save_prefs(cx);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(hash_caption(self.hash_index)),
                                )
                                .child(self.sidebar_field(
                                    "SIMILARITY % FOR IMAGES / VIDEO",
                                    Input::new(&self.similarity_input),
                                    cx,
                                ))
                                .child(
                                    Checkbox::new("exact")
                                        .label("Exact duplicates only")
                                        .checked(self.exact)
                                        .on_change(cx.listener(|this, value, _, cx| {
                                            this.exact = *value;
                                            this.save_prefs(cx);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Checkbox::new("no-cache")
                                        .label("Skip caches (hash + fingerprint)")
                                        .checked(self.no_cache)
                                        .on_change(cx.listener(|this, value, _, cx| {
                                            this.no_cache = *value;
                                            this.save_prefs(cx);
                                            cx.notify();
                                        })),
                                ),
                        ),
                    )
                    .child(
                        v_flex().gap_2().child(section_title("SYSTEM", cx)).child(
                            v_flex()
                                .gap_3()
                                .child(self.sidebar_field(
                                    "WORKER THREADS (0 = AUTO)",
                                    Input::new(&self.jobs_input),
                                    cx,
                                ))
                                .child(
                                    Checkbox::new("trash")
                                        .label("Move to trash (recoverable)")
                                        .checked(self.trash)
                                        .on_change(cx.listener(|this, value, _, cx| {
                                            this.trash = *value;
                                            this.save_prefs(cx);
                                            cx.notify();
                                        })),
                                )
                                .child(self.sidebar_section("DEFAULT KEEP (NEW SCANS)", cx))
                                .child(
                                    RadioGroup::new("default-keep")
                                        .children(["First", "Smallest", "Newest", "Oldest"])
                                        .selected_index(self.keep_index)
                                        .on_change(cx.listener(|this, value, _, cx| {
                                            this.apply_keep_mode(*value, cx);
                                        })),
                                )
                                .child(
                                    Checkbox::new("default-sort-biggest")
                                        .label("Sort biggest reclaim first")
                                        .checked(self.sort_biggest)
                                        .on_change(cx.listener(|this, value, _, cx| {
                                            this.sort_biggest = *value;
                                            this.save_prefs(cx);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Checkbox::new("remember-folders")
                                        .label("Remember scan folders between runs")
                                        .checked(self.remember_folders)
                                        .on_change(cx.listener(|this, value, _, cx| {
                                            this.remember_folders = *value;
                                            this.save_prefs(cx);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            "Per-scan scope (folders, types, sizes) lives in the side nav. Theme lives in the top bar. All settings save automatically.",
                                        ),
                                ),
                        ),
                    ),
            )
    }

    /// ffmpeg/ffprobe availability line shown above the Scan button.
    fn render_media_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.media_checked {
            return h_flex()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Checking for ffmpeg…"),
                )
                .into_any_element();
        }
        let (icon, text, color) = match (self.ffmpeg_ok, self.ffprobe_ok) {
            (true, true) => (
                IconName::CircleCheck,
                "ffmpeg ready — video compare on",
                cx.theme().success,
            ),
            (false, true) => (
                IconName::TriangleAlert,
                "ffmpeg missing — video compare off",
                cx.theme().warning,
            ),
            (true, false) => (
                IconName::TriangleAlert,
                "ffprobe missing — media details off",
                cx.theme().warning,
            ),
            (false, false) => (
                IconName::TriangleAlert,
                "ffmpeg missing — video compare + details off",
                cx.theme().warning,
            ),
        };
        h_flex()
            .gap_1()
            .items_center()
            .child(icon)
            .child(div().text_xs().text_color(color).child(text))
            .into_any_element()
    }

    /// Size filter with a log-scale slider synced to an exact-value box.
    /// Drag for ballpark, type for exact; empty box means "no limit".
    fn render_size_row(
        &self,
        label: &'static str,
        slider: &Entity<SliderState>,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(self.sidebar_section(label, cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(Slider::new(slider).horizontal()))
                    .child(div().w(px(96.)).child(Input::new(input))),
            )
    }

    fn sidebar_section(&self, label: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(label)
    }

    fn sidebar_field(
        &self,
        label: &'static str,
        input: Input,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(self.sidebar_section(label, cx))
            .child(input)
    }

    /// First-run card: the sidebar holds a dozen options, so the empty
    /// results pane teaches the 3-step flow instead of sitting blank.
    fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let step = |n: &'static str, text: &'static str| {
            h_flex()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(cx.theme().accent)
                        .child(n),
                )
                .child(div().text_sm().child(text))
        };
        v_flex()
            .w_full()
            .gap_2()
            .p_4()
            .border_1()
            .rounded_md()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child("Find duplicate files in 3 steps"),
            )
            .child(step(
                "1.",
                "Add a folder on the left (type it and Add, or Browse…).",
            ))
            .child(step("2.", "Press “Scan for duplicates”."))
            .child(step(
                "3.",
                "Review each group, untick anything that must survive, then Delete selected.",
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Keepers are never deleted. Deletion goes to the trash by default."),
            )
    }

    fn render_results(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Two rows: expand/collapse + select-all on top, all action
        // controls on the second row right-aligned. The scan status lives
        // in the status bar (and the progress card while scanning), so the
        // header never repeats it. Flex items will not shrink below
        // content width here, so controls must never share a row with
        // free-length text — otherwise they clip off the edge.
        let header = v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(self.render_expand_toggle(cx))
                    .child(self.render_select_presets(cx)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .justify_end()
                    .child(self.render_sim_filter(cx))
                    .child(self.render_keep_segmented(cx))
                    .child(self.render_sort_toggle(cx))
                    .child(
                        Button::new("save-json")
                            .label("Save JSON")
                            .disabled(self.scanning || !self.has_scanned)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_report(window, cx);
                            })),
                    )
                    .child(self.render_delete_bar(cx)),
            );
        let mut list = v_flex().gap_2();
        if self.scanning {
            list = list.child(self.render_progress(cx));
        }
        if !self.has_scanned && !self.scanning {
            list = list.child(self.render_empty_state(cx));
        }
        if self.has_scanned && !self.groups.is_empty() {
            list = list.child(self.render_stat_chips(cx));
        }
        if self.ffprobe_note {
            list = list.child(div().text_xs().text_color(cx.theme().warning).child(
                "⚠ ffprobe not found on PATH — install FFmpeg for media resolution/duration.",
            ));
        }
        if self.has_scanned && self.groups.is_empty() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child("✔ No duplicate files found."),
            );
        }
        // Render in pages: thousands of groups would otherwise build
        // thousands of elements up front and stall the UI. Order comes
        // from sort_groups (scan / toggle / filter events only) — rendering
        // itself never re-sorts, so keeper picks can't move groups around.
        let total = self.groups.len();
        for gi in 0..total.min(self.group_limit) {
            list = list.child(self.render_group(gi, cx));
        }
        if total > self.group_limit {
            let remaining = total - self.group_limit;
            list = list.child(
                Button::new("show-more-groups")
                    .label(format!("Show more ({remaining} remaining)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.group_limit += GROUP_PAGE;
                        cx.notify();
                    })),
            );
        }
        v_flex()
            .flex_1()
            .h_full()
            .gap_3()
            .p_4()
            .child(header)
            .child(div().flex_1().w_full().overflow_y_scrollbar().child(list))
    }

    /// Live scan progress card: phase label with "N of M" counts plus a
    /// real progress bar for the determinate hashing/comparing stages.
    fn render_progress(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut card = v_flex()
            .w_full()
            .gap_2()
            .p_3()
            .border_1()
            .rounded_md()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(cx.theme().accent)
                            .child(self.status.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} file(s) scanned", self.files_scanned)),
                    ),
            );
        if self.scan_determinate {
            let pct = if self.scan_total > 0 {
                (self.scan_done as f32 / self.scan_total as f32 * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            card = card.child(Progress::new("scan-progress").value(pct).w_full());
        }
        card
    }

    /// Summary chips: files scanned, groups found, reclaimable bytes.
    fn render_stat_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chip = |value: String, label: &'static str, color: gpui_kit::Hsla| {
            v_flex()
                .gap_0()
                .px_3()
                .py_1()
                .border_1()
                .rounded_md()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(color)
                        .child(value),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
        };
        h_flex()
            .gap_2()
            .child(chip(
                self.files_scanned.to_string(),
                "FILES SCANNED",
                cx.theme().foreground,
            ))
            .child(chip(
                self.groups.len().to_string(),
                "GROUPS",
                cx.theme().accent,
            ))
            .child(chip(
                util::human_bytes(self.reclaim_bytes),
                "RECLAIMABLE",
                cx.theme().success,
            ))
    }

    /// Every non-keeper path: the universe bulk selection operates on.
    /// Keepers are never selectable.
    fn all_dup_paths(&self) -> HashSet<PathBuf> {
        self.groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| !m.keep)
            .map(|m| m.path.clone())
            .collect()
    }

    /// Bulk selection presets: All / None / Invert. Keepers stay untouched.
    fn render_select_presets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let off = self.scanning || self.groups.is_empty();
        h_flex()
            .gap_1()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("SELECT"),
            )
            .child(
                Button::new("sel-all")
                    .label("All")
                    .disabled(off)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected = this.all_dup_paths();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("sel-none")
                    .label("None")
                    .disabled(off || self.selected.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected.clear();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("sel-invert")
                    .label("Invert")
                    .disabled(off)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let all = this.all_dup_paths();
                        this.selected = all.difference(&this.selected).cloned().collect();
                        cx.notify();
                    })),
            )
    }

    /// Bulk expand/collapse for the group list.
    fn render_expand_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let off = self.scanning || self.groups.is_empty();
        h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("expand-all")
                    .label("Expand all")
                    .disabled(off)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded = this.groups.iter().map(|g| g.index).collect();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("collapse-all")
                    .label("Collapse all")
                    .disabled(off || self.expanded.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded.clear();
                        cx.notify();
                    })),
            )
    }

    /// Live similarity floor: type a higher % to tighten the current
    /// results without rescanning (blank restores the scan floor).
    /// Shown only when similar-media groups exist; exact groups always
    /// qualify and are unaffected.
    fn render_sim_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_sim = self.groups.iter().any(|g| g.similarity.is_some());
        if self.scanning || self.groups.is_empty() || !has_sim {
            return div().into_any_element();
        }
        h_flex()
            .gap_1()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("SIM ≥{:.0}%", self.min_sim * 100.0)),
            )
            .child(div().w(px(64.)).child(Input::new(&self.min_sim_input)))
            .into_any_element()
    }

    /// Global keep mode as a segmented control (First / Smallest / Newest /
    /// Oldest); reassigns keepers on the current results, no rescan.
    fn render_keep_segmented(&self, cx: &mut Context<Self>) -> impl IntoElement {
        const MODES: [&str; 4] = ["First", "Smallest", "Newest", "Oldest"];
        let active = self.keep_index.unwrap_or(0).min(3);
        let off = self.scanning || self.groups.is_empty();
        let mut group = ButtonGroup::new("keep-mode").compact().disabled(off);
        for (i, label) in MODES.iter().enumerate() {
            group = group.child(
                Button::new(SharedString::from(format!("keep-mode-{i}")))
                    .label(*label)
                    .selected(i == active),
            );
        }
        group.on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
            if let Some(&i) = clicks.first() {
                this.apply_keep_mode(i, cx);
            }
        }))
    }

    /// Toggle: default discovery order vs biggest-reclaimable-first.
    fn render_sort_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = usize::from(self.sort_biggest);
        let off = self.scanning || self.groups.is_empty();
        ButtonGroup::new("sort-mode")
            .compact()
            .disabled(off)
            .child(
                Button::new("sort-default")
                    .label("Default")
                    .selected(active == 0),
            )
            .child(
                Button::new("sort-biggest")
                    .label("Biggest")
                    .selected(active == 1),
            )
            .on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
                if let Some(&i) = clicks.first() {
                    this.sort_biggest = i == 1;
                    // The toggle is THE explicit order change: sort now.
                    this.sort_groups();
                    this.save_prefs(cx);
                    cx.notify();
                }
            }))
    }

    /// Write the current results as JSON (same schema as `--json`).
    fn save_report(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(stats) = self.last_stats.clone() else {
            self.status = "Nothing to save yet — run a scan first.".to_string();
            cx.notify();
            return;
        };
        // Snapshot everything the async dialog task needs; the view may
        // change (or close) while the native dialog is open.
        let snapshot = ReportSnapshot {
            paths: self.last_paths.clone(),
            hash_algo: self.last_hash_algo.clone(),
            ffprobe_used: self.last_ffprobe,
            stats,
            groups: self.groups.clone(),
        };
        // Same rule as browse(): native dialogs stay off the GPUI thread.
        cx.spawn(async move |this, cx| {
            let file = rfd::AsyncFileDialog::new()
                .set_title("Save duplicate report")
                .set_file_name("dedupe-report.json")
                .save_file()
                .await;
            let Some(file) = file else { return };
            let ctx = report::ReportContext {
                paths: &snapshot.paths,
                hash_algo: &snapshot.hash_algo,
                verbose: false,
                ffprobe_used: snapshot.ffprobe_used,
            };
            let json = report::to_json(&ctx, &snapshot.stats, &snapshot.groups);
            let outcome = std::fs::write(file.path(), json);
            this.update(cx, |view, cx| {
                match outcome {
                    Ok(()) => {
                        view.status = format!("Saved report to {}.", file.path().display());
                    }
                    Err(e) => {
                        view.status = format!("Could not save report: {e}");
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn render_delete_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let targets = self.pending_targets();
        let can_undo = !self.scanning && !self.last_trashed.is_empty();
        h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("delete")
                    .label(format!("Delete selected ({})", targets.len()))
                    .disabled(self.scanning || self.groups.is_empty() || targets.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_delete_confirm(window, cx);
                    })),
            )
            .child(
                Button::new("undo-delete")
                    .label(format!("Undo delete ({})", self.last_trashed.len()))
                    .disabled(!can_undo)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.undo_delete(cx);
                    })),
            )
            .into_any_element()
    }

    /// Delete confirmation as a modal dialog: a per-group dry-run summary
    /// (keeper + exactly what will go, with sizes) plus in-dialog Keep
    /// picks. After a Keep change the dialog reopens fresh so it never
    /// shows stale keep/dup state.
    fn open_delete_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.pending_targets();
        if targets.is_empty() {
            self.status = "Nothing selected for deletion.".to_string();
            cx.notify();
            return;
        }
        let count = targets.len();
        let bytes: u64 = targets.iter().map(|t| t.size).sum();
        let bytes_str = util::human_bytes(bytes);
        let action = if self.trash {
            "move to the trash"
        } else {
            "permanently delete"
        };
        let view = cx.entity();
        // Per-group dry-run data, capped so huge selections stay readable.
        // The dialog closure must stay `Fn`, so everything is owned here.
        const CONFIRM_GROUPS: usize = 6;
        const CONFIRM_ROWS: usize = 4;
        struct ConfirmGroup {
            index: usize,
            keeper: String,
            rows: Vec<(String, u64, PathBuf)>,
            hidden_rows: usize,
            group_bytes: u64,
        }
        let mut confirm_groups: Vec<ConfirmGroup> = Vec::new();
        for g in &self.groups {
            let mut rows = Vec::new();
            let mut group_bytes = 0u64;
            let mut total = 0usize;
            for m in &g.members {
                if !m.keep && self.selected.contains(&m.path) {
                    total += 1;
                    group_bytes += m.size;
                    if rows.len() < CONFIRM_ROWS {
                        rows.push((m.path.display().to_string(), m.size, m.path.clone()));
                    }
                }
            }
            if total == 0 {
                continue;
            }
            let keeper = g
                .members
                .iter()
                .find(|m| m.keep)
                .map(|m| {
                    m.path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| m.path.display().to_string())
                })
                .unwrap_or_else(|| "—".to_string());
            confirm_groups.push(ConfirmGroup {
                index: g.index,
                keeper,
                rows,
                hidden_rows: total.saturating_sub(CONFIRM_ROWS),
                group_bytes,
            });
            if confirm_groups.len() >= CONFIRM_GROUPS {
                break;
            }
        }
        let hidden_groups = self
            .groups
            .iter()
            .filter(|g| {
                g.members
                    .iter()
                    .any(|m| !m.keep && self.selected.contains(&m.path))
            })
            .count()
            .saturating_sub(confirm_groups.len());
        window.open_dialog(
            cx,
            move |dialog, _, _| {
                let confirm_view = view.clone();
                let mut body = v_flex().gap_2().child(format!(
                    "This will {action} {count} file(s) ({bytes_str}). Keepers are kept; every file is re-hashed before removal."
                ));
                for cg in &confirm_groups {
                    let mut section = v_flex()
                        .gap_1()
                        .py_1()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .child(format!(
                                    "Group #{} · keeps {} · deletes {} ({})",
                                    cg.index,
                                    cg.keeper,
                                    cg.rows.len() + cg.hidden_rows,
                                    util::human_bytes(cg.group_bytes),
                                )),
                        );
                    for (name, size, path) in &cg.rows {
                        let gi = cg.index;
                        let keep_path = path.clone();
                        let reopen_view = confirm_view.clone();
                        section = section.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .truncate()
                                        .child(format!(
                                            "{} ({})",
                                            fit_text(name, 70),
                                            util::human_bytes(*size)
                                        )),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "dlg-keep-{gi}-{}",
                                        fit_text(name, 20)
                                    )))
                                    .label("Keep")
                                    .on_click(move |_, window, cx| {
                                        reopen_view.update(cx, |v, cx| {
                                            v.set_keeper(gi, &keep_path, cx);
                                        });
                                        window.close_dialog(cx);
                                        let reopen_view2 = reopen_view.clone();
                                        reopen_view2.update(cx, |v, cx| {
                                            v.open_delete_confirm(window, cx);
                                        });
                                    }),
                                ),
                        );
                    }
                    if cg.hidden_rows > 0 {
                        section = section.child(
                            div()
                                .text_xs()
                                .child(format!("… and {} more in this group", cg.hidden_rows)),
                        );
                    }
                    body = body.child(section);
                }
                if hidden_groups > 0 {
                    body = body.child(
                        div()
                            .text_xs()
                            .child(format!("… and {hidden_groups} more group(s)")),
                    );
                }
                dialog
                    .title(format!("Delete {count} file(s)?"))
                    .child(body)
                    .footer(
                        h_flex().gap_2().justify_end().child(
                            Button::new("cancel-delete").label("Cancel").on_click(
                                |_, window, cx| {
                                    window.close_dialog(cx);
                                },
                            ),
                        ).child(
                            Button::new("confirm-delete")
                                .danger()
                                .label(format!("Delete {count}"))
                                .on_click(move |_, window, cx| {
                                    confirm_view.update(cx, |v, cx| v.perform_delete(cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                    )
            },
        );
    }

    fn render_group(&mut self, gi: usize, cx: &mut Context<Self>) -> impl IntoElement {
        // Snapshot the group first: click handlers must own their data
        // ('static), so nothing borrowed from `self` may leak into them.
        let snapshot = self.groups[gi].clone_snapshot();
        let expanded = self.expanded.contains(&snapshot.index);
        let mut card = v_flex()
            .w_full()
            .gap_1()
            .p_3()
            .border_1()
            .rounded_md()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                Button::new(SharedString::from(format!("group-{}", snapshot.index)))
                    .label(snapshot.header_label(expanded))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let idx = snapshot.index;
                        if !this.expanded.remove(&idx) {
                            this.expanded.insert(idx);
                        }
                        cx.notify();
                    })),
            );
        if expanded {
            // Single-display grid for media groups: every member has a
            // large visual (image or video poster), so render each once as
            // a side-by-side card (left card / right card, info under each)
            // instead of big-preview + duplicate rows. Keeper cards get a
            // highlight border; clicking a preview promotes it to keeper.
            let thumbs: Vec<crate::ui_results::Thumb> =
                snapshot.members.iter().map(|m| self.thumb_for(m)).collect();
            let grid = snapshot.members.len() >= 2 && thumbs.iter().all(is_grid_preview);
            if grid {
                for chunk in snapshot.members.chunks(2).enumerate() {
                    let (ci, members) = chunk;
                    // Stretch keeps both cards in a row the same height so
                    // images and videos always display at the same size.
                    let mut row = h_flex().gap_2().pl_6().items_stretch();
                    for (offset, member) in members.iter().enumerate() {
                        let mi = ci * 2 + offset;
                        let checked = self.selected.contains(&member.path);
                        let thumb = self.thumb_for(member);
                        // Lazy inline strip: one background extraction per
                        // video, kicked off on first sight (render may run
                        // often; the loading set + disk cache dedupe it).
                        if matches!(
                            thumb,
                            crate::ui_results::Thumb::Poster(_)
                                | crate::ui_results::Thumb::PendingVideo
                        ) && !self.strips.contains_key(&member.path)
                            && !self.preview_loading.contains(&member.path)
                        {
                            self.preview_loading.insert(member.path.clone());
                            let job_path = member.path.clone();
                            let done_path = member.path.clone();
                            let size = member.size;
                            let mtime = member.mtime_secs;
                            cx.spawn(async move |this, cx| {
                                let frames = cx
                                    .background_executor()
                                    .spawn(async move {
                                        poster::video_strip(
                                            &job_path,
                                            size,
                                            mtime,
                                            poster::STRIP_FRAMES,
                                        )
                                    })
                                    .await;
                                this.update(cx, |view, cx| {
                                    view.preview_loading.remove(&done_path);
                                    if frames.is_empty() {
                                        view.status = "Inline preview unavailable — install ffmpeg or check the file."
                                            .to_string();
                                    } else {
                                        view.strips.insert(done_path, frames);
                                    }
                                    cx.notify();
                                })
                                .ok();
                            })
                            .detach();
                        }
                        let video = crate::ui_results::VideoPreview {
                            strip: self.strips.get(&member.path).cloned(),
                            loading: self.preview_loading.contains(&member.path),
                        };
                        row = row.child(render_media_card(
                            snapshot.index,
                            mi,
                            member,
                            checked,
                            &thumb,
                            video,
                            cx,
                        ));
                    }
                    // Odd tail: keep the single card left-aligned at half
                    // width instead of stretching full width.
                    if members.len() == 1 {
                        row = row.child(div().flex_1());
                    }
                    card = card.child(row);
                }
            } else {
                for (mi, member) in snapshot.members.iter().enumerate() {
                    let checked = self.selected.contains(&member.path);
                    let thumb = self.thumb_for(member);
                    card = card.child(render_member(
                        snapshot.index,
                        mi,
                        member,
                        checked,
                        &thumb,
                        cx,
                    ));
                }
            }
        }
        card
    }
}

impl Render for DedupeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().is_dark();
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            // Checklist C29: Escape always does something — backs out of
            // Settings to the main flow.
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" && this.view == AppView::Settings {
                    this.view = AppView::Main;
                    cx.notify();
                }
            }))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(cx.theme().accent)
                                    .child("◆ dedupe"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(if self.view == AppView::Settings {
                                Button::new("nav-back")
                                    .label("← Back")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.view = AppView::Main;
                                        cx.notify();
                                    }))
                            } else {
                                Button::new("nav-settings")
                                    .label("Settings")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.view = AppView::Settings;
                                        cx.notify();
                                    }))
                            })
                            .child(
                                Button::new("theme-toggle")
                                    .icon(if dark { IconName::Sun } else { IconName::Moon })
                                    .label(if dark { "Light" } else { "Dark" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        Theme::change(
                                            if dark {
                                                ThemeMode::Light
                                            } else {
                                                ThemeMode::Dark
                                            },
                                            None,
                                            cx,
                                        );
                                        this.save_prefs(cx);
                                    })),
                            ),
                    ),
            )
            .child(if self.view == AppView::Settings {
                div()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_settings(cx))
                    .into_any_element()
            } else {
                h_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_sidebar(cx))
                    .child(self.render_results(cx))
                    .into_any_element()
            })
            .child(
                h_flex()
                    .items_center()
                    .px_4()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{}  ·  scanned {} file(s) ({})",
                                self.status,
                                self.files_scanned,
                                util::human_bytes(self.bytes_scanned)
                            )),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: the size sliders must build without panicking. The state
    /// clamps on every builder call, so max() has to precede min() —
    /// min(1024) with the default max(100) panics inside f32::clamp.
    #[test]
    fn size_sliders_build_without_panic() {
        for default in [SIZE_SLIDER_MIN, SIZE_SLIDER_MAX] {
            let s = SliderState::new()
                .max(SIZE_SLIDER_MAX)
                .min(SIZE_SLIDER_MIN)
                .scale(SliderScale::Logarithmic)
                .default_value(default);
            assert_eq!(s.min_value(), SIZE_SLIDER_MIN);
            assert_eq!(s.max_value(), SIZE_SLIDER_MAX);
        }
    }
}
