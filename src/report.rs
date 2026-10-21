use crate::matching::Group;
use crate::media::MediaKind;
use crate::util::human_bytes;
use console::style;
use serde::Serialize;

const DIVIDER: &str = "────────────────────────────────────────────────";

pub struct ReportContext<'a> {
    pub paths: &'a [String],
    pub hash_algo: &'a str,
    pub verbose: bool,
    pub ffprobe_used: bool,
}

/// Aggregate counters captured from the scan phase (the entry list itself is
/// consumed by the hashing pipeline).
#[derive(Debug, Clone)]
pub struct ScanStats {
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub dirs_skipped: u64,
    pub files_skipped: u64,
    pub hardlinks_skipped: u64,
    pub dir_read_errors: Vec<String>,
}

pub fn group_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Other => "file",
    }
}

pub fn dup_file_count(groups: &[Group]) -> u64 {
    groups
        .iter()
        .map(|g| (g.members.len() - g.keep_count()) as u64)
        .sum()
}

pub fn reclaimable_bytes(groups: &[Group]) -> u64 {
    groups.iter().map(Group::dup_bytes).sum()
}

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
        let ident = match group.similarity {
            Some(s) => style(format!("{:.1}% similar", s * 100.0))
                .yellow()
                .bold()
                .to_string(),
            None => format!("{} {short_hash}", ctx.hash_algo),
        };
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
                line.push_str(&format!(
                    "  {} {}",
                    style("·").dim(),
                    style(format!("{:.1}% similar", sim * 100.0)).dim()
                ));
            }
            if ctx.verbose
                && let Some(info) = &member.media
            {
                let mut extra = info.summary();
                if let Some(codec) = &info.codec {
                    extra.push_str(&format!(", {codec}"));
                }
                line.push_str(&format!("  {} {}", style("·").dim(), style(extra).dim()));
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

#[derive(Serialize)]
struct JsonReport<'a> {
    hash_algorithm: &'a str,
    paths: &'a [String],
    files_scanned: usize,
    bytes_scanned: u64,
    dirs_skipped: u64,
    files_skipped: u64,
    hardlinks_skipped: u64,
    dir_read_errors: &'a [String],
    duplicate_groups: usize,
    duplicate_files: u64,
    reclaimable_bytes: u64,
    ffprobe_used: bool,
    groups: &'a [Group],
}

