# dedupe

<img src="assets/icons/logo.png" alt="dedupe logo" width="88">

Finds duplicate files — exact copies, and the same photo or video saved twice in a
different format.

Windows, macOS and Linux. One binary: open the desktop app, or run a scan from the
terminal.

## Screenshots

| Light | Dark | Before deleting |
| --- | --- | --- |
| ![The desktop app in light mode](docs/gui-light.png) | ![The desktop app in dark mode](docs/gui-dark.png) | ![The delete confirmation dialog](docs/gui-delete-confirm.png) |

## What it finds

- **Exact duplicates** — same bytes, whatever the file is called or wherever it sits.
- **Similar media** (on by default) — the same picture re-saved as `.png` or `.jpg`,
  the same clip re-encoded to `.mov`. Turn it off with `--exact`.
- **Disk clutter** — empty files and folders, temp files, broken symlinks,
  mislabelled extensions, bad names, the biggest files in a folder, duplicate
  folders, and duplicate music.

## Desktop app

Run `dedupe` with no arguments (or `dedupe --gui`).

1. Pick a folder on the left and press **Scan**.
2. Results are grouped by content. Green **KEEP** is the copy that stays, amber
   **Selected** is ticked for deletion.
3. **Delete** shows exactly what is going to go, group by group, then moves it to
   the trash. **Stop** cancels a scan instantly.

Folders can be marked as **protected** so nothing inside them is ever deleted.
The keep rule (First / Smallest / Newest / Oldest / Best quality) can be changed
without rescanning, and **Consolidate** gathers one copy of every group into a
folder of your choosing. Settings also cover the scan type, match mode, image
fingerprint and partial clips; light/dark and your preferences persist between
runs.

## Command line

```sh
dedupe ~/Pictures                          # scan and report
dedupe ~/Photos --delete --trash           # review each group, then delete
dedupe /incoming --reference-dir /backup -Dy   # no prompts, keep the references
dedupe ~/photos --consolidate-dir ~/sorted -y  # gather one copy of everything
dedupe ~/photos --move-dir D:\quarantine -y    # move duplicates aside instead
dedupe ~/photos --hardlink -y              # replace copies with hard links
dedupe ~/photos --strip-exif -y            # strip EXIF/XMP in place
dedupe ~/downloads --find empty-dirs -D -y # cleanup scan
dedupe ~/music --find music --music-by content
dedupe ~/photos --match fuzzy --fuzzy-threshold 90
dedupe ~/photos --csv report.csv --html report.html
```

Useful flags:

| Flag | What it does |
| --- | --- |
| `--find MODE` | Cleanup mode: `empty-files`, `empty-dirs`, `big-files`, `temp-files`, `invalid-symlinks`, `broken-files`, `bad-extensions`, `bad-names`, `duplicate-folders`, `music` |
| `--match` | Group by `content` (default), `size`, `name` or `fuzzy` |
| `--similarity PCT` | How close two images/videos must be (default 97) |
| `--image-hash` | `auto` (dHash), `phash`, `ahash` or `color` |
| `--partial-clips` | Also match a short clip found inside a longer one |
| `--trash` / `--dry-run` | Trash instead of deleting; or only list what would go |
| `--reference-dir` | Never delete anything in here (repeatable) |
| `--jobs N` | Worker threads (`0` = all but one core) |
| `--json` / `--csv` / `--html` | Machine-readable, spreadsheet or shareable report |

`dedupe --help` has the full list.

## Is my data safe?

- Every file is re-hashed immediately before it is removed; anything that changed
  meanwhile is skipped.
- Nothing is deleted without a per-group prompt unless you pass `-y`, and
  `--trash` puts files in the system trash where it can be restored.
- `--dry-run`, `--move-dir` and `--hardlink` give you ways to clean up without
  losing anything.
- Re-scans are fast: unchanged files are skipped using the size + mtime caches in
  `~/.dedupe/`. `--no-cache` rebuilds them, `--clear-cache` wipes them.

## Build

```sh
.\build.ps1            # Windows: release binary copied to .\dedupe.exe
cargo build --release  # any OS
```

Rust is the only requirement. FFmpeg is optional — it adds resolution, duration
and codec info, plus video comparison; everything else works without it.

Release downloads: `dedupe.exe` on Windows, the bare binary plus a `dedupe.desktop`
launcher on Linux, and a `dedupe.app` bundle next to the binary on macOS. The GUI
never opens a terminal window on any platform.

## How it works

1. Walk the paths, apply the type/size/depth/exclude filters.
2. Group by size, collapse hard links, then partial-hash and fully hash what is
   left (`blake3` by default).
3. Fingerprint the remaining images and videos (dHash on a 17×16 thumbnail; 8
   frames for video) and cluster them at the similarity threshold.
4. Report what can be reclaimed, or delete it after a re-hash.

## Development

```sh
cargo test
cargo clippy --all-targets
```

Commits run `cargo fmt`, `cargo clippy` and file hygiene checks through
[prek](https://prek.j178.dev/), the same ones CI uses (`prek.toml`):

```sh
prek install          # check automatically on commit
prek run --all-files  # check everything now
```

## License

MIT
