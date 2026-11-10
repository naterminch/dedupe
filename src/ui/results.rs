//! Main results pane: first-run guidance, live progress, duplicate groups and
//! cleanup findings.
//!
//! Group rows are built inside one `dynamic` region: every read of [`app`]
//! subscribes, so a scan, a keeper swap or a tick re-renders the list.

use std::path::{Path, PathBuf};

use nsh_base::{checkbox, collapsible, number_input, progress, radio, radio_group, toggle};
use nsh_core::AccessibilityState;
use nsh_core::element::BorderCurve;
use nsh_core::prelude::*;

use crate::matching::{Group, GroupMember};
use crate::media;
use crate::modes::{ModeResult, mode_title};
use crate::ui::actions;
use crate::ui::palette::{colors, radius, space, text};
use crate::ui::state::app;
use crate::ui::widgets::{
    caption, card, chevron, chip, danger_button, field_label, ghost_button, hint, motion,
    primary_button, secondary_button,
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
fn thumb_for(member: &GroupMember) -> Thumb {
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
        media::MediaKind::Video => app().with(|s| {
            s.posters
                .get(&member.path)
                .map_or(Thumb::PendingVideo, |poster| Thumb::Poster(poster.clone()))
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
        .child(scroll().flex_1().child(dynamic(list_body)))
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
                .child(ghost_button("Expand", || actions::set_all_expanded(true)))
                .child(ghost_button("Collapse", || {
                    actions::set_all_expanded(false)
                })),
        )
        .child(
            view()
                .flex_row()
                .gap(px(4.0))
                .child(ghost_button("Select all", || {
                    app().update(actions::reselect_all_dups);
                }))
                .child(ghost_button("Clear", actions::select_none)),
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
        .child(delete_bar())
        .child(consolidate_button())
        .child(ghost_button("Export JSON", actions::save_report))
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
        .child(secondary_button("Apply", || {
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
            row = row.child(danger_button(label, actions::perform_delete).tooltip(hint(tip)));
        }
        if can_undo {
            row = row.child(secondary_button("Undo delete", actions::undo_delete));
        }
        row
    }))
}

fn consolidate_button() -> View {
    view().child(dynamic(|| {
        if app().with(|s| s.consolidate_dest.is_some()) {
            primary_button("Move keepers here", actions::perform_consolidate)
        } else {
            secondary_button("Consolidate", actions::pick_consolidate_dir)
        }
    }))
}

fn list_body() -> View {
    let (scanning, has_scanned, total, ffprobe_note) =
        app().with(|s| (s.scanning, s.has_scanned, s.groups.len(), s.ffprobe_note));
    let mut list = view().flex_col().gap(px(space::MD));
    if scanning {
        list = list.child(progress_card());
    }
    if !has_scanned && !scanning {
        list = list.child(empty_state());
    }
    if has_scanned && total > 0 {
        list = list.child(stat_chips());
    }
    if ffprobe_note {
        list = list.child(note_card(
            "ffprobe not found on PATH - install FFmpeg for media resolution/duration.",
        ));
    }
    if has_scanned && total == 0 {
        list = list.child(note_card("No duplicate files found."));
    }
    let limit = app().with(|s| s.group_limit.min(s.groups.len()));
    for group in 0..limit {
        list = list.child(group_card(group));
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

fn group_card(position: usize) -> View {
    let (index, expanded, header, keep, dup) = app().with(|s| {
        let group = &s.groups[position];
        (
            group.index,
            s.expanded.contains(&group.index),
            group_header(group),
            group.keep_count(),
            group.dup_bytes(),
        )
    });
    let members = app().with(|s| s.groups[position].members.clone());
    // Per-member flags rather than a copy of the selection set: a scan
    // pre-selects every duplicate, so the set holds thousands of paths and
    // this runs once per visible group.
    let checked: Vec<bool> = app().with(|s| {
        members
            .iter()
            .map(|member| s.selected.contains(&member.path))
            .collect()
    });
    // Media groups keep the layout the app shipped with: every member is a
    // photo or a video, so each is shown once as a large card, two to a row.
    // Anything else (plain files, archives, mixed groups) stays a compact
    // row list, which reads better for paths than a wall of empty previews.
    let grid = members.len() >= 2 && members.iter().all(|m| is_grid_preview(&thumb_for(m)));
    let mut body = view().flex_col().gap(px(8.0)).px(px(12.0)).pb(px(12.0));
    if grid {
        for (row_index, pair) in members.chunks(2).enumerate() {
            let mut row = view().flex_row().items_stretch().gap(px(8.0));
            for (column, member) in pair.iter().enumerate() {
                row = row.child(media_card(
                    index,
                    member.clone(),
                    checked[row_index * 2 + column],
                ));
            }
            if pair.len() == 1 {
                // Odd tail: leave the lone card at half width.
                row = row.child(view().flex_1().min_w(px(0.0)));
            }
            body = body.child(row);
        }
    } else {
        for (member, is_checked) in members.iter().zip(&checked) {
            body = body.child(member_row(index, member.clone(), *is_checked));
        }
    }
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
                    .child(caption(format!(
                        "{keep} keep - {} dup",
                        members.len() - keep
                    )))
                    .child(chip(
                        util::human_bytes(dup),
                        colors::ACCENT_WASH,
                        colors::ACCENT,
                    )),
            )
            .content(body),
    )
}

/// Groups where every member has a large preview render as a card grid.
fn is_grid_preview(thumb: &Thumb) -> bool {
    matches!(thumb, Thumb::Image | Thumb::Poster(_) | Thumb::PendingVideo)
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

/// Height of a media card's preview, in points.
const CARD_PREVIEW_H: f32 = 180.0;

/// The large preview on top of a media card. Photos and videos (whose poster
/// ffmpeg extracts) share one box so cards in a row line up; a video whose
/// poster has not landed yet keeps the box with a play glyph.
fn card_preview(thumb: &Thumb, path: &Path) -> View {
    let frame = view()
        .w_full()
        .h(px(CARD_PREVIEW_H))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(radius::CONTROL))
        .bg(colors::INSET);
    let source = match thumb {
        Thumb::Image => Some(path.to_string_lossy().into_owned()),
        Thumb::Poster(poster) => Some(poster.to_string_lossy().into_owned()),
        Thumb::None | Thumb::PendingVideo => None,
    };
    match source {
        Some(source) => frame.child(
            image(source)
                .w_full()
                .h(px(CARD_PREVIEW_H))
                .object_fit(ResizeMode::Cover)
                .rounded(px(radius::CONTROL)),
        ),
        None => frame.border_1().border_color(colors::BORDER).child(
            text("▶")
                .text_size(px(text::HEADING))
                .text_color(colors::MUTED),
        ),
    }
}

/// One card of a media group: large preview, then the name, its folder and
/// the same controls a row carries. Only the border and the badge move when
/// the keeper changes, so the group never reflows during a decision.
fn media_card(group_index: usize, member: GroupMember, checked: bool) -> View {
    let thumb = thumb_for(&member);
    let similarity = member
        .similarity
        .map(|s| format!("{:.0}% similar", s * 100.0));
    // A row surface, not a card: it sits inside the group card, and a card in
    // a card draws a second border and shadow on the same edge.
    view()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .p(px(space::SM))
        .gap(px(space::SM))
        .bg(colors::PANE)
        .rounded(px(radius::CARD))
        .border_curve(BorderCurve::Continuous)
        .border_1()
        .border_color(if member.keep {
            colors::ACCENT
        } else {
            colors::BORDER
        })
        .child(card_preview(&thumb, &member.path))
        .child(
            view()
                .flex_col()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(
                    text(member_name(&member))
                        .truncate()
                        .text_size(px(text::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors::TEXT),
                )
                .child(caption(member_parent(&member))),
        )
        .child(member_actions(group_index, &member, checked, similarity))
}

fn member_row(group_index: usize, member: GroupMember, checked: bool) -> View {
    // Rendering only reads state: poster extraction is requested when a
    // group becomes visible (see `actions::request_group_posters`), never
    // while the tree is being built.
    let thumb = thumb_for(&member);
    let similarity = member
        .similarity
        .map(|s| format!("{:.0}% similar", s * 100.0));
    let name = member_name(&member);
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
    view()
        .flex_row()
        .items_start()
        .gap(px(space::SM))
        .px(px(space::SM))
        .py(px(space::XS))
        .rounded(px(radius::CONTROL))
        .bg(if member.keep {
            colors::ACCENT_WASH
        } else {
            colors::PANE
        })
        .transition_colors()
        .duration(motion(180.0))
        .child(member_thumb(&thumb, &member.path))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(4.0))
                .child(cell)
                .child(caption(member_parent(&member)))
                .child(member_actions(group_index, &member, checked, similarity)),
        )
}

/// The controls line both layouts use: keeper / selection state, size,
/// similarity, the reference badge and the Keep button.
fn member_actions(
    group_index: usize,
    member: &GroupMember,
    checked: bool,
    similarity: Option<String>,
) -> View {
    let label = if member.keep {
        "KEEP"
    } else if checked {
        "Selected"
    } else {
        "Select"
    };
    let active = member.keep || checked;
    let toggle_path = member.path.clone();
    let keep_path = member.path.clone();
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::SM))
        .child(
            checkbox()
                .checked(active)
                .disabled(member.keep)
                .on_change(move |_: &bool| actions::toggle_selected(&toggle_path))
                .px(px(8.0))
                .py(px(2.0))
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .rounded(px(radius::CONTROL))
                .bg(if member.keep {
                    colors::ACCENT_WASH_STRONG
                } else if checked {
                    colors::ACCENT_WASH
                } else {
                    colors::INSET
                })
                .border_1()
                .border_color(if active {
                    colors::ACCENT
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
                        .border_color(if active {
                            colors::ACCENT
                        } else {
                            colors::MUTED
                        })
                        .checked(|s| s.bg(colors::ACCENT).border_color(colors::ACCENT))
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
                        .text_color(if active {
                            colors::ACCENT
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
        .child(when(!member.keep, move || {
            let path = keep_path.clone();
            ghost_button("Keep", move || {
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
