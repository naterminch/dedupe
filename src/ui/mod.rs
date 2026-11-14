//! Desktop GUI for dedupe, built on Native Shell (nsh).
//!
//! Opens when the binary runs without a scan path (or with `--gui`). The GUI
//! drives the same [`crate::pipeline::run_scan`] as the CLI, so results are
//! identical; only the presentation differs.
//!
//! Layout: an icon rail (workspace + settings), a sidebar with the wordmark,
//! quick actions and the folder list, and a main pane that opens on the scan
//! setup form before a run and then shows one
//! collapsible card per duplicate group, plus a Settings page for global
//! defaults and a status bar. Pure helpers live in `crate::ui_helpers`; every
//! state transition lives in `actions`.
//!
//! Native-feel boundaries (do not reimplement inside the view layer):
//! folder picking and report saving go through native `rfd` dialogs off the
//! UI thread, deletions honor the OS trash, and theme mode persists to
//! `dedupe.json` and is restored before first paint.

mod actions;
mod app;
mod palette;
mod results;
mod root;
mod settings;
mod shell;
mod state;
#[cfg(test)]
mod tests;
mod toast;
mod widgets;
mod window_state;

pub use app::run;
