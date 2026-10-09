//! Desktop GUI for dedupe, built on gpui-kit.
//!
//! Opens when the binary runs without a scan path (or with `--gui`). The GUI
//! drives the same [`crate::pipeline::run_scan`] as the CLI, so results are
//! identical; only the presentation differs.
//!
//! Layout: a left settings sidebar (every CLI option), a main results pane
//! with one collapsible section per duplicate group, and a status bar.

use crate::{actions, cli, hashing, media, pipeline, poster, report, util};
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
    AppContext as _, Context, Entity, FontWeight, IntoElement, ObjectFit, ParentElement as _,
    Render, SharedString, Size, Styled as _, StyledImage as _, Subscription, TitlebarOptions,
    Window, WindowOptions, div, img, px,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Slider bounds for the size filters (log scale): 1 KB … 1 TB.
const SIZE_SLIDER_MIN: f32 = 1024.0;
const SIZE_SLIDER_MAX: f32 = 1_000_000_000_000.0;

/// Image formats the UI can thumbnail directly from disk.
const THUMB_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "bmp", "webp"];

/// Groups rendered before a "Show more" button takes over; keeps huge
/// result sets fast and navigable.
const GROUP_PAGE: usize = 100;

/// Member paths previewed inside the delete confirm dialog.
const CONFIRM_PREVIEW: usize = 8;

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

pub struct DedupeView {
    folder_input: Entity<InputState>,
    folders: Vec<FolderEntry>,
    types_input: Entity<InputState>,
    min_size_input: Entity<InputState>,
    max_size_input: Entity<InputState>,
    max_depth_input: Entity<InputState>,
    exclude_dir_input: Entity<InputState>,
    exclude_path_input: Entity<InputState>,
    similarity_input: Entity<InputState>,
    jobs_input: Entity<InputState>,
    hash_index: Option<usize>,
    keep_index: Option<usize>,
    exact: bool,
    no_cache: bool,
    trash: bool,
    sort_biggest: bool,
    last_stats: Option<report::ScanStats>,
    last_paths: Vec<String>,
    last_hash_algo: String,
    last_ffprobe: bool,
    last_ref_dirs: Vec<PathBuf>,
    min_size_slider: Entity<SliderState>,
    max_size_slider: Entity<SliderState>,
    ffmpeg_ok: bool,
    ffprobe_ok: bool,
    media_checked: bool,
    posters: HashMap<PathBuf, PathBuf>,
    dup_files: u64,
    reclaim_bytes: u64,
    scanning: bool,
    scan_done: u64,
    scan_total: u64,
    scan_determinate: bool,
    status: String,
    files_scanned: usize,
    bytes_scanned: u64,
    has_scanned: bool,
    ffprobe_note: bool,
    groups: Vec<crate::matching::Group>,
    /// How many groups render at once; the rest sit behind a "Show more"
    /// button so huge result sets stay navigable (and fast).
    group_limit: usize,
    last_hash: cli::HashAlgo,
    expanded: HashSet<usize>,
    selected: HashSet<PathBuf>,
    _subs: Vec<Subscription>,
}

