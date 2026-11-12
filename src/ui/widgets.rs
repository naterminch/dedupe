//! Styled primitives shared by every pane.
//!
//! Native Shell ships behaviour without looks; the visual language lives here
//! so panes compose widgets instead of repeating colours, radii and paddings.
//! Colours come from [`crate::ui::palette`] — no raw hex below this module.

use std::cell::Cell;
use std::thread::LocalKey;

use nsh_base::{ButtonElement, button, switch, toggle, tooltip};
use nsh_core::element::{BorderCurve, ImageSrc, Motion, MotionPreset, StyleRefinement, input, svg};
use nsh_core::prelude::*;

use crate::ui::palette::{colors, radius, space, text};
use crate::ui::state::app;

/// Transition length for colour and transform changes.
///
/// The platform's reduce-motion preference collapses it to zero: the same end
/// state arrives, without the animation. Reads the environment reactively, so
/// turning the setting off is picked up on the next frame.
pub(crate) fn motion(ms: f32) -> impl Fn() -> f32 + Copy + 'static {
    move || if reduce_motion() { 0.0 } else { ms }
}

/// Motion tokens: every duration in the app comes from these, and every one of
/// them goes through [`motion`], so reduce-motion collapses it to zero.
pub(crate) mod timing {
    /// A control acknowledging the pointer: hover washes, press scales.
    pub(crate) const FAST: f32 = 150.0;
    /// The default: a control changing its look.
    pub(crate) const BASE: f32 = 180.0;
    /// A surface arriving or leaving in place: a scrim, a card, a toast.
    pub(crate) const SLOW: f32 = 240.0;
}

/// A pane arriving: it fades in while rising a few points, so changing page
/// reads as one movement instead of a swap. Created where the pane is built,
/// so entering the page again plays it again; with reduce-motion on, nsh lands
/// it at the end without moving. Bind it with `.bind_motion(&m)`.
pub(crate) fn entering() -> Motion {
    // `nsh_base`'s `motion()` handle, not this module's duration binding.
    nsh_core::element::motion().entering(MotionPreset::FadeInUp)
}

/// A surface arriving in place, without travelling: a scrim over the whole
/// window must not move, and neither should a list that is only being rebuilt.
/// Same rules as [`entering`].
pub(crate) fn fading_in() -> Motion {
    nsh_core::element::motion().entering(MotionPreset::FadeIn)
}

/// The keyboard's focus ring, shared by every control.
///
/// `focus_visible` draws it only when the keyboard moved the focus, never
/// after a click or a tap, so pointer users keep the quiet look while the
/// keyboard always shows where it is.
pub(crate) fn focus_ring(s: StyleRefinement) -> StyleRefinement {
    s.outline(px(2.0)).outline_color(colors::ACCENT)
}

/// A wheel line: the unit nsh counts a mouse wheel notch's delta in.
const WHEEL_LINE: f32 = 16.0;

/// Lines in one mouse wheel notch: nsh's own step is three of them (~48 pt).
const NOTCH_LINES: f32 = 3.0;

/// How far one notch moves a pane that does not name its own step: about one
/// compact group card, so a notch lands on the next row instead of stopping
/// half-way through the one above it. nsh's own notch crawls through a card.
const WHEEL_STEP: f32 = 240.0;

/// Points one notch should move, read as the wheel turns: a list can change
/// shape (a photo card is taller than a row) while it is on screen.
pub(crate) type NotchStep = fn() -> f32;

/// How much of a page-sized notch's page stays in view, so the row the last
/// notch stopped on is not skipped.
const PAGE_KEEP: f32 = 64.0;

/// Where a pane sits, as its own wheel last left it: `(offset, asked, viewport)`
/// — the offset to move from, the one the last notch asked for, and the
/// pane's height (what a page-sized notch moves).
pub(crate) type WheelAt = Cell<(f32, f32, f32)>;

/// A scrolling pane whose mouse wheel moves faster than the platform's.
///
/// A view inside the scroll view hears the wheel before the scroll view does
/// and takes it (nothing scrolls natively then), so the app moves the pane
/// itself. `at` is where the wheel last left it, as `(offset, asked)`: the
/// offset to move from and the one the last notch asked for, so that a move
/// the app did not make (a key, the scroll bar, a jump to the top) replaces it
/// instead of being undone by the next notch.
pub(crate) fn fast_scroll(
    at: &'static LocalKey<WheelAt>,
    content: impl IntoElement + 'static,
) -> impl IntoElement {
    fast_scroll_by(at, || WHEEL_STEP, content)
}

