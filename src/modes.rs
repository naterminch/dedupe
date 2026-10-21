use crate::cancel::CancelToken;
use crate::cli::{Cli, FindMode, MatchMode, MusicMode};
use crate::hashing::HashEngine;
use crate::media;
use crate::scan::{self, FileEntry};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const AUDIO_EXTS: [&str; 12] = [
    "mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav", "wma", "aiff", "aif", "ape",
];

#[derive(Debug, Clone)]
pub struct Finding {
    pub path: PathBuf,
    pub size: u64,
    pub kind: &'static str,
    pub detail: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct FindingGroup {
    pub label: String,
    pub detail: String,
    pub members: Vec<PathBuf>,
    pub sizes: Vec<u64>,
    pub bytes: u64,
    pub reclaimable: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ModeResult {
    pub mode: FindMode,
    pub findings: Vec<Finding>,
    pub groups: Vec<FindingGroup>,
    pub files_scanned: u64,
    pub dirs_scanned: u64,
    pub bytes_scanned: u64,
    pub files_skipped: u64,
    pub dirs_skipped: u64,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
    pub cancelled: bool,
}

impl ModeResult {
    pub fn finding_count(&self) -> usize {
        self.findings.len()
    }

    pub fn reclaimable(&self) -> u64 {
        self.findings
            .iter()
            .filter(|f| !f.is_dir)
            .map(|f| f.size)
            .sum::<u64>()
            + self.groups.iter().map(|g| g.reclaimable).sum::<u64>()
    }

    pub fn is_empty(&self) -> bool {
        self.findings.is_empty() && self.groups.is_empty()
    }
}

pub fn mode_name(mode: FindMode) -> &'static str {
    match mode {
        FindMode::Duplicates => "duplicates",
        FindMode::EmptyFiles => "empty-files",
        FindMode::EmptyDirs => "empty-dirs",
        FindMode::BigFiles => "big-files",
        FindMode::TempFiles => "temp-files",
        FindMode::InvalidSymlinks => "invalid-symlinks",
        FindMode::BrokenFiles => "broken-files",
        FindMode::BadExtensions => "bad-extensions",
        FindMode::BadNames => "bad-names",
        FindMode::DuplicateFolders => "duplicate-folders",
        FindMode::Music => "music",
    }
}

pub fn mode_title(mode: FindMode) -> &'static str {
    match mode {
        FindMode::Duplicates => "Duplicate files",
        FindMode::EmptyFiles => "Empty files",
        FindMode::EmptyDirs => "Empty folders",
        FindMode::BigFiles => "Largest files",
        FindMode::TempFiles => "Temporary and backup files",
        FindMode::InvalidSymlinks => "Broken symlinks",
        FindMode::BrokenFiles => "Corrupt or unreadable media",
        FindMode::BadExtensions => "Files with the wrong extension",
        FindMode::BadNames => "Problematic file names",
        FindMode::DuplicateFolders => "Duplicate folder trees",
        FindMode::Music => "Duplicate music",
    }
}

#[derive(Debug)]
struct Tree {
    files: Vec<FileEntry>,
    dirs: Vec<PathBuf>,
    links: Vec<PathBuf>,
    roots: HashSet<PathBuf>,
    errors: Vec<String>,
    files_skipped: u64,
    dirs_skipped: u64,
}

pub fn run_mode(cli: &Cli, cancel: &CancelToken) -> ModeResult {
    let tree = walk_tree(cli, cancel);
    let mut result = ModeResult {
        mode: cli.find,
        files_scanned: tree.files.len() as u64,
        dirs_scanned: tree.dirs.len() as u64,
        bytes_scanned: tree.files.iter().map(|f| f.size).sum(),
        files_skipped: tree.files_skipped,
        dirs_skipped: tree.dirs_skipped,
        errors: tree.errors.clone(),
        cancelled: cancel.is_cancelled(),
        ..ModeResult::default()
    };
    if cancel.is_cancelled() {
        return result;
    }
    match cli.find {
        FindMode::Duplicates => {}
        FindMode::EmptyFiles => {
            result.findings = tree
                .files
                .iter()
                .filter(|f| f.size == 0)
                .map(|f| finding(f.path.clone(), 0, "empty", "zero-byte file", false))
                .collect();
        }
        FindMode::EmptyDirs => {
            result.findings = empty_dirs(&tree);
        }
        FindMode::BigFiles => {
            let mut files: Vec<&FileEntry> = tree.files.iter().collect();
            files.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
            let total = files.len();
            let take = cli.top.max(1);
            result.findings = files
                .into_iter()
                .take(take)
                .enumerate()
                .map(|(i, f)| {
                    finding(
                        f.path.clone(),
                        f.size,
                        "big",
                        format!("#{} of {} scanned files", i + 1, total),
                        false,
                    )
                })
                .collect();
        }
        FindMode::TempFiles => {
            result.findings = tree
                .files
                .iter()
                .filter_map(|f| {
                    temp_reason(&f.path)
                        .map(|reason| finding(f.path.clone(), f.size, "temp", reason, false))
                })
                .collect();
        }
        FindMode::InvalidSymlinks => {
            result.findings = invalid_symlinks(&tree);
        }
        FindMode::BrokenFiles => {
            result.findings = broken_files(&tree, &mut result.notes, cancel);
        }
        FindMode::BadExtensions => {
            result.findings = bad_extensions(&tree);
        }
        FindMode::BadNames => {
            result.findings = bad_names(&tree);
        }
        FindMode::DuplicateFolders => {
            let engine = HashEngine::new(cli.hash);
            let (groups, notes) = duplicate_folders(&tree, &engine, cancel);
            result.groups = groups;
            result.notes.extend(notes);
        }
        FindMode::Music => {
            result.groups = music_groups(cli, &tree, &mut result.notes, cancel);
        }
    }
    result.findings.sort_by(|a, b| a.path.cmp(&b.path));
    result.cancelled = cancel.is_cancelled();
    result
}

fn finding(
    path: PathBuf,
    size: u64,
    kind: &'static str,
    detail: impl Into<String>,
    is_dir: bool,
) -> Finding {
    Finding {
        path,
        size,
        kind,
        detail: detail.into(),
        is_dir,
    }
}

