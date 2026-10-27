use anyhow::Result;
use clap::Parser;
use console::style;
use dedupe::pipeline::{self, ScanPhase};
use dedupe::{cache, cancel, cli, report};

fn main() {
    let t0 = std::time::Instant::now();
    init_colors();
    let cli = cli::Cli::parse();
    // Clearing is a side effect, not a scan: handle it before deciding
    // between GUI and CLI so `dedupe --clear-cache` never opens a window.
    if cli.clear_cache {
        let (files, bytes) = cache::clear_caches();
        if !cli.quiet {
            eprintln!(
                "{} Cleared {} cache file(s) ({}).",
                style("✔").green().bold(),
                files,
                dedupe::util::human_bytes(bytes),
            );
        }
        if cli.paths.is_empty() {
            return;
        }
    }
    // GUI mode: explicit --gui, or no scan path given. clap still handles
    // `dedupe --help` / `dedupe --version` (exit 0) before we get here.
    if cli.gui || cli.paths.is_empty() {
        eprintln!("[startup] args parsed in {:?}", t0.elapsed());
        dedupe::ui::run();
        return;
    }
    if let Err(e) = run(&cli) {
        eprintln!("{} {e:#}", style("error:").red().bold());
        std::process::exit(1);
    }
}

/// Colors are enabled only when stdout is a terminal and `NO_COLOR` is unset,
/// so redirected / piped output and CI logs stay plain and parseable.
fn init_colors() {
    let no_color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let enabled = !no_color && console::Term::stdout().is_term();
    console::set_colors_enabled(enabled);
}

/// Build the animated progress bar used during scanning/hashing/fingerprinting,
/// or `None` when progress output is disabled (quiet/JSON modes).
fn new_progress_bar(enabled: bool) -> Option<indicatif::ProgressBar> {
    if !enabled {
        return None;
    }
    let bar = indicatif::ProgressBar::new(0);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.cyan} [{bar:40.cyan/blue}] {pos}/{len} files {msg:.dim}",
        )
        .ok()?
        .progress_chars("█▉▊▋▌▍▎▏  ")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    Some(bar)
}

