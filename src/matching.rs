use crate::hashing::DuplicateGroup;
use crate::media::{self, MediaInfo, MediaKind};
use rayon::prelude::*;
use serde::Serialize;
use std::cmp::Ordering;
use std::fmt;
use std::path::{Path, PathBuf};

mod copies;

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

impl GroupMember {
    /// Whether a delete may remove this copy: it is not the keeper, and it
    /// does not live in a reference (locked) folder.
    ///
    /// Protection is a property of the member, not of the keep flag. A
    /// reference folder's copies win the automatic keep decision
    /// ([`ref_pool`]), but two copies of one group can sit inside the same
    /// locked folder, and only one of them can be the keeper — the other one
    /// used to look like an ordinary duplicate to every caller.
    #[must_use]
    pub fn deletable(&self) -> bool {
        !self.keep && !self.reference
    }
}

/// A set of identical files, ordered with the KEEP member first.
///
/// Serialized manually (see below) so `reclaimable_bytes` is always
/// computed from the live keep flags — keeper swaps must never go stale.
#[derive(Debug, Clone)]
pub struct Group {
    pub index: usize,
    pub hash: String,
    pub media_kind: MediaKind,
    /// Worst pairwise similarity within the group (0..=1); `None` for exact
    /// groups.
    pub similarity: Option<f64>,
    /// Why the keeper is the keeper. Set with every keep decision — the
    /// scan's, [`reassign_keepers`], and a manual pick in the GUI.
    pub keep_reason: KeepReason,
    pub members: Vec<GroupMember>,
}

impl serde::Serialize for Group {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Group", 7)?;
        state.serialize_field("index", &self.index)?;
        state.serialize_field("hash", &self.hash)?;
        state.serialize_field("kind", &self.media_kind)?;
        if let Some(similarity) = self.similarity {
            state.serialize_field("similarity", &similarity)?;
        }
        state.serialize_field("reclaimable_bytes", &self.dup_bytes())?;
        state.serialize_field("keep_reason", &self.keep_reason)?;
        state.serialize_field("members", &self.members)?;
        state.end()
    }
}

impl Group {
    #[must_use]
    pub fn keep_count(&self) -> usize {
        self.members.iter().filter(|m| m.keep).count()
    }

    /// Copies a delete would actually remove — protected copies excluded, so
    /// the count on screen matches what the delete would do.
    #[must_use]
    pub fn dup_count(&self) -> usize {
        self.members.iter().filter(|m| m.deletable()).count()
    }

    /// Bytes that would be freed by deleting every deletable member.
    #[must_use]
    pub fn dup_bytes(&self) -> u64 {
        self.members
            .iter()
            .filter(|m| m.deletable())
            .map(|m| m.size)
            .sum()
    }

    /// Why `member` stays, when it is a keeper: the group's reason, except
    /// that a protected keeper the rule placed is kept for its folder (a
    /// group merged from several can hold more than one keeper). A pick by
    /// hand leaves one keeper, and says so.
    #[must_use]
    pub fn reason_for(&self, member: &GroupMember) -> Option<KeepReason> {
        member.keep.then(|| {
            if member.reference && self.keep_reason != KeepReason::ChosenByYou {
                KeepReason::Protected
            } else {
                self.keep_reason
            }
        })
    }
}

/// Which member of a duplicate group is kept (czkawka's delete methods
/// cover the same strategies via AEN/AEO/AEB/AES codes).
///
/// Every rule breaks its ties the same way: the copy whose name and folder
/// look least like a copy (`photo.jpg` over `photo (1).jpg`, `Pictures`
/// over `Downloads`), then the first path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepMode {
    /// The original by name: no measure of its own, so the copy tiebreak
    /// and path order decide (default).
    First,
    /// Smallest file.
    Smallest,
    /// Most recently modified (unknown mtime counts as oldest).
    Newest,
    /// Least recently modified (unknown mtime counts as newest, i.e. kept
    /// only when nothing else qualifies — it sorts last).
    Oldest,
    /// Highest quality media: highest resolution first (ffprobe metadata,
    /// falling back to fingerprint dimensions), then longest duration, then
    /// largest file (bitrate proxy). Members with unknown resolution rank
    /// below any known one. Exact groups hold identical bytes, so every
    /// quality signal ties and the tiebreak decides — same as [`KeepMode::First`].
    BestQuality,
}

