//! Pure UI helpers: text capping, hash captions, selector-to-CLI mapping,
//! and size-limit parsing.
//!
//! No view state lives here, only functions.

use crate::cli;
use crate::util;

/// Read (position, length) off a headless progress bar.
pub(crate) fn bar_pos(bar: &indicatif::ProgressBar) -> (u64, u64) {
    (bar.position(), bar.length().unwrap_or(0))
}

/// Cap a display string, keeping the tail — filenames live at the end of
/// paths, so a truncated middle would hide the useful part. Paired with
/// `.truncate()` (ellipsis) on the element for pixel-level clipping.
pub(crate) fn fit_text(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    format!(
        "…{}",
        s.chars().skip(count - max_chars + 1).collect::<String>()
    )
}

/// One-line explanation for each hash algorithm, shown under the selector.
pub(crate) fn hash_caption(index: Option<usize>) -> &'static str {
    match index.unwrap_or(0) {
        1 => "SHA-256 — widely supported standard; pick it to match other tools.",
        2 => "MD5 — legacy and collision-prone; only to match old hashes.",
        _ => "BLAKE3 — fastest modern hash; the right default for scans.",
    }
}

/// Map the scan-type selector onto the CLI `--find` enum.
pub(crate) fn find_mode(index: Option<usize>) -> cli::FindMode {
    match index.unwrap_or(0) {
        1 => cli::FindMode::EmptyFiles,
        2 => cli::FindMode::EmptyDirs,
        3 => cli::FindMode::BigFiles,
        4 => cli::FindMode::TempFiles,
        5 => cli::FindMode::InvalidSymlinks,
        6 => cli::FindMode::BrokenFiles,
        7 => cli::FindMode::BadExtensions,
        8 => cli::FindMode::BadNames,
        9 => cli::FindMode::DuplicateFolders,
        10 => cli::FindMode::Music,
        _ => cli::FindMode::Duplicates,
    }
}

/// Map the duplicate-matching selector onto the CLI `--match` enum.
pub(crate) fn match_mode(index: Option<usize>) -> cli::MatchMode {
    match index.unwrap_or(0) {
        1 => cli::MatchMode::Size,
        2 => cli::MatchMode::Name,
        3 => cli::MatchMode::Fuzzy,
        _ => cli::MatchMode::Content,
    }
}

/// Map the image-fingerprint selector onto the CLI `--image-hash` enum.
pub(crate) fn image_hash_algo(index: Option<usize>) -> cli::ImageHashAlgo {
    match index.unwrap_or(0) {
        1 => cli::ImageHashAlgo::Dhash,
        2 => cli::ImageHashAlgo::Phash,
        3 => cli::ImageHashAlgo::Ahash,
        4 => cli::ImageHashAlgo::Color,
        _ => cli::ImageHashAlgo::Auto,
    }
}

/// Caption under the scan-type selector so each cleanup mode explains itself.
pub(crate) fn find_mode_caption(index: Option<usize>) -> &'static str {
    match find_mode(index) {
        cli::FindMode::Duplicates => {
            "Exact and similar duplicates. Uses the matching options below."
        }
        cli::FindMode::EmptyFiles => "Files with no content (0 bytes).",
        cli::FindMode::EmptyDirs => "Folders that contain nothing.",
        cli::FindMode::BigFiles => "The largest files in scope, biggest first.",
        cli::FindMode::TempFiles => "Temporary, backup, and editor leftover files.",
        cli::FindMode::InvalidSymlinks => "Symlinks whose target is missing.",
        cli::FindMode::BrokenFiles => "Images or media that cannot be decoded.",
        cli::FindMode::BadExtensions => "Files whose magic bytes disagree with the extension.",
        cli::FindMode::BadNames => "Reserved names, invalid characters, trailing dots or spaces.",
        cli::FindMode::DuplicateFolders => "Identical folder trees by content signature.",
        cli::FindMode::Music => "Duplicate tracks by embedded tags or audio fingerprint.",
    }
}

/// Display text for "no size limit". The boxes never look empty-broken:
/// an unbounded end reads `unlimited`, and the same word (case-insensitive)
/// is accepted as input wherever a size is read.
pub(crate) const NO_LIMIT_LABEL: &str = "unlimited";

/// True when a size box means "no limit": blank (legacy) or `unlimited`.
pub(crate) fn is_no_limit(raw: &str) -> bool {
    let raw = raw.trim();
    raw.is_empty() || raw.eq_ignore_ascii_case(NO_LIMIT_LABEL)
}

/// Parse a size box into an optional byte bound. Blank or `unlimited`
/// means unbounded (`None`); anything else goes through [`util::parse_size`].
pub(crate) fn parse_size_limit(raw: &str) -> anyhow::Result<Option<u64>> {
    if is_no_limit(raw) {
        return Ok(None);
    }
    util::parse_size(raw.trim()).map(Some)
}

/// Absolute path from an OS file-drop URI (`file:///C:/a%20b.png`), or
/// `None` for anything that is not a local `file:` URL. Drops carry
/// `file:` URLs, not paths; `actions::drop_folders` needs the plain path.
pub(crate) fn file_url_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // A local drop's host is `localhost`; anything else is a network
    // share, not a scan root here.
    let rest = rest.strip_prefix("localhost/").unwrap_or(rest);
    // Fragments and queries are URL parts, never part of the path.
    let rest = rest.split(['#', '?']).next()?;
    let decoded = percent_decode(rest)?;
    let path = decoded.strip_suffix('/').unwrap_or(&decoded);
    // Windows drive drops arrive as `file:///C:/...`: lose the extra
    // leading slash so the path is in native `Path` form.
    if cfg!(target_os = "windows")
        && path.len() >= 3
        && path.as_bytes()[0] == b'/'
        && path.as_bytes()[1].is_ascii_alphabetic()
        && path.as_bytes()[2] == b':'
    {
        Some(path[1..].to_string())
    } else {
        Some(path.to_string())
    }
}

