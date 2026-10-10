use crate::hashing::HashEngine;
use crate::matching::Group;
use crate::util::human_bytes;
use anyhow::Result;
use console::style;
use std::fs;
use std::path::PathBuf;

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
}