fn walk_tree(cli: &Cli, cancel: &CancelToken) -> Tree {
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
            .max_depth(cli.max_depth.map(|d| d + 1).unwrap_or(usize::MAX))
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

#[allow(clippy::too_many_arguments)]
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
            .map(|e| ext_filter.contains(&e.to_ascii_lowercase()))
            .unwrap_or(false);
        if !ok {
            tree.files_skipped += 1;
            return;
        }
    }
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => {
            tree.files_skipped += 1;
            return;
        }
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

/// Minimal glob matcher: `**` crosses separators, `*` and `?` do not.
pub fn glob_match(pattern: &str, path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = if pattern.contains('/') || pattern.contains('\\') {
        text.as_str()
    } else {
        name.as_str()
    };
    let pat = pattern.replace('\\', "/").to_ascii_lowercase();
    let target = target.to_ascii_lowercase();
    if glob_bytes(pat.as_bytes(), target.as_bytes()) {
        return true;
    }
    if pat.contains('/') {
        for (i, b) in target.bytes().enumerate() {
            if b == b'/' && glob_bytes(pat.as_bytes(), &target.as_bytes()[i + 1..]) {
                return true;
            }
        }
    }
    false
}

fn glob_bytes(pattern: &[u8], text: &[u8]) -> bool {
    let mut memo: HashMap<(usize, usize), bool> = HashMap::new();
    fn go(
        p: &[u8],
        t: &[u8],
        pi: usize,
        ti: usize,
        memo: &mut HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(v) = memo.get(&(pi, ti)) {
            return *v;
        }
        let result = if pi == p.len() {
            ti == t.len()
        } else if p[pi] == b'*' {
            if pi + 1 < p.len() && p[pi + 1] == b'*' {
                let next = pi + 2;
                go(p, t, next, ti, memo) || (ti < t.len() && go(p, t, pi, ti + 1, memo))
            } else {
                go(p, t, pi + 1, ti, memo)
                    || (ti < t.len() && t[ti] != b'/' && go(p, t, pi, ti + 1, memo))
            }
        } else if pi < p.len() && p[pi] == b'?' {
            ti < t.len() && t[ti] != b'/' && go(p, t, pi + 1, ti + 1, memo)
        } else {
            ti < t.len() && p[pi] == t[ti] && go(p, t, pi + 1, ti + 1, memo)
        };
        memo.insert((pi, ti), result);
        result
    }
    go(pattern, text, 0, 0, &mut memo)
}

fn empty_dirs(tree: &Tree) -> Vec<Finding> {
    tree.dirs
        .iter()
        .filter(|d| !tree.roots.contains(*d))
        .filter(|d| {
            fs::read_dir(d)
                .map(|mut it| it.next().is_none())
                .unwrap_or(false)
        })
        .map(|d| finding(d.clone(), 0, "empty-dir", "empty folder", true))
        .collect()
}

fn invalid_symlinks(tree: &Tree) -> Vec<Finding> {
    tree.links
        .iter()
        .filter(|p| {
            fs::symlink_metadata(p)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
                && fs::metadata(p).is_err()
        })
        .map(|p| finding(p.clone(), 0, "symlink", "target does not exist", false))
        .collect()
}

fn temp_reason(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "tmp"
            | "temp"
            | "bak"
            | "old"
            | "orig"
            | "part"
            | "partial"
            | "crdownload"
            | "download"
            | "cache"
            | "swp"
            | "swo"
            | "swn"
    ) {
        return Some("temporary or backup extension");
    }
    if name.starts_with("~$") || name.starts_with(".~lock.") {
        return Some("Office lock file");
    }
    if name.ends_with('~') {
        return Some("backup file (trailing tilde)");
    }
    if matches!(
        name.as_str(),
        ".ds_store" | "thumbs.db" | "ehthumbs.db" | "desktop.ini"
    ) {
        return Some("operating-system metadata file");
    }
    None
}

fn broken_files(tree: &Tree, notes: &mut Vec<String>, cancel: &CancelToken) -> Vec<Finding> {
    let needs_ffprobe = tree
        .files
        .iter()
        .any(|f| media::classify(&f.path) == media::MediaKind::Video || is_audio(&f.path));
    let ffprobe = if needs_ffprobe {
        let ok = media::ffprobe_available();
        if !ok {
            notes.push(
                "ffprobe is not installed: corrupt audio/video files were not checked.".into(),
            );
        }
        ok
    } else {
        false
    };
    let mut out = Vec::new();
    for file in &tree.files {
        if cancel.is_cancelled() {
            break;
        }
        let ext = file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if media::classify(&file.path) == media::MediaKind::Image {
            if matches!(ext.as_str(), "svg" | "heic" | "heif" | "avif") {
                continue;
            }
            let decoded = (|| -> Result<(u32, u32), String> {
                let reader = image::ImageReader::open(&file.path).map_err(|e| e.to_string())?;
                let reader = reader.with_guessed_format().map_err(|e| e.to_string())?;
                reader.into_dimensions().map_err(|e| e.to_string())
            })();
            if let Err(err) = decoded {
                out.push(finding(
                    file.path.clone(),
                    file.size,
                    "broken",
                    format!("image cannot be decoded: {err}"),
                    false,
                ));
            }
        } else if ffprobe
            && (media::classify(&file.path) == media::MediaKind::Video || is_audio(&file.path))
            && !ffprobe_readable(&file.path)
        {
            out.push(finding(
                file.path.clone(),
                file.size,
                "broken",
                "ffprobe cannot read this audio/video file",
                false,
            ));
        }
    }
    out
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn ffprobe_readable(path: &Path) -> bool {
    crate::util::quiet_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<f64>()
                    .map(|d| d >= 0.0)
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn sniff_extension(path: &Path) -> Option<&'static str> {
    let mut file = File::open(path).ok()?;
    let mut head = [0u8; 16];
    let n = file.read(&mut head).ok()?;
    let head = &head[..n];
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpg");
    }
    if head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("png");
    }
    if head.starts_with(b"GIF8") {
        return Some("gif");
    }
    if head.starts_with(b"BM") {
        return Some("bmp");
    }
    if head.len() >= 12 && &head[0..4] == b"RIFF" {
        match &head[8..12] {
            b"WEBP" => return Some("webp"),
            b"WAVE" => return Some("wav"),
            b"AVI " => return Some("avi"),
            _ => {}
        }
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return Some("tiff");
    }
    if head.starts_with(b"%PDF") {
        return Some("pdf");
    }
    if head.starts_with(&[0x1F, 0x8B]) {
        return Some("gzip");
    }
    if head.starts_with(b"Rar!") {
        return Some("rar");
    }
    if head.starts_with(&[0x37, 0x7A, 0xBC, 0xAF]) {
        return Some("7z");
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("mkv");
    }
    if head.starts_with(b"fLaC") {
        return Some("flac");
    }
    if head.starts_with(b"OggS") {
        return Some("ogg");
    }
    if head.starts_with(b"ID3") {
        return Some("mp3");
    }
    if head.len() >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0 {
        return Some("mp3");
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Some("mp4");
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Some("zip");
    }
    None
}

