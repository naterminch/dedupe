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
use crate::history::{self, Method, RunKind};
use crate::matching::{GroupMember, KeepReason};
use crate::ui::state::{
    AppState, AppView, Cleared, FindingsDone, FindingsWent, FolderEntry, Removal, ReportSnapshot,
    RestoreAsk, ScanStatus, SettingsTab, Went, app, scan_status,
};
use crate::ui::window_state;
use crate::ui_helpers::{bar_pos, counted, find_mode, image_hash_algo, is_no_limit, match_mode};
use crate::{cli, hashing, media, pipeline, poster, prefs, similar, util};

// --- Boot -------------------------------------------------------------------

/// Set once per process: components re-run, bootstrapping must not.
static BOOTSTRAPPED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Restore preferences, theme and the ffmpeg probe, and register the bundled
/// faces. Runs once, on the first frame, after the component tree exists.
pub(crate) fn bootstrap() {
    if BOOTSTRAPPED.set(()).is_err() {
        return;
    }
    // Text naming a family is laid out again once its face lands, so the
    // first frame may show the system face for a moment; a face the
    // platform refuses leaves its text on the system one.
    for face in crate::ui::widgets::FACES {
        let answer = register_font(face);
        app_spawn(async move {
            if let Err(err) = answer.await {
                eprintln!("dedupe: a bundled font was refused: {err}");
            }
        });
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
    // The history file can be long; read it off the first paint too.
    app_spawn(async move {
        if let Ok(runs) = blocking(history::load).await {
            app().update(|s| s.history = Arc::new(runs));
        }
    });
}

// --- Navigation -------------------------------------------------------------

/// The pages swap whole panes: every list mounts at its top again, so the
/// wheel's remembered offsets no longer describe what is on screen.
fn forget_pane_offsets() {
    crate::ui::results::reset_lists();
    crate::ui::shell::reset_scroll();
    crate::ui::settings::reset_scroll();
    crate::ui::history::reset_scroll();
}

/// Show `page`. Only a page switch remounts the panes: pressing the rail item
/// of the page already open leaves it mounted, so forgetting the offsets
/// there would send the next wheel notch back to the top.
fn open_page(page: AppView) {
    if app().with_untracked(|s| s.view == page) {
        return;
    }
    app().update(|s| s.view = page);
    forget_pane_offsets();
}

pub(crate) fn open_workspace() {
    open_page(AppView::Main);
}

/// Open the History page, reading the history again on the way: a
/// command-line run since start-up shows up too. The list only changes when
/// the file says something new.
pub(crate) fn open_history() {
    open_page(AppView::History);
    app_spawn(async move {
        let Ok(runs) = blocking(history::load).await else {
            return;
        };
        app().update(|s| {
            if *s.history != runs {
                s.history = Arc::new(runs);
            }
        });
    });
}

pub(crate) fn open_settings() {
    open_page(AppView::Settings);
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

/// Put every persisted setting back to its shipped default: scan type,
/// matching and thresholds, filters, toggles and the theme.
///
/// Session data is deliberately left alone — the folder list, results and the
/// open page stay as they are, so a reset never costs a scan.
pub(crate) fn reset_settings() {
    let defaults = prefs::GuiPrefs::default();
    set_color_scheme(if defaults.dark {
        Appearance::Dark
    } else {
        Appearance::Light
    });
    app().update(|s| {
        // apply_prefs seeds both similarity boxes from the default, so the
        // reset leaves the two boxes showing one value.
        s.apply_prefs(&defaults);
    });
    // Bring on-screen results in step with the restored threshold (skipped
    // mid-scan: the running job owns the view until it finishes).
    if !app().with(|s| s.scanning) {
        apply_sim_input(&defaults.similarity);
    }
    save_prefs();
    crate::ui::toast::notify("Settings reset to defaults.".to_owned(), false);
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

/// Escape outside a text input: close the open confirmation (delete,
/// consolidate, the findings' act or a restore), otherwise leave the History
/// or Settings page for the workspace.
pub(crate) fn escape_leaves_page() {
    let confirming = app().with_untracked(|s| {
        s.confirm_delete || s.consolidating || s.confirm_findings || s.restore.is_some()
    });
    if !confirming {
        open_workspace();
        return;
    }
    app().update(|s| {
        if s.confirm_delete {
            s.confirm_delete = false;
        } else if s.confirm_findings {
            s.confirm_findings = false;
        } else if s.restore.is_some() {
            s.restore = None;
        } else {
            s.consolidating = false;
            s.consolidate_dest = None;
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

/// The sidebar's filtered list addresses folders by path: resolve it to the
/// live index, then reuse the index-based actions.
pub(crate) fn remove_folder_path(path: &str) {
    if let Some(index) = folder_index(path) {
        remove_folder(index);
    }
}

/// Empty the folder list in one go: after a batch of added or dropped
/// folders, starting over should not mean removing them one by one.
pub(crate) fn remove_all_folders() {
    app().update(|s| {
        s.folders.clear();
        s.folder_filter = None;
    });
    save_prefs();
}

pub(crate) fn toggle_reference_path(path: &str) {
    if let Some(index) = folder_index(path) {
        toggle_reference(index);
    }
}

fn folder_index(path: &str) -> Option<usize> {
    app().with_untracked(|s| s.folders.iter().position(|f| f.path == path))
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
        removal_index: s.removal.index(),
        sort_biggest: s.sort_biggest,
        remember_folders: s.remember_folders,
        wheel_page: s.wheel_page,
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
        history: false,
        restore: None,
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
    // One scan at a time: the running job owns the view's progress, results
    // and cancellation token, so a second start would silently discard the
    // first one's output. The sidebar row is disabled while a scan runs; this
    // guard also covers the keyboard and any click that lands mid-transition.
    if app().with_untracked(|s| s.scanning) {
        return;
    }
    // A fresh run re-reads the files it meets, so the memoized probe results
    // of the last run must not answer for them.
    crate::media::clear_probe_cache();
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
        // Both boxes show the threshold this run actually uses — including
        // the Settings box when it was blank and the default kicked in.
        let sim_text = format_similarity_pct(opts.similarity);
        s.similarity_input = sim_text.clone();
        s.min_sim_input = sim_text;
        s.scanning = true;
        s.scan_gen += 1;
        s.scan_cancel.reset();
        s.has_scanned = false;
        // The status bar reports these while a scan runs: zeroed here so a
        // rerun never opens showing the previous run's totals.
        s.files_scanned = 0;
        s.bytes_scanned = 0;
        s.groups.clear();
        s.mode_result = None;
        s.selected.clear();
        s.collapsed.clear();
        s.playing = None;
        s.status = pipeline::phase_label(pipeline::ScanPhase::Scanning).to_string();
    });
    // Fresh progress for this run, in its own signal: the app signal must not
    // change on a tick, or every pane that reads it is rebuilt mid-scan.
    scan_status().update(|p| {
        *p = ScanStatus {
            phase: pipeline::phase_label(pipeline::ScanPhase::Scanning).to_string(),
            ..ScanStatus::default()
        };
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
            let mut out = pipeline::run_scan(
                &opts,
                Some(&hash_bar),
                Some(&sim_bar),
                &|p| {
                    if let Ok(mut slot) = phase_write.lock() {
                        *slot = p;
                    }
                },
                &cancel,
            );
            // The cleanup workspace can act on its findings, and acting has
            // to prove each one unchanged: record what they hold now, as the
            // CLI does for `--delete`. Report-only modes skip the cost.
            if let Ok(scan) = out.as_mut()
                && let Some(result) = scan.mode_result.as_mut()
                && findings_act(result.mode) != FindingsAct::ReportOnly
            {
                if let Ok(mut slot) = phase_write.lock() {
                    *slot = pipeline::ScanPhase::Finishing;
                }
                let engine = hashing::HashEngine::new(opts.hash);
                crate::modes::attach_content_hashes(result, &engine, &cancel);
            }
            out
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
            let (done, total, determinate) = match ph {
                pipeline::ScanPhase::Scanning | pipeline::ScanPhase::Finishing => (0, 0, false),
                pipeline::ScanPhase::Hashing => {
                    let (d, t) = bar_pos(&hash_bar_poll);
                    (d, t, true)
                }
                pipeline::ScanPhase::ComparingMedia => {
                    let (d, t) = bar_pos(&sim_bar_poll);
                    (d, t, true)
                }
            };
            scan_status().update(|p| {
                *p = ScanStatus {
                    phase: pipeline::phase_label(ph).to_string(),
                    done,
                    total,
                    determinate: determinate && total > 0,
                };
            });
        }
    });
}

/// Stop the running scan: arm the cancellation token AND reset the view
/// immediately, so the UI never sits in "Stopping…" while a large file
/// finishes hashing. The pipeline winds down in the background and its late
/// output is discarded by the generation guard.
pub(crate) fn stop_scan() {
    scan_status().update(|p| *p = ScanStatus::default());
    app().update(|s| {
        s.scan_cancel.cancel();
        s.scan_gen += 1;
        s.scanning = false;
        s.has_scanned = false;
        s.groups.clear();
        s.mode_result = None;
        s.base_groups.clear();
        s.selected.clear();
        s.collapsed.clear();
        s.playing = None;
        s.last_stats = None;
        s.status = "Scan cancelled.".to_string();
    });
}

/// Leave a finished scan for the setup page, where the scan form sets up the
/// next run. Only the results go: the folders and the
/// prefs stay, so going back costs nothing.
pub(crate) fn back_to_setup() {
    if app().with_untracked(|s| s.scanning) {
        return;
    }
    scan_status().update(|p| *p = ScanStatus::default());
    crate::ui::results::reset_lists();
    app().update(|s| {
        // Everything the last run produced goes, not just the lists: the
        // status bar's counts, the reclaim totals and the per-run media caches
        // would otherwise keep describing a scan whose results are gone. The
        // generation bump is what makes clearing the media caches safe: a
        // poster still being extracted for this run is dropped when it lands
        // instead of being filed under a path the next run will show.
        s.scan_gen += 1;
        s.has_scanned = false;
        s.groups.clear();
        s.base_groups.clear();
        s.mode_result = None;
        s.selected.clear();
        s.cleared = None;
        s.findings_done = None;
        s.collapsed.clear();
        s.playing = None;
        s.group_limit = crate::ui::state::GROUP_PAGE;
        s.results_rev += 1;
        s.files_scanned = 0;
        s.bytes_scanned = 0;
        s.dup_files = 0;
        s.reclaim_bytes = 0;
        s.last_stats = None;
        s.last_paths.clear();
        s.last_ffprobe = false;
        s.last_ref_dirs.clear();
        s.ffprobe_note = false;
        s.posters.clear();
        s.strips.clear();
        s.poster_jobs.clear();
        s.status = "Choose a folder and press Scan.".to_string();
    });
}

fn apply_output(generation: u64, out: pipeline::ScanOutput) {
    let state = app();
    if !is_current_scan(generation) {
        return;
    }
    scan_status().update(|p| *p = ScanStatus::default());
    state.update(|s| {
        s.scanning = false;
        // A cancelled scan carries no groups by design: reset to the
        // pre-scan state (the walk cleared the pane at start anyway) and
        // report the stop. `has_scanned` stays false so the 3-step empty
        // state remains up.
        if out.cancelled {
            s.groups.clear();
            s.mode_result = None;
            s.base_groups.clear();
            s.selected.clear();
            s.collapsed.clear();
            s.playing = None;
            s.has_scanned = false;
            s.files_scanned = out.stats.files_scanned;
            s.bytes_scanned = out.stats.bytes_scanned;
            s.last_stats = None;
            s.status = format!(
                "Scan cancelled after {}. Press Scan to run again.",
                counted(out.stats.files_scanned as u64, "file", "files")
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
        s.confirm_findings = false;
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
            s.cleared = None;
            s.findings_done = None;
            s.collapsed.clear();
            s.playing = None;
            s.consolidate_dest = None;
            s.consolidating = false;
            s.group_limit = crate::ui::state::GROUP_PAGE;
            s.mode_limit = crate::ui::state::GROUP_PAGE;
            s.dup_files = findings as u64;
            s.reclaim_bytes = reclaim;
            s.status = if empty {
                format!(
                    "{title}: nothing found in {} ({}).",
                    counted(s.files_scanned as u64, "file", "files"),
                    util::human_bytes(s.bytes_scanned)
                )
            } else if group_count > 0 {
                format!(
                    "{title}: {} · {} · {} reclaimable.",
                    counted(group_count as u64, "group", "groups"),
                    counted(findings as u64, "item", "items"),
                    util::human_bytes(reclaim)
                )
            } else {
                format!(
                    "{title}: {} · {} reclaimable.",
                    counted(findings as u64, "finding", "findings"),
                    util::human_bytes(reclaim)
                )
            };
            crate::ui::toast::notify(s.status.clone(), false);
            s.posters.clear();
            s.strips.clear();
            s.poster_jobs.clear();
            return;
        }
        s.mode_result = None;
        s.groups = out.groups;
        // Pristine copy for live re-thresholding; the floor starts at the
        // scan threshold (everything qualifies).
        s.base_groups = s.groups.clone();
        // Fresh results: disarm any pending consolidate from last scan and
        // drop a confirmation that previewed those paths.
        s.consolidate_dest = None;
        s.consolidating = false;
        s.confirm_delete = false;
        s.cleared = None;
        s.findings_done = None;
        s.min_sim = s.scan_similarity;
        s.group_limit = crate::ui::state::GROUP_PAGE;
        // One display ordering for the whole scan (honors the sort toggle).
        sort_groups(s);
        // Pre-select every duplicate for deletion; the user unchecks what
        // should survive. Keepers and copies in a locked folder are never
        // selectable, and the rule for which is which lives in one place.
        reselect_all_dups(s);
        // Every group is open by default: the results are readable without
        // a click, and Collapse all is one press away. Nothing is remembered
        // as closed, so a later re-threshold or "Show more" opens too.
        s.collapsed.clear();
        recount(s);
        if s.groups.is_empty() {
            s.status = format!(
                "No duplicates — scanned {} ({}).",
                counted(s.files_scanned as u64, "file", "files"),
                util::human_bytes(s.bytes_scanned)
            );
        } else {
            // The header above the list already reports the groups, the
            // copies and the reclaimable space; the status line reports the
            // run itself, so the same sentence is never on screen twice.
            s.status = "Scan complete.".to_string();
        }
        for e in &out.cache_save_errors {
            s.status.push_str(&format!(" (cache write failed: {e})"));
        }
        // The page itself shows the scan's outcome (its groups or the
        // nothing-found card) and the status line repeats it; only a problem
        // the page does not show earns a toast.
        if !out.cache_save_errors.is_empty() {
            crate::ui::toast::notify(s.status.clone(), true);
        }
        // Preview strips are per-scan too: paths are gone, so drop frames
        // and in-flight flags.
        s.posters.clear();
        s.strips.clear();
        s.poster_jobs.clear();
    });
    // Posters for the groups the user can already see.
    request_expanded_posters();
}

// --- Media previews ---------------------------------------------------------

/// Extract one video's poster in the background, once.
///
/// The results view calls this for each video row it actually renders, so a
/// huge scan never spawns ffmpeg for files nobody looked at. `poster_jobs`
/// holds every path already requested this scan, so a failed extraction is
/// not retried on every re-render; the finished poster lands in `posters`,
/// which re-renders the row that asked for it.
pub(crate) fn ensure_poster(path: &Path, size: u64, mtime: Option<i64>) {
    let idle =
        app().with_untracked(|s| !s.posters.contains_key(path) && !s.poster_jobs.contains(path));
    if !idle {
        return;
    }
    let job = path.to_path_buf();
    let key = job.clone();
    // The scan this poster belongs to. A result that lands after the reader
    // left the results — or after the next run started — must not be filed
    // under the path, where the new cards would show it.
    let generation = app().with_untracked(|s| s.scan_gen);
    app().update(|s| {
        s.poster_jobs.insert(key.clone());
    });
    app_spawn(async move {
        let slot = poster_slot().await;
        let done = blocking(move || poster::video_poster(&job, size, mtime))
            .await
            .ok()
            .flatten();
        drop(slot);
        if let Some(thumb) = done {
            app().update(move |s| {
                if s.scan_gen == generation {
                    s.posters.insert(key, thumb);
                }
            });
        }
    });
}

/// How many poster extractions run at once.
///
/// Each is an `ffmpeg` process, and a finished scan asked for the posters of
/// every video on the whole page in one go: hundreds of processes started in
/// the same frame and left the machine (and with it the window) unresponsive
/// right as the results appeared. A few at a time keeps the UI drawing; the
/// posters still arrive, in the order they were asked for.
const POSTER_EXTRACTIONS: usize = 3;

/// Poster extractions in flight.
static POSTERS_RUNNING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A turn to extract one poster; given back when dropped.
#[derive(Debug)]
struct PosterSlot;

impl Drop for PosterSlot {
    fn drop(&mut self) {
        POSTERS_RUNNING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Wait for a free extraction turn.
///
/// Waiting is a task, not a thread: the queue costs nothing while it waits
/// and never holds up the UI thread.
async fn poster_slot() -> PosterSlot {
    use std::sync::atomic::Ordering;
    loop {
        // A compare-exchange loop rather than `fetch_update`: that method is
        // deprecated in current stable (it became `try_update`), and a plain
        // compare-exchange compiles unchanged on every toolchain.
        let running = POSTERS_RUNNING.load(Ordering::SeqCst);
        if running < POSTER_EXTRACTIONS
            && POSTERS_RUNNING
                .compare_exchange_weak(running, running + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            return PosterSlot;
        }
        timer(Duration::from_millis(50)).await;
    }
}

/// Ask for the posters of one group's videos.
///
/// Called when a group becomes visible (the groups a scan expands, and any
/// group the user expands) and never from the render path: writing state
/// while the tree is being built is re-entrancy the engine should not have
/// to survive.
pub(crate) fn request_group_posters(group_index: usize) {
    let jobs: Vec<(PathBuf, u64, Option<i64>)> = app().with_untracked(|s| {
        s.groups
            .iter()
            .find(|g| g.index == group_index)
            .map(|g| {
                g.members
                    .iter()
                    .filter(|m| media::classify(&m.path) == media::MediaKind::Video)
                    .map(|m| (m.path.clone(), m.size, m.mtime_secs))
                    .collect()
            })
            .unwrap_or_default()
    });
    for (path, size, mtime) in jobs {
        ensure_poster(&path, size, mtime);
    }
}

/// Ask for the posters of every open group on the rendered page.
///
/// Expansion is the default now, so the bound is the page (`group_limit`),
/// not the expansion set: a scan that finds thousands of groups must not
/// start an ffmpeg run per video before the user has scrolled to them.
fn request_expanded_posters() {
    let indexes: Vec<usize> = app().with_untracked(|s| {
        s.groups
            .iter()
            .take(s.group_limit)
            .filter(|group| !s.collapsed.contains(&group.index))
            .map(|group| group.index)
            .collect()
    });
    for index in indexes {
        request_group_posters(index);
    }
}

// --- Selection and ordering -------------------------------------------------

/// Mark every deletable duplicate for deletion (fresh intent after a
/// keeper/mode change). Copies in a locked folder stay unticked: there is no
/// delete that would take them.
pub(crate) fn reselect_all_dups(s: &mut AppState) {
    s.selected = s
        .groups
        .iter()
        .flat_map(|g| g.members.iter())
        .filter(|m| m.deletable())
        .map(|m| m.path.clone())
        .collect();
}

/// Recompute the summary counters from the current groups.
pub(crate) fn recount(s: &mut AppState) {
    let dups: u64 = s.groups.iter().map(|g| g.dup_count() as u64).sum();
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
    // A new order is a new list: the cards must be rebuilt in it.
    s.results_rev += 1;
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

/// Mirror keep flags, the keep reason and member order from the live view
/// back into the pristine scan results, so re-thresholding never loses
/// keeper choices. Members hidden by the floor keep their stored flags.
pub(crate) fn sync_keep_to_base(s: &mut AppState) {
    for g in &s.groups {
        if let Some(base) = s.base_groups.iter_mut().find(|b| b.index == g.index) {
            base.keep_reason = g.keep_reason;
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
        "Similarity ≥ {:.0}% — {}, {}.",
        t * 100.0,
        counted(s.groups.len() as u64, "group", "groups"),
        counted(s.dup_files, "duplicate", "duplicates"),
    );
}

/// Parse the live floor box: blank restores the scan floor, a number
/// tightens it, anything below the scan floor is clamped (loosening needs
/// discarded pairs — rescan instead). Before any scan there are no results
/// to filter, so any valid value simply becomes the threshold the next run
/// starts from.
///
/// A floor that takes effect is written back into both similarity boxes and
/// the preferences file, so the pill on the home page and the setting are
/// one value everywhere the app shows it and it survives a relaunch.
pub(crate) fn apply_sim_input(raw: &str) {
    let mut applied = false;
    app().update(|s| {
        let raw = raw.trim();
        let floor_text = format_similarity_pct(s.scan_similarity * 100.0);
        if raw.is_empty() {
            if (s.min_sim - s.scan_similarity).abs() > f64::EPSILON {
                apply_threshold(s, s.scan_similarity);
                applied = true;
            }
            // The cleared box must not leave the pill falling back to its
            // 97% default while the scan floor is something else.
            if s.min_sim_input != floor_text || s.similarity_input != floor_text {
                applied = true;
            }
            s.min_sim_input = floor_text.clone();
            s.similarity_input = floor_text;
            return;
        }
        match raw.parse::<f64>() {
            Ok(v) if (0.0..=100.0).contains(&v) => {
                let t = v / 100.0;
                let text = format_similarity_pct(v);
                if s.base_groups.is_empty() {
                    // No scan yet: the pill sets the threshold the next run
                    // starts from, so every valid value is accepted — even
                    // one saved looser than the shipped default.
                    s.scan_similarity = t;
                    s.min_sim = t;
                    s.min_sim_input = text.clone();
                    s.similarity_input = text;
                    applied = true;
                    return;
                }
                if t < s.scan_similarity {
                    s.status = format!(
                        "Floor {:.0}% is below the scan threshold {:.0}% — rescan to loosen.",
                        v,
                        s.scan_similarity * 100.0
                    );
                    return;
                }
                apply_threshold(s, t);
                s.min_sim_input = text.clone();
                s.similarity_input = text;
                applied = true;
            }
            _ => {}
        }
    });
    if applied {
        save_prefs();
    }
}

/// Format a similarity percent (0..=100) the way every similarity box shows
/// it: whole numbers stay whole, fractions keep their decimal places.
pub(crate) fn format_similarity_pct(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Promote one member to keeper of its group (per-group decision).
///
/// Member order never changes here: the grid keeps every card in place and
/// only the highlight (the keeper/selected badge and edge) moves. Ticks outside the
/// group are left alone too — a keeper pick says nothing about the other
/// groups' selections, and re-ticking them all would churn the whole list.
pub(crate) fn set_keeper(group_index: usize, path: &Path) {
    app().update(|s| set_keeper_in(s, group_index, path));
}

/// [`set_keeper`] on plain state, so the decision can be tested without the
/// app's signal.
pub(crate) fn set_keeper_in(s: &mut AppState, group_index: usize, path: &Path) {
    let Some(group) = s.groups.iter_mut().find(|g| g.index == group_index) else {
        return;
    };
    if group.members.iter().any(|m| m.keep && m.path == path) {
        // Already the keeper: nothing moves, nothing re-renders.
        return;
    }
    for member in group.members.iter_mut() {
        if member.keep {
            // "Keep this one instead" keeps one copy per group, so the
            // retiring keeper becomes a duplicate again, ticked like every
            // other one in its group — unless it sits in a locked folder,
            // where a tick means nothing. The swap is never silent: the
            // tile turns amber with the word Delete, the Reclaimable
            // figure rises, and the confirmation lists the copy. One untick
            // ("Keep too") takes it back off the list.
            member.keep = false;
            if !member.reference {
                s.selected.insert(member.path.clone());
            }
        }
        if member.path == path {
            member.keep = true;
            s.selected.remove(&member.path);
        }
    }
    group.keep_reason = KeepReason::ChosenByYou;
    sync_keep_to_base(s);
    // Keeper swap changes which bytes count as reclaimable.
    recount(s);
    // The keeper the open confirmation lists has changed: rebuild it even
    // though the tick count did not move.
    s.confirm_rev += 1;
    // Keeper change: disarm a pending consolidate (stale paths).
    s.consolidate_dest = None;
}

/// Set the keep rule the next scan applies. Results already on screen keep
/// their keepers and ticks: re-picking them here would silently undo every
/// choice the user made while reviewing.
pub(crate) fn set_keep_rule(index: usize) {
    app().update(|s| s.keep_index = Some(index));
    save_prefs();
}

/// Switch the results layout. The list's structure changes, so it is rebuilt
/// (that is what the revision is for) and any inline player stops: the card
/// or row it lived in no longer exists.
pub(crate) fn set_layout(layout: crate::ui::state::Layout) {
    app().update(|s| {
        if s.layout != layout {
            s.layout = layout;
            s.playing = None;
            s.results_rev += 1;
        }
    });
}

/// Play a video in its card's preview, or stop the one already playing.
///
/// One at a time: starting another replaces it. The player is muted, so a
/// click never starts sound the user did not ask for.
pub(crate) fn toggle_play(path: &Path) {
    app().update(|s| {
        let same = s.playing.as_deref() == Some(path);
        s.playing = if same { None } else { Some(path.to_path_buf()) };
    });
}

/// Flip one duplicate's tick (KEEP members are never selectable).
pub(crate) fn toggle_selected(path: &Path) {
    app().update(|s| {
        // `deletable`, not `!keep`: a tick on a locked copy would never be
        // carried out, so it must not be given.
        let is_dup = s
            .groups
            .iter()
            .flat_map(|g| g.members.iter())
            .any(|m| m.path == path && m.deletable());
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

pub(crate) fn show_more_findings() {
    app().update(|s| s.mode_limit += crate::ui::state::GROUP_PAGE);
}

pub(crate) fn show_more_groups() {
    app().update(|s| s.group_limit += crate::ui::state::GROUP_PAGE);
    // The newly revealed page is expanded like the rest, so its posters are
    // wanted too (already-requested videos are skipped).
    request_expanded_posters();
}

pub(crate) fn toggle_expanded(index: usize) {
    let mut opened = false;
    app().update(|s| {
        // Groups start open: the set holds only the ones the user closed.
        if s.collapsed.remove(&index) {
            opened = true;
        } else {
            s.collapsed.insert(index);
        }
    });
    if opened {
        request_group_posters(index);
    }
}

/// Open or close every group currently rendered.
pub(crate) fn set_all_expanded(open: bool) {
    app().update(|s| {
        s.collapsed.clear();
        if !open {
            // Only the rendered page is closed; a later page opens with the
            // rest when "Show more" reveals it.
            s.collapsed
                .extend(s.groups.iter().take(s.group_limit).map(|g| g.index));
        }
    });
    if open {
        request_expanded_posters();
    }
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
            if m.deletable() && s.selected.contains(&m.path) {
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

/// What a delete would take right now: the same set as [`pending_targets`],
/// counted without building it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Ticked {
    pub(crate) count: usize,
    pub(crate) bytes: u64,
}

pub(crate) fn ticked(s: &AppState) -> Ticked {
    s.groups
        .iter()
        .flat_map(|g| &g.members)
        .filter(|m| m.deletable() && s.selected.contains(&m.path))
        .fold(Ticked::default(), |t, m| Ticked {
            count: t.count + 1,
            bytes: t.bytes + m.size,
        })
}

/// Groups listed in the delete confirmation. Every group with a marked copy
/// is listed, and every copy within it: the confirmation is the last look
/// before files go, so it may hide nothing the review screen shows.
pub(crate) struct ConfirmGroup {
    pub(crate) index: usize,
    pub(crate) keeper: String,
    pub(crate) rows: Vec<GroupMember>,
    pub(crate) bytes: u64,
    pub(crate) similarity: Option<f64>,
    /// Its copies are byte-identical, so a hard link can replace them.
    pub(crate) linkable: bool,
}

/// Per-group dry run behind the delete confirmation, in display order.
pub(crate) fn confirm_groups(s: &AppState) -> Vec<ConfirmGroup> {
    let mut groups = Vec::new();
    for g in &s.groups {
        let mut rows = Vec::new();
        let mut bytes = 0u64;
        for m in &g.members {
            if !m.deletable() || !s.selected.contains(&m.path) {
                continue;
            }
            bytes += m.size;
            rows.push(m.clone());
        }
        if rows.is_empty() {
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
        groups.push(ConfirmGroup {
            index: g.index,
            keeper,
            rows,
            bytes,
            similarity: g.similarity,
            linkable: file_actions::can_hardlink(g),
        });
    }
    groups
}

/// The Delete button's job: open the confirmation for the ticked duplicates.
/// Nothing is removed until it is confirmed.
pub(crate) fn request_delete() {
    let state = app();
    if busy() {
        return;
    }
    if state.with_untracked(|s| pending_targets(s).is_empty()) {
        state.update(|s| {
            s.status = "Mark the copies you want deleted first.".to_string();
        });
        return;
    }
    // The dialog opens at its top, wherever the wheel left its list before.
    crate::ui::results::reset_delete_scroll();
    state.update(|s| s.confirm_delete = true);
}

/// Dismiss the confirmation without removing anything.
pub(crate) fn cancel_delete() {
    app().update(|s| s.confirm_delete = false);
}

/// Run the confirmed removal: the dialog closes first, so its rows are gone
/// before the files are.
pub(crate) fn confirm_delete() {
    app().update(|s| s.confirm_delete = false);
    perform_removal();
}

/// Pick what the confirmation does with the ticked copies, and remember it:
/// the toolbar button that opens the dialog names the same act.
pub(crate) fn set_removal(index: usize) {
    let removal = Removal::from_index(index);
    if app().with_untracked(|s| s.removal) == removal {
        return;
    }
    app().update(|s| s.removal = removal);
    save_prefs();
}

/// Choose the folder a move puts the ticked copies in, from the open
/// confirmation; it then lists where each one lands.
pub(crate) fn pick_move_dir() {
    let start = app().with_untracked(|s| {
        s.move_dest
            .as_ref()
            .map(|d| d.display().to_string())
            .or_else(|| s.folders.last().map(|f| f.path.clone()))
            .unwrap_or_default()
    });
    app_spawn(async move {
        let picked = blocking(move || {
            let mut dlg =
                rfd::FileDialog::new().set_title("Choose the folder to move the copies into");
            if !start.is_empty() && Path::new(&start).exists() {
                dlg = dlg.set_directory(&start);
            }
            dlg.pick_folder()
        })
        .await;
        let Ok(Some(dest)) = picked else { return };
        app().update(|s| {
            s.move_dest = Some(dest);
            s.confirm_rev += 1;
        });
    });
}

/// The ticked copies a hard-link run would replace: those in groups whose
/// copies are byte-identical, each paired with its group's keeper. The count
/// is of ticked copies left alone because their group is only similar.
pub(crate) fn link_plan(s: &AppState) -> (Vec<file_actions::LinkTarget>, usize) {
    let mut targets = Vec::new();
    let mut unlinkable = 0usize;
    for g in &s.groups {
        let ticked = g
            .members
            .iter()
            .filter(|m| m.deletable() && s.selected.contains(&m.path));
        let Some(keeper) = g.members.iter().find(|m| m.keep) else {
            continue;
        };
        if !file_actions::can_hardlink(g) {
            unlinkable += ticked.count();
            continue;
        }
        for m in ticked {
            targets.push(file_actions::LinkTarget {
                path: m.path.clone(),
                expected_hash: m.content_hash.clone().unwrap_or_else(|| g.hash.clone()),
                size: m.size,
                keeper: keeper.path.clone(),
                keeper_hash: keeper
                    .content_hash
                    .clone()
                    .unwrap_or_else(|| g.hash.clone()),
                keeper_size: keeper.size,
            });
        }
    }
    (targets, unlinkable)
}

/// What one removal run did, in the terms every kind of run shares, so the
/// list, the counters and the status line follow it the same way.
struct RemovalDone {
    /// The scan whose results the run acted on.
    generation: u64,
    removal: Removal,
    /// Paths that left their group: deleted, moved away or linked.
    done: Vec<PathBuf>,
    bytes: u64,
    went: Went,
    /// Copies left where they were because they changed since the scan (or
    /// failed; those are in `failures` too).
    skipped: Vec<PathBuf>,
    failures: Vec<(PathBuf, file_actions::ActionError)>,
    recorded: Recorded,
}

/// A finished run as the clean-up history took it, with the history read
/// back for the History page.
struct Recorded {
    /// The run's id; `None` when it changed nothing or could not be written.
    id: Option<u64>,
    runs: Vec<history::Run>,
}

/// Record a finished run and read the history back, on the worker thread
/// that ran it so the window never waits on the file.
fn record(
    kind: RunKind,
    method: Method,
    roots: &[String],
    hash: cli::HashAlgo,
    bytes: u64,
    items: Vec<history::Item>,
) -> Recorded {
    let id = history::record(history::NewRun {
        kind,
        method,
        roots,
        hash_algo: hashing::HashEngine::new(hash).name(),
        bytes,
        items,
    });
    Recorded {
        id,
        runs: history::load(),
    }
}

/// Take a recorded run in: the History page lists it, and Undo offers it
/// while any of it can still come back.
fn take_recorded(s: &mut AppState, recorded: Recorded) {
    s.history = Arc::new(recorded.runs);
    if recorded.id.is_some() {
        s.last_run = recorded.id;
    }
}

/// Claim the one running file act, saying `status` while it runs, and return
/// the scan generation it acts on; `None` (with a note) while another runs.
fn begin_act(status: &str) -> Option<u64> {
    let mut generation = None;
    app().update(|s| {
        if s.acting {
            s.status = BUSY.to_string();
        } else {
            s.acting = true;
            s.status = status.to_string();
            generation = Some(s.scan_gen);
        }
    });
    generation
}

/// What a file act says when another one is still running.
const BUSY: &str = "Wait for the current change to finish first.";

/// Refuse to open a confirmation while a file act runs: its plan would list
/// files that act is about to change.
fn busy() -> bool {
    let acting = app().with_untracked(|s| s.acting);
    if acting {
        app().update(|s| s.status = BUSY.to_string());
    }
    acting
}

/// The ticked copies go the way the confirmation chose: deleted (trash or
/// permanent per the setting), moved into the picked folder, or replaced by
/// hard links to their keeper. Every file is re-hashed first either way.
pub(crate) fn perform_removal() {
    let state = app();
    let targets = state.with_untracked(pending_targets);
    if targets.is_empty() {
        state.update(|s| {
            s.status = "Mark the copies you want deleted first.".to_string();
        });
        return;
    }
    let (removal, trash, hash, dest, roots) = state.with_untracked(|s| {
        (
            s.removal,
            s.trash,
            s.last_hash,
            s.move_dest.clone(),
            s.last_paths.clone(),
        )
    });
    match removal {
        Removal::Delete => {
            let disposition = if trash {
                file_actions::Disposition::Trash
            } else {
                file_actions::Disposition::Permanent
            };
            let Some(generation) = begin_act("Deleting…") else {
                return;
            };
            app_spawn(async move {
                let done = blocking(move || {
                    let engine = hashing::HashEngine::new(hash);
                    let mut outcome = file_actions::delete_targets(&targets, &engine, disposition);
                    let recorded = record(
                        RunKind::Duplicates,
                        disposition.into(),
                        &roots,
                        hash,
                        outcome.bytes_freed,
                        std::mem::take(&mut outcome.items),
                    );
                    (outcome, recorded)
                })
                .await;
                let Ok((outcome, recorded)) = done else {
                    worker_stopped("The delete stopped unexpectedly; nothing more was removed.");
                    return;
                };
                apply_removal(RemovalDone {
                    generation,
                    removal,
                    done: outcome.deleted,
                    bytes: outcome.bytes_freed,
                    went: disposition.into(),
                    skipped: outcome.skipped,
                    failures: outcome.failures,
                    recorded,
                });
            });
        }
        Removal::MoveAside => {
            let Some(dest) = dest else {
                state.update(|s| {
                    s.status = "Choose a folder to move the copies into first.".to_string()
                });
                return;
            };
            let moves = file_actions::plan_move_aside(&targets, &dest);
            let Some(generation) = begin_act("Moving copies…") else {
                return;
            };
            app_spawn(async move {
                let done = blocking(move || {
                    let engine = hashing::HashEngine::new(hash);
                    let mut outcome = file_actions::move_targets(&moves, &engine);
                    let recorded = record(
                        RunKind::MoveAside,
                        Method::Moved,
                        &roots,
                        hash,
                        outcome.bytes_moved,
                        std::mem::take(&mut outcome.items),
                    );
                    (outcome, recorded)
                })
                .await;
                let Ok((outcome, recorded)) = done else {
                    worker_stopped("The move stopped unexpectedly; nothing more was moved.");
                    return;
                };
                apply_removal(RemovalDone {
                    generation,
                    removal,
                    done: outcome.moved.into_iter().map(|(from, _)| from).collect(),
                    bytes: outcome.bytes_moved,
                    went: Went::Moved,
                    skipped: outcome.skipped,
                    failures: outcome.failures,
                    recorded,
                });
            });
        }
        Removal::Link => {
            let (links, _) = state.with_untracked(link_plan);
            if links.is_empty() {
                state.update(|s| {
                    s.status =
                        "Nothing to link: only identical copies can become hard links.".to_string();
                });
                return;
            }
            let Some(generation) = begin_act("Linking copies…") else {
                return;
            };
            app_spawn(async move {
                let done = blocking(move || {
                    let engine = hashing::HashEngine::new(hash);
                    let mut outcome = file_actions::link_targets(&links, &engine);
                    let recorded = record(
                        RunKind::HardLink,
                        Method::Linked,
                        &roots,
                        hash,
                        outcome.bytes_saved,
                        std::mem::take(&mut outcome.items),
                    );
                    (outcome, recorded)
                })
                .await;
                let Ok((outcome, recorded)) = done else {
                    worker_stopped("Linking stopped unexpectedly; nothing more was changed.");
                    return;
                };
                apply_removal(RemovalDone {
                    generation,
                    removal,
                    done: outcome.linked,
                    bytes: outcome.bytes_saved,
                    went: Went::Freed,
                    skipped: outcome.skipped,
                    failures: outcome.failures,
                    recorded,
                });
            });
        }
    }
}

fn worker_stopped(message: &str) {
    let message = message.to_string();
    app().update(|s| {
        s.acting = false;
        s.status = message;
    });
}

/// The last name of a path, for a sentence that has to stay short.
fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The clause naming copies that stayed because they changed since the scan:
/// the count, the first file, and why, so the sentence is actionable without
/// opening anything. Failures have their own clause.
fn changed_note<E>(skipped: &[PathBuf], failures: &[(PathBuf, E)]) -> String {
    let failed: HashSet<&PathBuf> = failures.iter().map(|(p, _)| p).collect();
    let changed: Vec<&PathBuf> = skipped.iter().filter(|p| !failed.contains(*p)).collect();
    match changed.first() {
        Some(path) => {
            let one = changed.len() == 1;
            format!(
                "; {} had changed since the scan, so {} stayed where {} {} — {}",
                counted(changed.len() as u64, "file", "files"),
                if one { "it" } else { "they" },
                if one { "it" } else { "they" },
                if one { "is" } else { "are" },
                file_name(path),
            )
        }
        None => String::new(),
    }
}

/// The clause naming what could not be done, with the first reason.
fn failed_note<E: std::fmt::Display>(failures: &[(PathBuf, E)], verb: &str) -> String {
    match failures.first() {
        Some((_, err)) => format!(
            "; {} could not be {verb}: {err}",
            counted(failures.len() as u64, "file", "files"),
        ),
        None => String::new(),
    }
}

/// Take the copies a removal run finished with out of the results, count what
/// went where, and say so.
fn apply_removal(run: RemovalDone) {
    app().update(|s| {
        s.acting = false;
        // Whatever page is up now, the History page lists the run and Undo
        // offers it.
        take_recorded(s, run.recorded);
        // A scan (or Back) since the run started: these results are not the
        // ones it acted on, so only the status says what happened.
        if s.scan_gen == run.generation {
            let gone: HashSet<PathBuf> = run.done.iter().cloned().collect();
            for groups in [&mut s.groups, &mut s.base_groups] {
                for g in groups.iter_mut() {
                    g.members.retain(|m| !gone.contains(&m.path));
                }
                // Pruning the pristine copy too means a later floor change
                // cannot resurrect copies that are gone.
                groups.retain(|g| g.members.len() >= 2);
            }
            // Members are gone: the list's cards are stale now.
            s.results_rev += 1;
            s.playing = None;
            for p in &run.done {
                s.selected.remove(p);
            }
            // Those paths are gone: disarm a pending consolidate.
            s.consolidate_dest = None;
            if !run.done.is_empty() {
                s.cleared = Some(Cleared::add(
                    s.cleared,
                    run.done.len() as u64,
                    run.bytes,
                    run.went,
                ));
            }
            recount(s);
        }
        let files = counted(run.done.len() as u64, "file", "files");
        let bytes = util::human_bytes(run.bytes);
        let changed = changed_note(&run.skipped, &run.failures);
        s.status = match (run.went, run.removal) {
            // The trash keeps the space until it is emptied: a trash delete
            // moves bytes, only a permanent one frees them.
            (Went::Trash, _) => format!(
                "Moved {files} to the trash ({bytes}){changed}{}.",
                failed_note(&run.failures, "removed")
            ),
            (Went::Moved, _) => format!(
                "Moved {files} ({bytes}) to {}{changed}{}.",
                s.move_dest
                    .as_ref()
                    .map_or_else(|| "the folder".to_string(), |d| d.display().to_string()),
                failed_note(&run.failures, "moved")
            ),
            (Went::Freed, Removal::Link) => {
                // A link whose old copy would not go is linked all the same;
                // it is named apart, since its space is still taken.
                let (kept, failed): (Vec<_>, Vec<_>) = run
                    .failures
                    .iter()
                    .map(|(p, e)| (p.clone(), e))
                    .partition(|(_, e)| matches!(e, file_actions::ActionError::BackupKept { .. }));
                let kept = match kept.first() {
                    Some((_, err)) => format!(
                        "; {} kept {} old {} beside the link: {err}",
                        counted(kept.len() as u64, "file", "files"),
                        if kept.len() == 1 { "its" } else { "their" },
                        if kept.len() == 1 { "copy" } else { "copies" },
                    ),
                    None => String::new(),
                };
                format!(
                    "Replaced {files} with hard links ({bytes} freed){changed}{}{kept}.",
                    failed_note(&failed, "linked")
                )
            }
            (Went::Freed, _) => format!(
                "Deleted {files} ({bytes} freed){changed}{}.",
                failed_note(&run.failures, "removed")
            ),
        };
        // With every group gone the cleared card says the same thing on the
        // page; a toast on top would only repeat it.
        let problems = !run.failures.is_empty() || !run.skipped.is_empty();
        if problems || !s.groups.is_empty() {
            crate::ui::toast::notify(s.status.clone(), !run.failures.is_empty());
        }
    });
}

// --- History and restore ----------------------------------------------------

/// Whether any file of `run` may still come back, by what the history knows:
/// not back yet, and trashed where this system can restore, or moved.
#[must_use]
pub(crate) fn can_come_back(run: &history::Run) -> bool {
    run.items.iter().any(|item| {
        !item.restored
            && match item.went {
                history::Went::Trash => file_actions::CAN_RESTORE_TRASH,
                history::Went::Moved { .. } => true,
                _ => false,
            }
    })
}

/// The Undo button's label for this session's last clean-up, or `None` when
/// nothing of it can come back.
#[must_use]
pub(crate) fn undo_label(s: &AppState) -> Option<&'static str> {
    let run = s.history.iter().find(|run| Some(run.id) == s.last_run)?;
    can_come_back(run).then_some(match run.method {
        Method::Trash => "Restore from trash",
        _ => "Move back",
    })
}

/// Undo: put this session's last clean-up back through the history, like
/// any restore. It asks nothing first: the files return to their own places
/// and nothing there is ever overwritten.
pub(crate) fn undo_last() {
    if let Some(id) = app().with_untracked(|s| s.last_run) {
        perform_restore(id, None);
    }
}

/// Open the restore confirmation for run `id`. What comes back and what
/// cannot is read from the disk and the trash first, off the UI thread; the
/// dialog opens once it is known.
pub(crate) fn request_restore(id: u64) {
    if busy() {
        return;
    }
    let Some(run) = app().with_untracked(|s| s.history.iter().find(|r| r.id == id).cloned()) else {
        return;
    };
    crate::ui::history::reset_restore_scroll();
    app_spawn(async move {
        let planned = blocking(move || {
            let items = file_actions::plan_restore(&run);
            (run, items)
        })
        .await;
        let Ok((run, items)) = planned else { return };
        app().update(|s| {
            s.restore = Some(RestoreAsk {
                run,
                items,
                left_out: HashSet::new(),
            });
        });
    });
}

/// Leave one file of the open restore where it is, or take it back in.
pub(crate) fn toggle_restore_item(path: &Path) {
    app().update(|s| {
        if let Some(ask) = s.restore.as_mut()
            && !ask.left_out.remove(path)
        {
            ask.left_out.insert(path.to_path_buf());
        }
    });
}

pub(crate) fn cancel_restore() {
    app().update(|s| s.restore = None);
}

/// The restore confirmation's commit: the files it lists as coming back,
/// less the ones left out, go back.
pub(crate) fn confirm_restore() {
    let mut taken = None;
    app().update(|s| taken = s.restore.take());
    let Some(ask) = taken else { return };
    let chosen: HashSet<PathBuf> = ask
        .items
        .iter()
        .filter(|item| item.verdict.comes_back() && !ask.left_out.contains(&item.path))
        .map(|item| item.path.clone())
        .collect();
    if !chosen.is_empty() {
        perform_restore(ask.run.id, Some(chosen));
    }
}

/// Put back run `id` — all of it, or only the `chosen` files — and say what
/// came back and what stayed where it is, and why.
fn perform_restore(id: u64, chosen: Option<HashSet<PathBuf>>) {
    if begin_act("Putting files back…").is_none() {
        return;
    }
    app_spawn(async move {
        let done = blocking(move || {
            let outcome = file_actions::restore_run(id, |path| {
                chosen.as_ref().is_none_or(|chosen| chosen.contains(path))
            });
            (outcome, history::load())
        })
        .await;
        let Ok((outcome, runs)) = done else {
            worker_stopped("Putting files back stopped unexpectedly; nothing more was moved.");
            return;
        };
        app().update(|s| {
            s.acting = false;
            s.history = Arc::new(runs);
            // The outcome cards offer Undo, so they follow what it did.
            s.results_rev += 1;
            let Some(outcome) = outcome else {
                s.status = "That clean-up is no longer in the history.".to_string();
                return;
            };
            let back = outcome.restored.len() as u64;
            let stayed: Vec<&file_actions::PlannedItem> = outcome
                .items
                .iter()
                .filter(|item| {
                    !matches!(
                        item.verdict,
                        file_actions::Verdict::Restored | file_actions::Verdict::AlreadyBack
                    )
                })
                .collect();
            let mut status = if back == 0 {
                "Nothing came back".to_string()
            } else {
                format!(
                    "Put back {} ({})",
                    counted(back, "file", "files"),
                    util::human_bytes(outcome.bytes)
                )
            };
            if let Some(first) = stayed.first() {
                status.push_str(&format!(
                    "; {} stayed where {} — {}: {}",
                    counted(stayed.len() as u64, "file", "files"),
                    if stayed.len() == 1 {
                        "it is"
                    } else {
                        "they are"
                    },
                    file_name(&first.path),
                    first.verdict.says()
                ));
            }
            if back > 0 && s.last_run == Some(id) {
                status.push_str(". Scan again to see them in the list");
                if s.cleared.is_some() {
                    s.cleared = Some(Cleared::Restored { files: back });
                }
                if s.findings_done.is_some() {
                    s.findings_done = Some(FindingsDone {
                        items: back,
                        bytes: 0,
                        went: FindingsWent::Restored,
                    });
                }
            }
            status.push('.');
            s.status = status;
            let failed = back == 0
                || stayed
                    .iter()
                    .any(|item| matches!(item.verdict, file_actions::Verdict::Failed(_)));
            crate::ui::toast::notify(s.status.clone(), failed);
        });
    });
}

// --- Consolidate ------------------------------------------------------------

/// Consolidate, step one: open the confirmation. It lists the keepers that
/// would move before anything else is asked; the folder is chosen inside it
/// ([`pick_consolidate_dir`]) and nothing moves until it is confirmed
/// ([`confirm_consolidate`]).
pub(crate) fn open_consolidate() {
    if busy() {
        return;
    }
    // The confirmation opens at its top, wherever the wheel left its list
    // the last time.
    crate::ui::results::reset_consolidate_scroll();
    app().update(|s| {
        s.consolidating = true;
        s.consolidate_dest = None;
    });
}

/// Consolidate, step two: pick the destination folder from the open
/// confirmation, which then lists every move into it.
pub(crate) fn pick_consolidate_dir() {
    let start = app().with_untracked(|s| {
        s.consolidate_dest
            .as_ref()
            .map(|d| d.display().to_string())
            .or_else(|| s.folders.last().map(|f| f.path.clone()))
            .unwrap_or_default()
    });
    app_spawn(async move {
        let picked = blocking(move || {
            let mut dlg = rfd::FileDialog::new().set_title("Choose the folder to consolidate into");
            if !start.is_empty() && Path::new(&start).exists() {
                dlg = dlg.set_directory(&start);
            }
            dlg.pick_folder()
        })
        .await;
        let Ok(Some(dest)) = picked else { return };
        app().update(|s| {
            // A pick that lands after the dialog closed has nothing to fill.
            if s.consolidating {
                s.consolidate_dest = Some(dest);
            }
        });
    });
}

/// Close the consolidate confirmation without moving anything.
pub(crate) fn cancel_consolidate() {
    app().update(|s| {
        s.consolidating = false;
        s.consolidate_dest = None;
    });
}

/// Consolidate, step three: the confirmation's Move. The dialog closes
/// first, then every group's keeper moves into the chosen folder. The plan is
/// rebuilt here rather than reused, so it matches the groups as they are now;
/// keepers are re-hashed before moving and member paths plus ticks follow
/// moved files.
pub(crate) fn confirm_consolidate() {
    let state = app();
    let mut taken = None;
    state.update(|s| {
        s.consolidating = false;
        taken = s.consolidate_dest.take();
    });
    let Some(dest) = taken else { return };
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
        });
        return;
    }
    let (hash, roots) = state.with_untracked(|s| (s.last_hash, s.last_paths.clone()));
    let Some(generation) = begin_act("Moving keepers…") else {
        return;
    };
    app_spawn(async move {
        let done = blocking(move || {
            let engine = hashing::HashEngine::new(hash);
            let mut outcome = file_actions::move_targets(&targets, &engine);
            let recorded = record(
                RunKind::Consolidate,
                Method::Moved,
                &roots,
                hash,
                outcome.bytes_moved,
                std::mem::take(&mut outcome.items),
            );
            (outcome, recorded)
        })
        .await;
        let Ok((outcome, recorded)) = done else {
            worker_stopped("Consolidate failed: worker stopped.");
            return;
        };
        app().update(|s| {
            s.acting = false;
            take_recorded(s, recorded);
            // A scan (or Back) since the move started: these results are not
            // the ones it moved, so only the status says what happened.
            let moved = if s.scan_gen == generation {
                outcome.moved.as_slice()
            } else {
                &[]
            };
            for (from, to) in moved {
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
            // Member paths moved: rebuild the list against the new ones.
            s.results_rev += 1;
            s.playing = None;
            let failed = match outcome.failures.first() {
                Some((_, err)) => format!(", {} failed ({err})", outcome.failures.len()),
                None => String::new(),
            };
            s.status = format!(
                "Consolidated {} ({}) into {}; {} skipped{failed}.",
                counted(outcome.moved.len() as u64, "keeper", "keepers"),
                util::human_bytes(outcome.bytes_moved),
                dest.display(),
                outcome.skipped.len() + skipped,
            );
            crate::ui::toast::notify(s.status.clone(), !outcome.failures.is_empty());
        });
    });
}

// --- Cleanup findings -------------------------------------------------------

/// What the cleanup workspace can do with its findings: delete them, strip
/// the metadata out of the photos it found, or nothing (duplicate folders are
/// listed for a person to judge, as on the command line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindingsAct {
    Delete,
    StripMetadata,
    ReportOnly,
}

#[must_use]
pub(crate) fn findings_act(mode: cli::FindMode) -> FindingsAct {
    if mode == cli::FindMode::PhotoMetadata {
        FindingsAct::StripMetadata
    } else if crate::modes::mode_deletes(mode) {
        FindingsAct::Delete
    } else {
        FindingsAct::ReportOnly
    }
}

/// What the findings' act would touch: every finding and every group member
/// after the first (the copy a group keeps), less what sits in a protected
/// folder. `protected` counts the ones a protected folder keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FindingsPlan {
    pub(crate) count: usize,
    pub(crate) bytes: u64,
    pub(crate) protected: usize,
}

#[must_use]
pub(crate) fn findings_plan(s: &AppState) -> FindingsPlan {
    let Some(result) = s.mode_result.as_ref() else {
        return FindingsPlan::default();
    };
    let mut plan = FindingsPlan::default();
    for finding in &result.findings {
        if file_actions::is_protected(&finding.path, &s.last_ref_dirs) {
            plan.protected += 1;
        } else {
            plan.count += 1;
            plan.bytes += finding.size;
        }
    }
    for group in &result.groups {
        for (index, path) in group.members.iter().enumerate().skip(1) {
            if file_actions::is_protected(path, &s.last_ref_dirs) {
                plan.protected += 1;
            } else {
                plan.count += 1;
                plan.bytes += group.sizes.get(index).copied().unwrap_or(0);
            }
        }
    }
    plan
}

/// The first `limit` items the findings' act would touch, in list order,
/// with their sizes: what its confirmation shows before it counts the rest.
#[must_use]
pub(crate) fn findings_rows(s: &AppState, limit: usize) -> Vec<(PathBuf, u64)> {
    let Some(result) = s.mode_result.as_ref() else {
        return Vec::new();
    };
    let singles = result.findings.iter().map(|f| (&f.path, f.size));
    let members = result.groups.iter().flat_map(|g| {
        g.members
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, path)| (path, g.sizes.get(i).copied().unwrap_or(0)))
    });
    singles
        .chain(members)
        .filter(|(path, _)| !file_actions::is_protected(path, &s.last_ref_dirs))
        .take(limit)
        .map(|(path, size)| (path.clone(), size))
        .collect()
}

/// Open the findings' confirmation. Nothing changes until it is confirmed.
pub(crate) fn request_findings_act() {
    if busy() {
        return;
    }
    if app().with_untracked(|s| findings_plan(s).count == 0) {
        app().update(|s| {
            s.status = "Nothing to clean up: every finding is in a protected folder.".to_string();
        });
        return;
    }
    crate::ui::results::reset_findings_scroll();
    app().update(|s| s.confirm_findings = true);
}

pub(crate) fn cancel_findings_act() {
    app().update(|s| s.confirm_findings = false);
}

/// Run the findings' act once it is confirmed: delete them (per the trash
/// setting) or strip their metadata. Every file is checked against the scan
/// first; anything that changed, and anything in a protected folder, stays.
pub(crate) fn confirm_findings_act() {
    let state = app();
    state.update(|s| s.confirm_findings = false);
    let Some(result) = state.with_untracked(|s| s.mode_result.clone()) else {
        return;
    };
    let (trash, hash, protected, roots) = state.with_untracked(|s| {
        (
            s.trash,
            s.last_hash,
            s.last_ref_dirs.clone(),
            s.last_paths.clone(),
        )
    });
    match findings_act(result.mode) {
        FindingsAct::ReportOnly => {}
        FindingsAct::Delete => {
            let disposition = if trash {
                file_actions::Disposition::Trash
            } else {
                file_actions::Disposition::Permanent
            };
            let Some(generation) = begin_act("Deleting…") else {
                return;
            };
            app_spawn(async move {
                let done = blocking(move || {
                    let engine = hashing::HashEngine::new(hash);
                    let mut outcome = file_actions::remove_findings(
                        &result.findings,
                        &result.groups,
                        disposition,
                        &protected,
                        &engine,
                    );
                    let recorded = record(
                        RunKind::Findings,
                        disposition.into(),
                        &roots,
                        hash,
                        outcome.bytes_freed,
                        std::mem::take(&mut outcome.items),
                    );
                    (outcome, recorded)
                })
                .await;
                let Ok((outcome, recorded)) = done else {
                    worker_stopped("The delete stopped unexpectedly; nothing more was removed.");
                    return;
                };
                let files = counted(outcome.removed.len() as u64, "item", "items");
                let bytes = util::human_bytes(outcome.bytes_freed);
                let changed = changed_note(&outcome.skipped, &outcome.failures);
                let failed = failed_note(&outcome.failures, "removed");
                let kept = protected_note(outcome.protected.len());
                let status = if trash {
                    format!("Moved {files} to the trash ({bytes}){changed}{failed}{kept}.")
                } else {
                    format!("Deleted {files} ({bytes} freed){changed}{failed}{kept}.")
                };
                let went = if trash {
                    FindingsWent::Trash
                } else {
                    FindingsWent::Deleted
                };
                apply_findings_done(
                    generation,
                    &outcome.removed,
                    outcome.bytes_freed,
                    went,
                    status,
                    !outcome.failures.is_empty(),
                    recorded,
                );
            });
        }
        FindingsAct::StripMetadata => {
            let Some(generation) = begin_act("Removing photo metadata…") else {
                return;
            };
            app_spawn(async move {
                let done = blocking(move || {
                    let engine = hashing::HashEngine::new(hash);
                    let mut outcome =
                        crate::modes::strip_metadata(&result.findings, &protected, &engine);
                    let recorded = record(
                        RunKind::StripMetadata,
                        Method::Rewritten,
                        &roots,
                        hash,
                        outcome.bytes_saved,
                        std::mem::take(&mut outcome.items),
                    );
                    (outcome, recorded)
                })
                .await;
                let Ok((outcome, recorded)) = done else {
                    worker_stopped(
                        "Removing metadata stopped unexpectedly; nothing more was changed.",
                    );
                    return;
                };
                let photos = counted(outcome.stripped.len() as u64, "photo", "photos");
                let bytes = util::human_bytes(outcome.bytes_saved);
                let changed = changed_note(&outcome.skipped, &outcome.failures);
                let failed = failed_note(&outcome.failures, "rewritten");
                let kept = protected_note(outcome.protected.len());
                apply_findings_done(
                    generation,
                    &outcome.stripped,
                    outcome.bytes_saved,
                    FindingsWent::Stripped,
                    format!(
                        "Removed the metadata from {photos} ({bytes} freed){changed}{failed}{kept}."
                    ),
                    !outcome.failures.is_empty(),
                    recorded,
                );
            });
        }
    }
}

fn protected_note(kept: usize) -> String {
    if kept == 0 {
        String::new()
    } else {
        format!(
            "; {} in a protected folder stayed",
            counted(kept as u64, "item", "items")
        )
    }
}

/// Take the findings a run finished with off the list and report it.
fn apply_findings_done(
    generation: u64,
    done: &[PathBuf],
    bytes: u64,
    went: FindingsWent,
    status: String,
    failed: bool,
    recorded: Recorded,
) {
    let gone: HashSet<&PathBuf> = done.iter().collect();
    app().update(|s| {
        s.acting = false;
        // Whatever page is up now, the History page lists the run and Undo
        // offers it.
        take_recorded(s, recorded);
        s.status = status;
        crate::ui::toast::notify(s.status.clone(), failed);
        // A scan (or Back) since the act started: these findings are not the
        // ones it acted on, so the status above is all that changes.
        if s.scan_gen != generation {
            return;
        }
        if !done.is_empty() {
            // Rounds add up while they go the same way; a different way
            // starts the count again, so the card never mixes two outcomes.
            let (items, total) = match s.findings_done {
                Some(before) if before.went == went => (before.items, before.bytes),
                _ => (0, 0),
            };
            s.findings_done = Some(FindingsDone {
                items: items + done.len() as u64,
                bytes: total + bytes,
                went,
            });
        }
        if let Some(result) = s.mode_result.as_mut() {
            result.findings.retain(|f| !gone.contains(&f.path));
            for group in &mut result.groups {
                // Members, their sizes and their hashes are parallel lists.
                let keep: Vec<bool> = group.members.iter().map(|m| !gone.contains(m)).collect();
                let before_sizes = group.sizes.clone();
                let mut flags = keep.iter();
                group.members.retain(|_| *flags.next().unwrap_or(&true));
                let mut flags = keep.iter();
                group.sizes.retain(|_| *flags.next().unwrap_or(&true));
                let mut flags = keep.iter();
                group.hashes.retain(|_| *flags.next().unwrap_or(&true));
                // `bytes` counts every member here (music groups are the
                // only ones acted on): drop the ones that went.
                let removed: u64 = before_sizes
                    .iter()
                    .zip(&keep)
                    .filter(|(_, kept)| !**kept)
                    .map(|(size, _)| size)
                    .sum();
                group.bytes = group.bytes.saturating_sub(removed);
                group.reclaimable = group.sizes.iter().skip(1).sum();
                // The detail opens with the member count ("3 tracks, …").
                let before = keep.len();
                let after = group.members.len();
                if after != before
                    && let Some(rest) = group.detail.strip_prefix(&format!("{before} "))
                {
                    group.detail = format!("{after} {rest}");
                }
            }
            result.groups.retain(|g| g.members.len() >= 2);
            // Largest first, as the scan ordered them.
            result
                .groups
                .sort_by_key(|g| std::cmp::Reverse(g.reclaimable));
            s.dup_files = result.finding_count() as u64;
            s.reclaim_bytes = result.reclaimable();
        }
    });
}

// --- Report -----------------------------------------------------------------

/// Export the current results (the duplicate groups, or a cleanup scan's
/// findings) through the system save dialog. The file type picked there
/// decides the format: JSON as `--json`, CSV as `--csv`, HTML as `--html`.
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
        mode_result: s.mode_result.clone(),
    });
    let name = snapshot
        .mode_result
        .as_ref()
        .map_or("dedupe-report", |r| crate::modes::mode_name(r.mode));
    let file_name = format!("{name}.json");
    app_spawn(async move {
        let file = blocking(move || {
            rfd::FileDialog::new()
                .set_title("Export report")
                .set_file_name(&file_name)
                .add_filter("JSON", &["json"])
                .add_filter("CSV", &["csv"])
                .add_filter("HTML", &["html", "htm"])
                .save_file()
        })
        .await;
        let Ok(Some(path)) = file else { return };
        let outcome = write_report(&path, &snapshot);
        app().update(|s| match outcome {
            Ok(()) => {
                s.status = format!("Exported the report to {}.", path.display());
                crate::ui::toast::notify(s.status.clone(), false);
            }
            Err(e) => {
                s.status = format!("Couldn't export the report: {e}");
                crate::ui::toast::notify(s.status.clone(), true);
            }
        });
    });
}

/// The format a report path asks for, by its extension; anything else is
/// JSON, the dialog's first type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReportFormat {
    Json,
    Csv,
    Html,
}

#[must_use]
pub(crate) fn report_format(path: &Path) -> ReportFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("csv") => ReportFormat::Csv,
        Some("html" | "htm") => ReportFormat::Html,
        _ => ReportFormat::Json,
    }
}

/// Write `snapshot` to `path` in the format its extension names, with the
/// same writers the CLI's report flags use.
fn write_report(path: &Path, snapshot: &ReportSnapshot) -> std::io::Result<()> {
    let ctx = crate::report::ReportContext {
        paths: &snapshot.paths,
        hash_algo: &snapshot.hash_algo,
        verbose: false,
        ffprobe_used: snapshot.ffprobe_used,
    };
    let mode = snapshot.mode_result.as_ref();
    match report_format(path) {
        ReportFormat::Csv => {
            crate::report::write_csv(path, &ctx, &snapshot.stats, &snapshot.groups, mode)
        }
        ReportFormat::Html => {
            crate::report::write_html(path, &ctx, &snapshot.stats, &snapshot.groups, mode)
        }
        ReportFormat::Json => {
            let json = match mode {
                Some(result) => crate::report::to_mode_json(&ctx, result),
                None => crate::report::to_json(&ctx, &snapshot.stats, &snapshot.groups),
            };
            std::fs::write(path, json)
        }
    }
}

// --- Cache ------------------------------------------------------------------

/// Wipe the on-disk hash and fingerprint caches from the Settings page.
pub(crate) fn clear_cache() {
    let (files, bytes) = crate::cache::clear_caches();
    let msg = format!(
        "Cleared {} ({})",
        counted(files as u64, "file", "files"),
        util::human_bytes(bytes)
    );
    app().update(|s| {
        s.cache_msg = msg.clone();
        s.status = format!("Cache cleared: {msg}");
        crate::ui::toast::notify(s.status.clone(), false);
    });
}
