use super::Finding;
use super::findings::finding;
use super::walk::{Tree, walk_tree};
use crate::actions::is_protected;
use crate::cancel::CancelToken;
use crate::cli::Cli;
use crate::hashing::HashEngine;
use crate::history::{Item, Went};
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
    /// Every photo rewritten, as the clean-up history records it.
    pub items: Vec<Item>,
}

/// What [`strip_metadata`] did, per file, so callers (e.g. the GUI) can
/// update their own state for exactly the photos that were rewritten.
#[derive(Debug, Default)]
pub struct StripOutcome {
    pub stripped: Vec<PathBuf>,
    /// Unverified (no recorded hash), changed since the scan, already free
    /// of metadata, or failed.
    pub skipped: Vec<PathBuf>,
    /// Inside a protected folder, so never touched.
    pub protected: Vec<PathBuf>,
    /// Reads or rewrites that failed, with the reason.
    pub failures: Vec<(PathBuf, String)>,
    pub bytes_saved: u64,
    /// Every photo rewritten, as the clean-up history records it.
    pub items: Vec<Item>,
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

/// What a photo says about itself: its size in pixels, and the camera and the
/// moment its EXIF block records, for a side-by-side look at two copies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhotoInfo {
    pub width: u32,
    pub height: u32,
    /// The camera's make and model, as one name ("Canon EOS R6").
    pub camera: Option<String>,
    /// When the picture was taken, as the camera wrote it: `YYYY:MM:DD
    /// HH:MM:SS` in the camera's own local time.
    pub taken: Option<String>,
}

/// Read a photo's header and EXIF block without decoding its pixels. `None`
/// when the file is not an image the decoder knows; a photo without EXIF
/// still reports its size.
#[must_use]
pub fn photo_info(path: &Path) -> Option<PhotoInfo> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let (width, height) = decoder.dimensions();
    let (camera, taken) = decoder
        .exif_metadata()
        .ok()
        .flatten()
        .map(|exif| camera_and_taken(&exif))
        .unwrap_or_default();
    Some(PhotoInfo {
        width,
        height,
        camera,
        taken,
    })
}

/// IFD0's make, model and the pointer to the EXIF sub-IFD, and the EXIF
/// sub-IFD's original date.
const TAG_MAKE: u16 = 0x010F;
const TAG_MODEL: u16 = 0x0110;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_DATE_TAKEN: u16 = 0x9003;

/// The camera name and the date taken from a raw EXIF block: a TIFF
/// structure, optionally behind JPEG's `Exif\0\0` marker. Anything malformed
/// reads as absent, never as an error: these are details, not decisions.
pub(crate) fn camera_and_taken(exif: &[u8]) -> (Option<String>, Option<String>) {
    let tiff = exif.strip_prefix(b"Exif\0\0").unwrap_or(exif);
    let Some(reader) = Tiff::new(tiff) else {
        return (None, None);
    };
    let Some(ifd0) = reader.u32_at(4) else {
        return (None, None);
    };
    let make = reader.ascii(ifd0, TAG_MAKE);
    let model = reader.ascii(ifd0, TAG_MODEL);
    let camera = match (make, model) {
        // Most models already start with the maker's name ("Canon EOS R6").
        (Some(make), Some(model))
            if !model
                .to_ascii_lowercase()
                .starts_with(&make.to_ascii_lowercase()) =>
        {
            Some(format!("{make} {model}"))
        }
        (_, Some(model)) => Some(model),
        (make, None) => make,
    };
    let taken = reader
        .long(ifd0, TAG_EXIF_IFD)
        .and_then(|sub| reader.ascii(sub, TAG_DATE_TAKEN));
    (camera, taken)
}

/// A TIFF structure in either byte order.
struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

impl<'a> Tiff<'a> {
    fn new(data: &'a [u8]) -> Option<Self> {
        let little = match data.get(..4)? {
            [b'I', b'I', 42, 0] => true,
            [b'M', b'M', 0, 42] => false,
            _ => return None,
        };
        Some(Self { data, little })
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        let bytes: [u8; 2] = self.data.get(at..at + 2)?.try_into().ok()?;
        Some(if self.little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    }

    fn u32_at(&self, at: usize) -> Option<usize> {
        let bytes: [u8; 4] = self.data.get(at..at + 4)?.try_into().ok()?;
        let value = if self.little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        };
        usize::try_from(value).ok()
    }