fn extension_matches(actual: &str, ext: &str) -> bool {
    match actual {
        "jpg" => matches!(ext, "jpg" | "jpeg" | "jfif"),
        "png" => ext == "png",
        "gif" => ext == "gif",
        "bmp" => ext == "bmp",
        "webp" => ext == "webp",
        "tiff" => matches!(ext, "tif" | "tiff"),
        "pdf" => ext == "pdf",
        "gzip" => matches!(ext, "gz" | "tgz"),
        "rar" => ext == "rar",
        "7z" => ext == "7z",
        "mkv" => matches!(ext, "mkv" | "webm"),
        "flac" => ext == "flac",
        "ogg" => matches!(ext, "ogg" | "oga" | "opus"),
        "mp3" => ext == "mp3",
        "mp4" => matches!(ext, "mp4" | "m4v" | "m4a" | "mov" | "3gp"),
        "wav" => ext == "wav",
        "avi" => ext == "avi",
        "zip" => matches!(
            ext,
            "zip" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub" | "jar" | "apk"
        ),
        _ => true,
    }
}

fn bad_extensions(tree: &Tree) -> Vec<Finding> {
    let mut out = Vec::new();
    for file in &tree.files {
        let Some(ext) = file.path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        let Some(actual) = sniff_extension(&file.path) else {
            continue;
        };
        if !extension_matches(actual, &ext) {
            let label = if actual == "zip" {
                "ZIP/office"
            } else {
                actual
            };
            out.push(finding(
                file.path.clone(),
                file.size,
                "bad-extension",
                format!("contents look like {label}, extension is .{ext}"),
                false,
            ));
        }
    }
    out
}

fn bad_name_reason(name: &str) -> Option<String> {
    let mut issues = Vec::new();
    if name.trim() != name {
        issues.push("leading or trailing whitespace");
    }
    if name.ends_with('.') {
        issues.push("ends with a dot");
    }
    if name.ends_with(' ') {
        issues.push("ends with a space");
    }
    if name.chars().any(|c| c.is_control()) {
        issues.push("contains control characters");
    }
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        issues.push("contains Windows-invalid characters");
    }
    if name.len() > 255 {
        issues.push("name is longer than 255 bytes");
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        issues.push("Windows-reserved device name");
    }
    if issues.is_empty() {
        None
    } else {
        Some(issues.join("; "))
    }
}

fn bad_names(tree: &Tree) -> Vec<Finding> {
    let mut out = Vec::new();
    for file in &tree.files {
        let name = file.path.file_name().unwrap_or_default().to_string_lossy();
        if let Some(reason) = bad_name_reason(&name) {
            out.push(finding(
                file.path.clone(),
                file.size,
                "bad-name",
                reason,
                false,
            ));
        }
    }
    for dir in &tree.dirs {
        if tree.roots.contains(dir) {
            continue;
        }
        let name = dir.file_name().unwrap_or_default().to_string_lossy();
        if let Some(reason) = bad_name_reason(&name) {
            out.push(finding(dir.clone(), 0, "bad-name", reason, true));
        }
    }
    out
}

fn duplicate_folders(
    tree: &Tree,
    engine: &HashEngine,
    cancel: &CancelToken,
) -> (Vec<FindingGroup>, Vec<String>) {
    let mut notes = Vec::new();
    let mut files_by_dir: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (i, file) in tree.files.iter().enumerate() {
        if let Some(parent) = file.path.parent() {
            files_by_dir
                .entry(parent.to_path_buf())
                .or_default()
                .push(i);
        }
    }
    let mut children: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for dir in &tree.dirs {
        if let Some(parent) = dir.parent() {
            children
                .entry(parent.to_path_buf())
                .or_default()
                .push(dir.clone());
        }
    }
    let mut dirs: Vec<PathBuf> = tree.dirs.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    let mut signatures: HashMap<PathBuf, String> = HashMap::new();
    let mut file_counts: HashMap<PathBuf, u64> = HashMap::new();
    let mut byte_counts: HashMap<PathBuf, u64> = HashMap::new();
    for dir in &dirs {
        if cancel.is_cancelled() {
            break;
        }
        let mut entries: Vec<String> = Vec::new();
        let mut count = 0u64;
        let mut bytes = 0u64;
        if let Some(files) = files_by_dir.get(dir) {
            for &i in files {
                let file = &tree.files[i];
                let name = file
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                match engine.full(&file.path) {
                    Ok(hash) => entries.push(format!("f:{name}\0{}\0{hash}", file.size)),
                    Err(err) => notes.push(format!("cannot hash {}: {err}", file.path.display())),
                }
                count += 1;
                bytes += file.size;
            }
        }
        if let Some(kids) = children.get(dir) {
            let mut kids = kids.clone();
            kids.sort();
            for kid in kids {
                if let Some(sig) = signatures.get(&kid) {
                    let name = kid
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    entries.push(format!("d:{name}\0{sig}"));
                    count += file_counts.get(&kid).copied().unwrap_or(0);
                    bytes += byte_counts.get(&kid).copied().unwrap_or(0);
                }
            }
        }
        entries.sort();
        let sig = blake3::hash(entries.join("\n").as_bytes())
            .to_hex()
            .to_string();
        signatures.insert(dir.clone(), sig);
        file_counts.insert(dir.clone(), count);
        byte_counts.insert(dir.clone(), bytes);
    }
    let mut by_sig: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for dir in &tree.dirs {
        if tree.roots.contains(dir) {
            continue;
        }
        let Some(sig) = signatures.get(dir) else {
            continue;
        };
        if file_counts.get(dir).copied().unwrap_or(0) == 0 {
            continue;
        }
        by_sig.entry(sig.clone()).or_default().push(dir.clone());
    }
    let mut groups = Vec::new();
    for members in by_sig.into_values() {
        if members.len() < 2 {
            continue;
        }
        let mut filtered: Vec<PathBuf> = Vec::new();
        for member in members {
            if filtered
                .iter()
                .any(|m| member.starts_with(m) || m.starts_with(&member))
            {
                continue;
            }
            filtered.push(member);
        }
        if filtered.len() < 2 {
            continue;
        }
        let bytes = file_counts.get(&filtered[0]).copied().unwrap_or(0);
        let total_bytes = byte_counts.get(&filtered[0]).copied().unwrap_or(0);
        let sizes = vec![total_bytes; filtered.len()];
        groups.push(FindingGroup {
            label: format!("Identical folder tree ({} copies)", filtered.len()),
            detail: format!(
                "{bytes} {}, {}",
                if bytes == 1 { "file" } else { "files" },
                crate::util::human_bytes(total_bytes)
            ),
            reclaimable: total_bytes.saturating_mul(filtered.len() as u64 - 1),
            members: filtered,
            sizes,
            bytes: total_bytes,
        });
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.reclaimable));
    (groups, notes)
}

