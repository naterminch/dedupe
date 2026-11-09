#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

use crate::prefs::{self, WindowGeometry};
use crate::ui::root::DedupeApp;
use crate::ui::window_state;

pub const APP_ID: &str = "com.naterminch.dedupe";

/// OS window title. A constant because the single-instance raiser looks
/// the running window up by this exact text.
pub(crate) const WINDOW_TITLE: &str = "dedupe";

pub fn run() {
    let t_start = std::time::Instant::now();
    // A second launch raises the running window instead of stacking one.
    if !window_state::detect_prior_instance(APP_ID) {
        return;
    }
    platform::set_ui_sets(vec![nsh_base::registry()]);
    // First paint at the saved size so the window does not visibly jump;
    // the position and maximized state are applied after first layout.
    let size = prefs::GuiPrefs::load()
        .window
        .filter(WindowGeometry::usable)
        .map(|g| (g.width as i32, g.height as i32))
        .unwrap_or((1120, 640));
    let exit = platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<DedupeApp>().app_id(APP_ID)),
        platform::AppOptions {
            title: WINDOW_TITLE.to_owned(),
            app_id: APP_ID.to_owned(),
            size,
            ..Default::default()
        },
    );
    eprintln!(
        "[startup] window closed after {:?} ({exit:?})",
        t_start.elapsed()
    );
}
