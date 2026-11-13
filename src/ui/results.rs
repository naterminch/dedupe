//! Main results pane: first-run guidance, live progress, duplicate groups and
//! cleanup findings.
//!
//! Group rows are built inside one `dynamic` region: every read of [`app`]
//! subscribes, so a scan, a keeper swap or a tick re-renders the list.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use nsh_core::prelude::*;

use crate::matching::{Group, GroupMember};
use crate::media;
use crate::modes::mode_title;
use crate::ui::actions;
use crate::ui::palette::{size, space};
use crate::ui::state::{Layout, app, scan_status};
use crate::ui::widgets::{
    BadgeTone, Decision, Ink, PreviewFill, PreviewGlyph, PreviewSize, SelectItem, action_bar,
    action_group, alert_dialog, amount, badge, body, caption, card, danger_button, description,
    disclosure, divider, dropdown_trigger, entering, error_notice, fading_in, fast_scroll,
    fast_scroll_by, fast_scroll_max, field, field_label, figure, ghost_button, ground, heading,
    help_dot, hint, icon_button, icon_ghost_button, icon_secondary_button, icon_segmented, icons,
    name, notice, number_field, number_text, option_select, panel_title, press_to_play,
    preview_frame, preview_glyph, preview_image, primary_button, progress_bar, quiet_button,
    secondary_button, section_title, segmented, tick, tile, video_player,
};
use crate::ui_helpers::{counted, fit_text};
use crate::util;

const KEEP_MODES: [&str; 5] = ["First", "Smallest", "Newest", "Oldest", "Best"];

/// Photo extensions the image element decodes. Formats it cannot (HEIC, SVG,
/// ICO, TIFF) keep the text-only row rather than a broken box.
const THUMB_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "bmp", "webp"];

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

/// The preview of one copy: the photo itself, a video's poster, or a glyph
/// for files that have none.
fn member_preview(thumb: &Thumb, path: &Path, size: PreviewSize) -> View {
    match thumb {
        Thumb::Image => {
            preview_frame(size, PreviewFill::Picture).child(preview_image(path_source(path), size))
        }
        Thumb::Poster(poster) => preview_frame(size, PreviewFill::Picture)
            .child(preview_image(path_source(poster), size)),
        // A file with no preview gets a document glyph in a row, not an empty
        // box: the box read as a thumbnail that failed to load. On a card it
        // is a photo the image element cannot decode, so the box stays plain.
        Thumb::None => match size {
            PreviewSize::Card => preview_frame(size, PreviewFill::Empty),
            PreviewSize::Thumb => {
                preview_frame(size, PreviewFill::Empty).child(preview_glyph(PreviewGlyph::Document))
            }
        },
        Thumb::PendingVideo => {
            preview_frame(size, PreviewFill::Empty).child(preview_glyph(PreviewGlyph::Clip))
        }
    }
}

/// A path as an image or video source.
fn path_source(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn results() -> View {
    ground()
        .flex_col()
        .flex_1()
        .h_full()
        .min_w(px(0.0))
        .overflow_hidden()
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
        // Before the first scan the workspace is the setup form, which brings
        // its own title row. Once a scan runs the results header and toolbars
        // take over until Back returns to the form.
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
                .child(heading(|| {
                    if app().with(|s| s.scanning) {
                        "Scanning"
                    } else {
                        "Review duplicates"
                    }
                }))
                .child(summary_line()),
        )
        .child(dynamic(|| {
            // The figure is the scan's answer, so it waits for the scan: a
            // running count of bytes found so far would read as the result.
            let done = computed(|| app().with(|s| s.has_scanned && !s.scanning));
            when(done.get(), || {
                answer_figure("Reclaimable", || {
                    util::human_bytes(app().with(|s| s.reclaim_bytes))
                })
            })
        }))
}

/// The screen's answer, labelled above it: the largest number on the
/// screen, right-aligned with the toolbar's far edge.
fn answer_figure(label: &'static str, value: impl Fn() -> String + 'static) -> View {
    view()
        .flex_col()
        .items_end()
        .gap(px(space::XXS))
        .child(field_label(label))
        .child(figure(value))
}

