//! Styled primitives shared by every pane.
//!
//! Native Shell ships behaviour without looks; the visual language lives here
//! so panes compose widgets instead of repeating colours, radii and paddings.
//! Colours come from [`crate::ui::palette`] — no raw hex below this module.

use nsh_base::{button, switch, tooltip};
use nsh_core::element::input;
use nsh_core::prelude::*;

use crate::ui::palette::{colors, radius, text};

/// A raised surface for panels, cards and dialogs.
pub(crate) fn card() -> View {
    view()
        .flex_col()
        .bg(colors::CARD)
        .border_1()
        .border_color(colors::BORDER)
        .rounded(px(radius::CARD))
}

/// Page-level heading ("dedupe", "Settings").
pub(crate) fn heading(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT)
}

/// Section heading inside a card.
pub(crate) fn panel_title(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::BODY))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT)
}

/// Secondary copy: hints, counts, filenames under a card title.
pub(crate) fn caption(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::CAPTION))
        .text_color(colors::MUTED)
}

/// Uppercase-ish label above a control.
pub(crate) fn field_label(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::CAPTION))
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors::MUTED)
}

/// The main call to action: filled with the accent colour.
pub(crate) fn primary_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    view().flex_col().child(
        button()
            .on_press(on_press)
            .flex_row()
            .items_center()
            .justify_center()
            .px(px(14.0))
            .py(px(8.0))
            .bg(colors::ACCENT)
            .rounded(px(radius::CONTROL))
            .hover(|s| s.opacity(0.9))
            .child(
                text(label)
                    .text_size(px(text::BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::ON_ACCENT),
            ),
    )
}

/// A quieter action next to a primary one.
pub(crate) fn secondary_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    view().flex_col().child(
        button()
            .on_press(on_press)
            .flex_row()
            .items_center()
            .justify_center()
            .px(px(14.0))
            .py(px(8.0))
            .bg(colors::INSET)
            .border_1()
            .border_color(colors::BORDER)
            .rounded(px(radius::CONTROL))
            .hover(|s| s.bg(colors::BORDER))
            .child(
                text(label)
                    .text_size(px(text::BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::TEXT),
            ),
    )
}

/// A borderless action for dense rows (Show more, Expand all…).
pub(crate) fn ghost_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    view().flex_col().child(
        button()
            .on_press(on_press)
            .flex_row()
            .items_center()
            .justify_center()
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(radius::CONTROL))
            .hover(|s| s.bg(colors::INSET))
            .child(
                text(label)
                    .text_size(px(text::BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::ACCENT),
            ),
    )
}

/// A destructive action (Delete N, Undo delete).
pub(crate) fn danger_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    view().flex_col().child(
        button()
            .on_press(on_press)
            .flex_row()
            .items_center()
            .justify_center()
            .px(px(14.0))
            .py(px(8.0))
            .bg(colors::DANGER)
            .rounded(px(radius::CONTROL))
            .hover(|s| s.opacity(0.9))
            .child(
                text(label)
                    .text_size(px(text::BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::ON_ACCENT),
            ),
    )
}

/// An on/off control on the platform switch semantics (Space and Return,
/// screen readers hear "checked"), drawn as a small track with a thumb.
pub(crate) fn toggle_switch(
    checked: bool,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    switch()
        .checked(checked)
        .on_change(move |_: &bool| on_press())
        .accessibility_label(label)
        .track_style(|s| {
            s.w(px(32.0))
                .h(px(20.0))
                .flex_row()
                .items_center()
                .p(px(2.0))
                .rounded_full()
                .bg(if checked {
                    colors::ACCENT
                } else {
                    colors::INSET
                })
                .border_1()
                .border_color(if checked {
                    colors::ACCENT
                } else {
                    colors::BORDER
                })
        })
        .thumb_style(|s| {
            s.w(px(16.0))
                .h(px(16.0))
                .rounded_full()
                .bg(colors::BACKGROUND)
        })
        .thumb_class("transition-transform checked:translate-x-3")
}

/// A small rounded tag (KEEP, REF, DUP, a file extension…).
pub(crate) fn chip(
    label: impl IntoReactive<SharedString>,
    background: AdaptiveColor,
    foreground: AdaptiveColor,
) -> View {
    view()
        .px(px(8.0))
        .py(px(2.0))
        .flex_row()
        .items_center()
        .bg(background)
        .rounded(px(radius::CONTROL))
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(foreground),
        )
}

/// A one-pixel rule between sections.
pub(crate) fn divider() -> View {
    view().h(px(1.0)).w_full().bg(colors::BORDER)
}

/// A labelled single-line text field bound to the shared state.
///
/// `value` and `on_change` are closures so callers can read and write
/// whichever [`AppState`](crate::ui::state::AppState) field they own without
/// passing a signal around.
pub(crate) fn field(
    placeholder: &'static str,
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(String) + 'static,
) -> Input {
    input()
        .value(value)
        .on_change(move |v: &str| on_change(v.to_owned()))
        .placeholder(placeholder)
        .placeholder_color(|| color(colors::MUTED))
        .caret_color(|| color(colors::ACCENT))
        .selection_color(|| color(colors::ACCENT))
        .flex_1()
        .px(px(10.0))
        .py(px(7.0))
        .text_size(px(text::BODY))
        .text_color(colors::TEXT)
        .bg(colors::INSET)
        .border_1()
        .border_color(colors::BORDER)
        .rounded(px(radius::CONTROL))
}

/// An info bubble for `.tooltip(..)`: dark in light theme, light in dark
/// theme, so it reads over any surface. The trigger's own label speaks for
/// screen readers; this hint is for the pointer.
pub(crate) fn hint(label: impl IntoReactive<SharedString>) -> impl IntoTooltip {
    tooltip()
        .rounded(px(radius::CONTROL))
        .px(px(10.0))
        .py(px(5.0))
        .bg(colors::TEXT)
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .text_color(colors::BACKGROUND),
        )
}
