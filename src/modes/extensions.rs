use super::Finding;
use super::findings::finding;
use super::walk::Tree;
use std::fs::File;
use std::io::Read;
use std::path::Path;

fn sniff_extension(path: &Path) -> Option<&'static str> {
    let mut file = File::open(path).ok()?;
    let mut head = [0u8; 16];
    let n = file.read(&mut head).ok()?;
    let head = &head[..n];
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpg");
    }
    if head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("png");
    }
    if head.starts_with(b"GIF8") {
        return Some("gif");
    }
    if head.starts_with(b"BM") {
        return Some("bmp");
    }
    if head.len() >= 12 && &head[0..4] == b"RIFF" {
        match &head[8..12] {
            b"WEBP" => return Some("webp"),
            b"WAVE" => return Some("wav"),
            b"AVI " => return Some("avi"),
            _ => {}
        }
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return Some("tiff");
    }
    if head.starts_with(b"%PDF") {
        return Some("pdf");
    }
    if head.starts_with(&[0x1F, 0x8B]) {
        return Some("gzip");
    }
    if head.starts_with(b"Rar!") {
        return Some("rar");
    }
    if head.starts_with(&[0x37, 0x7A, 0xBC, 0xAF]) {
        return Some("7z");
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("mkv");
    }
    if head.starts_with(b"fLaC") {
        return Some("flac");
    }
    if head.starts_with(b"OggS") {
        return Some("ogg");
    }
    if head.starts_with(b"ID3") {
        return Some("mp3");
    }
    if head.len() >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0 {
        return Some("mp3");
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Some("mp4");
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Some("zip");
    }
    None
}

fn extension_matches(actual: &str, ext: &str) -> bool {
    match actual {
        "jpg" => matches!(ext, "jpg" | "jpeg" | "jfif"),
        "png" => ext == "png",
        "gif" => ext == "gif",
        "bmp" => ext == "bmp",
        "webp" => ext == "webp",
        "tiff" => matches!(ext, "tif" | "tiff"),
        "pdf" => ext == "pdf",
        "gzip" => matches!(ext, "gz" | "tgz"),
        "rar" => ext == "rar",
        "7z" => ext == "7z",
        "mkv" => matches!(ext, "mkv" | "webm"),
        "flac" => ext == "flac",
        "ogg" => matches!(ext, "ogg" | "oga" | "opus"),
        "mp3" => ext == "mp3",
        "mp4" => matches!(ext, "mp4" | "m4v" | "m4a" | "mov" | "3gp"),
        "wav" => ext == "wav",
        "avi" => ext == "avi",
        "zip" => matches!(
            ext,
            "zip" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub" | "jar" | "apk"
        ),
        _ => true,
    }
}

pub(super) fn bad_extensions(tree: &Tree) -> Vec<Finding> {
    let mut out = Vec::new();
    for file in &tree.files {
        let Some(ext) = file.path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        let Some(actual) = sniff_extension(&file.path) else {
            continue;
        };
        if !extension_matches(actual, &ext) {
            let label = if actual == "zip" {
                "ZIP/office"
            } else {
                actual
            };
            out.push(finding(
                file.path.clone(),
                file.size,
                "bad-extension",
                format!("contents look like {label}, extension is .{ext}"),
                false,
            ));
        }
    }
    out
}

pub(super) fn bad_name_reason(name: &str) -> Option<String> {
    let mut issues = Vec::new();
    if name.trim() != name {
        issues.push("leading or trailing whitespace");
    }
    if name.ends_with('.') {
        issues.push("ends with a dot");
    }
    if name.ends_with(' ') {
        issues.push("ends with a space");
    }
    if name.chars().any(char::is_control) {
        issues.push("contains control characters");
    }
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        issues.push("contains Windows-invalid characters");
    }
    if name.len() > 255 {
        issues.push("name is longer than 255 bytes");
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        issues.push("Windows-reserved device name");
    }
    if issues.is_empty() {
        None
    } else {
        Some(issues.join("; "))
    }
}

pub(super) fn bad_names(tree: &Tree) -> Vec<Finding> {
    let mut out = Vec::new();
    for file in &tree.files {
        let name = file.path.file_name().unwrap_or_default().to_string_lossy();
        if let Some(reason) = bad_name_reason(&name) {
            out.push(finding(
                file.path.clone(),
                file.size,
                "bad-name",
                reason,
                false,
            ));
        }
    }
    for dir in &tree.dirs {
        if tree.roots.contains(dir) {
            continue;
        }
        let name = dir.file_name().unwrap_or_default().to_string_lossy();
        if let Some(reason) = bad_name_reason(&name) {
            out.push(finding(dir.clone(), 0, "bad-name", reason, true));
        }
    }
    out
}
