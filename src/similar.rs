//! Near-duplicate (perceptual) detection for images and videos.
//!
//! Exact duplicate detection compares content bytes; this module finds media
//! that is visually the same content in a different format or encoding:
//! - images: 64-bit difference hash (dHash) via the `image` crate,
//! - videos: 64-bit dHash of 8 evenly-spaced frames extracted with `ffmpeg`
//!   in a single decode pass (long videos fall back to per-frame seeks),
//!   compared as frame sequences with a small temporal tolerance.
//!
//! Fingerprints are persisted across runs in a per-user cache keyed by
//! path+size+mtime, so unchanged files are never re-decoded or re-sampled
//! (see the `cache` module).
//!
//! Files whose similarity is at or above a threshold (default 97%) are treated
//! as duplicates. Groups are formed by star clustering: the first unassigned
//! file becomes a group's keeper, and every later file joins the first keeper
//! it matches. This is deterministic (path order) and avoids the transitivity
//! problem of union-find (unrelated files can never chain into one group).

use crate::cache::{CacheFp, CacheWrite, FingerprintCache};
use crate::hashing::HashEngine;
use crate::matching::{self, Group, GroupMember, KeepMode};
use crate::media::{self, MediaInfo, MediaKind};
use crate::scan::FileEntry;
use anyhow::Result;
use indicatif::ProgressBar;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Default similarity threshold, as a percentage (0-100), surfaced as the
/// `--similarity` default. >= 97% => duplicates.
pub const DEFAULT_SIMILARITY_PCT: f64 = 97.0;

/// Number of frames sampled from each video.
const SIMILAR_FRAMES: usize = 8;
/// Fingerprint grid: (FRAME_W - 1) * FRAME_H bits = 16 * 16 = 256 bits
/// (4 x u64). The old 9x8 / 64-bit grid collided on videos sharing only a
/// coarse layout (e.g. dark sides + bright center skin tones at 9px wide),
/// grouping completely different clips at 98-100% similar.
const FRAME_W: usize = 17;
const FRAME_H: usize = 16;
/// Bits per frame fingerprint.
const HASH_BITS: f64 = 256.0;
/// One frame fingerprint: 256-bit difference hash.
pub type FrameHash = [u64; 4];

/// Videos longer than this use per-frame `-ss` seeks (decoding the whole
/// stream to sample 8 frames would be wasteful); shorter ones are decoded
/// once in a single fast pass. Measured: one pass is ~7-10x faster than 8
/// separate ffmpeg spawns for clips up to a minute or so.
const SEEK_THRESHOLD_MS: u64 = 90_000;

pub struct SimilarConfig {
    pub threshold: f64,
}

/// One file in a similar group, with its similarity (0..=1) against the
/// group's keeper (1.0 for the keeper itself), and its resolution (0 when
/// unknown, e.g. a video probed without ffprobe).
pub struct SimilarMember {
    pub entry: FileEntry,
    pub similarity: f64,
    pub w: u32,
    pub h: u32,
}

/// A set of files judged to be the same content (keeper first).
pub struct SimilarGroup {
    pub kind: MediaKind,
    pub members: Vec<SimilarMember>,
}

/// 256-bit difference hash: for each of the 16 rows, compare the 16
/// horizontally-adjacent pixel pairs on a 17x16 grid. Only the *relative*
/// brightness order is kept, so the hash is robust to re-encoding, format
/// changes and slight exposure differences.
pub fn dhash_gray(w: usize, h: usize, gray: &[u8]) -> FrameHash {
    debug_assert!(gray.len() >= w * h);
    let mut hash: FrameHash = [0; 4];
    let mut bit = 0usize;
    'outer: for y in 0..h {
        let row = y * w;
        for x in 0..w.saturating_sub(1) {
            if gray[row + x] > gray[row + x + 1] {
                hash[bit / 64] |= 1u64 << (bit % 64);
            }
            bit += 1;
            if bit == 256 {
                break 'outer;
            }
        }
    }
    hash
}

/// Fraction of identical bits between two 256-bit hashes (0..=1).
pub fn hash_similarity(a: &FrameHash, b: &FrameHash) -> f64 {
    let mut diff = 0u32;
    for i in 0..4 {
        diff += (a[i] ^ b[i]).count_ones();
    }
    1.0 - diff as f64 / HASH_BITS
}