/// One folder to scan, with an optional protection flag (Krokiet's Ref
/// checkbox pattern: protected folders are never deleted from).
#[derive(Debug, Clone)]
struct FolderEntry {
    path: String,
    reference: bool,
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
        let mk = |placeholder: &str, default: &str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(default)
            })
        };
        let folder_input = mk("Type a folder, then Add (or Browse…)", "", window, cx);
        let types_input = mk("jpg,png,mp4 (empty = all)", "", window, cx);
        let min_size_input = mk("100KB", "", window, cx);
        let max_size_input = mk("500MB", "", window, cx);
        let max_depth_input = mk("0 = top level only", "", window, cx);
        let exclude_dir_input = mk("node_modules", "", window, cx);
        let exclude_path_input = mk("substring", "", window, cx);
        let similarity_input = mk("0–100", "97", window, cx);
        let jobs_input = mk("0", "", window, cx);
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
        let min_size_slider = mk_size_slider(SIZE_SLIDER_MIN, cx);
        let max_size_slider = mk_size_slider(SIZE_SLIDER_MAX, cx);
        let mut view = Self {
            folder_input,
            folders: Vec::new(),
            types_input,
            min_size_input,
            max_size_input,
            max_depth_input,
            exclude_dir_input,
            exclude_path_input,
            similarity_input,
            jobs_input,
            hash_index: Some(0),
            keep_index: Some(0),
            exact: false,
            no_cache: false,
            trash: true,
            sort_biggest: false,
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
            group_limit: GROUP_PAGE,
            last_hash: cli::HashAlgo::Blake3,
            expanded: HashSet::new(),
            selected: HashSet::new(),
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
            jobs_raw.parse().map_err(|_| {
                "Worker threads must be a whole number (0 = auto).".to_string()
            })?
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
            if v.is_empty() { None } else { Some(v) }
        };
        // Validate sizes now so typos surface before the (slow) scan starts.
        for e in [&self.min_size_input, &self.max_size_input] {
            if let Some(v) = opt(e)
                && let Err(err) = util::parse_size(&v)
            {
                return Err(format!("Bad size “{v}”: {err:#}"));
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

    fn start_scan(&mut self, cx: &mut Context<Self>) {
        let opts = match self.collect_options(cx) {
            Ok(o) => o,
            Err(msg) => {
                self.status = msg;
                cx.notify();
                return;
            }
        };
        self.last_hash = opts.hash;
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
        self.group_limit = GROUP_PAGE;
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
        self.selected = self
            .groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| !m.keep)
            .map(|m| m.path.clone())
            .collect();
    }

    /// Promote one member to keeper of its group (per-group decision).
    fn set_keeper(&mut self, group_index: usize, path: &std::path::Path, cx: &mut Context<Self>) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.index == group_index) {
            for m in g.members.iter_mut() {
                m.keep = m.path == path;
            }
            g.members
                .sort_by(|a, b| (!a.keep).cmp(&!b.keep).then_with(|| a.path.cmp(&b.path)));
        }
        self.reselect_all_dups();
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
        cx.notify();
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
        for p in &outcome.deleted {
            self.selected.remove(p);
        }
        let verb = if self.trash {
            "Moved to trash"
        } else {
            "Deleted"
        };
        self.status = format!(
            "{verb} {} file(s) ({} freed); {} skipped.",
            outcome.deleted.len(),
            util::human_bytes(outcome.bytes_freed),
            outcome.skipped.len()
        );
        cx.notify();
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
        // Fixed folder row on top, scrollable options in the middle, and a
        // sticky Scan button at the bottom — usable at any window height.
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
                            .child("SCAN FOLDERS (TICK REF TO PROTECT)"),
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
                        .child(self.sidebar_field(
                            "FILE TYPES (EMPTY = ALL)",
                            Input::new(&self.types_input),
                            cx,
                        ))
                        .child(self.sidebar_section("HASH ALGORITHM", cx))
                        .child(
                            RadioGroup::new("hash")
                                .children(["blake3", "sha256", "md5"])
                                .selected_index(self.hash_index)
                                .on_change(cx.listener(|this, value, _, cx| {
                                    this.hash_index = Some(*value);
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
                        .child(self.sidebar_field(
                            "WORKER THREADS (0 = AUTO)",
                            Input::new(&self.jobs_input),
                            cx,
                        ))
                        .child(
                            Checkbox::new("exact")
                                .label("Exact duplicates only")
                                .checked(self.exact)
                                .on_change(cx.listener(|this, value, _, cx| {
                                    this.exact = *value;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Checkbox::new("no-cache")
                                .label("Skip caches (hash + fingerprint)")
                                .checked(self.no_cache)
                                .on_change(cx.listener(|this, value, _, cx| {
                                    this.no_cache = *value;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Checkbox::new("trash")
                                .label("Move to trash (recoverable)")
                                .checked(self.trash)
                                .on_change(cx.listener(|this, value, _, cx| {
                                    this.trash = *value;
                                    cx.notify();
                                })),
                        )
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
            .child(step("1.", "Add a folder on the left (type it and Add, or Browse…)."))
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
        let mut order: Vec<usize> = Vec::with_capacity(self.groups.len());
        for gi in 0..self.groups.len() {
            order.push(gi);
        }
        if self.sort_biggest {
            order.sort_by(|&a, &b| self.groups[b].dup_bytes().cmp(&self.groups[a].dup_bytes()));
        }
        // Render in pages: thousands of groups would otherwise build
        // thousands of elements up front and stall the UI.
        let total = order.len();
        for gi in order.into_iter().take(self.group_limit) {
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

    /// Bulk selection: one "Select all" checkbox (toggle all/none).
    /// Keepers are never selectable.
    fn render_select_presets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dups: HashSet<PathBuf> = self
            .groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| !m.keep)
            .map(|m| m.path.clone())
            .collect();
        let off = self.scanning || dups.is_empty();
        let all_selected = !dups.is_empty() && dups.iter().all(|p| self.selected.contains(p));
        Checkbox::new("sel-all")
            .label("All")
            .checked(all_selected)
            .disabled(off)
            .on_change(cx.listener(move |this, value, _, cx| {
                if *value {
                    this.selected = this
                        .groups
                        .iter()
                        .flat_map(|g| g.members.iter())
                        .filter(|m| !m.keep)
                        .map(|m| m.path.clone())
                        .collect();
                } else {
                    this.selected.clear();
                }
                cx.notify();
            }))
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
        group.on_click(cx.listener(
            move |this, clicks: &Vec<usize>, _, cx| {
                if let Some(&i) = clicks.first() {
                    this.apply_keep_mode(i, cx);
                }
            },
        ))
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
        Button::new("delete")
            .label(format!("Delete selected ({})", targets.len()))
            .disabled(self.scanning || self.groups.is_empty() || targets.is_empty())
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_delete_confirm(window, cx);
            }))
            .into_any_element()
    }

    /// Delete confirmation as a modal dialog (replaces the old inline bar).
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
        // Preview the actual filenames (capped): a destructive action
        // should show what it will touch, not just a count. Built inside
        // the dialog closure (which must stay `Fn`) from borrowed data.
        let shown: Vec<String> = targets
            .iter()
            .take(CONFIRM_PREVIEW)
            .map(|t| t.path.display().to_string())
            .collect();
        let hidden = count.saturating_sub(shown.len());
        window.open_dialog(
            cx,
            move |dialog, _, _| {
                let confirm_view = view.clone();
                let mut file_list = v_flex().gap_1().py_1();
                for p in &shown {
                    file_list =
                        file_list.child(div().text_xs().truncate().child(fit_text(p, 90)));
                }
                if hidden > 0 {
                    file_list = file_list
                        .child(div().text_xs().child(format!("… and {hidden} more")));
                }
                dialog
                    .title(format!("Delete {count} file(s)?"))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(format!(
                                "This will {action} {count} file(s) ({bytes_str}). Keepers are kept; every file is re-hashed before removal."
                            ))
                            .child(file_list),
                    )
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
        card
    }

    /// Decide what preview a member row shows.
    fn thumb_for(&self, member: &crate::matching::GroupMember) -> Thumb {
        match media::classify(&member.path) {
            media::MediaKind::Image => {
                let thumbable = member
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| THUMB_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                    .unwrap_or(false);
                if thumbable { Thumb::Image } else { Thumb::None }
            }
            media::MediaKind::Video => match self.posters.get(&member.path) {
                Some(p) => Thumb::Poster(p.clone()),
                None => Thumb::PendingVideo,
            },
            media::MediaKind::Other => Thumb::None,
        }
    }
}

/// Owned per-group data for rendering. Groups are replaced wholesale on
/// every scan/delete, so click handlers capture this snapshot instead of
/// borrowing the view (handlers must be `'static`).
struct GroupSnapshot {
    index: usize,
    header: String,
    members: Vec<crate::matching::GroupMember>,
}

trait SnapshotGroup {
    fn clone_snapshot(&self) -> GroupSnapshot;
}

impl SnapshotGroup for crate::matching::Group {
    fn clone_snapshot(&self) -> GroupSnapshot {
        // Exact groups omit the content hash: identical bytes are implied
        // by the grouping itself, and a hex prefix is not actionable.
        // Similar groups keep their score — that one informs the decision.
        let mut parts = vec![
            format!("Group #{}", self.index),
            report::group_kind_name(self.media_kind).to_uppercase(),
            format!("{} files", self.members.len()),
            format!("{} reclaimable", util::human_bytes(self.dup_bytes())),
        ];
        if let Some(s) = self.similarity {
            parts.push(format!("{:.1}% similar", s * 100.0));
        }
        if let Some(summary) = self
            .members
            .first()
            .and_then(|m| m.media.clone())
            .map(|info| info.summary())
        {
            parts.push(summary);
        }
        GroupSnapshot {
            index: self.index,
            header: parts.join(" · "),
            members: self.members.clone(),
        }
    }
}

impl GroupSnapshot {
    fn header_label(&self, expanded: bool) -> String {
        format!("{} {}", if expanded { "▾ " } else { "▸ " }, self.header)
    }
}

/// Read (position, length) off a headless progress bar.
fn bar_pos(bar: &indicatif::ProgressBar) -> (u64, u64) {
    (bar.position(), bar.length().unwrap_or(0))
}

/// Cap a display string, keeping the tail — filenames live at the end of
/// paths, so a truncated middle would hide the useful part. Paired with
/// `.truncate()` (ellipsis) on the element for pixel-level clipping.
fn fit_text(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    format!(
        "…{}",
        s.chars().skip(count - max_chars + 1).collect::<String>()
    )
}

/// What preview a member row shows: none, the image itself, a generated
/// video poster, or a placeholder while the poster extracts.
enum Thumb {
    None,
    Image,
    Poster(PathBuf),
    PendingVideo,
}

/// One-line explanation for each hash algorithm, shown under the selector.
fn hash_caption(index: Option<usize>) -> &'static str {
    match index.unwrap_or(0) {
        1 => "SHA-256 — widely supported standard; pick it to match other tools.",
        2 => "MD5 — legacy and collision-prone; only to match old hashes.",
        _ => "BLAKE3 — fastest modern hash; the right default for scans.",
    }
}

/// Mirror a size slider position into its text box. `empty_at_min`: the
/// min-size slider clears the box at the left end (no limit); the max-size
/// slider clears it at the right end.
fn size_slider_to_text(
    value: f32,
    empty_at_min: bool,
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<DedupeView>,
) {
    let at_empty_end = if empty_at_min {
        value <= SIZE_SLIDER_MIN * 1.001
    } else {
        value >= SIZE_SLIDER_MAX / 1.001
    };
    let text = if at_empty_end {
        String::new()
    } else {
        util::human_bytes(value.max(1.0) as u64)
    };
    input.update(cx, |state, cx| {
        state.set_value(SharedString::from(text), window, cx);
    });
}

/// Mirror a typed size into its slider. Empty means "no limit" (slider to
/// the empty end); unparsable text leaves the slider alone.
fn size_text_to_slider(
    raw: &str,
    empty_at_min: bool,
    slider: &Entity<SliderState>,
    window: &mut Window,
    cx: &mut Context<DedupeView>,
) {
    let raw = raw.trim();
    let target: f32 = if raw.is_empty() {
        if empty_at_min {
            SIZE_SLIDER_MIN
        } else {
            SIZE_SLIDER_MAX
        }
    } else {
        match util::parse_size(raw) {
            Ok(b) => (b as f32).clamp(SIZE_SLIDER_MIN, SIZE_SLIDER_MAX),
            Err(_) => return,
        }
    };
    slider.update(cx, |state, cx| {
        state.set_value(target, window, cx);
    });
}

/// Thumbnail box for a member row: the image itself, a video poster, a
/// placeholder while the poster extracts, or nothing for plain files.
fn render_thumb(
    path: &std::path::Path,
    thumb: &Thumb,
    cx: &mut Context<DedupeView>,
) -> impl IntoElement {
    match thumb {
        Thumb::None => div().into_any_element(),
        Thumb::Image => fixed_thumb(img(path)),
        Thumb::Poster(poster) => fixed_thumb(img(poster.clone())),
        Thumb::PendingVideo => div()
            .w(px(72.))
            .h(px(56.))
            .rounded_md()
            .bg(cx.theme().secondary)
            .flex()
            .items_center()
            .justify_center()
            .child(IconName::Play)
            .into_any_element(),
    }
}

/// Fixed 72×56 thumbnail box (cover-fit): async image loads must never
/// change row geometry, or rows overlap mid-layout.
fn fixed_thumb(image: gpui_kit::Img) -> gpui_kit::AnyElement {
    image
        .w(px(72.))
        .h(px(56.))
        .object_fit(ObjectFit::Cover)
        .rounded_md()
        .into_any_element()
}

fn render_member(
    group_index: usize,
    mi: usize,
    member: &crate::matching::GroupMember,
    checked: bool,
    thumb: &Thumb,
    cx: &mut Context<DedupeView>,
) -> impl IntoElement {
    // Three-line row: the file name gets top billing, the parent folder
    // sits dimmed underneath, and all fixed-size controls share a third
    // line. Flex items will not shrink below content width here, so text
    // must never share a row with buttons.
    let name = member
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| member.path.display().to_string());
    let parent = member
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut title = name;
    if let Some(sim) = member.similarity
        && !member.keep
    {
        title.push_str(&format!("  · {:.1}% similar", sim * 100.0));
    }
    if let Some(info) = &member.media {
        title.push_str(&format!("  · {}", info.summary()));
    }

    let mut actions = h_flex().gap_2().items_center();
    if member.keep {
        let (badge, color) = if member.reference {
            ("◈ REF", cx.theme().accent)
        } else {
            ("✓ KEEP", cx.theme().success)
        };
        actions = actions.child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(badge),
        );
    } else {
        let key = member.path.clone();
        actions = actions
            .child(
                Checkbox::new(SharedString::from(format!("dup-{group_index}-{mi}")))
                    .checked(checked)
                    .on_change(cx.listener(move |this, value, _, cx| {
                        if *value {
                            this.selected.insert(key.clone());
                        } else {
                            this.selected.remove(&key);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(cx.theme().warning)
                    .child("DUP"),
            );
    }
    actions = actions.child(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(util::human_bytes(member.size)),
    );
    if !member.keep {
        let keep_path = member.path.clone();
        actions = actions.child(
            Button::new(SharedString::from(format!("keep-{group_index}-{mi}")))
                .label("Keep")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_keeper(group_index, &keep_path, cx);
                })),
        );
    }
    if matches!(thumb, Thumb::Poster(_) | Thumb::PendingVideo) {
        let play_path = member.path.clone();
        actions = actions.child(
            Button::new(SharedString::from(format!("play-{group_index}-{mi}")))
                .icon(IconName::Play)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Err(e) = open::that(&play_path) {
                        this.status = format!("Could not play file: {e}");
                        cx.notify();
                    }
                })),
        );
    }
    {
        let reveal = member
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| member.path.clone());
        actions = actions.child(
            Button::new(SharedString::from(format!("reveal-{group_index}-{mi}")))
                .icon(IconName::FolderOpen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Err(e) = open::that(&reveal) {
                        this.status = format!("Could not open folder: {e}");
                        cx.notify();
                    }
                })),
        );
    }

    h_flex()
        .gap_2()
        .items_start()
        .pl_6()
        .child(render_thumb(&member.path, thumb, cx))
        .child(
            v_flex()
                .flex_1()
                .gap_1()
                .child(div().text_sm().truncate().child(fit_text(&title, 120)))
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(fit_text(&parent, 160)),
                )
                .child(actions),
        )
}

impl Render for DedupeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().is_dark();
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
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
                        Button::new("theme-toggle")
                            .icon(if dark { IconName::Sun } else { IconName::Moon })
                            .label(if dark { "Light" } else { "Dark" })
                            .on_click(cx.listener(move |_, _, _, cx| {
                                Theme::change(
                                    if dark {
                                        ThemeMode::Light
                                    } else {
                                        ThemeMode::Dark
                                    },
                                    None,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_sidebar(cx))
                    .child(self.render_results(cx)),
            )
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

    #[test]
    fn fit_text_keeps_short_strings_and_truncates_tails() {
        assert_eq!(fit_text("abc", 10), "abc");
        assert_eq!(fit_text("abcdef", 6), "abcdef");
        // Long paths keep the filename tail, prefixed with an ellipsis.
        let capped = fit_text("C:\\very\\long\\directory\\file.txt", 12);
        assert_eq!(capped, "…ry\\file.txt");
        assert_eq!(capped.chars().count(), 12);
    }
}
