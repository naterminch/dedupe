//! dedupe core: scan, hash, match, report and delete duplicate files.
//!
//! The library crate holds everything both front ends share. The CLI
//! (`src/main.rs`) adds progress bars and terminal output on top of
//! [`pipeline::run_scan`]; the GUI (`ui`) drives the same pipeline from a
//! gpui-kit window.

pub mod actions;
pub mod cache;
pub mod cli;
pub mod hashing;
pub mod matching;
pub mod media;
pub mod pipeline;
pub mod poster;
pub mod report;
pub mod scan;
pub mod similar;
pub mod ui;
pub mod util;
