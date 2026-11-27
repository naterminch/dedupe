//! How much a path looks like a copy of another file: the keep rule's
//! tiebreak. When the rule's own measure cannot tell two copies apart, the
//! one whose name and folder look like the original stays — `photo.jpg`
//! over `photo (1).jpg`, `Pictures\a.jpg` over `Downloads\a.jpg`.
//!
//! Pure string work on the paths as given: no file system access, the same
//! answer on every OS (both `/` and `\` separate folders, case is ignored).

use std::path::Path;

/// The word a copy's name carries, in the languages file managers name
/// copies in: English, German/Dutch, French, Spanish/Italian/Portuguese.
const COPY_WORDS: &[&str] = &["copy", "kopie", "copie", "copia"];

/// Folders whose files are usually a second copy of something kept
/// elsewhere: downloads, temporary and cache folders, the trash.
const COPY_FOLDERS: &[&str] = &[
    "download",
    "downloads",
    "temp",
    "tmp",
    "cache",
    ".cache",
    "caches",
    "trash",
    ".trash",
    ".trashes",
    "$recycle.bin",
    "recycler",
    "recycle bin",
];

/// Folder name beginnings with the same meaning: messenger media folders
/// (`WhatsApp Images`, `Telegram Desktop`, …) and per-user Linux trash
/// folders (`.Trash-1000`).
const COPY_FOLDER_PREFIXES: &[&str] = &["whatsapp", "telegram", ".trash-"];

/// One score per path, in order: how many copy signals it carries (0, 1 or
/// 2). Lower looks more like the original.
///
/// - its name: a ` (1)` / `(2)` counter, a `copy` word as a suffix
///   (`photo - Copy`, `photo copy 2`, `photo_copy`) or a prefix
///   (`Copy of photo`), or a bare counter (`photo-1`, `photo_2`,
///   `photo 3`) when another path in the set has the name without it;
/// - its folders: any one of them a download, temp, cache, trash or
///   messenger media folder.
#[must_use]
pub(crate) fn copy_scores(paths: &[&Path]) -> Vec<u8> {
    let stems: Vec<String> = paths.iter().map(|p| stem(p)).collect();
    paths
        .iter()
        .zip(&stems)
        .map(|(path, stem)| {
            let counted = strip_counter(stem).is_some_and(|base| stems.iter().any(|s| s == base));
            u8::from(has_copy_marker(stem) || counted) + u8::from(in_copy_folder(path))
        })
        .collect()
}

/// The path split into its parts on either separator, so Windows paths
/// read the same on every host.
fn parts(path: &Path) -> Vec<String> {
    path.to_string_lossy()
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The file name without its last extension, lowercased. A leading dot is
/// part of the name (`.bashrc` has no extension).
fn stem(path: &Path) -> String {
    let name = parts(path).pop().unwrap_or_default();
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].to_string(),
        _ => name,
    }
}

fn in_copy_folder(path: &Path) -> bool {
    let mut parts = parts(path);
    // The last part is the file itself, not a folder.
    parts.pop();
    parts.iter().any(|part| {
        COPY_FOLDERS.contains(&part.as_str())
            || COPY_FOLDER_PREFIXES
                .iter()
                .any(|prefix| part.starts_with(prefix))
    })
}

/// A name that says "copy" whatever else is in the folder.
fn has_copy_marker(stem: &str) -> bool {
    let stem = stem.trim_end();
    if strip_paren_counter(stem).is_some() {
        return true;
    }
    // macOS numbers further copies after the word: `photo copy 2`.
    let words_end = strip_trailing_number(stem, &[' ']).unwrap_or(stem);
    COPY_WORDS.iter().any(|word| {
        let suffix = words_end
            .strip_suffix(word)
            .and_then(|rest| rest.strip_suffix([' ', '_', '-', '.']))
            .is_some_and(|base| !base.trim_end_matches([' ', '-']).is_empty());
        // `Copy of photo`, and the older Windows `Copy (2) of photo`.
        let prefix = stem.strip_prefix(word).is_some_and(|rest| {
            let rest = rest.trim_start();
            let rest = rest
                .strip_prefix('(')
                .and_then(|r| r.split_once(')'))
                .filter(|(n, _)| is_counter(n))
                .map_or(rest, |(_, r)| r.trim_start());
            rest.strip_prefix("of ")
                .is_some_and(|name| !name.is_empty())
        });
        suffix || prefix
    })
}

/// `photo (1)` or `photo(1)`: the name before the counter.
fn strip_paren_counter(stem: &str) -> Option<&str> {
    let inner = stem.strip_suffix(')')?;
    let open = inner.rfind('(')?;
    is_counter(&inner[open + 1..])
        .then(|| inner[..open].trim_end())
        .filter(|base| !base.is_empty())
}

