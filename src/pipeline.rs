//! Shared scan orchestration used by both the CLI and the GUI.
//!
//! [`run_scan`] performs the full detection pipeline (walk → hash → group →
//! perceptual similar pass → cache save) without printing anything, so both
//! front ends get identical results. Progress display stays with the caller:
//! pass indicatif bars from the CLI, `None` from the GUI.

use crate::cancel::CancelToken;
use crate::{cache, cli, hashing, matching, media, modes, report, scan, similar, util};
use anyhow::Result;
use indicatif::ProgressBar;
use std::collections::{HashMap, HashSet};

/// Coarse scan stages. Reported through the `on_phase` callback so callers
/// can show status (CLI prints a line, the GUI updates its status bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Scanning,
    Hashing,
    ComparingMedia,
    Finishing,
}

/// Short human label for a phase, shared by the CLI and GUI status output.
#[must_use]
pub fn phase_label(phase: ScanPhase) -> &'static str {
    match phase {
        ScanPhase::Scanning => "Scanning folders…",
        ScanPhase::Hashing => "Hashing files…",
        ScanPhase::ComparingMedia => "Comparing media…",
        ScanPhase::Finishing => "Building report…",
    }
}

/// Everything a front end needs to render a report after a scan.
#[derive(Debug)]
pub struct ScanOutput {
    pub paths: Vec<String>,
    pub hash_algo: &'static str,
    pub stats: report::ScanStats,
    pub groups: Vec<matching::Group>,
    pub ffprobe_used: bool,
    /// A media scan ran without ffprobe: resolution/duration are missing.
    pub needs_ffprobe_note: bool,
    /// Videos were scanned without ffmpeg: video similarity was skipped.
    pub needs_ffmpeg_note: bool,
    /// Non-fatal cache write failures, for the caller to surface.
    pub cache_save_errors: Vec<String>,
    /// Canonicalized reference dirs (for badges/tips).
    pub reference_dirs: Vec<std::path::PathBuf>,
    /// The scan was cancelled (Stop) part-way: `groups` is empty by
    /// design and `stats` covers whatever the walk completed.
    pub cancelled: bool,
    /// Set when a non-duplicate cleanup tool (`--find`) ran. The duplicate
    /// `groups` vector is empty in that case.
    pub mode_result: Option<modes::ModeResult>,
}

/// Walk stats for a cancelled scan: whatever the walk completed, plus
/// `hardlinks_skipped` from wherever the pipeline stopped.
fn partial_stats(scanned: &scan::ScanResult, hardlinks_skipped: u64) -> report::ScanStats {
    report::ScanStats {
        files_scanned: scanned.entries.len(),
        bytes_scanned: scanned.entries.iter().map(|e| e.size).sum(),
        dirs_skipped: scanned.dirs_skipped,
        files_skipped: scanned.files_skipped,
        hardlinks_skipped,
        dir_read_errors: scanned.dir_read_errors.clone(),
    }
}

/// Output for a cancelled scan: the partial walk stats, no groups.
/// Front ends discard results when `cancelled` is set (the GUI shows
/// "Scan cancelled" and keeps the pane empty).
fn cancelled_output(
    cli: &cli::Cli,
    engine: &hashing::HashEngine,
    stats: report::ScanStats,
) -> ScanOutput {
    ScanOutput {
        paths: cli.paths.clone(),
        hash_algo: engine.name(),
        stats,
        groups: Vec::new(),
        ffprobe_used: false,
        needs_ffprobe_note: false,
        needs_ffmpeg_note: false,
        cache_save_errors: Vec::new(),
        reference_dirs: Vec::new(),
        cancelled: true,
        mode_result: None,
    }
}

