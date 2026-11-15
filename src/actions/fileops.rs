use super::ActionError;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Destination names promised to earlier moves of one run.
///
/// Names are compared case-folded: Windows and macOS volumes treat `A.jpg`
/// and `a.jpg` as one file, so two planned moves that differ only by case
/// would land on the same path. Folding on every OS costs a Linux user at
/// most a " (2)" suffix it did not strictly need.
#[derive(Debug, Default)]
pub(super) struct Claimed(HashSet<String>);

impl Claimed {
    fn key(path: &Path) -> String {
        path.to_string_lossy().to_lowercase()
    }

    pub(super) fn insert(&mut self, path: &Path) {
        self.0.insert(Self::key(path));
    }

    fn contains(&self, path: &Path) -> bool {
        self.0.contains(&Self::key(path))
    }
}

/// A free path for `file_name` directly under `dest`: the plain join when
/// available, otherwise `stem (2).ext`, `stem (3).ext`, ... `claimed`
/// reserves names promised to earlier moves this run so two keepers with
/// the same file name never collide.
pub(super) fn free_dest_path(
    dest: &Path,
    file_name: &std::ffi::OsStr,
    claimed: &Claimed,
) -> PathBuf {
    let taken = |p: &Path| fs::symlink_metadata(p).is_ok() || claimed.contains(p);
    let first = dest.join(file_name);
    if !taken(&first) {
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
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Times [`move_file`] looks for another free name when the planned one was
/// taken in the meantime, before it gives up.
const FREE_NAME_TRIES: usize = 16;

/// Move one file to `to`, or to the next free name beside it when `to` was
/// taken since it was planned, and return where it landed.
///
/// The move never replaces an existing file: the planned name is only a
/// check, and another process (or a name that differs only by case) can
/// occupy it before the move runs. On Windows the rename itself refuses an
/// existing target; elsewhere the file is hard-linked into place, which
/// refuses one too, and the old name is then removed. Across volumes it is
/// copied into a newly created file and the original removed after.
pub(super) fn move_file(from: &Path, to: &Path) -> Result<PathBuf, ActionError> {
    let parent = to.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ActionError::Destination {
        dir: parent.to_path_buf(),
        file: to.to_path_buf(),
        source,
    })?;
    let file_name = from.file_name().unwrap_or(from.as_os_str());
    let mut to = to.to_path_buf();
    let mut last = None;
    for _ in 0..FREE_NAME_TRIES {
        match place(from, &to) {
            Ok(()) => return Ok(to),
            Err(Place::Taken(err)) => {
                last = Some(err);
                to = free_dest_path(parent, file_name, &Claimed::default());
            }
            Err(Place::Failed(err)) => return Err(err),
        }
    }
    Err(ActionError::Move {
        from: from.to_path_buf(),
        to,
        rename: last.unwrap_or(io::Error::from(io::ErrorKind::AlreadyExists)),
        copy: io::Error::from(io::ErrorKind::AlreadyExists),
    })
}

/// Why one placement attempt did not land.
enum Place {
    /// The destination exists; try another name.
    Taken(io::Error),
    Failed(ActionError),
}

/// Put `from` at `to` without ever replacing what is at `to`.
fn place(from: &Path, to: &Path) -> Result<(), Place> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(Place::Taken(io::Error::from(io::ErrorKind::AlreadyExists)));
    }
    let moved = rename_no_replace(from, to);
    match moved {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Err(Place::Taken(err)),
        // Another volume, or a filesystem without links: copy instead.
        Err(rename) => copy_no_replace(from, to, rename),
    }
}

/// An atomic rename that fails when `to` exists.
#[cfg(windows)]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    use windows::core::HSTRING;
    let (from, to) = (
        HSTRING::from(from.as_os_str()),
        HSTRING::from(to.as_os_str()),
    );
    // SAFETY: both arguments are valid NUL-terminated wide strings that live
    // for the whole call. Without MOVEFILE_REPLACE_EXISTING the call refuses
    // an existing destination.
    unsafe { MoveFileExW(&from, &to, MOVEFILE_WRITE_THROUGH) }.map_err(|e| {
        // HRESULT_FROM_WIN32: the Win32 code sits in the low 16 bits.
        io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
    })
}

/// A hard link into place (which fails when `to` exists), then the old name
/// removed; a link that cannot drop its old name is taken back.
#[cfg(not(windows))]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::hard_link(from, to)?;
    if let Err(err) = fs::remove_file(from) {
        let _ = fs::remove_file(to);
        return Err(err);
    }
    Ok(())
}

/// Copy `from` into a newly created `to` (never an existing one), keep its
/// modification time, then remove `from`.
fn copy_no_replace(from: &Path, to: &Path, rename: io::Error) -> Result<(), Place> {
    let failed = |copy: io::Error| {
        Place::Failed(ActionError::Move {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            rename: io::Error::new(rename.kind(), rename.to_string()),
            copy,
        })
    };
    let mut out = match fs::OpenOptions::new().write(true).create_new(true).open(to) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => return Err(Place::Taken(err)),
        Err(err) => return Err(failed(err)),
    };
    let copied = (|| {
        let mut input = fs::File::open(from)?;
        io::copy(&mut input, &mut out)?;
        if let Ok(modified) = input.metadata().and_then(|m| m.modified()) {
            let _ = out.set_modified(modified);
        }
        out.sync_all()
    })();
    drop(out);
    if let Err(err) = copied {
        let _ = fs::remove_file(to);
        return Err(failed(err));
    }
    // The copy landed, so both paths now hold the file; a failed cleanup of
    // the original is its own kind of failure.
    fs::remove_file(from).map_err(|source| {
        Place::Failed(ActionError::CopiedButKept {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source,
        })
    })
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