/// [`fast_scroll`] for a pane that knows its own row height: the results list
/// draws tall photo cards in grid layout and compact rows in list layout, and
/// a notch should move one card either way.
pub(crate) fn fast_scroll_by(
    at: &'static LocalKey<WheelAt>,
    step: NotchStep,
    content: impl IntoElement + 'static,
) -> impl IntoElement {
    fast_wheel(
        scroll().flex_1().w_full().min_w(px(0.0)).min_h(px(0.0)),
        at,
        step,
        content,
    )
}

/// [`fast_scroll`] for a pane of a fixed maximum height: a dialog's list.
pub(crate) fn fast_scroll_max(
    at: &'static LocalKey<WheelAt>,
    max_height: Points,
    content: impl IntoElement + 'static,
) -> impl IntoElement {
    fast_wheel(scroll().max_h(max_height), at, || WHEEL_STEP, content)
}

/// The body of [`fast_scroll`] and [`fast_scroll_max`], over the scroll view
/// the pane asks for.
fn fast_wheel(
    area: Scroll,
    at: &'static LocalKey<WheelAt>,
    step: NotchStep,
    content: impl IntoElement + 'static,
) -> impl IntoElement {
    let scroller = element_ref();
    let handle = scroller;
    area.element_ref(&scroller)
        .on_layout(move |frame: &Bounds<Points>| {
            let height = frame.size.height.0;
            at.with(|cell| {
                let (pos, asked, _) = cell.get();
                cell.set((pos, asked, height));
            });
        })
        .on_scroll(move |e: &ScrollEvent| {
            at.with(|cell| {
                let (_, asked, viewport) = cell.get();
                if scroll_moved_elsewhere(asked, e.offset.y.0) {
                    cell.set((e.offset.y.0, asked, viewport));
                }
            });
        })
        .child(
            view()
                .w_full()
                .on_wheel(move |e: &WheelEvent| {
                    let lines = e.delta.points(px(WHEEL_LINE)).y.0;
                    // A touchpad sends fine, momentum-driven points already:
                    // only mouse notches get the longer step.
                    let precise = e.delta.precise();
                    let page = !precise && app().with_untracked(|s| s.wheel_page);
                    let (pos, _, viewport) = at.with(|cell| cell.get());
                    let step = wheel_step(lines, precise, page, viewport, step());
                    if step == 0.0 {
                        return;
                    }
                    let next = (pos + step).max(0.0);
                    at.with(|cell| cell.set((next, next, viewport)));
                    handle.scroll_to(point(px(0.0), px(next)), false);
                })
                .child(content),
        )
}

/// How far one wheel event moves a pane, given the delta in points.
///
/// A mouse notch is counted in whole notches so a wheel that reports several
/// lines per notch still moves one card per notch, and the count is signed so
/// the wheel turns both ways. Page mode moves a whole page per notch, less the
/// strip `PAGE_KEEP` keeps so a card never ends exactly on the fold; a
/// touchpad's precise deltas are already points and pass straight through.
fn wheel_step(lines: f32, precise: bool, page: bool, viewport: f32, card: f32) -> f32 {
    let notches = lines / (NOTCH_LINES * WHEEL_LINE);
    if precise {
        lines
    } else if page {
        (viewport - PAGE_KEEP).max(WHEEL_STEP) * notches
    } else {
        notches * card
    }
}

/// Whether a scroll event came from somewhere other than our own `scroll_to`.
///
/// `asked` is what we last aimed at; it is NaN until the wheel has moved the
/// pane once, and a scrollbar drag or keyboard scroll before (or after) that
/// is external, so its offset becomes the pane's remembered position.
fn scroll_moved_elsewhere(asked: f32, offset: f32) -> bool {
    !asked.is_finite() || (offset - asked).abs() > 0.5
}

