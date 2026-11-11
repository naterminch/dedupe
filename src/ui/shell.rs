//! Main-flow sidebar: source folders, per-run scope, and the scan controls.
//!
//! Global tuning lives on the Settings page, so the main flow stays a
//! three-step scan: folders, scope, Scan.

use nsh_core::prelude::*;

use crate::cli;
use crate::ui::actions;
use crate::ui::palette::{colors, radius, space, text};
use crate::ui::state::{FolderEntry, app};
use crate::ui::widgets::{
    caption, card, chip, danger_button, field, field_label, icon_button, icon_ghost_button, icons,
    primary_button, toggle_switch,
};
use crate::ui_helpers::{self, fit_text};

/// The left column of the main flow.
pub(crate) fn sidebar() -> impl IntoElement {
    view()
        .flex_col()
        .w(px(320.0))
        .h_full()
        .flex_shrink_0()
        .bg(colors::BACKGROUND)
        .border_r_1()
        .border_color(colors::BORDER)
        .child(header_block())
        .child(
            scroll().flex_1().child(
                view()
                    .flex_col()
                    .gap(px(space::MD))
                    .px(px(space::XL))
                    .pb(px(space::XL))
                    .child(scope_card()),
            ),
        )
        .child(scan_card())
}

fn header_block() -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(space::MD))
        .px(px(space::XL))
        .pt(px(space::XL))
        .pb(px(space::LG))
        .child(
            view()
                .flex_row()
                .items_start()
                .justify_between()
                .gap(px(space::MD))
                .child(
                    view()
                        .flex_col()
                        .gap(px(space::XS))
                        .child(section_kicker("Source folders"))
                        .child(caption("Protected folders are never deleted.")),
                )
                .child(chip(
                    || app().with(|s| s.folders.len().to_string()),
                    colors::INSET,
                    colors::MUTED,
                )),
        )
        .child(folder_list())
        .child(add_row())
}

fn section_kicker(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::ACCENT)
}

fn folder_list() -> impl IntoElement {
    view().flex_col().gap(px(space::SM)).child(
        when(|| app().with(|s| s.folders.is_empty()), empty_folders).otherwise(|| {
            each(
                || app().with(|s| s.folders.clone()),
                |f: &FolderEntry| f.path.clone(),
                folder_row,
            )
        }),
    )
}

fn empty_folders() -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(space::XS))
        .p(px(space::MD))
        .rounded(px(radius::CARD))
        .border_1()
        .border_dashed()
        .border_color(colors::BORDER)
        .child(
            text("No folders yet")
                .text_size(px(text::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::TEXT),
        )
        .child(caption("Add a folder below or browse to begin."))
}

fn folder_row(folder: ReadSignal<FolderEntry>, index: ReadSignal<usize>) -> impl IntoElement {
    dynamic(move || {
        let reference = folder.with(|f| f.reference);
        let path = folder.with(|f| fit_text(&f.path, 60));
        view()
            .flex_col()
            .gap(px(space::SM))
            .p(px(space::MD))
            .rounded(px(radius::CONTROL))
            .bg(if reference {
                colors::ACCENT_WASH
            } else {
                colors::PANE
            })
            .transition_colors()
            .duration(180.0)
            .border_1()
            .border_color(if reference {
                colors::ACCENT_WASH_STRONG
            } else {
                colors::BORDER
            })
            .child(
                text(path)
                    .text_size(px(text::BODY))
                    .truncate()
                    .text_color(colors::TEXT),
            )
            .child(
                view()
                    .flex_row()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(protect_chip(reference, index))
                    .child(icon_ghost_button(
                        icons::CLEAR,
                        "Remove folder",
                        move || {
                            actions::remove_folder(index.get());
                        },
                    )),
            )
    })
}

fn protect_chip(checked: bool, index: ReadSignal<usize>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::XXS))
        .px(px(space::SM))
        .py(px(space::XXS))
        .rounded_full()
        .bg(if checked {
            colors::ACCENT_WASH_STRONG
        } else {
            colors::INSET
        })
        .border_1()
        .border_color(if checked {
            colors::ACCENT
        } else {
            colors::BORDER
        })
        .child(
            text(if checked { "Protected" } else { "Protect" })
                .text_size(px(text::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if checked {
                    colors::ACCENT
                } else {
                    colors::MUTED
                }),
        )
        .child(toggle_switch(
            checked,
            "Protect this folder from deletion",
            move || actions::toggle_reference(index.get()),
        ))
}

fn add_row() -> impl IntoElement {
    view()
        .flex_row()
        .gap(px(space::SM))
        .items_center()
        .child(field(
            "Folder path",
            || app().with(|s| s.folder_input.clone()),
            |v| {
                app().update(|s| s.folder_input = v);
            },
        ))
        .child(icon_button(
            icons::PLUS,
            "Add folder",
            actions::add_from_input,
        ))
        .child(icon_button(
            icons::FOLDER,
            "Browse for a folder",
            actions::browse_folder,
        ))
}

