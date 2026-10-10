# dedupe

Finds duplicate files — exact copies and similar-looking photos/videos.

- **Exact duplicates** — same bytes. Copies, renames, backups.
- **Similar duplicates** (on by default) — same picture, different format
  (`.png` vs `.jpg`, `.mp4` vs `.mov`). Disable with `--exact`.

One binary, no install. Windows, macOS, Linux.

## Screenshots

| Light | Dark | Delete confirm |
| --- | --- | --- |
| ![GUI in light theme](docs/gui-light.png) | ![GUI in dark theme](docs/gui-dark.png) | ![Delete confirmation dialog](docs/gui-delete-confirm.png) |

## Features

### Exact duplicate detection

- Size → partial-hash → full-hash pipeline (jdupes-style). Only files sharing
  a size are ever hashed; only files sharing a partial hash are fully hashed.
- Partial hash is head (32 KB) + tail (32 KB) in one digest, so files sharing
  only a header separate early. Files ≤ 64 KB are hashed whole.
- Hash algorithm selectable: `blake3` (default), `sha256`, `md5`.
- Hard links count once (same inode = not two copies, deleting one frees
  nothing). Reported as `hardlinks_skipped`.
- Parallel hashing with rayon; `--jobs N` caps threads (`0` = auto:
  all-but-one core). Animated progress bar in the CLI.

### Similar media detection (perceptual, on by default)

- **Images:** 256-bit difference hash (dHash) over a 17×16 grayscale
  thumbnail. Robust to re-encoding, format changes, and slight exposure
  shifts. Same `.png` re-saved as `.jpg` matches at ≥ 97%.
- **Videos:** 8 evenly-spaced frames sampled with `ffmpeg` (one fast decode
  pass for clips ≤ 90 s, `-ss` seeks for longer ones), compared as frame
  sequences with ±2-frame tolerance, symmetrically (a static clip can't claim
  100% against a varied one just because one frame matches).
- Threshold tunable: `--similarity PCT` (0–100, default `97`). `--exact`
  disables the whole pass.
- Pre-filtered before any decode: images bucketed by aspect ratio via
  header-only reads, videos by probed resolution + duration (within 2 s or
  5%). Only files with a possible partner are fingerprinted.
- Deterministic star clustering: first file (path order) becomes the keeper,
  later files join the first keeper they match — unrelated files can never
  chain into one group through a middleman.
- For `--keep-smaller` on similar groups, the **highest-resolution** version
  is kept (a smaller file is usually the re-encoded copy), falling back to
  smallest size when resolution is unknown.

### Safe deletion

- Every file is **re-hashed immediately before deletion**. Changed since the
  scan → skipped with a warning, never removed. Similar-group members are
  verified against their own per-file content hash (their bytes legitimately
  differ from each other).
- `--trash` moves to the system trash (recoverable); default is permanent.
  GUI defaults to trash.
- `--dry-run` lists what would go without touching anything.
- Interactive per-group prompts (`y` / `n` / `a` = all remaining / `q` = quit)
  unless `-y` / `--yes` is given.
- `--reference-dir PATH` (repeatable): files under these folders are never
  deleted and always win the keep decision. Protect `/backup` while cleaning
  `/incoming`.
- Keeper choice: first path (default), `--keep-smaller`, `--keep-newest`,
  `--keep-oldest`. Flags conflict — pick one. The GUI can re-apply the keep
  rule without rescanning.

### Scope and filtering

- Scan files or directories, one or many paths: `dedupe a.txt /media /photos`.
- `--types jpg,png,mp4` (comma-separated, case-insensitive, dot optional).
- `--min-size` / `--max-size` with human sizes (`100KB`, `1.5MB`, `3GiB`,
  `4G`; `0` disables).
- `--max-depth N` (`0` = top-level files only).
- `--exclude-dir NAME` (repeatable, substring on directory name, prunes the
  subtree — e.g. `node_modules`) and `--exclude-path SUBSTR` (repeatable,
  substring on full path).
- Unreadable paths don't abort the scan; they're collected and reported.

### Reports

