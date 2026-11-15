use super::exif::{strip_jpeg_metadata, strip_png_metadata};
use super::extensions::bad_name_reason;
use super::findings::temp_reason;
use super::music::normalize_tag;
use super::names::{name_key, name_similarity};
use crate::cancel::CancelToken;
use crate::cli::{Cli, FindMode};
use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use clap::Parser;

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "dedupe-modes-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn cli_for(root: &Path, find: FindMode, extra: &[&str]) -> Cli {
    let mut args = vec![
        "dedupe".to_string(),
        "--find".to_string(),
        mode_name(find).to_string(),
    ];
    args.extend(extra.iter().map(|s| (*s).to_string()));
    args.push(root.to_string_lossy().into_owned());
    Cli::try_parse_from(args).expect("test CLI arguments")
}

#[test]
fn run_mode_finds_empty_big_and_temp_files() {
    let root = temp_root("basic");
    fs::create_dir_all(root.join("nested").join("empty")).unwrap();
    fs::write(root.join("zero.txt"), b"").unwrap();
    fs::write(root.join("big.bin"), vec![b'x'; 64]).unwrap();
    fs::write(root.join("old.tmp"), b"tmp").unwrap();
    let cancel = CancelToken::new();

    let empty_files = run_mode(&cli_for(&root, FindMode::EmptyFiles, &[]), &cancel);
    assert_eq!(empty_files.findings.len(), 1);
    assert_eq!(empty_files.findings[0].path, root.join("zero.txt"));
    assert_eq!(empty_files.findings[0].kind, "empty");

    let empty_dirs = run_mode(&cli_for(&root, FindMode::EmptyDirs, &[]), &cancel);
    assert_eq!(empty_dirs.findings.len(), 1);
    assert_eq!(
        empty_dirs.findings[0].path,
        root.join("nested").join("empty")
    );
    assert!(empty_dirs.findings[0].is_dir);

    let big = run_mode(
        &cli_for(&root, FindMode::BigFiles, &["--top", "1"]),
        &cancel,
    );
    assert_eq!(big.findings.len(), 1);
    assert_eq!(big.findings[0].path, root.join("big.bin"));
    assert_eq!(big.findings[0].size, 64);

    let temp = run_mode(&cli_for(&root, FindMode::TempFiles, &[]), &cancel);
    assert_eq!(temp.findings.len(), 1);
    assert_eq!(temp.findings[0].path, root.join("old.tmp"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn run_mode_flags_wrong_extension_and_broken_image() {
    let root = temp_root("media");
    let valid = image::ImageBuffer::from_pixel(2, 2, image::Rgb([255u8, 0, 0]));
    valid.save(root.join("real.png")).unwrap();
    fs::copy(root.join("real.png"), root.join("wrong.jpg")).unwrap();
    fs::write(root.join("broken.png"), b"not an image").unwrap();
    let cancel = CancelToken::new();

    let bad = run_mode(&cli_for(&root, FindMode::BadExtensions, &[]), &cancel);
    assert_eq!(bad.findings.len(), 1);
    assert!(bad.findings[0].path.ends_with("wrong.jpg"));
    assert!(bad.findings[0].detail.contains("png"));

    let broken = run_mode(&cli_for(&root, FindMode::BrokenFiles, &[]), &cancel);
    assert_eq!(broken.findings.len(), 1);
    assert!(broken.findings[0].path.ends_with("broken.png"));

    let _ = fs::remove_dir_all(&root);
}

fn id3_tag(title: &str, artist: &str, album: &str) -> Vec<u8> {
    fn frame(id: &[u8; 4], text: &str) -> Vec<u8> {
        let mut frame = Vec::new();
        frame.extend_from_slice(id);
        frame.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.push(0);
        frame.extend_from_slice(text.as_bytes());
        frame
    }
    let mut frames = frame(b"TIT2", title);
    frames.extend(frame(b"TPE1", artist));
    frames.extend(frame(b"TALB", album));
    let size = frames.len();
    let mut tag = vec![
        b'I',
        b'D',
        b'3',
        3,
        0,
        0,
        ((size >> 21) & 0x7f) as u8,
        ((size >> 14) & 0x7f) as u8,
        ((size >> 7) & 0x7f) as u8,
        (size & 0x7f) as u8,
    ];
    tag.extend(frames);
    tag
}

#[test]
fn run_mode_groups_tracks_with_matching_tags() {
    let root = temp_root("music");
    let tag = id3_tag("Song", "Artist", "Album");
    let mut first = tag.clone();
    first.push(b'a');
    let mut second = tag;
    second.extend_from_slice(b"bc");
    fs::write(root.join("first.mp3"), first).unwrap();
    fs::write(root.join("second.mp3"), second).unwrap();

    let result = run_mode(
        &cli_for(&root, FindMode::Music, &["--music-by", "tags"]),
        &CancelToken::new(),
    );
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].members.len(), 2);
    assert_eq!(result.groups[0].label, "Artist \u{2014} Song");
    assert!(result.reclaimable() > 0);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn strip_exif_only_rewrites_when_applied() {
    let root = temp_root("exif");
    let mut jpeg = vec![0xFF, 0xD8];
    let payload = b"Exif\0\0MM\0*";
    jpeg.extend_from_slice(&[0xFF, 0xE1]);
    jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    jpeg.extend_from_slice(payload);
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&4u32.to_be_bytes());
    png.extend_from_slice(b"eXIf");
    png.extend_from_slice(b"Exif");
    png.extend_from_slice(&[0, 0, 0, 0]);
    png.extend_from_slice(&0u32.to_be_bytes());
    png.extend_from_slice(b"IEND");
    png.extend_from_slice(&[0, 0, 0, 0]);
    let plain = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9];
    fs::write(root.join("photo.jpg"), &jpeg).unwrap();
    fs::write(root.join("image.png"), &png).unwrap();
    fs::write(root.join("plain.jpg"), plain).unwrap();

    let dry = strip_exif(
        &cli_for(&root, FindMode::Duplicates, &["--strip-exif"]),
        &CancelToken::new(),
    );
    assert!(!dry.applied);
    assert_eq!(dry.images_scanned, 3);
    assert_eq!(dry.images_with_metadata, 2);
    assert_eq!(dry.rewritten, 0);
    assert!(dry.bytes_saved > 0);
    assert_eq!(fs::read(root.join("photo.jpg")).unwrap(), jpeg);

    let applied = strip_exif(
        &cli_for(&root, FindMode::Duplicates, &["--strip-exif", "--yes"]),
        &CancelToken::new(),
    );
    assert!(applied.applied);
    assert_eq!(applied.images_scanned, 3);
    assert_eq!(applied.images_with_metadata, 2);
    assert_eq!(applied.rewritten, 2);
    let stripped_jpeg = fs::read(root.join("photo.jpg")).unwrap();
    let stripped_png = fs::read(root.join("image.png")).unwrap();
    assert!(!stripped_jpeg.windows(4).any(|w| w == b"Exif"));
    assert!(!stripped_png.windows(4).any(|w| w == b"eXIf"));
    assert_eq!(fs::read(root.join("plain.jpg")).unwrap(), plain);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn run_mode_groups_identical_folder_trees() {
    let root = temp_root("folders");
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    fs::write(root.join("a").join("photo.jpg"), b"same").unwrap();
    fs::write(root.join("b").join("photo.jpg"), b"same").unwrap();

    let result = run_mode(
        &cli_for(&root, FindMode::DuplicateFolders, &[]),
        &CancelToken::new(),
    );
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].members.len(), 2);
    assert_eq!(result.reclaimable(), 4);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn glob_matches_names_and_paths() {
    assert!(glob_match("*.log", Path::new("/tmp/app.log")));
    assert!(!glob_match("*.log", Path::new("/tmp/app.txt")));
    assert!(glob_match(
        "**/node_modules/**",
        Path::new("/tmp/node_modules/x.js")
    ));
    assert!(!glob_match("tmp/*.js", Path::new("/tmp/a/x.js")));
    assert!(glob_match("tmp/*.js", Path::new("/tmp/x.js")));
}

