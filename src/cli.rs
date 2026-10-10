use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Parser, ValueEnum};

/// Color scheme for the help output. clap emits these only when the target
/// stream is a terminal (ColorChoice::Auto), so piped output stays plain.
fn cli_styles() -> Styles {
    Styles::styled()
        .usage(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .header(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Yellow.on_default())
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Red.on_default())
}

/// Find duplicate files recursively in one or more paths.
///
/// Files are grouped by size, then compared with a partial hash followed by a
/// full content hash (jdupes-style pipeline). Images and videos additionally
/// have their resolution / duration probed via `ffprobe` when available, and
/// that metadata is reported alongside each duplicate group.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "dedupe",
    version,
    about = "Find duplicate files by content hash",
    long_about = None,
    override_usage = "dedupe [OPTIONS] <PATH>...",
    styles = cli_styles()
)]
pub struct Cli {
    /// Paths to scan (files or directories). Omit to open the graphical
    /// interface instead (same as --gui).
    #[arg(value_name = "PATH", num_args = 1..)]
    pub paths: Vec<String>,

    /// Maximum recursion depth. 0 = files directly in the given paths only
    /// (default: unlimited).
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,

    /// Only scan files with these extensions (comma-separated, case-insensitive,
    /// e.g. --types jpg,png,mp4,txt).
    #[arg(short, long, value_name = "EXTS", value_delimiter = ',')]
    pub types: Vec<String>,

    /// Within each duplicate group, prefer keeping the smallest file
    /// (for media, files in a group always share the same content, hence the
    /// same resolution -- so the smaller encoding is kept).
    /// Conflicts with --keep-newest/--keep-oldest/--keep-best-quality.
    #[arg(short, long)]
    pub keep_smaller: bool,

    /// Within each duplicate group, prefer keeping the most recently
    /// modified file. Conflicts with --keep-smaller/--keep-oldest/--keep-best-quality.
    #[arg(long)]
    pub keep_newest: bool,

    /// Within each duplicate group, prefer keeping the least recently
    /// modified file. Conflicts with --keep-smaller/--keep-newest/--keep-best-quality.
    #[arg(long)]
    pub keep_oldest: bool,

    /// Within each duplicate group, prefer keeping the highest-quality
    /// media: highest resolution, then longest duration, then largest file
    /// (bitrate proxy). Files without metadata tie-break by path; exact
    /// duplicates are byte-identical, so the first path wins there.
    /// Conflicts with --keep-smaller/--keep-newest/--keep-oldest.
    #[arg(long)]
    pub keep_best_quality: bool,

    /// Protect folders: files under these directories are never deleted
    /// and win the keep decision (repeatable). Like czkawka reference dirs.
    #[arg(long, value_name = "PATH", action = clap::ArgAction::Append)]
    pub reference_dir: Vec<String>,

    /// Gather one copy of everything: move each group's keeper into DIR
    /// (flattened; name collisions gain a " (2)" suffix). Keepers already
    /// under DIR and reference-dir keepers are left alone. Every keeper is
    /// re-hashed before moving; changed files are skipped, never moved.
    /// Combines with --delete (dups are removed too) and honors
    /// --dry-run and --yes.
    #[arg(long, value_name = "DIR")]
    pub consolidate_dir: Option<String>,

    /// Move duplicates to the system trash instead of deleting permanently.
    #[arg(long)]
    pub trash: bool,

    /// With --delete: print what would be removed without removing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Delete duplicate files. Prompts per group unless --yes is given.
    #[arg(short = 'D', long)]
    pub delete: bool,

    /// Assume "yes" for all deletion prompts.
    #[arg(short, long)]
    pub yes: bool,

    /// Hash algorithm used for content comparison.
    #[arg(long, value_enum, default_value_t = HashAlgo::Blake3)]
    pub hash: HashAlgo,

    /// Only detect byte-identical duplicates. Near-duplicate images/videos
    /// (the same content re-saved or re-encoded in a different format, e.g.
    /// .png vs .jpg, .mp4 vs .mov) are detected by default via perceptual
    /// hash; this flag disables that pass.
    #[arg(long)]
    pub exact: bool,

