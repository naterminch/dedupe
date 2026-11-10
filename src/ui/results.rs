//! Main results pane: first-run guidance, live progress, duplicate groups and
//! cleanup findings.
//!
//! Group rows are built inside one `dynamic` region: every read of [`app`]
//! subscribes, so a scan, a keeper swap or a tick re-renders the list.

use std::path::{Path, PathBuf};

use nsh_base::{checkbox, collapsible, number_input, progress, radio, radio_group, toggle};
use nsh_core::AccessibilityState;
use nsh_core::element::{BorderCurve, video};
use nsh_core::prelude::*;

use crate::matching::{Group, GroupMember};
use crate::media;
use crate::modes::{ModeResult, mode_title};
use crate::ui::actions;
use crate::ui::palette::{colors, radius, space, text};
use crate::ui::state::{Layout, app};
use crate::ui::widgets::{
    caption, card, chevron, chip, danger_button, divider, field_label, ghost_button, hint,
    icon_button, icon_chip, icon_ghost_button, icons, motion, secondary_button,
};
use crate::ui_helpers::fit_text;
use crate::util;

const KEEP_MODES: [&str; 5] = ["First", "Smallest", "Newest", "Oldest", "Best"];

/// Photo extensions the image element decodes. Formats it cannot (HEIC, SVG,
/// ICO, TIFF) keep the text-only row rather than a broken box.
const THUMB_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "bmp", "webp"];

/// Row thumbnail box, in points. Fixed so an image finishing its async load
/// never changes the row's geometry (rows would overlap mid-layout).
const THUMB_W: f32 = 72.0;
const THUMB_H: f32 = 56.0;

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
        .child(
            when(|| app().with(|s| s.mode_result.is_some()), mode_workspace)
                .otherwise(duplicate_workspace),
        )
}

fn kicker(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::ACCENT)
}

fn title(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT)
}

fn duplicate_workspace() -> View {
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .gap(px(space::LG))
        .p(px(space::XL))
        .child(header_row())
        .child(selection_row())
        .child(action_row())
        // Rebuild only when the results change shape: a scan, a threshold, a
        // delete, a consolidate, a re-sort, Show more. Keeper picks and ticks
        // update their own cards, so pressing Keep never rebuilds (or
        // reorders) the list.
        .child(scroll().flex_1().child(dynamic(|| {
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
        })))
}

fn header_row() -> View {
    view()
        .flex_row()
        .justify_between()
        .items_end()
        .gap(px(16.0))
        .child(
            view()
                .flex_col()
                .min_w(px(0.0))
                .gap(px(4.0))
                .child(title(|| {
                    if app().with(|s| s.has_scanned) {
                        "Review duplicates"
                    } else {
                        "Find duplicate files"
                    }
                })),
        )
        .child(chip(
            || {
                let state = app();
                if state.with(|s| s.scanning) {
                    "Running".to_string()
                } else if state.with(|s| s.has_scanned) {
                    format!("{} groups", state.with(|s| s.groups.len()))
                } else {
                    "Ready".to_string()
                }
            },
            colors::INSET,
            colors::MUTED,
        ))
}

fn selection_row() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
            view()
                .flex_row()
                .gap(px(4.0))
                .child(icon_ghost_button(
                    icons::EXPAND,
                    "Expand all groups",
                    || actions::set_all_expanded(true),
                ))
                .child(icon_ghost_button(
                    icons::COLLAPSE,
                    "Collapse all groups",
                    || actions::set_all_expanded(false),
                )),
        )
        .child(
            view()
                .flex_row()
                .gap(px(4.0))
                .child(icon_ghost_button(
                    icons::SELECT_ALL,
                    "Select all duplicates",
                    || app().update(actions::reselect_all_dups),
                ))
                .child(icon_ghost_button(
                    icons::CLEAR,
                    "Clear selection",
                    actions::select_none,
                )),
        )
}

