use crate::ui::{DedupeView, SettingsTab, ThemeReadability};
use crate::ui_helpers;
use crate::ui_helpers::hash_caption;
use crate::{cache, cli, util};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::input::{Input, InputState, NumberInput};
use gpui_kit::component::label::Label;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarMenuItem};
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Side};
use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    linear_color_stop, linear_gradient, px,
};

impl DedupeView {
    /// Settings page: kit Sidebar nav (General | Preferences) + content
    /// pane, mirroring the main page layout. General holds everyday
    /// switches (trash, remembered folders, cache); Preferences holds
    /// matching and scan-engine tuning. Everything auto-saves to
    /// `gui-prefs.json`. (The top bar ← Back button and Escape leave
    /// Settings.)
    pub(crate) fn render_settings(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let nav_general = view.clone();
        h_flex()
            .size_full()
            .overflow_hidden()
            .child(
                Sidebar::new("settings-nav")
                    .side(Side::Left)
                    .collapsible(SidebarCollapsible::None)
                    .w(px(200.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(cx.theme().background, 0.),
                        linear_color_stop(cx.theme().muted.opacity(0.28), 1.),
                    ))
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .child(
                        SidebarMenuItem::new("General")
                            .icon(IconName::Settings2)
                            .active(self.settings_tab == SettingsTab::General)
                            .on_click(move |_, _, cx| {
                                nav_general.update(cx, |this, cx| {
                                    this.settings_tab = SettingsTab::General;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        SidebarMenuItem::new("Preferences")
                            .icon(IconName::Palette)
                            .active(self.settings_tab == SettingsTab::Preferences)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.settings_tab = SettingsTab::Preferences;
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .w_full()
                    .overflow_y_scrollbar()
                    .bg(linear_gradient(
                        145.,
                        linear_color_stop(cx.theme().background, 0.),
                        linear_color_stop(cx.theme().accent_wash(), 1.),
                    ))
                    .child(match self.settings_tab {
                        SettingsTab::General => self.render_settings_general(cx).into_any_element(),
                        SettingsTab::Preferences => {
                            self.render_settings_preferences(cx).into_any_element()
                        }
                    }),
            )
    }
    pub(crate) fn render_settings_general(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (files, bytes) = cache::cache_summary();
        let cache_summary = format!("{} file(s) · {}", files, util::human_bytes(bytes));
        let behavior_rows = vec![
            self.setting_row(
                "tip-trash",
                "Move deleted files to trash",
                "Deleted duplicates go to the system trash so they can be restored.",
                Switch::new("trash")
                    .checked(self.trash)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.trash = *value;
                        this.save_prefs(cx);
                        cx.notify();
                    })),
            )
            .into_any_element(),
            self.setting_row(
                "tip-remember-folders",
                "Remember scan folders",
                "Restore the last folder list the next time the app opens.",
                Switch::new("remember-folders")
                    .checked(self.remember_folders)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.remember_folders = *value;
                        this.save_prefs(cx);
                        cx.notify();
                    })),
            )
            .into_any_element(),
        ];
        let cache_rows = vec![
            self.setting_row(
                "tip-cache-disk",
                "Cache on disk",
                "Hashes and fingerprints stored in ~/.dedupe/ to speed up repeat scans.",
                div()
                    .text_size(px(cx.theme().text_sm_px()))
                    .text_color(cx.theme().muted_foreground)
                    .child(cache_summary),
            )
            .into_any_element(),
            self.setting_row(
                "tip-clear-cache",
                "Clear cached hashes and fingerprints",
                "Force the next scan to re-read and re-fingerprint every file.",
                Button::new("clear-cache")
                    .icon(IconName::RefreshCw)
                    .label("Clear caches")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let (files, bytes) = cache::clear_caches();
                        this.cache_msg =
                            format!("Cleared {} file(s) ({})", files, util::human_bytes(bytes),);
                        this.pending_toast =
                            Some((format!("Cache cleared: {}", this.cache_msg), false));
                        cx.notify();
                    })),
            )
            .into_any_element(),
        ];

        v_flex()
            .w_full()
            .max_w(px(780.))
            .mx_auto()
            .p_8()
            .gap_5()
            .child(self.settings_header(
                cx,
                "General",
                "Everyday behavior and the local scan cache.",
            ))
            .child(self.settings_card(
                "settings-general",
                "tip-general",
                "Behavior",
                "App-wide behavior for scans and deletions.",
                behavior_rows,
            ))
            .child(self.settings_card(
                "settings-cache",
                "tip-cache",
                "Cache",
                "Hashes and fingerprints cached between runs.",
                cache_rows,
            ))
            .child(
                div()
                    .text_size(px(cx.theme().text_xs_px()))
                    .text_color(cx.theme().muted_foreground)
                    .child(self.cache_msg.clone()),
            )
    }
    pub(crate) fn render_settings_preferences(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mode = ui_helpers::find_mode(self.find_index);
        let duplicates = mode == cli::FindMode::Duplicates;
        let music = mode == cli::FindMode::Music;

        let mut scan_rows: Vec<AnyElement> = vec![
            v_flex()
                .gap_2()
                .child(self.pref_title(
                    "tip-find",
                    "What to find",
                    ui_helpers::find_mode_caption(self.find_index),
                ))
                .child(
                    RadioGroup::horizontal("find-mode")
                        .children([
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
                        ])
                        .selected_index(self.find_index)
                        .on_change(cx.listener(|this, value, _, cx| {
                            this.find_index = Some(*value);
                            this.save_prefs(cx);
                            cx.notify();
                        })),
                )
                .into_any_element(),
        ];
        if mode == cli::FindMode::BigFiles {
            scan_rows.push(
                self.setting_row(
                    "tip-top",
                    "Largest files to report",
                    "How many entries the big-files scan lists, biggest first.",
                    NumberInput::new(&self.top_input),
                )
                .into_any_element(),
            );
        }
        let scan_type = self.settings_card(
            "settings-scan-type",
            "tip-scan-type",
            "Scan type",
            "Choose what the next scan looks for.",
            scan_rows,
        );

        let mut duplicate_rows = vec![
            self.setting_row(
                "tip-matching",
                "Match duplicates by",
                "Content hashes bytes. Size groups same-size files. Name matches base names. Fuzzy tolerates small typos.",
                RadioGroup::horizontal("match-mode")
                    .children(["Content", "Size", "Name", "Fuzzy name"])
                    .selected_index(self.match_index)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.match_index = Some(*value);
                        this.save_prefs(cx);
                        cx.notify();
                    })),
            )
            .into_any_element(),
        ];
        if ui_helpers::match_mode(self.match_index) == cli::MatchMode::Fuzzy {
            duplicate_rows.push(
                self.setting_row(
                    "tip-fuzzy",
                    "Fuzzy threshold %",
                    "Minimum base-name similarity. 85% is a good default.",
                    NumberInput::new(&self.fuzzy_threshold_input)
                        .suffix(div().text_size(px(cx.theme().text_xs_px())).child("%")),
                )
                .into_any_element(),
            );
        }
        duplicate_rows.push(
            self.setting_row(
                "tip-hash",
                "Hash algorithm",
                format!(
                    "{} Choose another algorithm only to match hashes from a different tool.",
                    hash_caption(self.hash_index)
                ),
                RadioGroup::horizontal("hash")
                    .children(["blake3", "sha256", "md5"])
                    .selected_index(self.hash_index)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.hash_index = Some(*value);
                        this.save_prefs(cx);
                        cx.notify();
                    })),
            )
            .into_any_element(),
        );
        duplicate_rows.push(
            self.setting_row(
                "tip-exact",
                "Exact duplicates only",
                "Skip the similar-media pass; only byte-identical files are grouped.",
                Switch::new("exact")
                    .checked(self.exact)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.exact = *value;
                        this.save_prefs(cx);
                        cx.notify();
                    })),
            )
            .into_any_element(),
        );
        duplicate_rows.push(
            self.setting_row(
                "tip-no-cache",
                "Ignore cache and re-hash everything",
                "Ignore cached hashes and fingerprints for this run.",
                Switch::new("no-cache")
                    .checked(self.no_cache)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.no_cache = *value;
                        this.save_prefs(cx);
                        cx.notify();
                    })),
            )
            .into_any_element(),
        );
        let duplicates_group = self.settings_card(
            "settings-duplicates",
            "tip-duplicates",
            "Duplicates",
            "Only shown when scanning for duplicates.",
            duplicate_rows,
        );

        let mut media_rows = vec![
            self.setting_row(
                "tip-similarity",
                "Similarity %",
                "How close two images or videos must be to group. 97% catches re-encodes.",
                NumberInput::new(&self.similarity_input)
                    .suffix(div().text_size(px(cx.theme().text_xs_px())).child("%")),
            )
            .into_any_element(),
            self.setting_row(
                "tip-image-hash",
                "Image fingerprint",
                "dHash is fast and default. pHash tolerates crops and colour shifts. aHash ignores colour. Color grid compares average colours.",
                RadioGroup::horizontal("image-hash")
                    .children(["Auto (dHash)", "dHash", "pHash", "aHash", "Color grid"])
                    .selected_index(self.image_hash_index)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.image_hash_index = Some(*value);
                        this.save_prefs(cx);
                        cx.notify();
                    })),
            )
            .into_any_element(),
            self.setting_row(
                "tip-partial-clips",
                "Find shorter clips inside longer videos",
                "Also look for a shorter video contained in a longer one.",
                Switch::new("partial-clips")
                    .checked(self.partial_clips)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.partial_clips = *value;
                        this.save_prefs(cx);
                        cx.notify();
                    })),
            )
            .into_any_element(),
        ];
        if self.partial_clips {
            media_rows.push(
                self.setting_row(
                    "tip-partial-similarity",
                    "Partial-clip similarity %",
                    "How much of the shorter clip must match inside the longer one.",
                    NumberInput::new(&self.partial_similarity_input)
                        .suffix(div().text_size(px(cx.theme().text_xs_px())).child("%")),
                )
                .into_any_element(),
            );
        }
        let media_group = self.settings_card(
            "settings-media",
            "tip-media",
            "Images & video",
            "Similar-media detection runs alongside exact duplicate matching.",
            media_rows,
        );

        let music_group = self.settings_card(
            "settings-music",
            "tip-music",
            "Music",
            "How duplicate tracks are matched when scanning for music.",
            vec![
                self.setting_row(
                    "tip-music-mode",
                    "Match tracks by",
                    "Embedded tags compare artist, title and album. Audio fingerprint compares the audio itself.",
                    RadioGroup::horizontal("music-mode")
                        .children(["Embedded tags", "Audio fingerprint"])
                        .selected_index(Some(usize::from(self.music_by_content)))
                        .on_change(cx.listener(|this, value, _, cx| {
                            this.music_by_content = *value == 1;
                            this.save_prefs(cx);
                            cx.notify();
                        })),
                )
                .into_any_element(),
            ],
        );

        let system_group = self.settings_card(
            "settings-system",
            "tip-system",
            "System",
            "Performance and the defaults applied to new scans.",
            vec![
                self.setting_row(
                    "tip-jobs",
                    "Worker threads",
                    "Parallel hash workers. 0 uses every core but one.",
                    NumberInput::new(&self.jobs_input),
                )
                .into_any_element(),
                self.setting_row(
                    "tip-keep",
                    "Default keep",
                    "Which copy a new scan keeps when duplicates are found.",
                    RadioGroup::horizontal("default-keep")
                        .children(["First", "Smallest", "Newest", "Oldest", "Best quality"])
                        .selected_index(self.keep_index)
                        .on_change(cx.listener(|this, value, _, cx| {
                            this.apply_keep_mode(*value, cx);
                        })),
                )
                .into_any_element(),
                self.setting_row(
                    "tip-sort-biggest",
                    "Sort by most reclaimable space",
                    "List groups with the most reclaimable bytes first.",
                    Switch::new("default-sort-biggest")
                        .checked(self.sort_biggest)
                        .on_change(cx.listener(|this, value, _, cx| {
                            this.sort_biggest = *value;
                            this.save_prefs(cx);
                            cx.notify();
                        })),
                )
                .into_any_element(),
            ],
        );

        let mut page = v_flex()
            .w_full()
            .max_w(px(780.))
            .mx_auto()
            .p_8()
            .gap_5()
            .child(self.settings_header(
                cx,
                "Preferences",
                "Defaults for the next scan. Hover an option name for details.",
            ))
            .child(scan_type);
        if duplicates {
            page = page.child(duplicates_group);
            if !self.exact {
                page = page.child(media_group);
            }
        }
        if music {
            page = page.child(music_group);
        }
        page.child(system_group).child(
            div()
                .text_size(px(cx.theme().text_xs_px()))
                .text_color(cx.theme().muted_foreground)
                .child("Changes save automatically."),
        )
    }
    /// Size filter with a log-scale slider synced to an exact-value box.
    /// Drag for ballpark, type for exact; empty box means "no limit".
    pub(crate) fn render_size_row(
        &self,
        label: &'static str,
        slider: &Entity<SliderState>,
        input: &Entity<InputState>,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .gap_1p5()
            .child(self.sidebar_section(label, None))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(Slider::new(slider).horizontal()))
                    .child(div().w(px(96.)).child(Input::new(input))),
            )
    }
    pub(crate) fn pref_title(
        &self,
        id: &'static str,
        title: impl Into<SharedString>,
        hint: impl Into<SharedString>,
    ) -> impl IntoElement {
        let hint: SharedString = hint.into();
        div()
            .id(id)
            .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
            .child(Label::new(title))
    }
    pub(crate) fn settings_header(
        &self,
        cx: &mut Context<Self>,
        title: &'static str,
        description: &'static str,
    ) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_size(px(cx.theme().text_xs_px()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().accent)
                    .child("Settings"),
            )
            .child(
                div()
                    .text_size(px(cx.theme().text_heading_px()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(cx.theme().text_sm_px()))
                    .text_color(cx.theme().muted_foreground)
                    .child(description),
            )
            .child(
                div()
                    .h(px(2.))
                    .w(px(56.))
                    .rounded_full()
                    .bg(cx.theme().accent),
            )
    }
    pub(crate) fn setting_row(
        &self,
        id: &'static str,
        label: &'static str,
        hint: impl Into<SharedString>,
        control: impl IntoElement,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_6()
            .child(div().flex_1().child(self.pref_title(id, label, hint)))
            .child(div().flex_shrink_0().max_w(px(480.)).child(control))
    }
    pub(crate) fn settings_rows(&self, rows: Vec<AnyElement>) -> impl IntoElement {
        let mut body = v_flex().w_full();
        for (ix, row) in rows.into_iter().enumerate() {
            if ix > 0 {
                body = body.child(Separator::horizontal().my_1());
            }
            body = body.child(row);
        }
        body
    }
    pub(crate) fn settings_card(
        &self,
        id: &'static str,
        title_id: &'static str,
        title: &'static str,
        hint: impl Into<SharedString>,
        rows: Vec<AnyElement>,
    ) -> impl IntoElement {
        GroupBox::new()
            .id(id)
            .fill()
            .title(self.pref_title(title_id, title, hint))
            .child(self.settings_rows(rows))
    }
}
