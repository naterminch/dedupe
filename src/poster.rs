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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_and_sensitive_to_every_input() {
        let p = Path::new("C:\\media\\clip.mp4");
        let base = key_for(p, 1024, Some(7));
        assert_eq!(key_for(p, 1024, Some(7)), base, "same inputs, same key");
        assert_ne!(key_for(p, 1025, Some(7)), base, "size matters");
        assert_ne!(key_for(p, 1024, Some(8)), base, "mtime matters");
        assert_ne!(key_for(p, 1024, None), base, "unknown mtime differs");
        assert_ne!(
            key_for(Path::new("C:\\media\\other.mp4"), 1024, Some(7)),
            base,
            "path matters"
        );
        assert_eq!(base.len(), 64, "blake3 hex digest");
    }

    #[test]
    fn cache_dir_lives_under_the_temp_dir() {
        let dir = cache_dir();
        assert_eq!(dir.parent().unwrap(), std::env::temp_dir());
        assert_eq!(
            dir.file_name().unwrap().to_string_lossy(),
            "dedupe-posters"
        );
    }

    #[test]
    fn missing_file_never_panics_and_returns_none() {
        // ffmpeg is either absent (None) or fails on a bogus path (None).
        let missing = std::env::temp_dir().join(format!(
            "dedupe-poster-missing-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(!missing.exists());
        assert_eq!(video_poster(&missing, 999, Some(1)), None);
    }
}
