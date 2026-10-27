use super::{ReportContext, ScanStats, dup_file_count, reclaimable_bytes};
use crate::matching::Group;
use serde::Serialize;

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

pub fn print_mode_json(ctx: &ReportContext, result: &crate::modes::ModeResult) {
    println!("{}", to_mode_json(ctx, result));
}

#[must_use]
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

/// Serialize the report (also used by the GUI's Save button).
///
/// # Panics
///
/// Panics if serialization fails. That would mean an invariant broke: the only
/// value `serde_json` can reject here is a non-finite float, and every
/// similarity is derived from finite ratios.
#[must_use]
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
    serde_json::to_string(&report).expect("report fields are finite and serializable")
}