fn run(cli: &cli::Cli) -> Result<()> {
    if !cli.quiet && !cli.json {
        eprintln!(
            "{} scanning {} path(s)...",
            style("◆").cyan().bold(),
            cli.paths.len()
        );
    }

    let progress_bar = new_progress_bar(!cli.quiet && !cli.json);
    let similar_bar = new_progress_bar(!cli.quiet && !cli.json);
    let on_phase = |phase: ScanPhase| {
        if cli.quiet || cli.json {
            return;
        }
        if phase == ScanPhase::ComparingMedia {
            eprintln!(
                "{} comparing media by perceptual hash (threshold {}%)...",
                style("◆").cyan().bold(),
                cli.similarity
            );
        }
    };
    // The CLI has no Stop button; a fresh token is never set.
    let cancel = cancel::CancelToken::new();

    if cli.strip_exif {
        let exif = dedupe::modes::strip_exif(cli, &cancel);
        if !cli.quiet && !cli.json {
            if exif.applied {
                eprintln!(
                    "{} Removed EXIF/XMP metadata from {} of {} image(s) ({} saved); {} skipped.",
                    style("✔").green().bold(),
                    style(exif.rewritten).green().bold(),
                    exif.images_scanned,
                    style(dedupe::util::human_bytes(exif.bytes_saved))
                        .green()
                        .bold(),
                    exif.skipped
                );
            } else if exif.images_with_metadata > 0 {
                eprintln!(
                    "{} Dry run: {} of {} image(s) carry EXIF/XMP metadata ({} would be saved); nothing was changed.",
                    style("◌").cyan().bold(),
                    style(exif.images_with_metadata).cyan().bold(),
                    exif.images_scanned,
                    style(dedupe::util::human_bytes(exif.bytes_saved))
                        .cyan()
                        .bold(),
                );
            } else {
                eprintln!(
                    "{} No EXIF/XMP metadata found in {} image(s).",
                    style("◌").dim(),
                    exif.images_scanned
                );
            }
        }
    }

    let out = pipeline::run_scan(
        cli,
        progress_bar.as_ref(),
        similar_bar.as_ref(),
        &on_phase,
        &cancel,
    )?;

    if out.needs_ffprobe_note && !cli.quiet && !cli.json {
        eprintln!(
            "{} ffprobe not found on PATH; image/video resolution and duration metadata will not be reported (install FFmpeg to enable)",
            style("⚠").yellow()
        );
    }
    if out.needs_ffmpeg_note && !cli.quiet && !cli.json {
        eprintln!(
            "{} ffmpeg not found on PATH; videos will not be compared for similarity (images still are)",
            style("⚠").yellow()
        );
    }
    if !out.cache_save_errors.is_empty() && !cli.quiet && !cli.json {
        for e in &out.cache_save_errors {
            eprintln!("{} could not write {e}", style("⚠").yellow());
        }
    }

    let ctx = report::ReportContext {
        paths: &out.paths,
        hash_algo: out.hash_algo,
        verbose: cli.verbose,
        ffprobe_used: out.ffprobe_used,
    };

    if let Some(mode_result) = &out.mode_result {
        if cli.json {
            report::print_mode_json(&ctx, mode_result);
        } else {
            report::print_mode_human(&ctx, mode_result);
        }
        write_exports(cli, &ctx, &out.stats, &out.groups, Some(mode_result))?;
        run_mode_actions(cli, mode_result)?;
        return Ok(());
    }

    if cli.json {
        report::print_json(&ctx, &out.stats, &out.groups);
    } else {
        report::print_human(&ctx, &out.stats, &out.groups);
    }
    write_exports(cli, &ctx, &out.stats, &out.groups, None)?;

    let consolidate_dest: Option<std::path::PathBuf> =
        cli.consolidate_dir.as_ref().map(std::path::PathBuf::from);

    if cli.move_dir.is_some() && cli.hardlink {
        anyhow::bail!("--move-dir and --hardlink cannot be combined");
    }

    if cli.delete || consolidate_dest.is_some() || cli.move_dir.is_some() || cli.hardlink {
        if out.groups.is_empty() {
            if !cli.quiet {
                let msg = if cli.move_dir.is_some() {
                    "Nothing to move."
                } else if cli.hardlink {
                    "Nothing to hard-link."
                } else if cli.delete {
                    "Nothing to delete."
                } else {
                    "Nothing to consolidate."
                };
                eprintln!("{}", style(msg).dim());
            }
        } else {
            let engine = dedupe::hashing::HashEngine::new(cli.hash);
            // Consolidate first: keepers land in place before dups go, so a
            // combined --consolidate-dir + --delete run ends with one copy
            // in the consolidate directory and nothing elsewhere.
            if let Some(dest) = &consolidate_dest {
                if cli.dry_run {
                    dry_run_consolidate(&out.groups, dest, cli.quiet);
                } else {
                    let moved =
                        dedupe::actions::consolidate_keepers(&out.groups, dest, &engine, cli.yes);
                    println!(
                        "{} Consolidated {} file(s) ({}) into {}; {} skipped.",
                        style("✔").green().bold(),
                        style(moved.files_moved).green().bold(),
                        style(dedupe::util::human_bytes(moved.bytes_moved))
                            .green()
                            .bold(),
                        dest.display(),
                        moved.files_skipped
                    );
                }
            }
            if let Some(dest) = &cli.move_dir {
                let dest = std::path::PathBuf::from(dest);
                if cli.dry_run {
                    let mut count = 0u64;
                    let mut bytes = 0u64;
                    for group in &out.groups {
                        for member in group.members.iter().filter(|m| !m.keep) {
                            count += 1;
                            bytes += member.size;
                            if !cli.quiet && !cli.json {
                                println!(
                                    "would move {} -> {} ({})",
                                    member.path.display(),
                                    dest.display(),
                                    dedupe::util::human_bytes(member.size)
                                );
                            }
                        }
                    }
                    println!(
                        "{} Dry run: would move {} duplicate file(s) ({}); nothing was moved.",
                        style("◌").cyan().bold(),
                        style(count).cyan().bold(),
                        style(dedupe::util::human_bytes(bytes)).cyan().bold(),
                    );
                } else {
                    let moved =
                        dedupe::actions::move_duplicates(&out.groups, &dest, &engine, cli.yes)?;
                    println!(
                        "{} Moved {} duplicate file(s) ({}) into {}; {} skipped.",
                        style("✔").green().bold(),
                        style(moved.files_moved).green().bold(),
                        style(dedupe::util::human_bytes(moved.bytes_moved))
                            .green()
                            .bold(),
                        dest.display(),
                        moved.files_skipped
                    );
                }
            } else if cli.hardlink {
                if cli.dry_run {
                    let count: u64 = out
                        .groups
                        .iter()
                        .map(|g| g.members.iter().filter(|m| !m.keep).count() as u64)
                        .sum();
                    let bytes: u64 = out
                        .groups
                        .iter()
                        .flat_map(|g| g.members.iter().filter(|m| !m.keep))
                        .map(|m| m.size)
                        .sum();
                    println!(
                        "{} Dry run: would replace {} duplicate file(s) with hard links ({} saved); nothing was changed.",
                        style("◌").cyan().bold(),
                        style(count).cyan().bold(),
                        style(dedupe::util::human_bytes(bytes)).cyan().bold(),
                    );
                } else {
                    let linked =
                        dedupe::actions::hardlink_duplicates(&out.groups, &engine, cli.yes);
                    println!(
                        "{} Hard-linked {} duplicate file(s) ({} saved); {} skipped.",
                        style("✔").green().bold(),
                        style(linked.linked).green().bold(),
                        style(dedupe::util::human_bytes(linked.bytes_saved))
                            .green()
                            .bold(),
                        linked.files_skipped
                    );
                }
            } else if cli.delete {
                let disposition = if cli.trash {
                    dedupe::actions::Disposition::Trash
                } else {
                    dedupe::actions::Disposition::Permanent
                };
                if cli.dry_run {
                    // List what would go, touch nothing.
                    let mut count = 0u64;
                    let mut bytes = 0u64;
                    for group in &out.groups {
                        let mut group_count = 0u64;
                        let mut group_bytes = 0u64;
                        for member in &group.members {
                            if member.keep {
                                continue;
                            }
                            group_count += 1;
                            group_bytes += member.size;
                            if !cli.quiet && !cli.json {
                                println!(
                                    "would delete {} ({})",
                                    member.path.display(),
                                    dedupe::util::human_bytes(member.size)
                                );
                            }
                        }
                        count += group_count;
                        bytes += group_bytes;
                        if group_count > 0 && !cli.quiet && !cli.json {
                            println!(
                                "Group #{}: would delete {} file(s) ({})",
                                group.index,
                                group_count,
                                dedupe::util::human_bytes(group_bytes),
                            );
                        }
                    }
                    println!(
                        "{} Dry run: would {} {} file(s) ({}); nothing was removed.",
                        style("◌").cyan().bold(),
                        if cli.trash { "move to trash" } else { "delete" },
                        style(count).cyan().bold(),
                        style(dedupe::util::human_bytes(bytes)).cyan().bold(),
                    );
                } else {
                    let deletion = dedupe::actions::delete_duplicates(
                        &out.groups,
                        &engine,
                        cli.yes,
                        disposition,
                    );
                    let verb = if cli.trash {
                        "Moved to trash"
                    } else {
                        "Deleted"
                    };
                    println!(
                        "{} {verb} {} file(s) ({} freed); {} skipped.",
                        style("✔").green().bold(),
                        style(deletion.files_deleted).green().bold(),
                        style(dedupe::util::human_bytes(deletion.bytes_freed))
                            .green()
                            .bold(),
                        deletion.files_skipped
                    );
                }
            }
        }
    }

    Ok(())
}

