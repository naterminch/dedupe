//! Main results pane: first-run guidance, live progress, duplicate groups and
//! cleanup findings.
//!
//! Group rows are built inside one `dynamic` region: every read of [`app`]
//! subscribes, so a scan, a keeper swap or a tick re-renders the list.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use nsh_base::{
    SelectItem, checkbox, collapsible, dialog, number_input, progress, select, toggle,
    toggle_group, toolbar, toolbar_group,
};
use nsh_core::AccessibilityState;
use nsh_core::element::{BorderCurve, ImageSrc, input, video};
use nsh_core::prelude::*;

use crate::matching::{Group, GroupMember};
use crate::media;
use crate::modes::{ModeResult, mode_title};
use crate::ui::actions;
use crate::ui::palette::{colors, radius, space, text};
use crate::ui::state::{Layout, app, scan_status};
use crate::ui::widgets::{
    bezel, bezel_core, caption, card, chevron, chip, danger_button, divider, entering, fading_in,
    fast_scroll, fast_scroll_by, fast_scroll_max, field_label, ghost_button, hint, icon,
    icon_button, icon_chip, icon_ghost_button, icon_pill_button, icon_secondary_button, icons,
    motion, primary_button, quiet_button, secondary_button, selection, timing,
};
use crate::ui_helpers::{counted, fit_text};
use crate::util;

const KEEP_MODES: [&str; 5] = ["First", "Smallest", "Newest", "Oldest", "Best"];

/// Photo extensions the image element decodes. Formats it cannot (HEIC, SVG,
/// ICO, TIFF) keep the text-only row rather than a broken box.
const THUMB_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "bmp", "webp"];

/// Row thumbnail box, in points. Fixed so an image finishing its async load
/// never changes the row's geometry (rows would overlap mid-layout).
const THUMB_W: f32 = 72.0;
const THUMB_H: f32 = 56.0;

/// Height of one group card, which is what one wheel notch moves: a photo card
/// in grid layout carries a large preview, a compact card two plain rows.
const CARD_STEP_GRID: f32 = 355.0;
const CARD_STEP_LIST: f32 = 240.0;

/// What preview a member row shows: the photo itself, a video's extracted
/// poster frame, a placeholder while that poster extracts, or nothing for
/// plain files.
enum Thumb {
    None,
    Image,
    Poster(PathBuf),
    PendingVideo,
}

/// Decide the preview for one member.
///
/// Videos cannot be decoded by the UI toolkit, so they show a poster frame
/// ffmpeg extracts into the shared state; until it lands the row shows a
/// placeholder. Photos render straight from the file path.
fn thumb_for(member: &GroupMember, poster: Option<&Path>) -> Thumb {
    match media::classify(&member.path) {
        media::MediaKind::Image => {
            // Only files that are still on disk become image sources: a
            // member whose file vanished (deleted in another app, an
            // unmounted drive) keeps the text row instead of asking the
            // backend to load a path that is not there.
            let thumbable = member
                .path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| THUMB_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                && member.path.is_file();
            if thumbable { Thumb::Image } else { Thumb::None }
        }
        // The poster frame (when ffmpeg extracted one) is part of the card's
        // own signature, so it swaps in without the group rebuilding.
        media::MediaKind::Video => poster.map_or(Thumb::PendingVideo, |frame| {
            Thumb::Poster(frame.to_path_buf())
        }),
        media::MediaKind::Other => Thumb::None,
    }
}

/// The thumbnail for a member row: the photo itself, a video's poster, or a
/// plain box (with a play glyph) for files that have no preview.
fn member_thumb(thumb: &Thumb, path: &Path) -> View {
    let frame = view()
        .w(px(THUMB_W))
        .h(px(THUMB_H))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(radius::CONTROL))
        .bg(colors::INSET);
    match thumb {
        Thumb::Image => frame.child(preview_image(path)),
        Thumb::Poster(poster) => frame.child(preview_image(poster)),
        Thumb::None => frame.border_1().border_color(colors::BORDER),
        Thumb::PendingVideo => frame.border_1().border_color(colors::BORDER).child(
            text("▶")
                .text_size(px(text::BODY))
                .text_color(colors::MUTED),
        ),
    }
}

/// A thumbnail image: NSH reads and downsamples the file off the UI thread
/// (WIC on Windows, the system decoders elsewhere) and caches it, and a plain
/// filesystem path is a valid image source. Sized and object-fitted on the
/// image itself, with no clipping wrapper around it.
fn preview_image(path: &Path) -> impl IntoElement {
    image(path.to_string_lossy().into_owned())
        .w(px(THUMB_W))
        .h(px(THUMB_H))
        .flex_shrink_0()
        .bg(colors::INSET)
        .rounded(px(radius::CONTROL))
        .object_fit(ResizeMode::Cover)
}

pub(crate) fn results() -> View {
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .bg(colors::BACKGROUND)
        .child(dynamic(|| {
            // Which workspace is up, not its progress: a scan tick must not
            // swap these panes out from under a press.
            let mode = computed(|| app().with(|s| s.mode_result.is_some()));
            if mode.get() {
                mode_workspace()
            } else {
                duplicate_workspace()
            }
        }))
}

fn kicker(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::MUTED)
}

fn title(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT)
}

// Where each long list sits, as the wheel last left it. The wheel is the
// app's over these lists (`fast_scroll`): `scroll_to` takes an absolute
// offset, so the pane keeps the one it moved to.
thread_local! {
    static RESULTS_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
    static MODE_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
    static DELETE_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
}

/// Puts the delete dialog's list back at its top: it opens anew each time,
/// while the wheel's remembered offset outlives it.
pub(crate) fn reset_delete_scroll() {
    DELETE_AT.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
}

/// Puts the workspace's lists back at their top: leaving the page unmounts
/// them, so the wheel's remembered offsets would point into a list that opens
/// at the start again.
pub(crate) fn reset_lists() {
    for at in [&RESULTS_AT, &MODE_AT, &DELETE_AT] {
        at.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
    }
}

fn duplicate_workspace() -> View {
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .gap(px(space::XL))
        .p(px(space::XXL))
        // Before the first scan the workspace is a centered hero: mascot,
        // question, option pills and the scan composer. Once a scan runs the
        // toolbar takes over and the hero never returns.
        .child(dynamic(|| {
            // The toolbar follows the page's shape, not its progress: reading
            // the app signal directly rebuilt these buttons on every scan tick,
            // which restarted their hover and dropped presses.
            let shape = computed(|| app().with(|s| (s.scanning, s.has_scanned)));
            let (scanning, has_scanned) = shape.get();
            if scanning || has_scanned {
                view()
                    .flex_col()
                    .gap(px(space::LG))
                    .child(header_row())
                    .child(action_row())
            } else {
                view()
            }
        }))
        // Rebuild only when the results change shape: a scan, a threshold, a
        // delete, a consolidate, a re-sort, Show more. Keeper picks and ticks
        // update their own cards, so pressing Keep never rebuilds (or
        // reorders) the list.
        .child(fast_scroll_by(
            &RESULTS_AT,
            card_step,
            dynamic(|| {
                let signature = computed(|| {
                    app().with(|s| {
                        (
                            s.results_rev,
                            s.scanning,
                            s.has_scanned,
                            s.groups.len(),
                            s.group_limit,
                            s.ffprobe_note,
                        )
                    })
                });
                list_body(signature.get())
            }),
        ))
}

fn header_row() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .justify_between()
        .items_end()
        .gap(px(space::LG))
        .child(
            view()
                .flex_col()
                .min_w(px(0.0))
                .flex_1()
                .gap(px(space::XS))
                .child(title(|| {
                    if app().with(|s| s.has_scanned) {
                        "Review duplicates"
                    } else {
                        "Find duplicate files"
                    }
                }))
                .child(summary_line()),
        )
        .child(list_actions())
}

