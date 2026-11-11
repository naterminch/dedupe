//! Styled primitives shared by every pane.
//!
//! Native Shell ships behaviour without looks; the visual language lives here
//! so panes compose widgets instead of repeating colours, radii and paddings.
//! Colours come from [`crate::ui::palette`] — no raw hex below this module.

use nsh_base::{button, radio, switch, tooltip};
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

/// Icons, as inline SVG stroked with `currentColor`: one markup works in both
/// appearances, at any size, and follows whatever tint the control gives it.
///
/// They are decorative. Every control that shows one owns the accessible name
/// (and the tooltip), which is what the tests press by (`label:...`).
pub(crate) mod icons {
    pub(crate) const GRID: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"3\" y=\"3\" width=\"7.5\" height=\"7.5\" rx=\"1.5\"/><rect x=\"13.5\" y=\"3\" width=\"7.5\" height=\"7.5\" rx=\"1.5\"/><rect x=\"3\" y=\"13.5\" width=\"7.5\" height=\"7.5\" rx=\"1.5\"/><rect x=\"13.5\" y=\"13.5\" width=\"7.5\" height=\"7.5\" rx=\"1.5\"/></svg>";
    pub(crate) const LIST: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M8 6h13M8 12h13M8 18h13\"/><path d=\"M3.5 6h.01M3.5 12h.01M3.5 18h.01\" stroke-width=\"2.6\"/></svg>";
    pub(crate) const EXPAND: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m6 10 6-6 6 6\"/><path d=\"m6 14 6 6 6-6\"/></svg>";
    pub(crate) const COLLAPSE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m6 4 6 6 6-6\"/><path d=\"m6 20 6-6 6 6\"/></svg>";
    pub(crate) const SELECT_ALL: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m2 13 4 4 8-9\"/><path d=\"m12 17 2 2 8-9\"/></svg>";
    pub(crate) const CLEAR: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M6 6l12 12M18 6 6 18\"/></svg>";
    pub(crate) const EXPORT: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M12 3v11\"/><path d=\"m7 9 5 5 5-5\"/><path d=\"M4 20h16\"/></svg>";
    pub(crate) const UNDO: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M4 11h10a5 5 0 0 1 0 10h-6\"/><path d=\"m8 7-4 4 4 4\"/></svg>";
    pub(crate) const CHECK: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m5 13 4 4L19 7\"/></svg>";
    pub(crate) const PLAY: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M8 5.5v13l11-6.5z\" fill=\"currentColor\" stroke=\"none\"/></svg>";
    pub(crate) const STOP: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"6.5\" y=\"6.5\" width=\"11\" height=\"11\" rx=\"2\" fill=\"currentColor\" stroke=\"none\"/></svg>";
    pub(crate) const PLUS: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M12 5v14M5 12h14\"/></svg>";
    pub(crate) const FOLDER: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 7.5A2.5 2.5 0 0 1 5.5 5h3.4l2 2.2h7.6A2.5 2.5 0 0 1 21 9.7v7.8a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z\"/></svg>";
    pub(crate) const FOLDER_MOVE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 7.5A2.5 2.5 0 0 1 5.5 5h3.4l2 2.2h4\"/><path d=\"M3 17.5V7.5\"/><path d=\"M13 18h8\"/><path d=\"m18 15 3 3-3 3\"/><path d=\"M3 20h7\"/></svg>";
    pub(crate) const SLIDERS: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3\"/><path d=\"M14 2v4M8 10v4M16 18v4\"/></svg>";
    pub(crate) const BACK: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M20 12H4\"/><path d=\"m10 6-6 6 6 6\"/></svg>";
    pub(crate) const SUN: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"4\"/><path d=\"M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.6 4.6l1.8 1.8M17.6 17.6l1.8 1.8M4.6 19.4l1.8-1.8M17.6 6.4l1.8-1.8\"/></svg>";
    pub(crate) const MOON: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M20.5 14.5A8.5 8.5 0 1 1 9.5 3.5a6.8 6.8 0 0 0 11 11z\"/></svg>";
}

/// A tinted icon in the theme's own colours.
pub(crate) fn icon(markup: &'static str, size: f32, tint: AdaptiveColor) -> impl IntoElement {
    svg(ImageSrc::svg_markup(markup))
        .size(px(size))
        .flex_shrink_0()
        .text_color(tint)
}

/// A square icon-only button for toolbars and dense rows. Its name lives in
/// the accessibility tree and the tooltip; nothing on screen spells it out.
pub(crate) fn icon_button(
    markup: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .flex_col()
        .child(
            button()
                .on_press(on_press)
                .accessibility_label(label)
                .flex_row()
                .items_center()
                .justify_center()
                .w(px(30.0))
                .h(px(30.0))
                .bg(colors::INSET)
                .border_1()
                .border_color(colors::BORDER)
                .rounded_full()
                .transition_colors()
                .duration(motion(180.0))
                .ease_out()
                .hover(|s| s.bg(colors::ACCENT_WASH))
                .child(icon(markup, 15.0, colors::TEXT)),
        )
        .tooltip(hint(label))
}

/// A borderless icon action, for the secondary controls of a row.
pub(crate) fn icon_ghost_button(
    markup: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .flex_col()
        .child(
            button()
                .on_press(on_press)
                .accessibility_label(label)
                .flex_row()
                .items_center()
                .justify_center()
                .w(px(28.0))
                .h(px(26.0))
                .rounded_full()
                .transition_colors()
                .duration(motion(180.0))
                .ease_out()
                .hover(|s| s.bg(colors::ACCENT_WASH))
                .child(icon(markup, 15.0, colors::ACCENT)),
        )
        .tooltip(hint(label))
}

/// An icon radio chip: the Grid/List switch and anything else that picks one
/// of a small set of views.
pub(crate) fn icon_chip(
    markup: &'static str,
    label: &'static str,
    active: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    radio()
        .checked(active)
        .on_change(move |_: &bool| on_press())
        .accessibility_label(label)
        .flex_row()
        .items_center()
        .justify_center()
        .w(px(30.0))
        .h(px(26.0))
        .rounded(px(radius::CONTROL))
        .bg(if active {
            colors::ACCENT_WASH_STRONG
        } else {
            colors::INSET
        })
        .border_1()
        .border_color(if active {
            colors::ACCENT
        } else {
            colors::BORDER
        })
        .child(icon(
            markup,
            14.0,
            if active {
                colors::ACCENT
            } else {
                colors::MUTED
            },
        ))
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
