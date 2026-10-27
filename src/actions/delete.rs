use super::guard::prompt;
use super::guard::verify_expected;
use super::{DeleteOutcome, DeleteTarget, DeletionStats, Disposition};
use crate::hashing::HashEngine;
use crate::matching::Group;
use crate::util::human_bytes;
use anyhow::Result;
use console::style;
use std::fs;

/// Delete `targets` non-interactively, re-hashing each file first.
///
/// Used directly by the GUI (which collects its own confirmation) and by
/// [`delete_duplicates`] below after its per-group prompts.
pub fn delete_targets(
    targets: &[DeleteTarget],
    engine: &HashEngine,
    disposition: Disposition,
) -> DeleteOutcome {
    let mut outcome = DeleteOutcome::default();
    for target in targets {
        match verify_expected(&target.path, &target.expected_hash, target.size, engine) {
            Ok(true) => {
                let removed: Result<(), String> = match disposition {
                    Disposition::Permanent => {
                        fs::remove_file(&target.path).map_err(|e| e.to_string())
                    }
                    Disposition::Trash => trash::delete(&target.path).map_err(|e| e.to_string()),
                };
                match removed {
                    Ok(()) => {
                        outcome.deleted.push(target.path.clone());
                        outcome.bytes_freed += target.size;
                    }
                    Err(e) => {
                        eprintln!(
                            "{} {}: {}",
                            style("⚠").yellow(),
                            style("cannot delete").yellow().bold(),
                            style(format!("{} ({e})", target.path.display())).dim()
                        );
                        outcome.skipped.push(target.path.clone());
                    }
                }
            }
            Ok(false) => {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("changed since the scan, skipping (not deleted)")
                        .yellow()
                        .bold(),
                    style(target.path.display()).dim()
                );
                outcome.skipped.push(target.path.clone());
            }
            Err(e) => {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("cannot verify, skipping").yellow().bold(),
                    style(format!("{} ({e})", target.path.display())).dim()
                );
                outcome.skipped.push(target.path.clone());
            }
        }
    }
    outcome
}

/// Delete every non-keep member of each group.
///
/// Safety: every file is re-hashed immediately before deletion and skipped
/// (with a warning) if it no longer matches the group's hash, so a file that
/// changed between the scan and the delete is never removed.
///
/// Unless `yes` is set, the user is prompted per group:
///   y = delete this group, a = delete this and all remaining, q = quit.
pub fn delete_duplicates(
    groups: &[Group],
    engine: &HashEngine,
    yes: bool,
    disposition: Disposition,
) -> DeletionStats {
    let mut stats = DeletionStats::default();
    let mut assume_yes = yes;

    'groups: for group in groups {
        let targets: Vec<DeleteTarget> = group
            .members
            .iter()
            .filter(|m| !m.keep)
            .map(|m| DeleteTarget {
                path: m.path.clone(),
                // Similar groups share no content hash, so each file is
                // verified against its own hash recorded during the scan.
                expected_hash: m.content_hash.clone().unwrap_or_else(|| group.hash.clone()),
                size: m.size,
            })
            .collect();
        if targets.is_empty() {
            continue;
        }
        let bytes: u64 = targets.iter().map(|t| t.size).sum();

        if !assume_yes {
            let sim = group
                .similarity
                .map(|s| format!(" ({:.1}% similar)", s * 100.0))
                .unwrap_or_default();
            let answer = prompt(&format!(
                "{} Delete {} file(s) ({} reclaimable{sim}) from group #{}? {} ",
                style("?").yellow().bold(),
                targets.len(),
                human_bytes(bytes),
                group.index,
                style("[y/n/a/q]").dim(),
            ));
            match answer.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => {}
                "a" | "all" => assume_yes = true,
                "q" | "quit" => break 'groups,
                _ => continue,
            }
        }

        let outcome = delete_targets(&targets, engine, disposition);
        stats.files_deleted += outcome.deleted.len() as u64;
        stats.bytes_freed += outcome.bytes_freed;
        stats.files_skipped += outcome.skipped.len() as u64;
    }

    stats
}