/// Why a group keeps the copy it keeps. Every front end shows it in the
/// same words (its [`Display`](fmt::Display)): the GUI beside the
/// **Keeping** badge, the terminal report, and the `keep_reason` of the
/// JSON, CSV and HTML reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    /// It sits in a protected (reference) folder, which always wins.
    Protected,
    /// It is sharper than another copy (similar media, `Smallest` or
    /// `BestQuality`); its own width and height.
    HighestResolution { width: u32, height: u32 },
    /// `BestQuality` with the resolution tied: longer, or a bigger file.
    BestQuality,
    /// `Smallest`: it is the smallest file.
    Smallest,
    /// `Newest`: its modification time (Unix seconds).
    Newest(i64),
    /// `Oldest`: its modification time (Unix seconds).
    Oldest(i64),
    /// The rule tied, and every other copy's name or folder looks more like
    /// a copy (`photo (1).jpg`, `Downloads\…`).
    OthersLookLikeCopies,
    /// The rule tied, and so did the copy signals: the first path in
    /// alphabetical order.
    FirstAlphabetically,
    /// The user picked it ("Keep this one instead").
    ChosenByYou,
}

impl fmt::Display for KeepReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Protected => f.write_str("in a protected folder"),
            Self::HighestResolution { width, height } => {
                write!(f, "highest resolution ({width}×{height})")
            }
            Self::BestQuality => f.write_str("best quality"),
            Self::Smallest => f.write_str("smallest file"),
            Self::Newest(secs) => write!(f, "newest ({})", local_date(secs)),
            Self::Oldest(secs) => write!(f, "oldest ({})", local_date(secs)),
            Self::OthersLookLikeCopies => f.write_str("the others look like copies"),
            Self::FirstAlphabetically => f.write_str("first alphabetically"),
            Self::ChosenByYou => f.write_str("chosen by you"),
        }
    }
}

impl Serialize for KeepReason {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// A modification time as a calendar day in the local time zone
/// ("12 Mar 2025").
fn local_date(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0).map_or_else(
        || "unknown date".to_string(),
        |t| {
            t.with_timezone(&chrono::Local)
                .format("%-d %b %Y")
                .to_string()
        },
    )
}

/// True when `path` lives under one of the reference (protected) folders.
///
/// Both sides are canonicalized once by the caller; comparison is
/// case-insensitive for Windows paths (drive-letter paths, or when running
/// on Windows) and case-sensitive otherwise. Both `/` and `\` separators
/// are honored on any host so Windows-style paths compare correctly even
/// on Linux CI (tests use `C:\...` literals everywhere).
#[must_use]
pub fn is_under_ref(path: &Path, reference_dirs: &[PathBuf]) -> bool {
    if reference_dirs.is_empty() {
        return false;
    }
    reference_dirs.iter().any(|r| {
        if path.starts_with(r) {
            return true;
        }
        let mut p = path.to_string_lossy().replace('\\', "/");
        let mut q = r.to_string_lossy().replace('\\', "/");
        while p.len() > 1 && p.ends_with('/') {
            p.pop();
        }
        while q.len() > 1 && q.ends_with('/') {
            q.pop();
        }
        let windows_path = cfg!(windows) || is_windows_drive_path(&p) || is_windows_drive_path(&q);
        if windows_path {
            p = p.to_ascii_lowercase();
            q = q.to_ascii_lowercase();
        }
        if p == q {
            return true;
        }
        if q == "/" {
            return p.starts_with('/');
        }
        p.starts_with(&format!("{q}/"))
    })
}

fn is_windows_drive_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Canonicalize reference dirs once per run (best-effort; unresolvable
/// entries fall back to the raw path).
#[must_use]
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
#[must_use]
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
                .map_or(MediaKind::Other, |m| media::classify(&m.path));
            let want_probe = probe && media_kind != MediaKind::Other;
            if want_probe {
                for member in &mut members {
                    member.media = media::probe_media(&member.path);
                }
            }

            // Choose the keeper.
            let (keep_idx, keep_reason) = pick_keeper(&members, keep, false);
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
                    keep_reason,
                    members: ordered,
                },
            )
        })
        .collect();

    let mut groups: Vec<Group> = groups.into_iter().map(|(_, g)| g).collect();
    groups.sort_by(|a, b| a.hash.cmp(&b.hash));
    groups
}

