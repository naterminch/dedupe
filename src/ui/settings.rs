//! Settings page: General (behavior, cache and reset) and Preferences (scan
//! defaults).
//!
//! Every control writes straight into [`AppState`] and persists through
//! `actions::save_prefs`.

use std::cell::Cell;

use nsh_core::prelude::*;

use crate::cache;
use crate::cli::{FindMode, MatchMode};
use crate::ui::actions;
use crate::ui::palette::space;
use crate::ui::state::{SettingsTab, app};
use crate::ui::widgets::{
    SelectItem, caption, card, description, divider, dropdown_trigger, fast_scroll, heading, icons,
    number_field, number_text, option_select, panel_title, radio_segmented, secondary_button,
    section_title, side_pane, side_tabs, toggle_switch,
};
use crate::ui_helpers::{counted, find_mode_caption, hash_caption};
use crate::util;

pub(crate) const FIND_LABELS: [&str; 11] = [
    "Duplicates",
    "Empty files",
    "Empty folders",
    "Big files",
    "Temp files",
    "Broken symlinks",
    "Corrupt media",
    "Wrong extension",
    "Bad names",
    "Duplicate folders",
    "Music",
];

const MATCH_LABELS: [&str; 4] = ["Content", "Size", "Name", "Fuzzy name"];
const HASH_LABELS: [&str; 3] = ["blake3", "sha256", "md5"];
const IMAGE_HASH_LABELS: [&str; 5] = ["Auto (dHash)", "dHash", "pHash", "aHash", "Color grid"];
const KEEP_LABELS: [&str; 5] = ["First", "Smallest", "Newest", "Oldest", "Best quality"];
const MUSIC_LABELS: [&str; 2] = ["Embedded tags", "Audio fingerprint"];

// Where the settings page sits, as the wheel last left it (`fast_scroll`).
thread_local! {
    static SETTINGS_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
}

/// Puts the settings page back at its top: leaving it unmounts the page, so
/// the wheel's remembered offset would point into a page that opens at the
/// start again.
pub(crate) fn reset_scroll() {
    SETTINGS_AT.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
}

pub(crate) fn settings_page() -> View {
    view()
        .flex_row()
        .flex_1()
        .h_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .child(nav())
        .child(view().flex_1().h_full().min_w(px(0.0)).child(fast_scroll(
            &SETTINGS_AT,
            dynamic(|| {
                // The page follows the settings it shows, not the app's
                // progress: reading the app signal directly rebuilt every
                // control here on each scan tick (and re-walked the cache
                // directory with it).
                let shape = computed(settings_shape);
                shape.get();
                match app().with_untracked(|s| s.settings_tab) {
                    SettingsTab::General => general_page(),
                    SettingsTab::Preferences => preferences_page(),
                }
            }),
        )))
}

/// Everything the Settings pages render from the app state.
///
/// A rebuild follows a change here, so a scan's progress ticks leave the
/// page alone while a real setting change still redraws it. The number
/// inputs are absent on purpose: they hold their own text and follow the
/// state reactively, so typing never rebuilds the page.
#[derive(Clone, PartialEq)]
struct SettingsShape {
    tab: SettingsTab,
    cache_msg: String,
    trash: bool,
    remember_folders: bool,
    wheel_page: bool,
    find_index: usize,
    /// Match mode, hash algorithm, no-cache, image fingerprint, partial
    /// clips, music-by-content: the choices the Preferences page builds from.
    matching: (usize, usize, bool, usize, bool, bool),
    keep_index: usize,
    sort_biggest: bool,
    exact: bool,
}

fn settings_shape() -> SettingsShape {
    app().with(|s| SettingsShape {
        tab: s.settings_tab,
        cache_msg: s.cache_msg.clone(),
        trash: s.trash,
        remember_folders: s.remember_folders,
        wheel_page: s.wheel_page,
        find_index: s.find_index.unwrap_or(0),
        matching: (
            s.match_index.unwrap_or(0),
            s.hash_index.unwrap_or(0),
            s.no_cache,
            s.image_hash_index.unwrap_or(0),
            s.partial_clips,
            s.music_by_content,
        ),
        keep_index: s.keep_index.unwrap_or(0),
        sort_biggest: s.sort_biggest,
        exact: s.exact,
    })
}

fn nav() -> View {
    side_pane(NAV_WIDTH)
        .gap(px(space::MD))
        .px(px(space::SM))
        .py(px(space::LG))
        .child(view().px(px(space::SM)).child(section_title("Settings")))
        .child(view().child(dynamic(|| {
            // The chosen tab alone rebuilds these; a scan tick while Settings
            // is open would restart their hover and swallow presses.
            let tab = computed(|| app().with(|s| s.settings_tab));
            side_tabs(
                &[
                    ("General", SettingsTab::General),
                    ("Preferences", SettingsTab::Preferences),
                ],
                tab.get(),
                actions::set_settings_tab,
            )
        })))
}

