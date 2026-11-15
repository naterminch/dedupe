use super::fileops::backup_path;
use super::guard::label_expected;
use super::guard::prompt;
use super::guard::verify_expected;
use super::{ActionError, HardlinkStats, LinkOutcome, LinkTarget};
use crate::hashing::HashEngine;
use crate::matching::Group;
use crate::util::human_bytes;
use console::style;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

/// Whether a group's copies may be replaced by hard links to its keeper.
///
/// Only byte-identical groups qualify: a similar-media group, or one matched
/// by size or name alone, holds different bytes, and a link would silently
/// swap one file's content for another's.
#[must_use]
pub fn can_hardlink(group: &Group) -> bool {
    group.similarity.is_none() && !label_expected(&group.hash)
}

/// How a keeper fared when it was verified, remembered so a keeper shared
/// by many targets is hashed once.
enum KeeperCheck {
    Unchanged,
    Changed,
    /// Unreadable; kept as kind + text because [`io::Error`] is not `Clone`
    /// and every affected target gets its own failure entry.
    Unreadable(io::ErrorKind, String),
}

/// Replace `targets` with hard links to their keepers, non-interactively.
///
/// Used directly by the GUI (which collects its own confirmation) and by
/// [`hardlink_duplicates`] below after its per-group prompts.
///
/// Safety: each keeper is verified once and each target is re-hashed just
/// before it is replaced; anything that changed since the scan is skipped
/// (with a warning), never linked. The target is first renamed to a backup
/// beside it, so a failed link puts the original back instead of losing it.
pub fn link_targets(targets: &[LinkTarget], engine: &HashEngine) -> LinkOutcome {
    let mut outcome = LinkOutcome::default();
    let mut keepers: HashMap<&Path, KeeperCheck> = HashMap::new();
    for target in targets {
        let check = keepers
            .entry(target.keeper.as_path())
            .or_insert_with(|| check_keeper(target, engine));
        match check {
            KeeperCheck::Unchanged => {}
            KeeperCheck::Changed => {
                outcome.skipped.push(target.path.clone());
                continue;
            }
            KeeperCheck::Unreadable(kind, text) => {
                outcome.skipped.push(target.path.clone());
                outcome.failures.push((
                    target.path.clone(),
                    ActionError::Read {
                        path: target.keeper.clone(),
                        source: io::Error::new(*kind, text.as_str()),
                    },
                ));
                continue;
            }
        }
        match verify_expected(&target.path, &target.expected_hash, target.size, engine) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("changed since the scan, skipping").yellow().bold(),
                    style(target.path.display()).dim()
                );
                outcome.skipped.push(target.path.clone());
                continue;
            }
            Err(err) => {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("cannot verify, skipping").yellow().bold(),
                    style(err.to_string()).dim()
                );
                outcome.skipped.push(target.path.clone());
                outcome.failures.push((target.path.clone(), err));
                continue;
            }
        }
        match replace_with_link(target) {
            Ok(()) => {
                outcome.linked.push(target.path.clone());
                outcome.bytes_saved += target.size;
            }
            // Linked, but the old copy is still on disk: the path is done
            // (it is the keeper's file now), the space is not.
            Err(err @ ActionError::BackupKept { .. }) => {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("linked, but the old copy remains").yellow().bold(),
                    style(err.to_string()).dim()
                );
                outcome.linked.push(target.path.clone());
                outcome.failures.push((target.path.clone(), err));
            }
            Err(err) => {
                let restored = !matches!(err, ActionError::Restore { .. });
                eprintln!(
                    "{} {}: {}",
                    if restored {
                        style("⚠").yellow()
                    } else {
                        style("⚠").red().bold()
                    },
                    if restored {
                        style("hard link failed, original kept").yellow().bold()
                    } else {
                        style("could not restore after a failed hard link")
                            .red()
                            .bold()
                    },
                    style(err.to_string()).dim()
                );
                outcome.skipped.push(target.path.clone());
                outcome.failures.push((target.path.clone(), err));
            }
        }
    }
    outcome
}

