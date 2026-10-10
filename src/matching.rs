use crate::hashing::DuplicateGroup;
use crate::media::{self, MediaInfo, MediaKind};
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A file inside a duplicate group, with its keep decision and media metadata.
#[derive(Debug, Clone, Serialize)]
pub struct GroupMember {
    pub path: PathBuf,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime_secs: Option<i64>,
    pub keep: bool,
    /// Under a `--reference-dir`: protected, never deleted. Display-only
    /// (skipped in JSON so script output stays stable).
    #[serde(skip_serializing)]
    pub reference: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInfo>,
    /// Perceptual similarity to the group keeper (0..=1). Set for groups found
    /// by `--similar`; `None` for exact-duplicate groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
    /// The member's own full content hash — used as a deletion safety check
    /// for similar groups (their bytes legitimately differ from each other).
    #[serde(skip_serializing)]
    pub content_hash: Option<String>,
    /// Fingerprint resolution for similar-media members (display-only).
    /// Used to re-apply the highest-resolution rule without rescanning.
    #[serde(skip_serializing)]
    pub fingerprint_res: Option<(u32, u32)>,
}

/// A set of identical files, ordered with the KEEP member first.
#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub index: usize,
    pub hash: String,
    #[serde(rename = "kind")]
    pub media_kind: MediaKind,
    /// Worst pairwise similarity within the group (0..=1); `None` for exact
    /// groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
    pub members: Vec<GroupMember>,
}

impl Group {
    pub fn keep_count(&self) -> usize {
        self.members.iter().filter(|m| m.keep).count()
    }

    /// Bytes that would be freed by deleting every non-keep member.
    pub fn dup_bytes(&self) -> u64 {
        self.members
            .iter()
            .filter(|m| !m.keep)
            .map(|m| m.size)
            .sum()
    }
}

/// Which member of a duplicate group is kept (czkawka's delete methods
/// cover the same strategies via AEN/AEO/AEB/AES codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepMode {
    /// Lexicographically first path (default).
    First,
    /// Smallest file, ties broken by path.
    Smallest,
    /// Most recently modified (unknown mtime counts as oldest).
    Newest,
    /// Least recently modified (unknown mtime counts as newest, i.e. kept
    /// only when nothing else qualifies — it sorts last).
    Oldest,
}

/// True when `path` lives under one of the reference (protected) folders.
/// Both sides are canonicalized once by the caller; comparison is
/// case-insensitive on Windows where the FS is.
pub fn is_under_ref(path: &Path, reference_dirs: &[PathBuf]) -> bool {
    if reference_dirs.is_empty() {
        return false;
    }
    #[cfg(windows)]
    {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        reference_dirs.iter().any(|r| {
            let rl = r.to_string_lossy().to_ascii_lowercase();
            lower == rl
                || lower.starts_with(&format!("{rl}\\"))
                || lower.starts_with(&format!("{rl}/"))
        })
    }
    #[cfg(not(windows))]
    {
        reference_dirs.iter().any(|r| path.starts_with(r))
    }
}

/// Canonicalize reference dirs once per run (best-effort; unresolvable
/// entries fall back to the raw path).
pub fn canonicalize_refs(raw: &[String]) -> Vec<PathBuf> {
    raw.iter()
        .map(|r| std::fs::canonicalize(r).unwrap_or_else(|_| PathBuf::from(r)))
        .collect()
}

