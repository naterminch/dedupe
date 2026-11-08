//! Main results pane: first-run guidance, live progress, duplicate groups and
//! cleanup findings.
//!
//! Group rows are built inside one `dynamic` region: every read of [`app`]
//! subscribes, so a scan, a keeper swap or a tick re-renders the list.

use nsh_core::element::input;
use nsh_core::prelude::*;

use crate::matching::{Group, GroupMember};
use crate::modes::{ModeResult, mode_title};
use crate::ui::actions;
use crate::ui::palette::{colors, radius, text};
use crate::ui::state::app;
use crate::ui::widgets::{
    caption, card, chip, danger_button, field_label, ghost_button, primary_button, secondary_button,
};
use crate::ui_helpers::fit_text;
use crate::util;

const KEEP_MODES: [&str; 5] = ["First", "Smallest", "Newest", "Oldest", "Best"];

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
        .gap(px(16.0))
        .p(px(24.0))
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
                .child(kicker("Duplicate workspace"))
                .child(title(|| {
                    if app().with(|s| s.has_scanned) {
                        "Review duplicates"
                    } else {
                        "Find duplicate files"
                    }
                }))
                .child(caption(|| {
                    let state = app();
                    if state.with(|s| s.scanning) {
                        "Scanning. Results will appear here.".to_string()
                    } else if state.with(|s| s.has_scanned) {
                        "Keep one copy. Select the rest.".to_string()
                    } else {
                        "Add folders and scan. Nothing is uploaded.".to_string()
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
                .child(ghost_button("Expand all", || {
                    actions::set_all_expanded(true)
                }))
                .child(ghost_button("Collapse all", || {
                    actions::set_all_expanded(false)
                })),
        )
        .child(
            view()
                .flex_row()
                .gap(px(4.0))
                .child(ghost_button("Select duplicates", || {
                    app().update(actions::reselect_all_dups);
                }))
                .child(ghost_button("Select none", actions::select_none)),
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

fn sim_filter() -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(field_label("Similarity"))
        .child(
            input()
                .value(|| app().with(|s| s.min_sim_input.clone()))
                .on_change(|v: &str| app().update(|s| s.min_sim_input = v.to_owned()))
                .placeholder("97")
                .placeholder_color(|| color(colors::MUTED))
                .caret_color(|| color(colors::ACCENT))
                .selection_color(|| color(colors::ACCENT))
                .w(px(64.0))
                .px(px(8.0))
                .py(px(6.0))
                .text_size(px(text::BODY))
                .text_color(colors::TEXT)
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
    view().flex_row().gap(px(2.0)).child(dynamic(|| {
        let current = app().with(|s| s.keep_index.unwrap_or(0));
        let mut row = view().flex_row().gap(px(2.0));
        for (index, label) in KEEP_MODES.iter().enumerate() {
            row = row.child(keep_chip(label, index, current == index));
        }
        row
    }))
}

fn keep_chip(label: &'static str, index: usize, active: bool) -> View {
    view()
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(radius::CONTROL))
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
        .cursor_pointer()
        .on_press(move || actions::apply_keep_mode(index))
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

fn sort_toggle() -> View {
    view().child(dynamic(|| {
        let biggest = app().with(|s| s.sort_biggest);
        ghost_button(
            if biggest {
                "Sort: biggest"
            } else {
                "Sort: path"
            },
            actions::toggle_sort_biggest,
        )
    }))
}

fn delete_bar() -> View {
    view().child(dynamic(|| {
        let (count, bytes, can_undo) = app().with(|s| {
            let targets = actions::pending_targets(s);
            (
                targets.len(),
                targets.iter().map(|t| t.size).sum::<u64>(),
                !s.last_trashed.is_empty(),
            )
        });
        let mut row = view().flex_row().gap(px(4.0)).items_center();
        if count > 0 {
            let label = format!("Delete {count} ({})", util::human_bytes(bytes));
            row = row.child(danger_button(label, actions::perform_delete));
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
    let mut list = view().flex_col().gap(px(8.0));
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
    let label = if determinate && total > 0 {
        format!("{done} / {total} files")
    } else {
        app().with(|s| s.status.clone())
    };
    card()
        .gap(px(8.0))
        .p(px(12.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(kicker("Scanning"))
                .child(caption(label)),
        )
        .child(
            view()
                .h(px(6.0))
                .w(px(320.0))
                .rounded(px(3.0))
                .bg(colors::INSET)
                .child(
                    view()
                        .h(px(6.0))
                        .w(px(320.0 * fraction))
                        .rounded(px(3.0))
                        .bg(colors::ACCENT),
                ),
        )
}

fn empty_state() -> View {
    let duplicates = crate::ui_helpers::find_mode(app().with(|s| s.find_index))
        == crate::cli::FindMode::Duplicates;
    let folders = app().with(|s| s.folders.len());
    let trash = app().with(|s| s.trash);
    card()
        .w_full()
        .max_w(px(980.0))
        .gap(px(20.0))
        .p(px(24.0))
        .self_center()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(24.0))
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
                        }))
                        .child(caption(if duplicates {
                            "One copy is always kept. Review each group and remove the rest."
                        } else {
                            "Scan for empty files, broken media, bad names, and duplicate folders."
                        })),
                )
                .child(
                    card()
                        .w(px(250.0))
                        .flex_shrink_0()
                        .gap(px(8.0))
                        .p(px(16.0))
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
                .gap(px(12.0))
                .child(step_card(
                    "01",
                    "Add folders",
                    "Choose the folders you want to scan.",
                ))
                .child(step_card(
                    "02",
                    "Run a scan",
                    if duplicates {
                        "Files, photos, and videos are compared on this PC."
                    } else {
                        "Cleanup rules check for specific problems."
                    },
                ))
                .child(step_card(
                    "03",
                    "Review results",
                    if duplicates {
                        "Keep the right copy and remove the rest."
                    } else {
                        "Inspect every finding before using the CLI."
                    },
                )),
        )
}

fn preview_row(label: &'static str, value: String) -> View {
    view().flex_col().gap(px(1.0)).child(caption(label)).child(
        text(value)
            .text_size(px(text::BODY))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colors::TEXT),
    )
}

fn step_card(step: &'static str, heading: &'static str, body: &'static str) -> View {
    card()
        .flex_1()
        .min_w(px(220.0))
        .gap(px(8.0))
        .p(px(12.0))
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
        .child(caption(body))
}

fn note_card(message: &'static str) -> View {
    card().p(px(12.0)).bg(colors::INSET).child(caption(message))
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
    let selected = app().with(|s| s.selected.clone());
    card()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .p(px(12.0))
                .child(ghost_button(if expanded { "v" } else { ">" }, move || {
                    actions::toggle_expanded(index)
                }))
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
        .child(when(expanded, move || {
            view()
                .flex_col()
                .gap(px(2.0))
                .px(px(12.0))
                .pb(px(12.0))
                .children(members.iter().map(|member| {
                    member_row(index, member.clone(), selected.contains(&member.path))
                }))
        }))
}

fn member_row(group_index: usize, member: GroupMember, checked: bool) -> View {
    let (background, foreground, label) = if member.keep {
        (colors::ACCENT_WASH_STRONG, colors::ACCENT, "KEEP")
    } else if checked {
        (colors::ACCENT_WASH, colors::ACCENT, "Selected")
    } else {
        (colors::INSET, colors::MUTED, "Select")
    };
    let toggle_path = member.path.clone();
    let keep_path = member.path.clone();
    let similarity = member
        .similarity
        .map(|s| format!("{:.0}%", s * 100.0))
        .unwrap_or_default();
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(radius::CONTROL))
        .bg(if member.keep {
            colors::ACCENT_WASH
        } else {
            colors::BACKGROUND
        })
        .child(
            view()
                .px(px(8.0))
                .py(px(2.0))
                .rounded(px(radius::CONTROL))
                .bg(background)
                .cursor_pointer()
                .on_press(move || actions::toggle_selected(&toggle_path))
                .child(
                    text(label)
                        .text_size(px(text::CAPTION))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(foreground),
                ),
        )
        .child(
            text(fit_text(&member.path.to_string_lossy(), 100))
                .flex_1()
                .truncate()
                .text_size(px(text::CAPTION))
                .text_color(colors::TEXT),
        )
        .child(caption(util::human_bytes(member.size)))
        .child(when(!similarity.is_empty(), move || {
            hint(similarity.clone())
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

fn hint(label: String) -> View {
    view().child(
        text(label)
            .text_size(px(text::CAPTION))
            .text_color(colors::MUTED),
    )
}

fn mode_workspace() -> View {
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .gap(px(16.0))
        .p(px(24.0))
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
        .gap(px(4.0))
        .p(px(10.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(chip(finding.kind, colors::INSET, colors::MUTED))
                .child(
                    text(fit_text(&finding.path.to_string_lossy(), 110))
                        .flex_1()
                        .truncate()
                        .text_size(px(text::BODY))
                        .text_color(colors::TEXT),
                )
                .child(caption(util::human_bytes(finding.size))),
        )
        .child(caption(finding.detail.clone()))
}

fn mode_group_card(group: &crate::modes::FindingGroup) -> View {
    card()
        .gap(px(4.0))
        .p(px(10.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
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
        .children(
            group
                .members
                .iter()
                .map(|member| caption(fit_text(&member.to_string_lossy(), 110))),
        )
}

fn note_card_error(message: &str) -> View {
    card()
        .p(px(10.0))
        .border_color(colors::DANGER)
        .child(caption(message.to_owned()))
}
