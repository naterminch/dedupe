//! Main-flow sidebar: wordmark, quick actions, the source-folder list and a
//! bottom tip card.
//!
//! Folder protection, removal and filtering live on the rows themselves; the
//! scan controls live in the workspace's composer, so the sidebar stays a
//! quiet index of places.

use nsh_base::button;
use nsh_core::prelude::*;

use crate::ui::actions;
use crate::ui::palette::{colors, radius, space, text};
use crate::ui::state::{FolderEntry, app};
use crate::ui::widgets::{
    caption, card, chip, divider, field, focus_ring, heading, hint, icon, icon_ghost_button, icons,
    motion,
};
use crate::ui_helpers::fit_text;

/// The left column of the main flow.
pub(crate) fn sidebar() -> impl IntoElement {
    view()
        .flex_col()
        .w(px(300.0))
        .h_full()
        .flex_shrink_0()
        .bg(colors::SIDEBAR)
        .border_r_1()
        .border_color(colors::BORDER)
        .child(header_block())
        .child(
            scroll().flex_1().min_h(px(0.0)).child(
                view()
                    .flex_col()
                    .gap(px(space::XS))
                    .px(px(space::SM))
                    .pb(px(space::MD))
                    .child(quick_actions())
                    .child(
                        view()
                            .px(px(space::SM))
                            .pt(px(space::MD))
                            .child(section_kicker("Folders")),
                    )
                    .child(folder_list()),
            ),
        )
        .child(tip_card())
}

fn header_block() -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(space::SM))
        .px(px(space::LG))
        .pt(px(space::LG))
        .pb(px(space::SM))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(heading("dedupe"))
                .child(view().flex_row().gap(px(2.0)).child(icon_ghost_button(
                    icons::SEARCH,
                    "Filter folders",
                    toggle_filter,
                ))),
        )
        .child(view().child(dynamic(|| {
            // Only the filter opening rebuilds this: reading the app signal
            // directly rebuilt it on every scan tick, replacing the field
            // under the pointer.
            let open = computed(|| app().with(|s| s.folder_filter.is_some()));
            if open.get() {
                view().child(filter_field())
            } else {
                view()
            }
        })))
}

fn toggle_filter() {
    app().update(|s| {
        s.folder_filter = match s.folder_filter.take() {
            Some(_) => None,
            None => Some(String::new()),
        };
    });
}

fn filter_field() -> impl IntoElement {
    field(
        "Filter folders",
        || app().with(|s| s.folder_filter.clone().unwrap_or_default()),
        |value| {
            app().update(|s| s.folder_filter = Some(value));
        },
    )
}

fn section_kicker(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::MUTED)
}

/// The "New chat"-style rows: add, scan and the trash switch, each a full
/// quiet row with an orange status pill where one exists.
fn quick_actions() -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(2.0))
        .child(side_row(
            icons::FOLDER,
            "Browse for a folder",
            None,
            RowState::Ready,
            actions::browse_folder,
        ))
        // Clearing the list only means something while it has folders.
        .child(view().child(dynamic(|| {
            // Folder changes alone flip this row: a scan tick must not rebuild
            // it under the pointer, or its hover restarts and its press drops.
            let any = computed(|| app().with(|s| !s.folders.is_empty()));
            side_row(
                icons::CLEAR,
                "Remove all folders",
                None,
                if any.get() {
                    RowState::Ready
                } else {
                    RowState::Disabled
                },
                actions::remove_all_folders,
            )
        })))
        .child(view().child(dynamic(|| {
            // The switch alone rebuilds this row, not the scan's progress.
            let trash = computed(|| app().with(|s| s.trash));
            let trash = trash.get();
            side_row(
                icons::TRASH,
                "Trash deletes",
                Some(view().child(chip(
                    if trash { "On" } else { "Off" },
                    if trash {
                        colors::ACCENT_WASH_STRONG
                    } else {
                        colors::INSET
                    },
                    if trash { colors::ACCENT } else { colors::MUTED },
                ))),
                RowState::Ready,
                toggle_trash,
            )
        })))
}

fn toggle_trash() {
    app().update(|s| s.trash = !s.trash);
    actions::save_prefs();
}

/// Whether a sidebar row can be pressed right now.
///
/// Two variants instead of a bare `bool`: `RowState::Disabled` names the state
/// the argument means, so `side_row(..)` call sites stay readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowState {
    Ready,
    Disabled,
}

/// One quiet sidebar action: icon, label and an optional trailing badge.
///
/// A [`RowState::Disabled`] row is dimmed, loses its hover and refuses
/// presses; its accessible name stays either way.
fn side_row(
    markup: &'static str,
    label: &'static str,
    trailing: Option<View>,
    state: RowState,
    on_press: impl Fn() + 'static,
) -> View {
    let ready = state == RowState::Ready;
    let mut row = button()
        .on_press(on_press)
        .disabled(!ready)
        .accessibility_label(label)
        .flex_row()
        .items_center()
        .gap(px(space::SM))
        .w_full()
        .px(px(space::SM))
        .py(px(space::XS))
        .rounded(px(radius::CONTROL))
        .opacity(if ready { 1.0 } else { 0.45 })
        .transition_colors()
        .duration(motion(150.0))
        .ease_out()
        .hover(move |s| if ready { s.bg(colors::PANE) } else { s })
        .active(|s| s.bg(colors::ELEVATED))
        .focus_visible(focus_ring)
        .child(icon(markup, 16.0, colors::MUTED))
        .child(
            text(label)
                .flex_1()
                .text_size(px(text::BODY))
                .text_color(colors::TEXT),
        );
    if let Some(trailing) = trailing {
        row = row.child(trailing);
    }
    view().child(row)
}

