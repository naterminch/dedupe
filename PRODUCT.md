# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

The product is a native desktop app for Windows, macOS and Linux, not a website
or a mobile app. It is one Rust binary with an nsh GUI (`src/ui/`) and a CLI.
`web` is recorded only because Impeccable's platform values (web / ios /
android / adaptive) have no option for desktop. The owner's decision is that
the GUI uses one design on every OS and does not adapt to each OS's design
language. DESIGN.md is the visual authority for that one look.

## Users

Primary: someone at home whose photo, video and download folders have filled
the disk with copies. They are not technical and use the desktop app, not the
terminal. Their job is to get disk space back without losing anything they meant
to keep.

The CLI serves technical users and scripts. It is a capability of the product,
but the GUI user comes first in design decisions.

## Product Purpose

dedupe finds duplicate files, shows how much space deleting them would free, and
removes or gathers the extra copies. It finds:

- exact duplicates (same bytes, any name or location);
- similar media (the same photo or video re-saved in another format or
  re-encoded), on by default;
- disk clutter: empty files and folders, temp files, broken symlinks,
  mislabelled extensions, bad names, the biggest files, duplicate folders,
  duplicate music.

Success means the user ends a session with space freed and no file lost that
they wanted.

## Positioning

The owner confirmed one point: dedupe is a single free, MIT-licensed binary
that runs on Windows, macOS and Linux, with both a desktop app and a CLI.

Two other candidates were offered and not chosen as positioning: safety
(re-hash before delete, protected folders, Trash) and matching similar media.
Both remain real capabilities, listed below. Speed was also offered and not
chosen.

## Operating Context

- GUI flow: add folders (Browse, paste a path, or drop on the window), choose
  what to find, scan, review groups with the Reclaimable figure, then delete,
  consolidate, or export.
- Delete follows the Trash setting: on, files go to the system trash; off, they
  are deleted permanently. A confirmation dialog lists what will go first.
- The keep rule (First / Smallest / Newest / Oldest / Best quality) can change
  after a scan without rescanning.
- Folders can be marked protected: their copies always win the keep decision
  and are never deleted.
- FFmpeg is optional. It adds media info, video comparison and audio
  fingerprints.
- Settings and light/dark mode persist between runs. Caches live in
  `~/.dedupe/`.

## Capabilities and Constraints

- Every file is re-hashed immediately before it is removed; anything that
  changed in the meantime is skipped.
- Nothing inside a protected folder or `--reference-dir` is ever deleted, in any
  mode.
- Ways to clean up without losing data: dry run, move-aside, hard links,
  consolidate.
- Terms used across the UI: **Keeping** (the badge on the copy that stays),
  **Delete** / **Keep too** (a copy ticked / unticked for deletion),
  **Reclaimable** (the space a delete frees), **protected** folder,
  **Consolidate**, cleanup scan / findings.
- One binary per OS. The GUI never opens a terminal window.

## Brand Commitments

- Name: `dedupe`, lowercase.
- Logo: `assets/icons/logo.png`; app icon: `assets/icons/`, `assets/icon.ico`.
- License: MIT.

## Evidence on Hand

- Screenshots in light and dark of home, results and delete confirmation:
  `docs/gui-*.png`.
- The README describes features, CLI flags and data-safety behavior.
- No testimonials, user counts, benchmarks or press exist. Do not invent them.

## Product Principles

1. Built for the person at home first: plain words, no jargon in the GUI.
2. Losing a wanted file is the worst outcome. Every destructive step shows what
   will go before it goes.
3. Lead with the answer: how much space comes back, and which copies go.
4. One product everywhere: the same capabilities in the GUI and the CLI, on
   every OS.
