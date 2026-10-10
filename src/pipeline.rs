//! Shared scan orchestration used by both the CLI and the GUI.
//!
//! [`run_scan`] performs the full detection pipeline (walk → hash → group →
//! perceptual similar pass → cache save) without printing anything, so both
//! front ends get identical results. Progress display stays with the caller:
//! pass indicatif bars from the CLI, `None` from the GUI.

use crate::{cache, cli, hashing, matching, media, report, scan, similar, util};
use anyhow::Result;
use indicatif::ProgressBar;

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
}

/// Run the full detection pipeline for `cli`.
///
/// `progress` spans the exact-duplicate hashing work and `similar_progress`
/// the perceptual pass; both are finished (cleared) here when given. Pass
/// `None` for headless/GUI callers.
pub fn run_scan(
    cli: &cli::Cli,
    progress: Option<&ProgressBar>,
    similar_progress: Option<&ProgressBar>,
    on_phase: &dyn Fn(ScanPhase),
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
    let scanned = scan::scan(cli, size_min, size_max);

    let has_media_candidates = scanned
        .entries
        .iter()
        .any(|e| media::classify(&e.path) != media::MediaKind::Other);
    let ffprobe_used = has_media_candidates && media::ffprobe_available();

    on_phase(ScanPhase::Hashing);
    let engine = hashing::HashEngine::new(cli.hash);
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
        hashing::find_duplicate_groups(entries, &engine, hash_cache.as_mut(), progress)?;
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
        let exact_paths: std::collections::HashSet<std::path::PathBuf> = groups
            .iter()
            .flat_map(|g| g.members.iter().map(|m| m.path.clone()))
            .collect();
        let candidates: Vec<scan::FileEntry> = similar_entries
            .into_iter()
            .filter(|e| !exact_paths.contains(&e.path))
            .collect();
        let similar_cfg = similar::SimilarConfig {
            threshold: similar_threshold,
        };
        let (similar, writes) = similar::find_similar(
            candidates,
            &similar_cfg,
            ffprobe_used,
            cache.as_ref(),
            similar_progress,
        );
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
        let built = if !similar.is_empty() {
            similar::build_similar_groups(
                similar,
                keep,
                &reference_dirs,
                ffprobe_used,
                &engine,
                groups.len() + 1,
                similar_progress,
            )?
        } else {
            Vec::new()
        };
        if let Some(bar) = similar_progress {
            bar.finish_and_clear();
        }
        groups.extend(built);
    }

    on_phase(ScanPhase::Finishing);
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
            min_size: None,
            max_size: None,
            exclude_dir: vec![],
            exclude_path: vec![],
            json: false,
            verbose: false,
            quiet: true,
            jobs: 0,
            gui: false,
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
            let err = run_scan(&cli, None, None, &|p| {
                seen.lock().unwrap().push(p);
            })
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
        let err = run_scan(&cli, None, None, &|_| {}).unwrap_err();
        assert!(err.to_string().contains("conflict"), "{err}");
    }

    #[test]
    fn rejects_unparsable_size_filter() {
        let mut cli = test_cli(vec![".".to_string()]);
        cli.min_size = Some("not-a-size".to_string());
        assert!(run_scan(&cli, None, None, &|_| {}).is_err());
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
    fn run_scan_on_empty_dir_succeeds_with_no_groups() {
        let dir = tmpdir("empty");
        let out = run_scan(
            &test_cli(vec![dir.to_str().unwrap().to_string()]),
            None,
            None,
            &|_| {},
        )
        .unwrap();
        assert!(out.groups.is_empty());
        assert_eq!(out.stats.files_scanned, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
