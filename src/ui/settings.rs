//! Settings page: General (behavior, cache and reset) and Preferences (scan
//! defaults).
//!
//! Every control writes straight into [`AppState`] and persists through
//! `actions::save_prefs`.

use std::cell::Cell;

use nsh_base::{number_input, radio, radio_group, tab, tabs};
use nsh_core::prelude::*;

use crate::cache;
use crate::cli::{FindMode, MatchMode};
use crate::ui::actions;
use crate::ui::palette::{colors, radius, space, text};
use crate::ui::state::{SettingsTab, app};
use crate::ui::widgets::{
    bezel, bezel_core, caption, divider, fast_scroll, focus_ring, heading, help_dot, motion,
    page_heading, panel_title, secondary_button, selection, timing, toggle_switch,
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
    view()
        .flex_col()
        .w(px(280.0))
        .h_full()
        .flex_shrink_0()
        .gap(px(space::SM))
        .p(px(space::LG))
        .bg(colors::SIDEBAR)
        .border_r_1()
        .border_color(colors::BORDER)
        .child(heading("Settings"))
        .gap(px(2.0))
        .child(view().child(dynamic(|| {
            // The chosen tab alone rebuilds these; a scan tick while Settings
            // is open would restart their hover and swallow presses.
            let tab = computed(|| app().with(|s| s.settings_tab));
            let tab = tab.get();
            tabs()
                .w_full()
                .gap(px(2.0))
                .indicator_class("inset-x-0")
                .indicator_style(|s| {
                    s.rounded(px(radius::CONTROL))
                        .bg(colors::ACCENT_WASH_STRONG)
                })
                .child(nav_item("General", SettingsTab::General, tab))
                .child(nav_item("Preferences", SettingsTab::Preferences, tab))
        })))
}

fn nav_item(label: &'static str, which: SettingsTab, current: SettingsTab) -> impl IntoElement {
    let active = which == current;
    tab()
        .selected(active)
        .on_press(move |_: &()| actions::set_settings_tab(which))
        .accessibility_label(label)
        .w_full()
        .px(px(space::MD))
        .py(px(space::SM))
        .child(
            text(label)
                .text_size(px(text::BODY))
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if active {
                    colors::ACCENT_INK
                } else {
                    colors::MUTED
                }),
        )
}

fn page_column(children: Vec<View>) -> View {
    let mut column = view()
        .flex_col()
        .w_full()
        .max_w(px(970.0))
        .gap(px(space::XXXL))
        .px(px(space::XXXL))
        .py(px(space::XXL));
    for child in children {
        column = column.child(child);
    }
    column
}

/// A page's own heading: the title, then one line of what it governs. The
/// upper-case copy of the title above it said the same word twice.
fn header(title: &'static str, description: &'static str) -> View {
    view()
        .flex_col()
        .gap(px(space::XS))
        .child(page_heading(title))
        .child(caption(description))
}

fn row(label: &'static str, hint: impl IntoReactive<SharedString>, control: View) -> View {
    // A narrow window stacks the control under its label rather than pushing
    // the row past the card's edge.
    view()
        .flex_row()
        .flex_wrap()
        .w_full()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(space::XS))
                .child(panel_title(label))
                .child(help_dot(hint)),
        )
        .child(view().flex_shrink_0().child(control))
}

fn section(title: &'static str, description: &'static str, rows: Vec<View>) -> View {
    let mut body = view().flex_col().w_full().gap(px(space::SM));
    for (index, content) in rows.into_iter().enumerate() {
        if index > 0 {
            body = body.child(divider());
        }
        body = body.child(content);
    }
    bezel().w_full().child(
        bezel_core()
            .gap(px(space::MD))
            .p(px(space::LG))
            .child(panel_title(title))
            .child(caption(description))
            .child(body),
    )
}

