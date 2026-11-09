//! Everything the panes can *do*: scans, deletes, exports, preferences.
//!
//! Views only render. Every state transition lives here so a button handler
//! is a one-liner and the same logic stays reachable from tests. Long work
//! (walking, hashing, media probing, native dialogs, deletions) runs on the
//! runtime's blocking threads and applies its result back on the UI thread.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nsh_core::prelude::*;

use crate::actions as file_actions;
use crate::ui::state::{AppState, AppView, FolderEntry, ReportSnapshot, SettingsTab, app};
use crate::ui::window_state;
use crate::ui_helpers::{bar_pos, find_mode, image_hash_algo, is_no_limit, match_mode};
use crate::{cli, hashing, media, pipeline, poster, prefs, similar, util};

// --- Boot -------------------------------------------------------------------

/// Set once per process: components re-run, bootstrapping must not.
static BOOTSTRAPPED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Restore preferences, theme and the ffmpeg probe. Runs once, on the first
/// frame, after the component tree exists.
pub(crate) fn bootstrap() {
    if BOOTSTRAPPED.set(()).is_err() {
        return;
    }
    let saved = prefs::GuiPrefs::load();
    set_color_scheme(if saved.dark {
        Appearance::Dark
    } else {
        Appearance::Light
    });
    app().update(|s| *s = AppState::from_prefs(&saved));
    // Window frame: replay the saved geometry, then start the live
    // sampler so `save_prefs` always stores the real frame.
    window_state::restore(saved.window);
    window_state::start_watching();
    // Two process spawns can cost hundreds of milliseconds: keep them off
    // the first paint.
    app_spawn(async move {
        let probed = blocking(|| (media::ffmpeg_available(), media::ffprobe_available())).await;
        let (ffmpeg_ok, ffprobe_ok) = probed.unwrap_or((false, false));
        app().update(|s| {
            s.ffmpeg_ok = ffmpeg_ok;
            s.ffprobe_ok = ffprobe_ok;
            s.media_checked = true;
        });
    });
}

// --- Navigation -------------------------------------------------------------

pub(crate) fn leave_settings() {
    app().update(|s| {
        if s.view == AppView::Settings {
            s.view = AppView::Main;
        }
    });
}

pub(crate) fn open_settings() {
    app().update(|s| s.view = AppView::Settings);
}

pub(crate) fn set_settings_tab(tab: SettingsTab) {
    app().update(|s| s.settings_tab = tab);
}

pub(crate) fn toggle_theme() {
    let dark_now = dark();
    set_color_scheme(if dark_now {
        Appearance::Light
    } else {
        Appearance::Dark
    });
    save_prefs();
}

// --- Folders ----------------------------------------------------------------

/// Add folders from a raw string (semicolon/newline separated), as typed
/// into the sidebar box. A file path holding a `;` stays intact because
/// splitting only happens at the input boundary.
pub(crate) fn add_folders(raw: &str) {
    add_folder_paths(raw.split([';', '\n']).map(ToOwned::to_owned).collect());
}

/// Add folder paths (deduplicated, in order). The one funnel for typed
/// input, native dialog picks and OS drops, so all three behave alike.
pub(crate) fn add_folder_paths(paths: Vec<String>) {
    app().update(|s| {
        for raw in paths {
            let p = raw.trim();
            if p.is_empty() || s.folders.iter().any(|f| f.path == p) {
                continue;
            }
            s.folders.push(FolderEntry {
                path: p.to_string(),
                reference: false,
            });
        }
    });
    save_prefs();
}

/// Folder drop from the OS (Explorer, the desktop, ...). Only folders are
/// scan roots — the pipeline walks directories — so a drop of plain files
/// explains itself in a toast instead of doing nothing.
pub(crate) fn drop_folders(uris: &[String]) {
    let folders: Vec<String> = uris
        .iter()
        .filter_map(|uri| crate::ui_helpers::file_url_path(uri))
        .filter(|p| Path::new(p).is_dir())
        .collect();
    if folders.is_empty() {
        crate::ui::toast::notify(
            "Only folders can be scanned — drop a folder onto the window.".to_owned(),
            false,
        );
        return;
    }
    add_folder_paths(folders);
}

/// Escape outside a text input: leave the Settings page when it is open.
pub(crate) fn escape_leaves_settings() {
    app().update(|s| {
        if s.view == AppView::Settings {
            s.view = AppView::Main;
        }
    });
}

pub(crate) fn add_from_input() {
    let raw = app().with_untracked(|s| s.folder_input.trim().to_string());
    add_folders(&raw);
    app().update(|s| s.folder_input.clear());
}

pub(crate) fn remove_folder(index: usize) {
    app().update(|s| {
        if index < s.folders.len() {
            s.folders.remove(index);
        }
    });
    save_prefs();
}

