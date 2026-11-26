use super::consolidate::move_targets;
use super::fileops::{Claimed, free_dest_path};
use super::guard::prompt;
use super::{ActionError, DeleteTarget, MoveDuplicateStats, MoveTarget};
use crate::hashing::HashEngine;
use crate::matching::Group;
use crate::util::human_bytes;
use console::style;
use std::fs;
use std::path::Path;

/// Decide where each of `targets` lands under `dest` (flattened), in input
/// order, without touching disk beyond existence checks.
///
/// Name collisions — against the filesystem and against earlier targets in
/// this plan — gain a " (2)", " (3)", ... suffix, the same rule as
/// consolidation, so a move can never overwrite an unrelated file.
#[must_use]
pub fn plan_move_aside(targets: &[DeleteTarget], dest: &Path) -> Vec<MoveTarget> {
    let mut claimed = Claimed::default();
    targets
        .iter()
        .map(|target| {
            let file_name = target
                .path
                .file_name()
                .unwrap_or_else(|| target.path.as_os_str());
            let to = free_dest_path(dest, file_name, &claimed);
            claimed.insert(&to);
            MoveTarget {
                from: target.path.clone(),
                to,
                expected_hash: target.expected_hash.clone(),
                size: target.size,
            }
        })
        .collect()
}

/// Move every deletable member of each group into `dest` instead of deleting
/// it (see [`plan_move_aside`] and [`move_targets`]).
///
/// Keepers and copies inside a reference folder are never moved: a reference
/// folder is a safety net, so none of its copies are targets.
///
/// Unless `yes` is set, the user is prompted per group:
///   y = move this group, a = move this and all remaining, q = quit.
///
/// # Errors
///
/// Returns [`ActionError::Destination`] when `dest` cannot be created.
pub fn move_duplicates(
    groups: &[Group],
    dest: &Path,
    engine: &HashEngine,
    yes: bool,
) -> Result<MoveDuplicateStats, ActionError> {
    fs::create_dir_all(dest).map_err(|source| ActionError::Destination {
        dir: dest.to_path_buf(),
        file: dest.to_path_buf(),
        source,
    })?;
    let mut stats = MoveDuplicateStats::default();
    let mut assume_yes = yes;
    'groups: for group in groups {
        let targets: Vec<DeleteTarget> = group
            .members
            .iter()
            .filter(|m| m.deletable())
            .map(|m| DeleteTarget {
                path: m.path.clone(),
                expected_hash: m.content_hash.clone().unwrap_or_else(|| group.hash.clone()),
                size: m.size,
            })
            .collect();
        if targets.is_empty() {
            continue;
        }
        if !assume_yes {
            let bytes: u64 = targets.iter().map(|t| t.size).sum();
            let answer = prompt(&format!(
                "{} Move {} duplicate(s) ({} reclaimable) from group #{} to {}? {} ",
                style("?").yellow().bold(),
                targets.len(),
                human_bytes(bytes),
                group.index,
                dest.display(),
                style("[y/n/a/q]").dim(),
            ));
            match answer.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => {}
                "a" | "all" => assume_yes = true,
                "q" | "quit" => break 'groups,
                _ => {
                    stats.files_skipped += targets.len() as u64;
                    continue;
                }
            }
        }
        // Planned per group, after earlier groups have landed: their files
        // now exist under `dest`, so the collision check sees them on disk.
        let outcome = move_targets(&plan_move_aside(&targets, dest), engine);
        stats.files_moved += outcome.moved.len() as u64;
        stats.bytes_moved += outcome.bytes_moved;
        stats.files_skipped += outcome.skipped.len() as u64;
        stats.failures.extend(outcome.failures);
        stats.items.extend(outcome.items);
    }
    Ok(stats)
}
