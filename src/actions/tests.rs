use super::*;
use crate::cli::HashAlgo;
use crate::hashing::HashEngine;
use crate::matching::{Group, GroupMember};
use crate::media::MediaKind;
use std::fs;
use std::path::PathBuf;

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

    let stats = delete_duplicates(&groups, &eng, true, Disposition::Permanent);
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
    let stats = consolidate_keepers(&groups, &dest, &engine(), true);
    assert_eq!(stats.files_moved, 1);
    assert_eq!(stats.bytes_moved, 15);
    assert_eq!(stats.files_skipped, 0);
    assert!(!keep.exists());
    assert!(dest.join("a.txt").exists());
    assert!(dup.exists(), "dups are not touched by consolidate");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn move_duplicates_moves_non_keep_and_counts_bytes() {
    let dir = tmpdir("movedup");
    let keep = dir.join("keep.txt");
    let dup = dir.join("dup.txt");
    fs::write(&keep, b"identical bytes").unwrap();
    fs::write(&dup, b"identical bytes").unwrap();

    let dest = dir.join("quarantine");
    let groups = vec![keeper_group(keep.clone(), dup.clone(), false)];
    let stats = move_duplicates(&groups, &dest, &engine(), true).unwrap();
    assert_eq!(stats.files_moved, 1);
    assert_eq!(stats.bytes_moved, 15);
    assert_eq!(stats.files_skipped, 0);
    assert!(keep.exists());
    assert!(!dup.exists());
    assert_eq!(fs::read(dest.join("dup.txt")).unwrap(), b"identical bytes");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn move_duplicates_skips_changed_files() {
    let dir = tmpdir("movedchanged");
    let keep = dir.join("keep.txt");
    let dup = dir.join("dup.txt");
    fs::write(&keep, b"identical bytes").unwrap();
    fs::write(&dup, b"identical bytes").unwrap();
    let groups = vec![keeper_group(keep.clone(), dup.clone(), false)];

    fs::write(&dup, b"changed content").unwrap();
    let dest = dir.join("quarantine");
    let stats = move_duplicates(&groups, &dest, &engine(), true).unwrap();
    assert_eq!(stats.files_moved, 0);
    assert_eq!(stats.files_skipped, 1);
    assert!(dup.exists(), "changed duplicate must stay put");
    assert!(!dest.join("dup.txt").exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hardlink_duplicates_replaces_duplicate_with_link() {
    let dir = tmpdir("hardlink");
    let keep = dir.join("keep.txt");
    let dup = dir.join("dup.txt");
    fs::write(&keep, b"identical bytes").unwrap();
    fs::write(&dup, b"identical bytes").unwrap();

    let groups = vec![keeper_group(keep.clone(), dup.clone(), false)];
    let stats = hardlink_duplicates(&groups, &engine(), true);
    assert_eq!(stats.linked, 1);
    assert_eq!(stats.bytes_saved, 15);
    assert_eq!(stats.files_skipped, 0);
    assert!(keep.exists() && dup.exists());

    fs::write(&keep, b"changed content").unwrap();
    assert_eq!(fs::read(&dup).unwrap(), b"changed content");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hardlink_duplicates_skips_similar_groups() {
    let dir = tmpdir("hardlinksimilar");
    let keep = dir.join("keep.txt");
    let dup = dir.join("dup.txt");
    fs::write(&keep, b"identical bytes").unwrap();
    fs::write(&dup, b"identical bytes").unwrap();

    let mut group = keeper_group(keep, dup.clone(), false);
    group.similarity = Some(0.99);
    let stats = hardlink_duplicates(&[group], &engine(), true);
    assert_eq!(stats.linked, 0);
    assert_eq!(stats.files_skipped, 1);
    assert!(dup.exists());

    let _ = fs::remove_dir_all(&dir);
}