pub(crate) fn toggle_reference(index: usize) {
    app().update(|s| {
        if let Some(f) = s.folders.get_mut(index) {
            f.reference = !f.reference;
        }
    });
    save_prefs();
}

pub(crate) fn browse_folder() {
    let start =
        app().with_untracked(|s| s.folders.last().map(|f| f.path.clone()).unwrap_or_default());
    app_spawn(async move {
        // A blocking native dialog must never run on the UI thread.
        let picked = blocking(move || {
            let mut dlg = rfd::FileDialog::new().set_title("Choose a folder to scan");
            if !start.is_empty() && Path::new(&start).exists() {
                dlg = dlg.set_directory(&start);
            }
            dlg.pick_folder()
        })
        .await;
        if let Ok(Some(path)) = picked {
            add_folders(&path.display().to_string());
        }
    });
}

// --- Preferences ------------------------------------------------------------

/// Snapshot the current sidebar into `dedupe.json`. Best-effort: failures
/// are ignored so a read-only home dir never breaks the GUI.
pub(crate) fn save_prefs() {
    let state = app();
    let out = state.with_untracked(|s| prefs::GuiPrefs {
        version: 1,
        // Off means "don't keep": persist an empty list so stale folders
        // never resurface; the live session list is untouched.
        folders: if s.remember_folders {
            s.folders
                .iter()
                .map(|f| prefs::FolderPref {
                    path: f.path.clone(),
                    reference: f.reference,
                })
                .collect()
        } else {
            Vec::new()
        },
        types: s.types_input.clone(),
        min_size: s.min_size_input.clone(),
        max_size: s.max_size_input.clone(),
        max_depth: s.max_depth_input.clone(),
        exclude_dir: s.exclude_dir_input.clone(),
        exclude_path: s.exclude_path_input.clone(),
        similarity: s.similarity_input.clone(),
        partial_similarity: s.partial_similarity_input.clone(),
        fuzzy_threshold: s.fuzzy_threshold_input.clone(),
        top: s.top_input.clone(),
        ignore: s.ignore_input.clone(),
        jobs: s.jobs_input.clone(),
        hash_index: s.hash_index.unwrap_or(0),
        keep_index: s.keep_index.unwrap_or(0),
        find_index: s.find_index.unwrap_or(0),
        match_index: s.match_index.unwrap_or(0),
        image_hash_index: s.image_hash_index.unwrap_or(0),
        partial_clips: s.partial_clips,
        music_by_content: s.music_by_content,
        exact: s.exact,
        no_cache: s.no_cache,
        trash: s.trash,
        sort_biggest: s.sort_biggest,
        remember_folders: s.remember_folders,
        dark: dark(),
        window: window_state::current(),
    });
    out.save_async();
}

// --- Scan options -----------------------------------------------------------