#[test]
fn name_keys_ignore_copy_suffixes() {
    assert_eq!(name_key(Path::new("Holiday.JPG")), "holiday");
    assert_eq!(name_key(Path::new("Holiday - Copy (2).jpg")), "holiday");
    assert_eq!(name_key(Path::new("my_file-1.png")), "my file 1");
}

#[test]
fn fuzzy_similarity_is_one_for_equal_names() {
    assert!((name_similarity("holiday photo", "holiday photo") - 1.0).abs() < 1e-9);
    assert!(name_similarity("holiday photo", "holiday pictures") < 0.8);
}

#[test]
fn temp_detection_covers_common_patterns() {
    assert!(temp_reason(Path::new("a.tmp")).is_some());
    assert!(temp_reason(Path::new("~$report.docx")).is_some());
    assert!(temp_reason(Path::new("backup.txt~")).is_some());
    assert!(temp_reason(Path::new("normal.txt")).is_none());
}

#[test]
fn bad_names_find_reserved_and_invalid() {
    assert!(bad_name_reason("CON.txt").is_some());
    assert!(bad_name_reason("bad:name.txt").is_some());
    assert!(bad_name_reason("trailing .").is_some());
    assert!(bad_name_reason("fine.txt").is_none());
}

#[test]
fn tag_normalization_is_case_and_punctuation_insensitive() {
    assert_eq!(normalize_tag("  Hello, World! "), "hello world");
    assert_eq!(normalize_tag("Track 01"), "track 01");
}