/// The one line under the title: what the scan found, in the order a reader
/// acts on it — how many groups, and how many copies are ticked to go.
fn summary_line() -> impl IntoElement {
    description(|| {
        let state = app();
        if state.with(|s| s.scanning) {
            "The list appears when the scan finishes.".to_string()
        } else {
            let groups = state.with(|s| s.groups.len());
            let dups = state.with(|s| s.dup_files);
            format!(
                "{} · {}",
                counted(groups as u64, "group", "groups"),
                counted(dups, "duplicate", "duplicates"),
            )
        }
    })
}

/// The list's own controls, side by side at the toolbar's far end: show
/// every group, or fold them away. They are about reading the list, not
/// deciding anything, so they take the quietest look in the bar.
fn list_actions() -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::XXS))
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
}

/// Two bands: what you do to the results, then what shapes them. Actions run
/// along the top — Back and Delete together on the left, the whole-set
/// commands on the right — and the options that only tune the list sit in a
/// second row, every one of them a segmented control of the same height.
fn action_row() -> impl IntoElement {
    view()
        .flex_col()
        .w_full()
        .gap(px(space::MD))
        .child(result_actions())
        .child(scan_options())
}

/// The actions that act on the whole result set: destructive changes to the
/// files on the left, moving, reporting and folding on the right.
fn result_actions() -> impl IntoElement {
    action_bar(
        "Results actions",
        vec![
            action_group().child(back_button()).child(delete_bar()),
            action_group()
                .child(consolidate_button())
                .child(icon_button(
                    icons::EXPORT,
                    "Export JSON report",
                    actions::save_report,
                ))
                .child(list_actions()),
        ],
    )
}

/// The scan options: the similarity floor and the keeper rule on the left,
/// the sort and layout choices at the far edge. They wrap only when the
/// keeper's segments run out of room.
fn scan_options() -> impl IntoElement {
    action_bar(
        "Scan options",
        vec![
            action_group()
                .child(sim_filter())
                .child(keep_choice_group()),
            action_group().child(sort_toggle()).child(layout_toggle()),
        ],
    )
}

/// Re-running a finished scan. In the cleanup workspace the findings are
/// read-only and there is no setup form, so its header carries the next run
/// itself. Hidden while a scan runs, where the progress card's Stop is the
/// only scan action.
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
        .child(number_field(
            "97",
            || app().with(|s| s.min_sim_input.trim().parse::<f64>().ok()),
            |next| app().update(|s| s.min_sim_input = number_text(next)),
            (0.0, 100.0),
            112.0,
        ))
        .child(icon_button(icons::CHECK, "Apply similarity", || {
            let raw = app().with(|s| s.min_sim_input.clone());
            actions::apply_sim_input(&raw);
        }))
}

/// The keeper rule with its name and its explanation: the segments decide
/// which copy of a group survives a delete, and the help dot carries the
/// rules themselves, which a first-time user cannot guess from one word.
fn keep_choice_group() -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::SM))
        .child(field_label("Keep"))
        .child(help_dot(KEEP_HINT))
        .child(keep_modes())
}

/// The rules behind the chips, written from the comparators that pick the
/// keeper ([`crate::matching::KeepMode`]): "first found" is really path
/// order, and in similar-media groups the smallest chip ranks resolution
/// first.
const KEEP_HINT: &str = "Which copy of each group survives a delete. First: the alphabetically \
first path. Smallest: the smallest file — in similar-media groups the highest resolution wins \
first. Newest / Oldest: by modified date. Best: the highest resolution, then the longest \
duration, then the largest file.";

/// One keeper rule at a time, as a segmented control.
fn keep_modes() -> View {
    view().child(dynamic(|| {
        // The rule alone rebuilds these segments; a scan tick would restart
        // their hover and drop a press mid-flight.
        let current = computed(|| app().with(|s| s.keep_index.unwrap_or(0)));
        segmented(KEEP_MODES, current.get(), actions::apply_keep_mode)
    }))
}

/// The sort rule as two segments: path order, or the most space first.
fn sort_toggle() -> impl IntoElement {
    view().child(dynamic(|| {
        let biggest = computed(|| app().with(|s| s.sort_biggest));
        segmented(
            ["By path", "Biggest first"],
            usize::from(biggest.get()),
            |index| {
                if app().with_untracked(|s| s.sort_biggest) != (index == 1) {
                    actions::toggle_sort_biggest();
                }
            },
        )
    }))
}