/// Read the sidebar form into CLI options. This is the GUI equivalent of
/// clap parsing: every field maps 1:1 to a `dedupe` flag.
pub(crate) fn collect_options(s: &AppState) -> Result<cli::Cli, String> {
    let split_multi = |text: &str| {
        text.split([',', ';', '\n'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    };

    let paths: Vec<String> = s.folders.iter().map(|f| f.path.clone()).collect();
    if paths.is_empty() {
        return Err("Add a folder to scan first (type one and Add, or Browse…).".to_string());
    }
    let similarity_raw = s.similarity_input.trim();
    let similarity: f64 = if similarity_raw.is_empty() {
        similar::DEFAULT_SIMILARITY_PCT
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
    let partial_raw = s.partial_similarity_input.trim();
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
    let fuzzy_raw = s.fuzzy_threshold_input.trim();
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
    let top_raw = s.top_input.trim();
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
    let jobs_raw = s.jobs_input.trim();
    let jobs: usize = if jobs_raw.is_empty() {
        0
    } else {
        jobs_raw
            .parse()
            .map_err(|_| "Worker threads must be a whole number (0 = auto).".to_string())?
    };
    let max_depth_raw = s.max_depth_input.trim();
    let max_depth: Option<usize> = if max_depth_raw.is_empty() {
        None
    } else {
        Some(
            max_depth_raw
                .parse()
                .map_err(|_| "Max depth must be a whole number (e.g. 0, 2).".to_string())?,
        )
    };
    let opt = |text: &str| {
        let v = text.trim();
        if is_no_limit(v) {
            None
        } else {
            Some(v.to_string())
        }
    };
    // Validate sizes now so typos surface before the (slow) scan starts.
    // Blank or `unlimited` means unbounded.
    for raw in [&s.min_size_input, &s.max_size_input] {
        if let Err(err) = crate::ui_helpers::parse_size_limit(raw) {
            return Err(format!("Bad size “{raw}”: {err:#}"));
        }
    }
    Ok(cli::Cli {
        paths,
        max_depth,
        types: split_multi(&s.types_input),
        keep_smaller: s.keep_index.unwrap_or(0) == 1,
        keep_newest: s.keep_index.unwrap_or(0) == 2,
        keep_oldest: s.keep_index.unwrap_or(0) == 3,
        keep_best_quality: s.keep_index.unwrap_or(0) == 4,
        consolidate_dir: None,
        reference_dir: s
            .folders
            .iter()
            .filter(|f| f.reference)
            .map(|f| f.path.clone())
            .collect(),
        trash: s.trash,
        dry_run: false,
        delete: false,
        yes: false,
        hash: match s.hash_index.unwrap_or(0) {
            1 => cli::HashAlgo::Sha256,
            2 => cli::HashAlgo::Md5,
            _ => cli::HashAlgo::Blake3,
        },
        exact: s.exact,
        similarity,
        no_cache: s.no_cache,
        clear_cache: false,
        min_size: opt(&s.min_size_input),
        max_size: opt(&s.max_size_input),
        exclude_dir: split_multi(&s.exclude_dir_input),
        exclude_path: split_multi(&s.exclude_path_input),
        json: false,
        verbose: false,
        quiet: true,
        jobs,
        gui: false,
        find: find_mode(s.find_index),
        r#match: match_mode(s.match_index),
        fuzzy_threshold,
        top,
        image_hash: image_hash_algo(s.image_hash_index),
        partial_clips: s.partial_clips,
        partial_similarity,
        music_by: if s.music_by_content {
            cli::MusicMode::Content
        } else {
            cli::MusicMode::Tags
        },
        csv: None,
        html: None,
        move_dir: None,
        hardlink: false,
        strip_exif: false,
        ignore: split_multi(&s.ignore_input),
    })
}

// --- Scanning ---------------------------------------------------------------

/// True while `generation` is the latest scan — false once a Stop or a newer
/// scan has superseded the task that captured it.
fn is_current_scan(generation: u64) -> bool {
    app().with_untracked(|s| s.scan_gen == generation)
}

/// Spawn a task under a detached owner.
///
/// `spawn` ends its task with the owner it was made under, and a press
/// handler's owner is the view branch the handler's own state change
/// collapses (press Scan → `scanning = true` collapses the Scan branch).
/// Tasks born there must therefore outlive that branch: a detached owner
/// keeps them running for the app's life; the generation guards and
/// cancellation tokens bound each one to its run.
pub(crate) fn app_spawn<T: 'static>(future: impl Future<Output = T> + 'static) {
    Owner::detached().run(|| spawn(future));
}

pub(crate) fn start_scan() {
    save_prefs();
    let state = app();
    let opts = match state.with_untracked(collect_options) {
        Ok(o) => o,
        Err(msg) => {
            state.update(|s| s.status = msg);
            return;
        }
    };
    // New generation: invalidates any in-flight (stopped) scan task, and a
    // fresh cancellation token for this run — Stop sets it and the pipeline
    // polls it at every phase boundary.
    state.update(|s| {
        s.last_hash = opts.hash;
        s.scan_similarity = (opts.similarity / 100.0).clamp(0.0, 1.0);
        s.scanning = true;
        s.scan_gen += 1;
        s.scan_cancel.reset();
        s.scan_done = 0;
        s.scan_total = 0;
        s.scan_determinate = false;
        s.has_scanned = false;
        s.groups.clear();
        s.mode_result = None;
        s.selected.clear();
        s.expanded.clear();
        s.status = pipeline::phase_label(pipeline::ScanPhase::Scanning).to_string();
    });
    let generation = state.with_untracked(|s| s.scan_gen);
    let cancel = state.with_untracked(|s| s.scan_cancel.clone());
    // Progress crosses threads through shared state; a timer polls it into
    // the view. The bars are headless indicatif meters — polled for
    // position/length, never drawn — so the pipeline needs no GUI-specific
    // progress plumbing.
    let phase: Arc<Mutex<pipeline::ScanPhase>> =
        Arc::new(Mutex::new(pipeline::ScanPhase::Scanning));
    let phase_write = phase.clone();
    let hash_bar = indicatif::ProgressBar::hidden();
    let sim_bar = indicatif::ProgressBar::hidden();
    let hash_bar_poll = hash_bar.clone();
    let sim_bar_poll = sim_bar.clone();
    app_spawn(async move {
        let out = blocking(move || {
            pipeline::run_scan(
                &opts,
                Some(&hash_bar),
                Some(&sim_bar),
                &|p| {
                    if let Ok(mut slot) = phase_write.lock() {
                        *slot = p;
                    }
                },
                &cancel,
            )
        })
        .await;
        if !is_current_scan(generation) {
            return;
        }
        let out = match out {
            Ok(result) => result,
            Err(e) => Err(anyhow::anyhow!("scan worker stopped: {e}")),
        };
        match out {
            Ok(output) => apply_output(generation, output),
            Err(e) => {
                let message = format!("Scan failed: {e:#}");
                app().update(|s| {
                    s.scanning = false;
                    s.status = message;
                });
            }
        }
    });
    app_spawn(async move {
        loop {
            timer(Duration::from_millis(200)).await;
            if !app().with_untracked(|s| s.scanning) {
                break;
            }
            let ph = phase
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
            app().update(|s| {
                s.status = label;
                s.scan_done = done;
                s.scan_total = total;
                s.scan_determinate = determinate && total > 0;
            });
        }
    });
}

/// Stop the running scan: arm the cancellation token AND reset the view
/// immediately, so the UI never sits in "Stopping…" while a large file
/// finishes hashing. The pipeline winds down in the background and its late
/// output is discarded by the generation guard.
pub(crate) fn stop_scan() {
    app().update(|s| {
        s.scan_cancel.cancel();
        s.scan_gen += 1;
        s.scanning = false;
        s.scan_done = 0;
        s.scan_total = 0;
        s.scan_determinate = false;
        s.has_scanned = false;
        s.groups.clear();
        s.mode_result = None;
        s.base_groups.clear();
        s.selected.clear();
        s.expanded.clear();
        s.last_stats = None;
        s.status = "Scan cancelled.".to_string();
    });
}

fn apply_output(generation: u64, out: pipeline::ScanOutput) {
    let state = app();
    if !is_current_scan(generation) {
        return;
    }
    state.update(|s| {
        s.scanning = false;
        s.scan_done = 0;
        s.scan_total = 0;
        s.scan_determinate = false;
        // A cancelled scan carries no groups by design: reset to the
        // pre-scan state (the walk cleared the pane at start anyway) and
        // report the stop. `has_scanned` stays false so the 3-step empty
        // state remains up.
        if out.cancelled {
            s.groups.clear();
            s.mode_result = None;
            s.base_groups.clear();
            s.selected.clear();
            s.expanded.clear();
            s.has_scanned = false;
            s.files_scanned = out.stats.files_scanned;
            s.bytes_scanned = out.stats.bytes_scanned;
            s.last_stats = None;
            s.status = format!(
                "Scan cancelled after {} file(s). Press Scan to run again.",
                out.stats.files_scanned
            );
            return;
        }
        s.has_scanned = true;
        s.files_scanned = out.stats.files_scanned;
        s.bytes_scanned = out.stats.bytes_scanned;
        s.ffprobe_note = out.needs_ffprobe_note;
        s.last_stats = Some(out.stats);
        s.last_paths = out.paths.clone();
        s.last_hash_algo = out.hash_algo.to_string();
        s.last_ffprobe = out.ffprobe_used;
        s.last_ref_dirs = out.reference_dirs;
        if let Some(mode) = out.mode_result {
            let title = crate::modes::mode_title(mode.mode);
            let findings = mode.finding_count();
            let group_count = mode.groups.len();
            let reclaim = mode.reclaimable();
            let empty = mode.is_empty();
            s.mode_result = Some(mode);
            s.groups.clear();
            s.base_groups.clear();
            s.selected.clear();
            s.expanded.clear();
            s.consolidate_dest = None;
            s.group_limit = crate::ui::state::GROUP_PAGE;
            s.dup_files = findings as u64;
            s.reclaim_bytes = reclaim;
            s.status = if empty {
                format!(
                    "{title}: nothing found in {} file(s) ({}).",
                    s.files_scanned,
                    util::human_bytes(s.bytes_scanned)
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
            crate::ui::toast::notify(s.status.clone(), false);
            s.posters.clear();
            s.strips.clear();
            s.preview_loading.clear();
            return;
        }
        s.mode_result = None;
        s.groups = out.groups;
        // Pristine copy for live re-thresholding; the floor starts at the
        // scan threshold (everything qualifies).
        s.base_groups = s.groups.clone();
        // Fresh results: disarm any pending consolidate from last scan.
        s.consolidate_dest = None;
        s.min_sim = s.scan_similarity;
        s.group_limit = crate::ui::state::GROUP_PAGE;
        // One display ordering for the whole scan (honors the sort toggle).
        sort_groups(s);
        // Pre-select every duplicate for deletion (KEEP members are never
        // selectable); the user unchecks what should survive.
        s.selected = s
            .groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| !m.keep)
            .map(|m| m.path.clone())
            .collect();
        // Expand the first few groups; huge result sets stay navigable.
        s.expanded = s.groups.iter().take(10).map(|g| g.index).collect();
        recount(s);
        if s.groups.is_empty() {
            s.status = format!(
                "No duplicates — scanned {} file(s) ({}).",
                s.files_scanned,
                util::human_bytes(s.bytes_scanned)
            );
        } else {
            s.status = format!(
                "{} group(s) · {} duplicate file(s) · {} reclaimable.",
                s.groups.len(),
                s.dup_files,
                util::human_bytes(s.reclaim_bytes)
            );
        }
        for e in &out.cache_save_errors {
            s.status.push_str(&format!(" (cache write failed: {e})"));
        }
        crate::ui::toast::notify(s.status.clone(), false);
        // Preview strips are per-scan too: paths are gone, so drop frames
        // and in-flight flags.
        s.posters.clear();
        s.strips.clear();
        s.preview_loading.clear();
    });
    // Generate video posters off the UI thread; each completion re-renders
    // its row's thumbnail.
    let jobs: Vec<(PathBuf, u64, Option<i64>)> = state.with_untracked(|s| {
        s.groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| media::classify(&m.path) == media::MediaKind::Video)
            .take(120)
            .map(|m| (m.path.clone(), m.size, m.mtime_secs))
            .collect()
    });
    if jobs.is_empty() {
        return;
    }
    app_spawn(async move {
        for (path, size, mtime) in jobs {
            let done =
                blocking(move || poster::video_poster(&path, size, mtime).map(|p| (path, p)))
                    .await
                    .ok()
                    .flatten();
            if let Some((orig, thumb)) = done {
                app().update(|s| {
                    s.posters.insert(orig, thumb);
                });
            }
        }
    });
}

// --- Selection and ordering -------------------------------------------------

/// Mark every non-keep member for deletion (fresh intent after a
/// keeper/mode change).
pub(crate) fn reselect_all_dups(s: &mut AppState) {
    s.selected = s
        .groups
        .iter()
        .flat_map(|g| g.members.iter())
        .filter(|m| !m.keep)
        .map(|m| m.path.clone())
        .collect();
}

/// Recompute the summary counters from the current groups.
pub(crate) fn recount(s: &mut AppState) {
    let dups: u64 = s
        .groups
        .iter()
        .map(|g| (g.members.len() - g.keep_count()) as u64)
        .sum();
    s.dup_files = dups;
    s.reclaim_bytes = s.groups.iter().map(|g| g.dup_bytes()).sum();
}

/// Order `groups` for display: biggest-reclaimable-first while the sort
/// toggle is on, otherwise the scan's discovery order (via `base_groups`).
/// Stable sort, so ties never jitter.
///
/// Called ONLY on explicit order-changing events (new scan, sort toggle,
/// similarity filter, global keep mode) — never on keeper picks or renders,
/// so clicking a card cannot reshuffle the list underneath.
pub(crate) fn sort_groups(s: &mut AppState) {
    if s.sort_biggest {
        s.groups.sort_by_key(|b| std::cmp::Reverse(b.dup_bytes()));
    } else {
        let pos: HashMap<usize, usize> = s
            .base_groups
            .iter()
            .enumerate()
            .map(|(i, g)| (g.index, i))
            .collect();
        s.groups
            .sort_by_key(|g| pos.get(&g.index).copied().unwrap_or(usize::MAX));
    }
}

/// Mirror keep flags and member order from the live view back into the
/// pristine scan results, so re-thresholding never loses keeper choices.
/// Members hidden by the floor keep their stored flags.
pub(crate) fn sync_keep_to_base(s: &mut AppState) {
    for g in &s.groups {
        if let Some(base) = s.base_groups.iter_mut().find(|b| b.index == g.index) {
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

/// Re-derive the live groups from the pristine scan results at floor `t`
/// (fraction). Exact groups always qualify; similar members need
/// similarity-to-keeper >= `t`. Groups left with fewer than 2 members drop
/// out (restorable by lowering the floor — no rescan).
pub(crate) fn apply_threshold(s: &mut AppState, t: f64) {
    let t = t.clamp(s.scan_similarity, 1.0);
    s.min_sim = t;
    s.groups = s
        .base_groups
        .iter()
        .filter_map(|g| {
            let members: Vec<_> = g
                .members
                .iter()
                .filter(|m| m.similarity.is_none_or(|sim| sim >= t))
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
    s.group_limit = crate::ui::state::GROUP_PAGE;
    reselect_all_dups(s);
    recount(s);
    // Fresh member set: re-apply the display order once, here.
    sort_groups(s);
    s.status = format!(
        "Similarity ≥ {:.0}% — {} group(s), {} duplicate file(s).",
        t * 100.0,
        s.groups.len(),
        s.dup_files,
    );
}

/// Parse the live floor box: blank restores the scan floor, a number
/// tightens it, anything below the scan floor is clamped (loosening needs
/// discarded pairs — rescan instead).
pub(crate) fn apply_sim_input(raw: &str) {
    app().update(|s| {
        let raw = raw.trim();
        if raw.is_empty() {
            if (s.min_sim - s.scan_similarity).abs() > f64::EPSILON {
                let floor = s.scan_similarity;
                apply_threshold(s, floor);
            }
            return;
        }
        match raw.parse::<f64>() {
            Ok(v) if (0.0..=100.0).contains(&v) => {
                let t = v / 100.0;
                if t < s.scan_similarity {
                    s.status = format!(
                        "Floor {:.0}% is below the scan threshold {:.0}% — rescan to loosen.",
                        v,
                        s.scan_similarity * 100.0
                    );
                    return;
                }
                apply_threshold(s, t);
            }
            _ => {}
        }
    });
}

/// Promote one member to keeper of its group (per-group decision).
/// Member order never changes here: the grid keeps every card in place and
/// only the highlight (KEEP/DUP badge + border) moves.
pub(crate) fn set_keeper(group_index: usize, path: &Path) {
    app().update(|s| {
        if let Some(g) = s.groups.iter_mut().find(|g| g.index == group_index) {
            for m in g.members.iter_mut() {
                m.keep = m.path == path;
            }
        }
        reselect_all_dups(s);
        sync_keep_to_base(s);
        // Keeper swap changes which bytes count as reclaimable.
        recount(s);
        // Keeper change: disarm a pending consolidate (stale paths).
        s.consolidate_dest = None;
    });
}

/// Apply a new global keep mode to the current results (no rescan).
pub(crate) fn apply_keep_mode(index: usize) {
    app().update(|s| {
        s.keep_index = Some(index);
        let keep = match index {
            1 => crate::matching::KeepMode::Smallest,
            2 => crate::matching::KeepMode::Newest,
            3 => crate::matching::KeepMode::Oldest,
            4 => crate::matching::KeepMode::BestQuality,
            _ => crate::matching::KeepMode::First,
        };
        crate::matching::reassign_keepers(&mut s.groups, keep);
        reselect_all_dups(s);
        // New keepers everywhere: disarm a pending consolidate (stale paths).
        s.consolidate_dest = None;
        let name = [
            "First found",
            "Smallest",
            "Newest",
            "Oldest",
            "Best quality",
        ][index.min(4)];
        s.status = format!("Keep mode: {name} applied to {} group(s).", s.groups.len());
        sync_keep_to_base(s);
        // New keepers everywhere: re-apply the display order once, here.
        sort_groups(s);
    });
    save_prefs();
}

/// Flip one duplicate's tick (KEEP members are never selectable).
pub(crate) fn toggle_selected(path: &Path) {
    app().update(|s| {
        let is_dup = s
            .groups
            .iter()
            .flat_map(|g| g.members.iter())
            .any(|m| m.path == path && !m.keep);
        if !is_dup {
            return;
        }
        if !s.selected.remove(path) {
            s.selected.insert(path.to_path_buf());
        }
    });
}

pub(crate) fn toggle_sort_biggest() {
    app().update(|s| {
        s.sort_biggest = !s.sort_biggest;
        sort_groups(s);
    });
    save_prefs();
}

pub(crate) fn show_more_groups() {
    app().update(|s| s.group_limit += crate::ui::state::GROUP_PAGE);
}

pub(crate) fn toggle_expanded(index: usize) {
    app().update(|s| {
        if !s.expanded.remove(&index) {
            s.expanded.insert(index);
        }
    });
}

/// Open or close every group currently rendered.
pub(crate) fn set_all_expanded(open: bool) {
    app().update(|s| {
        s.expanded.clear();
        if open {
            s.expanded
                .extend(s.groups.iter().take(s.group_limit).map(|g| g.index));
        }
    });
}

/// Clear the delete selection (KEEP members are never selected anyway).
pub(crate) fn select_none() {
    app().update(|s| s.selected.clear());
}

// --- Deletion ---------------------------------------------------------------

/// Files currently marked for deletion (selected checkboxes that are still
/// non-keep members of a group).
pub(crate) fn pending_targets(s: &AppState) -> Vec<file_actions::DeleteTarget> {
    let mut out = Vec::new();
    for g in &s.groups {
        for m in &g.members {
            if !m.keep && s.selected.contains(&m.path) {
                out.push(file_actions::DeleteTarget {
                    path: m.path.clone(),
                    expected_hash: m.content_hash.clone().unwrap_or_else(|| g.hash.clone()),
                    size: m.size,
                });
            }
        }
    }
    out
}

pub(crate) fn perform_delete() {
    let state = app();
    let targets = state.with_untracked(pending_targets);
    if targets.is_empty() {
        state.update(|s| {
            s.status = "Tick the duplicate copies you want deleted first.".to_string();
        });
        return;
    }
    let trash = state.with_untracked(|s| s.trash);
    let hash = state.with_untracked(|s| s.last_hash);
    state.update(|s| s.status = "Deleting…".to_string());
    app_spawn(async move {
        let outcome = blocking(move || {
            let engine = hashing::HashEngine::new(hash);
            let disposition = if trash {
                file_actions::Disposition::Trash
            } else {
                file_actions::Disposition::Permanent
            };
            file_actions::delete_targets(&targets, &engine, disposition)
        })
        .await;
        let Ok(outcome) = outcome else {
            app().update(|s| s.status = "Delete failed: worker stopped.".to_string());
            return;
        };
        app().update(|s| {
            let gone: HashSet<PathBuf> = outcome.deleted.iter().cloned().collect();
            for g in &mut s.groups {
                g.members.retain(|m| !gone.contains(&m.path));
            }
            s.groups.retain(|g| g.members.len() >= 2);
            // Prune the pristine copy too so a later floor change cannot
            // resurrect deleted files.
            for g in &mut s.base_groups {
                g.members.retain(|m| !gone.contains(&m.path));
            }
            s.base_groups.retain(|g| g.members.len() >= 2);
            for p in &outcome.deleted {
                s.selected.remove(p);
            }
            // Deleted paths are gone: disarm a pending consolidate.
            s.consolidate_dest = None;
            let verb = if trash { "Moved to trash" } else { "Deleted" };
            // Trash receipts enable one-step undo; permanent deletes cannot
            // be undone, so any stale receipt is dropped.
            if trash {
                s.last_trashed = outcome.deleted.clone();
            } else {
                s.last_trashed.clear();
            }
            recount(s);
            s.status = format!(
                "{verb} {} file(s) ({} freed); {} skipped.",
                outcome.deleted.len(),
                util::human_bytes(outcome.bytes_freed),
                outcome.skipped.len()
            );
            crate::ui::toast::notify(s.status.clone(), false);
        });
    });
}

/// Restore the last trash run (undo). Permanent deletes have no receipt and
/// cannot be restored. Restored files reappear on disk; press Scan to
/// refresh the groups.
pub(crate) fn undo_delete() {
    let state = app();
    let receipt = state.with_untracked(|s| s.last_trashed.clone());
    if receipt.is_empty() {
        state.update(|s| {
            s.status = "Nothing to restore — nothing has been deleted yet.".to_string();
        });
        return;
    }
    // `trash::os_limited` (list/restore) only exists on Windows and
    // Freedesktop-trash Unix — there is no public macOS API for it, so the
    // module is cfg-gated out there. Mirror the crate's gate here.
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
        app_spawn(async move {
            let wanted: HashSet<PathBuf> = receipt.into_iter().collect();
            let restored = blocking(move || {
                let items = trash::os_limited::list().map_err(|e| e.to_string())?;
                let mine: Vec<_> = items
                    .into_iter()
                    .filter(|it| wanted.contains(&it.original_path()))
                    .collect();
                if mine.is_empty() {
                    return Ok::<usize, String>(0);
                }
                let n = mine.len();
                trash::os_limited::restore_all(mine).map_err(|e| e.to_string())?;
                Ok(n)
            })
            .await;
            let restored = match restored {
                Ok(result) => result,
                Err(e) => Err(format!("restore worker stopped: {e}")),
            };
            app().update(|s| match restored {
                Ok(0) => {
                    s.status = "Nothing to restore — the trash was already emptied.".to_string();
                    s.last_trashed.clear();
                }
                Ok(n) => {
                    s.status = format!("Restored {n} file(s). Press Scan to refresh.");
                    s.last_trashed.clear();
                }
                Err(e) => {
                    s.status = format!("Restore failed: {e}");
                }
            });
        });
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
        state.update(|s| {
            s.last_trashed.clear();
            s.status =
                "Undo is not supported on this platform (trash restore needs Windows/Linux)."
                    .to_string();
        });
    }
}

// --- Consolidate ------------------------------------------------------------

/// First click of the two-click consolidate: pick the destination folder,
/// then preview the planned keeper moves in the status line. The second
/// click (same button) executes via [`perform_consolidate`].
pub(crate) fn pick_consolidate_dir() {
    let start =
        app().with_untracked(|s| s.folders.last().map(|f| f.path.clone()).unwrap_or_default());
    app_spawn(async move {
        let picked = blocking(move || {
            let mut dlg = rfd::FileDialog::new().set_title("Choose the consolidate folder");
            if !start.is_empty() && Path::new(&start).exists() {
                dlg = dlg.set_directory(&start);
            }
            dlg.pick_folder()
        })
        .await;
        let Ok(Some(dest)) = picked else { return };
        app().update(|s| {
            let (moves, skipped) = file_actions::plan_consolidation(&s.groups, &dest);
            if moves.is_empty() {
                s.consolidate_dest = None;
                s.status =
                    "Nothing to move — every keeper is protected or already in that folder."
                        .to_string();
            } else {
                let bytes: u64 = moves.iter().map(|m| m.target.size).sum();
                s.consolidate_dest = Some(dest.clone());
                s.status = format!(
                    "Consolidate {} keeper(s) ({}) into {}? ({} skipped) Press “Move keepers” again to move them.",
                    moves.len(),
                    util::human_bytes(bytes),
                    dest.display(),
                    skipped.len(),
                );
            }
        });
    });
}

/// Second click: move every group's keeper into the armed destination. The
/// plan is rebuilt live so retries after partial moves stay correct;
/// keepers are re-hashed before moving and member paths plus ticks follow
/// moved files.
pub(crate) fn perform_consolidate() {
    let state = app();
    let Some(dest) = state.with_untracked(|s| s.consolidate_dest.clone()) else {
        state.update(|s| {
            s.status = "Choose the folder that keepers should be moved into.".to_string();
        });
        return;
    };
    let (targets, skipped) = state.with_untracked(|s| {
        let (moves, skipped) = file_actions::plan_consolidation(&s.groups, &dest);
        (
            moves.iter().map(|m| m.target.clone()).collect::<Vec<_>>(),
            skipped.len(),
        )
    });
    if targets.is_empty() {
        state.update(|s| {
            s.status = "Nothing to move — every keeper is protected or already in that folder."
                .to_string();
            s.consolidate_dest = None;
        });
        return;
    }
    let hash = state.with_untracked(|s| s.last_hash);
    state.update(|s| s.status = "Moving keepers…".to_string());
    app_spawn(async move {
        let outcome = blocking(move || {
            let engine = hashing::HashEngine::new(hash);
            file_actions::consolidate_targets(&targets, &engine)
        })
        .await;
        let Ok(outcome) = outcome else {
            app().update(|s| s.status = "Consolidate failed: worker stopped.".to_string());
            return;
        };
        app().update(|s| {
            for (from, to) in &outcome.moved {
                for groups in [&mut s.groups, &mut s.base_groups] {
                    for m in groups.iter_mut().flat_map(|g| g.members.iter_mut()) {
                        if m.path == *from {
                            m.path = to.clone();
                        }
                    }
                }
                // Ticks follow moved files.
                if s.selected.remove(from) {
                    s.selected.insert(to.clone());
                }
            }
            s.consolidate_dest = None;
            s.status = format!(
                "Consolidated {} keeper(s) ({}) into {}; {} skipped.",
                outcome.moved.len(),
                util::human_bytes(outcome.bytes_moved),
                dest.display(),
                outcome.skipped.len() + skipped,
            );
        });
    });
}

// --- Report -----------------------------------------------------------------

/// Write the current results as JSON (same schema as `--json`).
pub(crate) fn save_report() {
    let state = app();
    let Some(stats) = state.with_untracked(|s| s.last_stats.clone()) else {
        state.update(|s| s.status = "Nothing to save yet — run a scan first.".to_string());
        return;
    };
    // Snapshot everything the async task needs; the view may change (or
    // close) while the native dialog is open.
    let snapshot = state.with_untracked(|s| ReportSnapshot {
        paths: s.last_paths.clone(),
        hash_algo: s.last_hash_algo.clone(),
        ffprobe_used: s.last_ffprobe,
        stats,
        groups: s.groups.clone(),
    });
    app_spawn(async move {
        let file = blocking(move || {
            rfd::FileDialog::new()
                .set_title("Save duplicate report")
                .set_file_name("dedupe-report.json")
                .save_file()
        })
        .await;
        let Ok(Some(path)) = file else { return };
        let json = {
            let ctx = crate::report::ReportContext {
                paths: &snapshot.paths,
                hash_algo: &snapshot.hash_algo,
                verbose: false,
                ffprobe_used: snapshot.ffprobe_used,
            };
            crate::report::to_json(&ctx, &snapshot.stats, &snapshot.groups)
        };
        let outcome = std::fs::write(&path, json);
        app().update(|s| match outcome {
            Ok(()) => {
                s.status = format!("Saved report to {}.", path.display());
                crate::ui::toast::notify(s.status.clone(), false);
            }
            Err(e) => {
                s.status = format!("Could not save report: {e}");
                crate::ui::toast::notify(s.status.clone(), true);
            }
        });
    });
}

// --- Cache ------------------------------------------------------------------

/// Wipe the on-disk hash and fingerprint caches from the Settings page.
pub(crate) fn clear_cache() {
    let (files, bytes) = crate::cache::clear_caches();
    let msg = format!("Cleared {files} file(s) ({})", util::human_bytes(bytes));
    app().update(|s| {
        s.cache_msg = msg.clone();
        s.status = format!("Cache cleared: {msg}");
        crate::ui::toast::notify(s.status.clone(), false);
    });
}
