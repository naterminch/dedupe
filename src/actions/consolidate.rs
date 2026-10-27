use super::fileops::free_dest_path;
use super::fileops::move_file;
use super::guard::prompt;
use super::guard::verify_expected;
use super::{ConsolidateOutcome, ConsolidateStats, MoveTarget, PlannedMove};
use crate::hashing::HashEngine;
use crate::matching::{self, Group};
use crate::util::human_bytes;
use console::style;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Decide where every group's keeper would go under `dest` (flattened),
/// without touching disk.
///
/// Skipped keepers are returned with their reason instead: reference-dir
/// keepers are protected and never moved; keepers already under `dest`
/// are already consolidated. Name collisions — against the filesystem and
/// against earlier moves planned this run — gain a " (2)", " (3)", ...
/// suffix, so a move can never overwrite an unrelated file.
pub fn plan_consolidation(
    groups: &[Group],
    dest: &Path,
) -> (Vec<PlannedMove>, Vec<(PathBuf, &'static str)>) {
    let dest_canon = matching::canonicalize_refs(&[dest.display().to_string()]);
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let mut moves = Vec::new();
    let mut skipped = Vec::new();
    for group in groups {
        let Some(keeper) = group.members.iter().find(|m| m.keep) else {
            continue;
        };
        if keeper.reference {
            skipped.push((keeper.path.clone(), "reference-dir keeper, left in place"));
            continue;
        }
        let canon = matching::canonical_member(&keeper.path);
        if matching::is_under_ref(&canon, &dest_canon) {
            skipped.push((
                keeper.path.clone(),
                "already under the consolidate directory",
            ));
            continue;
        }
        let file_name = keeper
            .path
            .file_name()
            .unwrap_or_else(|| keeper.path.as_os_str());
        let to = free_dest_path(dest, file_name, &claimed);
        if to == keeper.path {
            skipped.push((
                keeper.path.clone(),
                "already under the consolidate directory",
            ));
            continue;
        }
        claimed.insert(to.clone());
        moves.push(PlannedMove {
            group_index: group.index,
            target: MoveTarget {
                from: keeper.path.clone(),
                to,
                expected_hash: keeper
                    .content_hash
                    .clone()
                    .unwrap_or_else(|| group.hash.clone()),
                size: keeper.size,
            },
        });
    }
    (moves, skipped)
}

/// Move `targets` non-interactively, re-hashing each file first.
///
/// Mirrors [`delete_targets`]: a keeper that changed since the scan is
/// skipped (with a warning), never moved.
pub fn consolidate_targets(targets: &[MoveTarget], engine: &HashEngine) -> ConsolidateOutcome {
    let mut outcome = ConsolidateOutcome::default();
    for target in targets {
        match verify_expected(&target.from, &target.expected_hash, target.size, engine) {
            Ok(true) => match move_file(&target.from, &target.to) {
                Ok(()) => {
                    outcome.moved.push((target.from.clone(), target.to.clone()));
                    outcome.bytes_moved += target.size;
                }
                Err(e) => {
                    eprintln!(
                        "{} {}: {}",
                        style("⚠").yellow(),
                        style("cannot move").yellow().bold(),
                        style(format!(
                            "{} -> {} ({e})",
                            target.from.display(),
                            target.to.display()
                        ))
                        .dim()
                    );
                    outcome.skipped.push(target.from.clone());
                }
            },
            Ok(false) => {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("changed since the scan, skipping (not moved)")
                        .yellow()
                        .bold(),
                    style(target.from.display()).dim()
                );
                outcome.skipped.push(target.from.clone());
            }
            Err(e) => {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("cannot verify, skipping").yellow().bold(),
                    style(format!("{} ({e})", target.from.display())).dim()
                );
                outcome.skipped.push(target.from.clone());
            }
        }
    }
    outcome
}

/// Move every group's keeper into `dest` (see [`plan_consolidation`]).
///
/// Safety: every keeper is re-hashed immediately before moving and skipped
/// (with a warning) if it no longer matches the scan, so a file that
/// changed between the scan and the move is never relocated.
///
/// Unless `yes` is set, the user is prompted per group:
///   y = move this group, a = move this and all remaining, q = quit.
pub fn consolidate_keepers(
    groups: &[Group],
    dest: &Path,
    engine: &HashEngine,
    yes: bool,
) -> ConsolidateStats {
    let mut stats = ConsolidateStats::default();
    let mut assume_yes = yes;

    let (moves, skipped) = plan_consolidation(groups, dest);
    for (path, reason) in &skipped {
        eprintln!(
            "{} {}: {}",
            style("–").dim(),
            style("skipping").dim(),
            style(format!("{} ({reason})", path.display())).dim()
        );
        stats.files_skipped += 1;
    }

    // Prompt per group; moves run in plan order so collision suffixes match
    // the dry-run listing.
    let mut by_group: Vec<(usize, Vec<&MoveTarget>)> = Vec::new();
    for planned in &moves {
        match by_group.last_mut() {
            Some((idx, list)) if *idx == planned.group_index => list.push(&planned.target),
            _ => by_group.push((planned.group_index, vec![&planned.target])),
        }
    }
    'groups: for (group_index, targets) in &by_group {
        if !assume_yes {
            let first = &targets[0];
            let answer = prompt(&format!(
                "{} Move keeper {} ({}) to {} (group #{group_index})? {} ",
                style("?").yellow().bold(),
                first.from.display(),
                human_bytes(first.size),
                first.to.display(),
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

        let owned: Vec<MoveTarget> = targets.iter().map(|t| (*t).clone()).collect();
        let outcome = consolidate_targets(&owned, engine);
        stats.files_moved += outcome.moved.len() as u64;
        stats.bytes_moved += outcome.bytes_moved;
        stats.files_skipped += outcome.skipped.len() as u64;
    }

    stats
}