- Human report: `✓ KEEP` stays, `✗ DUP` goes, `◈ REF` = protected reference.
  Per group: kind (`FILE`/`IMAGE`/`VIDEO`), file count, size each,
  reclaimable bytes, content-hash prefix or `xx.x% similar`, plus keeper
  resolution/duration. `--verbose` adds per-file codec metadata.
- `--json` emits machine-readable JSON (same shape the GUI's Save button
  writes): counts, `reclaimable_bytes`, `ffprobe_used`, per-group members
  with sizes, similarity, and media info.
- Colors only when stdout is a terminal and `NO_COLOR` is unset — piped
  output stays plain. `--quiet` suppresses progress/status lines.

### Media metadata (optional FFmpeg)

- `ffprobe` provides resolution / duration / codec per group when available;
  `ffmpeg` enables video similarity sampling. Both are optional — without
  them, exact dedup and image similarity still work, and the CLI tells you
  what was skipped.
- Supported by extension: 14 image types (`jpg/jpeg/jfif/png/gif/bmp/webp/
  tiff/tif/heic/heif/avif/svg/ico`), 16 video types (`mp4/mkv/avi/mov/webm/
  m4v/flv/wmv/mpg/mpeg/ts/m2ts/3gp/ogv/rmvb/vob`).

### Caching (fast repeat scans)

