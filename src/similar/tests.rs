use super::groups::{best_similar_keep, partial_clip_similarity};
use super::hash::{
    ahash_gray, color_hash, dhash_gray, frame_seq_similarity, frame_seq_similarity_sym,
    hash_similarity, image_algo_code, phash_gray,
};
use super::images::{ImageFp, aspect_key, cluster_images, image_fingerprint};
use super::{DEFAULT_SIMILARITY_PCT, FRAME_H, FRAME_W, FrameHash, SimilarMember};
use crate::cli::ImageHashAlgo;
use crate::matching::KeepMode;
use crate::scan::FileEntry;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

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
    assert_eq!(dhash_gray(FRAME_W, FRAME_H, &gray), [u64::MAX; 4]);
}

#[test]
fn similarity_is_bit_fraction() {
    assert_eq!(hash_similarity(&[0; 4], &[0; 4]), 1.0);
    assert_eq!(hash_similarity(&[0; 4], &[1, 0, 0, 0]), 255.0 / 256.0);
    assert_eq!(hash_similarity(&[0; 4], &[u64::MAX; 4]), 0.0);
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
fn keep_best_quality_prefers_resolution_then_larger_size() {
    // Resolution leads, like Smallest — but ties break toward the
    // bigger file (bitrate proxy), the opposite of Smallest.
    let members = vec![
        sim_member("/media/low.jpg", 8_000_000, 1280, 720),
        sim_member("/media/high.png", 2_000_000, 3840, 2160),
    ];
    assert_eq!(
        best_similar_keep(&members, &all_idx(&members), KeepMode::BestQuality),
        1
    );
    let members = vec![
        sim_member("/media/a.jpg", 100_000, 1920, 1080),
        sim_member("/media/b.jpg", 500_000, 1920, 1080),
    ];
    assert_eq!(
        best_similar_keep(&members, &all_idx(&members), KeepMode::BestQuality),
        1
    );
    // Same resolution and size: first path wins.
    let members = vec![
        sim_member("/media/b.jpg", 500_000, 1920, 1080),
        sim_member("/media/a.jpg", 500_000, 1920, 1080),
    ];
    assert_eq!(
        best_similar_keep(&members, &all_idx(&members), KeepMode::BestQuality),
        1
    );
    // Unknown resolution loses to known, however big.
    let members = vec![
        sim_member("/media/huge.jpg", 9_000_000, 0, 0),
        sim_member("/media/known.jpg", 100_000, 640, 480),
    ];
    assert_eq!(
        best_similar_keep(&members, &all_idx(&members), KeepMode::BestQuality),
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

    let f1 = image_fingerprint(entry(&png, 0), ImageHashAlgo::Auto).expect("png decodes");
    let f2 = image_fingerprint(entry(&jpg, 0), ImageHashAlgo::Auto).expect("jpg decodes");
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
        .map(|p| image_fingerprint(entry(p, 0), ImageHashAlgo::Auto).expect("decodes"))
        .collect();
    let mut buckets: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, fp) in fps.iter().enumerate() {
        buckets.entry(aspect_key(fp.w, fp.h)).or_default().push(i);
    }
    let groups = cluster_images(
        &fps,
        buckets,
        DEFAULT_SIMILARITY_PCT / 100.0,
        &HashSet::new(),
    );
    assert_eq!(groups.len(), 1, "a and b group, c stays alone");
    assert_eq!(groups[0].members.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ahash_ignores_uniform_brightness_offset() {
    let mut a = vec![0u8; 16 * 16];
    let mut b = vec![0u8; 16 * 16];
    for y in 0..16 {
        for x in 0..16 {
            a[y * 16 + x] = ((x * 13 + y * 7) % 200) as u8;
        }
    }
    for i in 0..a.len() {
        b[i] = a[i].saturating_add(40);
    }
    let ha = ahash_gray(16, 16, &a);
    let hb = ahash_gray(16, 16, &b);
    assert!(hash_similarity(&ha, &hb) > 0.9);
}

#[test]
fn phash_distinguishes_different_patterns() {
    let mut a = vec![0u8; 32 * 32];
    let mut b = vec![0u8; 32 * 32];
    for y in 0..32 {
        for x in 0..32 {
            a[y * 32 + x] = if (x / 4 + y / 4) % 2 == 0 { 255 } else { 0 };
            b[y * 32 + x] = ((x * 8) % 256) as u8;
        }
    }
    let ha = phash_gray(32, 32, &a);
    let hb = phash_gray(32, 32, &b);
    assert!((hash_similarity(&ha, &ha) - 1.0).abs() < 1e-9);
    assert!(hash_similarity(&ha, &hb) < 0.95);
}

#[test]
fn color_hash_reflects_palette() {
    let red = [255u8, 0, 0].repeat(16);
    let blue = [0u8, 0, 255].repeat(16);
    let hr = color_hash(&red, 4, 4);
    let hb = color_hash(&blue, 4, 4);
    assert!((hash_similarity(&hr, &hr) - 1.0).abs() < 1e-9);
    assert!(hash_similarity(&hr, &hb) < 0.9);
    assert_ne!(hr, [0; 4]);
}

#[test]
fn partial_clip_finds_contained_sequence() {
    let long: Vec<FrameHash> = (0..8).map(|i| [i as u64, 0, 0, 0]).collect();
    let short = long[2..6].to_vec();
    assert!(partial_clip_similarity(&short, &long) > 0.99);
    let other: Vec<FrameHash> = (0..4).map(|i| [100 + i as u64, u64::MAX, 0, 0]).collect();
    assert!(partial_clip_similarity(&other, &long) < 0.9);
}

#[test]
fn image_algo_cache_tags_are_distinct() {
    assert_eq!(image_algo_code(ImageHashAlgo::Auto), 0);
    assert_eq!(image_algo_code(ImageHashAlgo::Dhash), 0);
    assert_eq!(image_algo_code(ImageHashAlgo::Phash), 1);
    assert_eq!(image_algo_code(ImageHashAlgo::Ahash), 2);
    assert_eq!(image_algo_code(ImageHashAlgo::Color), 3);
}
