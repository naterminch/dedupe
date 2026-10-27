use crate::ui::theme::apply_readable_theme;
use crate::ui::{
    AppView, DedupeView, FolderEntry, GROUP_PAGE, SIZE_SLIDER_MAX, SIZE_SLIDER_MIN, SettingsTab,
};
use crate::ui_helpers;
use crate::ui_helpers::{
    NO_LIMIT_LABEL, bar_pos, is_no_limit, parse_size_limit, prefs_size_to_slider,
    size_slider_to_text, size_text_to_slider,
};
use crate::{cli, media, pipeline, poster, prefs, util};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::{InputEvent, InputState, NumberStep};
use gpui_kit::component::slider::{SliderEvent, SliderScale, SliderState};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{AppContext as _, Context, Entity, Focusable as _, Window};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

impl DedupeView {
    /// Build the initial view: restore saved prefs, wire the slider/text
    /// sync subscriptions, kick off the ffmpeg probe and the theme
    /// restore before the first paint.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = prefs::GuiPrefs::load();
        let mk = |placeholder: &str, default: &str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(default)
            })
        };
        let mk_num =
            |placeholder: &str, default: &str, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .default_value(default)
                        .step(NumberStep::Fixed(1.0))
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
        let max_depth_input = mk_num("unlimited", &saved.max_depth, window, cx);
        let exclude_dir_input = mk("node_modules", &saved.exclude_dir, window, cx);
        let exclude_path_input = mk("substring", &saved.exclude_path, window, cx);
        let similarity_input = mk_num("0–100", &saved.similarity, window, cx);
        let partial_similarity_input = mk_num("85", &saved.partial_similarity, window, cx);
        let fuzzy_threshold_input = mk_num("85", &saved.fuzzy_threshold, window, cx);
        let top_input = mk_num("50", &saved.top, window, cx);
        let ignore_input = mk("*.tmp; node_modules", &saved.ignore, window, cx);
        let jobs_input = mk_num("0", &saved.jobs, window, cx);
        // Live similarity floor for the current results (blank = scan
        // default). Lowering below the scan floor needs a rescan.
        let min_sim_input = mk_num("e.g. 98", "", window, cx);
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
            settings_tab: SettingsTab::General,
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
            partial_similarity_input,
            fuzzy_threshold_input,
            top_input,
            ignore_input,
            jobs_input,
            hash_index: Some(saved.hash_index.min(2)),
            keep_index: Some(saved.keep_index.min(4)),
            find_index: Some(saved.find_index.min(10)),
            match_index: Some(saved.match_index.min(3)),
            image_hash_index: Some(saved.image_hash_index.min(4)),
            partial_clips: saved.partial_clips,
            music_by_content: saved.music_by_content,
            exact: saved.exact,
            no_cache: saved.no_cache,
            cache_msg: String::new(),
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
            pending_toast: None,
            files_scanned: 0,
            bytes_scanned: 0,
            has_scanned: false,
            ffprobe_note: false,
            groups: Vec::new(),
            mode_result: None,
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
            consolidate_dest: None,
            scan_cancel: crate::cancel::CancelToken::new(),
            scan_gen: 0,
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
        view._subs.push(cx.subscribe_in(
            &view.folder_input,
            window,
            |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_from_input(window, cx);
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
        Theme::change(
            if saved.dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            None,
            cx,
        );
        apply_readable_theme(cx);
        // Checklist A4: initial focus lands in the folder input so the user
        // can type immediately after launch.
        let initial_focus = view.folder_input.read(cx).focus_handle(cx);
        window.focus(&initial_focus, cx);
        view
    }
    /// True while `gen` is the latest scan generation — false once a Stop
    /// or a newer scan has superseded the task that captured `gen`.
    pub(crate) fn is_current_scan(&self, generation: u64) -> bool {
        self.scan_gen == generation
    }
    /// Stop the running scan: arm the cancellation token AND reset the
    /// view immediately, so the UI never sits in "Stopping…" while a large
    /// file finishes hashing. The pipeline winds down in the background
    /// (bounded by per-file / per-frame / per-member checks) and its late
    /// output is discarded by the generation guard — the bumped generation
    /// also makes any in-flight task's `apply_output` a no-op.
    pub(crate) fn stop_scan(&mut self, cx: &mut Context<Self>) {
        self.scan_cancel.cancel();
        self.scan_gen += 1;
        self.scanning = false;
        self.scan_done = 0;
        self.scan_total = 0;
        self.scan_determinate = false;
        self.has_scanned = false;
        self.groups.clear();
        self.mode_result = None;
        self.base_groups.clear();
        self.selected.clear();
        self.expanded.clear();
        self.last_stats = None;
        self.status = "Scan cancelled.".to_string();
        cx.notify();
    }
    /// Escape always does something: back out of Settings to the main
    /// flow. No-op on the main view. Extracted from the key listener so
    /// the state transition is directly testable (the offscreen test
    /// harness does not route keystrokes into `open_window` windows).
    pub(crate) fn leave_settings(&mut self, cx: &mut Context<Self>) {
        if self.view == AppView::Settings {
            self.view = AppView::Main;
            cx.notify();
        }
    }
    /// Read the sidebar form into CLI options. This is the GUI equivalent of
    /// clap parsing: every field maps 1:1 to a `dedupe` flag.
    pub(crate) fn collect_options(&self, cx: &mut Context<Self>) -> Result<cli::Cli, String> {
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
        let partial_raw = val(&self.partial_similarity_input).trim().to_string();
        let partial_similarity: f64 = if partial_raw.is_empty() {
            85.0
        } else {
            partial_raw.parse().map_err(|_| {
                "Partial-clip similarity must be a number between 0 and 100.".to_string()
            })?
        };
        if !(0.0..=100.0).contains(&partial_similarity) {
            return Err(format!(
                "Partial-clip similarity must be between 0 and 100 (got {partial_similarity})."
            ));
        }
        let fuzzy_raw = val(&self.fuzzy_threshold_input).trim().to_string();
        let fuzzy_threshold: f64 = if fuzzy_raw.is_empty() {
            85.0
        } else {
            fuzzy_raw
                .parse()
                .map_err(|_| "Fuzzy threshold must be a number between 0 and 100.".to_string())?
        };
        if !(0.0..=100.0).contains(&fuzzy_threshold) {
            return Err(format!(
                "Fuzzy threshold must be between 0 and 100 (got {fuzzy_threshold})."
            ));
        }
        let top_raw = val(&self.top_input).trim().to_string();
        let top: usize = if top_raw.is_empty() {
            50
        } else {
            top_raw
                .parse()
                .map_err(|_| "Largest-files count must be a whole number above 0.".to_string())?
        };
        if top == 0 {
            return Err("Largest-files count must be above 0.".to_string());
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
            keep_best_quality: self.keep_index.unwrap_or(0) == 4,
            consolidate_dir: None,
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
            clear_cache: false,
            min_size: opt(&self.min_size_input),
            max_size: opt(&self.max_size_input),
            exclude_dir: split_multi(&val(&self.exclude_dir_input)),
            exclude_path: split_multi(&val(&self.exclude_path_input)),
            json: false,
            verbose: false,
            quiet: true,
            jobs,
            gui: false,
            find: ui_helpers::find_mode(self.find_index),
            r#match: ui_helpers::match_mode(self.match_index),
            fuzzy_threshold,
            top,
            image_hash: ui_helpers::image_hash_algo(self.image_hash_index),
            partial_clips: self.partial_clips,
            partial_similarity,
            music_by: if self.music_by_content {
                cli::MusicMode::Content
            } else {
                cli::MusicMode::Tags
            },
            csv: None,
            html: None,
            move_dir: None,
            hardlink: false,
            strip_exif: false,
            ignore: split_multi(&val(&self.ignore_input)),
        })
    }
    /// Snapshot the current sidebar into `gui-prefs.json`. Best-effort:
    /// failures are ignored so a read-only home dir never breaks the GUI.
    /// Text fields are saved here (at scan time) plus on every discrete
    /// toggle, which avoids a disk write per keystroke.
    pub(crate) fn save_prefs(&self, cx: &mut Context<Self>) {
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
            partial_similarity: val(&self.partial_similarity_input),
            fuzzy_threshold: val(&self.fuzzy_threshold_input),
            top: val(&self.top_input),
            ignore: val(&self.ignore_input),
            jobs: val(&self.jobs_input),
            hash_index: self.hash_index.unwrap_or(0),
            keep_index: self.keep_index.unwrap_or(0),
            find_index: self.find_index.unwrap_or(0),
            match_index: self.match_index.unwrap_or(0),
            image_hash_index: self.image_hash_index.unwrap_or(0),
            partial_clips: self.partial_clips,
            music_by_content: self.music_by_content,
            exact: self.exact,
            no_cache: self.no_cache,
            trash: self.trash,
            sort_biggest: self.sort_biggest,
            remember_folders: self.remember_folders,
            dark: cx.theme().is_dark(),
        };
        out.save_async();
    }
    pub(crate) fn start_scan(&mut self, cx: &mut Context<Self>) {
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
        // New generation: invalidates any in-flight (stopped) scan task,
        // and a fresh cancellation token for this run — Stop sets it and
        // the pipeline polls it at every phase boundary.
        self.scan_gen += 1;
        let generation = self.scan_gen;
        self.scan_cancel.reset();
        let cancel = self.scan_cancel.clone();
        self.scan_done = 0;
        self.scan_total = 0;
        self.scan_determinate = false;
        self.has_scanned = false;
        self.groups.clear();
        self.mode_result = None;
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
                    pipeline::run_scan(
                        &opts,
                        Some(&hash_bar),
                        Some(&sim_bar),
                        &|p| {
                            if let Ok(mut s) = phase_write.lock() {
                                *s = p;
                            }
                        },
                        &cancel,
                    )
                })
                .await;
            this.update(cx, |view: &mut Self, cx| {
                // A Stop (or a newer scan) bumps the generation while this
                // task was in flight: its late output must not clobber the
                // state that replaced it.
                if !view.is_current_scan(generation) {
                    return;
                }
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
    pub(crate) fn apply_output(&mut self, out: pipeline::ScanOutput, cx: &mut Context<Self>) {
        self.scanning = false;
        self.scan_done = 0;
        self.scan_total = 0;
        self.scan_determinate = false;
        // A cancelled scan carries no groups by design: reset to the
        // pre-scan state (the walk cleared the pane at start anyway) and
        // report the stop. `has_scanned` stays false so the 3-step empty
        // state remains up.
        if out.cancelled {
            self.groups.clear();
            self.mode_result = None;
            self.base_groups.clear();
            self.selected.clear();
            self.expanded.clear();
            self.has_scanned = false;
            self.files_scanned = out.stats.files_scanned;
            self.bytes_scanned = out.stats.bytes_scanned;
            self.last_stats = None;
            self.status = format!(
                "Scan cancelled after {} file(s). Press Scan to run again.",
                out.stats.files_scanned
            );
            cx.notify();
            return;
        }
        self.has_scanned = true;
        self.files_scanned = out.stats.files_scanned;
        self.bytes_scanned = out.stats.bytes_scanned;
        self.ffprobe_note = out.needs_ffprobe_note;
        self.last_stats = Some(out.stats);
        self.last_paths = out.paths.clone();
        self.last_hash_algo = out.hash_algo.to_string();
        self.last_ffprobe = out.ffprobe_used;
        self.last_ref_dirs = out.reference_dirs;
        if let Some(mode) = out.mode_result {
            let title = crate::modes::mode_title(mode.mode);
            let findings = mode.finding_count();
            let group_count = mode.groups.len();
            let reclaim = mode.reclaimable();
            let empty = mode.is_empty();
            self.mode_result = Some(mode);
            self.groups.clear();
            self.base_groups.clear();
            self.selected.clear();
            self.expanded.clear();
            self.consolidate_dest = None;
            self.group_limit = GROUP_PAGE;
            self.dup_files = findings as u64;
            self.reclaim_bytes = reclaim;
            self.status = if empty {
                format!(
                    "{title}: nothing found in {} file(s) ({}).",
                    self.files_scanned,
                    util::human_bytes(self.bytes_scanned)
                )
            } else if group_count > 0 {
                format!(
                    "{title}: {group_count} group(s) · {findings} item(s) · {} reclaimable.",
                    util::human_bytes(reclaim)
                )
            } else {
                format!(
                    "{title}: {findings} finding(s) · {} reclaimable.",
                    util::human_bytes(reclaim)
                )
            };
            self.pending_toast = Some((self.status.clone(), false));
            self.posters.clear();
            self.strips.clear();
            self.preview_loading.clear();
            cx.notify();
            return;
        }
        self.mode_result = None;
        self.groups = out.groups;
        // Pristine copy for live re-thresholding; the floor starts at the
        // scan threshold (everything qualifies).
        self.base_groups = self.groups.clone();
        // Fresh results: disarm any pending consolidate from last scan.
        self.consolidate_dest = None;
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
        self.pending_toast = Some((self.status.clone(), false));
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
}