#[test]
fn jpeg_exif_app1_is_removed() {
    let mut data = vec![0xFF, 0xD8];
    let payload = b"Exif\0\0MM\0*";
    let len = (payload.len() + 2) as u16;
    data.extend_from_slice(&[0xFF, 0xE1]);
    data.extend_from_slice(&len.to_be_bytes());
    data.extend_from_slice(payload);
    data.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
    let out = strip_jpeg_metadata(&data).expect("metadata found");
    assert!(!out.windows(4).any(|w| w == b"Exif"));
    assert_eq!(&out[..2], &[0xFF, 0xD8]);
    assert_eq!(&out[out.len() - 2..], &[0xFF, 0xD9]);
}

#[test]
fn jpeg_without_metadata_is_untouched() {
    let data = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9];
    assert!(strip_jpeg_metadata(&data).is_none());
}

#[test]
fn png_exif_chunk_is_removed() {
    let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let payload = b"Exif";
    data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    data.extend_from_slice(b"eXIf");
    data.extend_from_slice(payload);
    data.extend_from_slice(&[0, 0, 0, 0]);
    data.extend_from_slice(&[0, 0, 0, 0]);
    data.extend_from_slice(b"IEND");
    data.extend_from_slice(&[0, 0, 0, 0]);
    let out = strip_png_metadata(&data).expect("metadata found");
    assert!(!out.windows(4).any(|w| w == b"eXIf"));
    assert!(out.windows(4).any(|w| w == b"IEND"));
}

/// A minimal JPEG whose only extra segment is an APP1 Exif block carrying
/// `payload` after the `Exif\0\0` header.
fn jpeg_with_exif(payload: &[u8]) -> Vec<u8> {
    let mut body = b"Exif\0\0".to_vec();
    body.extend_from_slice(payload);
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
    jpeg.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    jpeg.extend_from_slice(&body);
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
    jpeg
}

fn blake3() -> crate::hashing::HashEngine {
    crate::hashing::HashEngine::new(crate::cli::HashAlgo::Blake3)
}

#[test]
fn photo_metadata_mode_finds_a_jpeg_with_exif() {
    let root = temp_root("photo-metadata");
    let jpeg = jpeg_with_exif(b"MM\0*");
    fs::write(root.join("photo.jpg"), &jpeg).unwrap();
    fs::write(
        root.join("plain.jpg"),
        [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9],
    )
    .unwrap();
    fs::write(root.join("notes.txt"), b"Exif\0\0 not a photo").unwrap();

    let result = run_mode(
        &cli_for(&root, FindMode::PhotoMetadata, &[]),
        &CancelToken::new(),
    );

    assert_eq!(result.findings.len(), 1);
    let found = &result.findings[0];
    assert_eq!(found.path, root.join("photo.jpg"));
    assert_eq!(found.kind, "metadata");
    assert!(!found.is_dir);
    assert!(found.detail.contains("EXIF/XMP"));
    // Marker (2) + length (2) + "Exif\0\0" (6) + payload (4).
    assert_eq!(found.size, 14);
    assert_eq!(result.reclaimable(), 14);
    assert!(!mode_deletes(FindMode::PhotoMetadata));
    assert!(!mode_deletes(FindMode::DuplicateFolders));
    assert!(mode_deletes(FindMode::TempFiles));

    let _ = fs::remove_dir_all(&root);
}

