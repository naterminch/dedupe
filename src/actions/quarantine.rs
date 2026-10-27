use super::MoveDuplicateStats;
use super::fileops::free_dest_path;
use super::fileops::move_file;
use super::guard::prompt;
use super::guard::verify_expected;
use crate::hashing::HashEngine;
use crate::matching::{self, Group};
use crate::util::human_bytes;
use anyhow::Result;
use console::style;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn move_duplicates(
    groups: &[Group],
    dest: &Path,
    engine: &HashEngine,
    yes: bool,
) -> Result<MoveDuplicateStats> {
    fs::create_dir_all(dest)?;
    let mut stats = MoveDuplicateStats::default();
    let mut assume_yes = yes;
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    'groups: for group in groups {
        let targets: Vec<&matching::GroupMember> =
            group.members.iter().filter(|m| !m.keep).collect();
        if targets.is_empty() {
            continue;
        }
        if !assume_yes {
            let bytes: u64 = targets.iter().map(|m| m.size).sum();
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
        for member in targets {
            let expected = member.content_hash.as_deref().unwrap_or(&group.hash);
            match verify_expected(&member.path, expected, member.size, engine) {
                Ok(true) => {}
                Ok(false) => {
                    eprintln!(
                        "{} {}: {}",
                        style("⚠").yellow(),
                        style("changed since the scan, skipping (not moved)")
                            .yellow()
                            .bold(),
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
            let file_name = member
                .path
                .file_name()
                .unwrap_or_else(|| member.path.as_os_str());
            let to = free_dest_path(dest, file_name, &claimed);
            claimed.insert(to.clone());
            match move_file(&member.path, &to) {
                Ok(()) => {
                    stats.files_moved += 1;
                    stats.bytes_moved += member.size;
                }
                Err(e) => {
                    eprintln!(
                        "{} {}: {}",
                        style("⚠").yellow(),
                        style("cannot move").yellow().bold(),
                        style(format!(
                            "{} -> {} ({e})",
                            member.path.display(),
                            to.display()
                        ))
                        .dim()
                    );
                    stats.files_skipped += 1;
                }
            }
        }
    }
    Ok(stats)
}