/// Run the full detection pipeline for `cli`.
///
/// `progress` spans the exact-duplicate hashing work and `similar_progress`
/// the perceptual pass; both are finished (cleared) here when given. Pass
/// `None` for headless/GUI callers. `cancel` is polled at every phase
/// boundary (walk, hashing, fingerprinting, group building): once set, the
/// pipeline stops at the next boundary and returns early with
/// [`ScanOutput::cancelled`] set. Headless callers pass a fresh, never-set
/// [`CancelToken`].
///
/// # Errors
///
/// Returns an error when `--similarity` is outside 0..=100, when
/// `--min-size`/`--max-size` cannot be parsed, or when the similar-group build
/// fails.
pub fn run_scan(
    cli: &cli::Cli,
    progress: Option<&ProgressBar>,
    similar_progress: Option<&ProgressBar>,
    on_phase: &dyn Fn(ScanPhase),
    cancel: &CancelToken,
) -> Result<ScanOutput> {
    if !(0.0..=100.0).contains(&cli.similarity) {
        anyhow::bail!(
            "--similarity must be between 0 and 100 (got {})",
            cli.similarity
        );
    }
    // Cap worker threads before any parallel stage runs: hashing,
    // ffprobe/ffmpeg fingerprinting and group assembly all share rayon's
    // global pool, so sizing it here throttles every stage at once. The
    // default leaves one core free for the rest of the system.
    crate::util::init_thread_pool(cli.jobs);
    let keep_modes = [
        cli.keep_smaller,
        cli.keep_newest,
        cli.keep_oldest,
        cli.keep_best_quality,
    ];
    if keep_modes.iter().filter(|&&b| b).count() > 1 {
        anyhow::bail!(
            "--keep-smaller, --keep-newest, --keep-oldest and --keep-best-quality conflict; pick one"
        );
    }
    let keep = if cli.keep_smaller {
        matching::KeepMode::Smallest
    } else if cli.keep_newest {
        matching::KeepMode::Newest
    } else if cli.keep_oldest {
        matching::KeepMode::Oldest
    } else if cli.keep_best_quality {
        matching::KeepMode::BestQuality
    } else {
        matching::KeepMode::First
    };
    let reference_dirs = matching::canonicalize_refs(&cli.reference_dir);
    let mut cache_save_errors: Vec<String> = Vec::new();
    let similar_threshold = cli.similarity / 100.0;
    let engine = hashing::HashEngine::new(cli.hash);
    if cli.find != cli::FindMode::Duplicates {
        let mode_result = modes::run_mode(cli, cancel);
        let stats = report::ScanStats {
            files_scanned: mode_result.files_scanned as usize,
            bytes_scanned: mode_result.bytes_scanned,
            dirs_skipped: mode_result.dirs_skipped,
            files_skipped: mode_result.files_skipped,
            hardlinks_skipped: 0,
            dir_read_errors: mode_result.errors.clone(),
        };
        let cancelled = mode_result.cancelled;
        return Ok(ScanOutput {
            paths: cli.paths.clone(),
            hash_algo: engine.name(),
            stats,
            groups: Vec::new(),
            ffprobe_used: false,
            needs_ffprobe_note: false,
            needs_ffmpeg_note: false,
            cache_save_errors: Vec::new(),
            reference_dirs: Vec::new(),
            cancelled,
            mode_result: Some(mode_result),
        });
    }
    let size_min = cli
        .min_size
        .as_deref()
        .map(util::parse_size)
        .transpose()?
        .unwrap_or(0);
    let size_max = cli
        .max_size
        .as_deref()
        .map(util::parse_size)
        .transpose()?
        .unwrap_or(0);

    on_phase(ScanPhase::Scanning);
    let scanned = scan::scan(cli, size_min, size_max, cancel);
    if cancel.is_cancelled() {
        return Ok(cancelled_output(cli, &engine, partial_stats(&scanned, 0)));
    }

    if cli.r#match != cli::MatchMode::Content {
        let files_scanned = scanned.entries.len();
        let bytes_scanned: u64 = scanned.entries.iter().map(|e| e.size).sum();
        let buckets = modes::match_entries(&scanned.entries, cli.r#match, cli.fuzzy_threshold);
        let duplicate_groups: Vec<hashing::DuplicateGroup> = buckets
            .into_iter()
            .map(|bucket| hashing::DuplicateGroup {
                hash: bucket.key,
                members: bucket.entries,
            })
            .collect();
        let groups = matching::assemble_groups(duplicate_groups, keep, &reference_dirs, false);
        let stats = report::ScanStats {
            files_scanned,
            bytes_scanned,
            dirs_skipped: scanned.dirs_skipped,
            files_skipped: scanned.files_skipped,
            hardlinks_skipped: 0,
            dir_read_errors: scanned.dir_read_errors,
        };
        return Ok(ScanOutput {
            paths: cli.paths.clone(),
            hash_algo: engine.name(),
            stats,
            groups,
            ffprobe_used: false,
            needs_ffprobe_note: false,
            needs_ffmpeg_note: false,
            cache_save_errors,
            reference_dirs,
            cancelled: false,
            mode_result: None,
        });
    }

    let has_media_candidates = scanned
        .entries
        .iter()
        .any(|e| media::classify(&e.path) != media::MediaKind::Other);
    let ffprobe_used = has_media_candidates && media::ffprobe_available();

    on_phase(ScanPhase::Hashing);
    let files_scanned = scanned.entries.len();
    let bytes_scanned: u64 = scanned.entries.iter().map(|e| e.size).sum();
    let entries = scanned.entries;
    let similar_entries = if cli.exact {
        Vec::new()
    } else {
        entries.clone()
    };
    // Content-hash cache: repeat scans of unchanged files skip re-hashing
    // entirely. --no-cache bypasses it; the delete safety re-hash in
    // actions never reads from it.
    let mut hash_cache = if cli.no_cache {
        None
    } else {
        Some(cache::HashCache::load())
    };
    let (groups, hardlinks_skipped) =
        hashing::find_duplicate_groups(entries, &engine, hash_cache.as_mut(), progress, cancel);
    if cancel.is_cancelled() {
        // `scanned.entries` was consumed by hashing; the walk fields survive.
        let stats = report::ScanStats {
            files_scanned,
            bytes_scanned,
            dirs_skipped: scanned.dirs_skipped,
            files_skipped: scanned.files_skipped,
            hardlinks_skipped,
            dir_read_errors: scanned.dir_read_errors,
        };
        return Ok(cancelled_output(cli, &engine, stats));
    }
    if let Some(c) = &hash_cache
        && let Err(e) = c.save()
    {
        cache_save_errors.push(format!("content-hash cache: {e}"));
    }

    let stats = report::ScanStats {
        files_scanned,
        bytes_scanned,
        dirs_skipped: scanned.dirs_skipped,
        files_skipped: scanned.files_skipped,
        hardlinks_skipped,
        dir_read_errors: scanned.dir_read_errors,
    };

    let mut groups = matching::assemble_groups(groups, keep, &reference_dirs, ffprobe_used);

    // Perceptual fingerprint cache: loaded only when the similar pass runs,
    // saved (best-effort) after it. --no-cache / --exact bypass it entirely.
    let mut cache = if cli.exact || cli.no_cache {
        None
    } else {
        Some(cache::FingerprintCache::load())
    };

    let mut needs_ffmpeg_note = false;
    if !cli.exact {
        on_phase(ScanPhase::ComparingMedia);
        let has_videos = similar_entries
            .iter()
            .any(|e| media::classify(&e.path) == media::MediaKind::Video);
        needs_ffmpeg_note = has_videos && !media::ffmpeg_available();
        // Similar pass: leftovers PLUS the keeper of every exact media
        // group. The keeper is the comparison target — without it a
        // rescaled or re-encoded variant would have nothing to match
        // against and be reported as its own (or no) group, even though
        // its byte-identical siblings sit right there.
        let keeper_group: HashMap<std::path::PathBuf, usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.similarity.is_none())
            .filter_map(|(gi, g)| {
                g.members
                    .iter()
                    .find(|m| m.keep)
                    .map(|m| (m.path.clone(), gi))
            })
            .collect();
        let exact_members: HashSet<std::path::PathBuf> = groups
            .iter()
            .flat_map(|g| g.members.iter().map(|m| m.path.clone()))
            .collect();
        // Only media keepers seed the pool: non-media exact groups can
        // never match perceptually, and find_similar's bucket pruning
        // keeps a lone keeper from ever being decoded.
        let keepers: HashSet<std::path::PathBuf> = keeper_group
            .keys()
            .filter(|p| media::classify(p) != media::MediaKind::Other)
            .cloned()
            .collect();
        let candidates: Vec<scan::FileEntry> = similar_entries
            .into_iter()
            .filter(|e| !exact_members.contains(&e.path) || keepers.contains(&e.path))
            .collect();
        let similar_cfg = similar::SimilarConfig {
            threshold: similar_threshold,
            image_hash: cli.image_hash,
            partial_clips: cli.partial_clips,
            partial_similarity: cli.partial_similarity,
        };
        let (similar, writes) = similar::find_similar(
            candidates,
            &similar_cfg,
            ffprobe_used,
            cache.as_ref(),
            similar_progress,
            &keepers,
            cancel,
        );
        if cancel.is_cancelled() {
            // Save the fingerprints that did complete, then stop: merging
            // and group building would only re-hash work for a scan whose
            // results are about to be discarded.
            if let Some(c) = &mut cache {
                for w in writes {
                    c.insert_write(w);
                }
                let _ = c.save();
            }
            return Ok(cancelled_output(cli, &engine, stats));
        }
        if let Some(c) = &mut cache {
            for w in writes {
                c.insert_write(w);
            }
            // A cache write failure (read-only home dir, ...) is non-fatal:
            // remember it for the caller to surface; the scan still succeeds.
            if let Err(e) = c.save() {
                cache_save_errors.push(format!("fingerprint cache: {e}"));
            }
        }
        // Clusters that matched an exact keeper merge into that group; the
        // rest become fresh similar groups.
        let leftover = similar::merge_into_exact_groups(
            &mut groups,
            similar,
            &keeper_group,
            &reference_dirs,
            ffprobe_used,
            &engine,
        );
        let built = if leftover.is_empty() {
            Vec::new()
        } else {
            similar::build_similar_groups(
                leftover,
                keep,
                &reference_dirs,
                ffprobe_used,
                &engine,
                groups.len() + 1,
                similar_progress,
                cancel,
            )
        };
        if let Some(bar) = similar_progress {
            bar.finish_and_clear();
        }
        groups.extend(built);
    }

    on_phase(ScanPhase::Finishing);
    if cancel.is_cancelled() {
        return Ok(cancelled_output(cli, &engine, stats));
    }
    Ok(ScanOutput {
        paths: cli.paths.clone(),
        hash_algo: engine.name(),
        stats,
        groups,
        ffprobe_used,
        needs_ffprobe_note: has_media_candidates && !ffprobe_used,
        needs_ffmpeg_note,
        cache_save_errors,
        reference_dirs,
        cancelled: false,
        mode_result: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cli(paths: Vec<String>) -> cli::Cli {
        cli::Cli {
            paths,
            max_depth: None,
            types: vec![],
            keep_smaller: false,
            keep_newest: false,
            keep_oldest: false,
            keep_best_quality: false,
            consolidate_dir: None,
            reference_dir: vec![],
            trash: false,
            dry_run: false,
            delete: false,
            yes: false,
            hash: cli::HashAlgo::Blake3,
            exact: true,
            similarity: 97.0,
            no_cache: true,
            clear_cache: false,
            min_size: None,
            max_size: None,
            exclude_dir: vec![],
            exclude_path: vec![],
            json: false,
            verbose: false,
            quiet: true,
            jobs: 0,
            gui: false,
            find: cli::FindMode::Duplicates,
            r#match: cli::MatchMode::Content,
            fuzzy_threshold: 85.0,
            top: 50,
            image_hash: cli::ImageHashAlgo::Auto,
            partial_clips: false,
            partial_similarity: 85.0,
            music_by: cli::MusicMode::Tags,
            csv: None,
            html: None,
            move_dir: None,
            hardlink: false,
            strip_exif: false,
            ignore: vec![],
        }
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dedupe-pipeline-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn phase_labels_cover_every_phase() {
        assert_eq!(phase_label(ScanPhase::Scanning), "Scanning folders…");
        assert_eq!(phase_label(ScanPhase::Hashing), "Hashing files…");
        assert_eq!(phase_label(ScanPhase::ComparingMedia), "Comparing media…");
        assert_eq!(phase_label(ScanPhase::Finishing), "Building report…");
    }

    #[test]
    fn rejects_similarity_out_of_range() {
        let seen = std::sync::Mutex::new(Vec::new());
        for bad in [101.0, -1.0, f64::NAN] {
            let mut cli = test_cli(vec![".".to_string()]);
            cli.similarity = bad;
            let err = run_scan(
                &cli,
                None,
                None,
                &|p| {
                    seen.lock().unwrap().push(p);
                },
                &CancelToken::new(),
            )
            .unwrap_err();
            assert!(
                err.to_string().contains("--similarity"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn rejects_conflicting_keep_flags() {
        let mut cli = test_cli(vec![".".to_string()]);
        cli.keep_smaller = true;
        cli.keep_newest = true;
        let err = run_scan(&cli, None, None, &|_| {}, &CancelToken::new()).unwrap_err();
        assert!(err.to_string().contains("conflict"), "{err}");
    }

    #[test]
    fn rejects_unparsable_size_filter() {
        let mut cli = test_cli(vec![".".to_string()]);
        cli.min_size = Some("not-a-size".to_string());
        assert!(run_scan(&cli, None, None, &|_| {}, &CancelToken::new()).is_err());
    }

    #[test]
    fn run_scan_finds_exact_duplicates_and_reports_phases() {
        let dir = tmpdir("exact");
        std::fs::write(dir.join("a.txt"), b"pipeline payload").unwrap();
        std::fs::write(dir.join("b.txt"), b"pipeline payload").unwrap();
        std::fs::write(dir.join("unique.txt"), b"something else").unwrap();

        let seen = std::sync::Mutex::new(Vec::new());
        let out = run_scan(
            &test_cli(vec![dir.to_str().unwrap().to_string()]),
            None,
            None,
            &|p| {
                seen.lock().unwrap().push(p);
            },
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(out.groups.len(), 1);
        assert_eq!(out.groups[0].members.len(), 2);
        assert_eq!(out.stats.files_scanned, 3);
        assert_eq!(out.hash_algo, "blake3");
        // All four phases fire in order.
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                ScanPhase::Scanning,
                ScanPhase::Hashing,
                ScanPhase::Finishing
            ]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preset_cancel_flag_stops_before_any_work() {
        let dir = tmpdir("cancel");
        std::fs::write(dir.join("a.txt"), b"payload").unwrap();
        std::fs::write(dir.join("b.txt"), b"payload").unwrap();

        let cancel = CancelToken::new();
        cancel.cancel();
        let out = run_scan(
            &test_cli(vec![dir.to_str().unwrap().to_string()]),
            None,
            None,
            &|_| {},
            &cancel,
        )
        .unwrap();
        assert!(out.cancelled, "a preset flag must cancel immediately");
        assert!(out.groups.is_empty(), "no groups on a cancelled scan");
        assert_eq!(out.stats.files_scanned, 0, "the walk stops at entry one");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleared_token_runs_normally() {
        // Regression guard: reset() must give the next scan a clean flag.
        let dir = tmpdir("reset");
        std::fs::write(dir.join("a.txt"), b"payload").unwrap();
        std::fs::write(dir.join("b.txt"), b"payload").unwrap();

        let cancel = CancelToken::new();
        cancel.cancel();
        cancel.reset();
        let out = run_scan(
            &test_cli(vec![dir.to_str().unwrap().to_string()]),
            None,
            None,
            &|_| {},
            &cancel,
        )
        .unwrap();
        assert!(!out.cancelled);
        assert_eq!(out.groups.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_scan_dispatches_cleanup_modes_without_duplicate_groups() {
        let dir = tmpdir("mode");
        std::fs::write(dir.join("empty.txt"), b"").unwrap();
        std::fs::write(dir.join("full.txt"), b"content").unwrap();

        let mut cli = test_cli(vec![dir.to_str().unwrap().to_string()]);
        cli.find = cli::FindMode::EmptyFiles;
        let out = run_scan(&cli, None, None, &|_| {}, &CancelToken::new()).unwrap();

        assert!(
            out.groups.is_empty(),
            "cleanup modes have no duplicate groups"
        );
        assert_eq!(out.stats.files_scanned, 2);
        let mode = out.mode_result.expect("cleanup result is attached");
        assert_eq!(mode.mode, cli::FindMode::EmptyFiles);
        assert_eq!(mode.findings.len(), 1);
        assert!(mode.findings[0].path.ends_with("empty.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_scan_on_empty_dir_succeeds_with_no_groups() {
        let dir = tmpdir("empty");
        let out = run_scan(
            &test_cli(vec![dir.to_str().unwrap().to_string()]),
            None,
            None,
            &|_| {},
            &CancelToken::new(),
        )
        .unwrap();
        assert!(out.groups.is_empty());
        assert_eq!(out.stats.files_scanned, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The user-facing bug: an exact-duplicate pair PLUS a downscaled
    /// re-encode of the same picture must land in ONE group. The old
    /// pipeline dropped every exact-group member from the similar-pass
    /// candidate pool, so the variant had nothing to match against and
    /// the pair stayed a group of two.
    #[test]
    fn downscaled_variant_joins_its_exact_group() {
        use image::ImageBuffer;
        let dir = tmpdir("merge");
        // Smooth gradient: survives a 4x downscale under dHash.
        let img: ImageBuffer<image::Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(400, 300, |x, y| {
            image::Rgb([(x * 255 / 400) as u8, (y * 255 / 300) as u8, 128])
        });
        img.save(dir.join("photo.png")).unwrap();
        std::fs::copy(dir.join("photo.png"), dir.join("photo-copy.png")).unwrap();
        let small = image::imageops::resize(&img, 100, 75, image::imageops::FilterType::Triangle);
        small.save(dir.join("photo-small.png")).unwrap();

        let mut cli = test_cli(vec![dir.to_str().unwrap().to_string()]);
        cli.exact = false;
        cli.similarity = 90.0;
        let out = run_scan(&cli, None, None, &|_| {}, &CancelToken::new()).unwrap();

        assert_eq!(out.groups.len(), 1, "one picture => one group");
        let g = &out.groups[0];
        assert_eq!(
            g.members.len(),
            3,
            "the downscaled variant must join the exact group, got {:?}",
            g.members.iter().map(|m| m.path.clone()).collect::<Vec<_>>()
        );
        assert!(
            g.members
                .iter()
                .any(|m| m.path.ends_with("photo-small.png"))
        );
        // The merged group's identity is now its worst member match.
        assert!(g.similarity.is_some_and(|s| s >= 0.9));
        assert_eq!(g.members.iter().filter(|m| m.keep).count(), 1);
        // The variant carries a similarity score and a deletion-safety hash.
        let small = g
            .members
            .iter()
            .find(|m| m.path.ends_with("photo-small.png"))
            .unwrap();
        assert!(!small.keep);
        assert!(small.similarity.is_some_and(|s| s >= 0.9));
        assert!(small.content_hash.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The user's actual Pictures shape: the smaller variant is itself
    /// duplicated byte-for-byte, so the scan finds TWO exact groups of the
    /// same picture (the big pair + the small pair). Both must fold into
    /// ONE group of 4.
    #[test]
    fn duplicated_small_variant_folds_into_one_group() {
        use image::ImageBuffer;
        let dir = tmpdir("fold");
        let img: ImageBuffer<image::Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(400, 300, |x, y| {
            image::Rgb([(x * 255 / 400) as u8, (y * 255 / 300) as u8, 128])
        });
        img.save(dir.join("photo.png")).unwrap();
        std::fs::copy(dir.join("photo.png"), dir.join("photo-copy.png")).unwrap();
        let small = image::imageops::resize(&img, 100, 75, image::imageops::FilterType::Triangle);
        small.save(dir.join("small.png")).unwrap();
        std::fs::copy(dir.join("small.png"), dir.join("small-copy.png")).unwrap();

        let mut cli = test_cli(vec![dir.to_str().unwrap().to_string()]);
        cli.exact = false;
        cli.similarity = 90.0;
        let out = run_scan(&cli, None, None, &|_| {}, &CancelToken::new()).unwrap();

        assert_eq!(out.groups.len(), 1, "one picture => one group");
        let g = &out.groups[0];
        assert_eq!(g.members.len(), 4, "both exact pairs fold together");
        assert!(
            g.similarity.is_some_and(|s| s >= 0.9),
            "merged group identity = worst member match"
        );
        // Exactly one keeper, and it belongs to the big pair.
        let keepers: Vec<&str> = g
            .members
            .iter()
            .filter(|m| m.keep)
            .map(|m| m.path.to_str().unwrap())
            .collect();
        assert_eq!(keepers.len(), 1, "one keeper, got {keepers:?}");
        assert!(keepers[0].contains("photo"));
        // Both smalls carry a similarity score; both bigs stay exact.
        let smalls: Vec<_> = g
            .members
            .iter()
            .filter(|m| m.path.to_string_lossy().contains("small"))
            .collect();
        assert_eq!(smalls.len(), 2);
        for s in smalls {
            assert!(s.similarity.is_some_and(|v| v >= 0.9));
            assert!(s.content_hash.is_some());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
