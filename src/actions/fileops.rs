use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A free path for `file_name` directly under `dest`: the plain join when
/// available, otherwise `stem (2).ext`, `stem (3).ext`, ... `claimed`
/// reserves names promised to earlier moves this run so two keepers with
/// the same file name never collide.
pub(super) fn free_dest_path(
    dest: &Path,
    file_name: &std::ffi::OsStr,
    claimed: &HashSet<PathBuf>,
) -> PathBuf {
    let first = dest.join(file_name);
    if !first.exists() && !claimed.contains(&first) {
        return first;
    }
    let name = Path::new(file_name);
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 2u32;
    loop {
        let candidate = dest.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() && !claimed.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Move one file, falling back to copy + remove when the atomic rename
/// fails (cross-device moves). The destination parent is created.
pub(super) fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(rename_err) => fs::copy(from, to)
            .and_then(|_| fs::remove_file(from))
            .map_err(|copy_err| {
                format!("rename failed ({rename_err}); copy fallback failed ({copy_err})")
            }),
    }
}

pub(super) fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut n = 0u32;
    loop {
        let candidate = parent.join(format!(".{name}.dedupe-link-{}-{n}", std::process::id()));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}