/// The states every control needs: a press that reads as pressed, the
/// keyboard's ring, and a disabled look for controls that can be blocked.
///
/// `nsh_base` draws none of these by itself, so a button without them looks
/// inert on press and hides the keyboard's focus entirely.
trait ControlStates: Sized {
    fn control_states(self) -> Self;
}

impl ControlStates for ButtonElement {
    fn control_states(self) -> Self {
        // A press sinks the control a hair as well as dimming it: the pointer
        // gets an answer on the frame it lands, not only on release.
        self.active(|s| s.opacity(0.9).scale(0.97))
            .transition()
            .duration(motion(timing::FAST))
            .ease_out()
            .focus_visible(focus_ring)
            .disabled_style(|s| s.opacity(0.45))
    }
}

/// A card surface for panels, cards and dialogs.
pub(crate) fn card() -> View {
    view()
        .flex_col()
        .bg(colors::CARD)
        .border_1()
        .border_color(colors::HAIRLINE)
        .rounded(px(radius::SHELL))
        .border_curve(BorderCurve::Continuous)
}

/// The outer tray of the double-bezel: a soft tinted shell with a hairline
/// seam and a 3 pt inset, so a card reads as glass seated in a machined tray
/// instead of a rectangle painted on the background. Content goes in a
/// [`bezel_core`] nested inside it.
pub(crate) fn bezel() -> View {
    view()
        .flex_col()
        .p(px(3.0))
        .bg(colors::SHELL)
        .border_1()
        .border_color(colors::HAIRLINE)
        .rounded(px(radius::SHELL))
        .border_curve(BorderCurve::Continuous)
}

/// The core surface inside a [`bezel`]: the card tint, a concentric radius
/// (the tray's radius minus its inset) so the two curves stay parallel, and
/// its own hairline where it meets the tray.
pub(crate) fn bezel_core() -> View {
    view()
        .flex_col()
        .w_full()
        .min_w(px(0.0))
        .bg(colors::CARD)
        .border_1()
        .border_color(colors::HAIRLINE)
        .rounded(px(radius::SHELL - 3.0))
        .border_curve(BorderCurve::Continuous)
}

/// Page-level heading ("dedupe", "Settings").
pub(crate) fn heading(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT)
}

/// Page title ("General", "Preferences"), one step above [`heading`].
pub(crate) fn page_heading(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::PAGE))
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

/// The smallest label in the system: an eyebrow above a heading. The copy is
/// written upper-case; weight carries the tracking, since the engine owns the
/// letter spacing.
pub(crate) fn eyebrow(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::MICRO))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::MUTED)
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
            .transition_colors()
            .duration(motion(timing::BASE))
            .ease_out()
            .hover(|s| s.opacity(0.92))
            .control_states()
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
            .duration(motion(timing::BASE))
            .ease_out()
            .hover(|s| s.bg(colors::ELEVATED))
            .control_states()
            .child(
                text(label)
                    .text_size(px(text::BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::TEXT),
            ),
    )
}

