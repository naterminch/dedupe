use crate::cli;
use crate::ui::{DedupeView, ThemeReadability};
use crate::ui_helpers;
use crate::ui_helpers::fit_text;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, NumberInput};
use gpui_kit::component::label::Label;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::{
    Context, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _, div,
    linear_color_stop, linear_gradient, px,
};

impl DedupeView {
    /// Included-folders list with per-row Ref toggles (Krokiet pattern).
    pub(crate) fn render_folder_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex().gap_2();
        if self.folders.is_empty() {
            list = list.child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded(px(12.))
                    .border_1()
                    .border_dashed()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .size(px(26.))
                                    .rounded_full()
                                    .bg(cx.theme().muted)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(IconName::Folder),
                            )
                            .child(
                                div()
                                    .text_size(px(cx.theme().text_sm_px()))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("No folders yet"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(cx.theme().text_xs_px()))
                            .text_color(cx.theme().muted_foreground)
                            .child("Add a folder below or browse to begin."),
                    ),
            );
        }
        for (i, folder) in self.folders.iter().enumerate() {
            let reference = folder.reference;
            list = list.child(
                ListItem::new(SharedString::from(format!("folder-{i}")))
                    .px_2()
                    .py_2()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(if reference {
                        cx.theme().accent.opacity(0.35)
                    } else {
                        cx.theme().border
                    })
                    .bg(if reference {
                        cx.theme().accent_wash()
                    } else {
                        cx.theme().background
                    })
                    .selected(reference)
                    .accessibility_label(fit_text(&folder.path, 120))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .size(px(28.))
                                            .rounded(px(9.))
                                            .bg(if reference {
                                                cx.theme().accent.opacity(0.14)
                                            } else {
                                                cx.theme().muted
                                            })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_color(if reference {
                                                cx.theme().accent
                                            } else {
                                                cx.theme().muted_foreground
                                            })
                                            .child(IconName::Folder),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .text_size(px(cx.theme().text_sm_px()))
                                            .truncate()
                                            .child(fit_text(&folder.path, 120)),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        Checkbox::new(SharedString::from(format!("ref-{i}")))
                                            .label("Protect")
                                            .checked(reference)
                                            .on_change(cx.listener(move |this, value, _, cx| {
                                                if let Some(f) = this.folders.get_mut(i) {
                                                    f.reference = *value;
                                                    this.save_prefs(cx);
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!("rm-folder-{i}")))
                                            .icon(IconName::Delete)
                                            .tooltip("Remove folder")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if i < this.folders.len() {
                                                    this.folders.remove(i);
                                                    this.save_prefs(cx);
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            ),
                    ),
            );
        }
        list
    }
    pub(crate) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Side nav: scan folders + per-run scope. Global tuning lives on the
        // Settings page — the main flow stays a 3-step scan.
        v_flex()
            .w_full()
            .h_full()
            .bg(linear_gradient(
                180.,
                linear_color_stop(cx.theme().background, 0.),
                linear_color_stop(cx.theme().muted.opacity(0.28), 1.),
            ))
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .gap_3()
                    .px_4()
                    .pt_5()
                    .pb_4()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_start()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(cx.theme().text_xs_px()))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(cx.theme().accent)
                                            .child("Source folders"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(cx.theme().text_sm_px()))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Where to look"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(cx.theme().text_xs_px()))
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Protected folders are never deleted."),
                                    ),
                            )
                            .child(Tag::secondary().child(self.folders.len().to_string())),
                    )
                    .child(self.render_folder_list(cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .child(Input::new(&self.folder_input).cleanable(true)),
                            )
                            .child(
                                Button::new("add-folder")
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label("Add")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_from_input(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("browse")
                                    .outline()
                                    .icon(IconName::FolderOpen)
                                    .label("Browse")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.browse(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div().flex_1().w_full().overflow_y_scrollbar().child(
                    v_flex().gap_3().px_4().pb_4().child(
                        v_flex()
                            .gap_3()
                            .p_3()
                            .rounded(px(14.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().card_surface())
                            .shadow_sm()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_start()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .text_size(px(cx.theme().text_xs_px()))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(cx.theme().accent)
                                                    .child("Scan scope"),
                                            )
                                            .child(Label::new("Narrow the search")),
                                    )
                                    .child(Tag::secondary().child("Optional")),
                            )
                            .child(self.sidebar_field(
                                "File types",
                                None,
                                Input::new(&self.types_input),
                                cx,
                            ))
                            .child(self.render_size_row(
                                "Min size",
                                &self.min_size_slider,
                                &self.min_size_input,
                                cx,
                            ))
                            .child(self.render_size_row(
                                "Max size",
                                &self.max_size_slider,
                                &self.max_size_input,
                                cx,
                            ))
                            .child(self.sidebar_field(
                                "Max depth",
                                Some("empty = unlimited"),
                                NumberInput::new(&self.max_depth_input),
                                cx,
                            ))
                            .child(self.sidebar_field(
                                "Skip directories",
                                Some("; separated"),
                                Input::new(&self.exclude_dir_input),
                                cx,
                            ))
                            .child(self.sidebar_field(
                                "Skip paths containing",
                                Some("; separated"),
                                Input::new(&self.exclude_path_input),
                                cx,
                            ))
                            .child(self.sidebar_field(
                                "Ignore globs",
                                Some("; separated"),
                                Input::new(&self.ignore_input),
                                cx,
                            )),
                    ),
                ),
            )
            .child(
                div().px_4().pt_2().pb_4().child(
                    v_flex()
                        .gap_3()
                        .p_3()
                        .rounded(px(16.))
                        .border_1()
                        .border_color(cx.theme().accent.opacity(0.25))
                        .bg(linear_gradient(
                            145.,
                            linear_color_stop(cx.theme().accent_wash_strong(), 0.),
                            linear_color_stop(cx.theme().background, 1.),
                        ))
                        .shadow_md()
                        .child(
                            h_flex()
                                .justify_between()
                                .items_start()
                                .gap_3()
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(cx.theme().text_xs_px()))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(cx.theme().accent)
                                                .child(if self.scanning {
                                                    "Scanning"
                                                } else {
                                                    "Ready to scan"
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(cx.theme().text_sm_px()))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(if self.scanning {
                                                    "Working through your files"
                                                } else {
                                                    "Your files stay on this PC."
                                                }),
                                        ),
                                )
                                .child(
                                    Tag::secondary()
                                        .child(format!("{} folders", self.folders.len())),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(cx.theme().text_xs_px()))
                                .text_color(cx.theme().muted_foreground)
                                .child(if self.scanning {
                                    "Stop at any time; the view resets instantly."
                                } else {
                                    "No uploads."
                                }),
                        )
                        .child(self.render_media_status(cx))
                        .child(if self.scanning {
                            // Stop replaces Scan for the duration of the run:
                            // the pipeline notices at its next phase
                            // boundary (bounded by one small unit of work).
                            Button::new("stop-scan")
                                .danger()
                                .icon(IconName::Square)
                                .w_full()
                                .label("Stop")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.stop_scan(cx);
                                }))
                                .into_any_element()
                        } else {
                            Button::new("scan")
                                .primary()
                                .icon(IconName::Search)
                                .w_full()
                                .label(
                                    if ui_helpers::find_mode(self.find_index)
                                        == cli::FindMode::Duplicates
                                    {
                                        "Scan for duplicates"
                                    } else {
                                        "Run cleanup scan"
                                    },
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_scan(cx);
                                }))
                                .into_any_element()
                        }),
                ),
            )
    }
    /// ffmpeg/ffprobe availability line shown above the Scan button.
    pub(crate) fn render_media_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.media_checked {
            return Tag::secondary()
                .child("Checking for ffmpeg…")
                .into_any_element();
        }
        let theme = cx.theme();
        let (tag, text) = match (self.ffmpeg_ok, self.ffprobe_ok) {
            (true, true) => (
                Tag::custom(
                    theme.accent_wash_strong(),
                    theme.accent,
                    theme.accent_wash_strong(),
                )
                .rounded_full(),
                "ffmpeg ready",
            ),
            (false, true) => (Tag::warning(), "ffmpeg missing — video compare off"),
            (true, false) => (Tag::warning(), "ffprobe missing — media details off"),
            (false, false) => (
                Tag::warning(),
                "ffmpeg missing — video compare + details off",
            ),
        };
        tag.self_start().child(text).into_any_element()
    }
    /// Field/section label with an optional dimmed hint. The kit's `Label`
    /// owns the typography: sentence-case label, hint after it in muted
    /// color ("Max depth" + "empty = unlimited").
    pub(crate) fn sidebar_section(
        &self,
        label: impl Into<SharedString>,
        secondary: Option<&'static str>,
    ) -> impl IntoElement {
        let label = Label::new(label);
        match secondary {
            Some(hint) => label.secondary(hint),
            None => label,
        }
    }
    pub(crate) fn sidebar_field(
        &self,
        label: &'static str,
        secondary: Option<&'static str>,
        input: impl IntoElement,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .gap_1p5()
            .child(self.sidebar_section(label, secondary))
            .child(input)
    }
}
