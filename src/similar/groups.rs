use super::hash::{hash_similarity, image_algo_code};
use super::images::{ImageFp, aspect_key, cluster_images, image_dimensions, image_fingerprint};
use super::video::{VideoFp, cluster_videos, durations_close, video_fingerprint};
use super::{FrameHash, SimilarConfig, SimilarGroup, SimilarMember};
use crate::cache::CacheFp;
use crate::cache::CacheWrite;
use crate::cache::FingerprintCache;
use crate::cancel::CancelToken;
use crate::hashing::HashEngine;
use crate::matching;
use crate::matching::Group;
use crate::matching::GroupMember;
use crate::matching::KeepMode;
use crate::media;
use crate::media::MediaInfo;
use crate::media::MediaKind;
use crate::scan::FileEntry;
use indicatif::ProgressBar;
use rayon::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

/// Find perceptual duplicates among `entries`.
///
/// `entries` should be the
/// files NOT already claimed by an exact-duplicate group, PLUS the
/// keepers of exact media groups (paths in `prioritize`) — without a
/// keeper in the pool a rescaled or re-encoded variant has nothing to
/// match against, even though its byte-identical siblings are right
/// there. `probe` says whether ffprobe is available (needed for the fast
/// video sampling path). `cache` supplies fingerprints from earlier runs
/// — a hit skips decoding / frame sampling entirely for that file (valid
/// only while its size and mtime are unchanged). When given, a `progress`
/// bar spans the fingerprinting work (length = number of media files).
/// Expensive work is pruned up front: images are bucketed by aspect ratio
/// with header-only reads (no decode) and videos by resolution + a cheap
/// duration check, so only files with a possible partner are fingerprinted
/// (a lone prioritized keeper is never decoded). Returns the groups plus
/// the newly computed fingerprints, which the caller should store back
/// into the cache. `cancel` is polled per file: once set, remaining
/// decodes / frame extractions are skipped and the partial result is
/// returned.
#[must_use]
pub fn find_similar(
    entries: Vec<FileEntry>,
    config: &SimilarConfig,
    probe: bool,
    cache: Option<&FingerprintCache>,
    progress: Option<&ProgressBar>,
    prioritize: &HashSet<PathBuf>,
    cancel: &CancelToken,
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
    // hash only images that share a bucket with a possible partner. Files
    // whose fingerprint is already cached skip the header read AND the decode.
    let image_algo = image_algo_code(config.image_hash);
    let mut aspect_buckets: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut cached_img: HashMap<usize, (u32, u32, FrameHash)> = HashMap::new();
    for (i, e) in images.iter().enumerate() {
        let cached = cache.and_then(|c| c.get(&e.path, e.size, e.mtime_secs));
        if let Some(CacheFp::Image { w, h, hash, algo }) = cached
            && *algo == image_algo
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
            if cancel.is_cancelled() {
                return None;
            }
            let fp = if needs_fp.contains(&i) {
                image_fingerprint(e.clone(), config.image_hash)
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
                algo: image_algo,
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
        groups.extend(cluster_images(
            &img_fps,
            buckets,
            config.threshold,
            prioritize,
        ));
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
        let duration_of = |i: usize| -> Option<u64> {
            cached_vid
                .get(&i)
                .map(|v| v.2)
                .or_else(|| probe_map.get(&videos[i].path).and_then(|m| m.duration_ms))
        };
        let mut cand: HashSet<usize> = HashSet::new();
        for idxs in res_buckets.values() {
            if idxs.len() < 2 {
                continue;
            }
            let dur: Vec<Option<u64>> = idxs.iter().map(|&i| duration_of(i)).collect();
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
        if config.partial_clips {
            for i in 0..videos.len() {
                match duration_of(i) {
                    Some(di) => {
                        if (0..videos.len()).any(|j| {
                            j != i
                                && duration_of(j)
                                    .is_some_and(|dj| dj * 100 < di * 90 || di * 100 < dj * 90)
                        }) {
                            cand.insert(i);
                        }
                    }
                    None => {
                        cand.insert(i);
                    }
                }
            }
        }
        cand.extend(unprobed);
        let new_fps: Vec<(usize, VideoFp)> = videos
            .par_iter()
            .enumerate()
            .filter_map(|(i, e)| {
                if cancel.is_cancelled() {
                    return None;
                }
                let fp = if cand.contains(&i) && !cached_vid.contains_key(&i) {
                    let info = probe_map.get(&e.path);
                    video_fingerprint(e.clone(), info, cancel)
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
            let exact_groups = cluster_videos(&vid_fps, buckets, config.threshold, prioritize);
            if config.partial_clips {
                let grouped: HashSet<PathBuf> = exact_groups
                    .iter()
                    .flat_map(|g| g.members.iter().map(|m| m.entry.path.clone()))
                    .collect();
                groups.extend(partial_clip_groups(
                    &vid_fps,
                    &grouped,
                    config.partial_similarity / 100.0,
                ));
            }
            groups.extend(exact_groups);
        }
    }

    (groups, writes)
}

fn partial_clip_groups(
    fps: &[VideoFp],
    already_grouped: &HashSet<PathBuf>,
    threshold: f64,
) -> Vec<SimilarGroup> {
    let mut idx: Vec<usize> = (0..fps.len())
        .filter(|&i| !already_grouped.contains(&fps[i].entry.path) && fps[i].frames.len() >= 2)
        .collect();
    idx.sort_by_key(|&i| std::cmp::Reverse(fps[i].duration_ms));
    let mut used = vec![false; idx.len()];
    let mut out = Vec::new();
    for a in 0..idx.len() {
        if used[a] {
            continue;
        }
        for b in (a + 1)..idx.len() {
            if used[b] {
                continue;
            }
            let long = &fps[idx[a]];
            let short = &fps[idx[b]];
            if short.frames.len() > long.frames.len() {
                continue;
            }
            if long.duration_ms > 0
                && short.duration_ms > 0
                && short.duration_ms * 100 >= long.duration_ms * 90
            {
                continue;
            }
            let sim = partial_clip_similarity(&short.frames, &long.frames);
            if sim >= threshold {
                used[a] = true;
                used[b] = true;
                out.push(SimilarGroup {
                    kind: MediaKind::Video,
                    members: vec![
                        SimilarMember {
                            entry: long.entry.clone(),
                            similarity: 1.0,
                            w: long.w,
                            h: long.h,
                        },
                        SimilarMember {
                            entry: short.entry.clone(),
                            similarity: sim,
                            w: short.w,
                            h: short.h,
                        },
                    ],
                });
                break;
            }
        }
    }
    out
}

pub(super) fn partial_clip_similarity(short: &[FrameHash], long: &[FrameHash]) -> f64 {
    if short.is_empty() || long.len() < short.len() {
        return 0.0;
    }
    let mut prev = vec![f64::NEG_INFINITY; long.len()];
    for (k, hs) in short.iter().enumerate() {
        let mut cur = vec![f64::NEG_INFINITY; long.len()];
        let mut best_prev = f64::NEG_INFINITY;
        for (j, hl) in long.iter().enumerate() {
            let sim = hash_similarity(hs, hl);
            if k == 0 {
                cur[j] = sim;
            } else {
                if j > 0 {
                    best_prev = best_prev.max(prev[j - 1]);
                }
                if best_prev.is_finite() {
                    cur[j] = best_prev + sim;
                }
            }
        }
        prev = cur;
    }
    let best = prev.into_iter().fold(f64::NEG_INFINITY, f64::max);
    if best.is_finite() {
        best / short.len() as f64
    } else {
        0.0
    }
}

/// Merge similar clusters into the exact groups they belong to.
///
/// A cluster whose leader is an exact-group keeper describes the same
/// picture as that byte-identical group. Everything the cluster touches
/// folds into the lowest-index touched group:
///
/// - **Other touched exact groups** are absorbed whole — their members
///   are byte-identical to their keeper, which the cluster matched, so
///   they are the same picture (the user's "2 same size + 1 smaller"
///   case, where the smaller variant is itself duplicated exactly).
///   Absorbed members lose their keep flag unless reference-protected,
///   which is never deletable and therefore always keep.
/// - **Leftovers** (rescaled / re-encoded variants) are appended with
///   their similarity score against the keeper, a media probe and a
///   deletion-safety content hash.
///
/// The target group's keeper stands (any later keep-mode change
/// re-elects keepers across the merged group). Emptied groups are
/// dropped and indices renumbered so they stay contiguous.
///
/// Returns the clusters that touch no exact group; the caller builds
/// those as fresh similar groups.
pub fn merge_into_exact_groups(
    groups: &mut Vec<Group>,
    similar: Vec<SimilarGroup>,
    keeper_group: &HashMap<PathBuf, usize>,
    reference_dirs: &[PathBuf],
    probe: bool,
    engine: &HashEngine,
) -> Vec<SimilarGroup> {
    let mut leftover = Vec::new();
    for sg in similar {
        // Exact groups whose keeper sits in this cluster.
        let touched: Vec<usize> = sg
            .members
            .iter()
            .filter_map(|m| keeper_group.get(&m.entry.path).copied())
            .collect();
        if touched.is_empty() {
            leftover.push(sg);
            continue;
        }
        let target = touched.iter().copied().min().unwrap_or(0);
        // Absorb the other touched exact groups whole.
        for &gi in &touched {
            if gi == target {
                continue;
            }
            // The cluster's score for this group's keeper; its members
            // are byte-identical to it, so they inherit the score.
            let score = groups[gi].members.iter().find(|m| m.keep).and_then(|k| {
                sg.members
                    .iter()
                    .find(|m| m.entry.path == k.path)
                    .map(|m| m.similarity)
            });
            for mut m in std::mem::take(&mut groups[gi].members) {
                // Reference members are never deletable, so they keep.
                m.keep = m.keep && m.reference;
                m.similarity = score;
                // Their own group's shared full hash (already computed by
                // the exact pass — free): the deletion safety re-hash must
                // not fall back to the target group's hash, or these files
                // could never be deleted.
                m.content_hash = Some(groups[gi].hash.clone());
                groups[target].members.push(m);
            }
        }
        // Append the true leftovers (keepers were handled above).
        for m in &sg.members {
            if keeper_group.contains_key(&m.entry.path) {
                continue;
            }
            let reference = !reference_dirs.is_empty()
                && matching::is_under_ref(
                    &matching::canonical_member(&m.entry.path),
                    reference_dirs,
                );
            groups[target].members.push(GroupMember {
                path: m.entry.path.clone(),
                size: m.entry.size,
                mtime_secs: m.entry.mtime_secs,
                keep: false,
                reference,
                media: if probe {
                    media::probe_media(&m.entry.path)
                } else {
                    None
                },
                similarity: Some(m.similarity),
                content_hash: engine.full(&m.entry.path).ok(),
                fingerprint_res: (m.w > 0 && m.h > 0).then_some((m.w, m.h)),
            });
        }
        // A group that absorbed anything is no longer purely exact: its
        // identity becomes the worst member match, not a hash that only
        // the byte-identical members share.
        let min_sim = groups[target]
            .members
            .iter()
            .filter_map(|m| m.similarity)
            .fold(f64::INFINITY, f64::min);
        if min_sim.is_finite() {
            groups[target].similarity = Some(min_sim);
        }
    }
    groups.retain(|g| !g.members.is_empty());
    for (i, g) in groups.iter_mut().enumerate() {
        g.index = i + 1;
    }
    leftover
}

/// Convert perceptual groups into report-ready `Group`s (with media metadata,
/// keep decisions, and a content hash per member used as a deletion safety
/// check).
///
/// `start_index` continues the numbering after the exact groups.
/// `cancel` is polled per group: cancelled groups are dropped from the
/// output (the caller discards a cancelled scan anyway).
#[expect(
    clippy::too_many_arguments,
    reason = "threads shared scan context into each group; a bag struct would allocate per call"
)]
#[must_use]
pub fn build_similar_groups(
    groups: Vec<SimilarGroup>,
    keep: KeepMode,
    reference_dirs: &[PathBuf],
    probe: bool,
    engine: &HashEngine,
    start_index: usize,
    progress: Option<&ProgressBar>,
    cancel: &CancelToken,
) -> Vec<Group> {
    if let Some(bar) = progress {
        bar.set_position(0);
        bar.set_length(groups.len() as u64);
        bar.set_message("building similar groups");
    }
    let built: Vec<Group> = groups
        .into_par_iter()
        .enumerate()
        .filter_map(|(i, sg)| {
            if cancel.is_cancelled() {
                return None;
            }
            Some(build_one_similar_group(
                i,
                &sg,
                keep,
                reference_dirs,
                probe,
                engine,
                start_index,
                progress,
                cancel,
            ))
        })
        .collect();
    built
}

/// Build one report-ready similar group (extracted so the parallel map can
/// skip cancelled work without touching the group-building logic).
#[expect(
    clippy::too_many_arguments,
    reason = "threads shared scan context into one group; called from a parallel map"
)]
fn build_one_similar_group(
    i: usize,
    sg: &SimilarGroup,
    keep: KeepMode,
    reference_dirs: &[PathBuf],
    probe: bool,
    engine: &HashEngine,
    start_index: usize,
    progress: Option<&ProgressBar>,
    cancel: &CancelToken,
) -> Group {
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
    // legitimately differ from each other. Per-member granularity: a
    // video member's full hash can take seconds; a Stop must land
    // between members.
    for m in &mut members {
        if cancel.is_cancelled() {
            break;
        }
        m.content_hash = engine.full(&m.path).ok();
    }

    // Keeper: reference-dir members are protected and win; among
    // the candidates the keep rule applies — for `Smallest` the
    // highest-resolution version (a smaller file is usually the
    // re-encoded/lower-res copy, so the best original is kept),
    // falling back to smallest size when unknown; for `BestQuality`
    // ties break toward the largest file instead.
    let pool: Vec<usize> = {
        let refs: Vec<usize> = sg
            .members
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                matching::is_under_ref(&matching::canonical_member(&m.entry.path), reference_dirs)
                    && !reference_dirs.is_empty()
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
}

/// Best keeper index among `pool` under `keep`.
///
/// `Smallest` prefers the highest resolution (width ? height); members with
/// unknown resolution (0?0, e.g. a video probed without ffprobe) rank below
/// any member with known resolution, falling back to smallest size.
/// `BestQuality` also leads with resolution, but ties break toward the
/// largest file (bitrate proxy) instead of the smallest.
pub(super) fn best_similar_keep(
    members: &[SimilarMember],
    pool: &[usize],
    keep: KeepMode,
) -> usize {
    let area = |m: &SimilarMember| -> Option<u64> {
        (m.w > 0 && m.h > 0).then_some(u64::from(m.w) * u64::from(m.h))
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
            KeepMode::BestQuality => match (area(m), area(b)) {
                (Some(a), Some(ba)) => {
                    (a, m.entry.size) > (ba, b.entry.size)
                        || ((a, m.entry.size) == (ba, b.entry.size) && m.entry.path < b.entry.path)
                }
                (Some(_), None) => true,
                (None, None) => {
                    m.entry.size > b.entry.size
                        || (m.entry.size == b.entry.size && m.entry.path < b.entry.path)
                }
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
