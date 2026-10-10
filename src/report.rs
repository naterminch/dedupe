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
