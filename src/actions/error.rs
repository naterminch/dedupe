//! Typed failures for the delete, move and hard-link paths.
//!
//! These errors cross into the GUI, which tells them apart by variant: a trash
//! call that the desktop refused reads differently from a destination folder
//! that could not be created, even though both used to arrive as one formatted
//! string. `anyhow` stays in `main.rs`, where every error is printed and the
//! type never matters.

use std::io;
use std::path::PathBuf;

/// What went wrong while removing, moving or hard-linking files.
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

    /// A duplicate could not be set aside before being replaced by a hard
    /// link, so it was left untouched.
    #[error("could not set {} aside for hard-linking: {source}", path.display())]
    Stage {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The hard link could not be created (often a different volume); the
    /// original file was put back.
    #[error("could not hard-link {} to {}: {source}", path.display(), keeper.display())]
    Link {
        path: PathBuf,
        keeper: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The hard link failed and the original could not be put back either:
    /// its bytes survive only at `backup`.
    #[error(
        "hard link for {} failed ({link}) and the original could not be restored; it is kept at {}: {source}",
        path.display(),
        backup.display()
    )]
    Restore {
        path: PathBuf,
        backup: PathBuf,
        link: io::Error,
        #[source]
        source: io::Error,
    },

    /// The hard link is in place, but the original's staged copy beside it
    /// could not be removed, so its space is still taken.
    #[error(
        "{} is now a hard link, but its old copy could not be removed from {}: {source}",
        path.display(),
        backup.display()
    )]
    BackupKept {
        path: PathBuf,
        backup: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl ActionError {
    /// The file the failure is about.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        match self {
            Self::Destination { file, .. } => file,
            Self::Read { path, .. }
            | Self::Remove { path, .. }
            | Self::Trash { path, .. }
            | Self::Stage { path, .. }
            | Self::Link { path, .. }
            | Self::Restore { path, .. }
            | Self::BackupKept { path, .. } => path,
            Self::CopiedButKept { from, .. } | Self::Move { from, .. } => from,
        }
    }
}