pub struct MatchBucket {
    pub key: String,
    pub entries: Vec<FileEntry>,
}

pub fn match_entries(
    entries: &[FileEntry],
    mode: MatchMode,
    fuzzy_threshold: f64,
) -> Vec<MatchBucket> {
    match mode {
        MatchMode::Size => {
            let mut map: BTreeMap<u64, Vec<FileEntry>> = BTreeMap::new();
            for e in entries {
                map.entry(e.size).or_default().push(e.clone());
            }
            map.into_iter()
                .filter(|(_, v)| v.len() >= 2)
                .map(|(size, entries)| MatchBucket {
                    key: format!("size:{size}"),
                    entries,
                })
                .collect()
        }
        MatchMode::Name => {
            let mut map: BTreeMap<String, Vec<FileEntry>> = BTreeMap::new();
            for e in entries {
                map.entry(name_key(&e.path)).or_default().push(e.clone());
            }
            map.into_iter()
                .filter(|(_, v)| v.len() >= 2)
                .map(|(key, entries)| MatchBucket { key, entries })
                .collect()
        }
        MatchMode::Fuzzy => fuzzy_buckets(entries, fuzzy_threshold),
        MatchMode::Content => Vec::new(),
    }
}

fn name_key(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lowered = stem.to_ascii_lowercase();
    let mut base = lowered.as_str();
    for marker in [" - copy", " copy", " - kopie", " kopie"] {
        if let Some(idx) = base.rfind(marker)
            && idx > 0
            && base[idx + marker.len()..]
                .trim()
                .chars()
                .all(|c| c.is_ascii_digit() || c == '(' || c == ')' || c == ' ')
        {
            base = &base[..idx];
        }
    }
    let mut normalized = String::new();
    let mut last_space = false;
    for c in base.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                normalized.push(lc);
            }
            last_space = false;
        } else if !last_space {
            normalized.push(' ');
            last_space = true;
        }
    }
    normalized.trim().to_string()
}

fn fuzzy_buckets(entries: &[FileEntry], threshold: f64) -> Vec<MatchBucket> {
    let threshold = (threshold / 100.0).clamp(0.0, 1.0);
    let mut keyed: Vec<(String, &FileEntry)> = entries
        .iter()
        .filter_map(|e| {
            let key = name_key(&e.path);
            if key.is_empty() { None } else { Some((key, e)) }
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.path.cmp(&b.1.path)));
    let mut clusters: Vec<(String, Vec<FileEntry>)> = Vec::new();
    for (key, entry) in keyed {
        let mut best: Option<(f64, usize)> = None;
        for (i, (rep, _)) in clusters.iter().enumerate() {
            let ratio = name_similarity(&key, rep);
            if ratio >= threshold && best.map(|(b, _)| ratio > b).unwrap_or(true) {
                best = Some((ratio, i));
            }
        }
        match best {
            Some((_, i)) => clusters[i].1.push(entry.clone()),
            None => clusters.push((key, vec![entry.clone()])),
        }
    }
    clusters
        .into_iter()
        .filter(|(_, v)| v.len() >= 2)
        .map(|(key, entries)| MatchBucket {
            key: format!("fuzzy:{key}"),
            entries,
        })
        .collect()
}

/// Normalized Levenshtein similarity in 0..=1.
pub fn name_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let dist = prev[b.len()] as f64;
    1.0 - dist / a.len().max(b.len()) as f64
}

fn music_groups(
    cli: &Cli,
    tree: &Tree,
    notes: &mut Vec<String>,
    cancel: &CancelToken,
) -> Vec<FindingGroup> {
    let tracks: Vec<&FileEntry> = tree.files.iter().filter(|f| is_audio(&f.path)).collect();
    if tracks.is_empty() {
        return Vec::new();
    }
    if cli.music_by == MusicMode::Content {
        return crate::audio::music_groups(&tracks, cli.similarity / 100.0, notes, cancel);
    }
    let mut map: BTreeMap<String, (TrackTags, Vec<PathBuf>, u64)> = BTreeMap::new();
    for track in tracks {
        if cancel.is_cancelled() {
            break;
        }
        let Some(tags) = read_tags(&track.path) else {
            continue;
        };
        let title = normalize_tag(&tags.title);
        if title.is_empty() {
            continue;
        }
        let artist = normalize_tag(&tags.artist);
        let album = normalize_tag(&tags.album);
        let key = format!("{title}\u{1f}{artist}\u{1f}{album}");
        let entry = map
            .entry(key)
            .or_insert_with(|| (tags.clone(), Vec::new(), 0));
        entry.1.push(track.path.clone());
        entry.2 += track.size;
    }
    let mut out = Vec::new();
    for (_, (tags, members, bytes)) in map {
        if members.len() < 2 {
            continue;
        }
        let mut members_sized: Vec<(PathBuf, u64)> = members
            .into_iter()
            .map(|p| {
                let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                (p, size)
            })
            .collect();
        members_sized.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let sizes: Vec<u64> = members_sized.iter().map(|(_, s)| *s).collect();
        let largest = sizes.first().copied().unwrap_or(0);
        let members_sorted: Vec<PathBuf> = members_sized.into_iter().map(|(p, _)| p).collect();
        let label = if tags.artist.trim().is_empty() {
            tags.title.trim().to_string()
        } else {
            format!("{} — {}", tags.artist.trim(), tags.title.trim())
        };
        out.push(FindingGroup {
            label,
            detail: if tags.album.trim().is_empty() {
                format!("{} tracks, same tags", members_sorted.len())
            } else {
                format!(
                    "{} tracks, album: {}",
                    members_sorted.len(),
                    tags.album.trim()
                )
            },
            reclaimable: bytes.saturating_sub(largest),
            sizes,
            bytes,
            members: members_sorted,
        });
    }
    out.sort_by_key(|g| std::cmp::Reverse(g.reclaimable));
    out
}