fn write_exports(
    cli: &cli::Cli,
    ctx: &report::ReportContext,
    stats: &report::ScanStats,
    groups: &[dedupe::matching::Group],
    mode_result: Option<&dedupe::modes::ModeResult>,
) -> Result<()> {
    if let Some(path) = &cli.csv {
        let path = std::path::Path::new(path);
        report::write_csv(path, ctx, stats, groups, mode_result)?;
        if !cli.quiet && !cli.json {
            eprintln!(
                "{} CSV report written to {}",
                style("✔").green().bold(),
                path.display()
            );
        }
    }
    if let Some(path) = &cli.html {
        let path = std::path::Path::new(path);
        report::write_html(path, ctx, stats, groups, mode_result)?;
        if !cli.quiet && !cli.json {
            eprintln!(
                "{} HTML report written to {}",
                style("✔").green().bold(),
                path.display()
            );
        }
    }
    Ok(())
}

fn run_mode_actions(cli: &cli::Cli, result: &dedupe::modes::ModeResult) -> Result<()> {
    if !cli.delete && cli.move_dir.is_none() && !cli.hardlink {
        return Ok(());
    }
    if cli.move_dir.is_some() || cli.hardlink {
        anyhow::bail!(
            "--move-dir and --hardlink apply to duplicate matching, not --find cleanup modes"
        );
    }
    if result.mode == cli::FindMode::DuplicateFolders {
        if !cli.quiet {
            eprintln!(
                "{} duplicate folder trees are report-only; remove them manually after review",
                style("⚠").yellow()
            );
        }
        return Ok(());
    }
    if result.is_empty() {
        if !cli.quiet {
            eprintln!("{}", style("Nothing to delete.").dim());
        }
        return Ok(());
    }
    if cli.dry_run {
        println!(
            "{} Dry run: would delete {} finding(s) and {} duplicate group member(s); nothing was removed.",
            style("◌").cyan().bold(),
            result.findings.len(),
            result
                .groups
                .iter()
                .map(|g| g.members.len().saturating_sub(1))
                .sum::<usize>(),
        );
        return Ok(());
    }
    let disposition = if cli.trash {
        dedupe::actions::Disposition::Trash
    } else {
        dedupe::actions::Disposition::Permanent
    };
    let mut deleted = 0u64;
    let mut bytes = 0u64;
    let mut skipped = 0u64;
    if !result.findings.is_empty() {
        let stats = dedupe::actions::delete_findings(&result.findings, disposition, cli.yes);
        deleted += stats.deleted;
        bytes += stats.bytes_freed;
        skipped += stats.skipped;
    }
    if !result.groups.is_empty() {
        let stats = dedupe::actions::delete_finding_groups(&result.groups, disposition, cli.yes);
        deleted += stats.deleted;
        bytes += stats.bytes_freed;
        skipped += stats.skipped;
    }
    let verb = if cli.trash {
        "Moved to trash"
    } else {
        "Deleted"
    };
    println!(
        "{} {verb} {} item(s) ({} freed); {} skipped.",
        style("✔").green().bold(),
        style(deleted).green().bold(),
        style(dedupe::util::human_bytes(bytes)).green().bold(),
        skipped
    );
    Ok(())
}

