//! Styled primitives shared by every pane.
//!
//! Native Shell ships behaviour without looks; the visual language lives here
//! so panes compose widgets instead of repeating colours, radii and paddings.
//! Colours come from [`crate::ui::palette`] — no raw hex below this module.

use nsh_base::{button, switch, tooltip};
use nsh_core::element::{BorderCurve, ImageSrc, input, svg};
use nsh_core::prelude::*;

use crate::ui::palette::{colors, radius, space, text};

/// Transition length for colour and transform changes.
///
/// The platform's reduce-motion preference collapses it to zero: the same end
/// state arrives, without the animation. Reads the environment reactively, so
/// turning the setting off is picked up on the next frame.
pub(crate) fn motion(ms: f32) -> f32 {
    if reduce_motion() { 0.0 } else { ms }
}

/// A raised surface for panels, cards and dialogs.
pub(crate) fn card() -> View {
    view()
        .flex_col()
        .bg(colors::GLASS)
        .border_1()
        .border_color(colors::BORDER)
        .rounded(px(radius::CARD))
        .border_curve(BorderCurve::Continuous)
        .shadow_sm()
        .shadow_color(colors::SHADOW)
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
            .px(px(space::LG))
            .py(px(space::SM))
            .bg(colors::ACCENT)
            .rounded_full()
            .shadow_xs()
            .shadow_color(colors::SHADOW)
            .transition_colors()
            .duration(motion(180.0))
            .ease_out()
            .hover(|s| s.opacity(0.92))
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
            .px(px(space::LG))
            .py(px(space::SM))
            .bg(colors::PANE)
            .border_1()
            .border_color(colors::BORDER)
            .rounded_full()
            .transition_colors()
            .duration(motion(180.0))
            .ease_out()
            .hover(|s| s.bg(colors::ACCENT_WASH))
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
            .px(px(space::MD))
            .py(px(space::SM))
            .rounded_full()
            .transition_colors()
            .duration(motion(180.0))
            .ease_out()
            .hover(|s| s.bg(colors::ACCENT_WASH))
            .child(
                text(label)
                    .text_size(px(text::BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::ACCENT),
            ),
    )
}

/// A right-pointing chevron, drawn as an SVG so it stays sharp at any scale.
/// Its `currentColor` stroke follows the element's text color, so it tracks
/// the theme. Turning one glyph beats swapping two: the same markup rotates
/// 90° when the group opens, so the disclosure animates instead of blinking.
const CHEVRON: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" \
fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.5\" stroke-linecap=\"round\" \
stroke-linejoin=\"round\"><path d=\"m9 6 6 6-6 6\"/></svg>";

/// The disclosure chevron of a group header: right when closed, turned down
/// when open. Decorative — the header row itself is the button, so it carries
/// the expanded state for screen readers.
pub(crate) fn chevron(open: bool) -> impl IntoElement {
    svg(ImageSrc::svg_markup(CHEVRON))
        .size(px(16.0))
        .flex_shrink_0()
        .text_color(colors::MUTED)
        .transition_transform()
        .duration(motion(150.0))
        .rotate(if open { deg(90.0) } else { deg(0.0) })
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
            .px(px(space::LG))
            .py(px(space::SM))
            .bg(colors::DANGER)
            .rounded_full()
            .shadow_xs()
            .shadow_color(colors::SHADOW)
            .transition_colors()
            .duration(motion(180.0))
            .ease_out()
            .hover(|s| s.opacity(0.92))
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
                .p(px(space::XXS))
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
        .px(px(space::SM))
        .py(px(space::XXS))
        .flex_row()
        .items_center()
        .bg(background)
        .rounded_full()
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(foreground),
        )
}

/// A one-pixel rule between sections.
pub(crate) fn divider() -> View {
    // One device pixel, not one point: a point-wide rule doubles on a 2x
    // display and reads as a filled bar instead of a hairline.
    view()
        .h(move || px(1.0 / pixel_ratio()))
        .w_full()
        .bg(colors::BORDER)
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
        .px(px(space::MD))
        .py(px(space::SM))
        .text_size(px(text::BODY))
        .text_color(colors::TEXT)
        .bg(colors::INSET)
        .border_1()
        .border_color(colors::BORDER)
        .rounded(px(radius::CONTROL))
        .transition_colors()
        .duration(motion(180.0))
}

/// An info bubble for `.tooltip(..)`: dark in light theme, light in dark
/// theme, so it reads over any surface. The trigger's own label speaks for
/// screen readers; this hint is for the pointer.
pub(crate) fn hint(label: impl IntoReactive<SharedString>) -> impl IntoTooltip {
    tooltip()
        .rounded(px(radius::CONTROL))
        .px(px(space::SM))
        .py(px(space::XS))
        .bg(colors::TEXT)
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .text_color(colors::BACKGROUND),
        )
}