#[derive(Debug, Clone, Default)]
struct TrackTags {
    title: String,
    artist: String,
    album: String,
}

fn normalize_tag(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out.trim().to_string()
}

fn read_tags(path: &Path) -> Option<TrackTags> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => read_mp3_tags(path),
        "flac" => read_flac_tags(path),
        "m4a" | "mp4" | "m4b" | "aac" => read_mp4_tags(path),
        "wav" => read_wav_tags(path),
        _ => None,
    }
    .filter(|t| !t.title.trim().is_empty())
}

fn synchsafe(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .fold(0usize, |acc, &b| (acc << 7) | (b & 0x7f) as usize)
}

fn read_mp3_tags(path: &Path) -> Option<TrackTags> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut header = [0u8; 10];
    if file.read_exact(&mut header).is_err() {
        return None;
    }
    if &header[0..3] == b"ID3" {
        let version = header[3];
        let flags = header[5];
        let tag_size = synchsafe(&header[6..10]).min(32 * 1024 * 1024);
        let mut data = vec![0u8; tag_size];
        file.read_exact(&mut data).ok()?;
        if flags & 0x80 != 0 {
            data = deunsync(&data);
        }
        let mut tags = TrackTags::default();
        let mut pos = 0usize;
        while pos + 10 <= data.len() {
            let id = &data[pos..pos + 4];
            if id.iter().all(|&b| b == 0) {
                break;
            }
            let frame_size = if version >= 4 {
                synchsafe(&data[pos + 4..pos + 8])
            } else {
                u32::from_be_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize
            };
            let body_start = pos + 10;
            if frame_size == 0 || body_start + frame_size > data.len() {
                break;
            }
            let body = &data[body_start..body_start + frame_size];
            match id {
                b"TIT2" => tags.title = decode_id3_text(body),
                b"TPE1" => tags.artist = decode_id3_text(body),
                b"TALB" => tags.album = decode_id3_text(body),
                _ => {}
            }
            pos = body_start + frame_size;
        }
        if !tags.title.trim().is_empty() {
            return Some(tags);
        }
    }
    if size >= 128 {
        file.seek(SeekFrom::End(-128)).ok()?;
        let mut tail = [0u8; 128];
        file.read_exact(&mut tail).ok()?;
        if &tail[0..3] == b"TAG" {
            let text = |s: &[u8]| {
                String::from_utf8_lossy(s)
                    .trim_matches('\0')
                    .trim()
                    .to_string()
            };
            let tags = TrackTags {
                title: text(&tail[3..33]),
                artist: text(&tail[33..63]),
                album: text(&tail[63..93]),
            };
            if !tags.title.is_empty() {
                return Some(tags);
            }
        }
    }
    None
}

fn deunsync(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        out.push(data[i]);
        if data[i] == 0xFF && i + 1 < data.len() && data[i + 1] == 0x00 {
            i += 1;
        }
        i += 1;
    }
    out
}

fn decode_id3_text(body: &[u8]) -> String {
    let Some((&encoding, text)) = body.split_first() else {
        return String::new();
    };
    let trim = |s: String| s.trim_matches('\0').trim().to_string();
    match encoding {
        0 => trim(text.iter().map(|&b| b as char).collect()),
        1 => {
            let mut s = String::new();
            if text.len() >= 2 {
                let le = text[0] == 0xFF && text[1] == 0xFE;
                let be = text[0] == 0xFE && text[1] == 0xFF;
                let start = if le || be { 2 } else { 0 };
                let mut units = Vec::new();
                let mut i = start;
                while i + 1 < text.len() {
                    let u = if le {
                        u16::from_le_bytes([text[i], text[i + 1]])
                    } else {
                        u16::from_be_bytes([text[i], text[i + 1]])
                    };
                    units.push(u);
                    i += 2;
                }
                s = String::from_utf16_lossy(&units);
            }
            trim(s)
        }
        2 => {
            let mut units = Vec::new();
            let mut i = 0;
            while i + 1 < text.len() {
                units.push(u16::from_be_bytes([text[i], text[i + 1]]));
                i += 2;
            }
            trim(String::from_utf16_lossy(&units))
        }
        _ => trim(String::from_utf8_lossy(text).into_owned()),
    }
}

fn read_flac_tags(path: &Path) -> Option<TrackTags> {
    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"fLaC" {
        return None;
    }
    let mut tags = TrackTags::default();
    loop {
        let mut header = [0u8; 4];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7f;
        let len = ((header[1] as usize) << 16) | ((header[2] as usize) << 8) | header[3] as usize;
        if kind == 4 {
            let mut data = vec![0u8; len];
            file.read_exact(&mut data).ok()?;
            let mut pos = 0usize;
            if data.len() < 8 {
                break;
            }
            let vendor = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
            pos += 4 + vendor;
            if pos + 4 > data.len() {
                break;
            }
            let count = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
            pos += 4;
            for _ in 0..count {
                if pos + 4 > data.len() {
                    break;
                }
                let clen = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
                pos += 4;
                if pos + clen > data.len() {
                    break;
                }
                let comment = String::from_utf8_lossy(&data[pos..pos + clen]);
                pos += clen;
                if let Some((key, value)) = comment.split_once('=') {
                    match key.to_ascii_uppercase().as_str() {
                        "TITLE" => tags.title = value.to_string(),
                        "ARTIST" => tags.artist = value.to_string(),
                        "ALBUM" => tags.album = value.to_string(),
                        _ => {}
                    }
                }
            }
        } else {
            file.seek(SeekFrom::Current(len as i64)).ok()?;
        }
        if last {
            break;
        }
    }
    Some(tags)
}

