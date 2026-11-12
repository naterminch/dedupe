//! The one directory walk shared by duplicate scanning and the cleanup modes.
//!
//! Filtering lives here on purpose. It used to be implemented twice — once for
//! the duplicate scan, once for `--find` modes — and the two copies drifted:
//! the cleanup walker ignored `--ignore` globs the docs promise, and the
//! duplicate walker reported a missing path where the cleanup walker reported
//! a dangling link. A caller now only says what to do with what the walk
//! keeps, which is what [`Sink`] is for.

use crate::cancel::CancelToken;
use crate::cli::Cli;
use crate::modes::glob_match;
use crate::scan::{self, FileEntry};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Size window for the walk, already parsed by the caller.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub size_min: u64,
    /// `0` disables the upper bound.
    pub size_max: u64,
}

impl Limits {
    /// Parse `--min-size`/`--max-size` the way the cleanup modes do: a value
    /// that does not parse falls back to "no bound" rather than failing here.
    #[must_use]
    pub fn from_cli(cli: &Cli) -> Self {
        let read = |raw: Option<&str>| {
            raw.and_then(|value| crate::util::parse_size(value).ok())
                .unwrap_or(0)
        };
        Self {
            size_min: read(cli.min_size.as_deref()),
            size_max: read(cli.max_size.as_deref()),
        }
    }
}

/// Receives what the walk keeps, skips or cannot read.
///
/// Every method has a default, so a caller overrides only the categories it
/// collects; the duplicate scan, for example, has no use for directories.
pub trait Sink {
    /// A file that passed every filter.
    fn file(&mut self, entry: FileEntry);

    /// A directory that survived pruning.
    fn dir(&mut self, _path: PathBuf) {}

    /// A symlink, dangling or not, so the cleanup modes can classify it.
    fn link(&mut self, _path: PathBuf) {}

    /// A given path the walk descended into.
    fn root(&mut self, _path: PathBuf) {}

    /// A file rejected by a filter, or one whose metadata could not be read.
    fn file_skipped(&mut self) {}

    /// Directories pruned by `--exclude-dir`, `--exclude-path` or `--ignore`.
    fn dirs_skipped(&mut self, _count: u64) {}

    /// A path that could not be walked at all.
    fn error(&mut self, message: String);
}

/// Walk every configured path once, handing accepted entries to `sink`.
///
/// Cancellation is polled per path and per entry, so an interrupted walk stops
/// within one entry and leaves the sink with a partial result.
pub fn walk(cli: &Cli, limits: Limits, cancel: &CancelToken, sink: &mut impl Sink) {
    let extensions = scan::extension_filter(&cli.types);
    let filter = Filter {
        type_filtered: !extensions.is_empty(),
        extensions,
    };
    let mut walker = Walker {
        cli,
        limits,
        filter,
        seen: HashSet::new(),
        sink,
    };
    walker.run(cancel);
}

struct Filter {
    extensions: HashSet<String>,
    type_filtered: bool,
}

struct Walker<'a, S: Sink> {
    cli: &'a Cli,
    limits: Limits,
    filter: Filter,
    seen: HashSet<PathBuf>,
    sink: &'a mut S,
}

impl<S: Sink> Walker<'_, S> {
    fn run(&mut self, cancel: &CancelToken) {
        let paths = self.cli.paths.clone();
        for raw in &paths {
            if cancel.is_cancelled() {
                return;
            }
            self.walk_root(Path::new(raw), cancel);
        }
    }

    fn walk_root(&mut self, path: &Path, cancel: &CancelToken) {
        let Ok(link_meta) = fs::symlink_metadata(path) else {
            self.sink
                .error(format!("path does not exist: {}", path.display()));
            return;
        };
        if link_meta.file_type().is_symlink() {
            self.sink.link(path.to_path_buf());
            if fs::metadata(path).is_err() {
                // A dangling link is worth reporting, but there is nothing to
                // walk through it.
                return;
            }
        }
        if path.is_file() {
            self.accept_file(path);
            return;
        }
        if !path.is_dir() {
            return;
        }
        self.sink.root(path.to_path_buf());

        let cli = self.cli;
        let mut pruned = 0u64;
        {
            let entries = WalkDir::new(path)
                // walkdir counts the root as depth 0, while `--max-depth 0`
                // means "direct children only", hence the +1 offset.
                .max_depth(cli.max_depth.map_or(usize::MAX, |depth| depth + 1))
                .into_iter()
                .filter_entry(|entry| {
                    if entry.depth() == 0 {
                        return true; // never prune the root itself
                    }
                    if entry.file_type().is_dir() && excluded_tree(entry.path(), cli) {
                        pruned += 1;
                        return false;
                    }
                    true
                });
            for entry in entries {
                if cancel.is_cancelled() {
                    return;
                }
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        self.sink.error(format!("cannot read: {err}"));
                        continue;
                    }
                };
                let path = entry.path();
                if entry.file_type().is_symlink() {
                    self.sink.link(path.to_path_buf());
                    continue;
                }
                if entry.file_type().is_dir() {
                    if !excluded(path, self.cli) {
                        self.sink.dir(path.to_path_buf());
                    }
                    continue;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                self.accept_file(path);
            }
        }
        self.sink.dirs_skipped(pruned);
    }

    fn accept_file(&mut self, path: &Path) {
        if excluded(path, self.cli)
            || (self.filter.type_filtered
                && !scan::matches_extension(path, &self.filter.extensions))
        {
            self.sink.file_skipped();
            return;
        }
        let Ok(meta) = fs::metadata(path) else {
            self.sink.file_skipped();
            return;
        };
        let size = meta.len();
        if size < self.limits.size_min || (self.limits.size_max > 0 && size > self.limits.size_max)
        {
            self.sink.file_skipped();
            return;
        }
        // A path can be reached twice (a file given directly and found by the
        // walk, or two overlapping roots): keep the first sighting only.
        if !self.seen.insert(path.to_path_buf()) {
            return;
        }
        let mtime_secs = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs() as i64);
        self.sink.file(FileEntry {
            path: path.to_path_buf(),
            size,
            mtime_secs,
        });
    }
}