/// The one line under the title: what the scan found, ordered the way a
/// reader acts on it — how many groups, how many copies go, how much space
/// that frees. It replaces the chip row that repeated the same three numbers
/// a second time above the list, and with them a one-line hash explainer that
/// belongs where the hash is chosen (Settings, Preferences).
fn summary_line() -> impl IntoElement {
    caption(|| {
        let state = app();
        if state.with(|s| s.scanning) {
            "Scanning. The list appears when it finishes.".to_string()
        } else {
            let groups = state.with(|s| s.groups.len());
            let dups = state.with(|s| s.dup_files);
            let bytes = state.with(|s| s.reclaim_bytes);
            format!(
                "{} · {} · {} reclaimable",
                counted(groups as u64, "group", "groups"),
                counted(dups, "duplicate", "duplicates"),
                util::human_bytes(bytes)
            )
        }
    })
}

/// The list's own controls, held in one pill so four unlabelled glyphs read
/// as one control group instead of four buttons floating at the page edges:
/// the disclosure pair on the left, the selection pair on the right, one
/// hairline between them.
fn list_actions() -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::XXS))
        .p(px(space::XXS))
        .bg(colors::INSET)
        .border_1()
        .border_color(colors::HAIRLINE)
        .rounded_full()
        .child(icon_ghost_button(
            icons::EXPAND,
            "Expand all groups",
            || actions::set_all_expanded(true),
        ))
        .child(icon_ghost_button(
            icons::COLLAPSE,
            "Collapse all groups",
            || actions::set_all_expanded(false),
        ))
        .child(view().w(px(1.0)).h(px(16.0)).bg(colors::BORDER))
        .child(icon_ghost_button(
            icons::SELECT_ALL,
            "Select all duplicates",
            || app().update(actions::reselect_all_dups),
        ))
        .child(icon_ghost_button(
            icons::CLEAR,
            "Clear selection",
            actions::select_none,
        ))
}

/// Two bands, not one: what you do to the results, then what shapes them.
///
/// All fifteen controls used to share a line, so a red Delete competed with
/// the similarity box for the same glance and the row wrapped into an
/// unpredictable second line. Actions run along the top — Back and Delete
/// together on the left, the two whole-set commands on the right — and the
/// options that only tune the list sit below a hairline in nsh's smaller
/// toolbar row, their chips one step quieter than the actions above them.
fn action_row() -> impl IntoElement {
    view()
        .flex_col()
        .w_full()
        .gap(px(space::MD))
        .child(result_actions())
        .child(
            view()
                .w_full()
                .border_t_1()
                .border_color(colors::HAIRLINE)
                .pt(px(space::MD))
                .child(scan_options()),
        )
}

/// The actions that act on the whole result set. nsh's toolbar gives them one
/// roving keyboard stop; the groups mark the two clusters — destructive
/// changes to the files on the left, moving and reporting on the right.
fn result_actions() -> impl IntoElement {
    toolbar()
        .accessibility_label("Results actions")
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .w_full()
        .gap(px(space::MD))
        .child(
            toolbar_group()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                // Back sits with Delete, not in the header: both act on the
                // whole result set, and the pill matches the row's sizing.
                .child(back_button())
                .child(delete_bar()),
        )
        .child(
            toolbar_group()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                .child(consolidate_button())
                .child(icon_pill_button(
                    icons::EXPORT,
                    "Export JSON report",
                    actions::save_report,
                )),
        )
}

/// The scan options: the similarity floor on the left with the keeper rule
/// beside it, the sort and layout choices pushed to the far edge. They wrap
/// only when the five keep chips run out of room, so the row stays one line
/// at the widths the window actually opens at.
fn scan_options() -> impl IntoElement {
    toolbar()
        .accessibility_label("Scan options")
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .w_full()
        .gap(px(space::MD))
        .child(
            toolbar_group()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(space::MD))
                .child(sim_filter())
                .child(keep_modes()),
        )
        .child(
            toolbar_group()
                .flex_row()
                .items_center()
                .gap(px(space::XXS))
                .child(sort_toggle())
                .child(layout_toggle()),
        )
}

/// Re-running a finished scan. In the cleanup workspace the findings are
/// read-only and there is no composer, so its header carries the next run
/// itself. Hidden while a scan runs, where the progress card's Stop is the
/// only scan action. The duplicates page takes the same action from the
/// composer it keeps on screen.
fn scan_again_button() -> impl IntoElement {
    dynamic(|| {
        let scanning = computed(|| app().with(|s| s.scanning));
        when(!scanning.get(), || {
            secondary_button("Scan again", actions::start_scan)
        })
    })
}

/// Leave the results for the setup page. Hidden while a scan runs, where the
/// progress card's Stop owns cancelling.
fn back_button() -> impl IntoElement {
    when(!app().with(|s| s.scanning), || {
        icon_secondary_button(icons::ARROW_LEFT, "Back", actions::back_to_setup)
            .tooltip(hint("Back to scan setup"))
    })
}

fn sim_filter() -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::SM))
        .child(field_label("Similarity"))
        .child(
            number_input()
                .value(|| app().with(|s| s.min_sim_input.trim().parse::<f64>().ok()))
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .placeholder("97")
                .on_change(|next: &Option<f64>| {
                    let text = match next {
                        Some(value) if value.fract() == 0.0 => format!("{}", *value as i64),
                        Some(value) => format!("{value}"),
                        None => String::new(),
                    };
                    app().update(|s| s.min_sim_input = text);
                })
                .input_style(|s| s.flex_1().text_center().text_color(colors::TEXT))
                .decrement_button(text("−").text_color(colors::MUTED))
                .increment_button(text("+").text_color(colors::MUTED))
                .decrement_button_style(|s| s.w(px(20.0)).items_center().justify_center())
                .increment_button_style(|s| s.w(px(20.0)).items_center().justify_center())
                .flex_row()
                .w(px(120.0))
                .h(px(space::XXL))
                .bg(colors::INSET)
                .border_1()
                .border_color(colors::BORDER)
                .rounded(px(radius::CONTROL)),
        )
        .child(icon_button(icons::CHECK, "Apply similarity", || {
            let raw = app().with(|s| s.min_sim_input.clone());
            actions::apply_sim_input(&raw);
        }))
}

/// One keeper rule at a time: nsh's toggle group is the view switcher here
/// (a press releases the others and takes the new rule).
fn keep_modes() -> View {
    view().child(dynamic(|| {
        // The rule alone rebuilds these chips; a scan tick would restart
        // their hover and drop a press mid-flight.
        let current = computed(|| app().with(|s| s.keep_index.unwrap_or(0)));
        let current = current.get();
        let mut row = view().flex_row().flex_wrap().gap(px(2.0));
        for (index, label) in KEEP_MODES.iter().enumerate() {
            row = row.child(keep_chip(label, index, current == index));
        }
        toggle_group().child(row)
    }))
}

fn keep_chip(label: &'static str, index: usize, active: bool) -> impl IntoElement {
    let (fill, border, content) = selection(active);
    toggle()
        .pressed(active)
        // Only the press that takes this rule counts: releasing it would
        // leave the set with nothing chosen.
        .on_change(move |next: &bool| {
            if *next {
                actions::apply_keep_mode(index);
            }
        })
        .accessibility_label(label)
        .px(px(space::SM))
        .py(px(space::XS))
        .rounded_full()
        .bg(fill)
        .border_1()
        .border_color(border)
        // Picking another rule cross-fades the chosen pill.
        .transition_colors()
        .duration(motion(timing::FAST))
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(content),
        )
}

