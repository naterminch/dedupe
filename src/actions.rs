use crate::hashing::HashEngine;
use crate::matching::{self, Group};
use crate::util::human_bytes;
use anyhow::Result;
use console::style;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct DeletionStats {
    pub files_deleted: u64,
    pub bytes_freed: u64,
    pub files_skipped: u64,
}

/// Where deleted files go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Remove permanently (previous behavior).
    Permanent,
    /// Move to the system trash (recoverable).
    Trash,
}

/// One file the caller wants deleted, with the hash it must still match.
///
/// The hash is verified immediately before deletion: a file that changed
/// since the scan is never removed.
#[derive(Debug, Clone)]
pub struct DeleteTarget {
    pub path: PathBuf,
    pub expected_hash: String,
    pub size: u64,
}

/// Per-file outcome of [`delete_targets`], so callers (e.g. the GUI) can
/// update their own state for exactly the files that were removed.
#[derive(Debug, Default)]
pub struct DeleteOutcome {
    pub deleted: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub bytes_freed: u64,
}

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
        match engine.full(&target.path) {
            Ok(h) if h.as_str() == target.expected_hash => {
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
            Ok(_) => {
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
                    style("cannot re-hash, skipping").yellow().bold(),
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
) -> Result<DeletionStats> {
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

    Ok(stats)
}

fn prompt(message: &str) -> String {
    use std::io::Write;
    print!("{message}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line
}

/// One keeper the caller wants moved into the consolidate directory, with
/// the hash it must still match.
///
/// The hash is verified immediately before moving: a file that changed
/// since the scan is never moved.
#[derive(Debug, Clone)]
pub struct MoveTarget {
    pub from: PathBuf,
    pub to: PathBuf,
    pub expected_hash: String,
    pub size: u64,
}

/// A keeper move planned by [`plan_consolidation`], tagged with its group
/// so callers can report per-group progress.
#[derive(Debug, Clone)]
pub struct PlannedMove {
    pub group_index: usize,
    pub target: MoveTarget,
}

/// Per-file outcome of [`consolidate_targets`], so callers (e.g. the GUI)
/// can update their own state for exactly the files that were moved.
#[derive(Debug, Default)]
pub struct ConsolidateOutcome {
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub skipped: Vec<PathBuf>,
    pub bytes_moved: u64,
}

#[derive(Debug, Default)]
pub struct ConsolidateStats {
    pub files_moved: u64,
    pub bytes_moved: u64,
    pub files_skipped: u64,
}

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

/// A free path for `file_name` directly under `dest`: the plain join when
/// available, otherwise `stem (2).ext`, `stem (3).ext`, ... `claimed`
/// reserves names promised to earlier moves this run so two keepers with
/// the same file name never collide.
fn free_dest_path(dest: &Path, file_name: &std::ffi::OsStr, claimed: &HashSet<PathBuf>) -> PathBuf {
    let first = dest.join(file_name);
    if !first.exists() && !claimed.contains(&first) {
        return first;
    }
    let name = Path::new(file_name);
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 2u32;
    loop {
        let candidate = dest.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() && !claimed.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Move one file, falling back to copy + remove when the atomic rename
/// fails (cross-device moves). The destination parent is created.
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(rename_err) => fs::copy(from, to)
            .and_then(|_| fs::remove_file(from))
            .map_err(|copy_err| {
                format!("rename failed ({rename_err}); copy fallback failed ({copy_err})")
            }),
    }
}

/// Move `targets` non-interactively, re-hashing each file first.
///
/// Mirrors [`delete_targets`]: a keeper that changed since the scan is
/// skipped (with a warning), never moved.
pub fn consolidate_targets(targets: &[MoveTarget], engine: &HashEngine) -> ConsolidateOutcome {
    let mut outcome = ConsolidateOutcome::default();
    for target in targets {
        match engine.full(&target.from) {
            Ok(h) if h.as_str() == target.expected_hash => {
                match move_file(&target.from, &target.to) {
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
                }
            }
            Ok(_) => {
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
                    style("cannot re-hash, skipping").yellow().bold(),
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
) -> Result<ConsolidateStats> {
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

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::HashAlgo;
    use crate::matching::{Group, GroupMember};
    use crate::media::MediaKind;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dedupe-actions-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn engine() -> HashEngine {
        HashEngine::new(HashAlgo::Blake3)
    }

    fn target_for(path: &PathBuf) -> DeleteTarget {
        let eng = engine();
        DeleteTarget {
            size: fs::metadata(path).unwrap().len(),
            expected_hash: eng.full(path).unwrap(),
            path: path.clone(),
        }
    }

    fn move_target_for(path: &PathBuf, to: PathBuf) -> MoveTarget {
        let eng = engine();
        MoveTarget {
            size: fs::metadata(path).unwrap().len(),
            expected_hash: eng.full(path).unwrap(),
            from: path.clone(),
            to,
        }
    }

    fn member(path: PathBuf, size: u64, keep: bool, hash: Option<String>) -> GroupMember {
        GroupMember {
            path,
            size,
            mtime_secs: None,
            keep,
            reference: false,
            media: None,
            similarity: None,
            content_hash: hash,
            fingerprint_res: None,
        }
    }

    fn keeper_group(keep_path: PathBuf, dup_path: PathBuf, reference_keeper: bool) -> Group {
        let eng = engine();
        let hash = eng.full(&keep_path).unwrap();
        let mut keeper = member(keep_path, 15, true, Some(hash.clone()));
        keeper.reference = reference_keeper;
        Group {
            index: 1,
            hash: hash.clone(),
            media_kind: MediaKind::Other,
            similarity: None,
            members: vec![keeper, member(dup_path, 15, false, Some(hash))],
        }
    }

    #[test]
    fn permanent_delete_removes_matching_targets_and_counts_bytes() {
        let dir = tmpdir("perm");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, b"same payload here").unwrap();
        fs::write(&b, b"same payload here").unwrap();

        let targets = vec![target_for(&a), target_for(&b)];
        let outcome = delete_targets(&targets, &engine(), Disposition::Permanent);
        assert_eq!(outcome.deleted.len(), 2);
        assert!(outcome.skipped.is_empty());
        assert_eq!(outcome.bytes_freed, 2 * "same payload here".len() as u64);
        assert!(!a.exists() && !b.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_file_is_skipped_and_survives() {
        let dir = tmpdir("changed");
        let p = dir.join("f.txt");
        fs::write(&p, b"version one").unwrap();
        let mut target = target_for(&p);
        // Modify after the "scan": the recorded hash no longer matches.
        fs::write(&p, b"version two, different").unwrap();
        target.size = fs::metadata(&p).unwrap().len();

        let outcome = delete_targets(&[target], &engine(), Disposition::Permanent);
        assert!(outcome.deleted.is_empty());
        assert_eq!(outcome.skipped, vec![p.clone()]);
        assert_eq!(outcome.bytes_freed, 0);
        assert!(p.exists(), "changed file must never be deleted");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_skipped_not_failed() {
        let dir = tmpdir("missing");
        let p = dir.join("gone.txt");
        let target = DeleteTarget {
            path: p.clone(),
            expected_hash: "deadbeef".to_string(),
            size: 10,
        };
        let outcome = delete_targets(&[target], &engine(), Disposition::Permanent);
        assert!(outcome.deleted.is_empty());
        assert_eq!(outcome.skipped, vec![p]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_targets_yield_empty_outcome() {
        let outcome = delete_targets(&[], &engine(), Disposition::Permanent);
        assert!(outcome.deleted.is_empty() && outcome.skipped.is_empty());
        assert_eq!(outcome.bytes_freed, 0);
    }

    #[test]
    #[cfg(windows)]
    fn locked_file_is_skipped_and_survives() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tmpdir("locked");
        let p = dir.join("locked.txt");
        fs::write(&p, b"locked content").unwrap();
        let target = target_for(&p);
        // Share reads/writes but not delete: re-hashing succeeds, removal
        // fails with a sharing violation.
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        let _lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&p)
            .unwrap();

        let outcome = delete_targets(&[target], &engine(), Disposition::Permanent);
        assert!(outcome.deleted.is_empty());
        assert_eq!(outcome.skipped, vec![p.clone()]);
        assert!(p.exists(), "failed delete must leave the file");

        drop(_lock);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trash_disposition_removes_path_from_location() {
        let dir = tmpdir("trash");
        let p = dir.join("t.txt");
        fs::write(&p, b"trash me").unwrap();

        let outcome = delete_targets(&[target_for(&p)], &engine(), Disposition::Trash);
        assert_eq!(outcome.deleted.len(), 1);
        assert!(!p.exists(), "trashed file must leave its location");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_duplicates_yes_removes_only_non_keep() {
        let dir = tmpdir("yesmode");
        let keep = dir.join("keep.txt");
        let dup = dir.join("dup.txt");
        fs::write(&keep, b"identical bytes").unwrap();
        fs::write(&dup, b"identical bytes").unwrap();
        let eng = engine();
        let hash = eng.full(&keep).unwrap();
        let groups = vec![Group {
            index: 1,
            hash: hash.clone(),
            media_kind: MediaKind::Other,
            similarity: None,
            members: vec![
                member(keep.clone(), 15, true, Some(hash.clone())),
                member(dup.clone(), 15, false, Some(hash.clone())),
            ],
        }];

        let stats = delete_duplicates(&groups, &eng, true, Disposition::Permanent).unwrap();
        assert_eq!(stats.files_deleted, 1);
        assert_eq!(stats.files_skipped, 0);
        assert!(keep.exists() && !dup.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn consolidate_moves_keeper_and_counts_bytes() {
        let dir = tmpdir("consmove");
        let keep = dir.join("keep.txt");
        fs::write(&keep, b"identical bytes").unwrap();
        let dest = dir.join("vault");
        let to = dest.join("keep.txt");

        let outcome = consolidate_targets(&[move_target_for(&keep, to.clone())], &engine());
        assert_eq!(outcome.moved, vec![(keep.clone(), to.clone())]);
        assert!(outcome.skipped.is_empty());
        assert_eq!(outcome.bytes_moved, 15);
        assert!(!keep.exists(), "source must be gone after the move");
        assert_eq!(fs::read(&to).unwrap(), b"identical bytes");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn consolidate_skips_changed_keeper() {
        let dir = tmpdir("conschanged");
        let keep = dir.join("keep.txt");
        fs::write(&keep, b"version one").unwrap();
        let mut target = move_target_for(&keep, dir.join("vault").join("keep.txt"));
        fs::write(&keep, b"version two, different").unwrap();
        target.size = fs::metadata(&keep).unwrap().len();

        let outcome = consolidate_targets(&[target], &engine());
        assert!(outcome.moved.is_empty());
        assert_eq!(outcome.skipped, vec![keep.clone()]);
        assert_eq!(outcome.bytes_moved, 0);
        assert!(keep.exists(), "changed keeper must never be moved");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_assigns_collision_suffix() {
        let dir = tmpdir("conscollide");
        let src = dir.join("src");
        let dest = dir.join("vault");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        // An unrelated file already owns the plain name.
        fs::write(dest.join("keep.txt"), b"someone else").unwrap();
        let keep = src.join("keep.txt");
        let dup = src.join("dup.txt");
        fs::write(&keep, b"identical bytes").unwrap();
        fs::write(&dup, b"identical bytes").unwrap();

        let groups = vec![keeper_group(keep.clone(), dup, false)];
        let (moves, skipped) = plan_consolidation(&groups, &dest);
        assert!(skipped.is_empty());
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].target.to, dest.join("keep (2).txt"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_skips_reference_and_already_there_keepers() {
        let dir = tmpdir("consskip");
        let backup = dir.join("backup");
        let incoming = dir.join("incoming");
        let dest = dir.join("vault");
        for d in [&backup, &incoming, &dest] {
            fs::create_dir_all(d).unwrap();
        }
        // Reference keeper: protected, never moved.
        let ref_keep = backup.join("r.txt");
        let ref_dup = incoming.join("r.txt");
        fs::write(&ref_keep, b"protected copy!").unwrap();
        fs::write(&ref_dup, b"protected copy!").unwrap();
        // Keeper already under dest: already consolidated.
        let home = dest.join("h.txt");
        let away = incoming.join("h.txt");
        fs::write(&home, b"already home!!!").unwrap();
        fs::write(&away, b"already home!!!").unwrap();

        let mut g1 = keeper_group(ref_keep.clone(), ref_dup, true);
        g1.index = 1;
        let mut g2 = keeper_group(home.clone(), away, false);
        g2.index = 2;
        let (moves, skipped) = plan_consolidation(&[g1, g2], &dest);
        assert!(moves.is_empty());
        assert_eq!(skipped.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn consolidate_keepers_yes_moves_every_keeper() {
        let dir = tmpdir("consyes");
        let src = dir.join("src");
        let dest = dir.join("vault");
        fs::create_dir_all(&src).unwrap();
        let keep = src.join("a.txt");
        let dup = src.join("b.txt");
        fs::write(&keep, b"identical bytes").unwrap();
        fs::write(&dup, b"identical bytes").unwrap();

        let groups = vec![keeper_group(keep.clone(), dup.clone(), false)];
        let stats =
            consolidate_keepers(&groups, &dest, &engine(), true).expect("consolidate works");
        assert_eq!(stats.files_moved, 1);
        assert_eq!(stats.bytes_moved, 15);
        assert_eq!(stats.files_skipped, 0);
        assert!(!keep.exists());
        assert!(dest.join("a.txt").exists());
        assert!(dup.exists(), "dups are not touched by consolidate");

        let _ = fs::remove_dir_all(&dir);
    }
}