/// Folders the user typed, dropped or browsed for. The filter, when open,
/// narrows the list by path; rows address the live list by path so a
/// filtered view never removes the wrong folder.
fn folder_list() -> impl IntoElement {
    view().flex_col().gap(px(2.0)).child(dynamic(|| {
        // The list and the filter rebuild the rows; a scan tick must not,
        // or the Protect and Remove buttons under the pointer restart their
        // hover and never report a press.
        let rows = computed(|| {
            filtered_folders()
                .into_iter()
                .map(|folder| (folder.path, folder.reference))
                .collect::<Vec<_>>()
        });
        let rows = rows.get();
        if rows.is_empty() {
            view()
                .px(px(space::SM))
                .py(px(space::SM))
                .child(caption("No folders yet"))
        } else {
            view().flex_col().gap(px(2.0)).children(
                rows.into_iter()
                    .map(|(path, reference)| folder_row(path, reference)),
            )
        }
    }))
}

fn filtered_folders() -> Vec<FolderEntry> {
    app().with(|s| {
        let query = s
            .folder_filter
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(str::to_lowercase);
        match query {
            Some(q) => s
                .folders
                .iter()
                .filter(|f| f.path.to_lowercase().contains(&q))
                .cloned()
                .collect(),
            None => s.folders.clone(),
        }
    })
}

/// One folder row. It takes the row's values rather than a live signal: the
/// list rebuilds when a folder changes, never on a scan tick.
fn folder_row(path: String, reference: bool) -> View {
    let shown = fit_text(&path, 42);
    view()
        .flex_row()
        .items_center()
        .gap(px(space::XS))
        .px(px(space::SM))
        .py(px(space::XXS))
        .rounded(px(radius::CONTROL))
        .bg(if reference {
            colors::PANE
        } else {
            colors::SIDEBAR
        })
        .transition_colors()
        .duration(motion(150.0))
        .hover(|s| s.bg(colors::PANE))
        .child(icon(
            icons::FOLDER,
            15.0,
            if reference {
                colors::ACCENT
            } else {
                colors::MUTED
            },
        ))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    text(shown)
                        .truncate()
                        .text_size(px(text::BODY))
                        .text_color(colors::TEXT),
                )
                .tooltip(hint(path.clone())),
        )
        .child(row_ghost(
            icons::LOCK,
            if reference {
                "Unprotect folder"
            } else {
                "Protect folder"
            },
            if reference {
                colors::ACCENT
            } else {
                colors::MUTED
            },
            {
                let path = path.clone();
                move || actions::toggle_reference_path(&path)
            },
        ))
        .child(row_ghost(icons::CLEAR, "Remove folder", colors::MUTED, {
            let path = path.clone();
            move || actions::remove_folder_path(&path)
        }))
}

/// A small borderless icon action for a folder row.
fn row_ghost(
    markup: &'static str,
    label: &'static str,
    tint: AdaptiveColor,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .flex_shrink_0()
        .child(
            button()
                .on_press(on_press)
                .accessibility_label(label)
                .flex_row()
                .items_center()
                .justify_center()
                .w(px(26.0))
                .h(px(24.0))
                .rounded(px(radius::CONTROL))
                .transition_colors()
                .duration(motion(150.0))
                .ease_out()
                .hover(|s| s.bg(colors::ELEVATED))
                .active(|s| s.opacity(0.9))
                .focus_visible(focus_ring)
                .hit_slop(EdgeInsets::all(px(4.0)))
                .child(icon(markup, 14.0, tint)),
        )
        .tooltip(hint(label))
}

/// The sponsored-card slot at the bottom: what protection does and where to
/// manage it.
fn tip_card() -> impl IntoElement {
    view().px(px(space::MD)).pb(px(space::MD)).child(
        card()
            .gap(px(space::XS))
            .p(px(space::MD))
            .child(section_kicker("TIP"))
            .child(
                text("Protect the originals")
                    .text_size(px(text::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors::TEXT),
            )
            .child(caption("Protected folders are never touched by a delete."))
            .child(divider())
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(space::XS))
                            .child(icon(icons::LOCK, 14.0, colors::MUTED))
                            .child(caption("Protection")),
                    )
                    .child(
                        view()
                            .flex_col()
                            .child(
                                button()
                                    .on_press(actions::open_settings)
                                    .accessibility_label("Open settings")
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .w(px(26.0))
                                    .h(px(24.0))
                                    .bg(colors::PANE)
                                    .rounded_full()
                                    .transition_colors()
                                    .duration(motion(150.0))
                                    .ease_out()
                                    .hover(|s| s.bg(colors::ELEVATED))
                                    .active(|s| s.opacity(0.9))
                                    .focus_visible(focus_ring)
                                    .hit_slop(EdgeInsets::all(px(4.0)))
                                    .child(icon(icons::ARROW_UP_RIGHT, 13.0, colors::TEXT)),
                            )
                            .tooltip(hint("Open settings")),
                    ),
            ),
    )
}
