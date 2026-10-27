use super::hash::{ahash_gray, color_hash, dhash_gray, hash_similarity, phash_gray};
use super::{FRAME_H, FRAME_W, FrameHash, SimilarGroup, SimilarMember};
use crate::cli::ImageHashAlgo;
use crate::media::MediaKind;
use crate::scan::FileEntry;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(super) struct ImageFp {
    pub(super) entry: FileEntry,
    pub(super) hash: FrameHash,
    pub(super) w: u32,
    pub(super) h: u32,
}

/// Read only the image header (no pixel decode) — cheap enough to run for
/// every image so we can bucket by aspect ratio before decoding anything.
pub(super) fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
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

pub(super) fn image_fingerprint(entry: FileEntry, algo: ImageHashAlgo) -> Option<ImageFp> {
    let img = image::open(&entry.path).ok()?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    let hash = match algo {
        ImageHashAlgo::Dhash | ImageHashAlgo::Auto => {
            let gray = img.to_luma8();
            let d_small = image::imageops::resize(
                &gray,
                FRAME_W as u32,
                FRAME_H as u32,
                image::imageops::FilterType::Triangle,
            );
            dhash_gray(FRAME_W, FRAME_H, d_small.as_raw())
        }
        ImageHashAlgo::Ahash => {
            let gray = img.to_luma8();
            let small =
                image::imageops::resize(&gray, 16, 16, image::imageops::FilterType::Triangle);
            ahash_gray(16, 16, small.as_raw())
        }
        ImageHashAlgo::Phash => {
            let gray = img.to_luma8();
            let small =
                image::imageops::resize(&gray, 32, 32, image::imageops::FilterType::Triangle);
            phash_gray(32, 32, small.as_raw())
        }
        ImageHashAlgo::Color => {
            let rgb = img.to_rgb8();
            let small = image::imageops::resize(&rgb, 4, 4, image::imageops::FilterType::Triangle);
            color_hash(small.as_raw(), 4, 4)
        }
    };
    Some(ImageFp { entry, hash, w, h })
}

/// Bucket key: aspect ratio rounded to 1/100, so re-scaled versions of the
/// same photo (same ratio) are compared against each other.
pub(super) fn aspect_key(w: u32, h: u32) -> i64 {
    if h == 0 {
        return 0;
    }
    ((f64::from(w) / f64::from(h)) * 100.0).round() as i64
}

/// Order one bucket for star clustering: prioritized entries (exact-group
/// keepers) lead, then path order for determinism. A prioritized entry
/// therefore becomes its cluster's leader whenever one is present, which
/// keeps every member's similarity score measured against that keeper —
/// so a merged-into-exact group shows "X% similar to the keeper".
pub(super) fn sort_bucket<T>(
    idxs: &mut [usize],
    fps: &[T],
    prioritize: &HashSet<PathBuf>,
    path_of: impl Fn(&T) -> &PathBuf,
) {
    idxs.sort_by(|&a, &b| {
        let pa = !prioritize.contains(path_of(&fps[a]));
        let pb = !prioritize.contains(path_of(&fps[b]));
        pa.cmp(&pb)
            .then_with(|| path_of(&fps[a]).cmp(path_of(&fps[b])))
    });
}

pub(super) fn cluster_images(
    fps: &[ImageFp],
    buckets: HashMap<i64, Vec<usize>>,
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
                if hash_similarity(&fps[i].hash, &fps[leader].hash) >= threshold {
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
                groups.push(make_image_group(fps, c));
            }
        }
    }
    groups
}

pub(super) fn make_image_group(fps: &[ImageFp], cluster: Vec<usize>) -> SimilarGroup {
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