/// Mean over `a`'s frames of the best similarity found inside a ±`window`
/// neighborhood of `b` — tolerates small temporal drift between encodings.
/// NOTE: asymmetric — use [`frame_seq_similarity_sym`] for grouping so a
/// static clip (all frames identical) cannot score 1.0 against a varied
/// clip just because its single repeated hash appears in the other.
pub fn frame_seq_similarity(a: &[FrameHash], b: &[FrameHash], window: usize) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut total = 0.0;
    for (i, ha) in a.iter().enumerate() {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(b.len());
        let mut best: f64 = 0.0;
        for hb in &b[lo..hi] {
            best = best.max(hash_similarity(ha, hb));
        }
        total += best;
    }
    total / a.len() as f64
}

/// Symmetric sequence similarity: min of both directions. Prevents the
/// false-positive where clip B's uniform frames are all contained in clip A
/// (B→A = 1.0) while A→B is lower — the pair must match both ways.
pub fn frame_seq_similarity_sym(a: &[FrameHash], b: &[FrameHash], window: usize) -> f64 {
    frame_seq_similarity(a, b, window).min(frame_seq_similarity(b, a, window))
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

struct ImageFp {
    entry: FileEntry,
    hash: FrameHash,
    w: u32,
    h: u32,
}

/// Read only the image header (no pixel decode) — cheap enough to run for
/// every image so we can bucket by aspect ratio before decoding anything.
fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

fn image_fingerprint(entry: FileEntry) -> Option<ImageFp> {
    let img = image::open(&entry.path).ok()?;
    let gray = img.to_luma8();
    let (w, h) = (gray.width(), gray.height());
    if w == 0 || h == 0 {
        return None;
    }
    let small = image::imageops::resize(
        &gray,
        FRAME_W as u32,
        FRAME_H as u32,
        image::imageops::FilterType::Triangle,
    );
    let hash = dhash_gray(FRAME_W, FRAME_H, small.as_raw());
    Some(ImageFp { entry, hash, w, h })
}

/// Bucket key: aspect ratio rounded to 1/100, so re-scaled versions of the
/// same photo (same ratio) are compared against each other.
fn aspect_key(w: u32, h: u32) -> i64 {
    if h == 0 {
        return 0;
    }
    ((w as f64 / h as f64) * 100.0).round() as i64
}

fn cluster_images(
    fps: &[ImageFp],
    buckets: HashMap<i64, Vec<usize>>,
    threshold: f64,
) -> Vec<SimilarGroup> {
    let mut groups = Vec::new();
    for mut idxs in buckets.into_values() {
        idxs.sort_by(|&a, &b| fps[a].entry.path.cmp(&fps[b].entry.path));
        let mut leaders: Vec<usize> = Vec::new();
        let mut cluster: Vec<Vec<usize>> = Vec::new();
        for &i in &idxs {
            let mut joined = None;
            for (gi, &leader) in leaders.iter().enumerate() {
                if hash_similarity(&fps[i].hash, &fps[leader].hash) >= threshold {
                    joined = Some(gi);
                    break;
                }
            }
            match joined {
                Some(gi) => cluster[gi].push(i),
                None => {
                    leaders.push(i);
                    cluster.push(vec![i]);
                }
            }
        }
        for c in cluster {
            if c.len() >= 2 {
                groups.push(make_image_group(fps, c));
            }
        }
    }
    groups
}

fn make_image_group(fps: &[ImageFp], cluster: Vec<usize>) -> SimilarGroup {
    let keeper = cluster[0];
    let keeper_hash = fps[keeper].hash;
    let members = cluster
        .into_iter()
        .map(|i| SimilarMember {
            entry: fps[i].entry.clone(),
            similarity: hash_similarity(&fps[i].hash, &keeper_hash),
            w: fps[i].w,
            h: fps[i].h,
        })
        .collect();
    SimilarGroup {
        kind: MediaKind::Image,
        members,
    }
}

// ---------------------------------------------------------------------------
// Videos
// ---------------------------------------------------------------------------

struct VideoFp {
    entry: FileEntry,
    w: u32,
    h: u32,
    duration_ms: u64,
    frames: Vec<FrameHash>,
}

/// Sample `SIMILAR_FRAMES` evenly-spaced frames. With a known duration we use
/// one fast ffmpeg pass for short videos (a single full decode, `fps` filter
/// picks evenly time-spaced frames) or `-ss` keyframe seeks for long ones;
/// without a duration we fall back to a single `fps=1` pass (decodes the
/// whole stream, subsampled).
fn video_fingerprint(entry: FileEntry, probe: Option<&MediaInfo>) -> Option<VideoFp> {
    if let Some(info) = probe {
        let (w, h) = (info.width?, info.height?);
        let dur = info.duration_ms?;
        let frames = if dur <= SEEK_THRESHOLD_MS {
            extract_frames_pass(&entry.path, dur)
        } else {
            extract_frames_seek(&entry.path, dur)
        }?;
        Some(VideoFp {
            entry,
            w,
            h,
            duration_ms: dur,
            frames,
        })
    } else {
        let frames = extract_frames_oneshot(&entry.path)?;
        if frames.is_empty() {
            return None;
        }
        // Approximate duration from 1 fps output length (used for bucketing).
        let dur = frames.len() as u64 * 1000;
        Some(VideoFp {
            entry,
            w: 0,
            h: 0,
            duration_ms: dur,
            frames,
        })
    }
}

/// Sample `SIMILAR_FRAMES` frames in a single decode pass: the `fps` filter
/// emits one frame every `duration / SIMILAR_FRAMES` seconds, so the sampled
/// frames are evenly spaced over the whole clip. One spawn decodes the entire
/// stream once (cheap for short clips) instead of starting `SIMILAR_FRAMES`
/// separate ffmpeg processes.
fn extract_frames_pass(path: &Path, duration_ms: u64) -> Option<Vec<FrameHash>> {
    let dur_secs = duration_ms as f64 / 1000.0;
    if dur_secs <= 0.0 {
        return None;
    }
    let fps = format!("{SIMILAR_FRAMES}/{dur_secs:.6}");
    let out = crate::util::quiet_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .arg("-vf")
        .arg(format!("scale={FRAME_W}:{FRAME_H},fps={fps}"))
        .args(["-frames:v", &SIMILAR_FRAMES.to_string()])
        .args(["-f", "rawvideo", "-pix_fmt", "gray", "-"])
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.len() < FRAME_W * FRAME_H {
        return None;
    }
    Some(
        out.stdout
            .as_chunks::<{ FRAME_W * FRAME_H }>()
            .0
            .iter()
            .map(|f| dhash_gray(FRAME_W, FRAME_H, f))
            .collect(),
    )
}

fn extract_frames_seek(path: &Path, duration_ms: u64) -> Option<Vec<FrameHash>> {
    let secs = duration_ms as f64 / 1000.0;
    let mut hashes = Vec::with_capacity(SIMILAR_FRAMES);
    for i in 0..SIMILAR_FRAMES {
        let t = secs * (i as f64 + 0.5) / SIMILAR_FRAMES as f64;
        let out = crate::util::quiet_command("ffmpeg")
            .args(["-v", "error", "-ss"])
            .arg(format!("{t:.6}"))
            .args(["-i"])
            .arg(path)
            .args([
                "-frames:v",
                "1",
                "-vf",
                &format!("scale={FRAME_W}:{FRAME_H}"),
                "-f",
                "rawvideo",
                "-pix_fmt",
                "gray",
                "-",
            ])
            .output()
            .ok()?;
        if !out.status.success() || out.stdout.len() < FRAME_W * FRAME_H {
            return None;
        }
        hashes.push(dhash_gray(
            FRAME_W,
            FRAME_H,
            &out.stdout[..FRAME_W * FRAME_H],
        ));
    }
    Some(hashes)
}

fn extract_frames_oneshot(path: &Path) -> Option<Vec<FrameHash>> {
    let out = crate::util::quiet_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("scale={FRAME_W}:{FRAME_H},fps=1"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "-",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut all: Vec<FrameHash> = out
        .stdout
        .as_chunks::<{ FRAME_W * FRAME_H }>()
        .0
        .iter()
        .map(|f| dhash_gray(FRAME_W, FRAME_H, f))
        .collect();
    if all.is_empty() {
        return None;
    }
    if all.len() > SIMILAR_FRAMES {
        let step = all.len() as f64 / SIMILAR_FRAMES as f64;
        all = (0..SIMILAR_FRAMES)
            .map(|i| all[(i as f64 * step).round() as usize])
            .collect();
    }
    Some(all)
}

/// Durations within 2s (or 5% of the longer one, whichever is bigger) may be
/// the same video re-encoded; anything else cannot be.
fn durations_close(a_ms: u64, b_ms: u64) -> bool {
    let tol = 2_000u64.max((a_ms.max(b_ms) as f64 * 0.05) as u64);
    a_ms.abs_diff(b_ms) <= tol
}

fn duration_close(a: &VideoFp, b: &VideoFp) -> bool {
    durations_close(a.duration_ms, b.duration_ms)
}

fn cluster_videos(
    fps: &[VideoFp],
    buckets: HashMap<(u32, u32), Vec<usize>>,
    threshold: f64,
) -> Vec<SimilarGroup> {
    let mut groups = Vec::new();
    for mut idxs in buckets.into_values() {
        idxs.sort_by(|&a, &b| fps[a].entry.path.cmp(&fps[b].entry.path));
        let mut leaders: Vec<usize> = Vec::new();
        let mut cluster: Vec<Vec<usize>> = Vec::new();
        for &i in &idxs {
            let mut joined = None;
            for (gi, &leader) in leaders.iter().enumerate() {
                if duration_close(&fps[i], &fps[leader])
                    && frame_seq_similarity_sym(&fps[i].frames, &fps[leader].frames, 2) >= threshold
                {
                    joined = Some(gi);
                    break;
                }
            }
            match joined {
                Some(gi) => cluster[gi].push(i),
                None => {
                    leaders.push(i);
                    cluster.push(vec![i]);
                }
            }
        }
        for c in cluster {
            if c.len() >= 2 {
                groups.push(make_video_group(fps, c));
            }
        }
    }
    groups
}

fn make_video_group(fps: &[VideoFp], cluster: Vec<usize>) -> SimilarGroup {
    let keeper = cluster[0];
    let members = cluster
        .into_iter()
        .map(|i| SimilarMember {
            entry: fps[i].entry.clone(),
            similarity: frame_seq_similarity_sym(&fps[i].frames, &fps[keeper].frames, 2),
            w: fps[i].w,
            h: fps[i].h,
        })
        .collect();
    SimilarGroup {
        kind: MediaKind::Video,
        members,
    }
}

// ---------------------------------------------------------------------------
// Top level
// ---------------------------------------------------------------------------

/// Find perceptual duplicates among `entries` (which should be the files NOT
/// already claimed by an exact-duplicate group). `probe` says whether ffprobe
/// is available (needed for the fast video sampling path). `cache` supplies
/// fingerprints from earlier runs — a hit skips decoding / frame sampling
/// entirely for that file (valid only while its size and mtime are unchanged).
/// When given, a `progress` bar spans the fingerprinting work (length = number
/// of media files). Expensive work is pruned up front: images are bucketed by
/// aspect ratio with header-only reads (no decode) and videos by resolution +
/// a cheap duration check, so only files with a possible partner are
/// fingerprinted. Returns the groups plus the newly computed fingerprints,
/// which the caller should store back into the cache.
pub fn find_similar(
    entries: Vec<FileEntry>,
    config: &SimilarConfig,
    probe: bool,
    cache: Option<&FingerprintCache>,
    progress: Option<&ProgressBar>,
) -> (Vec<SimilarGroup>, Vec<CacheWrite>) {
    let mut images = Vec::new();
    let mut videos = Vec::new();
    for e in entries {
        match media::classify(&e.path) {
            MediaKind::Image => images.push(e),
            MediaKind::Video => videos.push(e),
            MediaKind::Other => {}
        }
    }
    let total = images.len() + videos.len();
    if let Some(bar) = progress {
        bar.set_length(total as u64);
        bar.set_message("fingerprinting media");
    }

    let mut groups = Vec::new();
    let mut writes: Vec<CacheWrite> = Vec::new();

    // Images: bucket by aspect ratio using header-only reads, then decode and
    // dHash only images that share a bucket with a possible partner. Files
    // whose fingerprint is already cached skip the header read AND the decode.
    let mut aspect_buckets: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut cached_img: HashMap<usize, (u32, u32, FrameHash)> = HashMap::new();
    for (i, e) in images.iter().enumerate() {
        if let Some(CacheFp::Image { w, h, hash }) =
            cache.and_then(|c| c.get(&e.path, e.size, e.mtime_secs))
        {
            cached_img.insert(i, (*w, *h, *hash));
            aspect_buckets
                .entry(aspect_key(*w, *h))
                .or_default()
                .push(i);
            continue;
        }
        if let Some((w, h)) = image_dimensions(&e.path) {
            aspect_buckets.entry(aspect_key(w, h)).or_default().push(i);
        }
    }
    let needs_fp: HashSet<usize> = aspect_buckets
        .values()
        .filter(|v| v.len() >= 2)
        .flatten()
        .copied()
        .filter(|i| !cached_img.contains_key(i))
        .collect();
    let new_fps: Vec<(usize, ImageFp)> = images
        .par_iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let fp = if needs_fp.contains(&i) {
                image_fingerprint(e.clone())
            } else {
                None
            };
            if let Some(bar) = progress {
                bar.inc(1);
            }
            fp.map(|fp| (i, fp))
        })
        .collect();
    writes.extend(new_fps.iter().filter_map(|(i, fp)| {
        let e = &images[*i];
        e.mtime_secs.map(|mt| CacheWrite {
            path: e.path.clone(),
            size: e.size,
            mtime_secs: mt,
            fp: CacheFp::Image {
                w: fp.w,
                h: fp.h,
                hash: fp.hash,
            },
        })
    }));
    let mut img_fps: Vec<ImageFp> = cached_img
        .iter()
        .map(|(&i, &(w, h, hash))| ImageFp {
            entry: images[i].clone(),
            hash,
            w,
            h,
        })
        .collect();
    img_fps.extend(new_fps.into_iter().map(|(_, fp)| fp));
    if !img_fps.is_empty() {
        let mut buckets: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, fp) in img_fps.iter().enumerate() {
            buckets.entry(aspect_key(fp.w, fp.h)).or_default().push(i);
        }
        groups.extend(cluster_images(&img_fps, buckets, config.threshold));
    }

    // Videos: probe headers (if possible), bucket by resolution, and only
    // extract frames for videos that have a duration-compatible partner in
    // their bucket — skips the expensive ffmpeg sampling for everything
    // unique. Cached fingerprints provide probe-equivalent metadata and skip
    // the sampling entirely.
    if !videos.is_empty() {
        let mut cached_vid: HashMap<usize, (u32, u32, u64, Vec<FrameHash>)> = HashMap::new();
        for (i, e) in videos.iter().enumerate() {
            if let Some(CacheFp::Video {
                w,
                h,
                duration_ms,
                frames,
            }) = cache.and_then(|c| c.get(&e.path, e.size, e.mtime_secs))
            {
                cached_vid.insert(i, (*w, *h, *duration_ms, frames.clone()));
            }
        }
        let probe_map: HashMap<std::path::PathBuf, MediaInfo> = if probe {
            videos
                .par_iter()
                .filter_map(|e| media::probe_media(&e.path).map(|m| (e.path.clone(), m)))
                .collect()
        } else {
            HashMap::new()
        };
        let mut res_buckets: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        let mut unprobed: Vec<usize> = Vec::new();
        for (i, e) in videos.iter().enumerate() {
            let res = cached_vid.get(&i).map(|v| (v.0, v.1)).or_else(|| {
                probe_map
                    .get(&e.path)
                    .and_then(|m| Some((m.width?, m.height?)))
            });
            match res {
                Some((w, h)) => res_buckets.entry((w, h)).or_default().push(i),
                None => unprobed.push(i),
            }
        }
        let mut cand: HashSet<usize> = HashSet::new();
        for idxs in res_buckets.values() {
            if idxs.len() < 2 {
                continue;
            }
            let dur: Vec<Option<u64>> = idxs
                .iter()
                .map(|&i| {
                    cached_vid
                        .get(&i)
                        .map(|v| v.2)
                        .or_else(|| probe_map.get(&videos[i].path).and_then(|m| m.duration_ms))
                })
                .collect();
            for (k, &i) in idxs.iter().enumerate() {
                let Some(di) = dur[k] else { continue };
                if dur
                    .iter()
                    .enumerate()
                    .any(|(j, dj)| j != k && dj.is_some_and(|d| durations_close(di, d)))
                {
                    cand.insert(i);
                }
            }
        }
        cand.extend(unprobed);
        let new_fps: Vec<(usize, VideoFp)> = videos
            .par_iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let fp = if cand.contains(&i) && !cached_vid.contains_key(&i) {
                    let info = probe_map.get(&e.path);
                    video_fingerprint(e.clone(), info)
                } else {
                    None
                };
                if let Some(bar) = progress {
                    bar.inc(1);
                }
                fp.map(|fp| (i, fp))
            })
            .collect();
        writes.extend(new_fps.iter().filter_map(|(i, fp)| {
            let e = &videos[*i];
            e.mtime_secs.map(|mt| CacheWrite {
                path: e.path.clone(),
                size: e.size,
                mtime_secs: mt,
                fp: CacheFp::Video {
                    w: fp.w,
                    h: fp.h,
                    duration_ms: fp.duration_ms,
                    frames: fp.frames.clone(),
                },
            })
        }));
        let mut vid_fps: Vec<VideoFp> = cached_vid
            .iter()
            .map(|(&i, &(w, h, dur, ref frames))| VideoFp {
                entry: videos[i].clone(),
                w,
                h,
                duration_ms: dur,
                frames: frames.clone(),
            })
            .collect();
        vid_fps.extend(new_fps.into_iter().map(|(_, fp)| fp));
        if !vid_fps.is_empty() {
            let mut buckets: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
            for (i, fp) in vid_fps.iter().enumerate() {
                buckets.entry((fp.w, fp.h)).or_default().push(i);
            }
            groups.extend(cluster_videos(&vid_fps, buckets, config.threshold));
        }
    }

    (groups, writes)
}

