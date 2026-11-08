#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

use crate::ui::root::DedupeApp;

pub const APP_ID: &str = "com.naterminch.dedupe";

/// Boot the GUI application. Returns when the window is closed.
pub fn run() {
    let t_start = std::time::Instant::now();
    platform::set_ui_sets(vec![nsh_base::registry()]);
    let exit = platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<DedupeApp>().app_id(APP_ID)),
        platform::AppOptions {
            title: "dedupe".to_owned(),
            app_id: APP_ID.to_owned(),
            size: (1120, 640),
            ..Default::default()
        },
    );
    eprintln!(
        "[startup] window closed after {:?} ({exit:?})",
        t_start.elapsed()
    );
}