fn read_wav_tags(path: &Path) -> Option<TrackTags> {
    let mut file = File::open(path).ok()?;
    let mut riff = [0u8; 12];
    file.read_exact(&mut riff).ok()?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return None;
    }
    let mut tags = TrackTags::default();
    loop {
        let mut header = [0u8; 8];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let len = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
        if &header[0..4] == b"LIST" {
            let mut data = vec![0u8; len.min(1024 * 1024)];
            file.read_exact(&mut data).ok()?;
            let mut pos = 0usize;
            while pos + 8 <= data.len() {
                let id = &data[pos..pos + 4];
                let clen = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
                pos += 8;
                if pos + clen > data.len() {
                    break;
                }
                let value = String::from_utf8_lossy(&data[pos..pos + clen])
                    .trim_matches('\0')
                    .trim()
                    .to_string();
                match id {
                    b"INAM" => tags.title = value,
                    b"IART" => tags.artist = value,
                    b"IPRD" => tags.album = value,
                    _ => {}
                }
                pos += clen + (clen & 1);
            }
        } else {
            file.seek(SeekFrom::Current(len as i64 + (len as i64 & 1)))
                .ok()?;
        }
    }
    Some(tags)
}

fn read_mp4_tags(path: &Path) -> Option<TrackTags> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let ilst = find_mp4_atom(&mut file, 0, size, b"ilst")?;
    read_ilst_tags(&mut file, ilst.0, ilst.1)
}

fn find_mp4_atom(file: &mut File, start: u64, end: u64, target: &[u8; 4]) -> Option<(u64, u64)> {
    let mut pos = start;
    while pos + 8 <= end {
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let mut atom_size = u32::from_be_bytes(header[0..4].try_into().ok()?) as u64;
        let kind = &header[4..8];
        let header_len = if atom_size == 1 {
            let mut ext = [0u8; 8];
            file.read_exact(&mut ext).ok()?;
            atom_size = u64::from_be_bytes(ext);
            16
        } else {
            8
        };
        if atom_size == 0 {
            atom_size = end - pos;
        }
        if atom_size < header_len || pos + atom_size > end {
            return None;
        }
        if kind == target {
            return Some((pos + header_len, pos + atom_size));
        }
        if (kind == b"moov"
            || kind == b"udta"
            || kind == b"trak"
            || kind == b"mdia"
            || kind == b"minf"
            || kind == b"stbl")
            && let Some(found) = find_mp4_atom(file, pos + header_len, pos + atom_size, target)
        {
            return Some(found);
        }
        if kind == b"meta"
            && let Some(found) = find_mp4_atom(file, pos + header_len + 4, pos + atom_size, target)
        {
            return Some(found);
        }
        pos += atom_size;
    }
    None
}

fn read_ilst_tags(file: &mut File, start: u64, end: u64) -> Option<TrackTags> {
    let mut tags = TrackTags::default();
    let mut pos = start;
    while pos + 8 <= end {
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let atom_size = u32::from_be_bytes(header[0..4].try_into().ok()?) as u64;
        let kind = &header[4..8];
        if atom_size < 8 || pos + atom_size > end {
            break;
        }
        if kind == b"\xa9nam" || kind == b"\xa9ART" || kind == b"\xa9alb" {
            file.seek(SeekFrom::Start(pos + 8)).ok()?;
            let mut data_header = [0u8; 16];
            if file.read_exact(&mut data_header).is_ok() && &data_header[4..8] == b"data" {
                let data_size = u32::from_be_bytes(data_header[0..4].try_into().ok()?) as u64;
                if data_size >= 16 && data_size <= atom_size {
                    let mut payload = vec![0u8; (data_size - 16) as usize];
                    file.read_exact(&mut payload).ok()?;
                    let value = String::from_utf8_lossy(&payload).trim().to_string();
                    if kind == b"\xa9nam" {
                        tags.title = value;
                    } else if kind == b"\xa9ART" {
                        tags.artist = value;
                    } else {
                        tags.album = value;
                    }
                }
            }
        }
        pos += atom_size;
    }
    Some(tags)
}

#[derive(Debug, Default)]
pub struct ExifReport {
    pub images_scanned: u64,
    pub images_with_metadata: u64,
    pub rewritten: u64,
    pub bytes_saved: u64,
    pub skipped: u64,
    pub applied: bool,
}

pub fn strip_exif(cli: &Cli, cancel: &CancelToken) -> anyhow::Result<ExifReport> {
    let tree = walk_tree(cli, cancel);
    let applied = cli.yes && !cli.dry_run;
    let mut report = ExifReport {
        applied,
        ..ExifReport::default()
    };
    for file in &tree.files {
        if cancel.is_cancelled() {
            break;
        }
        let ext = file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_png = match ext.as_str() {
            "jpg" | "jpeg" | "jfif" => false,
            "png" => true,
            _ => continue,
        };
        report.images_scanned += 1;
        let data = match fs::read(&file.path) {
            Ok(d) => d,
            Err(_) => {
                report.skipped += 1;
                continue;
            }
        };
        let stripped = if is_png {
            strip_png_metadata(&data)
        } else {
            strip_jpeg_metadata(&data)
        };
        let Some(stripped) = stripped else {
            continue;
        };
        report.images_with_metadata += 1;
        let saved = data.len().saturating_sub(stripped.len()) as u64;
        if !applied {
            report.bytes_saved += saved;
            continue;
        }
        match rewrite_in_place(&file.path, &stripped) {
            Ok(()) => {
                report.rewritten += 1;
                report.bytes_saved += saved;
            }
            Err(_) => report.skipped += 1,
        }
    }
    Ok(report)
}