fn scope_card() -> impl IntoElement {
    card()
        .gap(px(space::MD))
        .p(px(space::MD))
        .child(
            view()
                .flex_row()
                .items_start()
                .justify_between()
                .gap(px(space::MD))
                .child(section_kicker("Scan scope")),
        )
        .child(labelled(
            "File types",
            None,
            field(
                "e.g. jpg, png, mp4",
                || app().with(|s| s.types_input.clone()),
                |v| {
                    app().update(|s| s.types_input = v);
                },
            ),
        ))
        .child(labelled(
            "Min size",
            Some("e.g. 10MB, unlimited"),
            field(
                "unlimited",
                || app().with(|s| s.min_size_input.clone()),
                |v| {
                    app().update(|s| s.min_size_input = v);
                },
            ),
        ))
        .child(labelled(
            "Max size",
            Some("e.g. 2GB, unlimited"),
            field(
                "unlimited",
                || app().with(|s| s.max_size_input.clone()),
                |v| {
                    app().update(|s| s.max_size_input = v);
                },
            ),
        ))
        .child(labelled(
            "Max depth",
            Some("empty = unlimited"),
            field(
                "empty = unlimited",
                || app().with(|s| s.max_depth_input.clone()),
                |v| {
                    app().update(|s| s.max_depth_input = v);
                },
            ),
        ))
        .child(labelled(
            "Skip directories",
            Some("; separated"),
            field(
                "node_modules; target",
                || app().with(|s| s.exclude_dir_input.clone()),
                |v| {
                    app().update(|s| s.exclude_dir_input = v);
                },
            ),
        ))
        .child(labelled(
            "Skip paths containing",
            Some("; separated"),
            field(
                "C:\\Windows; /proc",
                || app().with(|s| s.exclude_path_input.clone()),
                |v| {
                    app().update(|s| s.exclude_path_input = v);
                },
            ),
        ))
        .child(labelled(
            "Ignore globs",
            Some("; separated"),
            field(
                "*.tmp; ~$*",
                || app().with(|s| s.ignore_input.clone()),
                |v| {
                    app().update(|s| s.ignore_input = v);
                },
            ),
        ))
}

fn labelled(
    label: &'static str,
    hint: Option<&'static str>,
    input: impl IntoElement,
) -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(space::XS))
        .child(
            view()
                .flex_row()
                .gap(px(space::XS))
                .items_center()
                .child(field_label(label))
                .child(hint.map(caption)),
        )
        .child(input)
}

fn scan_card() -> impl IntoElement {
    view()
        .px(px(space::XL))
        .pt(px(space::SM))
        .pb(px(space::XL))
        .child(
            card()
                .gap(px(space::MD))
                .p(px(space::MD))
                .border_color(colors::ACCENT_WASH_STRONG)
                .child(
                    view()
                        .flex_row()
                        .items_start()
                        .justify_between()
                        .gap(px(space::MD))
                        .child(
                            view()
                                .flex_col()
                                .gap(px(space::XS))
                                .child(section_kicker(|| {
                                    if app().with(|s| s.scanning) {
                                        "Scanning"
                                    } else {
                                        "Ready to scan"
                                    }
                                })),
                        )
                        .child(chip(
                            || format!("{} folders", app().with(|s| s.folders.len())),
                            colors::INSET,
                            colors::MUTED,
                        )),
                )
                .child(media_status())
                .child(
                    when(
                        || app().with(|s| s.scanning),
                        || danger_button("Stop", actions::stop_scan).w_full(),
                    )
                    .otherwise(|| primary_button(scan_label, actions::start_scan).w_full()),
                ),
        )
}

fn scan_label() -> &'static str {
    if ui_helpers::find_mode(app().with(|s| s.find_index)) == cli::FindMode::Duplicates {
        "Scan for duplicates"
    } else {
        "Run cleanup scan"
    }
}

/// ffmpeg/ffprobe availability, shown above the Scan button.
fn media_status() -> impl IntoElement {
    dynamic(|| {
        let (checked, ffmpeg, ffprobe) =
            app().with(|s| (s.media_checked, s.ffmpeg_ok, s.ffprobe_ok));
        if !checked {
            return chip("Checking for ffmpeg...", colors::INSET, colors::MUTED);
        }
        match (ffmpeg, ffprobe) {
            (true, true) => chip("ffmpeg ready", colors::ACCENT_WASH_STRONG, colors::ACCENT),
            (false, true) => chip(
                "ffmpeg missing - video compare off",
                colors::INSET,
                colors::MUTED,
            ),
            (true, false) => chip(
                "ffprobe missing - media details off",
                colors::INSET,
                colors::MUTED,
            ),
            (false, false) => chip(
                "ffmpeg missing - video compare + details off",
                colors::INSET,
                colors::MUTED,
            ),
        }
    })
}