    /// Similarity threshold (0-100) above which similar media counts as
    /// duplicated (default: 97).
    #[arg(
        long,
        value_name = "PCT",
        default_value_t = crate::similar::DEFAULT_SIMILARITY_PCT
    )]
    pub similarity: f64,

    /// Do not read or write the perceptual fingerprint cache (stored at
    /// ~/.dedupe/fingerprints.bin). Fingerprints are recomputed from scratch.
    #[arg(long)]
    pub no_cache: bool,

    /// Minimum file size to consider (e.g. 100KB, 1MB, 1GiB). 0 disables.
    #[arg(long, value_name = "SIZE")]
    pub min_size: Option<String>,

    /// Maximum file size to consider (e.g. 2GB). 0 disables.
    #[arg(long, value_name = "SIZE")]
    pub max_size: Option<String>,

    /// Skip directories whose name contains this substring (repeatable).
    #[arg(long, value_name = "NAME", action = clap::ArgAction::Append)]
    pub exclude_dir: Vec<String>,

    /// Skip files whose full path contains this substring (repeatable).
    #[arg(long, value_name = "SUBSTR", action = clap::ArgAction::Append)]
    pub exclude_path: Vec<String>,

    /// Emit machine-readable JSON instead of the human report.
    #[arg(short, long)]
    pub json: bool,

    /// Include per-file media metadata (resolution, duration, codec).
    #[arg(short, long)]
    pub verbose: bool,

    /// Suppress progress output.
    #[arg(short, long)]
    pub quiet: bool,

    /// Worker threads for hashing and media fingerprinting (0 = auto:
    /// all but one core, at least 1). Lower this if scans make the
    /// system laggy, e.g. --jobs 2 on a big media folder.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub jobs: usize,

    /// Open the graphical interface instead of running a command-line scan.
    /// The GUI also opens when no scan path is given.
    #[arg(long)]
    pub gui: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum HashAlgo {
    Blake3,
    Sha256,
    Md5,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn defaults_match_documented_behavior() {
        let cli = Cli::try_parse_from(["dedupe", "/media"]).unwrap();
        assert_eq!(cli.paths, vec!["/media".to_string()]);
        assert_eq!(cli.hash, HashAlgo::Blake3);
        assert!((cli.similarity - crate::similar::DEFAULT_SIMILARITY_PCT).abs() < f64::EPSILON);
        assert_eq!(cli.jobs, 0);
        assert!(!cli.trash && !cli.exact && !cli.delete && !cli.gui);
        assert!(cli.max_depth.is_none() && cli.min_size.is_none());
    }

    #[test]
    fn flags_parse_into_matching_fields() {
        let cli = Cli::try_parse_from([
            "dedupe",
            "--exact",
            "--trash",
            "--keep-newest",
            "--hash",
            "sha256",
            "--similarity",
            "90",
            "--jobs",
            "2",
            "--types",
            "jpg,png",
            "--reference-dir",
            "/backup",
            "a",
            "b",
        ])
        .unwrap();
        assert!(cli.exact && cli.trash && cli.keep_newest);
        assert_eq!(cli.hash, HashAlgo::Sha256);
        assert_eq!(cli.similarity, 90.0);
        assert_eq!(cli.jobs, 2);
        assert_eq!(cli.types, vec!["jpg".to_string(), "png".to_string()]);
        assert_eq!(cli.reference_dir, vec!["/backup".to_string()]);
        assert_eq!(cli.paths, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn short_flags_and_gui_flag_parse() {
        let cli = Cli::try_parse_from(["dedupe", "-D", "-y", "--gui"]).unwrap();
        assert!(cli.delete && cli.yes && cli.gui);
    }

    #[test]
    fn best_quality_and_consolidate_parse() {
        let cli = Cli::try_parse_from([
            "dedupe",
            "--keep-best-quality",
            "--consolidate-dir",
            "D:\\vault",
            ".",
        ])
        .unwrap();
        assert!(cli.keep_best_quality);
        assert!(!cli.keep_smaller && !cli.keep_newest && !cli.keep_oldest);
        assert_eq!(cli.consolidate_dir.as_deref(), Some("D:\\vault"));
        assert_eq!(cli.paths, vec![".".to_string()]);
        let cli = Cli::try_parse_from(["dedupe", "."]).unwrap();
        assert!(!cli.keep_best_quality);
        assert!(cli.consolidate_dir.is_none());
    }
}
