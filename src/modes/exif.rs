use super::Finding;
use super::findings::finding;
use super::walk::{Tree, walk_tree};
use crate::actions::is_protected;
use crate::cancel::CancelToken;
use crate::cli::Cli;
use crate::hashing::HashEngine;
use crate::util::human_bytes;
use std::fs;
use std::path::{Path, PathBuf};

/// What a metadata-stripping pass found and did.
#[derive(Debug, Default)]
pub struct ExifReport {
    /// JPEG and PNG files the pass looked at.
    pub images_scanned: u64,
    /// Of those, the ones carrying EXIF or XMP metadata.
    pub images_with_metadata: u64,
    /// Images actually rewritten (always 0 on a dry run).
    pub rewritten: u64,
    /// Bytes the rewrites removed, or would remove on a dry run.
    pub bytes_saved: u64,
    /// Images left alone: inside a reference dir, or reading or rewriting
    /// them failed.
    pub skipped: u64,
    /// Whether this run was allowed to write to disk.
    pub applied: bool,
}

/// What [`strip_metadata`] did, per file, so callers (e.g. the GUI) can
/// update their own state for exactly the photos that were rewritten.
#[derive(Debug, Default)]
pub struct StripOutcome {
    pub stripped: Vec<PathBuf>,
    /// Protected, unverified (no recorded hash), changed since the scan,
    /// already free of metadata, or failed.
    pub skipped: Vec<PathBuf>,
    /// Reads or rewrites that failed, with the reason.
    pub failures: Vec<(PathBuf, String)>,
    pub bytes_saved: u64,
}

/// Whether `path` names a format the stripper understands, judged by
/// extension like the rest of the scan.
fn is_photo(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "jfif" | "png"
        )
    })
}

/// The image without its EXIF/XMP metadata, or `None` when it carries none
/// (or is neither a JPEG nor a PNG). Both parsers check the signature, so a
/// mislabelled file is still handled by the format it actually is.
fn without_metadata(data: &[u8]) -> Option<Vec<u8>> {
    strip_jpeg_metadata(data).or_else(|| strip_png_metadata(data))
}

/// Strip EXIF and XMP metadata from the JPEG and PNG files under the scan
/// roots.
///
/// Writes only when `--yes` is passed and `--dry-run` is not, and rewrites
/// through a temporary file plus backup so an interrupted write cannot leave a
/// half-written image. Images inside a `--reference-dir` are never touched:
/// a reference folder is the user's safety net in every mode.
#[must_use]
pub fn strip_exif(cli: &Cli, cancel: &CancelToken) -> ExifReport {
    let tree = walk_tree(cli, cancel);
    let protected = crate::matching::canonicalize_refs(&cli.reference_dir);
    let applied = cli.yes && !cli.dry_run;
    let mut report = ExifReport {
        applied,
        ..ExifReport::default()
    };
    for file in &tree.files {
        if cancel.is_cancelled() {
            break;
        }
        if !is_photo(&file.path) {
            continue;
        }
        report.images_scanned += 1;
        if is_protected(&file.path, &protected) {
            report.skipped += 1;
            continue;
        }
        let Ok(data) = fs::read(&file.path) else {
            report.skipped += 1;
            continue;
        };
        let Some(stripped) = without_metadata(&data) else {
            continue;
        };
        report.images_with_metadata += 1;
        let saved = metadata_bytes(&data, &stripped);
        if !applied {
            report.bytes_saved += saved;
            continue;
        }
        match rewrite_in_place(&file.path, &stripped) {
            Ok(()) => {
                report.rewritten += 1;
                report.bytes_saved += saved;
            }
            Err(_) => report.skipped += 1,
        }
    }
    report
}

fn metadata_bytes(data: &[u8], stripped: &[u8]) -> u64 {
    data.len().saturating_sub(stripped.len()) as u64
}

/// One finding per photo that carries metadata, sized by what stripping it
/// would save, so the mode's reclaimable total is the space stripping gives
/// back. Unreadable photos are reported in `errors`.
pub(super) fn photo_metadata(
    tree: &Tree,
    errors: &mut Vec<String>,
    cancel: &CancelToken,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for file in &tree.files {
        if cancel.is_cancelled() {
            break;
        }
        if !is_photo(&file.path) {
            continue;
        }
        let data = match fs::read(&file.path) {
            Ok(data) => data,
            Err(e) => {
                errors.push(format!("{}: {e}", file.path.display()));
                continue;
            }
        };
        if let Some(stripped) = without_metadata(&data) {
            out.push(finding(
                file.path.clone(),
                metadata_bytes(&data, &stripped),
                "metadata",
                format!("EXIF/XMP metadata · photo {}", human_bytes(file.size)),
                false,
            ));
        }
    }
    out
}

