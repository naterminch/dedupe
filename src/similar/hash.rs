use super::{FrameHash, HASH_BITS};
use crate::cli::ImageHashAlgo;

/// 256-bit difference hash: for each of the 16 rows, compare the 16
/// horizontally-adjacent pixel pairs on a 17x16 grid.
///
/// Only the *relative* brightness order is kept, so the hash is robust to
/// re-encoding, format changes and slight exposure differences.
#[must_use]
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

/// 256-bit average hash: each bit says whether a 16x16 pixel is brighter
/// than the image mean. Cheap, but sensitive to uniform brightness shifts.
#[must_use]
pub fn ahash_gray(w: usize, h: usize, gray: &[u8]) -> FrameHash {
    let n = (w * h).min(256);
    if n == 0 || gray.len() < n {
        return [0; 4];
    }
    let mean = gray[..n].iter().map(|&v| u64::from(v)).sum::<u64>() / n as u64;
    let mut hash = [0u64; 4];
    for (i, &v) in gray[..n].iter().enumerate() {
        if u64::from(v) > mean {
            hash[i / 64] |= 1u64 << (i % 64);
        }
    }
    hash
}

/// 256-bit perceptual hash: the low-frequency 16x16 block of a 32x32 2D DCT,
/// thresholded at the median. Robust to re-encoding and mild filtering.
#[must_use]
pub fn phash_gray(w: usize, h: usize, gray: &[u8]) -> FrameHash {
    if w != 32 || h != 32 || gray.len() < 32 * 32 {
        return [0; 4];
    }
    let mut cos = [[0f64; 32]; 32];
    for (u, row) in cos.iter_mut().enumerate() {
        for (x, c) in row.iter_mut().enumerate() {
            *c = (((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI) / 64.0).cos();
        }
    }
    let mut rows = vec![0f64; 32 * 32];
    for y in 0..32 {
        for u in 0..32 {
            let mut sum = 0f64;
            for x in 0..32 {
                sum += f64::from(gray[y * 32 + x]) * cos[u][x];
            }
            rows[y * 32 + u] = sum;
        }
    }
    let mut coeffs = Vec::with_capacity(255);
    for (v, cos_v) in cos.iter().enumerate().take(16) {
        for u in 0..16 {
            if u == 0 && v == 0 {
                continue;
            }
            let mut sum = 0f64;
            for y in 0..32 {
                sum += rows[y * 32 + u] * cos_v[y];
            }
            coeffs.push(sum);
        }
    }
    let mut sorted = coeffs.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let mut hash = [0u64; 4];
    for (i, &c) in coeffs.iter().enumerate() {
        if c > median {
            hash[i / 64] |= 1u64 << (i % 64);
        }
    }
    hash
}

fn put_hash_bits(hash: &mut FrameHash, bit: &mut usize, value: u64, width: usize) {
    for k in (0..width).rev() {
        if (value >> k) & 1 == 1 {
            hash[*bit / 64] |= 1u64 << (*bit % 64);
        }
        *bit += 1;
    }
}

/// 256-bit colour hash: 4x4 cells, each RGB channel plus luma quantised to
/// 4 bits. Detects palette/layout matches that grayscale hashes miss.
#[must_use]
pub fn color_hash(rgb: &[u8], w: usize, h: usize) -> FrameHash {
    let cells = w * h;
    if cells == 0 || rgb.len() < cells * 3 {
        return [0; 4];
    }
    let mut hash = [0u64; 4];
    let mut bit = 0usize;
    for i in 0..cells {
        for c in 0..3 {
            let v = u64::from(rgb[i * 3 + c]);
            put_hash_bits(&mut hash, &mut bit, (v * 15 + 127) / 255, 4);
        }
    }
    for i in 0..cells {
        let p = i * 3;
        let luma = (2 * u64::from(rgb[p]) + 5 * u64::from(rgb[p + 1]) + u64::from(rgb[p + 2])) / 8;
        put_hash_bits(&mut hash, &mut bit, (luma * 15 + 127) / 255, 4);
    }
    hash
}

/// Cache tag for an image hash family; `auto` shares the dHash tag.
#[must_use]
pub fn image_algo_code(algo: ImageHashAlgo) -> u8 {
    match algo {
        ImageHashAlgo::Dhash | ImageHashAlgo::Auto => 0,
        ImageHashAlgo::Phash => 1,
        ImageHashAlgo::Ahash => 2,
        ImageHashAlgo::Color => 3,
    }
}

/// Fraction of identical bits between two 256-bit hashes (0..=1).
#[must_use]
pub fn hash_similarity(a: &FrameHash, b: &FrameHash) -> f64 {
    let mut diff = 0u32;
    for i in 0..4 {
        diff += (a[i] ^ b[i]).count_ones();
    }
    1.0 - f64::from(diff) / HASH_BITS
}

/// Mean over `a`'s frames of the best similarity found inside a ±`window`
/// neighborhood of `b` — tolerates small temporal drift between encodings.
///
/// NOTE: asymmetric — use [`frame_seq_similarity_sym`] for grouping so a
/// static clip (all frames identical) cannot score 1.0 against a varied
/// clip just because its single repeated hash appears in the other.
#[must_use]
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

/// Symmetric sequence similarity: min of both directions.
///
/// Prevents the false-positive where clip B's uniform frames are all contained in clip A
/// (B→A = 1.0) while A→B is lower — the pair must match both ways.
#[must_use]
pub fn frame_seq_similarity_sym(a: &[FrameHash], b: &[FrameHash], window: usize) -> f64 {
    frame_seq_similarity(a, b, window).min(frame_seq_similarity(b, a, window))
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------