fn strip_jpeg_metadata(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..2]);
    let mut i = 2usize;
    let mut removed = false;
    while i < data.len() {
        if data[i] != 0xFF {
            out.extend_from_slice(&data[i..]);
            break;
        }
        if i + 1 >= data.len() {
            out.extend_from_slice(&data[i..]);
            break;
        }
        let marker = data[i + 1];
        if marker == 0xD9 || marker == 0xDA {
            out.extend_from_slice(&data[i..]);
            break;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            out.extend_from_slice(&data[i..i + 2]);
            i += 2;
            continue;
        }
        if marker == 0x00 {
            out.extend_from_slice(&data[i..]);
            break;
        }
        if i + 4 > data.len() {
            return None;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 {
            return None;
        }
        let end = i + 2 + len;
        if end > data.len() {
            return None;
        }
        let payload = &data[i + 4..end];
        let metadata = marker == 0xE1
            && (payload.starts_with(b"Exif\0\0")
                || payload.starts_with(b"http://ns.adobe.com/xap/")
                || payload.starts_with(b"http://ns.adobe.com/xmp/"));
        if metadata {
            removed = true;
        } else {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    if removed { Some(out) } else { None }
}

fn strip_png_metadata(data: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if !data.starts_with(&SIGNATURE) {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&SIGNATURE);
    let mut i = 8usize;
    let mut removed = false;
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let end = i + 12 + len;
        if end > data.len() {
            return None;
        }
        let kind = &data[i + 4..i + 8];
        let payload = &data[i + 8..i + 8 + len];
        let xmp = (kind == b"iTXt" || kind == b"tEXt") && payload.starts_with(b"XML:com.adobe.xmp");
        if kind == b"eXIf" || xmp {
            removed = true;
        } else {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    if i < data.len() {
        out.extend_from_slice(&data[i..]);
    }
    if removed { Some(out) } else { None }
}

fn rewrite_in_place(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let pid = std::process::id();
    let mut n = 0u32;
    let (temp, backup) = loop {
        let temp = parent.join(format!(".{name}.dedupe-tmp-{pid}-{n}"));
        let backup = parent.join(format!(".{name}.dedupe-bak-{pid}-{n}"));
        if !temp.exists() && !backup.exists() {
            break (temp, backup);
        }
        n += 1;
    };
    fs::write(&temp, data)?;
    if let Err(e) = fs::rename(path, &backup) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    match fs::rename(&temp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(e) => {
            let _ = fs::rename(&backup, path);
            let _ = fs::remove_file(&temp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dedupe-modes-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn cli_for(root: &Path, find: FindMode, extra: &[&str]) -> Cli {
        let mut args = vec![
            "dedupe".to_string(),
            "--find".to_string(),
            mode_name(find).to_string(),
        ];
        args.extend(extra.iter().map(|s| (*s).to_string()));
        args.push(root.to_string_lossy().into_owned());
        Cli::try_parse_from(args).expect("test CLI arguments")
    }

    #[test]
    fn run_mode_finds_empty_big_and_temp_files() {
        let root = temp_root("basic");
        fs::create_dir_all(root.join("nested").join("empty")).unwrap();
        fs::write(root.join("zero.txt"), b"").unwrap();
        fs::write(root.join("big.bin"), vec![b'x'; 64]).unwrap();
        fs::write(root.join("old.tmp"), b"tmp").unwrap();
        let cancel = CancelToken::new();

        let empty_files = run_mode(&cli_for(&root, FindMode::EmptyFiles, &[]), &cancel);
        assert_eq!(empty_files.findings.len(), 1);
        assert_eq!(empty_files.findings[0].path, root.join("zero.txt"));
        assert_eq!(empty_files.findings[0].kind, "empty");

        let empty_dirs = run_mode(&cli_for(&root, FindMode::EmptyDirs, &[]), &cancel);
        assert_eq!(empty_dirs.findings.len(), 1);
        assert_eq!(
            empty_dirs.findings[0].path,
            root.join("nested").join("empty")
        );
        assert!(empty_dirs.findings[0].is_dir);

        let big = run_mode(
            &cli_for(&root, FindMode::BigFiles, &["--top", "1"]),
            &cancel,
        );
        assert_eq!(big.findings.len(), 1);
        assert_eq!(big.findings[0].path, root.join("big.bin"));
        assert_eq!(big.findings[0].size, 64);

        let temp = run_mode(&cli_for(&root, FindMode::TempFiles, &[]), &cancel);
        assert_eq!(temp.findings.len(), 1);
        assert_eq!(temp.findings[0].path, root.join("old.tmp"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_mode_flags_wrong_extension_and_broken_image() {
        let root = temp_root("media");
        let valid = image::ImageBuffer::from_pixel(2, 2, image::Rgb([255u8, 0, 0]));
        valid.save(root.join("real.png")).unwrap();
        fs::copy(root.join("real.png"), root.join("wrong.jpg")).unwrap();
        fs::write(root.join("broken.png"), b"not an image").unwrap();
        let cancel = CancelToken::new();

        let bad = run_mode(&cli_for(&root, FindMode::BadExtensions, &[]), &cancel);
        assert_eq!(bad.findings.len(), 1);
        assert!(bad.findings[0].path.ends_with("wrong.jpg"));
        assert!(bad.findings[0].detail.contains("png"));

        let broken = run_mode(&cli_for(&root, FindMode::BrokenFiles, &[]), &cancel);
        assert_eq!(broken.findings.len(), 1);
        assert!(broken.findings[0].path.ends_with("broken.png"));

        let _ = fs::remove_dir_all(&root);
    }

    fn id3_tag(title: &str, artist: &str, album: &str) -> Vec<u8> {
        fn frame(id: &[u8; 4], text: &str) -> Vec<u8> {
            let mut frame = Vec::new();
            frame.extend_from_slice(id);
            frame.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
            frame.extend_from_slice(&[0, 0]);
            frame.push(0);
            frame.extend_from_slice(text.as_bytes());
            frame
        }
        let mut frames = frame(b"TIT2", title);
        frames.extend(frame(b"TPE1", artist));
        frames.extend(frame(b"TALB", album));
        let size = frames.len();
        let mut tag = vec![
            b'I',
            b'D',
            b'3',
            3,
            0,
            0,
            ((size >> 21) & 0x7f) as u8,
            ((size >> 14) & 0x7f) as u8,
            ((size >> 7) & 0x7f) as u8,
            (size & 0x7f) as u8,
        ];
        tag.extend(frames);
        tag
    }

    #[test]
    fn run_mode_groups_tracks_with_matching_tags() {
        let root = temp_root("music");
        let tag = id3_tag("Song", "Artist", "Album");
        let mut first = tag.clone();
        first.push(b'a');
        let mut second = tag;
        second.extend_from_slice(b"bc");
        fs::write(root.join("first.mp3"), first).unwrap();
        fs::write(root.join("second.mp3"), second).unwrap();

        let result = run_mode(
            &cli_for(&root, FindMode::Music, &["--music-by", "tags"]),
            &CancelToken::new(),
        );
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].members.len(), 2);
        assert_eq!(result.groups[0].label, "Artist \u{2014} Song");
        assert!(result.reclaimable() > 0);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn strip_exif_only_rewrites_when_applied() {
        let root = temp_root("exif");
        let mut jpeg = vec![0xFF, 0xD8];
        let payload = b"Exif\0\0MM\0*";
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(payload);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&4u32.to_be_bytes());
        png.extend_from_slice(b"eXIf");
        png.extend_from_slice(b"Exif");
        png.extend_from_slice(&[0, 0, 0, 0]);
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IEND");
        png.extend_from_slice(&[0, 0, 0, 0]);
        let plain = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9];
        fs::write(root.join("photo.jpg"), &jpeg).unwrap();
        fs::write(root.join("image.png"), &png).unwrap();
        fs::write(root.join("plain.jpg"), plain).unwrap();

        let dry = strip_exif(
            &cli_for(&root, FindMode::Duplicates, &["--strip-exif"]),
            &CancelToken::new(),
        )
        .unwrap();
        assert!(!dry.applied);
        assert_eq!(dry.images_scanned, 3);
        assert_eq!(dry.images_with_metadata, 2);
        assert_eq!(dry.rewritten, 0);
        assert!(dry.bytes_saved > 0);
        assert_eq!(fs::read(root.join("photo.jpg")).unwrap(), jpeg);

        let applied = strip_exif(
            &cli_for(&root, FindMode::Duplicates, &["--strip-exif", "--yes"]),
            &CancelToken::new(),
        )
        .unwrap();
        assert!(applied.applied);
        assert_eq!(applied.images_scanned, 3);
        assert_eq!(applied.images_with_metadata, 2);
        assert_eq!(applied.rewritten, 2);
        let stripped_jpeg = fs::read(root.join("photo.jpg")).unwrap();
        let stripped_png = fs::read(root.join("image.png")).unwrap();
        assert!(!stripped_jpeg.windows(4).any(|w| w == b"Exif"));
        assert!(!stripped_png.windows(4).any(|w| w == b"eXIf"));
        assert_eq!(fs::read(root.join("plain.jpg")).unwrap(), plain);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_mode_groups_identical_folder_trees() {
        let root = temp_root("folders");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a").join("photo.jpg"), b"same").unwrap();
        fs::write(root.join("b").join("photo.jpg"), b"same").unwrap();

        let result = run_mode(
            &cli_for(&root, FindMode::DuplicateFolders, &[]),
            &CancelToken::new(),
        );
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].members.len(), 2);
        assert_eq!(result.reclaimable(), 4);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn glob_matches_names_and_paths() {
        assert!(glob_match("*.log", Path::new("/tmp/app.log")));
        assert!(!glob_match("*.log", Path::new("/tmp/app.txt")));
        assert!(glob_match(
            "**/node_modules/**",
            Path::new("/tmp/node_modules/x.js")
        ));
        assert!(!glob_match("tmp/*.js", Path::new("/tmp/a/x.js")));
        assert!(glob_match("tmp/*.js", Path::new("/tmp/x.js")));
    }

    #[test]
    fn name_keys_ignore_copy_suffixes() {
        assert_eq!(name_key(Path::new("Holiday.JPG")), "holiday");
        assert_eq!(name_key(Path::new("Holiday - Copy (2).jpg")), "holiday");
        assert_eq!(name_key(Path::new("my_file-1.png")), "my file 1");
    }

    #[test]
    fn fuzzy_similarity_is_one_for_equal_names() {
        assert!((name_similarity("holiday photo", "holiday photo") - 1.0).abs() < 1e-9);
        assert!(name_similarity("holiday photo", "holiday pictures") < 0.8);
    }

    #[test]
    fn temp_detection_covers_common_patterns() {
        assert!(temp_reason(Path::new("a.tmp")).is_some());
        assert!(temp_reason(Path::new("~$report.docx")).is_some());
        assert!(temp_reason(Path::new("backup.txt~")).is_some());
        assert!(temp_reason(Path::new("normal.txt")).is_none());
    }

    #[test]
    fn bad_names_find_reserved_and_invalid() {
        assert!(bad_name_reason("CON.txt").is_some());
        assert!(bad_name_reason("bad:name.txt").is_some());
        assert!(bad_name_reason("trailing .").is_some());
        assert!(bad_name_reason("fine.txt").is_none());
    }

    #[test]
    fn tag_normalization_is_case_and_punctuation_insensitive() {
        assert_eq!(normalize_tag("  Hello, World! "), "hello world");
        assert_eq!(normalize_tag("Track 01"), "track 01");
    }

    #[test]
    fn jpeg_exif_app1_is_removed() {
        let mut data = vec![0xFF, 0xD8];
        let payload = b"Exif\0\0MM\0*";
        let len = (payload.len() + 2) as u16;
        data.extend_from_slice(&[0xFF, 0xE1]);
        data.extend_from_slice(&len.to_be_bytes());
        data.extend_from_slice(payload);
        data.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
        let out = strip_jpeg_metadata(&data).expect("metadata found");
        assert!(!out.windows(4).any(|w| w == b"Exif"));
        assert_eq!(&out[..2], &[0xFF, 0xD8]);
        assert_eq!(&out[out.len() - 2..], &[0xFF, 0xD9]);
    }

    #[test]
    fn jpeg_without_metadata_is_untouched() {
        let data = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9];
        assert!(strip_jpeg_metadata(&data).is_none());
    }

    #[test]
    fn png_exif_chunk_is_removed() {
        let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let payload = b"Exif";
        data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        data.extend_from_slice(b"eXIf");
        data.extend_from_slice(payload);
        data.extend_from_slice(&[0, 0, 0, 0]);
        data.extend_from_slice(&[0, 0, 0, 0]);
        data.extend_from_slice(b"IEND");
        data.extend_from_slice(&[0, 0, 0, 0]);
        let out = strip_png_metadata(&data).expect("metadata found");
        assert!(!out.windows(4).any(|w| w == b"eXIf"));
        assert!(out.windows(4).any(|w| w == b"IEND"));
    }
}