/// Verify `target`'s keeper against the scan, warning when it cannot be used.
fn check_keeper(target: &LinkTarget, engine: &HashEngine) -> KeeperCheck {
    match verify_expected(
        &target.keeper,
        &target.keeper_hash,
        target.keeper_size,
        engine,
    ) {
        Ok(true) => KeeperCheck::Unchanged,
        Ok(false) => {
            eprintln!(
                "{} {}: {}",
                style("⚠").yellow(),
                style("keeper changed since the scan, skipping its copies")
                    .yellow()
                    .bold(),
                style(target.keeper.display()).dim()
            );
            KeeperCheck::Changed
        }
        Err(err) => {
            eprintln!(
                "{} {}: {}",
                style("⚠").yellow(),
                style("cannot verify keeper, skipping its copies")
                    .yellow()
                    .bold(),
                style(err.to_string()).dim()
            );
            match err {
                ActionError::Read { source, .. } => {
                    KeeperCheck::Unreadable(source.kind(), source.to_string())
                }
                other => KeeperCheck::Unreadable(io::ErrorKind::Other, other.to_string()),
            }
        }
    }
}

/// Stage the target beside itself, link the keeper into its place, then drop
/// the staged copy; a failed link renames the original back.
fn replace_with_link(target: &LinkTarget) -> Result<(), ActionError> {
    let backup = backup_path(&target.path);
    fs::rename(&target.path, &backup).map_err(|source| ActionError::Stage {
        path: target.path.clone(),
        source,
    })?;
    match fs::hard_link(&target.keeper, &target.path) {
        // The link already holds the same bytes, so a backup that will not
        // go loses nothing; but its space is not free, and the caller must
        // not count it as saved.
        Ok(()) => fs::remove_file(&backup).map_err(|source| ActionError::BackupKept {
            path: target.path.clone(),
            backup,
            source,
        }),
        Err(link) => match fs::rename(&backup, &target.path) {
            Ok(()) => Err(ActionError::Link {
                path: target.path.clone(),
                keeper: target.keeper.clone(),
                source: link,
            }),
            Err(source) => Err(ActionError::Restore {
                path: target.path.clone(),
                backup,
                link,
                source,
            }),
        },
    }
}

/// Replace every deletable member of each byte-identical group with a hard
/// link to the group's keeper (see [`link_targets`]).
///
/// Keepers and copies inside a reference folder are never replaced; groups
/// that are not byte-identical ([`can_hardlink`]) are skipped whole.
///
/// Unless `yes` is set, the user is prompted per group:
///   y = link this group, a = link this and all remaining, q = quit.
pub fn hardlink_duplicates(groups: &[Group], engine: &HashEngine, yes: bool) -> HardlinkStats {
    let mut stats = HardlinkStats::default();
    let mut assume_yes = yes;
    'groups: for group in groups {
        if !can_hardlink(group) {
            let skipped = group.members.iter().filter(|m| m.deletable()).count() as u64;
            if skipped > 0 {
                eprintln!(
                    "{} {}: {}",
                    style("–").dim(),
                    style("skipping").dim(),
                    style(format!(
                        "group #{} is not byte-identical, cannot hard-link",
                        group.index
                    ))
                    .dim()
                );
            }
            stats.files_skipped += skipped;
            continue;
        }
        let Some(keeper) = group.members.iter().find(|m| m.keep) else {
            continue;
        };
        let keeper_hash = keeper
            .content_hash
            .clone()
            .unwrap_or_else(|| group.hash.clone());
        let targets: Vec<LinkTarget> = group
            .members
            .iter()
            .filter(|m| m.deletable())
            .map(|m| LinkTarget {
                path: m.path.clone(),
                expected_hash: m.content_hash.clone().unwrap_or_else(|| group.hash.clone()),
                size: m.size,
                keeper: keeper.path.clone(),
                keeper_hash: keeper_hash.clone(),
                keeper_size: keeper.size,
            })
            .collect();
        if targets.is_empty() {
            continue;
        }
        if !assume_yes {
            let bytes: u64 = targets.iter().map(|t| t.size).sum();
            let answer = prompt(&format!(
                "{} Replace {} duplicate(s) with hard links to {} ({} saved, group #{})? {} ",
                style("?").yellow().bold(),
                targets.len(),
                keeper.path.display(),
                human_bytes(bytes),
                group.index,
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
        let outcome = link_targets(&targets, engine);
        stats.linked += outcome.linked.len() as u64;
        stats.bytes_saved += outcome.bytes_saved;
        stats.files_skipped += outcome.skipped.len() as u64;
        stats.failures.extend(outcome.failures);
    }
    stats
}