/// The sort rule as one text chip: same metrics and selection colours as the
/// keep-mode and layout chips beside it, so the bar reads as one row of pills.
fn sort_toggle() -> impl IntoElement {
    dynamic(|| {
        let biggest = computed(|| app().with(|s| s.sort_biggest));
        let biggest = biggest.get();
        let (fill, border, content) = selection(biggest);
        toggle()
            .pressed(biggest)
            .on_change(move |_: &bool| actions::toggle_sort_biggest())
            .accessibility_label("Sort groups by most reclaimable space first")
            .flex_row()
            .items_center()
            .justify_center()
            .px(px(space::SM))
            .py(px(space::XS))
            .rounded_full()
            .bg(fill)
            .border_1()
            .border_color(border)
            .transition_colors()
            .duration(motion(timing::FAST))
            .active(|s| s.opacity(0.9))
            .child(
                // A bare "Path" read as another choice in the keeper group
                // beside it; the chip says what it sorts.
                text(if biggest {
                    "Sort: Biggest"
                } else {
                    "Sort: Path"
                })
                .text_size(px(text::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(content),
            )
    })
}

/// Grid or list: the original two-column preview cards, or one compact row
/// list. It is a structural choice, so switching rebuilds the list.
fn layout_toggle() -> View {
    view().child(dynamic(|| {
        let current = computed(|| app().with(|s| s.layout));
        let current = current.get();
        let mut row = view().flex_row().gap(px(2.0));
        for (layout, markup, label) in [
            (Layout::Grid, icons::GRID, "Grid layout"),
            (Layout::List, icons::LIST, "List layout"),
        ] {
            row = row.child(icon_chip(markup, label, current == layout, move || {
                actions::set_layout(layout)
            }));
        }
        toggle_group().child(row)
    }))
}

fn delete_bar() -> View {
    view().child(dynamic(|| {
        // The selection and the trash switch decide this bar. A scan tick
        // rebuilding it replaced the Delete button under the pointer.
        let targets = computed(|| {
            app().with(|s| {
                let targets = actions::pending_targets(s);
                (
                    targets.len(),
                    targets.iter().map(|t| t.size).sum::<u64>(),
                    !s.last_trashed.is_empty(),
                    s.trash,
                )
            })
        });
        let (count, bytes, can_undo, trash) = targets.get();
        let mut row = view().flex_row().gap(px(4.0)).items_center();
        if count > 0 {
            let label = format!("Delete {count} ({})", util::human_bytes(bytes));
            let unit = if count == 1 { "file" } else { "files" };
            let tip = if trash {
                format!(
                    "Move {count} {unit} ({}) to the recycle bin",
                    util::human_bytes(bytes)
                )
            } else {
                format!(
                    "Permanently delete {count} {unit} ({})",
                    util::human_bytes(bytes)
                )
            };
            row = row.child(danger_button(label, actions::request_delete).tooltip(hint(tip)));
        }
        if can_undo {
            row = row.child(icon_pill_button(
                icons::UNDO,
                "Undo delete",
                actions::undo_delete,
            ));
        }
        row
    }))
}

/// The delete confirmation: a per-group dry run of exactly what will go,
/// with in-dialog Keep picks. Nothing is removed until "Delete N" is
/// pressed, and every file is re-hashed immediately before it is.
///
/// Reads its plan from the live selection, so ticking a box or changing a
/// keeper behind the dialog is reflected here instead of going stale.
pub(crate) fn delete_confirm() -> View {
    let (count, bytes, groups, hidden_groups, trash) = app().with(|s| {
        let targets = actions::pending_targets(s);
        let (groups, hidden_groups) = actions::confirm_groups(s);
        (
            targets.len(),
            targets.iter().map(|t| t.size).sum::<u64>(),
            groups,
            hidden_groups,
            s.trash,
        )
    });
    let mut body = view().flex_col().gap(px(space::MD));
    for (position, group) in groups.iter().enumerate() {
        let going = group.rows.len() + group.hidden_rows;
        // Each group reads as the pair it is: what stays, then what goes.
        // The old line ran both facts together ("Group #1 - keeps a - deletes
        // 1"), which is a sentence to parse at the moment of confirming a
        // deletion.
        let mut section = view()
            .flex_col()
            .gap(px(space::XS))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(space::SM))
                    .child(
                        text(format!("Keeping {}", group.keeper))
                            .flex_1()
                            .truncate()
                            .text_size(px(text::CAPTION))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors::TEXT),
                    )
                    .child(chip("KEEP", colors::KEEP_WASH_STRONG, colors::KEEP_INK)),
            )
            .child(caption(format!(
                "{} · {}",
                counted(going as u64, "copy goes", "copies go"),
                util::human_bytes(group.bytes),
            )));
        for path in &group.rows {
            let full = path.display().to_string();
            let shown = fit_text(&full, 56);
            let truncated = shown != full;
            let cell = text(shown)
                .flex_1()
                .truncate()
                .text_size(px(text::CAPTION))
                .text_color(colors::MUTED);
            let cell = if truncated {
                cell.tooltip(hint(full))
            } else {
                cell
            };
            let index = group.index;
            let keep_path = path.clone();
            section = section.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(space::SM))
                    .child(cell)
                    .child(quiet_button("Keep instead", move || {
                        actions::set_keeper(index, &keep_path);
                    })),
            );
        }
        if group.hidden_rows > 0 {
            section = section.child(caption(format!(
                "... and {} more in this group",
                group.hidden_rows
            )));
        }
        // One rule between plans: two groups of paths ran together as one
        // block, and the keeper line of the second looked like a repeat of
        // the first group's last row.
        if position > 0 {
            body = body.child(divider());
        }
        body = body.child(section);
    }
    if hidden_groups > 0 {
        body = body.child(caption(format!(
            "... and {} more",
            counted(hidden_groups as u64, "group", "groups")
        )));
    }
    if count == 0 {
        body = body.child(caption("Nothing is ticked for deletion."));
    }
    let description = if count == 0 {
        "Ticking a duplicate in a group lists it here.".to_string()
    } else {
        let files = counted(count as u64, "file", "files");
        let action = if trash {
            "go to the trash"
        } else {
            "are deleted permanently"
        };
        format!(
            "{files} ({}) {action}. Keepers stay, and every file is re-hashed before it goes.",
            util::human_bytes(bytes)
        )
    };
    let heading = format!("Delete {}?", counted(count as u64, "file", "files"));
    let panel = card()
        .w(px(520.0))
        .flex_shrink_0()
        .gap(px(space::SM))
        .p(px(space::LG))
        .child(
            text(heading.clone())
                .text_size(px(text::HEADING))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::TEXT),
        )
        .child(caption(description))
        .child(fast_scroll_max(&DELETE_AT, px(320.0), body))
        .child(divider())
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap(px(space::SM))
                .child(secondary_button("Cancel", actions::cancel_delete))
                .child(when(count > 0, move || {
                    danger_button(format!("Delete {count}"), actions::confirm_delete)
                })),
        );
    // nsh's dialog owns the modal layer: the focus moves into it, Escape and
    // the back button arrive as `on_cancel`, and screen readers hear an alert
    // dialog named by its title. The look is the app's: a scrim that takes the
    // presses which miss the panel, and a transparent centering layer
    // (box-none) so only the card is hit.
    //
    // The scrim sits in the page layer, below the dialog, not inside it: the
    // dialog's own entrance and exit slide its whole layer, so a scrim in
    // there travelled down the screen with the panel. Out here it stays put
    // and only the panel moves.
    // The scrim fades in under the panel. It stays put as the panel slides,
    // and it is the page's own layer, so it leaves when the dialog does.
    let scrim = fading_in();
    view()
        .child(
            overlay()
                .inset_0()
                .bg(colors::SCRIM)
                .on_press(actions::cancel_delete)
                .bind_motion(&scrim),
        )
        .child(
            dialog()
                .role(AccessibilityRole::AlertDialog)
                .label(heading.clone())
                .on_cancel(actions::cancel_delete)
                // A press that lands on the modal layer itself still closes
                // it, as the scrim did when it lived in here.
                .child(overlay().inset_0().on_press(actions::cancel_delete))
                .child(
                    overlay()
                        .inset_0()
                        .pointer_events(PointerEvents::BoxNone)
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .p(px(space::XL))
                        .child(panel),
                ),
        )
}

