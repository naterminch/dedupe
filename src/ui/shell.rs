//! Main-flow sidebar: wordmark, quick actions, the source-folder list and a
//! bottom tip card.
//!
//! Folder protection, removal and filtering live on the rows themselves; the
//! scan controls live in the workspace's setup form, so the sidebar stays a
//! quiet index of places.

use std::cell::Cell;

use nsh_core::prelude::*;

use crate::ui::actions;
use crate::ui::palette::{size, space};
use crate::ui::state::{FolderEntry, app};
use crate::ui::widgets::{
    Availability, Ink, RowEmphasis, body, caption, fast_scroll, field, field_label, glyph, hint,
    icon_ghost_button, icons, list_row, notice, panel_title, row_button, row_icon_button,
    side_pane, toggle_switch, wordmark,
};
use crate::ui_helpers::fit_text;

// Where the folder list sits, as the wheel last left it (`fast_scroll`).
thread_local! {
    static FOLDERS_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
}

/// Puts the folder list back at its top: the workspace unmounts with the
/// page, so the wheel's remembered offset would point into a list that opens
/// at the start again.
pub(crate) fn reset_scroll() {
    FOLDERS_AT.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
}

/// The folder pane's width: a long path stays legible to its last folder at
/// the default text size, and the work area keeps the surplus.
const SIDEBAR_WIDTH: f32 = 264.0;

/// About how many characters of a path fit a folder row's caption line at
/// 12 pt in the pane's width; the front is cut beyond it.
const FOLDER_PATH_CHARS: usize = 24;

/// The left column of the main flow.
pub(crate) fn sidebar() -> impl IntoElement {
    side_pane(SIDEBAR_WIDTH)
        .child(header_block())
        .child(fast_scroll(
            &FOLDERS_AT,
            view()
                .flex_col()
                .gap(px(space::XS))
                .px(px(space::SM))
                .pb(px(space::MD))
                .child(quick_actions())
                .child(
                    view()
                        .px(px(space::SM))
                        .pt(px(space::LG))
                        .pb(px(space::XS))
                        .child(field_label("Folders")),
                )
                .child(folder_list()),
        ))
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
                .child(wordmark())
                .child(icon_ghost_button(
                    icons::SEARCH,
                    "Filter folders",
                    toggle_filter,
                )),
        )
        .child(view().child(dynamic(|| {
            // Only the filter opening rebuilds this: reading the app signal
            // directly rebuilt it on every scan tick, replacing the field
            // under the pointer.
            let open = computed(|| app().with(|s| s.folder_filter.is_some()));
            if open.get() {
                view().flex_row().child(filter_field())
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

/// The pane's own actions: browse, clear, and the trash switch.
fn quick_actions() -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(space::XXS))
        .child(row_button(
            icons::FOLDER,
            "Browse for a folder",
            Availability::Ready,
            actions::browse_folder,
        ))
        // Clearing the list only means something while it has folders.
        .child(view().child(dynamic(|| {
            // Folder changes alone flip this row: a scan tick must not rebuild
            // it under the pointer, or its hover restarts and its press drops.
            let any = computed(|| app().with(|s| !s.folders.is_empty()));
            row_button(
                icons::CLEAR,
                "Remove all folders",
                if any.get() {
                    Availability::Ready
                } else {
                    Availability::Disabled
                },
                actions::remove_all_folders,
            )
        })))
        .child(trash_row())
}

/// The trash setting as the control it is: a switch, the same one Settings
/// draws for the same field.
fn trash_row() -> impl IntoElement {
    view().child(dynamic(|| {
        // The switch alone rebuilds this row, not the scan's progress.
        let trash = computed(|| app().with(|s| s.trash));
        view()
            .flex_row()
            .items_center()
            .gap(px(space::SM))
            .w_full()
            .min_h(px(size::CONTROL))
            .px(px(space::SM))
            .child(glyph(icons::TRASH, Ink::Muted))
            .child(body("Trash deletes").flex_1())
            .child(toggle_switch(trash.get(), "Trash deletes", toggle_trash))
    }))
}

fn toggle_trash() {
    app().update(|s| s.trash = !s.trash);
    actions::save_prefs();
}

/// Folders the user typed, dropped or browsed for. The filter, when open,
/// narrows the list by path; rows address the live list by path so a
/// filtered view never removes the wrong folder.
fn folder_list() -> impl IntoElement {
    view().flex_col().gap(px(space::XXS)).child(dynamic(|| {
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
            // An empty list is a place to act, not a label: name both ways in.
            view()
                .flex_col()
                .gap(px(space::XXS))
                .p(px(space::SM))
                .child(caption("No folders yet"))
                .child(caption("Drop a folder on the window, or press Browse."))
        } else {
            view().flex_col().gap(px(space::XXS)).children(
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

/// One folder row: the folder's name in ink over its full path, cut from the
/// front so the end that tells two folders apart stays. A protected folder's
/// row is lifted and its lock drawn closed in ink. It takes the row's values
/// rather than a live signal: the list rebuilds when a folder changes, never
/// on a scan tick.
fn folder_row(path: String, reference: bool) -> View {
    let folder_name = std::path::Path::new(&path)
        .file_name()
        .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
    let shown = fit_text(&path, FOLDER_PATH_CHARS);
    list_row(if reference {
        RowEmphasis::Lifted
    } else {
        RowEmphasis::Plain
    })
    .child(glyph(
        icons::FOLDER,
        if reference { Ink::Text } else { Ink::Muted },
    ))
    .child(
        view()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .child(panel_title(folder_name).truncate())
            .child(caption(shown).truncate())
            .tooltip(hint(path.clone())),
    )
    .child(row_icon_button(
        icons::LOCK,
        if reference {
            "Unprotect folder"
        } else {
            "Protect folder"
        },
        if reference { Ink::Text } else { Ink::Muted },
        {
            let path = path.clone();
            move || actions::toggle_reference_path(&path)
        },
    ))
    .child(row_icon_button(
        icons::CLEAR,
        "Remove folder",
        Ink::Muted,
        {
            let path = path.clone();
            move || actions::remove_folder_path(&path)
        },
    ))
}

/// The note at the bottom: what the lock on a folder row does. A sentence
/// that points at the real control beats an affordance that cannot do the
/// thing it hints at.
fn tip_card() -> impl IntoElement {
    view().px(px(space::SM)).pb(px(space::MD)).child(
        notice().child(panel_title("Protect the originals")).child(caption(
            "Press the lock on a folder row: its copies win the keep decision, and nothing inside it is ever deleted.",
        )),
    )
}
