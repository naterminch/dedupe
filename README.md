# dedupe

A small tool that finds duplicate files for you.

Point it at a folder and it scans everything inside, including subfolders.
It finds two kinds of duplicates:

- **Exact duplicates** — files that are the same, byte for byte. A copy, a
  rename, a backup — gone.
- **Similar duplicates** (on by default) — the same photo saved as `.png`
  *and* `.jpg`, or the same video saved as `.mp4` *and* `.mov`. The files are
  different, but the picture you see is the same. Turn this off with `--exact`.

One single binary. No install, no setup. Works on Windows, macOS and Linux.

## Graphical interface

Run **`dedupe-gui`** (no console window) or the console binary without a
path / with `--gui` (a terminal stays open — normal for console apps):

```sh
dedupe-gui     # desktop UI, no terminal
dedupe         # desktop UI + terminal
dedupe --gui   # same, explicit
dedupe <path>  # command-line scan (see below)
```

The GUI mirrors every CLI option: folder list with per-folder **Ref**
protection toggles (`Add` / `Browse…`), file types, hash algorithm,
similarity threshold, exact-only, trash, fingerprint cache, size sliders +
exact boxes, max depth and exclude filters. The **Keep** mode
(First / Smallest / Newest / Oldest) sits at the top next to Delete and
reassigns keepers instantly. Press **Scan**, expand a group to inspect its
files (image thumbnails and video posters included) — each row has a
**Keep** button for per-group decisions — use the **All** checkbox or
**Invert** to bulk-select, switch keeper strategy with the **Keep:**
button, sort biggest-first with **Sort**, save the machine-readable
report with **Save JSON**, and reveal any file's folder with its
folder button. Tick the duplicates to remove and press **Delete selected**
(keepers are never selectable; every file is re-hashed before removal,
exactly like `--delete`). Deletion moves to the system trash by default
(toggleable).