fn consolidate_button() -> View {
    view().child(dynamic(|| {
        let dest = computed(|| app().with(|s| s.consolidate_dest.is_some()));
        if dest.get() {
            icon_pill_button(
                icons::FOLDER_MOVE,
                "Move keepers into the chosen folder",
                actions::perform_consolidate,
            )
        } else {
            icon_pill_button(
                icons::FOLDER,
                "Choose a folder to consolidate into",
                actions::pick_consolidate_dir,
            )
        }
    }))
}

/// What decides when the results list is rebuilt. It leaves keeper picks and
/// ticks out on purpose: those are member-level decisions ([`member_slot`])
/// and must not rebuild, or reorder, the list.
type ListSignature = (u64, bool, bool, usize, usize, bool);

fn list_body((_, scanning, has_scanned, total, limit, ffprobe_note): ListSignature) -> View {
    // The column owns the pane's width. Without it the list is content-sized,
    // so a wide row (the composer's controls) stretches the whole column past
    // the window instead of wrapping inside it.
    let mut list = view().flex_col().w_full().min_w(px(0.0)).gap(px(space::MD));
    if scanning {
        list = list.child(dynamic(progress_card));
    }
    if !has_scanned && !scanning {
        list = list.child(dynamic(hero));
    }
    if ffprobe_note {
        list = list.child(note_card(
            "ffprobe not found on PATH - install FFmpeg for media resolution/duration.",
        ));
    }
    if has_scanned && total == 0 {
        list = list.child(nothing_found_card(
            "No duplicates found",
            "Every file is unique at this similarity. Add another folder or loosen the threshold, then scan again.",
            true,
        ));
    }
    // Structural snapshot, read untracked: nothing here subscribes, so this
    // list only re-runs when the signature above changes.
    let (groups, layout) = app().with_untracked(|s| (s.groups.clone(), s.layout));
    for (position, group) in groups.iter().take(limit).enumerate() {
        list = list.child(group_slot(position, group, layout));
    }
    if total > limit {
        let remaining = total - limit;
        list = list.child(view().flex_row().justify_center().child(ghost_button(
            format!("Show more ({remaining} remaining)"),
            actions::show_more_groups,
        )));
    }
    // A rebuilt list — a scan landing, a new filter, a delete — fades in where
    // it stands. It fades rather than rises because it keeps the scroll's own
    // place; only the page itself travels.
    let fade = fading_in();
    list.bind_motion(&fade)
}

fn progress_card() -> View {
    // Progress has a signal of its own: reading it here subscribes this card
    // alone, so a tick never rebuilds the rest of the window.
    let (done, total, determinate, phase) =
        scan_status().with(|p| (p.done, p.total, p.determinate, p.phase.clone()));
    let fraction = if determinate && total > 0 {
        (done as f32 / total as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // A scan counts twice — files to hash, then media to compare — so the
    // bar legitimately restarts with a fresh total when the phase changes.
    // The phase is the heading and the counts are the caption, so that
    // restart reads as a new stage instead of the same bar going backwards.
    let label = if determinate && total > 0 {
        format!("{done} of {total}")
    } else {
        phase.clone()
    };
    card()
        .gap(px(space::SM))
        .p(px(space::MD))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(kicker(phase))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(space::MD))
                        .child(caption(label))
                        .child(danger_button("Stop", actions::stop_scan)),
                ),
        )
        .child(
            progress()
                .value(fraction * 100.0)
                .indeterminate(!determinate)
                .accessibility_label("Scan progress")
                .track_style(|s| {
                    s.h(px(6.0))
                        .w(px(320.0))
                        .rounded(px(3.0))
                        .bg(colors::INSET)
                        .overflow_hidden()
                })
                // A determinate bar glides between ticks instead of stepping
                // in 30 pt jumps; an indeterminate one shimmers.
                .indicator_class(if determinate {
                    "transition-all"
                } else {
                    "w-full animate-pulse"
                })
                .indicator_style(|s| {
                    s.h(px(6.0))
                        .rounded(px(3.0))
                        .bg(colors::ACCENT)
                        .duration(motion(timing::SLOW))
                }),
        )
}

/// The first-run page: the logo, the question, the two option pills and the
/// scan composer. One decision after another, then the field that starts the
/// work — no card in between repeating what the composer already says.
fn hero() -> View {
    // The first-run page arrives as one piece: logo, question and composer
    // rise together instead of appearing fully formed.
    let enter = entering();
    view()
        .flex_col()
        .items_center()
        .w_full()
        .gap(px(space::XL))
        .pt(px(space::XXL))
        .pb(px(space::XL))
        .child(logo_plinth())
        .child(
            text("What can I dedupe?")
                .text_size(px(text::PAGE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::TEXT),
        )
        .child(pill_row())
        .child(composer())
        .bind_motion(&enter)
}

/// The hero's mark, seated on its own plinth. The artwork is a white tile, so
/// dropped straight onto the page it reads as a bright block in the dark
/// appearance and as nothing at all on the light one. The tray-and-core bezel
/// frames it instead: the same machined nesting every card in the app uses,
/// with the tile in the middle, so it looks placed rather than pasted.
fn logo_plinth() -> View {
    bezel().flex_shrink_0().child(
        bezel_core()
            .w(px(112.0))
            .h(px(112.0))
            .items_center()
            .justify_center()
            .bg(colors::PANE)
            .child(hero_logo()),
    )
}

/// The app's own logo, embedded in the binary: a single-file install has no
/// asset directory to load it from. It is decoded once at twice the size the
/// hero draws and handed over as pixels, so the mark stays crisp on a 2x
/// display without keeping the 512 px original in memory.
fn hero_logo() -> impl IntoElement {
    static LOGO: OnceLock<(u32, u32, Arc<[u8]>)> = OnceLock::new();
    let (width, height, pixels) = LOGO.get_or_init(|| {
        const SIDE: u32 = 192;
        let decoded = ::image::load_from_memory(include_bytes!("../../assets/icons/logo.png"))
            .expect("the embedded logo is a PNG")
            .into_rgba8();
        let scaled = ::image::imageops::resize(
            &decoded,
            SIDE,
            SIDE,
            ::image::imageops::FilterType::Lanczos3,
        );
        let pixels: Arc<[u8]> = scaled.into_raw().into();
        (SIDE, SIDE, pixels)
    });
    image(ImageSrc::rgba(*width, *height, Arc::clone(pixels)))
        .w(px(96.0))
        .h(px(96.0))
        .flex_shrink_0()
        .object_fit(ResizeMode::Contain)
}

/// The two dropdowns under the question: what to look for, and where
/// deletes land. One choice each: a press opens nsh's Select list with the
/// chosen item ticked, and the keys drive it like a native select. Where to
/// scan lives in the sidebar and the composer, so it is not repeated here.
fn pill_row() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_center()
        .gap(px(space::SM))
        .child(find_select())
        .child(trash_select())
}

/// One choice out of `items`, as nsh's Select: nsh owns the behaviour (the
/// list, the keys, typeahead, what screen readers hear), the app supplies
/// the look — its trigger, the popup's surface, the rows' resting text — and
/// nsh's own state classes give the highlighted and chosen rows their state.
fn option_select(
    items: Vec<SelectItem>,
    accessibility: &'static str,
    value: impl Fn() -> Option<SharedString> + 'static,
    on_change: impl Fn(&Option<SharedString>) + 'static,
    trigger: View,
) -> impl IntoElement {
    select()
        .items(items)
        .value(value)
        .on_change(on_change)
        .accessibility_label(accessibility)
        // The list hangs just under the trigger rather than touching it.
        .offset(px(4.0))
        .popup_style(|s| {
            s.min_w(px(220.0))
                .p(px(space::XXS))
                .bg(colors::CARD)
                .border_1()
                .border_color(colors::BORDER)
                .rounded(px(radius::CARD))
                .shadow_md()
        })
        // `focus:` marks the row the keys or the pointer are on, `selected:`
        // the choice in effect: nsh applies them, the app only names them.
        .item_class("rounded-md focus:bg-accent selected:font-medium")
        .item_style(|s| {
            s.px(px(space::SM))
                .py(px(space::XS))
                .text_size(px(text::BODY))
                .text_color(colors::TEXT)
        })
        .trigger(trigger)
        .w(px(220.0))
}

