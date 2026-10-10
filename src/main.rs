use anyhow::Result;
use clap::Parser;
use console::style;
use dedupe::pipeline::{self, ScanPhase};
use dedupe::{cache, cli, report};

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

/// Colors are enabled only when stdout is a terminal and NO_COLOR is unset,
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
    let out = pipeline::run_scan(cli, progress_bar.as_ref(), similar_bar.as_ref(), &on_phase)?;

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

    if cli.json {
        report::print_json(&ctx, &out.stats, &out.groups);
    } else {
        report::print_human(&ctx, &out.stats, &out.groups);
    }

    let consolidate_dest: Option<std::path::PathBuf> =
        cli.consolidate_dir.as_ref().map(std::path::PathBuf::from);

    if cli.delete || consolidate_dest.is_some() {
        if out.groups.is_empty() {
            if !cli.quiet {
                let msg = if cli.delete {
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
                        dedupe::actions::consolidate_keepers(&out.groups, dest, &engine, cli.yes)?;
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
            if cli.delete {
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
                    )?;
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
