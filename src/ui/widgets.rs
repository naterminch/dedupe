//! Styled primitives shared by every pane.
//!
//! Native Shell ships behaviour without looks; the visual language lives here
//! so panes compose widgets instead of repeating colours, radii and paddings.
//! Colours come from [`crate::ui::palette`] — no raw hex below this module.

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
    button_shell(label, colors::ACCENT, colors::ON_ACCENT, on_press)
}

/// A quieter action next to a primary one.
pub(crate) fn secondary_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .px(px(14.0))
        .py(px(8.0))
        .flex_row()
        .items_center()
        .justify_center()
        .bg(colors::INSET)
        .border_1()
        .border_color(colors::BORDER)
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .hover(|s| s.bg(colors::BORDER))
        .on_press(on_press)
        .child(
            text(label)
                .text_size(px(text::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::TEXT),
        )
}

/// A borderless action for dense rows (Show more, Expand all…).
pub(crate) fn ghost_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .px(px(10.0))
        .py(px(6.0))
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .hover(|s| s.bg(colors::INSET))
        .on_press(on_press)
        .child(
            text(label)
                .text_size(px(text::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::ACCENT),
        )
}

/// A destructive action (Delete N, Undo delete).
pub(crate) fn danger_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .px(px(14.0))
        .py(px(8.0))
        .flex_row()
        .items_center()
        .justify_center()
        .bg(colors::DANGER)
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .on_press(on_press)
        .child(
            text(label)
                .text_size(px(text::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors::ON_ACCENT),
        )
}

/// Shared body for the filled button variants.
fn button_shell(
    label: impl IntoReactive<SharedString>,
    background: AdaptiveColor,
    foreground: AdaptiveColor,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .px(px(14.0))
        .py(px(8.0))
        .flex_row()
        .items_center()
        .justify_center()
        .bg(background)
        .rounded(px(radius::CONTROL))
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .on_press(on_press)
        .child(
            text(label)
                .text_size(px(text::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(foreground),
        )
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