/// Pick the keeper of `members` under `keep`, and say why.
///
/// Reference-dir members are protected: the keeper always comes from them
/// when any exist. Among the candidates the rule's own measure decides
/// ([`rule_cmp`]); when it ties, the copy that looks least like a copy
/// ([`copies::copy_scores`]) wins, then the first path. `resolution`
/// selects the similar-media rule for `Smallest` (highest fingerprint
/// resolution first, then the smallest file).
pub(crate) fn pick_keeper(
    members: &[GroupMember],
    keep: KeepMode,
    resolution: bool,
) -> (usize, KeepReason) {
    let pool = ref_pool(members);
    let paths: Vec<&Path> = members.iter().map(|m| m.path.as_path()).collect();
    let copy = copies::copy_scores(&paths);
    let better = |a: usize, b: usize| match rule_cmp(&members[a], &members[b], keep, resolution) {
        Some((order, _)) => order == Ordering::Greater,
        None => (copy[a], &members[a].path) < (copy[b], &members[b].path),
    };
    let mut best = pool[0];
    for &i in &pool[1..] {
        if better(i, best) {
            best = i;
        }
    }
    (
        best,
        keep_reason(members, &pool, &copy, best, keep, resolution),
    )
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

/// Why `keeper` won `pool`: protection first; else the strongest measure of
/// the rule it beat another candidate on; else (the rule tied with every
/// candidate) the tiebreak that decided.
fn keep_reason(
    members: &[GroupMember],
    pool: &[usize],
    copy: &[u8],
    keeper: usize,
    keep: KeepMode,
    resolution: bool,
) -> KeepReason {
    let k = &members[keeper];
    if k.reference {
        return KeepReason::Protected;
    }
    let others = || pool.iter().copied().filter(move |&i| i != keeper);
    let won = others()
        .filter_map(|i| rule_cmp(k, &members[i], keep, resolution))
        .filter(|(order, _)| *order == Ordering::Greater)
        .map(|(_, measure)| measure)
        .min();
    let dims = if keep == KeepMode::Smallest {
        fingerprint_dims(k)
    } else {
        quality_dims(k)
    };
    match (won, dims, k.mtime_secs) {
        (Some(Measure::Resolution), Some((width, height)), _) => {
            KeepReason::HighestResolution { width, height }
        }
        (Some(Measure::Resolution | Measure::Quality), _, _) => KeepReason::BestQuality,
        (Some(Measure::Size), _, _) => KeepReason::Smallest,
        (Some(Measure::Time), _, Some(secs)) if keep == KeepMode::Newest => {
            KeepReason::Newest(secs)
        }
        (Some(Measure::Time), _, Some(secs)) => KeepReason::Oldest(secs),
        _ if others().next().is_some() && others().all(|i| copy[i] > copy[keeper]) => {
            KeepReason::OthersLookLikeCopies
        }
        _ => KeepReason::FirstAlphabetically,
    }
}

/// Re-apply a (possibly changed) keep mode to existing groups without
/// rescanning, reason included. Reference flags are stored per member, so
/// protection survives reassignment.
pub fn reassign_keepers(groups: &mut [Group], keep: KeepMode) {
    for g in groups {
        if g.members.is_empty() {
            continue;
        }
        let (best, reason) = pick_keeper(&g.members, keep, g.similarity.is_some());
        for (i, m) in g.members.iter_mut().enumerate() {
            m.keep = i == best;
        }
        g.keep_reason = reason;
        // Keeper first, rest by path (same order the pipeline produces).
        g.members
            .sort_by(|a, b| (!a.keep).cmp(&!b.keep).then_with(|| a.path.cmp(&b.path)));
    }
}

/// The part of a keep rule that told two copies apart, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Measure {
    Resolution,
    /// `BestQuality` past the resolution: duration, then file size.
    Quality,
    Size,
    Time,
}

