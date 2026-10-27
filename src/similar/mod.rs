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

mod groups;
mod hash;
mod images;
mod video;

use crate::cli::ImageHashAlgo;
use crate::media::MediaKind;
use crate::scan::FileEntry;

pub use groups::{build_similar_groups, find_similar, merge_into_exact_groups};

/// Default similarity threshold, as a percentage (0-100), surfaced as the
/// `--similarity` default. >= 97% => duplicates.
pub const DEFAULT_SIMILARITY_PCT: f64 = 97.0;

/// Number of frames sampled from each video.
const SIMILAR_FRAMES: usize = 8;

/// Fingerprint grid: `(FRAME_W - 1) * FRAME_H` bits = 16 * 16 = 256 bits
/// (4 x u64). The old 9x8 / 64-bit grid collided on videos sharing only a
/// coarse layout (e.g. dark sides + bright center skin tones at 9px wide),
/// grouping completely different clips at 98-100% similar.
const FRAME_W: usize = 17;

const FRAME_H: usize = 16;

/// Bits per frame fingerprint.
const HASH_BITS: f64 = 256.0;

/// Videos longer than this use per-frame `-ss` seeks (decoding the whole
/// stream to sample 8 frames would be wasteful); shorter ones are decoded
/// once in a single fast pass. Measured: one pass is ~7-10x faster than 8
/// separate ffmpeg spawns for clips up to a minute or so.
const SEEK_THRESHOLD_MS: u64 = 90_000;

/// One frame fingerprint: 256-bit difference hash.
pub type FrameHash = [u64; 4];

#[derive(Debug)]
pub struct SimilarConfig {
    pub threshold: f64,
    pub image_hash: ImageHashAlgo,
    pub partial_clips: bool,
    pub partial_similarity: f64,
}

/// One file in a similar group, with its similarity (0..=1) against the
/// group's keeper (1.0 for the keeper itself), and its resolution (0 when
/// unknown, e.g. a video probed without ffprobe).
#[derive(Debug)]
pub struct SimilarMember {
    pub entry: FileEntry,
    pub similarity: f64,
    pub w: u32,
    pub h: u32,
}

/// A set of files judged to be the same content (keeper first).
#[derive(Debug)]
pub struct SimilarGroup {
    pub kind: MediaKind,
    pub members: Vec<SimilarMember>,
}

#[cfg(test)]
mod tests;
