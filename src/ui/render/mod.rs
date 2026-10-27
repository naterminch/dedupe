use crate::ui::theme::apply_readable_theme;
use crate::ui::{AppView, DedupeView, ThemeReadability};

use crate::util;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::component::{IconName, Theme, ThemeMode};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, Styled as _, Window, div, img, linear_color_stop, linear_gradient, px,
};

mod controls;
mod modes;
mod results;
mod settings;
mod sidebar;

impl Render for DedupeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().is_dark();
        if let Some((message, is_error)) = self.pending_toast.take() {
            window.defer(cx, move |window, cx| {
                let note = if is_error {
                    Notification::error(message).autohide(true)
                } else {
                    Notification::success(message).autohide(true)
                };
                window.push_notification(note, cx);
            });
        }
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            // Checklist C29: Escape always does something — backs out of
            // Settings to the main flow.
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    this.leave_settings(cx);
                }
            }))
            .child(
                v_flex()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .px_6()
                            .py_3()
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(cx.theme().background, 0.),
                                linear_color_stop(cx.theme().muted.opacity(0.35), 1.),
                            ))
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        img("icons/logo.png")
                                            .w(px(34.))
                                            .h(px(34.))
                                            .rounded(px(10.)),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_0p5()
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .text_size(px(
                                                                cx.theme().text_sm_px() + 1.
                                                            ))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .child("dedupe"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(cx.theme().text_xs_px()))
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(concat!(
                                                                "v",
                                                                env!("CARGO_PKG_VERSION")
                                                            )),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(cx.theme().text_xs_px()))
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(if self.view == AppView::Settings {
                                                        "Tune the scanner, not your files."
                                                    } else {
                                                        "Find duplicates. Keep the one you want."
                                                    }),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(if self.view == AppView::Settings {
                                        Button::new("nav-back")
                                            .icon(IconName::ArrowLeft)
                                            .label("Back")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.view = AppView::Main;
                                                cx.notify();
                                            }))
                                    } else {
                                        Button::new("nav-settings")
                                            .icon(IconName::Settings)
                                            .label("Settings")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.view = AppView::Settings;
                                                cx.notify();
                                            }))
                                    })
                                    .child(
                                        Button::new("theme-toggle")
                                            .icon(if dark { IconName::Sun } else { IconName::Moon })
                                            .label(if dark { "Light" } else { "Dark" })
                                            .w(px(88.))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                Theme::change(
                                                    if dark {
                                                        ThemeMode::Light
                                                    } else {
                                                        ThemeMode::Dark
                                                    },
                                                    None,
                                                    cx,
                                                );
                                                this.save_prefs(cx);
                                                apply_readable_theme(cx);
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .h(px(1.))
                            .w_full()
                            .child(div().flex_1().bg(cx.theme().border))
                            .child(
                                div()
                                    .w(px(180.))
                                    .h(px(1.))
                                    .bg(cx.theme().accent.opacity(0.65)),
                            ),
                    ),
            )
            .child(if self.view == AppView::Settings {
                div()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_settings(cx))
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        h_resizable("main-split")
                            .child(
                                resizable_panel()
                                    .size(px(320.))
                                    .size_range(px(240.)..px(560.))
                                    .fixed()
                                    .child(self.render_sidebar(cx)),
                            )
                            .child(resizable_panel().child(self.render_results(cx))),
                    )
                    .into_any_element()
            })
            .child(
                StatusBar::new()
                    .left({
                        let mut left = h_flex().gap_2().items_center();
                        if self.scanning {
                            left = left.child(Spinner::new().color(cx.theme().accent));
                        }
                        left.child(
                            div()
                                .text_size(px(cx.theme().text_xs_px()))
                                .truncate()
                                .text_color(cx.theme().muted_foreground)
                                .child(self.status.clone()),
                        )
                    })
                    .right(
                        div()
                            .text_size(px(cx.theme().text_xs_px()))
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} file(s) scanned · {}",
                                self.files_scanned,
                                util::human_bytes(self.bytes_scanned)
                            )),
                    ),
            )
    }
}