/// Canonicalize a member path for reference matching (best-effort).
pub(crate) fn canonical_member(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Turn raw hash groups into report-ready groups:
/// - identify the group's media kind (from the member that will be kept),
/// - probe resolution/duration for media groups (in parallel),
/// - choose which member to keep (see [`KeepMode`]; members under a
///   reference dir always win and are marked `reference`).
pub fn assemble_groups(
    groups: Vec<DuplicateGroup>,
    keep: KeepMode,
    reference_dirs: &[PathBuf],
    probe: bool,
) -> Vec<Group> {
    let groups: Vec<(usize, Group)> = groups
        .into_par_iter()
        .enumerate()
        .map(|(i, group)| {
            let index = i + 1;
            // Keep decision before any reordering.
            let mut members: Vec<GroupMember> = group
                .members
                .into_iter()
                .map(|e| {
                    let canon = canonical_member(&e.path);
                    let reference =
                        !reference_dirs.is_empty() && is_under_ref(&canon, reference_dirs);
                    GroupMember {
                        path: e.path,
                        size: e.size,
                        mtime_secs: e.mtime_secs,
                        keep: false,
                        reference,
                        media: None,
                        similarity: None,
                        content_hash: None,
                        fingerprint_res: None,
                    }
                })
                .collect();

            // Probe media metadata (parallel across members).
            let media_kind = members
                .first()
                .map(|m| media::classify(&m.path))
                .unwrap_or(MediaKind::Other);
            let want_probe = probe && media_kind != MediaKind::Other;
            if want_probe {
                for member in &mut members {
                    member.media = media::probe_media(&member.path);
                }
            }

            // Choose the keeper.
            let keep_idx = choose_keep(&members, keep);
            members[keep_idx].keep = true;

            // Order: KEEP first, then the rest sorted by path for determinism.
            let keeper = members.remove(keep_idx);
            let mut rest = members;
            rest.sort_by(|a, b| a.path.cmp(&b.path));
            let mut ordered = vec![keeper];
            ordered.extend(rest);

            (
                index,
                Group {
                    index,
                    hash: group.hash,
                    media_kind,
                    similarity: None,
                    members: ordered,
                },
            )
        })
        .collect();

    let mut groups: Vec<Group> = groups.into_iter().map(|(_, g)| g).collect();
    groups.sort_by(|a, b| a.hash.cmp(&b.hash));
    groups
}

fn choose_keep(members: &[GroupMember], keep: KeepMode) -> usize {
    // Reference-dir members are protected: the keeper always comes from
    // them when any exist.
    let pool = ref_pool(members);
    choose_best(members, &pool, keep, false)
}

/// Indices the keeper may come from: reference members when any exist,
/// otherwise everybody.
fn ref_pool(members: &[GroupMember]) -> Vec<usize> {
    let refs: Vec<usize> = members
        .iter()
        .enumerate()
        .filter(|(_, m)| m.reference)
        .map(|(i, _)| i)
        .collect();
    if refs.is_empty() {
        (0..members.len()).collect()
    } else {
        refs
    }
}

/// Best index among `pool`. `resolution` selects the similar-media rule
/// (highest fingerprint resolution wins `Smallest`) over the plain
/// smallest-file rule.
fn choose_best(members: &[GroupMember], pool: &[usize], keep: KeepMode, resolution: bool) -> usize {
    let mut best = pool[0];
    for &i in &pool[1..] {
        let better = if resolution {
            is_better_keep_res(&members[i], &members[best], keep)
        } else {
            is_better_keep(&members[i], &members[best], keep)
        };
        if better {
            best = i;
        }
    }
    best
}

/// Re-apply a (possibly changed) keep mode to existing groups — e.g. the
/// GUI's top keep selector — without rescanning. Reference flags are
/// stored per member, so protection survives reassignment.
pub fn reassign_keepers(groups: &mut [Group], keep: KeepMode) {
    for g in groups {
        if g.members.is_empty() {
            continue;
        }
        let pool = ref_pool(&g.members);
        let best = choose_best(&g.members, &pool, keep, g.similarity.is_some());
        for (i, m) in g.members.iter_mut().enumerate() {
            m.keep = i == best;
        }
        // Keeper first, rest by path (same order the pipeline produces).
        g.members
            .sort_by(|a, b| (!a.keep).cmp(&!b.keep).then_with(|| a.path.cmp(&b.path)));
    }
}

/// Resolution-aware keeper comparison for similar-media groups.
fn is_better_keep_res(a: &GroupMember, b: &GroupMember, keep: KeepMode) -> bool {
    if keep != KeepMode::Smallest {
        return is_better_keep(a, b, keep);
    }
    let area = |m: &GroupMember| -> Option<u64> {
        m.fingerprint_res
            .filter(|(w, h)| *w > 0 && *h > 0)
            .map(|(w, h)| w as u64 * h as u64)
    };
    match (area(a), area(b)) {
        (Some(x), Some(y)) => x > y || (x == y && (a.size, &a.path) < (b.size, &b.path)),
        (Some(_), None) => true,
        (None, None) => (a.size, &a.path) < (b.size, &b.path),
        (None, Some(_)) => false,
    }
}

/// True when `a` is a better keeper than `b` under `keep` (ties broken by
/// path for determinism).
fn is_better_keep(a: &GroupMember, b: &GroupMember, keep: KeepMode) -> bool {
    match keep {
        KeepMode::First => a.path < b.path,
        KeepMode::Smallest => (a.size, &a.path) < (b.size, &b.path),
        // Unknown mtime sorts as oldest (Newest) / newest (Oldest): files
        // we know nothing about never win on a time rule.
        KeepMode::Newest => {
            (a.mtime_secs.unwrap_or(i64::MIN), std::cmp::Reverse(&a.path))
                > (b.mtime_secs.unwrap_or(i64::MIN), std::cmp::Reverse(&b.path))
        }
        KeepMode::Oldest => {
            (a.mtime_secs.unwrap_or(i64::MAX), &a.path)
                < (b.mtime_secs.unwrap_or(i64::MAX), &b.path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::FileEntry;
    use std::fs;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dedupe-match-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_group(dir: &std::path::Path, names: &[(&str, usize)]) -> DuplicateGroup {
        let members = names
            .iter()
            .map(|(name, size)| {
                let path = dir.join(name);
                fs::write(&path, vec![0u8; *size]).unwrap();
                FileEntry {
                    path,
                    size: *size as u64,
                    mtime_secs: Some(0),
                }
            })
            .collect();
        DuplicateGroup {
            hash: "abc".into(),
            members,
        }
    }

    fn no_refs() -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    #[test]
    fn is_under_ref_matches_only_inside_the_tree() {
        assert!(!is_under_ref(Path::new("C:\\a\\f.txt"), &no_refs()));
        let refs = vec![PathBuf::from("C:\\backup")];
        assert!(is_under_ref(Path::new("C:\\backup\\z.txt"), &refs));
        assert!(is_under_ref(Path::new("C:\\backup"), &refs));
        assert!(!is_under_ref(Path::new("C:\\backup2\\z.txt"), &refs));
        assert!(!is_under_ref(Path::new("C:\\other\\z.txt"), &refs));
    }

    #[test]
    fn canonicalize_refs_resolves_existing_and_passes_through_missing() {
        let dir = tmpdir("canon");
        let resolved = canonicalize_refs(&[dir.to_string_lossy().into_owned()]);
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].is_absolute());
        let missing = canonicalize_refs(&["Z:\\definitely\\not\\here-12345".to_string()]);
        assert_eq!(
            missing,
            vec![PathBuf::from("Z:\\definitely\\not\\here-12345")]
        );
        assert!(canonicalize_refs(&[]).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_groups_indexes_from_one_with_single_keeper_each() {
        let dir = tmpdir("indexing");
        let g1 = fake_group(&dir, &[("a.txt", 10), ("b.txt", 10)]);
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let g2 = fake_group(&sub, &[("c.txt", 5), ("d.txt", 5)]);
        let groups = assemble_groups(vec![g1, g2], KeepMode::First, &no_refs(), false);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].index, 1);
        assert_eq!(groups[1].index, 2);
        for g in &groups {
            assert_eq!(g.keep_count(), 1, "exactly one keeper per group");
            assert!(g.members.first().unwrap().keep, "keeper sorts first");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_keeps_first_path() {
        let dir = tmpdir("default");
        let group = fake_group(&dir, &[("z.txt", 10), ("a.txt", 20), ("m.txt", 5)]);
        let groups = assemble_groups(vec![group], KeepMode::First, &no_refs(), false);
        let members = &groups[0].members;
        assert_eq!(members.first().unwrap().path.file_name().unwrap(), "a.txt");
        assert!(members.first().unwrap().keep);
        assert_eq!(members.iter().filter(|m| m.keep).count(), 1);
        // z.txt (10) + m.txt (5) are duplicates; a.txt (20) is kept.
        assert_eq!(groups[0].dup_bytes(), 15);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_smaller_wins() {
        let dir = tmpdir("smaller");
        let group = fake_group(&dir, &[("a.txt", 10), ("b.txt", 20), ("c.txt", 2)]);
        let groups = assemble_groups(vec![group], KeepMode::Smallest, &no_refs(), false);
        let members = &groups[0].members;
        assert_eq!(members.first().unwrap().path.file_name().unwrap(), "c.txt");
        assert!(members.first().unwrap().keep);
        assert_eq!(groups[0].dup_bytes(), 30);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_newest_and_oldest_use_mtime() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let dir = tmpdir("mtime");
        let paths = ["old.txt", "new.txt"];
        for name in paths {
            fs::write(dir.join(name), b"x").unwrap();
        }
        // Force distinct mtimes (filesystem granularity varies).
        let old_t = SystemTime::now() - std::time::Duration::from_secs(3600);
        filetime_set(&dir.join("old.txt"), old_t);
        let mk = || DuplicateGroup {
            hash: "abc".into(),
            members: paths
                .iter()
                .map(|name| {
                    let path = dir.join(name);
                    let mtime = fs::metadata(&path)
                        .unwrap()
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    FileEntry {
                        path,
                        size: 1,
                        mtime_secs: mtime,
                    }
                })
                .collect(),
        };
        let groups = assemble_groups(vec![mk()], KeepMode::Newest, &no_refs(), false);
        assert_eq!(
            groups[0].members.first().unwrap().path.file_name().unwrap(),
            "new.txt"
        );
        let groups = assemble_groups(vec![mk()], KeepMode::Oldest, &no_refs(), false);
        assert_eq!(
            groups[0].members.first().unwrap().path.file_name().unwrap(),
            "old.txt"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn filetime_set(path: &std::path::Path, t: std::time::SystemTime) {
        let ft = fs::FileTimes::new().set_modified(t);
        if let Ok(f) = fs::File::options().write(true).open(path) {
            let _ = f.set_times(ft);
        }
    }

    #[test]
    fn reference_dir_wins_keeper_and_marks_members() {
        let dir = tmpdir("ref");
        let incoming = dir.join("incoming");
        let backup = dir.join("backup");
        fs::create_dir_all(&incoming).unwrap();
        fs::create_dir_all(&backup).unwrap();
        // a.txt sorts first but lives outside the reference dir.
        let group = DuplicateGroup {
            hash: "abc".into(),
            members: ["a.txt", "z.txt"]
                .iter()
                .map(|name| {
                    let parent = if *name == "z.txt" { &backup } else { &incoming };
                    let path = parent.join(name);
                    fs::write(&path, b"same").unwrap();
                    FileEntry {
                        path,
                        size: 4,
                        mtime_secs: Some(0),
                    }
                })
                .collect(),
        };
        let refs = canonicalize_refs(&[backup.display().to_string()]);
        let groups = assemble_groups(vec![group], KeepMode::First, &refs, false);
        let members = &groups[0].members;
        assert_eq!(members.first().unwrap().path.file_name().unwrap(), "z.txt");
        assert!(members.first().unwrap().reference);
        assert!(!members[1].reference);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reassign_keepers_applies_new_mode_without_rescan() {
        let dir = tmpdir("reassign");
        // a.txt sorts first and is smallest; b.txt is biggest.
        let group = fake_group(&dir, &[("b.txt", 20), ("a.txt", 5)]);
        let mut groups = assemble_groups(vec![group], KeepMode::First, &no_refs(), false);
        assert_eq!(
            groups[0].members.first().unwrap().path.file_name().unwrap(),
            "a.txt"
        );

        // Switch to Smallest: still a.txt here (it is both first and
        // smallest), so build a reversed case via Newest/Oldest on mtime.
        reassign_keepers(&mut groups, KeepMode::Smallest);
        assert_eq!(
            groups[0].members.first().unwrap().path.file_name().unwrap(),
            "a.txt"
        );
        assert!(groups[0].members.first().unwrap().keep);
        assert_eq!(groups[0].members.iter().filter(|m| m.keep).count(), 1);

        // Keeper stays first after reorder.
        reassign_keepers(&mut groups, KeepMode::First);
        let names: Vec<String> = groups[0]
            .members
            .iter()
            .map(|m| m.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reassign_keepers_respects_reference_flags() {
        let dir = tmpdir("reassign-ref");
        let incoming = dir.join("incoming");
        let backup = dir.join("backup");
        fs::create_dir_all(&incoming).unwrap();
        fs::create_dir_all(&backup).unwrap();
        let group = DuplicateGroup {
            hash: "abc".into(),
            members: ["a.txt", "z.txt"]
                .iter()
                .map(|name| {
                    let parent = if *name == "z.txt" { &backup } else { &incoming };
                    let path = parent.join(name);
                    fs::write(&path, b"same").unwrap();
                    FileEntry {
                        path,
                        size: 4,
                        mtime_secs: Some(0),
                    }
                })
                .collect(),
        };
        let refs = canonicalize_refs(&[backup.display().to_string()]);
        let mut groups = assemble_groups(vec![group], KeepMode::First, &refs, false);
        assert_eq!(
            groups[0].members.first().unwrap().path.file_name().unwrap(),
            "z.txt"
        );
        // Reassigning another mode must not dethrone the protected keeper.
        reassign_keepers(&mut groups, KeepMode::Smallest);
        assert_eq!(
            groups[0].members.first().unwrap().path.file_name().unwrap(),
            "z.txt"
        );
        assert!(groups[0].members.first().unwrap().keep);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn media_kind_detected_from_extension() {
        let dir = tmpdir("kind");
        let group = fake_group(&dir, &[("pic.JPG", 10), ("pic2.jpg", 10)]);
        let groups = assemble_groups(vec![group], KeepMode::First, &no_refs(), false);
        // Probe disabled in this test; kind still comes from the first member.
        assert_eq!(groups[0].media_kind, MediaKind::Image);
        let _ = fs::remove_dir_all(&dir);
    }
}
