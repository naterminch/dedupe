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
