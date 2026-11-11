//! Root component: nav rail, body and status bar.
//!
//! The rail is shared by every view: Home (the scan workspace) and Settings,
//! with the theme switch at the bottom. The body swaps between the scan
//! workspace (sidebar + results) and the Settings page. Preferences are
//! restored once, on the first frame, through `actions::bootstrap`.

use nsh_base::button;
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::ui::palette::{colors, space};
use crate::ui::state::{AppView, app};
use crate::ui::widgets::{caption, icon, icon_ghost_button, icons, motion};
use crate::ui::{actions, results, settings, shell, toast};
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
    view()
        .flex_row()
        .flex_1()
        .h_full()
        .bg(colors::BACKGROUND)
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
            if app().with(|s| s.confirm_delete) {
                results::delete_confirm()
            } else {
                view()
            }
        }))
}

/// The left icon rail, shared by every view. The active view carries a
/// quiet pane-tinted pill, not an accent tint: selection is a place, not a
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
            .w(px(52.0))
            .h_full()
            .flex_shrink_0()
            .py(px(space::SM))
            .gap(px(space::XS))
            .bg(colors::BACKGROUND)
            .child(rail_item(
                icons::HOME,
                "Scan workspace",
                view_now == AppView::Main,
                actions::leave_settings,
            ))
            .child(rail_item(
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

/// One rail button: an icon in a rounded slot; the active view's slot is a
/// pane-tinted pill. The label lives in the accessibility tree.
fn rail_item(markup: &'static str, label: &'static str, active: bool, on_press: fn()) -> View {
    view().child(
        button()
            .on_press(on_press)
            .accessibility_label(label)
            .flex_row()
            .items_center()
            .justify_center()
            .w(px(36.0))
            .h(px(36.0))
            .rounded(px(10.0))
            .bg(if active {
                colors::PANE
            } else {
                colors::BACKGROUND
            })
            .transition_colors()
            .duration(motion(180.0))
            .ease_out()
            .hover(|s| s.bg(colors::PANE))
            .child(icon(
                markup,
                20.0,
                if active { colors::TEXT } else { colors::MUTED },
            )),
    )
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
            if page.get() == AppView::Settings {
                view()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(settings::settings_page())
            } else {
                view()
                    .flex_row()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .child(shell::sidebar())
                    .child(results::results())
            }
        }))
}

fn status_bar() -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(space::LG))
        .px(px(space::XL))
        .py(px(space::MD))
        .bg(colors::CARD)
        .border_t_1()
        .border_color(colors::BORDER)
        .child(caption(|| app().with(|s| s.status.clone())))
        .child(caption(|| {
            let state = app();
            let files = state.with(|s| s.files_scanned);
            let bytes = state.with(|s| s.bytes_scanned);
            format!("{files} file(s) scanned - {}", util::human_bytes(bytes))
        }))
}