/// A photo's hash pins the bytes the scan classified: a photo saved again
/// before the hashes are attached keeps the scan's hash, so a strip refuses
/// it instead of blessing the new contents.
#[test]
fn photo_metadata_findings_are_hashed_from_the_scanned_bytes() {
    let root = temp_root("photo-metadata-hash");
    let photo = root.join("photo.jpg");
    fs::write(&photo, jpeg_with_exif(b"MM\0*")).unwrap();
    let mut result = run_mode(
        &cli_for(&root, FindMode::PhotoMetadata, &[]),
        &CancelToken::new(),
    );
    let scanned = blake3().full(&photo).unwrap();
    assert_eq!(result.findings[0].hash, Some(scanned.clone()));

    fs::write(&photo, jpeg_with_exif(b"II*\0")).unwrap();
    attach_content_hashes(&mut result, &blake3(), &CancelToken::new());

    assert_eq!(
        result.findings[0].hash,
        Some(scanned),
        "the photo's later bytes are not hashed in"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn strip_metadata_strips_only_verified_unprotected_photos() {
    let root = temp_root("strip-metadata");
    let backup = root.join("backup");
    fs::create_dir_all(&backup).unwrap();
    let jpeg = jpeg_with_exif(b"MM\0*");
    let verified = root.join("verified.jpg");
    let changed = root.join("changed.jpg");
    let unhashed = root.join("unhashed.jpg");
    let guarded = backup.join("guarded.jpg");
    for path in [&verified, &changed, &unhashed, &guarded] {
        fs::write(path, &jpeg).unwrap();
    }
    let mut result = run_mode(
        &cli_for(&root, FindMode::PhotoMetadata, &[]),
        &CancelToken::new(),
    );
    attach_content_hashes(&mut result, &blake3(), &CancelToken::new());
    assert_eq!(result.findings.len(), 4);
    for finding in &mut result.findings {
        if finding.path == unhashed {
            finding.hash = None;
        }
    }
    let edited = jpeg_with_exif(b"II*\0");
    fs::write(&changed, &edited).unwrap();
    let protected = vec![fs::canonicalize(&backup).unwrap()];

    let outcome = strip_metadata(&result.findings, &protected, &blake3());

    assert_eq!(outcome.stripped, vec![verified.clone()]);
    assert_eq!(outcome.bytes_saved, 14);
    assert!(outcome.failures.is_empty());
    let mut skipped = outcome.skipped.clone();
    skipped.sort();
    let mut expected = vec![changed.clone(), unhashed.clone()];
    expected.sort();
    assert_eq!(skipped, expected);
    assert_eq!(outcome.protected, vec![guarded.clone()]);
    assert!(
        !fs::read(&verified)
            .unwrap()
            .windows(4)
            .any(|w| w == b"Exif")
    );
    assert_eq!(
        fs::read(&changed).unwrap(),
        edited,
        "a changed photo is left alone"
    );
    assert_eq!(
        fs::read(&unhashed).unwrap(),
        jpeg,
        "an unverified photo is left alone"
    );
    assert_eq!(
        fs::read(&guarded).unwrap(),
        jpeg,
        "a protected photo is left alone"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn strip_exif_leaves_reference_dir_photos_alone() {
    let root = temp_root("strip-exif-reference");
    let backup = root.join("backup");
    fs::create_dir_all(&backup).unwrap();
    let jpeg = jpeg_with_exif(b"MM\0*");
    fs::write(root.join("photo.jpg"), &jpeg).unwrap();
    fs::write(backup.join("guarded.jpg"), &jpeg).unwrap();
    let backup_arg = backup.to_string_lossy().into_owned();

    let report = strip_exif(
        &cli_for(
            &root,
            FindMode::Duplicates,
            &["--strip-exif", "--yes", "--reference-dir", &backup_arg],
        ),
        &CancelToken::new(),
    );

    assert_eq!(report.rewritten, 1);
    assert_eq!(report.skipped, 1);
    assert_eq!(fs::read(backup.join("guarded.jpg")).unwrap(), jpeg);
    assert!(
        !fs::read(root.join("photo.jpg"))
            .unwrap()
            .windows(4)
            .any(|w| w == b"Exif")
    );

    let _ = fs::remove_dir_all(&root);
}
