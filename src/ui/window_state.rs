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
//!   which persists it in `dedupe.json`.
//! - [`restore`]: replays the saved position (and maximized state) right
//!   after the first launch layout. The saved size needs no replay: it is
//!   the client size, and `AppOptions.size` already applied it at creation.
//! - [`exit_when_window_closes`]: ends the process once the window is gone,
//!   because the platform's message loop stops there without returning.
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
    use nsh_core::desktop::{WindowFrame, WindowMode};
    let Some(g) = saved.filter(WindowGeometry::usable) else {
        return;
    };
    // The main window: this runs before any render, so the command addresses
    // the root directly instead of resolving a window from context.
    let window = nsh_core::element::desktop::main_window();
    // Position and mode only. `AppOptions.size` already applied the saved
    // client size at creation, so replaying it here would set the *outer*
    // frame to a client size: the window would reopen one title bar and two
    // borders smaller than it was left, with a visible shrink just after the
    // first paint. Its size is not this function's to set.
    window.set_bounds(WindowFrame {
        x: Some(nsh_core::Points(g.x)),
        y: Some(nsh_core::Points(g.y)),
        width: None,
        height: None,
        animated: Some(false),
    });
    if g.maximized {
        window.set_mode(WindowMode::Maximized);
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

/// End the process when the app window closes.
///
/// Closing the window destroys it, but on nsh 0.4.0 `run_app` never returns
/// afterwards: its loop stops pumping messages and the UI thread parks with
/// nothing left to dispatch, so `main` would never finish. The window is
/// gone either way, yet the process lingers in Task Manager and keeps the
/// single-instance mutex, so the next launch raises no window and looks
/// broken. Watching for our window is the one signal that survives: it is
/// seen once the app is up, and its disappearance means the user closed it.
///
/// Two consecutive misses are required so a lookup that races a momentary
/// window teardown (a DPI or theme re-create) cannot quit the app.
#[cfg(all(target_os = "windows", not(test)))]
pub(crate) fn exit_when_window_closes() {
    const POLL: std::time::Duration = std::time::Duration::from_millis(200);
    const MISSES: u8 = 2;
    let spawned = std::thread::Builder::new()
        .name("dedupe-close-watch".into())
        .spawn(|| {
            let mut seen = false;
            let mut misses = 0;
            loop {
                std::thread::sleep(POLL);
                if our_window().is_some() {
                    seen = true;
                    misses = 0;
                } else if seen {
                    misses += 1;
                    if misses >= MISSES {
                        break;
                    }
                }
            }
            save_frame();
            // `std::process::exit` runs `ExitProcess`, which detaches every
            // loaded DLL; the parked UI thread holds graphics and shell locks,
            // so that detach blocks. Terminate the process outright instead:
            // the frame above is already on disk, and the job object closes
            // with us, taking any running ffmpeg with it.
            // SAFETY: terminates only this process, and only after the work
            // this function is responsible for (persisting the frame) is done.
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
                let _ = TerminateProcess(GetCurrentProcess(), 0);
            }
        });
    let _ = spawned;
}

#[cfg(any(not(target_os = "windows"), test))]
pub(crate) fn exit_when_window_closes() {}

/// Persist the last sampled frame before the window watcher exits.
///
/// The sampler keeps running while the window is up, so the last frame is the
/// size and place the user actually left behind. Read-modify-write on disk so
/// the settings the app already saved stay as they are.
#[cfg(all(target_os = "windows", not(test)))]
fn save_frame() {
    let Some(frame) = current() else {
        return;
    };
    let mut prefs = crate::prefs::GuiPrefs::load();
    prefs.window = Some(frame);
    let _ = prefs.save();
}

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
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowPlacement, GetWindowRect, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED,
        WINDOWPLACEMENT,
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
        // The width and height are the *client* size, because that is what
        // `AppOptions.size` means when this frame restores at the next
        // launch. `rcNormalPosition` is the outer frame: storing that would
        // add the title bar and borders to the window on every launch.
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client).ok()?;
        let mut outer = RECT::default();
        GetWindowRect(hwnd, &mut outer).ok()?;
        let (width_px, height_px) = restored_client_px(
            (outer.right - outer.left, outer.bottom - outer.top),
            (client.right - client.left, client.bottom - client.top),
            (rc.right - rc.left, rc.bottom - rc.top),
        );
        // Both are in physical pixels while nsh sizes windows in points
        // (`px = points × dpi / 96` in the platform), so convert back to the
        // points `AppOptions.size` expects.
        let to_points = |px: i32| px as f32 * 96.0 / dpi as f32;
        Some(WindowGeometry {
            x: to_points(rc.left),
            y: to_points(rc.top),
            width: to_points(width_px),
            height: to_points(height_px),
            maximized: placement.showCmd as i32 == SW_SHOWMAXIMIZED.0,
        })
    }
}

/// The client size a restored-down window will have, from the current outer
/// and client sizes and the placement's restore-down frame.
///
/// While the window is maximized `GetClientRect` reports the work area, not
/// the size it goes back to: sampling it unchanged would restore the window at
/// screen size and leave nothing sensible to un-maximize to. The non-client
/// extents are constant for a given DPI and window style, so they come out of
/// the current pair and come off the restore-down frame. An un-maximized
/// window has `normal == outer` and lands back on its client size.
#[cfg(target_os = "windows")]
fn restored_client_px(outer: (i32, i32), client: (i32, i32), normal: (i32, i32)) -> (i32, i32) {
    (
        (normal.0 - (outer.0 - client.0)).max(1),
        (normal.1 - (outer.1 - client.1)).max(1),
    )
}

#[cfg(all(target_os = "windows", test))]
mod tests {
    use super::restored_client_px;

    #[test]
    fn a_maximized_window_keeps_its_restore_down_size() {
        // Measured on this machine: a maximized 2576x1408 outer framing a
        // 2560x1369 client, restoring down to a 1016x739 outer frame.
        assert_eq!(
            restored_client_px((2576, 1408), (2560, 1369), (1016, 739)),
            (1000, 700)
        );
    }

    #[test]
    fn a_normal_window_saves_its_own_client_size() {
        assert_eq!(
            restored_client_px((1296, 859), (1280, 820), (1296, 859)),
            (1280, 820)
        );
    }

    #[test]
    fn a_frame_larger_than_the_placement_never_zeroes_the_window() {
        assert_eq!(restored_client_px((100, 100), (20, 20), (10, 10)), (1, 1));
    }
}
