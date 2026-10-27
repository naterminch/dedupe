use super::hash::{dhash_gray, frame_seq_similarity_sym};
use super::images::sort_bucket;
use super::{
    FRAME_H, FRAME_W, FrameHash, SEEK_THRESHOLD_MS, SIMILAR_FRAMES, SimilarGroup, SimilarMember,
};
use crate::cancel::CancelToken;
use crate::media::MediaInfo;
use crate::media::MediaKind;
use crate::scan::FileEntry;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(super) struct VideoFp {
    pub(super) entry: FileEntry,
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) duration_ms: u64,
    pub(super) frames: Vec<FrameHash>,
}

/// Sample `SIMILAR_FRAMES` evenly-spaced frames. With a known duration we use
/// one fast ffmpeg pass for short videos (a single full decode, `fps` filter
/// picks evenly time-spaced frames) or `-ss` keyframe seeks for long ones;
/// without a duration we fall back to a single `fps=1` pass (decodes the
/// whole stream, subsampled).
pub(super) fn video_fingerprint(
    entry: FileEntry,
    probe: Option<&MediaInfo>,
    cancel: &CancelToken,
) -> Option<VideoFp> {
    if let Some(info) = probe {
        let (w, h) = (info.width?, info.height?);
        let dur = info.duration_ms?;
        let frames = if dur <= SEEK_THRESHOLD_MS {
            extract_frames_pass(&entry.path, dur)
        } else {
            extract_frames_seek(&entry.path, dur, cancel)
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
pub(super) fn extract_frames_pass(path: &Path, duration_ms: u64) -> Option<Vec<FrameHash>> {
    let dur_secs = duration_ms as f64 / 1000.0;
    if dur_secs <= 0.0 {
        return None;
    }
    let fps = format!("{SIMILAR_FRAMES}/{dur_secs:.6}");
    // `-threads 1`: we already run one ffmpeg per worker thread, so internal
    // ffmpeg threading would oversubscribe the machine (N processes x M
    // threads). Single-threaded decode is plenty for 17x16 thumbnails.
    let out = crate::util::quiet_command("ffmpeg")
        .args(["-v", "error", "-threads", "1", "-i"])
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

pub(super) fn extract_frames_seek(
    path: &Path,
    duration_ms: u64,
    cancel: &CancelToken,
) -> Option<Vec<FrameHash>> {
    let secs = duration_ms as f64 / 1000.0;
    let mut hashes = Vec::with_capacity(SIMILAR_FRAMES);
    for i in 0..SIMILAR_FRAMES {
        // Per-frame granularity: eight sequential ffmpeg spawns on a long
        // video can take minutes; a Stop must land between spawns.
        if cancel.is_cancelled() {
            return None;
        }
        let t = secs * (i as f64 + 0.5) / SIMILAR_FRAMES as f64;
        let out = crate::util::quiet_command("ffmpeg")
            .args(["-v", "error", "-ss"])
            .arg(format!("{t:.6}"))
            .args(["-threads", "1", "-i"])
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

pub(super) fn extract_frames_oneshot(path: &Path) -> Option<Vec<FrameHash>> {
    let out = crate::util::quiet_command("ffmpeg")
        .args(["-v", "error", "-threads", "1", "-i"])
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
pub(super) fn durations_close(a_ms: u64, b_ms: u64) -> bool {
    let tol = 2_000u64.max((a_ms.max(b_ms) as f64 * 0.05) as u64);
    a_ms.abs_diff(b_ms) <= tol
}

pub(super) fn duration_close(a: &VideoFp, b: &VideoFp) -> bool {
    durations_close(a.duration_ms, b.duration_ms)
}

pub(super) fn cluster_videos(
    fps: &[VideoFp],
    buckets: HashMap<(u32, u32), Vec<usize>>,
    threshold: f64,
    prioritize: &HashSet<PathBuf>,
) -> Vec<SimilarGroup> {
    let mut groups = Vec::new();
    for mut idxs in buckets.into_values() {
        sort_bucket(idxs.as_mut_slice(), fps, prioritize, |fp| &fp.entry.path);
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
            if let Some(gi) = joined {
                cluster[gi].push(i);
            } else {
                leaders.push(i);
                cluster.push(vec![i]);
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

pub(super) fn make_video_group(fps: &[VideoFp], cluster: Vec<usize>) -> SimilarGroup {
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
