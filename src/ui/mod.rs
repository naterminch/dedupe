//! Desktop GUI for dedupe, built on gpui-kit.
//!
//! Opens when the binary runs without a scan path (or with `--gui`). The GUI
//! drives the same [`crate::pipeline::run_scan`] as the CLI, so results are
//! identical; only the presentation differs.
//!
//! Layout: a side nav (folders + per-run scope + Scan), a main results
//! pane with one collapsible section per duplicate group, a Settings page
//! for global defaults (matching + system), and a status bar. Pure helpers
//! live in [`crate::ui_helpers`]; group/member rows live in
//! [`crate::ui_results`].
//!
//! Native-feel boundaries (do not reimplement inside the view layer):
//! folder picking and report saving go through native `rfd` dialogs off the
//! GPUI thread, deletions honor the OS trash, and theme mode persists to
//! `gui-prefs.json` and is restored before first paint.

mod app;
mod folders;
mod operations;
mod render;
mod save;
mod scan;
mod selection;
mod theme;
mod view;

pub use app::run;
pub(crate) use theme::ThemeReadability;
pub(crate) use view::{
    AppView, DedupeView, FolderEntry, GROUP_PAGE, ReportSnapshot, SIZE_SLIDER_MAX, SIZE_SLIDER_MIN,
    SettingsTab, THUMB_EXTS,
};

#[cfg(test)]
mod tests;