/// Convert perceptual groups into report-ready `Group`s (with media metadata,
/// keep decisions, and a content hash per member used as a deletion safety
/// check). `start_index` continues the numbering after the exact groups.
pub fn build_similar_groups(
    groups: Vec<SimilarGroup>,
    keep: KeepMode,
    reference_dirs: &[PathBuf],
    probe: bool,
    engine: &HashEngine,
    start_index: usize,
    progress: Option<&ProgressBar>,
) -> Result<Vec<Group>> {
    if let Some(bar) = progress {
        bar.set_position(0);
        bar.set_length(groups.len() as u64);
        bar.set_message("building similar groups");
    }
    let built: Vec<Group> = groups
        .into_par_iter()
        .enumerate()
        .map(|(i, sg)| {
            if let Some(bar) = progress {
                bar.inc(1);
            }
            let index = start_index + i;
            let mut members: Vec<GroupMember> = sg
                .members
                .iter()
                .map(|m| {
                    let reference = !reference_dirs.is_empty()
                        && matching::is_under_ref(
                            &matching::canonical_member(&m.entry.path),
                            reference_dirs,
                        );
                    GroupMember {
                        path: m.entry.path.clone(),
                        size: m.entry.size,
                        mtime_secs: m.entry.mtime_secs,
                        keep: false,
                        reference,
                        media: None,
                        similarity: Some(m.similarity),
                        content_hash: None,
                        fingerprint_res: (m.w > 0 && m.h > 0).then_some((m.w, m.h)),
                    }
                })
                .collect();

            if probe {
                for m in &mut members {
                    m.media = media::probe_media(&m.path);
                }
            }
            // Content hash per member: re-verified before any deletion so a
            // changed file is never removed, even though the members' bytes
            // legitimately differ from each other.
            for m in &mut members {
                m.content_hash = engine.full(&m.path).ok();
            }

            // Keeper: reference-dir members are protected and win; among
            // the candidates the keep rule applies — for `Smallest` the
            // highest-resolution version (a smaller file is usually the
            // re-encoded/lower-res copy, so the best original is kept),
            // falling back to smallest size when unknown.
            let pool: Vec<usize> = {
                let refs: Vec<usize> = sg
                    .members
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| {
                        matching::is_under_ref(
                            &matching::canonical_member(&m.entry.path),
                            reference_dirs,
                        ) && !reference_dirs.is_empty()
                    })
                    .map(|(i, _)| i)
                    .collect();
                if refs.is_empty() {
                    (0..sg.members.len()).collect()
                } else {
                    refs
                }
            };
            let keep_idx = best_similar_keep(&sg.members, &pool, keep);
            members[keep_idx].keep = true;
            let keeper = members.remove(keep_idx);
            let mut rest = members;
            rest.sort_by(|a, b| a.path.cmp(&b.path));
            let mut ordered = vec![keeper];
            ordered.extend(rest);

            let similarity = sg
                .members
                .iter()
                .map(|m| m.similarity)
                .fold(1.0f64, f64::min);

            Group {
                index,
                hash: format!("similar#{index}"),
                media_kind: sg.kind,
                similarity: Some(similarity),
                members: ordered,
            }
        })
        .collect();
    Ok(built)
}