/// `photo-1`, `photo_2`, `photo 3`: the name before a bare counter.
fn strip_counter(stem: &str) -> Option<&str> {
    strip_trailing_number(stem, &['-', '_', ' '])
}

fn strip_trailing_number<'a>(stem: &'a str, separators: &[char]) -> Option<&'a str> {
    let base = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    if !is_counter(&stem[base.len()..]) {
        return None;
    }
    base.strip_suffix(separators)
        .map(str::trim_end)
        .filter(|base| !base.is_empty())
}

/// One to three digits: a file manager's copy counter, not a date or a
/// camera's frame number.
fn is_counter(s: &str) -> bool {
    (1..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(paths: &[&str]) -> Vec<u8> {
        let paths: Vec<&Path> = paths.iter().map(Path::new).collect();
        copy_scores(&paths)
    }

    fn one(path: &str) -> u8 {
        scores(&[path])[0]
    }

    #[test]
    fn copy_names_score_on_their_own() {
        for name in [
            "/p/photo (1).jpg",
            "/p/photo(2).jpg",
            "/p/photo - Copy.jpg",
            "/p/photo - Copy (3).jpg",
            "/p/photo copy.jpg",
            "/p/photo copy 2.jpg",
            "/p/photo_copy.jpg",
            "/p/photo-COPY.jpg",
            "/p/Copy of photo.jpg",
            "/p/Copy (2) of photo.jpg",
            "/p/Foto - Kopie.jpg",
            "/p/photo - copie.jpg",
            "/p/foto - copia.jpg",
        ] {
            assert_eq!(one(name), 1, "{name}");
        }
    }

    #[test]
    fn ordinary_names_do_not_look_like_copies() {
        for name in [
            "/p/photo.jpg",
            "/p/photocopy.jpg",
            "/p/copy.txt",
            "/p/copyright.txt",
            "/p/Copy of.txt",
            "/p/IMG_0001.jpg",
            "/p/report 2024.pdf",
            "/p/scan (draft).pdf",
            "/p/(1).jpg",
            "/p/.bashrc",
        ] {
            assert_eq!(one(name), 0, "{name}");
        }
    }

    #[test]
    fn bare_counters_need_a_sibling_without_them() {
        // Alone, `holiday-1` is just a name.
        assert_eq!(scores(&["/a/holiday-1.jpg", "/b/other.jpg"]), vec![0, 0]);
        // Beside `holiday`, it is the second copy — in any folder, with any
        // extension (a similar group can mix formats).
        assert_eq!(scores(&["/a/holiday-1.jpg", "/b/Holiday.png"]), vec![1, 0]);
        assert_eq!(scores(&["/a/holiday_2.jpg", "/a/holiday.jpg"]), vec![1, 0]);
        assert_eq!(scores(&["/a/holiday 3.jpg", "/a/holiday.jpg"]), vec![1, 0]);
        // A camera's frame number is not a counter.
        assert_eq!(scores(&["/a/IMG_0001.jpg", "/a/IMG.jpg"]), vec![0, 0]);
    }

    #[test]
    fn copy_folders_score_wherever_they_sit() {
        for path in [
            "C:\\Users\\me\\Downloads\\a.jpg",
            "/home/me/Download/a.jpg",
            "C:\\Users\\me\\AppData\\Local\\Temp\\x\\a.jpg",
            "/tmp/a.jpg",
            "/home/me/.cache/thumbs/a.jpg",
            "/Users/me/Library/Caches/a.jpg",
            "/Users/me/.Trash/a.jpg",
            "/home/me/.local/share/Trash/files/a.jpg",
            "/media/usb/.Trash-1000/files/a.jpg",
            "D:\\$RECYCLE.BIN\\S-1-5\\a.jpg",
            "/sdcard/WhatsApp/Media/WhatsApp Images/a.jpg",
            "C:\\Users\\me\\Downloads\\Telegram Desktop\\a.jpg",
        ] {
            assert!(one(path) >= 1, "{path}");
        }
        // Only folders count: a file named like one does not.
        assert_eq!(one("/p/downloads.txt"), 0);
        assert_eq!(one("/p/temperature/a.jpg"), 0);
    }

    #[test]
    fn signals_add_up() {
        // An original in Temp still beats its `(1)` twin in Temp.
        assert_eq!(
            scores(&["/tmp/photo.jpg", "/tmp/photo (1).jpg"]),
            vec![1, 2]
        );
    }
}
