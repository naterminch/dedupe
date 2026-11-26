//! History page: every clean-up dedupe ran, newest first, and the way back.
//!
//! Each run is one row of a ruled card: what happened, when, where, how much,
//! and what is already back. A run that can still come back offers
//! **Put back…**, which opens a confirmation listing every file that comes
//! back and every one that cannot, with the reason in plain words; a file can
//! be left out of it. The list is the history file's own (at most
//! [`crate::history::MAX_RUNS`] runs), so it is built whole like the app's
//! other bounded lists.

use std::cell::Cell;
use std::path::Path;
use std::sync::Arc;

use nsh_core::prelude::*;

use crate::actions::{CAN_RESTORE_TRASH, PlannedItem};
use crate::history::{self, Method, Run};
use crate::ui::actions;
use crate::ui::palette::space;
use crate::ui::state::app;
use crate::ui::widgets::{
    alert_dialog, amount, caption, card, description, divider, fading_in, fast_scroll,
    fast_scroll_max, field_label, heading, hint, panel_title, primary_button, quiet_button,
    secondary_button, section_title, title_block,
};
use crate::ui_helpers::{counted, fit_text};
use crate::util;

// Where the page and the restore dialog's list sit, as the wheel last left
// them (`fast_scroll`).
thread_local! {
    static HISTORY_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
    static RESTORE_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
}

/// Puts the page back at its top: leaving it unmounts the list.
pub(crate) fn reset_scroll() {
    HISTORY_AT.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
}

/// Puts the restore dialog's list back at its top: it opens anew each time.
pub(crate) fn reset_restore_scroll() {
    RESTORE_AT.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
}

/// About how many characters of a folder list a row shows before the front
/// is cut.
const ROOTS_CHARS: usize = 70;

/// About how many characters of a folder the restore dialog shows under a
/// file's name.
const FOLDER_CHARS: usize = 60;

/// Files the restore dialog draws per section before it counts the rest.
const RESTORE_ROWS: usize = 100;

/// The tallest the restore dialog's list grows before it scrolls, so its
/// buttons stay on screen at the default window height.
const RESTORE_LIST_MAX_H: f32 = 320.0;

pub(crate) fn history_page() -> View {
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .min_w(px(0.0))
        .gap(px(space::XL))
        .px(px(space::MA))
        .py(px(space::XXL))
        .child(title_block(
            heading("History"),
            description(|| {
                app().with(|s| match s.history.len() {
                    0 => "Every clean-up you run is listed here, so you can put back what went."
                        .to_string(),
                    n => format!(
                        "{}, newest first. Put back what can still come back.",
                        counted(n as u64, "clean-up", "clean-ups")
                    ),
                })
            }),
        ))
        .child(fast_scroll(
            &HISTORY_AT,
            dynamic(|| {
                // A recorded run or a restore swaps the history for a new
                // list; nothing else rebuilds the rows under the pointer.
                let shown = computed(|| app().with(|s| Arc::as_ptr(&s.history) as usize));
                shown.get();
                let runs = app().with_untracked(|s| Arc::clone(&s.history));
                let fade = fading_in();
                run_list(&runs).bind_motion(&fade)
            }),
        ))
}

fn run_list(runs: &[Run]) -> View {
    if runs.is_empty() {
        return view().child(
            card().gap(px(space::XS)).p(px(space::XL)).child(section_title("Nothing here yet")).child(
                description(
                    "Deleting or moving copies, linking them, gathering keepers and removing photo metadata each leave a line here — from this app and from the command line — with what went and how to put it back.",
                ),
            ),
        );
    }
    let mut rows = card().p(px(space::XS));
    for (index, run) in runs.iter().enumerate() {
        if index > 0 {
            rows = rows.child(view().px(px(space::SM)).child(divider()));
        }
        rows = rows.child(run_row(run));
    }
    view().flex_col().child(rows)
}

/// One run: what happened over when, where and what is back; its size on the
/// right edge every row shares; then its way back, or why there is none.
fn run_row(run: &Run) -> View {
    let mut line = vec![history::friendly_stamp(run.when)];
    if let Some(roots) = roots_label(run) {
        line.push(roots);
    }
    match run.back() {
        0 => {}
        n if n == run.items.len() => line.push("all back".to_string()),
        n => line.push(format!("{n} of {} back", run.items.len())),
    }
    let id = run.id;
    let way_back = if actions::can_come_back(run) {
        secondary_button("Put back…", move || actions::request_restore(id)).tooltip(hint(
            "See what comes back and what cannot before anything moves",
        ))
    } else {
        view().flex_shrink_0().child(caption(no_way_back(run)))
    };
    view()
        .flex_row()
        .items_center()
        .gap(px(space::MD))
        .px(px(space::SM))
        .py(px(space::SM))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(space::XXS))
                .child(panel_title(run.describe()).truncate())
                .child(caption(line.join(" · ")).truncate()),
        )
        .child(amount(run.size_label()))
        .child(way_back)
}

