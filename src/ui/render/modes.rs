use crate::ui::{DedupeView, GROUP_PAGE, ThemeReadability};
use crate::ui_helpers;
use crate::{cli, util};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::{
    Context, FontWeight, IntoElement, ParentElement as _, Styled as _, div, linear_color_stop,
    linear_gradient, px,
};

impl DedupeView {
    pub(crate) fn render_mode_results(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(mode) = self.mode_result.as_ref() else {
            return div().into_any_element();
        };
        let title = crate::modes::mode_title(mode.mode);
        let findings = mode.finding_count();
        let groups = mode.groups.len();
        let group_members: usize = mode.groups.iter().map(|g| g.members.len()).sum();
        let results = findings + group_members;
        let summary = if groups > 0 {
            format!("{groups} group(s) · {results} item(s)")
        } else {
            format!("{findings} finding(s)")
        };
        let note = if mode.mode == cli::FindMode::DuplicateFolders {
            "Duplicate-folder scans are report-only."
        } else {
            "Cleanup scan results are read-only here. Run the CLI with --delete or --trash to act on them."
        };
        let reclaim = mode.reclaimable();
        let limit = self.group_limit;
        let border = cx.theme().border;
        let background = cx.theme().card_surface();
        let muted = cx.theme().muted_foreground;
        let xs = px(cx.theme().text_xs_px());
        let value_size = px(cx.theme().text_sm_px() + 3.);
        let summary_card =
            |icon: IconName, value: String, label: &'static str, color: gpui_kit::Hsla| {
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_3()
                    .items_center()
                    .p_3()
                    .border_1()
                    .rounded(px(12.))
                    .border_color(border)
                    .bg(background)
                    .shadow_sm()
                    .child(
                        div()
                            .size(px(34.))
                            .rounded(px(10.))
                            .bg(color.opacity(0.12))
                            .text_color(color)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_0()
                            .child(
                                div()
                                    .text_size(value_size)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(color)
                                    .child(value),
                            )
                            .child(div().text_size(xs).text_color(muted).child(label)),
                    )
            };
        let header = v_flex().gap_3().child(
            h_flex()
                .justify_between()
                .items_end()
                .gap_4()
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(cx.theme().text_xs_px()))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(cx.theme().accent)
                                .child("Cleanup results"),
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
                                .text_color(muted)
                                .child(note),
                        ),
                )
                .child(Tag::secondary().child(summary)),
        );
        let mut list = v_flex().gap_2();
        if self.scanning {
            list = list.child(self.render_progress(cx));
        }
        if !self.has_scanned && !self.scanning {
            list = list.child(self.render_empty_state(cx));
        }
        if self.has_scanned {
            list = list.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(summary_card(
                        IconName::Search,
                        self.files_scanned.to_string(),
                        "Files scanned",
                        cx.theme().foreground,
                    ))
                    .child(summary_card(
                        IconName::Folder,
                        if groups > 0 {
                            results.to_string()
                        } else {
                            findings.to_string()
                        },
                        if groups > 0 { "Items" } else { "Findings" },
                        cx.theme().accent,
                    ))
                    .child(summary_card(
                        IconName::Delete,
                        util::human_bytes(reclaim),
                        "Reclaimable",
                        cx.theme().success,
                    )),
            );
        }
        for note in &mode.notes {
            list = list.child(
                div()
                    .text_size(px(cx.theme().text_xs_px()))
                    .text_color(cx.theme().muted_foreground)
                    .child(note.clone()),
            );
        }
        if !mode.errors.is_empty() {
            list = list.child(Alert::warning(
                "mode-errors",
                format!("{} path(s) could not be read.", mode.errors.len()),
            ));
        }
        if mode.is_empty() {
            list = list.child(Alert::success(
                "mode-empty",
                format!("Nothing found in {} file(s).", mode.files_scanned),
            ));
        }
        for finding in mode.findings.iter().take(limit) {
            list = list.child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .border_1()
                    .rounded(px(12.))
                    .border_color(border)
                    .bg(background)
                    .shadow_sm()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Tag::secondary().child(finding.kind))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(cx.theme().text_sm_px()))
                                    .truncate()
                                    .child(ui_helpers::fit_text(
                                        &finding.path.display().to_string(),
                                        110,
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(cx.theme().text_xs_px()))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if finding.is_dir {
                                        "folder".to_string()
                                    } else {
                                        util::human_bytes(finding.size)
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(cx.theme().text_xs_px()))
                            .text_color(cx.theme().muted_foreground)
                            .child(finding.detail.clone()),
                    ),
            );
        }
        for group in mode.groups.iter().take(limit) {
            let mut card = v_flex()
                .gap_1()
                .p_3()
                .border_1()
                .rounded(px(12.))
                .border_color(border)
                .bg(background)
                .shadow_sm()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(cx.theme().text_sm_px()))
                                .font_weight(FontWeight::BOLD)
                                .child(group.label.clone()),
                        )
                        .child(Tag::secondary().child(format!("{} copies", group.members.len()))),
                )
                .child(
                    div()
                        .text_size(px(cx.theme().text_xs_px()))
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} · {} reclaimable",
                            group.detail,
                            util::human_bytes(group.reclaimable)
                        )),
                );
            for (index, member) in group.members.iter().enumerate().take(12) {
                let size = group.sizes.get(index).copied().unwrap_or(0);
                card = card.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(cx.theme().text_xs_px()))
                                .truncate()
                                .child(ui_helpers::fit_text(&member.display().to_string(), 110)),
                        )
                        .child(
                            div()
                                .text_size(px(cx.theme().text_xs_px()))
                                .text_color(cx.theme().muted_foreground)
                                .child(util::human_bytes(size)),
                        ),
                );
            }
            if group.members.len() > 12 {
                card = card.child(
                    div()
                        .text_size(px(cx.theme().text_xs_px()))
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("+{} more", group.members.len() - 12)),
                );
            }
            list = list.child(card);
        }
        let shown = findings.min(limit) + groups.min(limit);
        let total_items = findings + groups;
        if total_items > shown {
            let remaining = total_items - shown;
            list = list.child(
                Button::new("show-more-mode")
                    .icon(IconName::ChevronDown)
                    .label(format!("Show more ({remaining} remaining)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.group_limit += GROUP_PAGE;
                        cx.notify();
                    })),
            );
        }
        v_flex()
            .flex_1()
            .h_full()
            .gap_4()
            .p_6()
            .bg(linear_gradient(
                145.,
                linear_color_stop(cx.theme().background, 0.),
                linear_color_stop(cx.theme().accent_wash(), 1.),
            ))
            .child(header)
            .child(div().flex_1().w_full().overflow_y_scrollbar().child(list))
            .into_any_element()
    }
}