/// What the next scan looks for, as one choice out of the scan modes.
fn find_select() -> impl IntoElement {
    let items: Vec<SelectItem> = crate::ui::settings::FIND_LABELS
        .iter()
        .map(|label| SelectItem::new(*label, *label))
        .collect();
    option_select(
        items,
        "What to find",
        || Some(find_label()),
        |picked: &Option<SharedString>| {
            let Some(picked) = picked.as_deref() else {
                return;
            };
            if let Some(index) = crate::ui::settings::FIND_LABELS
                .iter()
                .position(|label| *label == picked)
            {
                app().update(|s| s.find_index = Some(index));
                actions::save_prefs();
            }
        },
        dropdown_trigger(icons::SEARCH, find_label),
    )
}

/// Where a delete puts the files, as one choice of two.
fn trash_select() -> impl IntoElement {
    option_select(
        vec![
            SelectItem::new("trash", "Move to trash"),
            SelectItem::new("delete", "Delete permanently"),
        ],
        "Where deleted files go",
        || {
            Some(SharedString::from(if app().with(|s| s.trash) {
                "trash"
            } else {
                "delete"
            }))
        },
        |picked: &Option<SharedString>| {
            app().update(|s| s.trash = matches!(picked.as_deref(), Some("trash")));
            actions::save_prefs();
        },
        dropdown_trigger(icons::TRASH, || {
            SharedString::from(trash_label(app().with(|s| s.trash)))
        }),
    )
}

fn find_label() -> SharedString {
    let index = app().with(|s| s.find_index.unwrap_or(0));
    SharedString::from(crate::ui::settings::FIND_LABELS[index.min(10)])
}

fn trash_label(trash: bool) -> &'static str {
    if trash {
        "Move to trash"
    } else {
        "Delete permanently"
    }
}

/// The trigger a Select opens from: the pill's look (icon, current choice, a
/// chevron) as the combobox that takes the press.
fn dropdown_trigger(markup: &'static str, label: impl Fn() -> SharedString + 'static) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::XS))
        .px(px(space::MD))
        .py(px(space::XS))
        .bg(colors::INSET)
        .border_1()
        .border_color(colors::BORDER)
        .rounded(px(radius::CONTROL))
        .transition_colors()
        .duration(motion(timing::BASE))
        .ease_out()
        .hover(|s| s.bg(colors::ELEVATED))
        .child(icon(markup, 14.0, colors::MUTED))
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::TEXT),
        )
        .child(icon(icons::CHEVRON_DOWN, 13.0, colors::MUTED))
}

/// The scan composer: a big input over a control row, like the reference's
/// prompt card. The + button adds the typed path, the pills pick the keeper
/// default, the pill on the right sets the similarity floor, and the blue
/// button starts the scan.
///
/// It belongs to the first-run page alone: once results are on screen the
/// page is about acting on them, and the header's back arrow returns here to
/// set up the next run.
fn composer() -> View {
    card()
        .w_full()
        .max_w(px(860.0))
        .gap(px(space::MD))
        .p(px(space::LG))
        .child(composer_input())
        .child(caption(
            "Drop a folder here, or paste a path and press + to add it.",
        ))
        .child(divider())
        // The row reflows instead of forcing the card past the window edge:
        // the keep chips wrap among themselves, and the similarity pill and
        // the scan button move down as one group when they no longer fit.
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(space::SM))
                .w_full()
                .child(icon_button(
                    icons::PLUS,
                    "Add folder",
                    actions::add_from_input,
                ))
                .child(keep_modes())
                .child(view().flex_1().min_w(px(space::SM)))
                .child(
                    view()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(space::SM))
                        .child(media_status())
                        .child(sim_pill())
                        .child(primary_button(scan_label, actions::start_scan)),
                ),
        )
}

/// The folder path field of the composer.
fn composer_input() -> impl IntoElement {
    input()
        .value(|| app().with(|s| s.folder_input.clone()))
        .on_change(|value: &str| app().update(|s| s.folder_input = value.to_owned()))
        .placeholder("Scan a folder")
        .placeholder_color(|| color(colors::MUTED))
        .caret_color(|| color(colors::ACCENT))
        .selection_color(|| color(colors::ACCENT))
        .w_full()
        .h(px(space::XXL))
        .px(px(space::MD))
        .text_size(px(text::BODY))
        .text_color(colors::TEXT)
        // The hero's one input is a well, like every other field in the app:
        // bare on the card it read as a heading, with the hint below as its
        // caption and nothing saying "type here".
        .bg(colors::INSET)
        .border_1()
        .border_color(colors::HAIRLINE)
        .rounded(px(radius::CONTROL))
}

/// The similarity floor of a run: one value out of the five offered, as a
/// dropdown like the hero's.
fn sim_pill() -> impl IntoElement {
    let items: Vec<SelectItem> = SIM_STEPS
        .iter()
        .map(|step| SelectItem::new(step.to_string(), format!("{step}%")))
        .collect();
    select()
        .items(items)
        .value(|| {
            Some(SharedString::from(format!(
                "{}",
                current_similarity() as i64
            )))
        })
        .accessibility_label("Similarity floor")
        .w(px(150.0))
        .popup_style(|s| {
            s.min_w(px(150.0))
                .p(px(space::XS))
                .bg(colors::CARD)
                .border_1()
                .border_color(colors::BORDER)
                .rounded(px(radius::CARD))
        })
        .item_style(|s| {
            s.px(px(space::SM))
                .py(px(space::XS))
                .rounded(px(radius::CONTROL))
                .text_size(px(text::BODY))
                .text_color(colors::TEXT)
        })
        .on_change(|picked: &Option<SharedString>| {
            let Some(picked) = picked else {
                return;
            };
            let text = picked.to_string();
            app().update(|s| s.min_sim_input = text.clone());
            actions::apply_sim_input(&text);
        })
        .trigger(dropdown_trigger(icons::SLIDERS, || {
            SharedString::from(format!("{}%", current_similarity() as i64))
        }))
}

/// The similarity steps the composer offers, loosest last.
const SIM_STEPS: [f64; 5] = [100.0, 97.0, 95.0, 90.0, 85.0];

fn current_similarity() -> f64 {
    app()
        .with(|s| s.min_sim_input.trim().parse::<f64>().ok())
        .unwrap_or(97.0)
}

/// The scan button's label, per scan type. Read inside the hero's dynamic
/// region, so cycling the mode pill relabels it without a rebuild.
fn scan_label() -> &'static str {
    if crate::ui_helpers::find_mode(app().with(|s| s.find_index))
        == crate::cli::FindMode::Duplicates
    {
        "Scan for duplicates"
    } else {
        "Run cleanup scan"
    }
}

/// ffmpeg/ffprobe availability as one compact chip in the composer row:
/// media comparison and metadata quietly depend on them.
fn media_status() -> impl IntoElement {
    dynamic(|| {
        let (checked, ffmpeg, ffprobe) =
            app().with(|s| (s.media_checked, s.ffmpeg_ok, s.ffprobe_ok));
        if !checked {
            return chip("Checking ffmpeg…", colors::INSET, colors::MUTED);
        }
        match (ffmpeg, ffprobe) {
            (true, true) => chip(
                "ffmpeg ready",
                colors::ACCENT_WASH_STRONG,
                colors::ACCENT_INK,
            ),
            (true, false) => chip("No ffprobe", colors::INSET, colors::MUTED),
            (false, _) => chip("No ffmpeg", colors::INSET, colors::MUTED),
        }
    })
}

