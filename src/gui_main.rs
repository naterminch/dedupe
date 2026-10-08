//! GUI-only entry point: always opens the dedupe window, never a terminal.
//!
//! The `windows_subsystem` attribute drops the console window on Windows,
//! so double-clicking (or `dedupe-gui` from Run) shows just the app. For
//! command-line scans, use the console `dedupe` binary instead.
#![windows_subsystem = "windows"]

fn main() {
    dedupe::ui::run();
}