/// The settings nav's width: its two labels and the title, with room to
/// spare, and every other point for the page.
const NAV_WIDTH: f32 = 200.0;

/// A settings page: its title, then its sections in reading order, each
/// split from the next by a full-width rule.
fn page_column(
    title: &'static str,
    summary: &'static str,
    sections: Vec<View>,
    footer: View,
) -> View {
    let mut column = view()
        .flex_col()
        .w_full()
        .max_w(px(PAGE_MAX_W))
        .gap(px(space::XXL))
        .px(px(space::XXL))
        .py(px(space::XXL))
        .child(
            view()
                .flex_col()
                .gap(px(space::XS))
                .child(heading(title))
                .child(description(summary)),
        );
    for section in sections {
        column = column.child(divider()).child(section);
    }
    column.child(footer)
}

/// The widest a settings page grows: a label column and a panel of a
/// comfortable measure. Wider, the switches drift away from their titles.
const PAGE_MAX_W: f32 = 1040.0;

/// The narrowest a row's name and description get before the control wraps
/// under them, and the narrowest a section's panel gets before it wraps under
/// its label column.
const ROW_LABEL_MIN_W: f32 = 240.0;
const PANEL_MIN_W: f32 = 360.0;

/// The label column of a section: its title and what it governs.
const SECTION_LABEL_W: f32 = 240.0;

/// One row inside a section's panel: the setting's name over what it does,
/// the control at the far end. A narrow window stacks the control under its
/// label rather than pushing the row past the panel's edge.
fn row(label: &'static str, hint: impl IntoReactive<SharedString>, control: View) -> View {
    view()
        .flex_row()
        .flex_wrap()
        .w_full()
        .items_center()
        .justify_between()
        .gap(px(space::LG))
        .px(px(space::LG))
        .py(px(space::MD))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(ROW_LABEL_MIN_W))
                .gap(px(space::XXS))
                .child(panel_title(label))
                .child(caption(hint)),
        )
        .child(view().flex_shrink_0().child(control))
}

/// Like [`row`], but the control takes the panel's full width under its
/// name: a set of segments that needs the room.
fn choice_row(label: &'static str, hint: impl IntoReactive<SharedString>, control: View) -> View {
    view()
        .flex_col()
        .w_full()
        .min_w(px(0.0))
        .gap(px(space::SM))
        .px(px(space::LG))
        .py(px(space::MD))
        .child(
            view()
                .flex_col()
                .gap(px(space::XXS))
                .child(panel_title(label))
                .child(caption(hint)),
        )
        .child(control)
}

/// One section: a label column (title and what it governs) beside the
/// panel that holds its rows. The column is never empty beside a tall panel:
/// the description is the section's help.
fn section(title: &'static str, summary: &'static str, rows: Vec<View>) -> View {
    let mut panel = card().flex_1().min_w(px(PANEL_MIN_W));
    for (index, content) in rows.into_iter().enumerate() {
        if index > 0 {
            panel = panel.child(view().px(px(space::LG)).child(divider()));
        }
        panel = panel.child(content);
    }
    view()
        .flex_row()
        .flex_wrap()
        .w_full()
        .items_start()
        .gap(px(space::XL))
        .child(
            view()
                .flex_col()
                .w(px(SECTION_LABEL_W))
                .flex_shrink_0()
                .gap(px(space::XS))
                .pt(px(space::MD))
                .child(section_title(title))
                .child(description(summary)),
        )
        .child(panel)
}

