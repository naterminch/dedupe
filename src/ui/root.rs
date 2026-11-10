//! Root component: header, body and status bar.
//!
//! The body swaps between the scan workspace (sidebar + results) and the
//! Settings page. Preferences are restored once, on the first frame, through
//! `actions::bootstrap`.

use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::ui::palette::{colors, space};
use crate::ui::state::{AppView, app};
use crate::ui::widgets::{caption, ghost_button, heading, secondary_button};
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
        },
        |_| dismiss_keyboard(),
    );
    on_key_with(
        "escape",
        KeyOptions {
            when: When::NotEditing,
            repeat: false,
        },
        |_| actions::escape_leaves_settings(),
    );
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .bg(colors::BACKGROUND)
        // Folders dropped from Explorer land in the scan list.
        .on_file_drop(|event: &FileDropEvent| actions::drop_folders(&event.uris))
        .child(header())
        .child(body())
        .child(status_bar())
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

fn header() -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(space::LG))
        .px(px(space::XL))
        .py(px(space::LG))
        .bg(colors::GLASS)
        .backdrop_blur(24.0_f32)
        .border_b_1()
        .border_color(colors::BORDER)
        .child(
            view()
                .flex_col()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(heading("dedupe"))
                .child(caption(format!("v{}", env!("CARGO_PKG_VERSION")))),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(dynamic(|| {
                    let label = if dark() { "Light" } else { "Dark" };
                    ghost_button(label, actions::toggle_theme)
                }))
                .child(dynamic(|| {
                    if app().with(|s| s.view) == AppView::Settings {
                        secondary_button("Back", actions::leave_settings)
                    } else {
                        secondary_button("Settings", actions::open_settings)
                    }
                })),
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
        .bg(colors::GLASS)
        .backdrop_blur(20.0_f32)
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