/// Dry-run preview for --consolidate-dir: every planned keeper move with a
/// per-group subtotal, then the skipped keepers and a total. Touches
/// nothing; the plan matches what a real run would do (same collision
/// suffixes).
fn dry_run_consolidate(groups: &[dedupe::matching::Group], dest: &std::path::Path, quiet: bool) {
    let (moves, skipped) = dedupe::actions::plan_consolidation(groups, dest);
    let mut count = 0u64;
    let mut bytes = 0u64;
    // Moves arrive in group order; subtotal each group (one keeper each).
    let mut idx = 0;
    while idx < moves.len() {
        let group_index = moves[idx].group_index;
        let mut group_count = 0u64;
        let mut group_bytes = 0u64;
        while idx < moves.len() && moves[idx].group_index == group_index {
            let target = &moves[idx].target;
            group_count += 1;
            group_bytes += target.size;
            if !quiet {
                println!(
                    "would move {} -> {} ({})",
                    target.from.display(),
                    target.to.display(),
                    dedupe::util::human_bytes(target.size),
                );
            }
            idx += 1;
        }
        count += group_count;
        bytes += group_bytes;
        if !quiet {
            println!(
                "Group #{group_index}: would move {group_count} keeper(s) ({})",
                dedupe::util::human_bytes(group_bytes),
            );
        }
    }
    for (path, reason) in &skipped {
        if !quiet {
            println!("would skip {} ({reason})", path.display());
        }
    }
    println!(
        "{} Dry run: would consolidate {} keeper(s) ({}) into {}; nothing was moved.",
        style("◌").cyan().bold(),
        style(count).cyan().bold(),
        style(dedupe::util::human_bytes(bytes)).cyan().bold(),
        dest.display(),
    );
}
