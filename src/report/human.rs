use super::{ReportContext, ScanStats, dup_file_count, group_kind_name, reclaimable_bytes};
use crate::matching::Group;
use crate::util::human_bytes;
use console::style;
use std::fmt::Write as _;

const DIVIDER: &str = "────────────────────────────────────────────────";

pub fn print_human(ctx: &ReportContext, stats: &ScanStats, groups: &[Group]) {
    let reclaim = reclaimable_bytes(groups);
    let dups = dup_file_count(groups);

    println!();
    let summary = format!(
        "Scanned {} path(s) · {} files ({}) · {} duplicate group(s) · {} duplicate file(s)",
        ctx.paths.len(),
        stats.files_scanned,
        human_bytes(stats.bytes_scanned),
        groups.len(),
        dups,
    );
    println!("{}", style(summary).bold());
    if reclaim > 0 {
        println!(
            "{}",
            style(format!(
                "Reclaimable with --delete: {}",
                human_bytes(reclaim)
            ))
            .yellow()
            .bold()
        );
    }
    if stats.dirs_skipped > 0 || stats.files_skipped > 0 {
        println!(
            "{}",
            style(format!(
                "Skipped {} file(s) and {} directory tree(s) via filters.",
                stats.files_skipped, stats.dirs_skipped
            ))
            .dim()
        );
    }
    if stats.hardlinks_skipped > 0 {
        println!(
            "{}",
            style(format!(
                "Skipped {} hardlink(s): same file under another name, counted once.",
                stats.hardlinks_skipped
            ))
            .dim()
        );
    }
    if !stats.dir_read_errors.is_empty() {
        println!(
            "{} {}",
            style("⚠").yellow(),
            style(format!(
                "{} path(s) could not be read (first: {})",
                stats.dir_read_errors.len(),
                stats.dir_read_errors.first().unwrap_or(&String::new())
            ))
            .yellow()
        );
    }
    if !ctx.ffprobe_used {
        println!(
            "{}",
            style(
                "Note: ffprobe not found on PATH; image/video resolution and duration are not reported. Install FFmpeg to enable media metadata."
            )
            .dim()
        );
    }

    if groups.is_empty() {
        println!();
        println!("{}", style("✔ No duplicate files found.").green().bold());
        return;
    }

    for group in groups {
        let short_hash = &group.hash[..group.hash.len().min(12)];
        let keeper_media = group.members.first().and_then(|m| m.media.clone());

        println!();
        println!("{}", style(DIVIDER).dim());
        let kind = style(group_kind_name(group.media_kind).to_uppercase())
            .magenta()
            .bold();
        // Similar groups are identified by their similarity %, exact groups by
        // the content hash.
        let ident = group.similarity.map_or_else(
            || format!("{} {short_hash}", ctx.hash_algo),
            |s| {
                style(format!("{:.1}% similar", s * 100.0))
                    .yellow()
                    .bold()
                    .to_string()
            },
        );
        println!(
            "{} {} · {kind} · {} files · {} each · {} reclaimable · {ident}",
            style("◆").cyan().bold(),
            style(format!("Group #{}", group.index)).cyan().bold(),
            group.members.len(),
            human_bytes(group.members[0].size),
            human_bytes(group.dup_bytes()),
        );
        if let Some(info) = &keeper_media {
            println!("{} {}", style("  ↳").dim(), style(info.summary()).dim());
        }

        // Align the size column across the group's members.
        let path_w = group
            .members
            .iter()
            .map(|m| m.path.display().to_string().chars().count())
            .max()
            .unwrap_or(0);
        for member in &group.members {
            let (glyph, badge, color) = if member.reference && member.keep {
                ("◈", "REF", console::Style::new().cyan())
            } else if member.keep {
                ("✓", "KEEP", console::Style::new().green())
            } else {
                ("✗", "DUP", console::Style::new().yellow())
            };
            let mut line = format!(
                "  {} {}  {:<width$}  {}",
                color.apply_to(glyph),
                color.bold().apply_to(badge),
                member.path.display(),
                style(human_bytes(member.size)).magenta(),
                width = path_w,
            );
            if let Some(sim) = member.similarity
                && !member.keep
            {
                let _ = write!(
                    line,
                    "  {} {}",
                    style("·").dim(),
                    style(format!("{:.1}% similar", sim * 100.0)).dim()
                );
            }
            if ctx.verbose
                && let Some(info) = &member.media
            {
                let mut extra = info.summary();
                if let Some(codec) = &info.codec {
                    let _ = write!(extra, ", {codec}");
                }
                let _ = write!(line, "  {} {}", style("·").dim(), style(extra).dim());
            }
            println!("{line}");
        }
    }

    if dups > 0 {
        println!();
        println!(
            "{} run with {} to remove the {} duplicate file(s), or {} to keep one copy (smallest; highest-resolution for similar media).",
            style("Tip:").cyan().bold(),
            style("--delete").green(),
            dups,
            style("--delete --keep-smaller").green(),
        );
    }
}

