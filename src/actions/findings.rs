use super::guard::prompt;
use super::{Disposition, FindingDeleteStats};
use crate::hashing::HashEngine;
use crate::modes::Finding;
use crate::modes::FindingGroup;
use crate::modes::content_hash;
use crate::util::human_bytes;
use anyhow::Result;
use console::style;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Whether `path` sits inside one of the reference directories.
///
/// A reference directory is the user's own safety net — a backup, or a folder
/// the GUI shows as protected — so a cleanup mode may still find clutter in
/// there but must never remove it.
#[must_use]
pub fn is_protected(path: &Path, protected: &[PathBuf]) -> bool {
    if protected.is_empty() {
        return false;
    }
    // Both spellings matter: a symlink inside a protected folder points outside
    // it, and a dangling link cannot be resolved at all. Comparing the path
    // without following links keeps both of those inside the fence.
    let lexical = absolute(path);
    let resolved = crate::matching::canonical_member(path);
    protected
        .iter()
        .any(|dir| lexical.starts_with(dir) || resolved.starts_with(dir))
}

/// `path` as an absolute path with `.` and `..` folded lexically, without
/// resolving symlinks.
fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Compare a finding against what the scan recorded.
///
/// Directories are covered by the type test instead. Anything else has to prove
/// it still holds the same bytes, and a finding with no recorded hash has no
/// proof: it is kept rather than removed on trust.
fn content_unchanged(finding: &Finding, engine: &HashEngine) -> bool {
    if finding.is_dir {
        return true;
    }
    let Some(expected) = finding.hash.as_deref() else {
        return false;
    };
    content_hash(&finding.path, engine).is_some_and(|now| now == expected)
}

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
    protected: &[PathBuf],
    engine: &HashEngine,
) -> FindingDeleteStats {
    let mut stats = FindingDeleteStats::default();
    if findings.is_empty() {
        return stats;
    }
    // Protected folders are decided before anything is asked or removed: a
    // finding inside one is not a candidate, and saying so up front keeps the
    // prompt's count honest.
    let (protected_findings, deleteable): (Vec<&Finding>, Vec<&Finding>) = findings
        .iter()
        .partition(|finding| is_protected(&finding.path, protected));
    if !protected_findings.is_empty() {
        stats.skipped += protected_findings.len() as u64;
        eprintln!(
            "{} {}",
            style("⚠").yellow(),
            style(format!(
                "{} found item(s) are inside a protected folder, keeping them",
                protected_findings.len()
            ))
            .yellow()
            .bold()
        );
    }
    if deleteable.is_empty() {
        return stats;
    }
    let bytes: u64 = deleteable
        .iter()
        .filter(|f| !f.is_dir)
        .map(|f| f.size)
        .sum();
    if !yes {
        let answer = prompt(&format!(
            "{} Delete {} found item(s) ({} reclaimable)? {} ",
            style("?").yellow().bold(),
            deleteable.len(),
            human_bytes(bytes),
            style("[y/N]").dim(),
        ));
        if !matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes" | "a" | "all"
        ) {
            stats.skipped += deleteable.len() as u64;
            return stats;
        }
    }
    for finding in deleteable {
        if !finding_still_valid(finding) || !content_unchanged(finding, engine) {
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
    protected: &[PathBuf],
    engine: &HashEngine,
) -> FindingDeleteStats {
    let mut stats = FindingDeleteStats::default();
    let mut assume_yes = yes;
    'groups: for group in groups {
        // The first member is the keeper; the rest are deletion candidates,
        // minus anything sitting in a protected folder.
        let mut targets: Vec<(usize, &PathBuf)> = Vec::new();
        let mut kept = 0usize;
        for (index, path) in group.members.iter().enumerate().skip(1) {
            if is_protected(path, protected) {
                kept += 1;
            } else {
                targets.push((index, path));
            }
        }
        if kept > 0 {
            stats.skipped += kept as u64;
            eprintln!(
                "{} {}",
                style("⚠").yellow(),
                style(format!(
                    "{kept} copy(ies) of “{}” are inside a protected folder, keeping them",
                    group.label
                ))
                .yellow()
                .bold()
            );
        }
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
            let content_ok = group
                .hashes
                .get(i)
                .and_then(|hash| hash.as_deref())
                .is_some_and(|expected| {
                    content_hash(path, engine).is_some_and(|now| now == expected)
                });
            if !size_ok || !content_ok {
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
                hash: None,
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