/// Best keeper index among `pool` under `keep`.
///
/// `Smallest` prefers the highest resolution (width ? height); members with
/// unknown resolution (0?0, e.g. a video probed without ffprobe) rank below
/// any member with known resolution, falling back to smallest size.
fn best_similar_keep(members: &[SimilarMember], pool: &[usize], keep: KeepMode) -> usize {
    let area = |m: &SimilarMember| -> Option<u64> {
        (m.w > 0 && m.h > 0).then_some(m.w as u64 * m.h as u64)
    };
    let mut best = pool[0];
    for &i in &pool[1..] {
        let (m, b) = (&members[i], &members[best]);
        let better = match keep {
            KeepMode::First => m.entry.path < b.entry.path,
            KeepMode::Smallest => match (area(m), area(b)) {
                (Some(a), Some(ba)) => {
                    a > ba
                        || (a == ba
                            && (m.entry.size, &m.entry.path) < (b.entry.size, &b.entry.path))
                }
                (Some(_), None) => true,
                (None, None) => (m.entry.size, &m.entry.path) < (b.entry.size, &b.entry.path),
                (None, Some(_)) => false,
            },
            KeepMode::Newest => {
                (
                    m.entry.mtime_secs.unwrap_or(i64::MIN),
                    std::cmp::Reverse(&m.entry.path),
                ) > (
                    b.entry.mtime_secs.unwrap_or(i64::MIN),
                    std::cmp::Reverse(&b.entry.path),
                )
            }
            KeepMode::Oldest => {
                (m.entry.mtime_secs.unwrap_or(i64::MAX), &m.entry.path)
                    < (b.entry.mtime_secs.unwrap_or(i64::MAX), &b.entry.path)
            }
        };
        if better {
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &Path, size: u64) -> FileEntry {
        FileEntry {
            path: path.to_path_buf(),
            size,
            mtime_secs: None,
        }
    }

    #[test]
    fn dhash_constant_image_is_zero() {
        let gray = vec![100u8; FRAME_W * FRAME_H];
        assert_eq!(dhash_gray(FRAME_W, FRAME_H, &gray), [0; 4]);
    }

    #[test]
    fn dhash_gradual_gradient_sets_bits() {
        // Brightness strictly decreases left -> right: every pair differs
        // (gray[x] > gray[x+1] sets the bit).
        let mut gray = vec![0u8; FRAME_W * FRAME_H];
        for y in 0..FRAME_H {
            for x in 0..FRAME_W {
                gray[y * FRAME_W + x] = ((FRAME_W - 1 - x) * 10) as u8;
            }
        }
        assert_eq!(
            dhash_gray(FRAME_W, FRAME_H, &gray),
            [u64::MAX; 4]
        );
    }

    #[test]
    fn similarity_is_bit_fraction() {
        assert_eq!(hash_similarity(&[0; 4], &[0; 4]), 1.0);
        assert_eq!(
            hash_similarity(&[0; 4], &[1, 0, 0, 0]),
            255.0 / 256.0
        );
        assert_eq!(
            hash_similarity(&[0; 4], &[u64::MAX; 4]),
            0.0
        );
    }

    #[test]
    fn frame_seq_tolerates_small_shift() {
        // a[i] = i+1 appears exactly at b[i+1] (b is shifted one position).
        let mk = |v: u64| -> FrameHash { [v, 0, 0, 0] };
        let a: Vec<FrameHash> = (1..=8).map(mk).collect();
        let b: Vec<FrameHash> = (0..8).map(mk).collect();
        // 7 of 8 frames match exactly; the unpaired head matches its nearest
        // neighbor at 255/256 -> ~0.999.
        let sim = frame_seq_similarity(&a, &b, 2);
        assert!(sim > 0.99, "sim = {sim}");
        // Without a tolerance window the same shift drops and is strictly worse.
        let sim0 = frame_seq_similarity(&a, &b, 0);
        assert!(sim0 < sim && sim0 < 1.0);
    }

    #[test]
    fn frame_seq_sym_is_min_of_both_directions() {
        // Uniform B (all frames identical to one of A's) scores higher B→A
        // than A→B — the symmetric score must be the lower one so static
        // clips cannot claim 100% against varied clips.
        let mk = |v: u64| -> FrameHash { [v, 0, 0, 0] };
        let varied: Vec<FrameHash> = (0..8).map(mk).collect();
        let uniform: Vec<FrameHash> = vec![mk(0); 8];
        let fwd = frame_seq_similarity(&uniform, &varied, 2);
        let back = frame_seq_similarity(&varied, &uniform, 2);
        assert!(fwd > back, "fwd = {fwd}, back = {back}");
        assert_eq!(
            frame_seq_similarity_sym(&uniform, &varied, 2),
            fwd.min(back)
        );
    }

    fn all_idx(members: &[SimilarMember]) -> Vec<usize> {
        (0..members.len()).collect()
    }

    fn sim_member(path: &str, size: u64, w: u32, h: u32) -> SimilarMember {
        SimilarMember {
            entry: entry(Path::new(path), size),
            similarity: 1.0,
            w,
            h,
        }
    }

    #[test]
    fn keep_smaller_prefers_higher_resolution() {
        // A 4K PNG is bigger than a 720p JPG, but with --keep-smaller the
        // higher-resolution version must be the keeper.
        let members = vec![
            sim_member("/media/low.jpg", 200_000, 1280, 720),
            sim_member("/media/high.png", 8_000_000, 3840, 2160),
        ];
        assert_eq!(
            best_similar_keep(&members, &all_idx(&members), KeepMode::Smallest),
            1
        );
    }

    #[test]
    fn keep_smaller_ties_break_by_smallest_size() {
        // Same resolution: the smallest file wins (like before).
        let members = vec![
            sim_member("/media/c.png", 7_000_000, 1920, 1080),
            sim_member("/media/a.png", 5_000_000, 1920, 1080),
            sim_member("/media/b.png", 3_000_000, 1920, 1080),
        ];
        assert_eq!(
            best_similar_keep(&members, &all_idx(&members), KeepMode::Smallest),
            2
        );
    }

    #[test]
    fn keep_smaller_unknown_resolution_falls_back_to_smallest() {
        // Videos probed without ffprobe have 0x0: fall back to smallest.
        let members = vec![
            sim_member("/media/a.mp4", 10_000_000, 0, 0),
            sim_member("/media/b.mp4", 5_000_000, 0, 0),
            sim_member("/media/c.mp4", 8_000_000, 0, 0),
        ];
        assert_eq!(
            best_similar_keep(&members, &all_idx(&members), KeepMode::Smallest),
            1
        );
    }

    #[test]
    fn keep_smaller_known_resolution_beats_unknown() {
        // A tiny unknown-res file must lose to a bigger known-res one.
        let members = vec![
            sim_member("/media/tiny.jpg", 50_000, 0, 0),
            sim_member("/media/known.png", 900_000, 1920, 1080),
        ];
        assert_eq!(
            best_similar_keep(&members, &all_idx(&members), KeepMode::Smallest),
            1
        );
    }

    #[test]
    fn png_and_jpeg_of_same_image_are_similar() {
        use image::ImageBuffer;
        let dir = std::env::temp_dir().join(format!("dedupe-sim-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("photo.png");
        let jpg = dir.join("photo.jpg");

        // A synthetic image with plenty of structure.
        let img: ImageBuffer<image::Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(120, 90, |x, y| {
            let c = ((x * 2 + y * 3) % 256) as u8;
            image::Rgb([c, c.wrapping_mul(2), 255 - c])
        });
        img.save(&png).unwrap();
        // JPEG re-encode of the same pixels.
        img.save_with_format(&jpg, image::ImageFormat::Jpeg)
            .unwrap();

        let f1 = image_fingerprint(entry(&png, 0)).expect("png decodes");
        let f2 = image_fingerprint(entry(&jpg, 0)).expect("jpg decodes");
        let sim = hash_similarity(&f1.hash, &f2.hash);
        assert!(
            sim >= DEFAULT_SIMILARITY_PCT / 100.0,
            "png/jpg similarity {sim} < 0.97"
        );
        assert_eq!(aspect_key(f1.w, f1.h), aspect_key(f2.w, f2.h));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clustering_separates_different_images() {
        use image::ImageBuffer;
        let dir = std::env::temp_dir().join(format!("dedupe-sim-clust-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let make = |name: &str, f: fn(u32, u32) -> u8| {
            let path = dir.join(name);
            let img: ImageBuffer<image::Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(64, 64, |x, y| {
                let c = f(x, y);
                image::Rgb([c, c, c])
            });
            img.save(&path).unwrap();
            path
        };
        let a = make("a.png", |x, y| ((x + y) % 256) as u8);
        let b = make("b.png", |x, y| ((x + y) % 256) as u8); // identical fn => identical
        let c = make("c.png", |x, y| ((x * 7 + y * 13) % 256) as u8); // different

        let fps: Vec<ImageFp> = [&a, &b, &c]
            .iter()
            .map(|p| image_fingerprint(entry(p, 0)).expect("decodes"))
            .collect();
        let mut buckets: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, fp) in fps.iter().enumerate() {
            buckets.entry(aspect_key(fp.w, fp.h)).or_default().push(i);
        }
        let groups = cluster_images(&fps, buckets, DEFAULT_SIMILARITY_PCT / 100.0);
        assert_eq!(groups.len(), 1, "a and b group, c stays alone");
        assert_eq!(groups[0].members.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