pub fn print_mode_human(ctx: &ReportContext, result: &crate::modes::ModeResult) {
    let title = crate::modes::mode_title(result.mode);
    let reclaim = result.reclaimable();
    println!();
    println!(
        "{}",
        style(format!(
            "Scanned {} path(s) · {} files ({}) · {title} · {} finding(s)",
            ctx.paths.len(),
            result.files_scanned,
            human_bytes(result.bytes_scanned),
            result.findings.len() + result.groups.len(),
        ))
        .bold()
    );
    if reclaim > 0 {
        println!(
            "{}",
            style(format!(
                "Reclaimable with --delete: {}",
                human_bytes(reclaim)
            ))
            .yellow()
            .bold()
        );
    }
    if result.cancelled {
        println!("{}", style("Scan cancelled; partial results.").yellow());
    }
    if !result.errors.is_empty() {
        println!(
            "{} {}",
            style("⚠").yellow(),
            style(format!(
                "{} path(s) could not be read (first: {})",
                result.errors.len(),
                result.errors.first().map_or("", String::as_str)
            ))
            .yellow()
        );
    }
    for note in &result.notes {
        println!("{}", style(note).dim());
    }
    if result.is_empty() {
        println!();
        println!("{}", style("✔ Nothing found.").green().bold());
        return;
    }
    if !result.findings.is_empty() {
        println!();
        for finding in &result.findings {
            let size = if finding.is_dir {
                "folder".to_string()
            } else {
                human_bytes(finding.size)
            };
            println!(
                "  {} {}  {}  {}",
                style("✗").yellow().bold(),
                style(size).magenta(),
                finding.path.display(),
                style(format!("· {}", finding.detail)).dim(),
            );
        }
    }
    for (i, group) in result.groups.iter().enumerate() {
        println!();
        println!("{}", style(DIVIDER).dim());
        println!(
            "{} {} · {} · {} · {} reclaimable",
            style("◆").cyan().bold(),
            style(format!("Group #{}", i + 1)).cyan().bold(),
            group.label,
            style(&group.detail).dim(),
            human_bytes(group.reclaimable),
        );
        for (n, member) in group.members.iter().enumerate() {
            let (glyph, badge, color) = if n == 0 {
                ("✓", "KEEP", console::Style::new().green())
            } else {
                ("✗", "DUP", console::Style::new().yellow())
            };
            let size = group
                .sizes
                .get(n)
                .copied()
                .map(human_bytes)
                .or_else(|| std::fs::metadata(member).ok().map(|m| human_bytes(m.len())))
                .unwrap_or_default();
            println!(
                "  {} {}  {}  {}",
                color.apply_to(glyph),
                color.bold().apply_to(badge),
                member.display(),
                style(size).magenta(),
            );
        }
    }
    if result.mode == crate::cli::FindMode::Music {
        println!();
        println!(
            "{} run with {} to remove all but the largest track in each group.",
            style("Tip:").cyan().bold(),
            style("--delete --keep-best-quality").green(),
        );
    } else if !result.groups.is_empty() {
        println!();
        println!(
            "{} duplicate folder trees are report-only; review them before removing anything.",
            style("Tip:").cyan().bold(),
        );
    }
}
