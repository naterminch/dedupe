use super::glob_match;
use crate::cancel::CancelToken;
use crate::cli::Cli;
use crate::scan::{self, FileEntry};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug)]
pub(super) struct Tree {
    pub(super) files: Vec<FileEntry>,
    pub(super) dirs: Vec<PathBuf>,
    pub(super) links: Vec<PathBuf>,
    pub(super) roots: HashSet<PathBuf>,
    pub(super) errors: Vec<String>,
    pub(super) files_skipped: u64,
    pub(super) dirs_skipped: u64,
}

pub(super) fn walk_tree(cli: &Cli, cancel: &CancelToken) -> Tree {
    let ext_filter = scan::extension_filter(&cli.types);
    let type_filtered = !ext_filter.is_empty();
    let size_min = cli
        .min_size
        .as_deref()
        .and_then(|s| crate::util::parse_size(s).ok())
        .unwrap_or(0);
    let size_max = cli
        .max_size
        .as_deref()
        .and_then(|s| crate::util::parse_size(s).ok())
        .unwrap_or(0);
    let mut tree = Tree {
        files: Vec::new(),
        dirs: Vec::new(),
        links: Vec::new(),
        roots: HashSet::new(),
        errors: Vec::new(),
        files_skipped: 0,
        dirs_skipped: 0,
    };
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for raw in &cli.paths {
        if cancel.is_cancelled() {
            return tree;
        }
        let path = Path::new(raw);
        let Ok(link_meta) = fs::symlink_metadata(path) else {
            tree.errors
                .push(format!("path does not exist: {}", path.display()));
            continue;
        };
        if link_meta.file_type().is_symlink() {
            tree.links.push(path.to_path_buf());
            if fs::metadata(path).is_err() {
                continue;
            }
        }
        if path.is_file() {
            collect_mode_file(
                path,
                cli,
                &ext_filter,
                type_filtered,
                size_min,
                size_max,
                &mut seen,
                &mut tree,
            );
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        tree.roots.insert(path.to_path_buf());
        let mut pruned = 0u64;
        let walker = WalkDir::new(path)
            .max_depth(cli.max_depth.map_or(usize::MAX, |d| d + 1))
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let excluded_dir = e.file_type().is_dir() && excluded_dir(e.path(), cli);
                let ignored = ignored_path(e.path(), cli);
                if e.file_type().is_dir() && (excluded_dir || ignored) {
                    pruned += 1;
                    return false;
                }
                true
            });
        for entry in walker {
            if cancel.is_cancelled() {
                return tree;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    tree.errors.push(format!("cannot read: {err}"));
                    continue;
                }
            };
            let path = entry.path();
            if entry.file_type().is_symlink() {
                tree.links.push(path.to_path_buf());
                continue;
            }
            if entry.file_type().is_dir() {
                if !ignored_path(path, cli) {
                    tree.dirs.push(path.to_path_buf());
                }
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            collect_mode_file(
                path,
                cli,
                &ext_filter,
                type_filtered,
                size_min,
                size_max,
                &mut seen,
                &mut tree,
            );
        }
        tree.dirs_skipped += pruned;
    }
    tree
}

#[expect(
    clippy::too_many_arguments,
    reason = "walk callback: filters and output are independent inputs, and a context struct would duplicate `Cli`"
)]
fn collect_mode_file(
    path: &Path,
    cli: &Cli,
    ext_filter: &HashSet<String>,
    type_filtered: bool,
    size_min: u64,
    size_max: u64,
    seen: &mut HashSet<PathBuf>,
    tree: &mut Tree,
) {
    if ignored_path(path, cli) {
        tree.files_skipped += 1;
        return;
    }
    if type_filtered {
        let ok = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ext_filter.contains(&e.to_ascii_lowercase()));
        if !ok {
            tree.files_skipped += 1;
            return;
        }
    }
    let Ok(meta) = fs::metadata(path) else {
        tree.files_skipped += 1;
        return;
    };
    let size = meta.len();
    if size < size_min || (size_max > 0 && size > size_max) {
        tree.files_skipped += 1;
        return;
    }
    if !seen.insert(path.to_path_buf()) {
        return;
    }
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    tree.files.push(FileEntry {
        path: path.to_path_buf(),
        size,
        mtime_secs,
    });
}

fn excluded_dir(path: &Path, cli: &Cli) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    cli.exclude_dir.iter().any(|ex| name.contains(ex.as_str()))
}

fn ignored_path(path: &Path, cli: &Cli) -> bool {
    let raw = path.to_string_lossy();
    if cli.exclude_path.iter().any(|ex| raw.contains(ex.as_str())) {
        return true;
    }
    cli.ignore.iter().any(|pattern| glob_match(pattern, path))
}
