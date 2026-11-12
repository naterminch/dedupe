//! Typed failures for the delete and move paths.
//!
//! These errors cross into the GUI, which tells them apart by variant: a trash
//! call that the desktop refused reads differently from a destination folder
//! that could not be created, even though both used to arrive as one formatted
//! string. `anyhow` stays in `main.rs`, where every error is printed and the
//! type never matters.

use std::io;
use std::path::PathBuf;

/// What went wrong while removing or moving files.
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    /// The destination directory for a move could not be created.
    #[error("could not create {} for {}: {source}", dir.display(), file.display())]
    Destination {
        dir: PathBuf,
        file: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A candidate could not be read, so it could not be verified.
    #[error("could not read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Permanent removal failed.
    #[error("could not remove {}: {source}", path.display())]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The system trash refused the file.
    #[error("could not send {} to the trash: {source}", path.display())]
    Trash {
        path: PathBuf,
        #[source]
        source: trash::Error,
    },

    /// The file was copied into place but the original could not be removed,
    /// so both paths now hold a copy.
    #[error(
        "copied {} to {} but could not remove the original: {source}",
        from.display(),
        to.display()
    )]
    CopiedButKept {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Neither the atomic rename nor the copy fallback worked.
    #[error(
        "could not move {} to {}: rename failed ({rename}); copy fallback failed ({copy})",
        from.display(),
        to.display()
    )]
    Move {
        from: PathBuf,
        to: PathBuf,
        #[source]
        rename: io::Error,
        copy: io::Error,
    },
}

impl ActionError {
    /// The file the failure is about.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        match self {
            Self::Destination { file, .. } => file,
            Self::Read { path, .. } | Self::Remove { path, .. } | Self::Trash { path, .. } => path,
            Self::CopiedButKept { from, .. } | Self::Move { from, .. } => from,
        }
    }
}