pub fn print_json(ctx: &ReportContext, stats: &ScanStats, groups: &[Group]) {
    println!("{}", to_json(ctx, stats, groups));
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
                result.errors.first().map(String::as_str).unwrap_or("")
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

pub fn print_mode_json(ctx: &ReportContext, result: &crate::modes::ModeResult) {
    println!("{}", to_mode_json(ctx, result));
}

pub fn to_mode_json(ctx: &ReportContext, result: &crate::modes::ModeResult) -> String {
    let findings: Vec<serde_json::Value> = result
        .findings
        .iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path.to_string_lossy(),
                "size": f.size,
                "kind": f.kind,
                "detail": f.detail,
                "is_dir": f.is_dir,
            })
        })
        .collect();
    let groups: Vec<serde_json::Value> = result
        .groups
        .iter()
        .map(|g| {
            serde_json::json!({
                "label": g.label,
                "detail": g.detail,
                "bytes": g.bytes,
                "reclaimable_bytes": g.reclaimable,
                "members": g.members.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({
        "mode": crate::modes::mode_name(result.mode),
        "paths": ctx.paths,
        "files_scanned": result.files_scanned,
        "bytes_scanned": result.bytes_scanned,
        "dirs_scanned": result.dirs_scanned,
        "files_skipped": result.files_skipped,
        "dirs_skipped": result.dirs_skipped,
        "finding_count": result.findings.len(),
        "group_count": result.groups.len(),
        "reclaimable_bytes": result.reclaimable(),
        "cancelled": result.cancelled,
        "dir_read_errors": result.errors,
        "notes": result.notes,
        "findings": findings,
        "groups": groups,
    })
    .to_string()
}

pub fn write_csv(
    path: &std::path::Path,
    ctx: &ReportContext,
    stats: &ScanStats,
    groups: &[Group],
    mode_result: Option<&crate::modes::ModeResult>,
) -> std::io::Result<()> {
    let mut out = String::from("mode,kind,group,path,size,detail\n");
    if let Some(result) = mode_result {
        for finding in &result.findings {
            out.push_str(&csv_row(&[
                crate::modes::mode_name(result.mode),
                finding.kind,
                "",
                &finding.path.to_string_lossy(),
                &finding.size.to_string(),
                &finding.detail,
            ]));
        }
        for group in &result.groups {
            for member in &group.members {
                let size = std::fs::metadata(member).map(|m| m.len()).unwrap_or(0);
                out.push_str(&csv_row(&[
                    crate::modes::mode_name(result.mode),
                    "group",
                    &group.label,
                    &member.to_string_lossy(),
                    &size.to_string(),
                    &group.detail,
                ]));
            }
        }
    } else {
        let _ = stats;
        for group in groups {
            for member in &group.members {
                let kind = if member.keep { "keep" } else { "duplicate" };
                let detail = group
                    .similarity
                    .map(|s| format!("{:.1}% similar", s * 100.0))
                    .unwrap_or_else(|| group.hash.clone());
                out.push_str(&csv_row(&[
                    "duplicates",
                    kind,
                    &format!("#{}", group.index),
                    &member.path.to_string_lossy(),
                    &member.size.to_string(),
                    &detail,
                ]));
            }
        }
        let _ = ctx;
    }
    std::fs::write(path, out)
}

fn csv_row(fields: &[&str]) -> String {
    let mut row = String::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            row.push(',');
        }
        if field.contains([',', '"', '\n', '\r']) {
            row.push('"');
            row.push_str(&field.replace('"', "\"\""));
            row.push('"');
        } else {
            row.push_str(field);
        }
    }
    row.push('\n');
    row
}

pub fn write_html(
    path: &std::path::Path,
    ctx: &ReportContext,
    stats: &ScanStats,
    groups: &[Group],
    mode_result: Option<&crate::modes::ModeResult>,
) -> std::io::Result<()> {
    let title = mode_result
        .map(|m| crate::modes::mode_title(m.mode))
        .unwrap_or("Duplicate files");
    let summary = mode_result
        .map(|m| {
            format!(
                "{} files scanned · {} findings · {} reclaimable",
                m.files_scanned,
                m.findings.len() + m.groups.len(),
                human_bytes(m.reclaimable())
            )
        })
        .unwrap_or_else(|| {
            format!(
                "{} files scanned · {} groups · {} duplicate files · {} reclaimable",
                stats.files_scanned,
                groups.len(),
                dup_file_count(groups),
                human_bytes(reclaimable_bytes(groups))
            )
        });
    let mut body = String::new();
    body.push_str(&format!(
        "<header><h1>{}</h1><p>{}</p><p class=\"paths\">{}</p></header>",
        escape_html(title),
        escape_html(&summary),
        escape_html(&ctx.paths.join(" · ")),
    ));
    if let Some(result) = mode_result {
        if !result.findings.is_empty() {
            body.push_str("<h2>Findings</h2><table><thead><tr><th>Kind</th><th>Path</th><th>Size</th><th>Detail</th></tr></thead><tbody>");
            for f in &result.findings {
                body.push_str(&format!(
                    "<tr><td>{}</td><td class=\"path\">{}</td><td>{}</td><td>{}</td></tr>",
                    escape_html(f.kind),
                    escape_html(&f.path.to_string_lossy()),
                    if f.is_dir {
                        "folder".to_string()
                    } else {
                        human_bytes(f.size)
                    },
                    escape_html(&f.detail),
                ));
            }
            body.push_str("</tbody></table>");
        }
        for (i, group) in result.groups.iter().enumerate() {
            body.push_str(&format!(
                "<h2>Group #{} · {}</h2><p>{}</p><table><thead><tr><th>Role</th><th>Path</th><th>Size</th></tr></thead><tbody>",
                i + 1,
                escape_html(&group.label),
                escape_html(&group.detail),
            ));
            for (n, member) in group.members.iter().enumerate() {
                let size = std::fs::metadata(member).map(|m| m.len()).unwrap_or(0);
                body.push_str(&format!(
                    "<tr><td class=\"{}\">{}</td><td class=\"path\">{}</td><td>{}</td></tr>",
                    if n == 0 { "keep" } else { "dup" },
                    if n == 0 { "KEEP" } else { "DUP" },
                    escape_html(&member.to_string_lossy()),
                    human_bytes(size),
                ));
            }
            body.push_str("</tbody></table>");
        }
    } else {
        for group in groups {
            body.push_str(&format!(
                "<h2>Group #{} · {}</h2>",
                group.index,
                group
                    .similarity
                    .map(|s| format!("{:.1}% similar", s * 100.0))
                    .unwrap_or_else(|| escape_html(&group.hash[..group.hash.len().min(24)])),
            ));
            body.push_str(
                "<table><thead><tr><th>Role</th><th>Path</th><th>Size</th></tr></thead><tbody>",
            );
            for member in &group.members {
                body.push_str(&format!(
                    "<tr><td class=\"{}\">{}</td><td class=\"path\">{}</td><td>{}</td></tr>",
                    if member.keep { "keep" } else { "dup" },
                    if member.keep { "KEEP" } else { "DUP" },
                    escape_html(&member.path.to_string_lossy()),
                    human_bytes(member.size),
                ));
            }
            body.push_str("</tbody></table>");
        }
    }
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>dedupe report</title><style>body{{font:15px/1.5 system-ui,sans-serif;max-width:1100px;margin:2rem auto;padding:0 1rem;color:#222}}h1{{margin-bottom:.2rem}}h2{{margin-top:2rem}}p{{color:#555}}.paths{{font-size:.85rem;color:#777}}table{{border-collapse:collapse;width:100%;margin-top:.5rem}}th,td{{text-align:left;padding:.45rem .6rem;border-bottom:1px solid #e5e5e5;vertical-align:top}}th{{background:#fafafa;font-size:.8rem;text-transform:uppercase;letter-spacing:.04em}}.path{{font-family:ui-monospace,monospace;font-size:.85rem;word-break:break-all}}.keep{{color:#0a7d32;font-weight:600}}.dup{{color:#b26a00;font-weight:600}}footer{{margin-top:3rem;font-size:.8rem;color:#888}}</style></head><body>{body}<footer>Generated by dedupe</footer></body></html>"
    );
    std::fs::write(path, html)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Serialize the report (also used by the GUI's Save button).
pub fn to_json(ctx: &ReportContext, stats: &ScanStats, groups: &[Group]) -> String {
    let report = JsonReport {
        hash_algorithm: ctx.hash_algo,
        paths: ctx.paths,
        files_scanned: stats.files_scanned,
        bytes_scanned: stats.bytes_scanned,
        dirs_skipped: stats.dirs_skipped,
        files_skipped: stats.files_skipped,
        hardlinks_skipped: stats.hardlinks_skipped,
        dir_read_errors: &stats.dir_read_errors,
        duplicate_groups: groups.len(),
        duplicate_files: dup_file_count(groups),
        reclaimable_bytes: reclaimable_bytes(groups),
        ffprobe_used: ctx.ffprobe_used,
        groups,
    };
    serde_json::to_string(&report).expect("report serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::GroupMember;

    #[test]
    fn to_json_has_expected_shape() {
        let paths = ["/media".to_string()];
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "blake3",
            verbose: false,
            ffprobe_used: false,
        };
        let stats = ScanStats {
            files_scanned: 2,
            bytes_scanned: 20,
            dirs_skipped: 0,
            files_skipped: 0,
            hardlinks_skipped: 1,
            dir_read_errors: Vec::new(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&to_json(&ctx, &stats, &[])).expect("valid JSON");
        assert_eq!(v["hash_algorithm"], "blake3");
        assert_eq!(v["hardlinks_skipped"], 1);
        assert_eq!(v["duplicate_groups"], 0);
    }

    #[test]
    fn to_mode_json_has_expected_shape() {
        use crate::cli::FindMode;
        use crate::modes::{Finding, FindingGroup, ModeResult};
        use std::path::PathBuf;

        let paths = ["/media".to_string()];
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "blake3",
            verbose: false,
            ffprobe_used: false,
        };
        let result = ModeResult {
            mode: FindMode::EmptyFiles,
            findings: vec![Finding {
                path: PathBuf::from("a.txt"),
                size: 0,
                kind: "empty",
                detail: "zero-byte file".to_string(),
                is_dir: false,
            }],
            groups: vec![FindingGroup {
                label: "Identical folder tree (2 copies)".to_string(),
                detail: "1 file, 6 B".to_string(),
                members: vec![PathBuf::from("one"), PathBuf::from("two")],
                sizes: vec![6, 6],
                bytes: 6,
                reclaimable: 6,
            }],
            files_scanned: 2,
            bytes_scanned: 6,
            dirs_scanned: 1,
            files_skipped: 0,
            dirs_skipped: 0,
            errors: vec!["denied".to_string()],
            notes: vec!["note".to_string()],
            cancelled: false,
        };

        let v: serde_json::Value =
            serde_json::from_str(&to_mode_json(&ctx, &result)).expect("valid JSON");
        assert_eq!(v["mode"], "empty-files");
        assert_eq!(v["finding_count"], 1);
        assert_eq!(v["group_count"], 1);
        assert_eq!(v["reclaimable_bytes"], 6);
        assert_eq!(v["findings"][0]["kind"], "empty");
        assert_eq!(v["findings"][0]["size"], 0);
        assert_eq!(v["findings"][0]["is_dir"], false);
        assert_eq!(v["groups"][0]["members"].as_array().unwrap().len(), 2);
        assert_eq!(v["groups"][0]["reclaimable_bytes"], 6);
        assert_eq!(v["dir_read_errors"], serde_json::json!(["denied"]));
    }

    #[test]
    fn csv_and_html_exports_cover_modes_and_duplicates() {
        use crate::cli::FindMode;
        use crate::modes::{Finding, ModeResult};
        use std::path::PathBuf;

        let root = std::env::temp_dir().join(format!(
            "dedupe-report-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let csv_path = root.join("mode.csv");
        let html_path = root.join("mode.html");
        let dup_csv_path = root.join("duplicates.csv");
        let dup_html_path = root.join("duplicates.html");

        let paths = ["/media".to_string()];
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "blake3",
            verbose: false,
            ffprobe_used: false,
        };
        let stats = ScanStats {
            files_scanned: 3,
            bytes_scanned: 76,
            dirs_skipped: 0,
            files_skipped: 0,
            hardlinks_skipped: 0,
            dir_read_errors: Vec::new(),
        };
        let mode_result = ModeResult {
            mode: FindMode::BigFiles,
            findings: vec![Finding {
                path: PathBuf::from("a.bin"),
                size: 12,
                kind: "big",
                detail: "#1 of 3 scanned files".to_string(),
                is_dir: false,
            }],
            files_scanned: 3,
            bytes_scanned: 76,
            ..ModeResult::default()
        };
        write_csv(&csv_path, &ctx, &stats, &[], Some(&mode_result)).unwrap();
        write_html(&html_path, &ctx, &stats, &[], Some(&mode_result)).unwrap();
        let csv = std::fs::read_to_string(&csv_path).unwrap();
        assert!(csv.starts_with("mode,kind,group,path,size,detail\n"));
        assert!(csv.contains("big-files,big,,a.bin,12,#1 of 3 scanned files"));
        let html = std::fs::read_to_string(&html_path).unwrap();
        assert!(html.contains("<h1>Largest files</h1>"));
        assert!(html.contains("<h2>Findings</h2>"));
        assert!(html.contains("a.bin"));

        let groups = vec![group(
            1,
            MediaKind::Other,
            vec![member("k.txt", 100, true), member("d.txt", 100, false)],
        )];
        write_csv(&dup_csv_path, &ctx, &stats, &groups, None).unwrap();
        write_html(&dup_html_path, &ctx, &stats, &groups, None).unwrap();
        let csv = std::fs::read_to_string(&dup_csv_path).unwrap();
        assert!(csv.contains("duplicates,keep,#1,k.txt,100,abc123"));
        assert!(csv.contains("duplicates,duplicate,#1,d.txt,100,abc123"));
        let html = std::fs::read_to_string(&dup_html_path).unwrap();
        assert!(html.contains("<h1>Duplicate files</h1>"));
        assert!(html.contains("Group #1"));
        assert!(html.contains("k.txt"));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn member(path: &str, size: u64, keep: bool) -> GroupMember {
        GroupMember {
            path: std::path::PathBuf::from(path),
            size,
            mtime_secs: None,
            keep,
            reference: false,
            media: None,
            similarity: None,
            content_hash: None,
            fingerprint_res: None,
        }
    }

    fn group(index: usize, kind: MediaKind, members: Vec<GroupMember>) -> Group {
        Group {
            index,
            hash: "abc123".to_string(),
            media_kind: kind,
            similarity: None,
            members,
        }
    }

    #[test]
    fn group_kind_names_cover_every_media_kind() {
        assert_eq!(group_kind_name(MediaKind::Image), "image");
        assert_eq!(group_kind_name(MediaKind::Video), "video");
        assert_eq!(group_kind_name(MediaKind::Other), "file");
    }

    #[test]
    fn counters_count_only_non_keep_members() {
        let groups = vec![
            group(
                1,
                MediaKind::Other,
                vec![
                    member("k.txt", 100, true),
                    member("d1.txt", 100, false),
                    member("d2.txt", 100, false),
                ],
            ),
            group(
                2,
                MediaKind::Image,
                vec![member("k.png", 50, true), member("d.png", 50, false)],
            ),
        ];
        assert_eq!(dup_file_count(&groups), 3);
        assert_eq!(reclaimable_bytes(&groups), 250);
        assert_eq!(dup_file_count(&[]), 0);
        assert_eq!(reclaimable_bytes(&[]), 0);
    }

    #[test]
    fn to_json_reports_group_math_and_members() {
        let paths = ["C:\\pics".to_string()];
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "sha256",
            verbose: false,
            ffprobe_used: true,
        };
        let stats = ScanStats {
            files_scanned: 3,
            bytes_scanned: 300,
            dirs_skipped: 1,
            files_skipped: 2,
            hardlinks_skipped: 0,
            dir_read_errors: vec!["denied".to_string()],
        };
        let groups = vec![group(
            1,
            MediaKind::Other,
            vec![member("k.txt", 100, true), member("d.txt", 100, false)],
        )];
        let v: serde_json::Value =
            serde_json::from_str(&to_json(&ctx, &stats, &groups)).expect("valid JSON");
        assert_eq!(v["duplicate_groups"], 1);
        assert_eq!(v["duplicate_files"], 1);
        assert_eq!(v["reclaimable_bytes"], 100);
        assert_eq!(v["dirs_skipped"], 1);
        assert_eq!(v["dir_read_errors"], serde_json::json!(["denied"]));
        assert_eq!(v["groups"][0]["members"].as_array().unwrap().len(), 2);
        // Per-group preview: kind keeps its renamed key, reclaimable is live.
        assert_eq!(v["groups"][0]["kind"], "other");
        assert_eq!(v["groups"][0]["reclaimable_bytes"], 100);
    }

    #[test]
    fn print_human_renders_skips_errors_and_similar_groups() {
        use crate::media::MediaInfo;
        let paths = ["C:\\pics".to_string()];
        let stats = ScanStats {
            files_scanned: 4,
            bytes_scanned: 400,
            dirs_skipped: 1,
            files_skipped: 2,
            hardlinks_skipped: 3,
            dir_read_errors: vec!["denied".to_string()],
        };
        let mut keeper = member("k.png", 100, true);
        keeper.media = Some(MediaInfo {
            width: Some(800),
            height: Some(600),
            duration_ms: None,
            codec: Some("png".to_string()),
        });
        let mut dup = member("d.png", 100, false);
        dup.similarity = Some(0.981);
        dup.media = Some(MediaInfo {
            width: Some(800),
            height: Some(600),
            duration_ms: None,
            codec: None,
        });
        let groups = vec![Group {
            index: 1,
            hash: "img".to_string(),
            media_kind: MediaKind::Image,
            similarity: Some(0.981),
            members: vec![keeper, dup],
        }];
        // Verbose exercises the per-member media/codec line.
        for verbose in [false, true] {
            let ctx = ReportContext {
                paths: &paths,
                hash_algo: "blake3",
                verbose,
                ffprobe_used: false,
            };
            // Must not panic on any branch; output goes to stdout.
            print_human(&ctx, &stats, &groups);
        }
        // Empty groups print the no-duplicates marker instead of dividers.
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "blake3",
            verbose: false,
            ffprobe_used: true,
        };
        print_human(&ctx, &stats, &[]);
    }
}
