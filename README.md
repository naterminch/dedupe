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

## GUI

```sh
dedupe-gui     # desktop app, no terminal
dedupe         # desktop app + terminal
dedupe --gui   # same, explicit
```

Pick folders, press **Scan**, tick duplicates, **Delete selected**.
Per-folder **Ref** toggles protect backups; **Keep** switches the keeper
(first / smallest / newest / oldest); deletion goes to trash by default.

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
`--trash` · `--dry-run` · `--exact` · `--no-cache` · `-y` (no prompts).
Full list: `dedupe --help`.

```sh
# Safe cleanup: protect /backup, trash the rest, no prompts
dedupe /incoming --reference-dir /backup --delete --yes --trash
```

`✓ KEEP` stays, `✗ DUP` goes. Files are re-hashed before removal, so a
file that changed since the scan is never deleted.

## Build

```sh
.\build.ps1            # Windows: release -> dedupe.exe + dedupe-gui.exe
cargo build --release  # any OS
```

Needs the Rust toolchain. Optional: FFmpeg (media details + video
comparison); without it, everything else still works.

## How it works

Same size → partial hash → full hash (`blake3` default). Images/videos
also get a visual fingerprint (97%+ similar counts, tunable). Big-file
hashes are cached in `~/.dedupe/`, reused only while size + mtime match.
Hard links count once.

## Tests

```sh
cargo test
cargo clippy --all-targets
```

## License

MIT