/// A secondary pill with a leading icon, sized exactly like
/// [`secondary_button`] so it lines up with a labelled action beside it.
/// For actions whose glyph carries part of the meaning (Back).
pub(crate) fn icon_secondary_button(
    markup: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> View {
    view().flex_col().child(
        button()
            .on_press(on_press)
            .accessibility_label(label)
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(space::XS))
            .px(px(space::LG))
            .py(px(space::SM))
            // No border: the hairline would add 2px and leave this pill a
            // different height from the solid pill it sits beside.
            .bg(colors::PANE)
            .rounded_full()
            .transition_colors()
            .duration(motion(timing::BASE))
            .ease_out()
            .hover(|s| s.bg(colors::ELEVATED))
            .control_states()
            .child(icon(markup, 15.0, colors::TEXT))
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
            .duration(motion(timing::BASE))
            .ease_out()
            .hover(|s| s.bg(colors::ACCENT_WASH))
            .control_states()
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
        .duration(motion(timing::FAST))
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
            .transition_colors()
            .duration(motion(timing::BASE))
            .ease_out()
            .hover(|s| s.opacity(0.92))
            .control_states()
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
            s.w(px(40.0))
                .h(px(22.0))
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
        .thumb_style(|s| s.w(px(16.0)).h(px(16.0)).rounded_full().bg(colors::TEXT))
        .thumb_class("transition-transform checked:translate-x-5")
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
        .border_color(colors::HAIRLINE)
        .rounded(px(radius::CONTROL))
        .transition_colors()
        .duration(motion(timing::BASE))
}

/// Icons, as inline SVG stroked with `currentColor`: one markup works in both
/// appearances, at any size, and follows whatever tint the control gives it.
///
/// They are decorative. Every control that shows one owns the accessible name
/// (and the tooltip), which is what the tests press by (`label:...`).
pub(crate) mod icons {
    pub(crate) const HOME: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 10.5 12 3l9 7.5\"/><path d=\"M5 9.5V21h14V9.5\"/></svg>";
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
    pub(crate) const SUN: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"4\"/><path d=\"M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.6 4.6l1.8 1.8M17.6 17.6l1.8 1.8M4.6 19.4l1.8-1.8M17.6 6.4l1.8-1.8\"/></svg>";
    pub(crate) const MOON: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M20.5 14.5A8.5 8.5 0 1 1 9.5 3.5a6.8 6.8 0 0 0 11 11z\"/></svg>";
    pub(crate) const SEARCH: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"11\" cy=\"11\" r=\"7\"/><path d=\"m20.5 20.5-4.2-4.2\"/></svg>";
    pub(crate) const TRASH: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M4 7h16\"/><path d=\"M9 7V5h6v2\"/><path d=\"m6 7 1 13h10l1-13\"/><path d=\"M10 11v6\"/><path d=\"M14 11v6\"/></svg>";
    pub(crate) const LOCK: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"5\" y=\"11\" width=\"14\" height=\"9\" rx=\"2\"/><path d=\"M8 11V8a4 4 0 0 1 8 0v3\"/></svg>";
    pub(crate) const CHEVRON_DOWN: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m6 9 6 6 6-6\"/></svg>";
    pub(crate) const HELP: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M9.6 9.2a2.4 2.4 0 1 1 3.6 2.1c-.8.5-1.2.9-1.2 1.7\"/><path d=\"M12 16.5h.01\" stroke-width=\"2.6\"/></svg>";
    pub(crate) const ARROW_LEFT: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M19 12H5\"/><path d=\"m11 18-6-6 6-6\"/></svg>";
    pub(crate) const ARROW_UP_RIGHT: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M7 17 17 7\"/><path d=\"M8 7h9v9\"/></svg>";
}

/// A tinted icon in the theme's own colours.
pub(crate) fn icon(markup: &'static str, size: f32, tint: AdaptiveColor) -> impl IntoElement {
    svg(ImageSrc::svg_markup(markup))
        .size(px(size))
        .flex_shrink_0()
        .text_color(tint)
}

/// The height of a labelled pill (`py(SM)` around a body line). An icon
/// control sitting in a row of pills takes this size so the row lines up.
pub(crate) const PILL_HEIGHT: f32 = 35.0;

/// A square icon-only button for toolbars and dense rows. Its name lives in
/// the accessibility tree and the tooltip; nothing on screen spells it out.
pub(crate) fn icon_button(
    markup: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> View {
    icon_button_sized(markup, label, on_press, 30.0)
}

/// A round icon button at pill height, for a row that mixes icon controls with
/// labelled pills (the results actions: Back, Delete, consolidate, export).
pub(crate) fn icon_pill_button(
    markup: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> View {
    icon_button_sized(markup, label, on_press, PILL_HEIGHT)
}

fn icon_button_sized(
    markup: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
    size: f32,
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
                .w(px(size))
                .h(px(size))
                .bg(colors::INSET)
                .border_1()
                .border_color(colors::HAIRLINE)
                .rounded_full()
                .transition_colors()
                .duration(motion(timing::BASE))
                .ease_out()
                .hover(|s| s.bg(colors::ELEVATED))
                .control_states()
                .hit_slop(EdgeInsets::all(px(4.0)))
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
                .duration(motion(timing::BASE))
                .ease_out()
                .hover(|s| s.bg(colors::ACCENT_WASH))
                .control_states()
                .hit_slop(EdgeInsets::all(px(4.0)))
                .child(icon(markup, 15.0, colors::ACCENT)),
        )
        .tooltip(hint(label))
}

/// Fill, border and content colours for a selected or idle option.
///
/// Every selectable control — settings chips, keep-mode chips, the layout
/// toggle — shares this one decision, so "this one is chosen" reads the same
/// everywhere and a theme change lands in one place.
pub(crate) fn selection(active: bool) -> (AdaptiveColor, AdaptiveColor, AdaptiveColor) {
    if active {
        (colors::ACCENT, colors::ACCENT, colors::ON_ACCENT)
    } else {
        (colors::INSET, colors::BORDER, colors::MUTED)
    }
}

/// An icon toggle: the Grid/List switcher and anything else that picks one
/// of a small set of views. `on_press` runs on the press that takes it, not
/// on one that would release it.
pub(crate) fn icon_chip(
    markup: &'static str,
    label: &'static str,
    active: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let (fill, border, content) = selection(active);
    toggle()
        .pressed(active)
        .on_change(move |next: &bool| {
            if *next {
                on_press();
            }
        })
        .accessibility_label(label)
        .flex_row()
        .items_center()
        .justify_center()
        .w(px(30.0))
        .h(px(26.0))
        .rounded(px(radius::CONTROL))
        .bg(fill)
        .border_1()
        .border_color(if active { border } else { colors::HAIRLINE })
        .transition()
        .duration(motion(timing::FAST))
        .active(|s| s.opacity(0.9).scale(0.97))
        .focus_visible(focus_ring)
        .disabled_style(|s| s.opacity(0.45))
        .child(icon(markup, 14.0, content))
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

/// A muted question mark with the row's explanation in its tooltip: the
/// settings rows keep the reference's clean single line, and the copy moves
/// into the hover.
pub(crate) fn help_dot(tip: impl IntoReactive<SharedString>) -> impl IntoElement {
    view()
        .flex_shrink_0()
        .child(icon(icons::HELP, 14.0, colors::MUTED))
        .tooltip(hint(tip))
}

#[cfg(test)]
mod tests {
    use super::{PAGE_KEEP, WHEEL_STEP, scroll_moved_elsewhere, wheel_step};

    /// A notch: three lines of 16 points, the platform's usual step.
    const NOTCH: f32 = 48.0;

    #[test]
    fn a_notch_moves_one_card_each_way() {
        assert_eq!(wheel_step(NOTCH, false, false, 800.0, 355.0), 355.0);
        assert_eq!(wheel_step(-NOTCH, false, false, 800.0, 355.0), -355.0);
        assert_eq!(wheel_step(2.0 * NOTCH, false, false, 800.0, 240.0), 480.0);
        assert_eq!(wheel_step(-2.0 * NOTCH, false, false, 800.0, 240.0), -480.0);
    }

    #[test]
    fn page_mode_keeps_the_wheel_direction() {
        let page = 800.0 - PAGE_KEEP;
        assert_eq!(wheel_step(NOTCH, false, true, 800.0, 355.0), page);
        assert_eq!(wheel_step(-NOTCH, false, true, 800.0, 355.0), -page);
        assert_eq!(
            wheel_step(2.0 * NOTCH, false, true, 800.0, 355.0),
            2.0 * page
        );
    }

    #[test]
    fn a_page_is_never_shorter_than_the_base_step() {
        let tall = WHEEL_STEP + PAGE_KEEP;
        assert_eq!(wheel_step(NOTCH, false, true, tall, 355.0), WHEEL_STEP);
        assert_eq!(wheel_step(-NOTCH, false, true, tall, 355.0), -WHEEL_STEP);
    }

    #[test]
    fn precise_deltas_pass_straight_through() {
        assert_eq!(wheel_step(7.0, true, false, 800.0, 355.0), 7.0);
        assert_eq!(wheel_step(-7.0, true, true, 800.0, 355.0), -7.0);
    }

    #[test]
    fn a_scroll_before_the_first_notch_is_external() {
        // `asked` starts as NaN: a scrollbar drag before any wheel movement
        // still has to become the remembered position.
        assert!(scroll_moved_elsewhere(f32::NAN, 240.0));
        // Our own target coming back is not a move.
        assert!(!scroll_moved_elsewhere(355.0, 355.0));
        // Anything else moved the pane.
        assert!(scroll_moved_elsewhere(355.0, 120.0));
    }
}