/// A scan that found nothing: what to change and where. `rescan` adds the
/// button that runs the same scan again — on the duplicates page the composer
/// lives back on the setup page, so without it this state is a dead end. The
/// cleanup workspace passes `false`: its header already carries the rerun.
fn nothing_found_card(message: &'static str, hint: &'static str, rescan: bool) -> View {
    let mut card = card()
        .gap(px(space::MD))
        .p(px(space::LG))
        .child(
            text(message)
                .text_size(px(text::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::TEXT),
        )
        .child(caption(hint));
    if rescan {
        card = card.child(
            view()
                .flex_row()
                .child(primary_button("Scan again", actions::start_scan)),
        );
    }
    card
}

fn note_card(message: &'static str) -> View {
    card()
        .p(px(space::MD))
        .bg(colors::INSET)
        .child(caption(message))
}

/// One group card, as the wheel's step: the layout decides a card's height,
/// and a group whose members have no preview stays a compact card even in
/// grid layout.
fn card_step() -> f32 {
    app().with_untracked(|s| {
        let grid = s.layout == Layout::Grid
            && s.groups.first().is_some_and(|g| {
                g.members.len() >= 2
                    && g.members
                        .iter()
                        .all(|m| is_grid_preview(&thumb_for(m, None)))
            });
        if grid { CARD_STEP_GRID } else { CARD_STEP_LIST }
    })
}

/// The group's own line: how many copies, how big each one is, and how close
/// they are when the group came from the similarity pass. No group number —
/// the reader acts on files, not on row ids.
fn group_header(group: &Group) -> String {
    let copies = group.members.len();
    let first = group.members.first().map(|m| m.size).unwrap_or(0);
    let size = if group.members.iter().all(|m| m.size == first) {
        format!(" · {} each", util::human_bytes(first))
    } else {
        String::new()
    };
    let similarity = group
        .similarity
        .map(|s| format!(" · {:.0}% similar", s * 100.0))
        .unwrap_or_default();
    format!(
        "{}{size}{similarity}",
        counted(copies as u64, "copy", "copies")
    )
}

/// One group's slot: rebuilt only when its disclosure flips. Keeper picks,
/// ticks and previews are member decisions and update their own cards.
///
/// Open is the default — `collapsed` holds only what the user closed — so a
/// freshly scanned group renders expanded without any per-group bookkeeping.
fn group_slot(position: usize, group: &Group, layout: Layout) -> View {
    let group = group.clone();
    let expanded = computed({
        let index = group.index;
        move || app().with(|s| !s.collapsed.contains(&index))
    });
    view().child(dynamic(move || {
        group_card(position, &group, expanded.get(), layout)
    }))
}

fn group_card(position: usize, group: &Group, expanded: bool, layout: Layout) -> View {
    let index = group.index;
    let header = group_header(group);
    let members = group.members.clone();
    // Grid is the original layout: media groups whose every member has a
    // preview become two-column cards with a large preview, and anything else
    // (plain files, archives, a group with a copy whose frame cannot be
    // decoded) stays a compact list. List draws every group as rows, so an
    // image and an archive always occupy the same box.
    let grid = layout == Layout::Grid
        && members.len() >= 2
        && members.iter().all(|m| is_grid_preview(&thumb_for(m, None)));
    let mut body = view()
        .flex_col()
        .gap(px(space::SM))
        .px(px(space::MD))
        .pb(px(space::MD));
    if grid {
        for (row_index, pair) in members.chunks(2).enumerate() {
            let mut row = view().flex_row().items_stretch().gap(px(8.0));
            for (column, member) in pair.iter().enumerate() {
                row = row.child(
                    // The row's own child, not the card, is what flex lays
                    // out: give the slot the equal share so both cards — and
                    // both previews — come out the same width.
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(member_slot(
                            position,
                            index,
                            row_index * 2 + column,
                            member.clone(),
                            true,
                        )),
                );
            }
            if pair.len() == 1 {
                // Odd tail: leave the lone card at half width.
                row = row.child(view().flex_1().min_w(px(0.0)));
            }
            body = body.child(row);
        }
    } else {
        for (member_pos, member) in members.iter().enumerate() {
            body = body.child(member_slot(
                position,
                index,
                member_pos,
                member.clone(),
                false,
            ));
        }
    }
    // The header counts move with the keeper; they update on their own
    // instead of taking the group's cards with them.
    let stats = computed(move || {
        app().with(|s| {
            s.groups
                .iter()
                .find(|g| g.index == index)
                .map_or((1, 0), |g| (g.keep_count(), g.dup_bytes()))
        })
    });
    let member_count = members.len();
    // `nsh`'s Collapsible owns the disclosure: the whole header is the
    // trigger (one big hit target instead of a 16 pt chevron to aim at), it
    // reports the expanded state to screen readers, and `reveal` springs the
    // body open and shut instead of snapping.
    bezel().child(
        bezel_core().child(
            collapsible()
                .open(expanded)
                .reveal(true)
                .child(
                    view()
                        .role(AccessibilityRole::Button)
                        .accessibility_state(move || AccessibilityState {
                            expanded: Some(expanded),
                            ..AccessibilityState::default()
                        })
                        .on_press(move || actions::toggle_expanded(index))
                        .flex_row()
                        .items_center()
                        .gap(px(space::SM))
                        .p(px(space::MD))
                        .rounded(px(radius::CONTROL))
                        .transition_colors()
                        .duration(motion(timing::FAST))
                        .hover(|s| s.bg(colors::INSET))
                        .child(chevron(expanded))
                        .child(
                            text(header)
                                .flex_1()
                                .truncate()
                                .text_size(px(text::BODY))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors::TEXT),
                        )
                        .child(dynamic(move || {
                            let (keep, reclaim) = stats.get();
                            let going = member_count.saturating_sub(keep);
                            view()
                                .flex_row()
                                .items_center()
                                .gap(px(space::SM))
                                .child(caption(format!(
                                    "{keep} kept · {} to delete",
                                    counted(going as u64, "copy", "copies"),
                                )))
                                // "reclaimable" is the word that turns a bare
                                // size into a reason to act.
                                .child(chip(
                                    format!("{} reclaimable", util::human_bytes(reclaim)),
                                    colors::ACCENT_WASH,
                                    colors::ACCENT_INK,
                                ))
                        })),
                )
                .content(body),
        ),
    )
}