/// Grid or list: two-column preview cards, or one compact row list. It is a
/// structural choice, so switching rebuilds the list.
fn layout_toggle() -> View {
    view().child(dynamic(|| {
        let current = computed(|| app().with(|s| s.layout));
        let layouts = [Layout::Grid, Layout::List];
        let selected = layouts
            .iter()
            .position(|l| *l == current.get())
            .unwrap_or(0);
        icon_segmented(
            [(icons::GRID, "Grid layout"), (icons::LIST, "List layout")],
            selected,
            move |index| actions::set_layout(layouts[index]),
        )
    }))
}

/// The whole-set decision: how much is ticked, and the one button that acts
/// on it.
///
/// With nothing ticked it offers the missing step by name — `Select all
/// duplicates` — instead of rendering nothing and leaving the row with only
/// Back. Selection used to hide behind an unlabelled glyph in the header
/// pill, which is the one step between reading the list and deleting from it.
/// What the delete bar renders from, named field by field: the tuple it used
/// to be had grown to five positional values, and `(usize, u64, bool, bool,
/// usize)` does not say which `bool` is the trash setting.
#[derive(Clone, Copy, PartialEq, Eq)]
struct DeleteBar {
    count: usize,
    bytes: u64,
    can_undo: bool,
    trash: bool,
    groups: usize,
}