/// `a` against `b` on the rule's own measure: `Greater` when `a` is the
/// better keeper, with the measure that decided; `None` when the rule cannot
/// tell them apart (always, for `First`), which leaves the copy tiebreak and
/// path order to decide.
fn rule_cmp(
    a: &GroupMember,
    b: &GroupMember,
    keep: KeepMode,
    resolution: bool,
) -> Option<(Ordering, Measure)> {
    let decided = |order: Ordering, measure| order.is_ne().then_some((order, measure));
    let area = |dims: Option<(u32, u32)>| dims.map(|(w, h)| u64::from(w) * u64::from(h));
    match keep {
        KeepMode::First => None,
        // Similar media: a smaller file is usually the re-encoded or
        // shrunk copy, so the sharpest version leads (unknown resolution
        // ranks below any known one), then the smallest file.
        KeepMode::Smallest => resolution
            .then(|| {
                decided(
                    area(fingerprint_dims(a)).cmp(&area(fingerprint_dims(b))),
                    Measure::Resolution,
                )
            })
            .flatten()
            .or_else(|| decided(b.size.cmp(&a.size), Measure::Size)),
        KeepMode::BestQuality => {
            let (qa, qb) = (quality_key(a), quality_key(b));
            decided(area(qa.0).cmp(&area(qb.0)), Measure::Resolution)
                .or_else(|| decided((qa.1, qa.2).cmp(&(qb.1, qb.2)), Measure::Quality))
        }
        // Unknown mtime sorts as oldest (Newest) / newest (Oldest): files
        // we know nothing about never win on a time rule.
        KeepMode::Newest => decided(
            a.mtime_secs
                .unwrap_or(i64::MIN)
                .cmp(&b.mtime_secs.unwrap_or(i64::MIN)),
            Measure::Time,
        ),
        KeepMode::Oldest => decided(
            b.mtime_secs
                .unwrap_or(i64::MAX)
                .cmp(&a.mtime_secs.unwrap_or(i64::MAX)),
            Measure::Time,
        ),
    }
}

/// Fingerprint dimensions (similar-media members), when known.
fn fingerprint_dims(m: &GroupMember) -> Option<(u32, u32)> {
    m.fingerprint_res.filter(|(w, h)| *w > 0 && *h > 0)
}

/// Resolution for [`KeepMode::BestQuality`]: ffprobe metadata, falling back
/// to fingerprint dimensions (always set for similar-media members).
fn quality_dims(m: &GroupMember) -> Option<(u32, u32)> {
    m.media
        .as_ref()
        .and_then(|info| match (info.width, info.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        })
        .or_else(|| fingerprint_dims(m))
}