fn action_row() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .child(sim_filter())
        .child(keep_modes())
        .child(sort_toggle())
        .child(layout_toggle())
        .child(delete_bar())
        .child(consolidate_button())
        .child(icon_ghost_button(
            icons::EXPORT,
            "Export JSON report",
            actions::save_report,
        ))
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

fn keep_modes() -> View {
    view().child(dynamic(|| {
        let current = app().with(|s| s.keep_index.unwrap_or(0));
        let mut row = view().flex_row().gap(px(2.0));
        for (index, label) in KEEP_MODES.iter().enumerate() {
            row = row.child(keep_chip(label, index, current == index));
        }
        radio_group().child(row)
    }))
}

fn keep_chip(label: &'static str, index: usize, active: bool) -> impl IntoElement {
    radio()
        .checked(active)
        .on_change(move |_: &bool| actions::apply_keep_mode(index))
        .accessibility_label(label)
        .px(px(space::SM))
        .py(px(space::XS))
        .rounded_full()
        .bg(if active {
            colors::ACCENT_WASH_STRONG
        } else {
            colors::INSET
        })
        .border_1()
        .border_color(if active {
            colors::ACCENT
        } else {
            colors::BORDER
        })
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if active {
                    colors::ACCENT
                } else {
                    colors::MUTED
                }),
        )
}

