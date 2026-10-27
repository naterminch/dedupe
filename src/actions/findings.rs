use super::guard::prompt;
use super::{Disposition, FindingDeleteStats};
use crate::modes::Finding;
use crate::modes::FindingGroup;
use crate::util::human_bytes;
use anyhow::Result;
use console::style;
use std::fs;

fn finding_still_valid(finding: &Finding) -> bool {
    match finding.kind {
        "empty-dir" => {
            fs::metadata(&finding.path)
                .map(|m| m.is_dir())
                .unwrap_or(false)
                && fs::read_dir(&finding.path)
                    .map(|mut it| it.next().is_none())
                    .unwrap_or(false)
        }
        "symlink" => {
            fs::symlink_metadata(&finding.path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
                && fs::metadata(&finding.path).is_err()
        }
        "empty" => fs::metadata(&finding.path)
            .map(|m| m.is_file() && m.len() == 0)
            .unwrap_or(false),
        _ => fs::metadata(&finding.path)
            .map(|m| {
                if finding.is_dir {
                    m.is_dir()
                } else {
                    m.is_file() && m.len() == finding.size
                }
            })
            .unwrap_or(false),
    }
}

fn remove_finding_path(finding: &Finding, disposition: Disposition) -> Result<(), String> {
    match disposition {
        Disposition::Trash => trash::delete(&finding.path).map_err(|e| e.to_string()),
        Disposition::Permanent => {
            if finding.is_dir || finding.kind == "empty-dir" {
                fs::remove_dir(&finding.path).map_err(|e| e.to_string())
            } else {
                fs::remove_file(&finding.path)
                    .or_else(|_| fs::remove_dir(&finding.path))
                    .map_err(|e| e.to_string())
            }
        }
    }
}

pub fn delete_findings(
    findings: &[Finding],
    disposition: Disposition,
    yes: bool,
) -> FindingDeleteStats {
    let mut stats = FindingDeleteStats::default();
    if findings.is_empty() {
        return stats;
    }
    let bytes: u64 = findings.iter().filter(|f| !f.is_dir).map(|f| f.size).sum();
    if !yes {
        let answer = prompt(&format!(
            "{} Delete {} found item(s) ({} reclaimable)? {} ",
            style("?").yellow().bold(),
            findings.len(),
            human_bytes(bytes),
            style("[y/N]").dim(),
        ));
        if !matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes" | "a" | "all"
        ) {
            stats.skipped = findings.len() as u64;
            return stats;
        }
    }
    for finding in findings {
        if !finding_still_valid(finding) {
            eprintln!(
                "{} {}: {}",
                style("⚠").yellow(),
                style("changed since the scan, skipping").yellow().bold(),
                style(finding.path.display()).dim()
            );
            stats.skipped += 1;
            continue;
        }
        match remove_finding_path(finding, disposition) {
            Ok(()) => {
                stats.deleted += 1;
                if !finding.is_dir {
                    stats.bytes_freed += finding.size;
                }
            }
            Err(err) => {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("cannot remove").yellow().bold(),
                    style(format!("{} ({err})", finding.path.display())).dim()
                );
                stats.skipped += 1;
            }
        }
    }
    stats
}

pub fn delete_finding_groups(
    groups: &[FindingGroup],
    disposition: Disposition,
    yes: bool,
) -> FindingDeleteStats {
    let mut stats = FindingDeleteStats::default();
    let mut assume_yes = yes;
    'groups: for group in groups {
        let targets: Vec<(usize, &std::path::PathBuf)> =
            group.members.iter().enumerate().skip(1).collect();
        if targets.is_empty() {
            continue;
        }
        if !assume_yes {
            let answer = prompt(&format!(
                "{} Delete {} duplicate(s) from “{}”? {} ",
                style("?").yellow().bold(),
                targets.len(),
                group.label,
                style("[y/n/a/q]").dim(),
            ));
            match answer.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => {}
                "a" | "all" => assume_yes = true,
                "q" | "quit" => break 'groups,
                _ => {
                    stats.skipped += targets.len() as u64;
                    continue;
                }
            }
        }
        for (i, path) in targets {
            let expected_size = group.sizes.get(i).copied().unwrap_or(0);
            let size_ok = fs::metadata(path)
                .map(|m| m.len() == expected_size)
                .unwrap_or(false);
            if !size_ok {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("changed since the scan, skipping").yellow().bold(),
                    style(path.display()).dim()
                );
                stats.skipped += 1;
                continue;
            }
            let finding = Finding {
                path: path.clone(),
                size: expected_size,
                kind: "group",
                detail: String::new(),
                is_dir: false,
            };
            match remove_finding_path(&finding, disposition) {
                Ok(()) => {
                    stats.deleted += 1;
                    stats.bytes_freed += expected_size;
                }
                Err(err) => {
                    eprintln!(
                        "{} {}: {}",
                        style("⚠").yellow(),
                        style("cannot remove").yellow().bold(),
                        style(format!("{} ({err})", path.display())).dim()
                    );
                    stats.skipped += 1;
                }
            }
        }
    }
    stats
}