fn delete_bar() -> View {
    view().child(dynamic(|| {
        // The selection and the trash switch decide this bar. A scan tick
        // rebuilding it replaced the Delete button under the pointer.
        let bar = computed(|| {
            app().with(|s| {
                let targets = actions::pending_targets(s);
                DeleteBar {
                    count: targets.len(),
                    bytes: targets.iter().map(|t| t.size).sum(),
                    can_undo: !s.last_trashed.is_empty(),
                    trash: s.trash,
                    groups: s.groups.len(),
                }
            })
        });
        let DeleteBar {
            count,
            bytes,
            can_undo,
            trash,
            groups,
        } = bar.get();
        let mut row = view().flex_row().gap(px(space::SM)).items_center();
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
            row = row
                .child(danger_button(label, actions::request_delete).tooltip(hint(tip)))
                // Clearing is the undo of Select all, so it sits beside the
                // action it takes the target away from.
                .child(ghost_button("Clear selection", actions::select_none));
        } else if groups > 0 {
            row = row.child(
                icon_secondary_button(icons::SELECT_ALL, "Select all duplicates", || {
                    app().update(actions::reselect_all_dups)
                })
                .tooltip(hint("Tick every copy that is not a keeper")),
            );
        }
        if can_undo {
            row = row.child(icon_button(
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
/// The tallest the delete plan grows before it scrolls inside the dialog:
/// about three groups, so the dialog's buttons stay on screen at 640 pt.
const DELETE_PLAN_MAX_H: f32 = 320.0;

/// About how many characters of a path fit beside "Keep instead" in the
/// delete dialog; the front is cut beyond it.
const PLAN_PATH_CHARS: usize = 56;

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
    // The plan fills the panel: without this the body is content-sized, so the
    // rules between groups stopped short of the panel's own edge.
    let mut body = view().flex_col().w_full().gap(px(space::MD));
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
                        panel_title(format!("Keeping {}", group.keeper))
                            .flex_1()
                            .truncate(),
                    )
                    .child(badge("Keeper", BadgeTone::Keep)),
            )
            .child(caption(format!(
                "{} · {}",
                counted(going as u64, "copy goes", "copies go"),
                util::human_bytes(group.bytes),
            )));
        for path in &group.rows {
            let full = path.display().to_string();
            let shown = fit_text(&full, PLAN_PATH_CHARS);
            let truncated = shown != full;
            let cell = caption(shown).flex_1().truncate();
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
    let description_text = if count == 0 {
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
    alert_dialog(
        heading,
        description_text,
        fast_scroll_max(&DELETE_AT, px(DELETE_PLAN_MAX_H), body),
        view()
            .child(secondary_button("Cancel", actions::cancel_delete))
            .child(when(count > 0, move || {
                danger_button(format!("Delete {count}"), actions::confirm_delete)
            })),
        actions::cancel_delete,
    )
}

fn consolidate_button() -> View {
    view().child(dynamic(|| {
        let dest = computed(|| app().with(|s| s.consolidate_dest.is_some()));
        if dest.get() {
            icon_button(
                icons::FOLDER_MOVE,
                "Move keepers into the chosen folder",
                actions::perform_consolidate,
            )
        } else {
            icon_button(
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
    // so a wide row (the setup form's controls) stretches the whole column past
    // the window instead of wrapping inside it.
    let mut list = view().flex_col().w_full().min_w(px(0.0)).gap(px(space::MD));
    if scanning {
        list = list.child(dynamic(progress_card));
    }
    if !has_scanned && !scanning {
        list = list.child(dynamic(setup));
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
        .gap(px(space::MD))
        .p(px(space::LG))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(space::MD))
                .child(
                    view()
                        .flex_col()
                        .min_w(px(0.0))
                        .gap(px(space::XXS))
                        .child(panel_title(phase))
                        .child(caption(label).tabular_nums()),
                )
                .child(danger_button("Stop", actions::stop_scan)),
        )
        .child(progress_bar(fraction, determinate))
}

/// The setup page: what the next scan covers and how it decides, as one
/// form, with the scan itself as the screen's one primary action. Its title
/// row sits where the results' title will, so starting a scan changes the
/// content under a header that stays put.
fn setup() -> View {
    // The page arrives as one piece instead of appearing fully formed.
    let enter = entering();
    view()
        .flex_col()
        .w_full()
        .gap(px(space::LG))
        .child(setup_header())
        .child(setup_form())
        .bind_motion(&enter)
}

/// The title, the scope of the next run in one plain line, and Scan.
fn setup_header() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .justify_between()
        .gap(px(space::LG))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(space::XS))
                .child(heading("New scan"))
                .child(description(scope_line)),
        )
        .child(primary_button(scan_label, actions::start_scan))
}

/// What the next run will do, from the choices below it: how many folders,
/// what it looks for, and where a delete sends the copies. Nothing is
/// removed by a scan, and the line says so where the reader decides.
fn scope_line() -> String {
    let (folders, trash) = app().with(|s| (s.folders.len(), s.trash));
    if folders == 0 {
        return "Add a folder below or drop one on the window. A scan only reads; nothing is deleted until you confirm.".to_string();
    }
    let outcome = if !finds_duplicates() {
        "findings are listed, nothing is deleted"
    } else if trash {
        "copies go to the trash after you confirm"
    } else {
        "copies are deleted permanently after you confirm"
    };
    format!(
        "{} in {} · {outcome}",
        find_label(),
        counted(folders as u64, "folder", "folders"),
    )
}

/// Whether the next scan looks for duplicates — the only scan the GUI acts
/// on; the cleanup scans list their findings read-only.
fn finds_duplicates() -> bool {
    crate::ui_helpers::find_mode(app().with(|s| s.find_index)) == crate::cli::FindMode::Duplicates
}

/// The scan's choices as labelled rows on one card, every rule visible:
/// where, what, which copy stays, how close is close, where deletes go, and
/// what video support the machine has.
fn setup_form() -> View {
    view().w_full().child(dynamic(|| {
        // The scan type alone reshapes the form: the keeper, similarity,
        // delete and video rows only mean something to a duplicate scan.
        let duplicates = computed(finds_duplicates);
        setup_rows(duplicates.get())
    }))
}

/// The form's rows for one scan type.
fn setup_rows(duplicates: bool) -> View {
    let mut rows: Vec<View> = vec![
        form_row(
            "Folders",
            Some(
                "Paste a path and press Add folder, or drop folders on the window. They appear in the sidebar.",
            ),
            view()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                .w_full()
                .child(field(
                    "Scan a folder",
                    || app().with(|s| s.folder_input.clone()),
                    |value| app().update(|s| s.folder_input = value),
                ))
                .child(icon_secondary_button(
                    icons::PLUS,
                    "Add folder",
                    actions::add_from_input,
                )),
        ),
        form_row(
            "Look for",
            None,
            view()
                .flex_col()
                .gap(px(space::XS))
                .child(find_select())
                .child(caption(|| {
                    crate::ui_helpers::find_mode_caption(app().with(|s| s.find_index))
                })),
        ),
    ];
    if duplicates {
        rows.extend([
        form_row(
            "Keep",
            Some(
                "The copy of each group that survives a delete. First is path order; Best is the highest resolution.",
            ),
            keep_modes(),
        ),
        form_row(
            "Similarity",
            Some(
                "How close two photos or videos must be to count as the same. 97% catches re-saves and re-encodes.",
            ),
            view().child(sim_pill()),
        ),
        form_row("Deleted copies", None, view().child(trash_select())),
        form_row("Video", None, media_status()),
        ]);
    }
    let mut form = card().w_full();
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            form = form.child(view().px(px(space::LG)).child(divider()));
        }
        form = form.child(row);
    }
    form
}

/// The label column of the setup form: wide enough for its longest label,
/// so every control starts on one line down the card.
const FORM_LABEL_W: f32 = 148.0;

/// One row of the setup form: the label on the left, the control and its
/// explanation on the right, both on the card's own inset.
fn form_row(label: &'static str, note: Option<&'static str>, control: View) -> View {
    // Controls keep their own width (a segmented control stretched to the
    // card's edge reads as a progress bar); the folder row asks for the full
    // width itself.
    let mut right = view()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .items_start()
        .gap(px(space::XS))
        .child(control);
    if let Some(note) = note {
        right = right.child(caption(note));
    }
    view()
        .flex_row()
        .items_start()
        .gap(px(space::LG))
        .px(px(space::LG))
        // 10 pt: the whole form, every row with its note, fits the default
        // 1120×640 window without scrolling.
        .py(px(space::SM + space::XXS))
        .child(
            view()
                .w(px(FORM_LABEL_W))
                .flex_shrink_0()
                .min_h(px(size::CONTROL))
                .justify_center()
                .child(panel_title(label)),
        )
        .child(right)
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

/// The similarity floor of a run: one value out of the five offered, as a
/// dropdown like the other choices on the setup form.
fn sim_pill() -> impl IntoElement {
    let items: Vec<SelectItem> = SIM_STEPS
        .iter()
        .map(|step| SelectItem::new(step.to_string(), format!("{step}%")))
        .collect();
    option_select(
        items,
        "Similarity floor",
        || {
            Some(SharedString::from(format!(
                "{}",
                current_similarity() as i64
            )))
        },
        |picked: &Option<SharedString>| {
            let Some(picked) = picked else {
                return;
            };
            let text = picked.to_string();
            app().update(|s| s.min_sim_input = text.clone());
            actions::apply_sim_input(&text);
        },
        dropdown_trigger(icons::SLIDERS, || {
            SharedString::from(format!("{}%", current_similarity() as i64))
        }),
    )
}

/// The similarity steps the setup form offers, loosest last.
const SIM_STEPS: [f64; 5] = [100.0, 97.0, 95.0, 90.0, 85.0];

fn current_similarity() -> f64 {
    app()
        .with(|s| s.min_sim_input.trim().parse::<f64>().ok())
        .unwrap_or(97.0)
}

/// The scan button's label, per scan type. Read inside the setup page's
/// dynamic region, so changing what to look for relabels it without a
/// rebuild.
fn scan_label() -> &'static str {
    if finds_duplicates() {
        "Scan for duplicates"
    } else {
        "Run cleanup scan"
    }
}

/// What the media chip says, from the two probe answers.
///
/// Previews need ffmpeg alone: it cuts the posters and plays the clip.
/// ffprobe only adds the resolution and duration that video matching uses, so
/// calling its absence "no previews" denied the thumbnails that still work.
#[must_use]
pub(crate) fn media_status_text(
    checked: bool,
    ffmpeg: bool,
    ffprobe: bool,
) -> (&'static str, &'static str) {
    if !checked {
        ("Checking video support…", "Looking for FFmpeg on PATH.")
    } else if ffmpeg && ffprobe {
        (
            "Video previews on",
            "FFmpeg adds video previews; ffprobe adds resolution, duration and video matching.",
        )
    } else if ffmpeg {
        (
            "Video previews on",
            "Thumbnails and playback work. Install ffprobe to add resolution, duration and video matching.",
        )
    } else {
        (
            "No video previews",
            "FFmpeg adds video previews and video matching. Image comparison works without it.",
        )
    }
}

/// ffmpeg/ffprobe availability as a row of the setup form: what the reader
/// gets, then what adds the rest. The detail is on the page, not in a
/// tooltip — it decides whether video matching runs at all.
fn media_status() -> View {
    view().child(dynamic(|| {
        let probe = computed(|| app().with(|s| (s.media_checked, s.ffmpeg_ok, s.ffprobe_ok)));
        let (checked, ffmpeg, ffprobe) = probe.get();
        let (label, detail) = media_status_text(checked, ffmpeg, ffprobe);
        view()
            .flex_col()
            .gap(px(space::XS))
            .child(
                view()
                    .min_h(px(size::CONTROL))
                    .justify_center()
                    .child(panel_title(label)),
            )
            .child(caption(detail))
    }))
}

/// A scan that found nothing: what to change and where. `rescan` adds the
/// button that runs the same scan again — on the duplicates page the scan form
/// lives back on the setup page, so without it this state is a dead end. The
/// cleanup workspace passes `false`: its header already carries the rerun.
fn nothing_found_card(message: &'static str, hint: &'static str, rescan: bool) -> View {
    let mut card = card().gap(px(space::MD)).p(px(space::XL)).child(
        view()
            .flex_col()
            .gap(px(space::XS))
            .child(section_title(message))
            .child(description(hint)),
    );
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
    notice().child(caption(message))
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
    // The disclosure insets the body onto the header's text column.
    let mut body = view().flex_col().gap(px(space::SM));
    if grid {
        for (row_index, pair) in members.chunks(2).enumerate() {
            let mut row = view().flex_row().items_stretch().gap(px(space::SM));
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
    // The header counts follow the ticks, not just the keepers: a group with
    // nothing selected is not "1 copy to delete", and the reclaimable figure
    // is what the ticked copies would actually give back. They update on
    // their own instead of taking the group's cards with them.
    let stats = computed(move || {
        app().with(|s| {
            s.groups
                .iter()
                .find(|g| g.index == index)
                .map_or((1, 0, 0), |g| {
                    let mut copies = 0usize;
                    let mut bytes = 0u64;
                    for m in &g.members {
                        if m.deletable() && s.selected.contains(&m.path) {
                            copies += 1;
                            bytes += m.size;
                        }
                    }
                    (g.keep_count(), copies, bytes)
                })
        })
    });
    let header = view()
        .flex_row()
        .flex_1()
        .min_w(px(0.0))
        .items_center()
        .gap(px(space::MD))
        .child(name(header).flex_1().truncate())
        .child(dynamic(move || {
            let (kept, copies, bytes) = stats.get();
            let mut row = view()
                .flex_row()
                .items_center()
                .gap(px(space::MD))
                .child(caption(if copies == 0 {
                    // Nothing ticked is a state worth naming: the group is
                    // intact, not half-deleted.
                    format!("{kept} kept · nothing ticked")
                } else {
                    format!(
                        "{kept} kept · {} to delete",
                        counted(copies as u64, "copy", "copies"),
                    )
                }));
            if copies > 0 {
                // The group's share of the answer figure, in ink: the hue
                // belongs to the total alone.
                row = row.child(amount(format!("{} to free", util::human_bytes(bytes))));
            }
            row
        }));
    disclosure(
        expanded,
        move || actions::toggle_expanded(index),
        header,
        body,
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

/// What one copy's tile says about it.
fn decision(keep: bool, checked: bool) -> Decision {
    if keep {
        Decision::Keep
    } else if checked {
        Decision::Delete
    } else {
        Decision::Undecided
    }
}

/// Whether a preview stands for a clip, which a press can play.
fn is_video(thumb: &Thumb) -> bool {
    matches!(thumb, Thumb::Poster(_) | Thumb::PendingVideo)
}

/// A copy's preview, playable when it is a clip: a press on the preview plays
/// it in that same box, muted, and the Stop button puts the poster back — the
/// tile never changes size, and one video plays at a time (see
/// [`actions::toggle_play`]). The Play button in the tile's controls is the
/// keyboard's way to the same action.
fn playable_preview(thumb: &Thumb, path: &Path, size: PreviewSize, playing: bool) -> View {
    if playing && is_video(thumb) {
        let stop_path = path.to_path_buf();
        let player = preview_frame(PreviewSize::Card, PreviewFill::Picture)
            .child(video_player(path_source(path)));
        return if size == PreviewSize::Card {
            player.child(
                view()
                    .absolute()
                    .top(px(space::SM))
                    .right(px(space::SM))
                    .child(icon_button(icons::STOP, "Stop video", move || {
                        actions::toggle_play(&stop_path);
                    })),
            )
        } else {
            player
        };
    }
    let preview = member_preview(thumb, path, size);
    if is_video(thumb) {
        let play_path = path.to_path_buf();
        press_to_play(preview, move || actions::toggle_play(&play_path))
    } else {
        preview
    }
}

/// One copy in the grid layout: large preview, then the name, its folder and
/// the same controls a row carries. Only the edge and the badge move when the
/// keeper changes, so the group never reflows during a decision.
fn media_card(
    group_index: usize,
    member: &GroupMember,
    keep: bool,
    checked: bool,
    poster: Option<&Path>,
    playing: bool,
) -> View {
    let thumb = thumb_for(member, poster);
    let video_member = is_video(&thumb);
    tile(decision(keep, checked))
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .w_full()
        .gap(px(space::SM))
        .child(playable_preview(
            &thumb,
            &member.path,
            PreviewSize::Card,
            playing,
        ))
        .child(
            view()
                .flex_col()
                .min_w(px(0.0))
                .gap(px(space::XXS))
                .child(name(member_name(member)).truncate())
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
            // Videos only: a photo has nothing to play and gets no control.
            video_member.then_some(playing),
        ))
}

/// One copy in the list layout: thumbnail beside name, folder and controls.
/// Playing is the one deliberate expansion: a 72 pt thumbnail is not a usable
/// player, so the row grows to hold one while every other row keeps its box.
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
    let video_member = is_video(&thumb);
    let full = member_name(member);
    let shown = fit_text(&full, NAME_CHARS);
    let cut = shown != full;
    let cell = name(shown).truncate();
    let cell = if cut { cell.tooltip(hint(full)) } else { cell };
    let info = view()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(space::XS))
        .child(cell)
        .child(caption(member_parent(member)))
        .child(member_actions(
            group_index,
            member,
            keep,
            checked,
            video_member.then_some(playing),
        ));
    let surface = tile(decision(keep, checked)).gap(px(space::SM));
    if playing && video_member {
        return surface
            .flex_col()
            .child(playable_preview(
                &thumb,
                &member.path,
                PreviewSize::Thumb,
                true,
            ))
            .child(info);
    }
    surface
        .flex_row()
        .items_start()
        .child(playable_preview(
            &thumb,
            &member.path,
            PreviewSize::Thumb,
            false,
        ))
        .child(info)
}

/// The controls line both layouts use: the tick, size, similarity, the
/// protected badge, Play for a clip, and the keeper pick.
fn member_actions(
    group_index: usize,
    member: &GroupMember,
    keep: bool,
    checked: bool,
    play: Option<bool>,
) -> View {
    let label = if keep {
        "Keeper"
    } else if checked {
        "Selected"
    } else {
        "Select"
    };
    let similarity = member
        .similarity
        .map(|s| format!("{:.0}% similar", s * 100.0));
    let toggle_path = member.path.clone();
    let keep_path = member.path.clone();
    let play_path = member.path.clone();
    let playing = play.unwrap_or(false);
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::SM))
        .child(if member.reference && !keep {
            // A locked copy is never deleted, so it has no tick to give: a
            // disabled checkbox would promise a choice that does not exist.
            badge("Protected", BadgeTone::Raised)
                .tooltip(hint("Inside a locked folder: this copy is never deleted."))
        } else {
            view().child(tick(label, decision(keep, checked), move || {
                actions::toggle_selected(&toggle_path)
            }))
        })
        .child(caption(util::human_bytes(member.size)).tabular_nums())
        .child(when(similarity.is_some(), move || {
            caption(similarity.clone().unwrap_or_default())
        }))
        // A protected duplicate already carries "Protected" in place of its
        // tick; a protected keeper carries it beside its own.
        .child(when(member.reference && keep, || {
            badge("Protected", BadgeTone::Raised)
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
        .gap(px(space::XL))
        .p(px(space::XXL))
        .child(mode_header())
        .child(fast_scroll(&MODE_AT, dynamic(mode_list)))
}

/// The cleanup scan's title row: what was looked for, how much was found, and
/// the space it holds as the answer figure. The findings are read-only here,
/// and the line under the title says where deleting them happens.
fn mode_header() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .justify_between()
        .gap(px(space::LG))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(space::XS))
                .child(heading(|| {
                    app().with(|s| {
                        s.mode_result
                            .as_ref()
                            .map(|r| mode_title(r.mode))
                            .unwrap_or("Cleanup scan")
                            .to_string()
                    })
                }))
                .child(description(|| {
                    app().with(|s| match s.mode_result.as_ref() {
                        Some(r) => format!(
                            "{} in {} scanned · read-only here; delete with the CLI's --delete or --trash",
                            counted(r.finding_count() as u64, "finding", "findings"),
                            counted(r.files_scanned, "file", "files"),
                        ),
                        None => "Run a cleanup scan to see findings.".to_string(),
                    })
                })),
        )
        .child(
            view()
                .flex_row()
                .items_end()
                .gap(px(space::XL))
                .child(scan_again_button())
                .child(answer_figure("Reclaimable", || {
                    app().with(|s| {
                        s.mode_result
                            .as_ref()
                            .map_or_else(String::new, |r| util::human_bytes(r.reclaimable()))
                    })
                })),
        )
}

fn mode_list() -> View {
    let Some(result) = app().with(|s| s.mode_result.clone()) else {
        return view().child(caption("Run a cleanup scan to see findings."));
    };
    let mut list = view().flex_col().gap(px(space::MD));
    if app().with(|s| s.scanning) {
        list = list.child(progress_card());
    }
    if result.is_empty() {
        list = list.child(nothing_found_card(
            "Nothing to clean up",
            "The scanned folders came back clean. Add a folder, or scan again after changing the threshold.",
            false,
        ));
        return list;
    }
    // A cleanup scan over a large tree returns findings by the thousand
    // ("empty files" in a download folder, "temp files" in a home
    // directory). Rows are built for what is rendered, so the list takes a
    // page at a time, the same rule the duplicate groups already follow.
    let limit = app().with(|s| s.mode_limit);
    if !result.findings.is_empty() {
        // Single findings are one list: one card, rows split by rules, the
        // sizes on one right edge — not a stack of identical cards.
        let mut rows = card().p(px(space::XS));
        for (index, finding) in result.findings.iter().take(limit).enumerate() {
            if index > 0 {
                rows = rows.child(view().px(px(space::SM)).child(divider()));
            }
            rows = rows.child(mode_finding_row(finding));
        }
        list = list.child(rows);
    }
    for group in result.groups.iter().take(limit) {
        list = list.child(mode_group_card(group));
    }
    for error in result.errors.iter().take(limit) {
        list = list.child(error_notice(error.clone()));
    }
    let remaining = result.findings.len().saturating_sub(limit)
        + result.groups.len().saturating_sub(limit)
        + result.errors.len().saturating_sub(limit);
    if remaining > 0 {
        list = list.child(view().flex_row().justify_center().child(ghost_button(
            format!("Show more ({remaining} remaining)"),
            actions::show_more_findings,
        )));
    }
    list
}

/// A path that may not fit: cut to the room it has, with the whole path in
/// a tooltip when it was cut. In ink it is a row's identity at body size;
/// muted it is a member listed under a group at caption size.
/// About how many characters of a path a full-width row shows before the
/// front is cut.
const PATH_CHARS: usize = 110;

/// About how many characters of a file name a list row shows before the
/// name is cut and moved into a tooltip.
const NAME_CHARS: usize = 120;

fn path_cell(path: &Path, width: usize, ink: Ink) -> Text {
    let full = path.to_string_lossy().to_string();
    let shown = fit_text(&full, width);
    let truncated = shown != full;
    let cell = match ink {
        Ink::Text => body(shown).truncate(),
        Ink::Muted => caption(shown).truncate(),
    };
    if truncated {
        cell.tooltip(hint(full))
    } else {
        cell
    }
}

/// One finding: its kind, its path over its detail, its size on the right
/// edge every row shares.
fn mode_finding_row(finding: &crate::modes::Finding) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::MD))
        .px(px(space::SM))
        .py(px(space::SM))
        .child(badge(finding.kind, BadgeTone::Neutral))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(space::XXS))
                .child(path_cell(&finding.path, PATH_CHARS, Ink::Text))
                .child(caption(finding.detail.clone()).truncate()),
        )
        .child(amount(util::human_bytes(finding.size)))
}

/// A group of findings (a duplicate folder, a run of duplicate tracks): its
/// label and the space it holds, then its members as plain paths.
fn mode_group_card(group: &crate::modes::FindingGroup) -> View {
    card()
        .gap(px(space::SM))
        .p(px(space::LG))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::MD))
                .child(name(group.label.clone()).flex_1().truncate())
                .child(amount(format!(
                    "{} reclaimable",
                    util::human_bytes(group.reclaimable)
                ))),
        )
        .child(caption(group.detail.clone()))
        .children(
            group
                .members
                .iter()
                .map(|member| path_cell(member, PATH_CHARS, Ink::Muted)),
        )
}