/// One of a few options, as a segmented control.
fn choices<const N: usize>(
    options: [&'static str; N],
    selected: usize,
    on_pick: impl Fn(usize) + Clone + 'static,
) -> View {
    radio_segmented(options, selected, on_pick)
}

/// A setting's control is the switch alone: the row already carries the
/// setting's name, so a repeated short label beside the thumb is read twice
/// (and spoken twice). The label stays as the switch's accessible name.
fn toggle(label: &'static str, value: bool, on_press: impl Fn() + 'static) -> View {
    view().child(toggle_switch(value, label, on_press))
}

/// A number setting, saved as it changes.
fn number(
    placeholder: &'static str,
    get: fn() -> String,
    set: fn(String),
    min: f64,
    max: f64,
) -> View {
    number_field(
        placeholder,
        move || get().trim().parse::<f64>().ok(),
        move |next| {
            set(number_text(next));
            actions::save_prefs();
        },
        (min, max),
        128.0,
    )
}

/// What to find, as a select: eleven scan types do not fit a segmented
/// control, and a wrapped one reads as two controls.
fn find_choice(selected: usize) -> View {
    let items: Vec<SelectItem> = FIND_LABELS
        .iter()
        .map(|label| SelectItem::new(*label, *label))
        .collect();
    let label = move || SharedString::from(FIND_LABELS[selected.min(FIND_LABELS.len() - 1)]);
    view().child(option_select(
        items,
        "What to find",
        move || Some(label()),
        |picked: &Option<SharedString>| {
            let Some(picked) = picked.as_deref() else {
                return;
            };
            if let Some(index) = FIND_LABELS.iter().position(|label| *label == picked) {
                app().update(|s| s.find_index = Some(index));
                actions::save_prefs();
            }
        },
        dropdown_trigger(icons::SEARCH, label),
    ))
}

fn general_page() -> View {
    let (files, bytes) = cache::cache_summary();
    let cache_msg = app().with(|s| s.cache_msg.clone());
    let trash = app().with(|s| s.trash);
    let remember = app().with(|s| s.remember_folders);
    let wheel_page = app().with(|s| s.wheel_page);
    page_column(
        "General",
        "Everyday behavior and the local scan cache.",
        vec![
            section(
                "Behavior",
                "App-wide behavior for scans and deletions.",
                vec![
                    row(
                        "Move deleted files to trash",
                        "Deleted duplicates go to the system trash so they can be restored.",
                        toggle("Trash", trash, || {
                            app().update(|s| s.trash = !s.trash);
                            actions::save_prefs();
                        }),
                    ),
                    row(
                        "Remember scan folders",
                        "Restore the last folder list the next time the app opens.",
                        toggle("Remember", remember, || {
                            app().update(|s| s.remember_folders = !s.remember_folders);
                            actions::save_prefs();
                        }),
                    ),
                    row(
                        "Page-sized mouse wheel",
                        "A wheel notch moves a whole page of the list instead of a few rows. Touchpads keep their own, finer scrolling.",
                        toggle("Page", wheel_page, || {
                            app().update(|s| s.wheel_page = !s.wheel_page);
                            actions::save_prefs();
                        }),
                    ),
                ],
            ),
            section(
                "Cache",
                "Hashes and fingerprints cached between runs in ~/.dedupe/.",
                vec![row(
                    "Cache on disk",
                    format!(
                        "{} · {}",
                        counted(files as u64, "file", "files"),
                        util::human_bytes(bytes)
                    ),
                    secondary_button("Clear caches", actions::clear_cache),
                )],
            ),
            section(
                "Reset",
                "Put the settings on this page and on Preferences back to their defaults.",
                vec![row(
                    "Reset all settings",
                    "Restores scan defaults, filters, toggles and the theme. Folders and current results stay.",
                    secondary_button("Reset to defaults", actions::reset_settings),
                )],
            ),
        ],
        view().child(caption(cache_msg)),
    )
}

fn preferences_page() -> View {
    let find_index = app().with(|s| s.find_index.unwrap_or(0));
    let mode = crate::ui_helpers::find_mode(Some(find_index));
    let duplicates = mode == FindMode::Duplicates;
    let music = mode == FindMode::Music;
    let exact = app().with(|s| s.exact);

    let mut scan_rows = vec![row(
        "What to find",
        find_mode_caption(Some(find_index)),
        find_choice(find_index),
    )];
    if mode == FindMode::BigFiles {
        scan_rows.push(row(
            "Largest files to report",
            "How many entries the big-files scan lists, biggest first.",
            number(
                "50",
                || app().with(|s| s.top_input.clone()),
                |value| {
                    app().update(|s| s.top_input = value);
                },
                1.0,
                10_000.0,
            ),
        ));
    }

    let mut sections = vec![section(
        "Scan type",
        "What the next scan looks for.",
        scan_rows,
    )];
    if duplicates {
        sections.push(duplicates_section());
        if !exact {
            sections.push(media_section());
        }
    }
    if music {
        sections.push(music_section());
    }
    sections.push(system_section());
    page_column(
        "Preferences",
        "Defaults for the next scan. Changes save as you make them.",
        sections,
        view().child(caption("Saved to dedupe.json in ~/.dedupe/.")),
    )
}

fn duplicates_section() -> View {
    let match_index = app().with(|s| s.match_index.unwrap_or(0));
    let hash_index = app().with(|s| s.hash_index.unwrap_or(0));
    let exact = app().with(|s| s.exact);
    let no_cache = app().with(|s| s.no_cache);
    let mut rows = vec![choice_row(
        "Match duplicates by",
        "Content hashes bytes. Size groups same-size files. Name matches base names. Fuzzy tolerates small typos.",
        choices(MATCH_LABELS, match_index, |index| {
            app().update(|s| s.match_index = Some(index));
            actions::save_prefs();
        }),
    )];
    if crate::ui_helpers::match_mode(Some(match_index)) == MatchMode::Fuzzy {
        rows.push(row(
            "Fuzzy threshold %",
            "Minimum base-name similarity. 85% is a good default.",
            number(
                "85",
                || app().with(|s| s.fuzzy_threshold_input.clone()),
                |value| {
                    app().update(|s| s.fuzzy_threshold_input = value);
                },
                0.0,
                100.0,
            ),
        ));
    }
    rows.push(choice_row(
        "Hash algorithm",
        hash_caption(Some(hash_index)),
        choices(HASH_LABELS, hash_index, |index| {
            app().update(|s| s.hash_index = Some(index));
            actions::save_prefs();
        }),
    ));
    rows.push(row(
        "Exact duplicates only",
        "Skip the similar-media pass; only byte-identical files are grouped.",
        toggle("Exact", exact, || {
            app().update(|s| s.exact = !s.exact);
            actions::save_prefs();
        }),
    ));
    rows.push(row(
        "Ignore cache and re-hash everything",
        "Ignore cached hashes and fingerprints for this run.",
        toggle("No cache", no_cache, || {
            app().update(|s| s.no_cache = !s.no_cache);
            actions::save_prefs();
        }),
    ));
    section(
        "Duplicates",
        "Only used when scanning for duplicates.",
        rows,
    )
}

fn media_section() -> View {
    let image_hash = app().with(|s| s.image_hash_index.unwrap_or(0));
    let partial_clips = app().with(|s| s.partial_clips);
    let mut rows = vec![
        row(
            "Similarity %",
            "How close two images or videos must be to group. 97% catches re-encodes.",
            number(
                "97",
                || app().with(|s| s.similarity_input.clone()),
                |value| {
                    app().update(|s| s.similarity_input = value);
                },
                0.0,
                100.0,
            ),
        ),
        choice_row(
            "Image fingerprint",
            "dHash is fast and default. pHash tolerates crops and colour shifts. aHash ignores colour. Color grid compares average colours.",
            choices(IMAGE_HASH_LABELS, image_hash, |index| {
                app().update(|s| s.image_hash_index = Some(index));
                actions::save_prefs();
            }),
        ),
        row(
            "Find shorter clips inside longer videos",
            "Also look for a shorter video contained in a longer one.",
            toggle("Partial clips", partial_clips, || {
                app().update(|s| s.partial_clips = !s.partial_clips);
                actions::save_prefs();
            }),
        ),
    ];
    if partial_clips {
        rows.push(row(
            "Partial-clip similarity %",
            "How much of the shorter clip must match inside the longer one.",
            number(
                "85",
                || app().with(|s| s.partial_similarity_input.clone()),
                |value| {
                    app().update(|s| s.partial_similarity_input = value);
                },
                0.0,
                100.0,
            ),
        ));
    }
    section(
        "Images & video",
        "Similar-media detection runs alongside exact duplicate matching.",
        rows,
    )
}

fn music_section() -> View {
    let by_content = app().with(|s| s.music_by_content);
    section(
        "Music",
        "How duplicate tracks are matched when scanning for music.",
        vec![choice_row(
            "Match tracks by",
            "Embedded tags compare artist, title and album. Audio fingerprint compares the audio itself.",
            choices(MUSIC_LABELS, usize::from(by_content), |index| {
                app().update(|s| s.music_by_content = index == 1);
                actions::save_prefs();
            }),
        )],
    )
}

fn system_section() -> View {
    let keep_index = app().with(|s| s.keep_index.unwrap_or(0));
    let sort_biggest = app().with(|s| s.sort_biggest);
    section(
        "System",
        "Performance and the defaults applied to new scans.",
        vec![
            row(
                "Worker threads",
                "Parallel hash workers. 0 uses every core but one.",
                number(
                    "0",
                    || app().with(|s| s.jobs_input.clone()),
                    |value| {
                        app().update(|s| s.jobs_input = value);
                    },
                    0.0,
                    64.0,
                ),
            ),
            choice_row(
                "Default keep",
                "Which copy a new scan keeps when duplicates are found.",
                choices(KEEP_LABELS, keep_index, actions::set_keep_rule),
            ),
            row(
                "Sort by most reclaimable space",
                "List groups with the most reclaimable bytes first.",
                toggle("Biggest first", sort_biggest, || {
                    app().update(|s| s.sort_biggest = !s.sort_biggest);
                    actions::save_prefs();
                }),
            ),
        ],
    )
}