/// Quality ranking for [`KeepMode::BestQuality`]: (resolution, duration,
/// size), each compared largest-first. Unknown resolution ranks below any
/// known one.
fn quality_key(m: &GroupMember) -> (Option<(u32, u32)>, u64, u64) {
    let duration = m
        .media
        .as_ref()
        .and_then(|info| info.duration_ms)
        .unwrap_or(0);
    (quality_dims(m), duration, m.size)
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

    fn quality_member(path: &str, size: u64, w: u32, h: u32, dur: Option<u64>) -> GroupMember {
        GroupMember {
            path: PathBuf::from(path),
            size,
            mtime_secs: None,
            keep: false,
            reference: false,
            media: Some(MediaInfo {
                width: Some(w),
                height: Some(h),
                duration_ms: dur,
                codec: None,
            }),
            similarity: None,
            content_hash: None,
            fingerprint_res: None,
        }
    }

    fn choose_keep(members: &[GroupMember], keep: KeepMode) -> usize {
        pick_keeper(members, keep, false).0
    }

    /// A plain (non-media) member with a size and modification time.
    fn plain(path: &str, size: u64, mtime: Option<i64>) -> GroupMember {
        GroupMember {
            path: PathBuf::from(path),
            size,
            mtime_secs: mtime,
            keep: false,
            reference: false,
            media: None,
            similarity: None,
            content_hash: None,
            fingerprint_res: None,
        }
    }

    /// A similar-media member: fingerprint dimensions only (0×0 = unknown).
    fn sim(path: &str, size: u64, w: u32, h: u32) -> GroupMember {
        GroupMember {
            similarity: Some(1.0),
            fingerprint_res: Some((w, h)),
            ..plain(path, size, None)
        }
    }

    fn path_of(members: &[GroupMember], pick: (usize, KeepReason)) -> (&str, KeepReason) {
        (members[pick.0].path.to_str().unwrap(), pick.1)
    }

    #[test]
    fn best_quality_prefers_highest_resolution_over_size() {
        // A 4K copy is smaller (better encoding) than the 720p copy: the
        // sharper file must win even though it is not the biggest.
        let members = vec![
            quality_member("/media/low.jpg", 8_000_000, 1280, 720, None),
            quality_member("/media/high.png", 2_000_000, 3840, 2160, None),
        ];
        assert_eq!(choose_keep(&members, KeepMode::BestQuality), 1);
    }

    #[test]
    fn best_quality_tie_breaks_by_size_then_path() {
        // Same resolution: the bigger file (bitrate proxy) wins.
        let members = vec![
            quality_member("/media/a.jpg", 100_000, 1920, 1080, None),
            quality_member("/media/b.jpg", 500_000, 1920, 1080, None),
        ];
        assert_eq!(choose_keep(&members, KeepMode::BestQuality), 1);
        // Same everything: first path wins (deterministic).
        let members = vec![
            quality_member("/media/b.jpg", 500_000, 1920, 1080, None),
            quality_member("/media/a.jpg", 500_000, 1920, 1080, None),
        ];
        assert_eq!(choose_keep(&members, KeepMode::BestQuality), 1);
    }

    #[test]
    fn best_quality_known_resolution_beats_unknown() {
        let members = vec![
            quality_member("/media/huge.jpg", 9_000_000, 0, 0, None),
            quality_member("/media/small.jpg", 100_000, 640, 480, None),
        ];
        assert_eq!(choose_keep(&members, KeepMode::BestQuality), 1);
    }

    #[test]
    fn best_quality_prefers_longer_duration() {
        let members = vec![
            quality_member("/media/short.mp4", 5_000_000, 1920, 1080, Some(30_000)),
            quality_member("/media/full.mp4", 5_000_000, 1920, 1080, Some(65_000)),
        ];
        assert_eq!(choose_keep(&members, KeepMode::BestQuality), 1);
    }

    #[test]
    fn best_quality_falls_back_to_fingerprint_res() {
        // No ffprobe metadata (e.g. binary missing): fingerprint dimensions
        // still rank the sharper file first.
        let mut high = quality_member("/media/high.png", 100_000, 0, 0, None);
        high.media = None;
        high.fingerprint_res = Some((3840, 2160));
        let mut low = quality_member("/media/low.jpg", 900_000, 0, 0, None);
        low.media = None;
        low.fingerprint_res = Some((1280, 720));
        assert_eq!(choose_keep(&[low, high], KeepMode::BestQuality), 1);
    }

    #[test]
    fn best_quality_on_exact_group_keeps_first_path() {
        // Identical bytes => every quality signal ties => first path.
        let dir = tmpdir("best-exact");
        let group = fake_group(&dir, &[("z.txt", 10), ("a.txt", 10)]);
        let groups = assemble_groups(vec![group], KeepMode::BestQuality, &no_refs(), false);
        assert_eq!(
            groups[0].members.first().unwrap().path.file_name().unwrap(),
            "a.txt"
        );
        let _ = fs::remove_dir_all(&dir);
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
        let old_t = SystemTime::now() - std::time::Duration::from_hours(1);
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
        assert_eq!(groups[0].keep_reason, KeepReason::Protected);
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

    /// The scan the user runs: `photo.jpg` and `photo (1).jpg`, identical.
    /// "photo (1)" sorts first byte-wise, but the original by name stays,
    /// and the group says why — even inside a temp folder, which counts
    /// against both copies alike.
    #[test]
    fn first_keeps_the_original_name_over_a_numbered_copy() {
        let dir = tmpdir("copy-name");
        let group = fake_group(&dir, &[("photo (1).jpg", 10), ("photo.jpg", 10)]);
        let groups = assemble_groups(vec![group], KeepMode::First, &no_refs(), false);
        let keeper = groups[0].members.first().unwrap();
        assert_eq!(keeper.path.file_name().unwrap(), "photo.jpg");
        assert!(keeper.keep);
        assert_eq!(groups[0].keep_reason, KeepReason::OthersLookLikeCopies);
        let json = serde_json::to_value(&groups[0]).unwrap();
        assert_eq!(json["keep_reason"], "the others look like copies");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_reason_when_nothing_looks_like_a_copy() {
        let members = vec![plain("/p/b.txt", 5, None), plain("/p/a.txt", 5, None)];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::First, false)),
            ("/p/a.txt", KeepReason::FirstAlphabetically)
        );
        // A copy folder counts like a copy name.
        let members = vec![
            plain("/home/me/Downloads/a.jpg", 5, None),
            plain("/home/me/Pictures/a.jpg", 5, None),
        ];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::First, false)),
            ("/home/me/Pictures/a.jpg", KeepReason::OthersLookLikeCopies)
        );
        // One other copy looks like the original too: the tie went to path
        // order, so the reason must not claim every other one is a copy.
        let members = vec![
            plain("/p/a (1).jpg", 5, None),
            plain("/p/b.jpg", 5, None),
            plain("/p/a.jpg", 5, None),
        ];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::First, false)),
            ("/p/a.jpg", KeepReason::FirstAlphabetically)
        );
    }

    #[test]
    fn smallest_reason_and_ties() {
        // The rule's own measure beats the copy tiebreak.
        let members = vec![plain("/p/a.txt", 10, None), plain("/p/a (1).txt", 4, None)];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Smallest, false)),
            ("/p/a (1).txt", KeepReason::Smallest)
        );
        let members = vec![plain("/p/a (1).txt", 4, None), plain("/p/a.txt", 4, None)];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Smallest, false)),
            ("/p/a.txt", KeepReason::OthersLookLikeCopies)
        );
        let members = vec![plain("/p/b.txt", 4, None), plain("/p/a.txt", 4, None)];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Smallest, false)),
            ("/p/a.txt", KeepReason::FirstAlphabetically)
        );
    }

    #[test]
    fn newest_and_oldest_reasons_carry_the_date() {
        let members = vec![
            plain("/p/a.txt", 1, Some(100)),
            plain("/p/b.txt", 1, Some(300)),
            plain("/p/c.txt", 1, None),
        ];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Newest, false)),
            ("/p/b.txt", KeepReason::Newest(300))
        );
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Oldest, false)),
            ("/p/a.txt", KeepReason::Oldest(100))
        );
        // Newest beats one copy and ties another: still "newest" — it is.
        let members = vec![
            plain("/p/b copy.txt", 1, Some(300)),
            plain("/p/b.txt", 1, Some(300)),
            plain("/p/a.txt", 1, Some(100)),
        ];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Newest, false)),
            ("/p/b.txt", KeepReason::Newest(300))
        );
        // Every time ties: the tiebreak speaks.
        let members = vec![
            plain("/p/b copy.txt", 1, Some(5)),
            plain("/p/b.txt", 1, Some(5)),
        ];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Oldest, false)),
            ("/p/b.txt", KeepReason::OthersLookLikeCopies)
        );
    }

    #[test]
    fn best_quality_reasons() {
        let members = vec![
            quality_member("/m/low.jpg", 8_000_000, 1280, 720, None),
            quality_member("/m/high.png", 2_000_000, 4032, 3024, None),
        ];
        let pick = pick_keeper(&members, KeepMode::BestQuality, false);
        assert_eq!(
            path_of(&members, pick),
            (
                "/m/high.png",
                KeepReason::HighestResolution {
                    width: 4032,
                    height: 3024
                }
            )
        );
        assert_eq!(pick.1.to_string(), "highest resolution (4032×3024)");
        // Same resolution: the longer clip is the better one.
        let members = vec![
            quality_member("/m/full.mp4", 5_000_000, 1920, 1080, Some(65_000)),
            quality_member("/m/cut.mp4", 5_000_000, 1920, 1080, Some(30_000)),
        ];
        assert_eq!(
            path_of(
                &members,
                pick_keeper(&members, KeepMode::BestQuality, false)
            ),
            ("/m/full.mp4", KeepReason::BestQuality)
        );
        // Everything ties (an exact group): the tiebreak decides.
        let members = vec![
            quality_member("/m/a - Copy.jpg", 9, 640, 480, None),
            quality_member("/m/a.jpg", 9, 640, 480, None),
        ];
        assert_eq!(
            path_of(
                &members,
                pick_keeper(&members, KeepMode::BestQuality, false)
            ),
            ("/m/a.jpg", KeepReason::OthersLookLikeCopies)
        );
    }

    #[test]
    fn similar_smallest_prefers_resolution_then_smallest_file() {
        // A 4K PNG is bigger than a 720p JPG, but the sharper one stays.
        let members = vec![
            sim("/m/low.jpg", 200_000, 1280, 720),
            sim("/m/high.png", 8_000_000, 3840, 2160),
        ];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Smallest, true)),
            (
                "/m/high.png",
                KeepReason::HighestResolution {
                    width: 3840,
                    height: 2160
                }
            )
        );
        // Same resolution: the smallest file.
        let members = vec![
            sim("/m/c.png", 7_000_000, 1920, 1080),
            sim("/m/a.png", 5_000_000, 1920, 1080),
            sim("/m/b.png", 3_000_000, 1920, 1080),
        ];
        assert_eq!(
            path_of(&members, pick_keeper(&members, KeepMode::Smallest, true)),
            ("/m/b.png", KeepReason::Smallest)
        );
        // Unknown resolution (no ffprobe) falls back to the smallest file,
        // and loses to any known resolution.
        let members = vec![
            sim("/m/a.mp4", 10_000_000, 0, 0),
            sim("/m/b.mp4", 5_000_000, 0, 0),
        ];
        assert_eq!(choose_keep_res(&members, KeepMode::Smallest), 1);
        let members = vec![
            sim("/m/tiny.jpg", 50_000, 0, 0),
            sim("/m/known.png", 900_000, 1920, 1080),
        ];
        assert_eq!(choose_keep_res(&members, KeepMode::Smallest), 1);
    }

    #[test]
    fn similar_best_quality_prefers_resolution_then_larger_file() {
        let members = vec![
            sim("/m/a.jpg", 100_000, 1920, 1080),
            sim("/m/b.jpg", 500_000, 1920, 1080),
        ];
        assert_eq!(choose_keep_res(&members, KeepMode::BestQuality), 1);
        let members = vec![
            sim("/m/huge.jpg", 9_000_000, 0, 0),
            sim("/m/known.jpg", 100_000, 640, 480),
        ];
        assert_eq!(choose_keep_res(&members, KeepMode::BestQuality), 1);
    }

    fn choose_keep_res(members: &[GroupMember], keep: KeepMode) -> usize {
        pick_keeper(members, keep, true).0
    }

    #[test]
    fn protected_keeper_says_so_whatever_the_rule_prefers() {
        // The protected copy is bigger, older and named like a copy: it
        // still stays, and the reason is the folder.
        let mut protected = plain("/backup/photo (1).jpg", 50, Some(1));
        protected.reference = true;
        let members = vec![plain("/in/photo.jpg", 10, Some(9)), protected];
        for keep in [
            KeepMode::First,
            KeepMode::Smallest,
            KeepMode::Newest,
            KeepMode::BestQuality,
        ] {
            assert_eq!(
                path_of(&members, pick_keeper(&members, keep, false)),
                ("/backup/photo (1).jpg", KeepReason::Protected),
                "{keep:?}"
            );
        }
    }

    #[test]
    fn reassign_keepers_rewrites_the_reason() {
        let mut groups = vec![Group {
            index: 1,
            hash: "h".into(),
            media_kind: MediaKind::Other,
            similarity: None,
            keep_reason: KeepReason::ChosenByYou,
            members: vec![
                plain("/p/a.txt", 10, Some(1)),
                plain("/p/b.txt", 3, Some(2)),
            ],
        }];
        groups[0].members[0].keep = true;
        reassign_keepers(&mut groups, KeepMode::Smallest);
        assert_eq!(groups[0].members[0].path, PathBuf::from("/p/b.txt"));
        assert_eq!(groups[0].keep_reason, KeepReason::Smallest);
        reassign_keepers(&mut groups, KeepMode::First);
        assert_eq!(groups[0].members[0].path, PathBuf::from("/p/a.txt"));
        assert_eq!(groups[0].keep_reason, KeepReason::FirstAlphabetically);
    }

    #[test]
    fn reasons_read_as_plain_words() {
        assert_eq!(KeepReason::Protected.to_string(), "in a protected folder");
        assert_eq!(KeepReason::ChosenByYou.to_string(), "chosen by you");
        // 12 Mar 2025 12:00 UTC: the same calendar day in nearly every zone.
        let newest = KeepReason::Newest(1_741_780_800).to_string();
        assert!(
            newest.starts_with("newest (") && newest.ends_with(" Mar 2025)"),
            "{newest}"
        );
    }
}
