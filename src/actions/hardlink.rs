use super::HardlinkStats;
use super::fileops::backup_path;
use super::guard::label_expected;
use super::guard::prompt;
use super::guard::verify_expected;
use crate::hashing::HashEngine;
use crate::matching::{self, Group};
use crate::util::human_bytes;
use console::style;
use std::fs;

pub fn hardlink_duplicates(groups: &[Group], engine: &HashEngine, yes: bool) -> HardlinkStats {
    let mut stats = HardlinkStats::default();
    let mut assume_yes = yes;
    'groups: for group in groups {
        if group.similarity.is_some() || label_expected(&group.hash) {
            let skipped = group.members.iter().filter(|m| !m.keep).count() as u64;
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
        let targets: Vec<&matching::GroupMember> =
            group.members.iter().filter(|m| !m.keep).collect();
        if targets.is_empty() {
            continue;
        }
        if !assume_yes {
            let bytes: u64 = targets.iter().map(|m| m.size).sum();
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
        let expected = keeper.content_hash.as_deref().unwrap_or(&group.hash);
        if !matches!(
            verify_expected(&keeper.path, expected, keeper.size, engine),
            Ok(true)
        ) {
            eprintln!(
                "{} {}: {}",
                style("⚠").yellow(),
                style("keeper changed since the scan, skipping group")
                    .yellow()
                    .bold(),
                style(keeper.path.display()).dim()
            );
            stats.files_skipped += targets.len() as u64;
            continue;
        }
        for member in targets {
            let member_expected = member.content_hash.as_deref().unwrap_or(&group.hash);
            match verify_expected(&member.path, member_expected, member.size, engine) {
                Ok(true) => {}
                Ok(false) => {
                    eprintln!(
                        "{} {}: {}",
                        style("⚠").yellow(),
                        style("changed since the scan, skipping").yellow().bold(),
                        style(member.path.display()).dim()
                    );
                    stats.files_skipped += 1;
                    continue;
                }
                Err(e) => {
                    eprintln!(
                        "{} {}: {}",
                        style("⚠").yellow(),
                        style("cannot verify, skipping").yellow().bold(),
                        style(format!("{} ({e})", member.path.display())).dim()
                    );
                    stats.files_skipped += 1;
                    continue;
                }
            }
            let backup = backup_path(&member.path);
            if let Err(e) = fs::rename(&member.path, &backup) {
                eprintln!(
                    "{} {}: {}",
                    style("⚠").yellow(),
                    style("cannot stage hard-link replacement").yellow().bold(),
                    style(format!("{} ({e})", member.path.display())).dim()
                );
                stats.files_skipped += 1;
                continue;
            }
            match fs::hard_link(&keeper.path, &member.path) {
                Ok(()) => {
                    let _ = fs::remove_file(&backup);
                    stats.linked += 1;
                    stats.bytes_saved += member.size;
                }
                Err(e) => {
                    if let Err(restore) = fs::rename(&backup, &member.path) {
                        eprintln!(
                            "{} {}: {}",
                            style("⚠").red().bold(),
                            style("could not restore after a failed hard link")
                                .red()
                                .bold(),
                            style(format!(
                                "{} (backup kept at {}: {restore})",
                                member.path.display(),
                                backup.display()
                            ))
                            .dim()
                        );
                    } else {
                        eprintln!(
                            "{} {}: {}",
                            style("⚠").yellow(),
                            style("hard link failed, original restored").yellow().bold(),
                            style(format!("{} ({e})", member.path.display())).dim()
                        );
                    }
                    stats.files_skipped += 1;
                }
            }
        }
    }
    stats
}