/// File name for a member, or the whole path when the file has none.
fn member_name(member: &GroupMember) -> String {
    member.path.file_name().map_or_else(
        || member.path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Folder the member lives in: the dimmed second line of either layout.
fn member_parent(member: &GroupMember) -> String {
    member
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// Height of the inline player a playing row expands to, in points. Rows
/// themselves always carry the same small preview box.
const PLAYER_H: f32 = 180.0;

/// One member's slot, with its own rebuild signature: its keeper flag, its
/// tick and its poster frame. Pressing Keep on another member therefore
/// updates the two cards involved and nothing else — the group keeps its
/// order, its scroll and its other cards untouched.
fn member_slot(
    group_pos: usize,
    group_index: usize,
    member_pos: usize,
    member: GroupMember,
    grid: bool,
) -> View {
    let path = member.path.clone();
    let signature = computed(move || {
        app().with(|s| {
            let current = s
                .groups
                .get(group_pos)
                .and_then(|g| g.members.get(member_pos));
            (
                current.is_some_and(|m| m.keep),
                s.selected.contains(&path),
                s.posters.get(&path).cloned(),
                s.playing.as_deref() == Some(path.as_path()),
            )
        })
    });
    view().child(dynamic(move || {
        let (keep, checked, poster, playing) = signature.get();
        if grid {
            media_card(
                group_index,
                &member,
                keep,
                checked,
                poster.as_deref(),
                playing,
            )
        } else {
            member_row(
                group_index,
                &member,
                keep,
                checked,
                poster.as_deref(),
                playing,
            )
        }
    }))
}

/// Groups where every member has a large preview render as a card grid.
fn is_grid_preview(thumb: &Thumb) -> bool {
    matches!(thumb, Thumb::Image | Thumb::Poster(_) | Thumb::PendingVideo)
}

/// The large preview on top of a media card. Photos and videos (whose poster
/// ffmpeg extracts) share one box so cards in a row line up; a video whose
/// poster has not landed yet keeps the box with a play glyph.
///
/// The inline, muted player both layouts use for a playing clip.
///
/// It ships without nsh's own control bar. That bar is the only thing the
/// Windows backend reads the media engine for while painting, and pausing a
/// clip happens from inside the engine's own mutable borrow: Media Foundation
/// then fires `MF_MEDIA_ENGINE_EVENT_PAUSE`, which invalidates the surface
/// (`nsh-platform-windows` video.rs:345) and repaints straight into
/// `draw_bar`'s `playing()` read (video.rs:507) with the borrow still held —
/// a hard panic ("RefCell already mutably borrowed") on every stop, replace
/// or layout switch. Play and Stop live in the row actions and on the card's
/// Stop pill, so the bar was redundant as well as unsafe.
fn inline_player(path: &Path) -> impl IntoElement {
    video(path.to_string_lossy().into_owned())
        .w_full()
        .h(px(PLAYER_H))
        .rounded(px(radius::CONTROL))
        .muted(true)
        .controls(false)
}

/// Clicking a video's preview plays the clip in that same box, muted, and the
/// `Stop` pill puts the poster frame back — the card never changes size, and
/// one video plays at a time (see [`actions::toggle_play`]). Photos never get
/// a player: only a poster frame or a pending one is playable.
fn card_preview(thumb: &Thumb, path: &Path, playing: bool) -> View {
    let frame = view()
        .w_full()
        .h(px(PLAYER_H))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(radius::CONTROL))
        .bg(colors::INSET);
    let playable = matches!(thumb, Thumb::Poster(_) | Thumb::PendingVideo);
    if playing && playable {
        let stop_path = path.to_path_buf();
        return frame.child(inline_player(path)).child(
            view()
                .absolute()
                .top(px(space::SM))
                .right(px(space::SM))
                .child(icon_button(icons::STOP, "Stop video", move || {
                    actions::toggle_play(&stop_path);
                })),
        );
    }
    let source = match thumb {
        Thumb::Image => Some(path.to_string_lossy().into_owned()),
        Thumb::Poster(poster) => Some(poster.to_string_lossy().into_owned()),
        Thumb::None | Thumb::PendingVideo => None,
    };
    let frame = match source {
        Some(source) => frame.child(
            image(source)
                .w_full()
                .h(px(PLAYER_H))
                .object_fit(ResizeMode::Cover)
                .rounded(px(radius::CONTROL)),
        ),
        None => {
            let empty = frame.border_1().border_color(colors::BORDER);
            match thumb {
                // Only a video waiting on its poster frame gets the glyph: a
                // photo the image element cannot decode is a plain box.
                Thumb::PendingVideo => empty.child(
                    text("▶")
                        .text_size(px(text::HEADING))
                        .text_color(colors::MUTED),
                ),
                _ => empty,
            }
        }
    };
    if playable {
        // The preview is the play button: a click loads the clip, muted.
        let play_path = path.to_path_buf();
        frame
            .transition_colors()
            .duration(motion(timing::BASE))
            .on_press(move || actions::toggle_play(&play_path))
    } else {
        frame
    }
}

/// One card of a media group: large preview, then the name, its folder and
/// the same controls a row carries. Only the border and the badge move when
/// the keeper changes, so the group never reflows during a decision.
fn media_card(
    group_index: usize,
    member: &GroupMember,
    keep: bool,
    checked: bool,
    poster: Option<&Path>,
    playing: bool,
) -> View {
    let thumb = thumb_for(member, poster);
    let similarity = member
        .similarity
        .map(|s| format!("{:.0}% similar", s * 100.0));
    // A row surface, not a card: it sits inside the group card, and a card in
    // a card draws a second border and shadow on the same edge.
    view()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .w_full()
        .p(px(space::SM))
        .gap(px(space::SM))
        .bg(if keep {
            colors::KEEP_WASH
        } else if checked {
            colors::SELECT_WASH
        } else {
            colors::PANE
        })
        .rounded(px(radius::CARD))
        .border_curve(BorderCurve::Continuous)
        .border_1()
        .border_color(if keep {
            colors::KEEP
        } else if checked {
            colors::SELECT
        } else {
            colors::BORDER
        })
        // Picking the keeper or ticking a copy changes only these three
        // colours: the change crosses over instead of snapping.
        .transition_colors()
        .duration(motion(timing::FAST))
        .child(card_preview(&thumb, &member.path, playing))
        .child(
            view()
                .flex_col()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(
                    text(member_name(member))
                        .truncate()
                        .text_size(px(text::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::TEXT),
                )
                // Truncated, never allowed to widen the card: a deep folder
                // would otherwise stretch this card (and its preview) past
                // its neighbour.
                .child(caption(member_parent(member)).truncate()),
        )
        .child(member_actions(
            group_index,
            member,
            keep,
            checked,
            similarity,
            // Videos only: a photo has nothing to play and gets no control.
            matches!(thumb, Thumb::Poster(_) | Thumb::PendingVideo).then_some(playing),
        ))
}

fn member_row(
    group_index: usize,
    member: &GroupMember,
    keep: bool,
    checked: bool,
    poster: Option<&Path>,
    playing: bool,
) -> View {
    // Rendering only reads state: poster extraction is requested when a
    // group becomes visible (see `actions::request_group_posters`), never
    // while the tree is being built.
    let thumb = thumb_for(member, poster);
    // `thumb_for` only maps videos to a poster frame or a pending one, so this
    // is the row's own "there is a player behind this" flag: photos never get
    // a play affordance.
    let video_member = matches!(thumb, Thumb::Poster(_) | Thumb::PendingVideo);
    let similarity = member
        .similarity
        .map(|s| format!("{:.0}% similar", s * 100.0));
    let name = member_name(member);
    let shown = fit_text(&name, 120);
    let truncated = shown != name;
    let cell = text(shown)
        .truncate()
        .text_size(px(text::BODY))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT);
    let cell = if truncated {
        cell.tooltip(hint(name))
    } else {
        cell
    };
    let thumb_box = member_thumb(&thumb, &member.path);
    let thumb_box = if video_member {
        // The thumbnail is the play button too: a click loads the clip muted.
        let play_path = member.path.clone();
        thumb_box
            .transition_colors()
            .duration(motion(timing::BASE))
            .on_press(move || actions::toggle_play(&play_path))
    } else {
        thumb_box
    };
    let info = view()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(4.0))
        .child(cell)
        .child(caption(member_parent(member)))
        .child(member_actions(
            group_index,
            member,
            keep,
            checked,
            similarity,
            video_member.then_some(playing),
        ));
    let surface = view()
        .px(px(space::SM))
        .py(px(space::XS))
        .rounded(px(radius::CONTROL))
        // Green keeps, amber ticks: two decisions that must never look
        // like the same one.
        .bg(if keep {
            colors::KEEP_WASH
        } else if checked {
            colors::SELECT_WASH
        } else {
            colors::PANE
        })
        .border_1()
        .border_color(if keep {
            colors::KEEP
        } else if checked {
            colors::SELECT
        } else {
            colors::BORDER
        })
        .transition_colors()
        .duration(motion(timing::BASE));
    if playing && video_member {
        // Playing is the one deliberate expansion: a 72 pt thumbnail is not a
        // usable player, so the row grows to hold one (muted, with the
        // platform's controls) while every other row keeps the small box.
        return surface
            .flex_col()
            .gap(px(space::SM))
            .child(inline_player(&member.path))
            .child(info);
    }
    surface
        .flex_row()
        .items_start()
        .gap(px(space::SM))
        .child(thumb_box)
        .child(info)
}

/// The controls line both layouts use: keeper / selection state, size,
/// similarity, the reference badge and the Keep button.
fn member_actions(
    group_index: usize,
    member: &GroupMember,
    keep: bool,
    checked: bool,
    similarity: Option<String>,
    play: Option<bool>,
) -> View {
    let label = if keep {
        "KEEP"
    } else if checked {
        "Selected"
    } else {
        "Select"
    };
    let active = keep || checked;
    let toggle_path = member.path.clone();
    let keep_path = member.path.clone();
    let play_path = member.path.clone();
    let playing = play.unwrap_or(false);
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::SM))
        .child(
            checkbox()
                .checked(active)
                .disabled(keep)
                .on_change(move |_: &bool| actions::toggle_selected(&toggle_path))
                .px(px(8.0))
                .py(px(2.0))
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .rounded(px(radius::CONTROL))
                .bg(if keep {
                    colors::KEEP_WASH_STRONG
                } else if checked {
                    colors::SELECT_WASH_STRONG
                } else {
                    colors::INSET
                })
                .border_1()
                .border_color(if keep {
                    colors::KEEP
                } else if checked {
                    colors::SELECT
                } else {
                    colors::BORDER
                })
                .indicator_style(|s| {
                    s.w(px(12.0))
                        .h(px(12.0))
                        .rounded(px(3.0))
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .border_color(if keep {
                            colors::KEEP
                        } else if checked {
                            colors::SELECT
                        } else {
                            colors::MUTED
                        })
                        .checked(move |s| {
                            let tint = if keep { colors::KEEP } else { colors::SELECT };
                            s.bg(tint).border_color(tint)
                        })
                })
                .indicator(
                    text(if active { "✓" } else { "" })
                        // Below the caption step on purpose: the glyph sits in
                        // a 12pt box, so the token size would overflow the
                        // control it marks. It still scales with the user's
                        // text size, like every other string here.
                        .text_size(px(9.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::ON_ACCENT),
                )
                .child(
                    text(label)
                        .text_size(px(text::CAPTION))
                        .font_weight(FontWeight::SEMIBOLD)
                        // The wash is a backing: the words on it take the ink
                        // step of the same hue so they stay readable.
                        .text_color(if keep {
                            colors::KEEP_INK
                        } else if checked {
                            colors::SELECT_INK
                        } else {
                            colors::MUTED
                        }),
                ),
        )
        .child(caption(util::human_bytes(member.size)))
        .child(when(similarity.is_some(), move || {
            caption(similarity.clone().unwrap_or_default())
        }))
        .child(when(member.reference, || {
            chip("REF", colors::ACCENT_WASH, colors::ACCENT_INK)
        }))
        .child(when(play.is_some(), move || {
            let path = play_path.clone();
            let (markup, label) = if playing {
                (icons::STOP, "Stop video")
            } else {
                (icons::PLAY, "Play video")
            };
            icon_ghost_button(markup, label, move || {
                actions::toggle_play(&path);
            })
        }))
        .child(when(!keep, move || {
            let path = keep_path.clone();
            // A blue check next to the amber checked box read as a second
            // tick; the action names itself instead.
            view()
                .child(quiet_button("Keep", move || {
                    actions::set_keeper(group_index, &path);
                }))
                .tooltip(hint("Keep this copy instead of deleting it"))
        }))
}