fn sort_toggle() -> impl IntoElement {
    dynamic(|| {
        let biggest = app().with(|s| s.sort_biggest);
        toggle()
            .pressed(biggest)
            .on_change(move |_: &bool| actions::toggle_sort_biggest())
            .accessibility_label("Sort groups by most reclaimable space first")
            .px(px(12.0))
            .py(px(6.0))
            .flex_row()
            .items_center()
            .justify_center()
            .rounded_full()
            .transition_colors()
            .duration(150.0)
            .hover(|s| s.bg(colors::INSET))
            .pressed_style(|s| {
                s.bg(colors::ACCENT_WASH_STRONG)
                    .border_color(colors::ACCENT)
            })
            .child(
                text(if biggest { "Biggest" } else { "Path" })
                    .text_size(px(text::BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::ACCENT),
            )
    })
}

/// Grid or list: the original two-column preview cards, or one compact row
/// list. It is a structural choice, so switching rebuilds the list.
fn layout_toggle() -> View {
    view().child(dynamic(|| {
        let current = app().with(|s| s.layout);
        let mut row = view().flex_row().gap(px(2.0));
        for (layout, markup, label) in [
            (Layout::Grid, icons::GRID, "Grid layout"),
            (Layout::List, icons::LIST, "List layout"),
        ] {
            row = row.child(icon_chip(markup, label, current == layout, move || {
                actions::set_layout(layout)
            }));
        }
        radio_group().child(row)
    }))
}

fn delete_bar() -> View {
    view().child(dynamic(|| {
        let (count, bytes, can_undo, trash) = app().with(|s| {
            let targets = actions::pending_targets(s);
            (
                targets.len(),
                targets.iter().map(|t| t.size).sum::<u64>(),
                !s.last_trashed.is_empty(),
                s.trash,
            )
        });
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
    let action = if trash {
        "move to the trash"
    } else {
        "permanently delete"
    };
    let mut body = view().flex_col().gap(px(space::MD));
    for group in &groups {
        let mut section = view().flex_col().gap(px(space::XS)).child(
            text(format!(
                "Group #{} - keeps {} - deletes {} ({})",
                group.index + 1,
                group.keeper,
                group.rows.len() + group.hidden_rows,
                util::human_bytes(group.bytes),
            ))
            .text_size(px(text::CAPTION))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colors::TEXT),
        );
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
                    .child(ghost_button("Keep", move || {
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
        body = body.child(section);
    }
    if hidden_groups > 0 {
        body = body.child(caption(format!("... and {hidden_groups} more group(s)")));
    }
    if count == 0 {
        body = body.child(caption("Nothing is ticked for deletion."));
    }
    let description = if count == 0 {
        "Ticking a duplicate in a group lists it here.".to_string()
    } else {
        format!(
            "This will {action} {count} file(s) ({}). Keepers are kept; every file is re-hashed before removal.",
            util::human_bytes(bytes)
        )
    };
    let panel = card()
        .w(px(520.0))
        .flex_shrink_0()
        .gap(px(space::SM))
        .p(px(space::LG))
        .shadow_lg()
        .shadow_color(colors::SHADOW)
        .child(
            text(format!("Delete {count} file(s)?"))
                .text_size(px(text::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::TEXT),
        )
        .child(caption(description))
        .child(scroll().max_h(px(320.0)).child(body))
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
    // Two layers, as a sheet is built: the scrim takes the presses that miss
    // the panel and Escape, the panel is transparent to them (box-none) so
    // only its card is hit.
    view()
        .child(
            overlay()
                .inset_0()
                .bg(colors::SCRIM)
                .on_press(actions::cancel_delete)
                .on_cancel(actions::cancel_delete),
        )
        .child(
            overlay()
                .inset_0()
                .pointer_events(PointerEvents::BoxNone)
                .flex_col()
                .items_center()
                .justify_center()
                .p(px(space::XL))
                .child(panel),
        )
}

fn consolidate_button() -> View {
    view().child(dynamic(|| {
        if app().with(|s| s.consolidate_dest.is_some()) {
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
    let mut list = view().flex_col().gap(px(space::MD));
    if scanning {
        list = list.child(dynamic(progress_card));
    }
    if !has_scanned && !scanning {
        list = list.child(dynamic(empty_state));
    }
    if has_scanned && total > 0 {
        list = list.child(dynamic(stat_chips));
    }
    if ffprobe_note {
        list = list.child(note_card(
            "ffprobe not found on PATH - install FFmpeg for media resolution/duration.",
        ));
    }
    if has_scanned && total == 0 {
        list = list.child(note_card("No duplicate files found."));
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
    list
}

fn progress_card() -> View {
    let (done, total, determinate) =
        app().with(|s| (s.scan_done, s.scan_total, s.scan_determinate));
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
        app().with(|s| s.status.clone())
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
                .child(kicker(|| app().with(|s| s.scan_phase.clone())))
                .child(caption(label)),
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
                .indicator_class(if determinate {
                    ""
                } else {
                    "w-full animate-pulse"
                })
                .indicator_style(|s| s.h(px(6.0)).rounded(px(3.0)).bg(colors::ACCENT)),
        )
}

fn empty_state() -> View {
    let duplicates = crate::ui_helpers::find_mode(app().with(|s| s.find_index))
        == crate::cli::FindMode::Duplicates;
    let folders = app().with(|s| s.folders.len());
    let trash = app().with(|s| s.trash);
    // Not a card: the preview panel and the three steps are cards, and a card
    // around them would nest surfaces two deep.
    view()
        .flex_col()
        .w_full()
        .max_w(px(980.0))
        .gap(px(space::XL))
        .self_center()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(space::XL))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(280.0))
                        .gap(px(12.0))
                        .child(kicker(if duplicates {
                            "Duplicate scanning"
                        } else {
                            "Cleanup scanning"
                        }))
                        .child(title(if duplicates {
                            "Find every copy. Keep the one you want."
                        } else {
                            "Put every stray file back in its place."
                        })),
                )
                .child(
                    card()
                        .w(px(250.0))
                        .flex_shrink_0()
                        .gap(px(space::SM))
                        .p(px(space::LG))
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    text("Scan preview")
                                        .text_size(px(text::BODY))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(colors::TEXT),
                                )
                                .child(chip("Ready", colors::ACCENT_WASH_STRONG, colors::ACCENT)),
                        )
                        .child(preview_row("Folders", format!("{folders} queued")))
                        .child(preview_row(
                            "Mode",
                            if duplicates {
                                "Duplicate scan".to_string()
                            } else {
                                "Cleanup scan".to_string()
                            },
                        ))
                        .child(preview_row(
                            "Delete",
                            if trash {
                                "To trash".to_string()
                            } else {
                                "Permanently".to_string()
                            },
                        )),
                ),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_stretch()
                .gap(px(space::MD))
                .child(step_card("01", "Add folders"))
                .child(step_card("02", "Run a scan"))
                .child(step_card("03", "Review results")),
        )
}

fn preview_row(label: &'static str, value: String) -> View {
    view()
        .flex_col()
        .gap(px(space::XXS))
        .child(caption(label))
        .child(
            text(value)
                .text_size(px(text::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::TEXT),
        )
}

fn step_card(step: &'static str, heading: &'static str) -> View {
    card()
        .flex_1()
        .min_w(px(220.0))
        .gap(px(space::SM))
        .p(px(space::MD))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(chip(step, colors::ACCENT_WASH, colors::ACCENT))
                .child(caption("Step")),
        )
        .child(
            text(heading)
                .text_size(px(text::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::TEXT),
        )
}

fn note_card(message: &'static str) -> View {
    card()
        .p(px(space::MD))
        .bg(colors::INSET)
        .child(caption(message))
}

fn stat_chips() -> View {
    let (groups, dups, reclaim, hash) = app().with(|s| {
        (
            s.groups.len(),
            s.dup_files,
            s.reclaim_bytes,
            crate::ui_helpers::hash_caption(Some(s.hash_index.unwrap_or(0))).to_string(),
        )
    });
    view()
        .flex_row()
        .flex_wrap()
        .gap(px(6.0))
        .child(chip(
            format!("{groups} groups"),
            colors::ACCENT_WASH,
            colors::ACCENT,
        ))
        .child(chip(
            format!("{dups} duplicates"),
            colors::INSET,
            colors::MUTED,
        ))
        .child(chip(
            format!("{} reclaimable", util::human_bytes(reclaim)),
            colors::INSET,
            colors::MUTED,
        ))
        .child(chip(format!("hash: {hash}"), colors::INSET, colors::MUTED))
}

fn group_header(group: &Group) -> String {
    let similarity = group
        .similarity
        .map(|s| format!(" - {:.0}% similar", s * 100.0))
        .unwrap_or_default();
    format!(
        "#{} - {} files - {}{similarity}",
        group.index + 1,
        group.members.len(),
        util::human_bytes(group.members.first().map(|m| m.size).unwrap_or(0)),
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
    let mut body = view().flex_col().gap(px(8.0)).px(px(12.0)).pb(px(12.0));
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
    card().child(
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
                    .duration(motion(150.0))
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
                        let (keep, dup) = stats.get();
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(space::SM))
                            .child(caption(format!(
                                "{keep} keep - {} dup",
                                member_count - keep
                            )))
                            .child(chip(
                                util::human_bytes(dup),
                                colors::ACCENT_WASH,
                                colors::ACCENT,
                            ))
                    })),
            )
            .content(body),
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
            .duration(motion(180.0))
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
            .duration(motion(180.0))
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
        .duration(motion(180.0));
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
                        .text_size(px(9.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::ON_ACCENT),
                )
                .child(
                    text(label)
                        .text_size(px(text::CAPTION))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if keep {
                            colors::KEEP
                        } else if checked {
                            colors::SELECT
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
            chip("REF", colors::ACCENT_WASH, colors::ACCENT)
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
            icon_ghost_button(icons::CHECK, "Keep this copy", move || {
                actions::set_keeper(group_index, &path);
            })
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
        .child(scroll().flex_1().child(dynamic(mode_list)))
}

fn mode_header() -> View {
    view()
        .flex_col()
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
        ))
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
        list = list.child(note_card("Nothing found. The scanned folders are clean."));
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
            format!("{} findings", result.finding_count()),
            colors::ACCENT_WASH,
            colors::ACCENT,
        ))
        .child(chip(
            format!("{} reclaimable", util::human_bytes(result.reclaimable())),
            colors::INSET,
            colors::MUTED,
        ))
        .child(chip(
            format!("{} files scanned", result.files_scanned),
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
                    colors::ACCENT,
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
        .child(caption(message.to_owned()))
}
