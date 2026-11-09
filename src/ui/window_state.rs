//! Window frame persistence and the single-instance guard (Windows).
//!
//! nsh carries no window position through its environment, so this module
//! bridges the gap with a few small Win32 calls:
//!
//! - [`detect_prior_instance`]: a named mutex under the `Local` namespace
//!   makes a second launch raise the running window and quit instead of
//!   stacking a second one.
//! - [`start_watching`]: a timer samples the real window frame every so
//!   often; [`current`] hands the last snapshot to `actions::save_prefs`,
//!   which persists it in `gui-prefs.json`.
//! - [`restore`]: replays a saved frame (and maximized state) right after
//!   the first launch layout.
//!
//! Everything is best-effort: a failed Win32 call leaves the defaults in
//! place and never takes the GUI down.

// Only the sampler uses prelude items (`Owner`, `set_interval`); test
// builds stub the sampler out, so the import follows it.
#[cfg(all(target_os = "windows", not(test)))]
use nsh_core::prelude::*;

use crate::prefs::WindowGeometry;

/// The last frame the sampler observed. Written on the Windows timer;
/// readable everywhere so `save_prefs` runs the same on all targets.
/// Poison is recovered: a panicking sampler would otherwise freeze the
/// value in the default `None`.
static LAST_FRAME: std::sync::Mutex<Option<WindowGeometry>> = std::sync::Mutex::new(None);

/// The last frame the sampler observed, when it is plausible enough to
/// persist. Without a sampler (other targets, tests) this is `None`.
#[must_use]
pub(crate) fn current() -> Option<WindowGeometry> {
    LAST_FRAME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .filter(WindowGeometry::usable)
}

#[cfg(target_os = "windows")]
pub(crate) fn detect_prior_instance(app_id: &str) -> bool {
    // `Local\` scopes the mutex to this logon session, so a second user
    // (RDP, fast user switching) still gets their own instance.
    let name = windows::core::HSTRING::from(format!(r"Local\{app_id}.window"));
    // SAFETY: `name` outlives the call; no security descriptor and no
    // initial ownership is the documented quiet-mutex usage, and
    // GetLastError tells "already existed" apart from "created".
    let prior = unsafe {
        match windows::Win32::System::Threading::CreateMutexW(None, false, &name) {
            Ok(_) => {
                windows::Win32::Foundation::GetLastError()
                    == windows::Win32::Foundation::ERROR_ALREADY_EXISTS
            }
            Err(_) => false,
        }
    };
    if prior {
        raise_running_window();
    }
    !prior
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_prior_instance(app_id: &str) -> bool {
    let _ = app_id;
    true
}

#[cfg(target_os = "windows")]
fn raise_running_window() {
    if let Some(hwnd) = our_window() {
        // SAFETY: FindWindow above only reads the window list; here we
        // merely restore and activate the previous instance's window.
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
            };
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Handle of this app's window: the fixed nsh window class plus our
/// title. A lookup failure simply means there is nothing to observe.
#[cfg(target_os = "windows")]
fn our_window() -> Option<windows::Win32::Foundation::HWND> {
    let class = windows::core::HSTRING::from("NshWindow");
    let title = windows::core::HSTRING::from(crate::ui::app::WINDOW_TITLE);
    // SAFETY: both strings outlive the call and FindWindow never mutates
    // anything; a miss is returned as an error, not a null panic.
    let hwnd =
        unsafe { windows::Win32::UI::WindowsAndMessaging::FindWindowW(&class, &title) }.ok()?;
    if hwnd.0.is_null() { None } else { Some(hwnd) }
}

#[cfg(target_os = "windows")]
pub(crate) fn restore(saved: Option<WindowGeometry>) {
    use nsh_core::desktop::WindowFrame;
    use nsh_core::element::desktop::{set_frame, toggle_maximize};
    let Some(g) = saved.filter(WindowGeometry::usable) else {
        return;
    };
    set_frame(WindowFrame {
        x: Some(nsh_core::Points(g.x)),
        y: Some(nsh_core::Points(g.y)),
        width: Some(nsh_core::Points(g.width)),
        height: Some(nsh_core::Points(g.height)),
        animated: Some(false),
    });
    if g.maximized {
        toggle_maximize();
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn restore(saved: Option<WindowGeometry>) {
    let _ = saved;
}

#[cfg(all(target_os = "windows", not(test)))]
pub(crate) fn start_watching() {
    const FRAME_SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(800);
    // The root component spans the whole app, but a detached owner makes
    // that assumption explicit: the timer lives for the process, not for
    // one view branch (same pattern as `actions::app_spawn`).
    Owner::detached().run(|| set_interval(FRAME_SAMPLE_INTERVAL, sample_real_window));
}

#[cfg(any(not(target_os = "windows"), test))]
pub(crate) fn start_watching() {}

/// Sample the real frame into [`LAST_FRAME`].
#[cfg(all(target_os = "windows", not(test)))]
fn sample_real_window() {
    let read = read_window_frame();
    if let (Ok(mut slot), Some(frame)) = (LAST_FRAME.lock(), read) {
        *slot = Some(frame);
    }
}

#[cfg(all(target_os = "windows", not(test)))]
fn read_window_frame() -> Option<WindowGeometry> {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, WINDOWPLACEMENT,
    };

    let hwnd = our_window()?;
    let mut placement = WINDOWPLACEMENT::default();
    // SAFETY: `hwnd` is our live window handle and `placement` is a
    // correctly sized out-parameter; the dpi lookup only reads state.
    unsafe {
        GetWindowPlacement(hwnd, &mut placement).ok()?;
        // `showCmd` is a u32 while the `SW_*` constants wrap an i32; the
        // documented range is 0..=10, so the cast is lossless.
        if placement.showCmd as i32 == SW_SHOWMINIMIZED.0 {
            // Iconized geometry is transient; keep the last real frame.
            return None;
        }
        let dpi = u32::max(GetDpiForWindow(hwnd), 1);
        let rc = placement.rcNormalPosition;
        // Placement is in physical pixels while nsh sizes windows in
        // points (`px = points × dpi / 96` in the platform), so convert
        // back to the points `set_frame` and `AppOptions.size` expect.
        let to_points = |px: i32| px as f32 * 96.0 / dpi as f32;
        Some(WindowGeometry {
            x: to_points(rc.left),
            y: to_points(rc.top),
            width: to_points(rc.right - rc.left),
            height: to_points(rc.bottom - rc.top),
            maximized: placement.showCmd as i32 == SW_SHOWMAXIMIZED.0,
        })
    }
}