> Building the GUI needs the gpui-kit system requirements (Windows: VS 2022
> C++ workload with an MSVC toolchain; see
> [installation](https://gpui-kit.com/docs/installation)). The CLI-only
> code paths have no extra requirements.

## Build

You need the Rust toolchain (https://rustup.rs).

```sh
# Windows (PowerShell) — builds the release binary and copies it to the project root:
.\build.ps1

# Any OS — manual build:
cargo build --release
# binary: target/release/dedupe
```

Optional: install [FFmpeg](https://ffmpeg.org). Duplicate detection works
without it — FFmpeg only adds extra media details (resolution, duration,
codec) and enables video comparison.

## Use

```
dedupe [OPTIONS] <PATH>...
```

You must give it a path. Run `dedupe .` to scan the current folder. Running
the binary with no path just prints the help.

### Options

| Flag | What it does |
| --- | --- |
| `-t, --types EXTS` | Scan only these extensions, comma-separated and case-insensitive, e.g. `--types jpg,png,mp4,txt` |
| `-k, --keep-smaller` | Mark the smallest file in each duplicate group as the keeper; for similar media, keep the **highest-resolution** version instead |
| `--keep-newest` / `--keep-oldest` | Keep the most / least recently modified file in each group (conflicts with `--keep-smaller`) |
| `--reference-dir PATH` | Protect folders (repeatable): files under these dirs are never deleted and win the keep decision — like czkawka reference dirs |
| `-D, --delete` | Delete the duplicate files. Prompts per group unless `-y` |
| `--trash` | Move duplicates to the system trash instead of deleting permanently |
| `--dry-run` | With `--delete`: print what would be removed without removing anything |
| `-y, --yes` | Assume "yes" for all deletion prompts |
| `--hash ALGO` | Hash algorithm: `blake3` (default), `sha256`, `md5` |
| `--exact` | Only byte-identical duplicates. Similar images/videos (same content in a different format, e.g. `.png` vs `.jpg`, `.mp4` vs `.mov`) are detected **by default**; this flag disables that pass |
| `--similarity PCT` | Similarity threshold (0-100) above which similar media counts as duplicated (default: 97) |
| `--no-cache` | Do not read or write the fingerprint cache (`~/.dedupe/fingerprints.bin`) — recompute fingerprints from scratch |
| `--min-size SIZE` / `--max-size SIZE` | Ignore files outside this size range (`100KB`, `2MB`, `1GiB`, ...) |
| `--max-depth N` | Limit recursion depth (`0` = files directly in the given paths only) |
| `--exclude-dir NAME` | Prune directories whose name contains this substring (repeatable) |
| `--exclude-path SUBSTR` | Skip files whose full path contains this substring (repeatable) |
| `-j, --json` | Machine-readable JSON output |
| `-v, --verbose` | Per-file media metadata (resolution, duration, codec) |
| `-q, --quiet` | Suppress progress output |

### Examples

```sh
# Find all duplicates under /media, subfolders included
dedupe /media

# Only images and videos
dedupe /media --types jpg,png,mp4,mkv

# Find duplicates, keeping the smallest copy, deleting the rest without prompts
dedupe /media --keep-smaller --delete --yes

# Show resolution/duration for media duplicates
dedupe /media --types jpg,mp4 --verbose

# Skip huge or tiny files, ignore node_modules
dedupe /projects --min-size 1KB --max-size 500MB --exclude-dir node_modules

# Scriptable output
dedupe /media --json

# Protect /backup, remove duplicates from /incoming, move to trash
dedupe /incoming --reference-dir /backup --delete --yes --trash

# Preview what would be deleted, keeping the newest copy
dedupe /media --keep-newest --delete --dry-run

# Same photo saved as both PNG and JPG (or a video re-encoded as .mov/.avi) —
# similar-duplicate detection is ON by default
dedupe /media

# Only exact (byte-identical) duplicates
dedupe /media --exact

# Tighter/looser similarity threshold
dedupe /media --similarity 98
```

### The report

```
Scanned 1 path(s) · 4 files (128.3 KB) · 1 duplicate group(s) · 2 duplicate file(s)
Reclaimable with --delete: 62.10 KB

────────────────────────────────────────────────────────────
◆ Group #1 · VIDEO · 2 files · 62.10 KB each · 62.10 KB reclaimable · blake3 69d57daf8d78
  ↳ 1280x720 · 4s · h264
  ✓ KEEP  /media/clip-backup.mp4                    62.10 KB
  ✗ DUP   /media/clip.mp4                           62.10 KB

Tip: run with --delete to remove the 2 duplicate file(s), or --delete --keep-smaller to keep one copy (smallest; highest-resolution for similar media).
```

`✓ KEEP` is the file that stays. `✗ DUP` is the file that `--delete` would
remove. Without `--keep-smaller`, the first path in the group is the keeper.
Colors show only on a terminal (respecting `NO_COLOR`); piped or `--json`
output stays plain.

## How it finds duplicates

1. **Group by size.** Only files of the same size can be duplicates.
2. **Hash.** Files with the same hash are exact duplicates.
3. **For images and videos, compare what you see, not the bytes:**
   - images get a fingerprint of their brightness pattern,
   - videos get fingerprints of 8 frames, spread evenly over the video.
   A fingerprint match of 97% or more counts as a duplicate (tune with
   `--similarity`). With `--keep-smaller`, the **highest-resolution** media
   version is kept.
4. **Remember between runs.** Content hashes (files ≥ 256 KB) and media
   fingerprints are saved in a cache, so the next scan of the same folders
   is much faster. A cached result is reused only while the file's size and
   modification time are unchanged, so it never goes stale. `--no-cache`
   skips it. Cache files live in `~/.dedupe/` (`hashes.bin`,
   `fingerprints.bin`); `DEDUPE_CACHE` overrides the location.
5. **Hard links count once.** Two names for the same inode are not two
   copies, so extra names are skipped (like czkawka).

## Tests

```sh
cargo test         # unit + integration tests
cargo clippy --all-targets
```

## License

MIT