/// Paths the user asked to skip outright: `--exclude-path` substrings and
/// `--ignore` globs, applied to files and directories alike.
fn excluded(path: &Path, cli: &Cli) -> bool {
    let raw = path.to_string_lossy();
    cli.exclude_path
        .iter()
        .any(|needle| raw.contains(needle.as_str()))
        || cli.ignore.iter().any(|pattern| glob_match(pattern, path))
}

/// Everything [`excluded`] covers, plus directories whose name matches
/// `--exclude-dir`. Pruning these keeps the walk out of subtrees it would
/// reject file by file anyway.
fn excluded_tree(path: &Path, cli: &Cli) -> bool {
    let name_matches = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            cli.exclude_dir
                .iter()
                .any(|needle| name.contains(needle.as_str()))
        });
    name_matches || excluded(path, cli)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::fs;

    #[derive(Default)]
    struct Recording {
        files: Vec<String>,
        dirs_skipped: u64,
        errors: Vec<String>,
    }

    impl Sink for Recording {
        fn file(&mut self, entry: FileEntry) {
            self.files.push(
                entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }

        fn dirs_skipped(&mut self, count: u64) {
            self.dirs_skipped += count;
        }

        fn error(&mut self, message: String) {
            self.errors.push(message);
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dedupe-walk-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn walk_dir(cli: &Cli) -> Recording {
        let mut sink = Recording::default();
        walk(
            cli,
            Limits {
                size_min: 0,
                size_max: 0,
            },
            &CancelToken::new(),
            &mut sink,
        );
        sink
    }

    #[test]
    fn ignore_glob_skips_matching_files() {
        let dir = tmpdir("ignore-file");
        fs::write(dir.join("keep.txt"), b"keep").unwrap();
        fs::write(dir.join("noise.log"), b"noise").unwrap();
        let cli =
            Cli::try_parse_from(["dedupe", "--ignore", "*.log", dir.to_str().unwrap()]).unwrap();

        let sink = walk_dir(&cli);

        assert_eq!(sink.files, vec!["keep.txt".to_string()]);
        assert!(
            sink.errors.is_empty(),
            "unexpected errors: {:?}",
            sink.errors
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignore_glob_prunes_matching_directories() {
        let dir = tmpdir("ignore-dir");
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("node_modules").join("dep.txt"), b"dep").unwrap();
        fs::write(dir.join("app.txt"), b"app").unwrap();
        let cli =
            Cli::try_parse_from(["dedupe", "--ignore", "node_modules", dir.to_str().unwrap()])
                .unwrap();

        let sink = walk_dir(&cli);

        assert_eq!(sink.files, vec!["app.txt".to_string()]);
        assert_eq!(sink.dirs_skipped, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_path_is_reported_once_and_the_walk_continues() {
        let dir = tmpdir("missing");
        fs::write(dir.join("a.txt"), b"a").unwrap();
        let cli = Cli::try_parse_from([
            "dedupe",
            r"Z:\definitely\not\here-54321",
            dir.to_str().unwrap(),
        ])
        .unwrap();

        let sink = walk_dir(&cli);

        assert_eq!(sink.files, vec!["a.txt".to_string()]);
        assert_eq!(sink.errors.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}