- Two persistent caches in `~/.dedupe/` (`%USERPROFILE%\.dedupe\` on Windows),
  overridable with `DEDUPE_CACHE`: `hashes.bin` (content hashes, only files
  ≥ 256 KB) and `fingerprints.bin` (perceptual fingerprints).
- Keyed by path + size + mtime (+ algorithm for hashes). Unchanged files skip
  re-hashing and re-decoding entirely. Stale entries miss for free; the file
  is capped at 50k entries and pruned at save time.
- Versioned binary format — corrupt or mismatched files become an empty
  cache. Writes are atomic (temp + rename) and best-effort: a failed save
  warns but never fails the scan. `--no-cache` bypasses both.

### GUI

```sh
dedupe-gui     # desktop app, no terminal
dedupe         # desktop app + terminal
dedupe --gui   # same, explicit
```

- Same pipeline as the CLI, so results are identical — only presentation
  differs. Opens automatically when no scan path is given.
- Pick folders, press **Scan**, tick duplicates, **Delete selected**. Per-folder
  **Ref** toggles protect backups; **Keep** switches the keeper
  (first / smallest / newest / oldest) without rescanning; deletion goes to
  trash by default.
- Image thumbnails inline; videos show cached poster strips (4 frames via
  `ffmpeg`, reused from the temp dir while size+mtime match). Results paged
  at 100 groups with sort-by-biggest; report exportable as JSON.
- Light/dark theme plus folders and options persist to
  `~/.dedupe/gui-prefs.json` (`DEDUPE_PREFS` overrides). Native folder/save
  dialogs, no console flashing for helper processes (they run
  below-normal priority on Windows).

> GUI builds need VS 2022 C++ workload on Windows
> ([details](https://gpui-kit.com/docs/installation)).

## CLI

```sh
dedupe <path>...            # scan (e.g. dedupe /media, dedupe .)
dedupe /media --delete      # delete duplicates (prompts per group)
dedupe /media --json        # scriptable output
```

Useful flags: `--types jpg,png,mp4` · `--keep-smaller` /
`--keep-newest` / `--keep-oldest` · `--reference-dir` (protect a folder) ·
`--trash` · `--dry-run` · `--exact` · `--no-cache` · `--jobs 2` (limit
worker threads when scans hog the machine) · `-y` (no prompts).
Full list: `dedupe --help`.

```sh
# Safe cleanup: protect /backup, trash the rest, no prompts
dedupe /incoming --reference-dir /backup --delete --yes --trash
```

`✓ KEEP` stays, `✗ DUP` goes. Files are re-hashed before removal, so a
file that changed since the scan is never deleted.

Full flag reference (`dedupe --help` is authoritative):

| Flag | Effect |
| --- | --- |
| `<PATH>...` | Files/dirs to scan; omit to open the GUI |
| `--gui` | Open the GUI instead of scanning |
| `--max-depth N` | Limit recursion (`0` = top level only) |
| `-t, --types EXTS` | Only these extensions |
| `--keep-smaller` / `--keep-newest` / `--keep-oldest` | Keeper rule (default: first path) |
| `--reference-dir PATH` | Protect a tree (repeatable) |
| `-D, --delete` | Delete non-keepers (prompts per group) |
| `-y, --yes` | Answer yes to all prompts |
| `--trash` / `--dry-run` | Trash instead of permanent / simulate |
| `--hash blake3\|sha256\|md5` | Content hash (default `blake3`) |
| `--exact` | Byte-identical only; skip perceptual pass |
| `--similarity PCT` | Similar-media threshold (default `97`) |
| `--no-cache` | Recompute hashes + fingerprints from scratch |
| `--min-size SIZE` / `--max-size SIZE` | Size window (`100KB`, `1GiB`, …) |
| `--exclude-dir NAME` / `--exclude-path SUBSTR` | Skip matches (repeatable) |
| `-j, --json` | JSON report |
| `-v, --verbose` | Per-file resolution/duration/codec |
| `-q, --quiet` | No progress output |
| `--jobs N` | Worker threads (`0` = auto) |

## Build

```sh
.\build.ps1            # Windows: release -> dedupe.exe + dedupe-gui.exe
cargo build --release  # any OS
```

Needs the Rust toolchain. Optional: FFmpeg (media details + video
comparison); without it, everything else still works.

- `dedupe.exe` = console binary (CLI; opens the GUI when run without a path).
- `dedupe-gui.exe` = GUI-only binary (no console window ever).

## How it works

One shared pipeline (`pipeline::run_scan`) drives both the CLI and the GUI
through four phases: **Scanning → Hashing → Comparing media → Building report**.

1. **Walk + filter (`scan`).** Recursively collect files under each path
   (`--max-depth` caps depth). Directly-given files are included as-is.
   Apply `--types`, `--exclude-dir` / `--exclude-path`, and the
   `--min-size` / `--max-size` window. Missing/unreadable paths are recorded,
   not fatal. Each survivor gets path + size + mtime.
2. **Group by size.** Files with a unique size can't be exact duplicates —
   dropped immediately. Within same-size groups, hard-linked names for one
   inode are collapsed to a single entry.
3. **Partial hash.** For each candidate, hash first 32 KB + last 32 KB
   (single digest; files ≤ 64 KB hashed whole). Consult the content-hash
   cache first — hits skip all I/O. Keep only groups still sharing a partial
   hash. Runs in parallel across size groups.
4. **Full hash.** Hash survivors fully (`blake3`/`sha256`/`md5`), merging
   cached full hashes with freshly computed ones so a cached file still meets
   a new partner. Identical full hashes = an exact-duplicate group.
5. **Assemble exact groups (`matching`).** Pick the keeper per `--keep-*`
   (reference-dir members always win), probe `ffprobe` metadata for media
   groups in parallel, order keeper-first then path-sorted.
6. **Similar pass (`similar`, skipped by `--exact`).** Take all files *not*
   already claimed by an exact group. Bucket images by aspect ratio and
   videos by resolution (+ duration compatibility) so unique files are never
   decoded. Fingerprint: image dHash / 8 video frame hashes via `ffmpeg`,
   reusing the fingerprint cache on size+mtime hits. Star-cluster within each
   bucket at `--similarity`. Each similar member records its own content hash
   as a deletion safety check, and the keeper follows the same keep rule
   (with highest-resolution winning `--keep-smaller`).
7. **Report + delete (`report`, `actions`).** Print human or JSON, with
   reclaimable bytes and skip/error notes. On `--delete`, prompt per group
   (unless `--yes`), then re-hash each target and delete (trash or permanent)
   only on match; count freed bytes and skips.

Caches (`~/.dedupe/hashes.bin`, `~/.dedupe/fingerprints.bin`) persist steps
3–4 and 6 across runs, validated by size + mtime, so rescanning an unchanged
media library touches almost no bytes.

## Tests

```sh
cargo test
cargo clippy --all-targets
```

## License

MIT
