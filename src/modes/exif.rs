use super::walk::walk_tree;
use crate::cancel::CancelToken;
use crate::cli::Cli;
use std::fs;
use std::path::Path;

#[derive(Debug, Default)]
pub struct ExifReport {
    pub images_scanned: u64,
    pub images_with_metadata: u64,
    pub rewritten: u64,
    pub bytes_saved: u64,
    pub skipped: u64,
    pub applied: bool,
}

#[must_use]
pub fn strip_exif(cli: &Cli, cancel: &CancelToken) -> ExifReport {
    let tree = walk_tree(cli, cancel);
    let applied = cli.yes && !cli.dry_run;
    let mut report = ExifReport {
        applied,
        ..ExifReport::default()
    };
    for file in &tree.files {
        if cancel.is_cancelled() {
            break;
        }
        let ext = file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_png = match ext.as_str() {
            "jpg" | "jpeg" | "jfif" => false,
            "png" => true,
            _ => continue,
        };
        report.images_scanned += 1;
        let Ok(data) = fs::read(&file.path) else {
            report.skipped += 1;
            continue;
        };
        let stripped = if is_png {
            strip_png_metadata(&data)
        } else {
            strip_jpeg_metadata(&data)
        };
        let Some(stripped) = stripped else {
            continue;
        };
        report.images_with_metadata += 1;
        let saved = data.len().saturating_sub(stripped.len()) as u64;
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
