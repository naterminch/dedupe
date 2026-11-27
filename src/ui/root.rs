//! Root component: nav rail, body and status bar.
//!
//! The rail is shared by every view: Home (the scan workspace), History and
//! Settings, with the theme switch at the bottom. The body swaps between the
//! scan workspace (sidebar + results), the History page and the Settings
//! page. Preferences are restored once, on the first frame, through
//! `actions::bootstrap`.

use std::cell::Cell;
use std::rc::Rc;

use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::ui::state::{AppView, app, scan_status};
use crate::ui::widgets::{
    RAIL_GAP, RAIL_PAD, caption, drop_sheet, entering, ground, icon_ghost_button, icons,
    move_rail_marker, rail_button, rail_marker, status_strip, theme_dissolve,
};
use crate::ui::{actions, compare, history, results, settings, shell, toast};
use crate::ui_helpers::counted;
use crate::util;

#[component]
pub fn DedupeApp() -> impl IntoElement {
    effect(|| untracked(actions::bootstrap));
    // Escape first closes the keyboard (fires even inside inputs);
    // outside an input it is navigation: close a confirmation, or leave the
    // History or Settings page. The bindings live under the root
    // component's owner: the whole app.
    on_key_with(
        "escape",
        KeyOptions {
            when: When::Always,
            repeat: false,
            pass_through: false,
            for_focus: false,
        },
        |_| dismiss_keyboard(),
    );
    on_key_with(
        "escape",
        KeyOptions {
            when: When::NotEditing,
            repeat: false,
            pass_through: false,
            for_focus: false,
        },
        |_| actions::escape_leaves_page(),
    );
    // A drag of folders over the window lays the drop sheet over the work.
    let dragging = signal(false);
    ground()
        .flex_row()
        .flex_1()
        .h_full()
        // Folders dropped from Explorer land in the scan list.
        .on_file_drag_enter(move || dragging.set(true))
        .on_file_drag_leave(move || dragging.set(false))
        .on_file_drop(move |event: &FileDropEvent| {
            dragging.set(false);
            actions::drop_folders(&event.uris);
        })
        .child(nav_rail())
        .child(
            view()
                .relative()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .child(body())
                .child(status_bar())
                .child(drop_sheet(move || dragging.get())),
        )
        .child(theme_dissolve())
        // The compare view covers the work while a pair is open. Rebuilt only
        // when it opens or closes: stepping between pairs swaps its page.
        .child(dynamic(|| {
            let open = computed(|| app().with(|s| s.comparing.is_some()));
            if open.get() {
                compare::compare_layer()
            } else {
                view()
            }
        }))
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
                        // The removal picked in the delete dialog rewrites its
                        // title, plan and commit; the findings' dialog opens
                        // on its own flag.
                        s.removal,
                        s.confirm_findings,
                        // The restore dialog opens once its plan is read, and
                        // follows the files left out of it.
                        s.restore.as_ref().map(|r| (r.run.id, r.left_out.len())),
                    )
                })
            });
            let (deleting, _, consolidating, _, _, _, _, findings, restoring) = open.get();
            if deleting {
                results::delete_confirm()
            } else if consolidating {
                results::consolidate_confirm()
            } else if findings {
                results::findings_confirm()
            } else if restoring.is_some() {
                history::restore_confirm()
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
/// of the body; the vermilion marker outlives the slots' rebuilds, so it
/// travels to the new slot instead of reappearing there.
fn nav_rail() -> View {
    let slot_of = |view: AppView| match view {
        AppView::Main => 0,
        AppView::History => 1,
        AppView::Settings => 2,
    };
    let travel = nsh_core::element::motion();
    let first = app().with_untracked(|s| slot_of(s.view));
    let shown = Rc::new(Cell::new(first));
    view()
        .child(dynamic(move || {
            // Rebuild only when the page changes. Reading the app signal here
            // directly rebuilt the rail on every scan tick (five a second),
            // which restarted the buttons' hover fade and dropped presses: a
            // button replaced between press and release never reports one.
            let page = computed(|| app().with(|s| s.view));
            let view_now = page.get();
            let slot = slot_of(view_now);
            let from = shown.replace(slot);
            if from != slot {
                move_rail_marker(travel, from, slot);
            }
            view()
                .flex_col()
                .items_center()
                .w(px(RAIL_WIDTH))
                .h_full()
                .flex_shrink_0()
                .py(px(RAIL_PAD))
                .gap(px(RAIL_GAP))
                .child(rail_button(
                    icons::HOME,
                    "Scan workspace",
                    view_now == AppView::Main,
                    actions::open_workspace,
                ))
                .child(rail_button(
                    icons::HISTORY,
                    "History",
                    view_now == AppView::History,
                    actions::open_history,
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
        .child(rail_marker(travel, first))
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
            let enter = entering();
            match page.get() {
                AppView::Settings => view()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(settings::settings_page())
                    .bind_motion(&enter),
                AppView::History => view()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(history::history_page())
                    .bind_motion(&enter),
                AppView::Main => view()
                    .flex_row()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(shell::sidebar())
                    .child(results::results())
                    .bind_motion(&enter),
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
