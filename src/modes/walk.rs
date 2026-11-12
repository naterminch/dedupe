use crate::cancel::CancelToken;
use crate::cli::Cli;
use crate::scan::FileEntry;
use crate::walk::{self, Limits, Sink};
use std::collections::HashSet;
use std::path::PathBuf;

/// What one cleanup walk found: the same filtered files the duplicate scan
/// sees, plus the directories, links and roots the modes classify in memory.
#[derive(Debug, Default)]
pub(super) struct Tree {
    pub(super) files: Vec<FileEntry>,
    pub(super) dirs: Vec<PathBuf>,
    pub(super) links: Vec<PathBuf>,
    pub(super) roots: HashSet<PathBuf>,
    pub(super) errors: Vec<String>,
    pub(super) files_skipped: u64,
    pub(super) dirs_skipped: u64,
}

impl Sink for Tree {
    fn file(&mut self, entry: FileEntry) {
        self.files.push(entry);
    }

    fn dir(&mut self, path: PathBuf) {
        self.dirs.push(path);
    }

    fn link(&mut self, path: PathBuf) {
        self.links.push(path);
    }

    fn root(&mut self, path: PathBuf) {
        self.roots.insert(path);
    }

    fn file_skipped(&mut self) {
        self.files_skipped += 1;
    }

    fn dirs_skipped(&mut self, count: u64) {
        self.dirs_skipped += count;
    }

    fn error(&mut self, message: String) {
        self.errors.push(message);
    }
}

/// Collect the tree the cleanup modes filter in memory.
pub(super) fn walk_tree(cli: &Cli, cancel: &CancelToken) -> Tree {
    let mut tree = Tree::default();
    walk::walk(cli, Limits::from_cli(cli), cancel, &mut tree);
    tree
}