    /// The 12-byte entry for `tag` in the IFD at `ifd`.
    fn entry(&self, ifd: usize, tag: u16) -> Option<usize> {
        let count = usize::from(self.u16_at(ifd)?);
        (0..count)
            .map(|i| ifd + 2 + i * 12)
            .find(|&at| self.u16_at(at) == Some(tag))
    }

    /// A LONG (type 4) value: an offset to another IFD.
    fn long(&self, ifd: usize, tag: u16) -> Option<usize> {
        let at = self.entry(ifd, tag)?;
        if self.u16_at(at + 2)? != 4 {
            return None;
        }
        self.u32_at(at + 8)
    }

    /// An ASCII (type 2) value, trimmed of its NULs and padding; `None`
    /// when it is empty.
    fn ascii(&self, ifd: usize, tag: u16) -> Option<String> {
        let at = self.entry(ifd, tag)?;
        if self.u16_at(at + 2)? != 2 {
            return None;
        }
        let len = self.u32_at(at + 4)?;
        // Up to four bytes sit in the entry itself; longer ones elsewhere.
        let start = if len <= 4 {
            at + 8
        } else {
            self.u32_at(at + 8)?
        };
        let bytes = self.data.get(start..start.checked_add(len)?)?;
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        (!text.is_empty()).then(|| text.to_string())
    }
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
    let engine = HashEngine::new(cli.hash);
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
        // The photo must still be the one just read when it is swapped out.
        let read = engine.bytes(&data);
        match rewrite_in_place(&file.path, &stripped, Some((&engine, &read))) {
            Ok(Rewrite::Done) => {
                report.rewritten += 1;
                report.bytes_saved += saved;
                report.items.push(Item::new(
                    file.path.clone(),
                    saved,
                    Went::Rewritten,
                    Some(read),
                ));
            }
            Ok(Rewrite::Changed) | Err(_) => report.skipped += 1,
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
///
/// Each finding's hash is taken from the very bytes classified here, so a
/// later strip proves the file is the one the scan saw, not whatever it held
/// when the hashes were attached.
pub(super) fn photo_metadata(
    tree: &Tree,
    engine: &HashEngine,
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
            let mut found = finding(
                file.path.clone(),
                metadata_bytes(&data, &stripped),
                "metadata",
                format!("EXIF/XMP metadata · photo {}", human_bytes(file.size)),
                false,
            );
            found.hash = Some(engine.bytes(&data));
            out.push(found);
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
            outcome.protected.push(path.clone());
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
        match rewrite_in_place(path, &stripped, Some((engine, expected))) {
            Ok(Rewrite::Done) => {
                let saved = metadata_bytes(&data, &stripped);
                outcome.bytes_saved += saved;
                outcome.stripped.push(path.clone());
                outcome.items.push(Item::new(
                    path.clone(),
                    saved,
                    Went::Rewritten,
                    Some(expected.to_string()),
                ));
            }
            // Saved by someone else after the check: their version stays.
            Ok(Rewrite::Changed) => outcome.skipped.push(path.clone()),
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

/// Whether a rewrite replaced the photo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rewrite {
    Done,
    /// The photo no longer held the bytes the rewrite was made from, so it
    /// was left exactly as it was.
    Changed,
}

/// Replace `path` with `data` through a temporary file beside it, keeping the
/// original as a backup until the new bytes are in place.
///
/// The temporary file is created new (never opened through an existing name,
/// so a symlink planted at a predictable name is refused rather than
/// followed). With `expect`, the photo is hashed once more just before it is
/// swapped out, and a photo saved by someone else since `data` was derived is
/// left alone.
fn rewrite_in_place(
    path: &Path,
    data: &[u8],
    expect: Option<(&HashEngine, &str)>,
) -> std::io::Result<Rewrite> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let pid = std::process::id();
    let mut n = 0u32;
    let (temp, mut file) = loop {
        let temp = parent.join(format!(".{name}.dedupe-tmp-{pid}-{n}"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    };
    if let Err(e) = file.write_all(data).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    drop(file);
    if let Some((engine, expected)) = expect
        && engine.full(path).ok().as_deref() != Some(expected)
    {
        let _ = fs::remove_file(&temp);
        return Ok(Rewrite::Changed);
    }
    let backup = loop {
        let backup = parent.join(format!(".{name}.dedupe-bak-{pid}-{n}"));
        if fs::symlink_metadata(&backup).is_err() {
            break backup;
        }
        n += 1;
    };
    if let Err(e) = fs::rename(path, &backup) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    match fs::rename(&temp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(Rewrite::Done)
        }
        Err(e) => {
            let _ = fs::rename(&backup, path);
            let _ = fs::remove_file(&temp);
            Err(e)
        }
    }
}
