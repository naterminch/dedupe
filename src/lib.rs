//! dedupe core: scan, hash, match, report and delete duplicate files.
//!
//! The library crate holds everything both front ends share. The CLI
//! (`src/main.rs`) adds progress bars and terminal output on top of
//! [`pipeline::run_scan`]; the GUI (`ui`) drives the same pipeline from a
//! Native Shell window.

/// Deleting, moving and quarantining what a scan found, and the re-checks
/// that run before each file is touched.
pub mod actions;
/// Audio fingerprinting for the music mode.
pub mod audio;
pub mod cache;
pub mod cancel;
/// Command-line arguments and the option types the pipeline takes.
pub mod cli;
pub mod glob;
/// Content hashing (full file and partial) with an in-run cache.
pub mod hashing;
/// Turning hashed and fingerprinted entries into duplicate groups.
pub mod matching;
/// Optional ffprobe lookups for image and video metadata.
pub mod media;
/// Cleanup modes: empty, temporary, oversized, broken and duplicate items.
pub mod modes;
pub mod pipeline;
pub mod poster;
pub mod prefs;
/// Rendering results as human text, JSON, CSV or HTML.
pub mod report;
/// Walking the scan roots into the entries the pipeline hashes.
pub mod scan;
pub mod similar;
pub mod ui;
pub mod ui_helpers;
/// Small shared formatting helpers.
pub mod util;
pub mod walk;
