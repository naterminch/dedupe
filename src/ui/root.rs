//! Root component: nav rail, body and status bar.
//!
//! The rail is shared by every view: Home (the scan workspace) and Settings,
//! with the theme switch at the bottom. The body swaps between the scan
//! workspace (sidebar + results) and the Settings page. Preferences are
//! restored once, on the first frame, through `actions::bootstrap`.

use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::ui::palette::space;
use crate::ui::state::{AppView, app, scan_status};
use crate::ui::widgets::{
    caption, entering, ground, icon_ghost_button, icons, rail_button, status_strip,
};
use crate::ui::{actions, results, settings, shell, toast};
use crate::ui_helpers::counted;
use crate::util;

#[component]
pub fn DedupeApp() -> impl IntoElement {
    effect(|| untracked(actions::bootstrap));
    // Escape first closes the keyboard (fires even inside inputs);
    // outside an input it is navigation: leave the Settings page. The
    // bindings live under the root component's owner: the whole app.
    on_key_with(
        "escape",
        KeyOptions {
            when: When::Always,
            repeat: false,
            pass_through: false,
        },
        |_| dismiss_keyboard(),
    );
    on_key_with(
        "escape",
        KeyOptions {
            when: When::NotEditing,
            repeat: false,
            pass_through: false,
        },
        |_| actions::escape_leaves_settings(),
    );
    ground()
        .flex_row()
        .flex_1()
        .h_full()
        // Folders dropped from Explorer land in the scan list.
        .on_file_drop(|event: &FileDropEvent| actions::drop_folders(&event.uris))
        .child(nav_rail())
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .child(body())
                .child(status_bar()),
        )
        .child(toast::toast_layer())
        // Above the panes and the toasts: a confirmation must not be buried.
        .child(dynamic(|| {
            // Only a confirmation opening rebuilds it: a scan tick would
            // replace the dialog's buttons between press and release.
            // The delete plan follows the ticked duplicates too: ticking one
            // in the dialog rebuilds it, a scan tick does not.
            let open = computed(|| {
                app().with(|s| {
                    (
                        s.confirm_delete,
                        s.selected.len(),
                        s.consolidating,
                        s.consolidate_dest.clone(),
                        // A keeper pick inside the open dialog moves one copy
                        // off the going list and the retiring keeper onto it,
                        // so the tick count alone would not rebuild it. The
                        // poster count is here so a video's thumbnail can
                        // arrive while the dialog is open.
                        s.confirm_rev,
                        s.posters.len(),
                    )
                })
            });
            let (deleting, _, consolidating, _, _, _) = open.get();
            if deleting {
                results::delete_confirm()
            } else if consolidating {
                results::consolidate_confirm()
            } else {
                view()
            }
        }))
}

/// The icon rail's width: a 36 pt slot with 10 pt either side.
const RAIL_WIDTH: f32 = 56.0;

/// The left icon rail, shared by every view. The active view carries a
/// neutral grey pill, not an accent tint: selection is a place, not a
/// state. Reacts to the view, so the active slot moves without a rebuild
/// of the body.
fn nav_rail() -> View {
    view().child(dynamic(|| {
        // Rebuild only when the page changes. Reading the app signal here
        // directly rebuilt the rail on every scan tick (five a second), which
        // restarted the buttons' hover fade and dropped presses: a button
        // replaced between press and release never reports one.
        let page = computed(|| app().with(|s| s.view));
        let view_now = page.get();
        view()
            .flex_col()
            .items_center()
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .py(px(space::MD))
            .gap(px(space::SM))
            .child(rail_button(
                icons::HOME,
                "Scan workspace",
                view_now == AppView::Main,
                actions::leave_settings,
            ))
            .child(rail_button(
                icons::SLIDERS,
                "Settings",
                view_now == AppView::Settings,
                actions::open_settings,
            ))
            .child(view().flex_1().justify_end().child(icon_ghost_button(
                if dark() { icons::SUN } else { icons::MOON },
                if dark() {
                    "Switch to light theme"
                } else {
                    "Switch to dark theme"
                },
                actions::toggle_theme,
            )))
    }))
}

fn body() -> impl IntoElement {
    view()
        .flex_row()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .child(dynamic(|| {
            // The body owns the results pane's scroll container, so it may
            // only rebuild when the page actually changes: rebuilding it on a
            // keeper pick or a tick would jump the list back to the top.
            let page = computed(|| app().with(|s| s.view));
            // Each page rises in as it mounts: the rail says where you are and
            // the page arriving says you moved, without a wipe between them.
            if page.get() == AppView::Settings {
                let enter = entering();
                view()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(settings::settings_page())
                    .bind_motion(&enter)
            } else {
                let enter = entering();
                view()
                    .flex_row()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(shell::sidebar())
                    .child(results::results())
                    .bind_motion(&enter)
            }
        }))
}

fn status_bar() -> impl IntoElement {
    status_strip()
        .child(caption(|| {
            // A running scan's phase is what the line reports. It lives in the
            // scan signal, so the status line follows a tick without the app
            // signal changing under every other pane.
            let phase = scan_status().with(|p| p.phase.clone());
            if app().with(|s| s.scanning) && !phase.is_empty() {
                phase
            } else {
                app().with(|s| s.status.clone())
            }
        }))
        .child(caption(|| {
            let state = app();
            // Nothing scanned and nothing running: the counters would only
            // report "0 file(s) scanned", which is not information.
            if !state.with(|s| s.scanning || s.has_scanned) {
                return String::new();
            }
            let files = state.with(|s| s.files_scanned);
            let bytes = state.with(|s| s.bytes_scanned);
            format!(
                "{} scanned · {}",
                counted(files as u64, "file", "files"),
                util::human_bytes(bytes)
            )
        }))
}
