//! Video poster frames for the GUI.
//!
//! Videos cannot be decoded by the UI toolkit, so the GUI shows a poster
//! frame extracted with `ffmpeg` (already an optional dependency of the
//! app). Posters are cached in the OS temp dir keyed by path+size+mtime,
//! so repeat scans never re-extract.

use std::path::{Path, PathBuf};

/// Directory holding cached poster PNGs.
fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("dedupe-posters")
}

/// Stable cache key: the poster is valid while size+mtime are unchanged.
fn key_for(path: &Path, size: u64, mtime: Option<i64>) -> String {
    let mut h = blake3::Hasher::new();
    h.update(path.to_string_lossy().as_bytes());
    h.update(&size.to_le_bytes());
    if let Some(m) = mtime {
        h.update(&m.to_le_bytes());
    }
    h.finalize().to_hex().to_string()
}

/// Extract (or reuse) a poster PNG for `path`.
///
/// Returns `None` when ffmpeg is missing or extraction fails — callers show
/// a placeholder instead. Blocking (spawns ffmpeg); call off the UI thread.
pub fn video_poster(path: &Path, size: u64, mtime: Option<i64>) -> Option<PathBuf> {
    if !crate::media::ffmpeg_available() {
        return None;
    }
    let out = cache_dir().join(format!("{}.png", key_for(path, size, mtime)));
    if out.exists() {
        return Some(out);
    }
    if std::fs::create_dir_all(cache_dir()).is_err() {
        return None;
    }
    // Single-threaded decode: posters extract one frame each, and the GUI
    // already runs these back to back — ffmpeg's internal threading would
    // only oversubscribe the machine during big scans.
    let ok = crate::util::quiet_command("ffmpeg")
        .args(["-y", "-v", "error", "-threads", "1", "-i"])
        .arg(path)
        .args(["-vframes", "1", "-vf", "scale=320:-1"])
        .arg(&out)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        && out.exists();
    if ok { Some(out) } else { None }
}