/// Strip the metadata from every photo in `findings` (from
/// `--find photo-metadata`), non-interactively.
///
/// Safety mirrors deletion: a photo inside a protected folder is skipped, and
/// each file must still hold the bytes recorded in `finding.hash` — a finding
/// with no recorded hash is never rewritten unverified. The file is read once
/// and that same buffer is both verified and stripped, so nothing can change
/// between the check and the rewrite's input.
#[must_use]
pub fn strip_metadata(
    findings: &[Finding],
    protected: &[PathBuf],
    engine: &HashEngine,
) -> StripOutcome {
    let mut outcome = StripOutcome::default();
    for finding in findings {
        let path = &finding.path;
        if is_protected(path, protected) {
            outcome.skipped.push(path.clone());
            continue;
        }
        let Some(expected) = finding.hash.as_deref() else {
            outcome.skipped.push(path.clone());
            continue;
        };
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                outcome.skipped.push(path.clone());
                outcome.failures.push((
                    path.clone(),
                    format!("could not read {}: {e}", path.display()),
                ));
                continue;
            }
        };
        if engine.bytes(&data) != expected {
            outcome.skipped.push(path.clone());
            continue;
        }
        let Some(stripped) = without_metadata(&data) else {
            outcome.skipped.push(path.clone());
            continue;
        };
        match rewrite_in_place(path, &stripped) {
            Ok(()) => {
                outcome.bytes_saved += metadata_bytes(&data, &stripped);
                outcome.stripped.push(path.clone());
            }
            Err(e) => {
                outcome.skipped.push(path.clone());
                outcome.failures.push((
                    path.clone(),
                    format!("could not rewrite {}: {e}", path.display()),
                ));
            }
        }
    }
    outcome
}

pub(super) fn strip_jpeg_metadata(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..2]);
    let mut i = 2usize;
    let mut removed = false;
    while i < data.len() {
        if data[i] != 0xFF {
            out.extend_from_slice(&data[i..]);
            break;
        }
        if i + 1 >= data.len() {
            out.extend_from_slice(&data[i..]);
            break;
        }
        let marker = data[i + 1];
        if marker == 0xD9 || marker == 0xDA {
            out.extend_from_slice(&data[i..]);
            break;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            out.extend_from_slice(&data[i..i + 2]);
            i += 2;
            continue;
        }
        if marker == 0x00 {
            out.extend_from_slice(&data[i..]);
            break;
        }
        if i + 4 > data.len() {
            return None;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        let end = i + 2 + len;
        if end > data.len() {
            return None;
        }
        let payload = &data[i + 4..end];
        let metadata = marker == 0xE1
            && (payload.starts_with(b"Exif\0\0")
                || payload.starts_with(b"http://ns.adobe.com/xap/")
                || payload.starts_with(b"http://ns.adobe.com/xmp/"));
        if metadata {
            removed = true;
        } else {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    if removed { Some(out) } else { None }
}

pub(super) fn strip_png_metadata(data: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if !data.starts_with(&SIGNATURE) {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&SIGNATURE);
    let mut i = 8usize;
    let mut removed = false;
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let end = i + 12 + len;
        if end > data.len() {
            return None;
        }
        let kind = &data[i + 4..i + 8];
        let payload = &data[i + 8..i + 8 + len];
        let xmp = (kind == b"iTXt" || kind == b"tEXt") && payload.starts_with(b"XML:com.adobe.xmp");
        if kind == b"eXIf" || xmp {
            removed = true;
        } else {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    if i < data.len() {
        out.extend_from_slice(&data[i..]);
    }
    if removed { Some(out) } else { None }
}

fn rewrite_in_place(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let pid = std::process::id();
    let mut n = 0u32;
    let (temp, backup) = loop {
        let temp = parent.join(format!(".{name}.dedupe-tmp-{pid}-{n}"));
        let backup = parent.join(format!(".{name}.dedupe-bak-{pid}-{n}"));
        if !temp.exists() && !backup.exists() {
            break (temp, backup);
        }
        n += 1;
    };
    fs::write(&temp, data)?;
    if let Err(e) = fs::rename(path, &backup) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    match fs::rename(&temp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(e) => {
            let _ = fs::rename(&backup, path);
            let _ = fs::remove_file(&temp);
            Err(e)
        }
    }
}