fn mode_workspace() -> View {
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .gap(px(space::LG))
        .p(px(space::XL))
        .child(mode_header())
        .child(fast_scroll(&MODE_AT, dynamic(mode_list)))
}

fn mode_header() -> View {
    view()
        .flex_row()
        .items_end()
        .justify_between()
        .gap(px(space::LG))
        .child(
            view()
                .flex_col()
                .min_w(px(0.0))
                .gap(px(4.0))
                .child(kicker("Cleanup workspace"))
                .child(title(|| {
                    app().with(|s| {
                        s.mode_result
                            .as_ref()
                            .map(|r| mode_title(r.mode))
                            .unwrap_or("Cleanup scan")
                            .to_string()
                    })
                }))
                .child(caption(
                    "Cleanup findings are read-only in the GUI; use the CLI with --delete or --trash.",
                )),
        )
        .child(scan_again_button())
}

fn mode_list() -> View {
    let Some(result) = app().with(|s| s.mode_result.clone()) else {
        return view().child(caption("Run a cleanup scan to see findings."));
    };
    let mut list = view().flex_col().gap(px(8.0));
    if app().with(|s| s.scanning) {
        list = list.child(progress_card());
    }
    list = list.child(mode_chips(&result));
    if result.is_empty() {
        list = list.child(nothing_found_card(
            "Nothing to clean up",
            "The scanned folders came back clean. Add a folder, or scan again after changing the threshold.",
            false,
        ));
        return list;
    }
    for finding in &result.findings {
        list = list.child(mode_finding_row(finding));
    }
    for group in &result.groups {
        list = list.child(mode_group_card(group));
    }
    for error in &result.errors {
        list = list.child(note_card_error(error));
    }
    list
}

fn mode_chips(result: &ModeResult) -> View {
    view()
        .flex_row()
        .flex_wrap()
        .gap(px(6.0))
        .child(chip(
            counted(result.finding_count() as u64, "finding", "findings"),
            colors::ACCENT_WASH,
            colors::ACCENT_INK,
        ))
        .child(chip(
            format!("{} reclaimable", util::human_bytes(result.reclaimable())),
            colors::INSET,
            colors::MUTED,
        ))
        .child(chip(
            format!("{} scanned", counted(result.files_scanned, "file", "files")),
            colors::INSET,
            colors::MUTED,
        ))
}

fn mode_finding_row(finding: &crate::modes::Finding) -> View {
    card()
        .gap(px(space::XS))
        .p(px(space::MD))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                .child(chip(finding.kind, colors::INSET, colors::MUTED))
                .child({
                    let full_path = finding.path.to_string_lossy().to_string();
                    let shown = fit_text(&full_path, 110);
                    let truncated = shown != full_path;
                    let cell = text(shown)
                        .flex_1()
                        .truncate()
                        .text_size(px(text::BODY))
                        .text_color(colors::TEXT);
                    if truncated {
                        cell.tooltip(hint(full_path))
                    } else {
                        cell
                    }
                })
                .child(caption(util::human_bytes(finding.size))),
        )
        .child(caption(finding.detail.clone()))
}

fn mode_group_card(group: &crate::modes::FindingGroup) -> View {
    card()
        .gap(px(space::XS))
        .p(px(space::MD))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                .child(
                    text(group.label.clone())
                        .flex_1()
                        .truncate()
                        .text_size(px(text::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::TEXT),
                )
                .child(chip(
                    format!("{} reclaimable", util::human_bytes(group.reclaimable)),
                    colors::ACCENT_WASH,
                    colors::ACCENT_INK,
                )),
        )
        .child(caption(group.detail.clone()))
        .children(group.members.iter().map(|member| {
            let full_path = member.to_string_lossy().to_string();
            let shown = fit_text(&full_path, 110);
            if shown == full_path {
                caption(shown)
            } else {
                caption(shown).tooltip(hint(full_path))
            }
        }))
}

fn note_card_error(message: &str) -> View {
    card()
        .p(px(space::MD))
        .border_color(colors::DANGER)
        .child(caption(message.to_owned()).text_color(colors::DANGER_TEXT))
}