/// Percent-decode (`%C3%BC` → `ü`). `None` when an escape is malformed or
/// the bytes are not UTF-8: junk URLs must never become junk paths.
pub(crate) fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let byte = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_text_keeps_short_strings_and_truncates_tails() {
        assert_eq!(fit_text("abc", 10), "abc");
        assert_eq!(fit_text("abcdef", 6), "abcdef");
        // Long paths keep the filename tail, prefixed with an ellipsis.
        let capped = fit_text("C:\\very\\long\\directory\\file.txt", 12);
        assert_eq!(capped, "…ry\\file.txt");
        assert_eq!(capped.chars().count(), 12);
    }

    #[test]
    fn size_limit_sentinel_parses() {
        assert_eq!(parse_size_limit("").unwrap(), None);
        assert_eq!(parse_size_limit("unlimited").unwrap(), None);
        assert_eq!(parse_size_limit("Unlimited").unwrap(), None);
        assert_eq!(parse_size_limit("  UNLIMITED  ").unwrap(), None);
        assert_eq!(parse_size_limit("100KB").unwrap(), Some(100_000));
        assert!(parse_size_limit("abc").is_err());
    }

    #[test]
    fn percent_decode_only_accepts_well_formed_utf8() {
        assert_eq!(percent_decode("a%20b"), Some("a b".to_string()));
        assert_eq!(percent_decode("m%C3%BCller"), Some("müller".to_string()));
        assert_eq!(percent_decode("plain"), Some("plain".to_string()));
        assert_eq!(percent_decode("bad%zz"), None);
        assert_eq!(percent_decode("short%C3"), None);
        assert_eq!(percent_decode("%FF%FE"), None);
    }

    #[test]
    fn file_urls_decode_to_paths() {
        // Windows drive drop straight from Explorer.
        if cfg!(target_os = "windows") {
            assert_eq!(
                file_url_path("file:///C:/media/albums"),
                Some("C:/media/albums".to_string())
            );
            assert_eq!(
                file_url_path("file:///C:/m%C3%BCller/"),
                Some("C:/müller".to_string())
            );
        }
        // Trailing separators, fragments and queries are not the path.
        assert_eq!(
            file_url_path("file:///home/al/dups/#frag"),
            Some("/home/al/dups".to_string())
        );
        assert_eq!(
            file_url_path("file:///home/al/a?b"),
            Some("/home/al/a".to_string())
        );
        // Loose strings and other schemes are something else entirely.
        assert_eq!(file_url_path("C:\\media"), None);
        assert_eq!(file_url_path("https://example.com/x"), None);
        // Malformed percent escapes are junk, never paths.
        assert_eq!(file_url_path("file:///a%zz"), None);
        assert_eq!(file_url_path("file:///a%C3"), None);
        assert_eq!(file_url_path("file:///a%FF"), None);
    }

    #[test]
    fn hash_caption_names_all_three_algorithms() {
        assert!(hash_caption(Some(0)).contains("BLAKE3"));
        assert!(hash_caption(None).contains("BLAKE3"));
        assert!(hash_caption(Some(1)).contains("SHA-256"));
        assert!(hash_caption(Some(2)).contains("MD5"));
    }

    #[test]
    fn selector_indices_map_to_cli_enums() {
        assert_eq!(find_mode(None), cli::FindMode::Duplicates);
        assert_eq!(find_mode(Some(1)), cli::FindMode::EmptyFiles);
        assert_eq!(find_mode(Some(4)), cli::FindMode::TempFiles);
        assert_eq!(find_mode(Some(9)), cli::FindMode::DuplicateFolders);
        assert_eq!(find_mode(Some(10)), cli::FindMode::Music);
        assert_eq!(find_mode(Some(99)), cli::FindMode::Duplicates);

        assert_eq!(match_mode(None), cli::MatchMode::Content);
        assert_eq!(match_mode(Some(1)), cli::MatchMode::Size);
        assert_eq!(match_mode(Some(2)), cli::MatchMode::Name);
        assert_eq!(match_mode(Some(3)), cli::MatchMode::Fuzzy);
        assert_eq!(match_mode(Some(99)), cli::MatchMode::Content);

        assert_eq!(image_hash_algo(None), cli::ImageHashAlgo::Auto);
        assert_eq!(image_hash_algo(Some(1)), cli::ImageHashAlgo::Dhash);
        assert_eq!(image_hash_algo(Some(2)), cli::ImageHashAlgo::Phash);
        assert_eq!(image_hash_algo(Some(3)), cli::ImageHashAlgo::Ahash);
        assert_eq!(image_hash_algo(Some(4)), cli::ImageHashAlgo::Color);
        assert_eq!(image_hash_algo(Some(99)), cli::ImageHashAlgo::Auto);
    }

    #[test]
    fn find_mode_captions_cover_every_selector_index() {
        for index in 0..=10 {
            assert!(!find_mode_caption(Some(index)).is_empty());
        }
    }
}
