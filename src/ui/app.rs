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
    install_crash_log();
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
    // Closing the window does not end `run_app` on nsh 0.4.0; the watcher
    // ends the process as soon as the window is gone, saving its frame.
    window_state::exit_when_window_closes();
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

/// Appended to by the panic hook below: the crash file in the temp dir.
const CRASH_LOG: &str = "dedupe-crash.log";

/// Record panics in a file before the default hook runs.
///
/// The GUI is launched from Explorer (and on Windows links as a
/// GUI-subsystem app), so it has no console: a panic in the view or the
/// platform backend would otherwise kill the window and leave no trace of
/// why. The path is written to stderr too, so a run started from a terminal
/// says where to look.
fn install_crash_log() {
    let path = std::env::temp_dir().join(CRASH_LOG);
    let report_path = path.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut text = format!("dedupe panicked: {info}\n");
        text.push_str(&format!(
            "backtrace:\n{}\n",
            std::backtrace::Backtrace::force_capture()
        ));
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&report_path)
            .and_then(|mut file| {
                use std::io::Write as _;
                file.write_all(text.as_bytes())
            });
        eprintln!(
            "dedupe crashed; details in {} (write: {written:?})",
            report_path.display()
        );
        previous(info);
    }));
}