fn choice_chip(
    label: &'static str,
    active: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let (fill, border, content) = selection(active);
    radio()
        .checked(active)
        .on_change(move |_: &bool| on_press())
        .accessibility_label(label)
        .px(px(space::SM))
        .py(px(space::XS))
        .rounded(px(radius::CONTROL))
        .bg(fill)
        .border_1()
        .border_color(border)
        // Choosing a different option cross-fades the pill instead of
        // repainting it. The idle pill still answers the pointer and the
        // keyboard: it looked inert without a hover or a focus ring.
        .transition_colors()
        .duration(motion(timing::FAST))
        .hover(move |s| if active { s } else { s.bg(colors::ELEVATED) })
        .active(|s| s.opacity(0.9))
        .focus_visible(focus_ring)
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

fn choices<const N: usize>(
    options: [&'static str; N],
    selected: usize,
    on_pick: impl Fn(usize) + Clone + 'static,
) -> View {
    // The wrap row needs a definite width or flex-wrap hugs everything
    // onto one line; every wrapper below therefore carries `w_full`.
    let mut row = view()
        .flex_row()
        .flex_wrap()
        .w_full()
        .min_w(px(0.0))
        .gap(px(4.0));
    for (index, label) in options.iter().enumerate() {
        let pick = on_pick.clone();
        row = row.child(choice_chip(label, index == selected, move || pick(index)));
    }
    view()
        .w_full()
        .min_w(px(0.0))
        .flex_col()
        .child(radio_group().w_full().child(row))
}

/// Like [`row`], but for choice chips: the label and hint stack on top and
/// the chips take the card's full width, so large sets (the 11 scan types)
/// wrap onto several lines instead of overflowing the card edge.
fn choice_row(label: &'static str, hint: impl IntoReactive<SharedString>, control: View) -> View {
    view()
        .flex_col()
        .w_full()
        .min_w(px(0.0))
        .gap(px(space::SM))
        .child(
            view()
                .flex_row()
                .items_center()
                .min_w(px(0.0))
                .gap(px(space::XS))
                .child(panel_title(label))
                .child(help_dot(hint)),
        )
        .child(control)
}

/// A setting's control is the switch alone: the row already carries the
/// setting's name, so a repeated short label beside the thumb is read twice
/// (and spoken twice). The label stays as the switch's accessible name.
fn toggle(label: &'static str, value: bool, on_press: impl Fn() + 'static) -> View {
    view().child(toggle_switch(value, label, on_press))
}

fn number(
    placeholder: &'static str,
    get: fn() -> String,
    set: fn(String),
    min: f64,
    max: f64,
) -> View {
    view().w(px(150.0)).child(
        number_input()
            .value(move || get().trim().parse::<f64>().ok())
            .min(min)
            .max(max)
            .step(1.0)
            .placeholder(placeholder)
            .on_change(move |next: &Option<f64>| {
                set(match next {
                    Some(value) if value.fract() == 0.0 => format!("{}", *value as i64),
                    Some(value) => format!("{value}"),
                    None => String::new(),
                });
                actions::save_prefs();
            })
            .input_style(|s| s.flex_1().text_center().text_color(colors::TEXT))
            .decrement_button(text("−").text_color(colors::MUTED))
            .increment_button(text("+").text_color(colors::MUTED))
            .decrement_button_style(|s| s.w(px(22.0)).items_center().justify_center())
            .increment_button_style(|s| s.w(px(22.0)).items_center().justify_center())
            .flex_row()
            .h(px(space::XXL))
            .bg(colors::INSET)
            .border_1()
            .border_color(colors::BORDER)
            .rounded(px(radius::CONTROL)),
    )
}

fn general_page() -> View {
    let (files, bytes) = cache::cache_summary();
    let cache_msg = app().with(|s| s.cache_msg.clone());
    let trash = app().with(|s| s.trash);
    let remember = app().with(|s| s.remember_folders);
    let wheel_page = app().with(|s| s.wheel_page);
    page_column(vec![
        header("General", "Everyday behavior and the local scan cache."),
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
        view().child(caption(cache_msg)),
    ])
}

fn preferences_page() -> View {
    let find_index = app().with(|s| s.find_index.unwrap_or(0));
    let mode = crate::ui_helpers::find_mode(Some(find_index));
    let duplicates = mode == FindMode::Duplicates;
    let music = mode == FindMode::Music;
    let exact = app().with(|s| s.exact);

    let mut scan_rows = vec![choice_row(
        "What to find",
        find_mode_caption(Some(find_index)),
        choices(FIND_LABELS, find_index, |index| {
            app().update(|s| s.find_index = Some(index));
            actions::save_prefs();
        }),
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

    let mut page = page_column(vec![
        header("Preferences", "Defaults for the next scan."),
        section(
            "Scan type",
            "Choose what the next scan looks for.",
            scan_rows,
        ),
    ]);

    if duplicates {
        page = page.child(duplicates_section());
        if !exact {
            page = page.child(media_section());
        }
    }
    if music {
        page = page.child(music_section());
    }
    page.child(system_section())
        .child(caption("Changes save automatically to dedupe.json."))
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
                    app().update(|s| {
                        s.similarity_input = value.clone();
                        // The home page pill shows the same similarity, so
                        // both boxes carry the one setting forward together.
                        s.min_sim_input = value;
                    });
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
                choices(KEEP_LABELS, keep_index, actions::apply_keep_mode),
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
