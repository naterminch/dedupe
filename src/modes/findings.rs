use super::walk::Tree;
use super::{AUDIO_EXTS, Finding};
use crate::cancel::CancelToken;
use crate::media;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn finding(
    path: PathBuf,
    size: u64,
    kind: &'static str,
    detail: impl Into<String>,
    is_dir: bool,
) -> Finding {
    Finding {
        path,
        size,
        kind,
        detail: detail.into(),
        is_dir,
    }
}

pub(super) fn empty_dirs(tree: &Tree) -> Vec<Finding> {
    tree.dirs
        .iter()
        .filter(|d| !tree.roots.contains(*d))
        .filter(|d| fs::read_dir(d).is_ok_and(|mut it| it.next().is_none()))
        .map(|d| finding(d.clone(), 0, "empty-dir", "empty folder", true))
        .collect()
}

pub(super) fn invalid_symlinks(tree: &Tree) -> Vec<Finding> {
    tree.links
        .iter()
        .filter(|p| {
            fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
                && fs::metadata(p).is_err()
        })
        .map(|p| finding(p.clone(), 0, "symlink", "target does not exist", false))
        .collect()
}

pub(super) fn temp_reason(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "tmp"
            | "temp"
            | "bak"
            | "old"
            | "orig"
            | "part"
            | "partial"
            | "crdownload"
            | "download"
            | "cache"
            | "swp"
            | "swo"
            | "swn"
    ) {
        return Some("temporary or backup extension");
    }
    if name.starts_with("~$") || name.starts_with(".~lock.") {
        return Some("Office lock file");
    }
    if name.ends_with('~') {
        return Some("backup file (trailing tilde)");
    }
    if matches!(
        name.as_str(),
        ".ds_store" | "thumbs.db" | "ehthumbs.db" | "desktop.ini"
    ) {
        return Some("operating-system metadata file");
    }
    None
}

pub(super) fn broken_files(
    tree: &Tree,
    notes: &mut Vec<String>,
    cancel: &CancelToken,
) -> Vec<Finding> {
    let needs_ffprobe = tree
        .files
        .iter()
        .any(|f| media::classify(&f.path) == media::MediaKind::Video || is_audio(&f.path));
    let ffprobe = if needs_ffprobe {
        let ok = media::ffprobe_available();
        if !ok {
            notes.push(
                "ffprobe is not installed: corrupt audio/video files were not checked.".into(),
            );
        }
        ok
    } else {
        false
    };
    let mut out = Vec::new();
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
        if media::classify(&file.path) == media::MediaKind::Image {
            if matches!(ext.as_str(), "svg" | "heic" | "heif" | "avif") {
                continue;
            }
            let decoded = (|| -> Result<(u32, u32), String> {
                let reader = image::ImageReader::open(&file.path).map_err(|e| e.to_string())?;
                let reader = reader.with_guessed_format().map_err(|e| e.to_string())?;
                reader.into_dimensions().map_err(|e| e.to_string())
            })();
            if let Err(err) = decoded {
                out.push(finding(
                    file.path.clone(),
                    file.size,
                    "broken",
                    format!("image cannot be decoded: {err}"),
                    false,
                ));
            }
        } else if ffprobe
            && (media::classify(&file.path) == media::MediaKind::Video || is_audio(&file.path))
            && !ffprobe_readable(&file.path)
        {
            out.push(finding(
                file.path.clone(),
                file.size,
                "broken",
                "ffprobe cannot read this audio/video file",
                false,
            ));
        }
    }
    out
}

pub(super) fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn ffprobe_readable(path: &Path) -> bool {
    crate::util::quiet_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .is_ok_and(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<f64>()
                    .is_ok_and(|d| d >= 0.0)
        })
}