/// The folders the run's scan looked at, cut to the row.
fn roots_label(run: &Run) -> Option<String> {
    let first = run.roots.first()?.display().to_string();
    let label = match run.roots.len() {
        1 => format!("in {first}"),
        n => format!("in {first} and {}", counted(n as u64 - 1, "more", "more")),
    };
    Some(fit_text(&label, ROOTS_CHARS))
}

/// Why a run offers no way back, in a few words.
fn no_way_back(run: &Run) -> &'static str {
    if run.back() == run.items.len() {
        return "All back";
    }
    match run.method {
        Method::Trash if !CAN_RESTORE_TRASH => "Put back from the Trash",
        Method::Permanent => "Deleted for good",
        Method::Linked => "Hard links stay",
        Method::Rewritten => "Can't be undone",
        Method::Trash | Method::Moved => "Nothing left to put back",
    }
}

/// The restore confirmation: what comes back and what cannot, file by file,
/// before anything moves. A file that can come back can be left out.
pub(crate) fn restore_confirm() -> View {
    let Some(ask) = app().with_untracked(|s| s.restore.clone()) else {
        return view();
    };
    let (ready, cannot): (Vec<&PlannedItem>, Vec<&PlannedItem>) =
        ask.items.iter().partition(|item| item.verdict.comes_back());
    let chosen: Vec<&&PlannedItem> = ready
        .iter()
        .filter(|item| !ask.left_out.contains(&item.path))
        .collect();
    let count = chosen.len() as u64;
    let bytes: u64 = chosen.iter().map(|item| item.size).sum();

    let mut body = view().flex_col().w_full().gap(px(space::MD));
    if !ready.is_empty() {
        body = body.child(field_label(format!("Comes back · {}", ready.len())));
        for item in ready.iter().take(RESTORE_ROWS) {
            let left_out = ask.left_out.contains(&item.path);
            body = body.child(file_row(item, Some(left_out)));
        }
        if ready.len() > RESTORE_ROWS {
            body = body.child(caption(format!(
                "… and {} more, which come back too",
                counted((ready.len() - RESTORE_ROWS) as u64, "file", "files")
            )));
        }
    }
    if !cannot.is_empty() {
        body = body.child(field_label(format!("Can't come back · {}", cannot.len())));
        for item in cannot.iter().take(RESTORE_ROWS) {
            body = body.child(file_row(item, None));
        }
        if cannot.len() > RESTORE_ROWS {
            body = body.child(caption(format!(
                "… and {} more",
                counted((cannot.len() - RESTORE_ROWS) as u64, "file", "files")
            )));
        }
    }

    let what = format!(
        "{} ({}).",
        ask.run.describe(),
        history::friendly_stamp(ask.run.when)
    );
    let (title, summary) = if ready.is_empty() {
        (
            "Nothing here can come back".to_string(),
            format!("{what} The list says why for each file."),
        )
    } else {
        let mut summary = format!(
            "{what} {} ({}) go back to the folders they came from. Nothing is overwritten: a file whose place is taken by then stays where it is.",
            counted(count, "file", "files"),
            util::human_bytes(bytes)
        );
        if !cannot.is_empty() {
            summary.push_str(&format!(
                " {} can't come back; the list says why.",
                counted(cannot.len() as u64, "file", "files")
            ));
        }
        (
            format!("Put back {}?", counted(count, "file", "files")),
            summary,
        )
    };
    let mut buttons = view().child(secondary_button(
        if ready.is_empty() { "Close" } else { "Cancel" },
        actions::cancel_restore,
    ));
    if count > 0 {
        buttons = buttons.child(primary_button(
            format!("Put back {count}"),
            actions::confirm_restore,
        ));
    }
    alert_dialog(
        title,
        summary,
        fast_scroll_max(&RESTORE_AT, px(RESTORE_LIST_MAX_H), body),
        buttons,
        actions::cancel_restore,
    )
}

/// One file of the restore: its name over its folder and what happens to it,
/// its size, and — for a file that can come back — the choice to leave it.
fn file_row(item: &PlannedItem, left_out: Option<bool>) -> View {
    let name = file_name(&item.path);
    let folder = item
        .path
        .parent()
        .map_or_else(String::new, |p| p.display().to_string());
    let says = if left_out == Some(true) {
        "Stays where it is now".to_string()
    } else {
        item.verdict.says()
    };
    let mut row = view()
        .flex_row()
        .items_center()
        .gap(px(space::SM))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(space::XXS))
                .child(panel_title(name).truncate())
                .child(caption(fit_text(&folder, FOLDER_CHARS)).truncate())
                .child(caption(says.clone()).truncate().tooltip(hint(says))),
        )
        .child(caption(util::human_bytes(item.size)));
    if let Some(left_out) = left_out {
        let path = item.path.clone();
        row = row.child(quiet_button(
            if left_out { "Put back too" } else { "Leave it" },
            move || actions::toggle_restore_item(&path),
        ));
    }
    row
}

/// A path's last name, for a row's title.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
