//! Styled primitives shared by every pane.
//!
//! Native Shell ships behaviour without looks; the visual language lives here
//! so panes compose widgets instead of repeating colours, radii and paddings.
//! Colours come from [`crate::ui::palette`] — no raw hex below this module.

use std::cell::Cell;
use std::sync::{Arc, OnceLock};
use std::thread::LocalKey;

/// The item of an [`option_select`] list: features build their lists from it.
pub(crate) use nsh_base::SelectItem;
use nsh_base::{
    ButtonElement, button, checkbox, collapsible, dialog, number_input, progress, radio,
    radio_group, select, switch, tab, tabs, toggle, toggle_group, toolbar, toolbar_group, tooltip,
};
use nsh_core::AccessibilityState;
use nsh_core::element::{
    BorderCurve, ImageSrc, Motion, MotionPreset, StyleRefinement, input, svg, video,
};
use nsh_core::prelude::*;

use crate::ui::palette::{colors, radius, size, space, text};
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
    s.outline(px(size::FOCUS_RING))
        .outline_color(colors::ACCENT)
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

/// The one container: a white card on the ground, one line at its edge, no
/// shadow and no tray around it.
pub(crate) fn card() -> View {
    view()
        .flex_col()
        .bg(colors::CARD)
        .border_1()
        .border_color(colors::BORDER)
        .rounded(px(radius::CARD))
        .border_curve(BorderCurve::Continuous)
}

/// The screen's title, one per screen. Screen readers hear it as a header.
pub(crate) fn heading(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT)
        .role(AccessibilityRole::Header)
}

/// A section's title: a settings section, the scan form, a dialog.
pub(crate) fn section_title(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::TITLE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT)
        .role(AccessibilityRole::Header)
}

/// A row's or a card's own title at body size.
pub(crate) fn panel_title(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::BODY))
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors::TEXT)
}

/// The figure a screen answers with: the space a delete gives back. The one
/// hue, the largest number on the screen, digits of equal width so it does
/// not jitter as the selection changes.
pub(crate) fn figure(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::FIGURE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::ACCENT)
        .tabular_nums()
}

/// Secondary copy: hints, counts, paths under a name.
pub(crate) fn caption(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::CAPTION))
        .text_color(colors::MUTED)
}

/// Body-size copy in the secondary ink: a row's description, a form hint
/// that has to be read rather than glanced at.
pub(crate) fn description(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::BODY))
        .text_color(colors::MUTED)
}

/// The sentence-case label beside or above a control.
pub(crate) fn field_label(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::CAPTION))
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors::MUTED)
}

/// What a button means, which decides its fill and its ink. The look of each
/// lives here once; screens pick the meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    /// The one action that moves the screen's job forward: ink-filled.
    Primary,
    /// A quieter action beside it: a grey fill.
    Secondary,
    /// A borderless action in a dense row.
    Ghost,
    /// Destruction: the deep red fill.
    Danger,
}

impl Tone {
    fn ink(self) -> AdaptiveColor {
        match self {
            Self::Primary => colors::ON_PRIMARY,
            Self::Secondary | Self::Ghost => colors::TEXT,
            Self::Danger => colors::ON_DANGER,
        }
    }
}

/// Every labelled button: one height, one radius, one press and focus, the
/// fill and ink from its [`Tone`]. `name` overrides the accessible name for a
/// button whose glyph carries part of the meaning.
fn action(
    tone: Tone,
    glyph: Option<&'static str>,
    name: Option<&'static str>,
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    let ink = tone.ink();
    let mut press = button()
        .on_press(on_press)
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(space::SM))
        .min_h(px(size::CONTROL))
        .px(px(space::MD))
        .rounded(px(radius::CONTROL))
        .transition_colors()
        .duration(motion(timing::BASE))
        .ease_out();
    press = match tone {
        Tone::Primary => press.bg(colors::PRIMARY).hover(|s| s.opacity(0.86)),
        Tone::Secondary => press.bg(colors::PANE).hover(|s| s.bg(colors::ELEVATED)),
        Tone::Ghost => press.hover(|s| s.bg(colors::PANE)),
        Tone::Danger => press.bg(colors::DANGER_SURFACE).hover(|s| s.opacity(0.9)),
    };
    if let Some(name) = name {
        press = press.accessibility_label(name);
    }
    if let Some(glyph) = glyph {
        press = press.child(icon(glyph, size::ICON, ink));
    }
    view().flex_col().child(
        press.control_states().child(
            text(label)
                .text_size(px(text::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ink),
        ),
    )
}

/// The screen's main call to action: filled with ink.
pub(crate) fn primary_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    action(Tone::Primary, None, None, label, on_press)
}

/// A quieter action next to a primary one.
pub(crate) fn secondary_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    action(Tone::Secondary, None, None, label, on_press)
}

/// A secondary button with a leading glyph, for actions whose glyph carries
/// part of the meaning (Back, Add folder, Select all).
pub(crate) fn icon_secondary_button(
    markup: &'static str,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> View {
    action(Tone::Secondary, Some(markup), Some(label), label, on_press)
}

/// A borderless labelled action (Clear selection, Show more).
pub(crate) fn ghost_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    action(Tone::Ghost, None, None, label, on_press)
}

/// A destructive action (Delete N, Stop).
pub(crate) fn danger_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    action(Tone::Danger, None, None, label, on_press)
}

/// A compact ink action inside a line of metadata: "Keep this one instead"
/// and "Keep too" in the delete plan. Caption-sized, so it sits in
/// the line without becoming the row's headline.
pub(crate) fn quiet_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> View {
    view().flex_shrink_0().child(
        button()
            .on_press(on_press)
            .flex_row()
            .items_center()
            .justify_center()
            .min_h(px(size::COMPACT))
            .px(px(space::SM))
            .rounded(px(radius::CONTROL))
            .bg(colors::PANE)
            .transition_colors()
            .duration(motion(timing::FAST))
            .ease_out()
            .hover(|s| s.bg(colors::ELEVATED))
            .control_states()
            .child(
                text(label)
                    .text_size(px(text::CAPTION))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors::TEXT),
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
        .size(px(size::ICON))
        .flex_shrink_0()
        .text_color(colors::MUTED)
        .transition_transform()
        .duration(motion(timing::FAST))
        .rotate(if open { deg(90.0) } else { deg(0.0) })
}

/// The switch's track and thumb. 36 wide less the 1 pt border and 2 pt
/// inset each side leaves 30 pt: a 14 pt thumb travels 16 pt, which is the
/// `translate-x-4` in its class.
const SWITCH_W: f32 = 36.0;
const SWITCH_H: f32 = 20.0;
const SWITCH_THUMB: f32 = 14.0;

/// An on/off control on the platform switch semantics (Space and Return,
/// screen readers hear "checked"): a 36×20 track, ink when on — a setting
/// being on is not an action, so it takes no hue.
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
            s.w(px(SWITCH_W))
                .h(px(SWITCH_H))
                .flex_row()
                .items_center()
                .p(px(space::XXS))
                .rounded_full()
                .bg(if checked {
                    colors::PRIMARY
                } else {
                    colors::INSET
                })
                .border_1()
                .border_color(if checked {
                    colors::PRIMARY
                } else {
                    colors::CONTROL_BORDER
                })
        })
        .thumb_style(|s| {
            s.w(px(SWITCH_THUMB))
                .h(px(SWITCH_THUMB))
                .rounded_full()
                .bg(if checked {
                    colors::ON_PRIMARY
                } else {
                    colors::MUTED
                })
        })
        .thumb_class("transition-transform checked:translate-x-4")
}

/// A small tag: a state or a kind (Keep, Protected, a finding's kind). Pale
/// pairs for status, the pane for anything neutral.
fn chip(
    label: impl IntoReactive<SharedString>,
    background: AdaptiveColor,
    foreground: AdaptiveColor,
) -> View {
    view()
        .flex_shrink_0()
        .px(px(space::SM))
        .py(px(space::XXS))
        .flex_row()
        .items_center()
        .bg(background)
        .rounded(px(radius::CONTROL))
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(foreground),
        )
}

/// A one-pixel rule inside a card, between rows or before a footer.
pub(crate) fn divider() -> View {
    // One device pixel, not one point: a point-wide rule doubles on a 2x
    // display and reads as a filled bar instead of a hairline.
    view()
        .h(move || px(1.0 / pixel_ratio()))
        .w_full()
        .bg(colors::HAIRLINE)
}

/// A single-line text field: white, outlined one step darker than a card
/// edge, the accent edge while it has the keyboard.
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
        .min_w(px(0.0))
        .h(px(size::CONTROL))
        .px(px(space::MD))
        .text_size(px(text::BODY))
        .text_color(colors::TEXT)
        .bg(colors::CARD)
        .border_1()
        .border_color(colors::CONTROL_BORDER)
        .rounded(px(radius::CONTROL))
        .transition_colors()
        .duration(motion(timing::BASE))
        .focus(|s| s.border_color(colors::ACCENT))
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
    pub(crate) const PLAY: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M8 5.5v13l11-6.5z\" fill=\"currentColor\" stroke=\"none\"/></svg>";
    pub(crate) const STOP: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"6.5\" y=\"6.5\" width=\"11\" height=\"11\" rx=\"2\" fill=\"currentColor\" stroke=\"none\"/></svg>";
    pub(crate) const PLUS: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M12 5v14M5 12h14\"/></svg>";
    pub(crate) const FOLDER: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 7.5A2.5 2.5 0 0 1 5.5 5h3.4l2 2.2h7.6A2.5 2.5 0 0 1 21 9.7v7.8a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z\"/></svg>";
    pub(crate) const FILE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M6 2.5h7.5L19 8v13.5H6z\"/><path d=\"M13.5 2.5V8H19\"/></svg>";
    pub(crate) const FOLDER_MOVE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 7.5A2.5 2.5 0 0 1 5.5 5h3.4l2 2.2h4\"/><path d=\"M3 17.5V7.5\"/><path d=\"M13 18h8\"/><path d=\"m18 15 3 3-3 3\"/><path d=\"M3 20h7\"/></svg>";
    pub(crate) const SLIDERS: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3\"/><path d=\"M14 2v4M8 10v4M16 18v4\"/></svg>";
    pub(crate) const SUN: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"4\"/><path d=\"M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.6 4.6l1.8 1.8M17.6 17.6l1.8 1.8M4.6 19.4l1.8-1.8M17.6 6.4l1.8-1.8\"/></svg>";
    pub(crate) const MOON: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M20.5 14.5A8.5 8.5 0 1 1 9.5 3.5a6.8 6.8 0 0 0 11 11z\"/></svg>";
    pub(crate) const SEARCH: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"11\" cy=\"11\" r=\"7\"/><path d=\"m20.5 20.5-4.2-4.2\"/></svg>";
    pub(crate) const TRASH: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M4 7h16\"/><path d=\"M9 7V5h6v2\"/><path d=\"m6 7 1 13h10l1-13\"/><path d=\"M10 11v6\"/><path d=\"M14 11v6\"/></svg>";
    pub(crate) const LOCK: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"5\" y=\"11\" width=\"14\" height=\"9\" rx=\"2\"/><path d=\"M8 11V8a4 4 0 0 1 8 0v3\"/></svg>";
    pub(crate) const CHEVRON_DOWN: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m6 9 6 6 6-6\"/></svg>";
    pub(crate) const ARROW_LEFT: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M19 12H5\"/><path d=\"m11 18-6-6 6-6\"/></svg>";
}

/// The app's own mark, embedded in the binary: a single-file install has no
/// asset directory to load it from. It is decoded once at 64 px and handed
/// over as pixels, so it stays crisp on a 2x display at the sizes the shell
/// draws it without keeping the 512 px original in memory.
pub(crate) fn logo(side: f32) -> impl IntoElement {
    static LOGO: OnceLock<(u32, u32, Arc<[u8]>)> = OnceLock::new();
    let (width, height, pixels) = LOGO.get_or_init(|| {
        const SIDE: u32 = 64;
        // The asset is embedded at build time, so a decode failure means the
        // repository's PNG is corrupt. That is a reason to draw nothing, not
        // to take the window down at first paint.
        let Some(decoded) =
            ::image::load_from_memory(include_bytes!("../../assets/icons/logo.png"))
                .ok()
                .map(|decoded| decoded.into_rgba8())
        else {
            return (1, 1, Arc::from([0_u8, 0, 0, 0].as_slice()));
        };
        let scaled = ::image::imageops::resize(
            &decoded,
            SIDE,
            SIDE,
            ::image::imageops::FilterType::Lanczos3,
        );
        let pixels: Arc<[u8]> = scaled.into_raw().into();
        (SIDE, SIDE, pixels)
    });
    image(ImageSrc::rgba(*width, *height, Arc::clone(pixels)))
        .w(px(side))
        .h(px(side))
        .flex_shrink_0()
        .object_fit(ResizeMode::Contain)
}

/// A tinted icon in the theme's own colours.
fn icon(markup: &'static str, size: f32, tint: AdaptiveColor) -> impl IntoElement {
    svg(ImageSrc::svg_markup(markup))
        .size(px(size))
        .flex_shrink_0()
        .text_color(tint)
}

/// A square icon-only button at control height, on the secondary fill. Its
/// name lives in the accessibility tree and the tooltip.
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
                .w(px(size::CONTROL))
                .h(px(size::CONTROL))
                .bg(colors::PANE)
                .rounded(px(radius::CONTROL))
                .transition_colors()
                .duration(motion(timing::BASE))
                .ease_out()
                .hover(|s| s.bg(colors::ELEVATED))
                .control_states()
                .child(icon(markup, size::ICON, colors::TEXT)),
        )
        .tooltip(hint(label))
}

/// A borderless icon action for the secondary controls of a row or a bar.
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
                .w(px(size::COMPACT))
                .h(px(size::COMPACT))
                .rounded(px(radius::CONTROL))
                .transition_colors()
                .duration(motion(timing::BASE))
                .ease_out()
                .hover(|s| s.bg(colors::PANE))
                .control_states()
                .hit_slop(EdgeInsets::all(px(size::HIT_SLOP)))
                .child(icon(markup, size::ICON, colors::MUTED)),
        )
        .tooltip(hint(label))
}

// ---------------------------------------------------------------------------
// Content text. Features name what a string is; the size, weight and ink of
// each role live here.
// ---------------------------------------------------------------------------

/// Running text at body size in ink.
pub(crate) fn body(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::BODY))
        .text_color(colors::TEXT)
}

/// The name of a thing the reader acts on: a file, a group, a folder.
pub(crate) fn name(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::BODY))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors::TEXT)
}

/// A size or a count in a row: ink, digits of equal width so a column of
/// them ends on one edge.
pub(crate) fn amount(label: impl IntoReactive<SharedString>) -> Text {
    text(label)
        .text_size(px(text::BODY))
        .font_weight(FontWeight::MEDIUM)
        .text_color(colors::TEXT)
        .tabular_nums()
}

/// The ink of a glyph: the reading ink, or the secondary one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ink {
    Text,
    Muted,
}

impl Ink {
    fn color(self) -> AdaptiveColor {
        match self {
            Self::Text => colors::TEXT,
            Self::Muted => colors::MUTED,
        }
    }
}

/// A 16 pt glyph beside a label in a row.
pub(crate) fn glyph(markup: &'static str, ink: Ink) -> impl IntoElement {
    icon(markup, size::ICON, ink.color())
}

/// The product's mark and name: the top of the folder pane.
pub(crate) fn wordmark() -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::SM))
        .child(logo(24.0))
        .child(
            text("dedupe")
                .text_size(px(text::TITLE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::TEXT),
        )
}

// ---------------------------------------------------------------------------
// Badges and notices.
// ---------------------------------------------------------------------------

/// What a badge says, which decides its backing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BadgeTone {
    /// A kind or a neutral fact, on the ground or a card.
    Neutral,
    /// A neutral fact on a pane-coloured tile: lifted to the card surface.
    Raised,
    /// The copy a group keeps.
    Keep,
}

/// A short state or kind (Keeper, Protected, a finding's kind).
pub(crate) fn badge(label: impl IntoReactive<SharedString>, tone: BadgeTone) -> View {
    let (fill, ink) = match tone {
        BadgeTone::Neutral => (colors::PANE, colors::MUTED),
        BadgeTone::Raised => (colors::CARD, colors::MUTED),
        BadgeTone::Keep => (colors::KEEP_WASH, colors::KEEP_INK),
    };
    chip(label, fill, ink)
}

/// A quiet block of explanation on the pane surface: a tip, a note about a
/// missing tool. No edge — it is information, not a container to act in.
pub(crate) fn notice() -> View {
    view()
        .flex_col()
        .gap(px(space::XS))
        .p(px(space::MD))
        .bg(colors::PANE)
        .rounded(px(radius::CARD))
}

/// A failure that belongs in the list it happened in: a red edge and the
/// error in its readable red.
pub(crate) fn error_notice(message: impl IntoReactive<SharedString>) -> View {
    card().p(px(space::MD)).border_color(colors::DANGER).child(
        text(message)
            .text_size(px(text::CAPTION))
            .text_color(colors::DANGER_TEXT),
    )
}

// ---------------------------------------------------------------------------
// Regions.
// ---------------------------------------------------------------------------

/// The window's ground, under every pane.
pub(crate) fn ground() -> View {
    view().bg(colors::BACKGROUND)
}

/// A side pane on the ground (the folder pane, the settings nav): one rule
/// at its trailing edge separates it from the work.
pub(crate) fn side_pane(width: f32) -> View {
    view()
        .flex_col()
        .w(px(width))
        .h_full()
        .flex_shrink_0()
        .bg(colors::SIDEBAR)
        .border_r_1()
        .border_color(colors::BORDER)
}

/// The status line at the foot of the window.
pub(crate) fn status_strip() -> View {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(space::LG))
        .px(px(space::XXL))
        .py(px(space::SM))
        .bg(colors::BACKGROUND)
        .border_t_1()
        .border_color(colors::BORDER)
}

// ---------------------------------------------------------------------------
// A group's copies.
// ---------------------------------------------------------------------------

/// What the reader decided about one copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    /// The copy the group keeps.
    Keep,
    /// Ticked to be deleted.
    Delete,
    /// Neither.
    Undecided,
}

/// The surface of one copy: a neutral tile whose edge says its decision —
/// green keeps, amber goes, an undecided copy has no edge of its own. Only
/// the edge and the badge carry the hue, so a group of five reads as five
/// tiles with two marks, not five coloured blocks.
pub(crate) fn tile(decision: Decision) -> View {
    view()
        .p(px(space::SM))
        .bg(colors::PANE)
        .rounded(px(radius::CARD))
        .border_curve(BorderCurve::Continuous)
        .border_1()
        .border_color(match decision {
            Decision::Keep => colors::KEEP,
            Decision::Delete => colors::SELECT,
            Decision::Undecided => colors::PANE,
        })
        // Picking the keeper or ticking a copy changes only the edge: the
        // change crosses over instead of snapping.
        .transition_colors()
        .duration(motion(timing::FAST))
}

/// The tick's box: a caption's height, its corner a quarter of its side.
const TICK_BOX: f32 = 12.0;
const TICK_BOX_RADIUS: f32 = 3.0;

/// A copy's tick: the keeper's is checked and fixed, a ticked copy's amber,
/// an idle one an outline. The word beside the box says the state, so the
/// hue is never the only signal.
pub(crate) fn tick(
    label: &'static str,
    decision: Decision,
    on_toggle: impl Fn() + 'static,
) -> impl IntoElement {
    let keep = decision == Decision::Keep;
    let active = decision != Decision::Undecided;
    let (fill, mark, ink) = match decision {
        Decision::Keep => (colors::KEEP_WASH, colors::KEEP, colors::KEEP_INK),
        Decision::Delete => (colors::SELECT_WASH, colors::SELECT, colors::SELECT_INK),
        Decision::Undecided => (colors::CARD, colors::MUTED, colors::MUTED),
    };
    checkbox()
        .checked(active)
        .disabled(keep)
        .on_change(move |_: &bool| on_toggle())
        .min_h(px(size::COMPACT))
        .px(px(space::SM))
        .flex_row()
        .items_center()
        .gap(px(space::XS + space::XXS))
        .rounded(px(radius::CONTROL))
        .bg(fill)
        .border_1()
        .border_color(if active { fill } else { colors::CONTROL_BORDER })
        .transition_colors()
        .duration(motion(timing::FAST))
        .hover(move |s| if active { s } else { s.bg(colors::PANE) })
        .active(|s| s.opacity(0.9))
        .focus_visible(focus_ring)
        .indicator_style(move |s| {
            s.w(px(TICK_BOX))
                .h(px(TICK_BOX))
                .rounded(px(TICK_BOX_RADIUS))
                .flex_row()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(mark)
                .checked(move |s| s.bg(mark).border_color(mark))
        })
        .indicator(
            text(if active { "✓" } else { "" })
                // Below the caption step on purpose: the glyph sits in a 12 pt
                // box, so the token size would overflow the control it marks.
                // It still scales with the user's text size.
                .text_size(px(9.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors::CARD),
        )
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ink),
        )
}

/// A row thumbnail's box, in points. Fixed so an image finishing its async
/// load never changes the row's geometry.
pub(crate) const THUMB_W: f32 = 72.0;
pub(crate) const THUMB_H: f32 = 56.0;

/// The height of a card's large preview, and of the inline player that
/// replaces it.
pub(crate) const PREVIEW_H: f32 = 180.0;

/// Which preview box: the large one on a grid card, or a list row's
/// thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreviewSize {
    Card,
    Thumb,
}

/// What a preview box holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreviewFill {
    /// A picture fills it.
    Picture,
    /// Nothing, or a glyph: the box takes an edge so it does not read as a
    /// failed image.
    Empty,
}

/// The box a preview sits in: the inset surface at the preview's size, with
/// a corner concentric with the tile around it.
pub(crate) fn preview_frame(size: PreviewSize, fill: PreviewFill) -> View {
    let frame = view()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(radius::INSET))
        .bg(colors::INSET);
    let frame = match size {
        PreviewSize::Card => frame.w_full().h(px(PREVIEW_H)),
        PreviewSize::Thumb => frame.w(px(THUMB_W)).h(px(THUMB_H)),
    };
    match fill {
        PreviewFill::Empty => frame.border_1().border_color(colors::BORDER),
        PreviewFill::Picture => frame,
    }
}

/// A picture inside a [`preview_frame`]: NSH reads and downsamples the file
/// off the UI thread and caches it; a plain path is a valid source.
pub(crate) fn preview_image(source: String, size: PreviewSize) -> impl IntoElement {
    let picture = image(source)
        .flex_shrink_0()
        .bg(colors::INSET)
        .rounded(px(radius::INSET))
        .object_fit(ResizeMode::Cover);
    match size {
        PreviewSize::Card => picture.w_full().h(px(PREVIEW_H)),
        PreviewSize::Thumb => picture.w(px(THUMB_W)).h(px(THUMB_H)),
    }
}

/// What a preview without a picture stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreviewGlyph {
    /// A file with no preview of its own.
    Document,
    /// A clip whose poster frame has not landed yet.
    Clip,
}

/// The glyph of a preview that has no picture.
pub(crate) fn preview_glyph(kind: PreviewGlyph) -> impl IntoElement {
    let markup = match kind {
        PreviewGlyph::Document => icons::FILE,
        PreviewGlyph::Clip => icons::PLAY,
    };
    icon(markup, size::ICON_LARGE, colors::MUTED)
}

/// A preview a pointer can press to play: the clip's own glyph or poster
/// is the larger target. It is a pointer convenience only — the Play button
/// in the copy's controls is the named, keyboard-reachable way to the same
/// action — so it dims on hover like a control without claiming a role.
pub(crate) fn press_to_play(preview: View, on_press: impl Fn() + 'static) -> View {
    preview
        .transition_colors()
        .duration(motion(timing::FAST))
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.8))
        .on_press(on_press)
}

/// The inline, muted player a playing clip opens into, without nsh's own
/// control bar.
///
/// That bar is the only thing the Windows backend reads the media engine for
/// while painting, and pausing a clip happens from inside the engine's own
/// mutable borrow: Media Foundation then fires `MF_MEDIA_ENGINE_EVENT_PAUSE`,
/// which invalidates the surface (`nsh-platform-windows` video.rs:345) and
/// repaints straight into `draw_bar`'s `playing()` read (video.rs:507) with
/// the borrow still held — a hard panic ("RefCell already mutably borrowed")
/// on every stop, replace or layout switch. Play and Stop live in the row
/// actions and on the card's Stop button, so the bar was redundant as well as
/// unsafe.
pub(crate) fn video_player(source: String) -> impl IntoElement {
    video(source)
        .w_full()
        .h(px(PREVIEW_H))
        .rounded(px(radius::INSET))
        .muted(true)
        .controls(false)
}

/// A disclosure card: the whole header is the trigger (one large target
/// instead of a 16 pt chevron), it reports its expanded state to screen
/// readers, and the body springs open and shut. The card's 4 pt inset puts
/// the header's hover on a concentric corner.
pub(crate) fn disclosure(
    open: bool,
    on_toggle: impl Fn() + 'static,
    header: impl IntoElement + 'static,
    body: View,
) -> View {
    card().p(px(space::XS)).child(
        collapsible()
            .open(open)
            .reveal(true)
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .accessibility_state(move || AccessibilityState {
                        expanded: Some(open),
                        ..AccessibilityState::default()
                    })
                    .on_press(on_toggle)
                    .w_full()
                    .flex_row()
                    .items_center()
                    .gap(px(space::SM))
                    .p(px(space::SM))
                    .rounded(px(radius::INSET))
                    .transition_colors()
                    .duration(motion(timing::FAST))
                    .hover(|s| s.bg(colors::PANE))
                    .active(|s| s.bg(colors::ELEVATED))
                    .focus_visible(focus_ring)
                    .child(chevron(open))
                    .child(header),
            )
            .content(body.px(px(space::SM)).pb(px(space::SM))),
    )
}

// ---------------------------------------------------------------------------
// Choice controls.
// ---------------------------------------------------------------------------

/// The corner of a segment inside its track: the track's radius less its
/// inset, so the two curves stay concentric.
const SEGMENT_RADIUS: f32 = radius::INSET;

/// The height of a segment: the track is a control tall, less its inset.
const SEGMENT_HEIGHT: f32 = size::CONTROL - 2.0 * space::XXS;

/// The groove a segmented control's options sit in: one silhouette at
/// control height, so "pick one of these" reads as one control and not as a
/// row of separate buttons.
fn segment_track() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::XXS))
        .p(px(space::XXS))
        .bg(colors::INSET)
        .rounded(px(radius::CONTROL))
}

/// Fill, border and ink for a segment: the chosen one is the card lifted out
/// of the groove in ink, the others sit in the groove in the secondary ink.
/// Selection is a place, not a colour.
fn selection(active: bool) -> (AdaptiveColor, AdaptiveColor, AdaptiveColor) {
    if active {
        (colors::CARD, colors::BORDER, colors::TEXT)
    } else {
        (colors::INSET, colors::INSET, colors::MUTED)
    }
}

/// The label of a labelled segment.
fn segment_label(label: &'static str, active: bool) -> Text {
    let (_, _, ink) = selection(active);
    text(label)
        .text_size(px(text::BODY))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink)
}

/// One of a few views or rules, on nsh's toggle group (a press releases the
/// others and takes the new one): the keeper rule, the sort. Only the press
/// that takes a segment counts — releasing the chosen one would leave the
/// set with nothing picked.
pub(crate) fn segmented<const N: usize>(
    labels: [&'static str; N],
    selected: usize,
    on_pick: impl Fn(usize) + Clone + 'static,
) -> View {
    let mut track = segment_track();
    for (index, label) in labels.into_iter().enumerate() {
        let active = index == selected;
        let (fill, border, _) = selection(active);
        let pick = on_pick.clone();
        track = track.child(
            toggle()
                .pressed(active)
                .on_change(move |next: &bool| {
                    if *next {
                        pick(index);
                    }
                })
                .accessibility_label(label)
                .flex_row()
                .items_center()
                .justify_center()
                .h(px(SEGMENT_HEIGHT))
                .px(px(space::SM + space::XXS))
                .rounded(px(SEGMENT_RADIUS))
                .bg(fill)
                .border_1()
                .border_color(border)
                .transition_colors()
                .duration(motion(timing::FAST))
                .hover(move |s| if active { s } else { s.bg(colors::ELEVATED) })
                .active(|s| s.opacity(0.9).scale(0.97))
                .focus_visible(focus_ring)
                .child(segment_label(label, active)),
        );
    }
    view().child(toggle_group().child(track))
}

/// [`segmented`] with glyphs for labels: the Grid/List switcher. Each glyph's
/// name is its accessible name and its tooltip.
pub(crate) fn icon_segmented<const N: usize>(
    items: [(&'static str, &'static str); N],
    selected: usize,
    on_pick: impl Fn(usize) + Clone + 'static,
) -> View {
    let mut track = segment_track();
    for (index, (markup, label)) in items.into_iter().enumerate() {
        let active = index == selected;
        let (fill, border, ink) = selection(active);
        let pick = on_pick.clone();
        track = track.child(
            view()
                .child(
                    toggle()
                        .pressed(active)
                        .on_change(move |next: &bool| {
                            if *next {
                                pick(index);
                            }
                        })
                        .accessibility_label(label)
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .w(px(SEGMENT_HEIGHT + space::XS))
                        .h(px(SEGMENT_HEIGHT))
                        .rounded(px(SEGMENT_RADIUS))
                        .bg(fill)
                        .border_1()
                        .border_color(border)
                        .transition_colors()
                        .duration(motion(timing::FAST))
                        .hover(move |s| if active { s } else { s.bg(colors::ELEVATED) })
                        .active(|s| s.opacity(0.9).scale(0.95))
                        .focus_visible(focus_ring)
                        .child(icon(markup, size::ICON_SMALL, ink)),
                )
                .tooltip(hint(label)),
        );
    }
    view().child(toggle_group().child(track))
}

/// One of a few options of a setting, on the platform radio semantics
/// (arrows move within the set, screen readers hear "1 of 4").
pub(crate) fn radio_segmented<const N: usize>(
    labels: [&'static str; N],
    selected: usize,
    on_pick: impl Fn(usize) + Clone + 'static,
) -> View {
    let mut track = segment_track();
    for (index, label) in labels.into_iter().enumerate() {
        let active = index == selected;
        let (fill, border, _) = selection(active);
        let pick = on_pick.clone();
        track = track.child(
            radio()
                .checked(active)
                .on_change(move |_: &bool| pick(index))
                .accessibility_label(label)
                .flex_row()
                .items_center()
                .justify_center()
                .h(px(SEGMENT_HEIGHT))
                .px(px(space::SM + space::XXS))
                .rounded(px(SEGMENT_RADIUS))
                .bg(fill)
                .border_1()
                .border_color(border)
                .transition_colors()
                .duration(motion(timing::FAST))
                .hover(move |s| if active { s } else { s.bg(colors::ELEVATED) })
                .active(|s| s.opacity(0.9).scale(0.97))
                .focus_visible(focus_ring)
                .child(segment_label(label, active)),
        );
    }
    view().flex_row().child(radio_group().child(track))
}

/// A whole number with a stepper either side, field-shaped: the similarity
/// floor, thread counts, thresholds. `on_change` gets what was typed or
/// stepped; an empty field is `None`.
pub(crate) fn number_field(
    placeholder: &'static str,
    value: impl Fn() -> Option<f64> + 'static,
    on_change: impl Fn(Option<f64>) + 'static,
    range: (f64, f64),
    width: f32,
) -> View {
    view().w(px(width)).child(
        number_input()
            .value(value)
            .min(range.0)
            .max(range.1)
            .step(1.0)
            .placeholder(placeholder)
            .on_change(move |next: &Option<f64>| on_change(*next))
            .input_style(|s| {
                s.flex_1()
                    .text_center()
                    .text_color(colors::TEXT)
                    .tabular_nums()
            })
            .decrement_button(text("−").text_color(colors::MUTED))
            .increment_button(text("+").text_color(colors::MUTED))
            .decrement_button_style(|s| s.w(px(size::COMPACT)).items_center().justify_center())
            .increment_button_style(|s| s.w(px(size::COMPACT)).items_center().justify_center())
            .flex_row()
            .h(px(size::CONTROL))
            .bg(colors::CARD)
            .border_1()
            .border_color(colors::CONTROL_BORDER)
            .rounded(px(radius::CONTROL))
            .overflow_hidden(),
    )
}

/// A number as the text a field holds: whole numbers without a decimal
/// point, nothing for an empty field.
#[must_use]
pub(crate) fn number_text(value: Option<f64>) -> String {
    match value {
        Some(value) if value.fract() == 0.0 => format!("{}", value as i64),
        Some(value) => format!("{value}"),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Navigation and rows.
// ---------------------------------------------------------------------------

/// The areas of a side pane as nsh's tabs: the chosen one a neutral pill
/// (the same one the rail uses), the others in the secondary ink.
pub(crate) fn side_tabs<T: Copy + PartialEq + 'static>(
    items: &[(&'static str, T)],
    current: T,
    on_pick: fn(T),
) -> impl IntoElement {
    let mut list = tabs()
        .w_full()
        .gap(px(space::XXS))
        .indicator_class("inset-x-0")
        .indicator_style(|s| s.rounded(px(radius::CONTROL)).bg(colors::ELEVATED));
    for &(label, which) in items {
        let active = which == current;
        list = list.child(
            tab()
                .selected(active)
                .on_press(move |_: &()| on_pick(which))
                .accessibility_label(label)
                .w_full()
                .min_h(px(size::CONTROL))
                .justify_center()
                .px(px(space::SM))
                .rounded(px(radius::CONTROL))
                .transition_colors()
                .duration(motion(timing::FAST))
                .hover(move |s| if active { s } else { s.bg(colors::PANE) })
                .focus_visible(focus_ring)
                .child(
                    text(label)
                        .text_size(px(text::BODY))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if active { colors::TEXT } else { colors::MUTED }),
                ),
        );
    }
    list
}

/// A rail slot's side: a 20 pt glyph with 8 pt around it.
const RAIL_SLOT: f32 = 36.0;

/// One slot of the icon rail: the active view's slot is a neutral pill.
/// The rail is icons only, so the pointer gets the name too, not just the
/// screen reader.
pub(crate) fn rail_button(
    markup: &'static str,
    label: &'static str,
    active: bool,
    on_press: fn(),
) -> View {
    view().tooltip(hint(label)).child(
        button()
            .on_press(on_press)
            .accessibility_label(label)
            .flex_row()
            .items_center()
            .justify_center()
            .w(px(RAIL_SLOT))
            .h(px(RAIL_SLOT))
            .rounded(px(radius::CONTROL))
            .bg(if active {
                colors::ELEVATED
            } else {
                colors::BACKGROUND
            })
            .transition_colors()
            .duration(motion(timing::BASE))
            .ease_out()
            .hover(move |s| if active { s } else { s.bg(colors::PANE) })
            .active(|s| s.bg(colors::ELEVATED).scale(0.95))
            .focus_visible(focus_ring)
            .disabled_style(|s| s.opacity(0.45))
            .hit_slop(EdgeInsets::all(px(size::HIT_SLOP)))
            .child(icon(
                markup,
                size::ICON_LARGE,
                if active { colors::TEXT } else { colors::MUTED },
            )),
    )
}

/// Whether a control can be pressed right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Availability {
    Ready,
    Disabled,
}

/// A full-width quiet action in a side pane: glyph and label. A disabled row
/// is dimmed, loses its hover and refuses presses; its name stays.
pub(crate) fn row_button(
    markup: &'static str,
    label: &'static str,
    availability: Availability,
    on_press: impl Fn() + 'static,
) -> View {
    let enabled = availability == Availability::Ready;
    view().child(
        button()
            .on_press(on_press)
            .disabled(!enabled)
            .accessibility_label(label)
            .flex_row()
            .items_center()
            .gap(px(space::SM))
            .w_full()
            .min_h(px(size::CONTROL))
            .px(px(space::SM))
            .rounded(px(radius::CONTROL))
            .transition_colors()
            .duration(motion(timing::FAST))
            .ease_out()
            .hover(move |s| if enabled { s.bg(colors::PANE) } else { s })
            .active(|s| s.bg(colors::ELEVATED))
            .focus_visible(focus_ring)
            .disabled_style(|s| s.opacity(0.45))
            .child(icon(markup, size::ICON, colors::MUTED))
            .child(body(label).flex_1()),
    )
}

/// How much a list row stands out from its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowEmphasis {
    Plain,
    /// Lifted onto the pane surface: a protected folder.
    Lifted,
}

/// A row in a side pane's list, answering hover.
pub(crate) fn list_row(emphasis: RowEmphasis) -> View {
    let marked = emphasis == RowEmphasis::Lifted;
    view()
        .flex_row()
        .items_center()
        .gap(px(space::SM))
        .px(px(space::SM))
        .py(px(space::XS + space::XXS))
        .rounded(px(radius::CONTROL))
        .bg(if marked {
            colors::PANE
        } else {
            colors::SIDEBAR
        })
        .transition_colors()
        .duration(motion(timing::FAST))
        .hover(|s| s.bg(colors::PANE))
}

/// A small icon action inside a [`list_row`]; `ink` is the glyph's, so a
/// closed lock reads in ink and an open one recedes.
pub(crate) fn row_icon_button(
    markup: &'static str,
    label: &'static str,
    ink: Ink,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .flex_shrink_0()
        .child(
            button()
                .on_press(on_press)
                .accessibility_label(label)
                .flex_row()
                .items_center()
                .justify_center()
                .w(px(size::COMPACT))
                .h(px(size::COMPACT))
                .rounded(px(radius::CONTROL))
                .transition_colors()
                .duration(motion(timing::FAST))
                .ease_out()
                .hover(|s| s.bg(colors::ELEVATED))
                .control_states()
                .hit_slop(EdgeInsets::all(px(size::HIT_SLOP)))
                .child(icon(markup, size::ICON_SMALL, ink.color())),
        )
        .tooltip(hint(label))
}

// ---------------------------------------------------------------------------
// Bars, progress and the confirmation dialog.
// ---------------------------------------------------------------------------

/// A toolbar of command groups: nsh's toolbar gives the whole bar one roving
/// keyboard stop, and arrows move within it. The first group holds the
/// leading edge, the last the trailing one; the bar wraps rather than
/// overflowing the pane.
pub(crate) fn action_bar(label: &'static str, groups: Vec<View>) -> impl IntoElement {
    let mut bar = toolbar()
        .accessibility_label(label)
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .w_full()
        .gap(px(space::MD));
    for group in groups {
        bar = bar.child(
            toolbar_group()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(space::SM))
                .child(group),
        );
    }
    bar
}

/// A row of controls inside an [`action_bar`] group.
pub(crate) fn action_group() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::SM))
}

/// The progress bar's thickness; its ends are fully rounded.
const PROGRESS_H: f32 = 6.0;

/// A scan's progress: a determinate bar glides between ticks instead of
/// stepping; an indeterminate one pulses.
pub(crate) fn progress_bar(fraction: f32, determinate: bool) -> impl IntoElement {
    progress()
        .value(fraction * 100.0)
        .indeterminate(!determinate)
        .accessibility_label("Scan progress")
        .track_style(|s| {
            s.h(px(PROGRESS_H))
                .w_full()
                .rounded(px(PROGRESS_H / 2.0))
                .bg(colors::INSET)
                .overflow_hidden()
        })
        .indicator_class(if determinate {
            "transition-all"
        } else {
            "w-full animate-pulse"
        })
        .indicator_style(|s| {
            s.h(px(PROGRESS_H))
                .rounded(px(PROGRESS_H / 2.0))
                .bg(colors::PRIMARY)
                .duration(motion(timing::SLOW))
        })
}

/// The confirmation dialog's width: a full Windows path at caption size
/// beside its thumbnail, size and quiet buttons without wrapping.
const DIALOG_W: f32 = 540.0;

/// A confirmation that interrupts: nsh's dialog owns the modal layer (focus
/// moves in, Escape and back arrive as `on_cancel`, screen readers hear an
/// alert dialog named by its title); the look is the app's.
///
/// The scrim sits in the page layer, below the dialog, not inside it: the
/// dialog's own entrance and exit slide its whole layer, so a scrim in there
/// travelled down the screen with the panel. Out here it fades in place and
/// only the panel moves. A press that misses the panel cancels.
pub(crate) fn alert_dialog(
    title: String,
    summary: String,
    content: impl IntoElement + 'static,
    actions: View,
    on_cancel: fn(),
) -> View {
    // The one surface here that floats above the page, so the one that
    // casts a shadow.
    let panel = card()
        .w(px(DIALOG_W))
        .flex_shrink_0()
        .gap(px(space::LG))
        .p(px(space::XL))
        .shadow_md()
        .child(
            view()
                .flex_col()
                .gap(px(space::XS))
                .child(section_title(title.clone()))
                .child(description(summary)),
        )
        .child(content)
        .child(divider())
        .child(actions.flex_row().justify_end().gap(px(space::SM)));
    let scrim = fading_in();
    view()
        .child(
            overlay()
                .inset_0()
                .bg(colors::SCRIM)
                .on_press(on_cancel)
                .bind_motion(&scrim),
        )
        .child(
            dialog()
                .role(AccessibilityRole::AlertDialog)
                .label(title)
                .on_cancel(on_cancel)
                .child(overlay().inset_0().on_press(on_cancel))
                .child(
                    overlay()
                        .inset_0()
                        .pointer_events(PointerEvents::BoxNone)
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .p(px(space::XL))
                        .child(panel),
                ),
        )
}

/// One choice out of `items`, as nsh's Select: nsh owns the behaviour (the
/// list, the keys, typeahead, what screen readers hear), the app supplies
/// the look — its trigger, the popup's surface, the rows' resting text — and
/// nsh's own state classes give the highlighted and chosen rows their state.
pub(crate) fn option_select(
    items: Vec<SelectItem>,
    accessibility: &'static str,
    value: impl Fn() -> Option<SharedString> + 'static,
    on_change: impl Fn(&Option<SharedString>) + 'static,
    trigger: View,
) -> impl IntoElement {
    select()
        .items(items)
        .value(value)
        .on_change(on_change)
        .accessibility_label(accessibility)
        // The list hangs just under the trigger rather than touching it.
        .offset(px(space::XS))
        .popup_style(popup_surface)
        // `focus:` marks the row the keys or the pointer are on, `selected:`
        // the choice in effect: nsh applies them, the app only names them.
        .item_class("focus:bg-accent selected:font-medium")
        .item_style(popup_item)
        .trigger(trigger)
        .w(px(SELECT_W))
}

/// The width of every select on the setup form, so their right edges line up.
pub(crate) const SELECT_W: f32 = 240.0;

/// The surface every popup list shares: a card that floats, so it alone
/// takes a shadow.
fn popup_surface(s: StyleRefinement) -> StyleRefinement {
    s.min_w(px(SELECT_W))
        .p(px(space::XS))
        .bg(colors::CARD)
        .border_1()
        .border_color(colors::BORDER)
        .rounded(px(radius::CARD))
        .shadow_md()
}

/// A row of a popup list, inset concentrically inside the popup.
fn popup_item(s: StyleRefinement) -> StyleRefinement {
    s.px(px(space::SM))
        .py(px(space::XS + space::XXS))
        .rounded(px(radius::INSET))
        .text_size(px(text::BODY))
        .text_color(colors::TEXT)
}

/// The trigger a Select opens from: a field-shaped control (icon, current
/// choice, a chevron at the far end) as the combobox that takes the press.
pub(crate) fn dropdown_trigger(
    markup: &'static str,
    label: impl Fn() -> SharedString + 'static,
) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::SM))
        .h(px(size::CONTROL))
        .px(px(space::MD))
        .bg(colors::CARD)
        .border_1()
        .border_color(colors::CONTROL_BORDER)
        .rounded(px(radius::CONTROL))
        .transition_colors()
        .duration(motion(timing::BASE))
        .ease_out()
        .hover(|s| s.bg(colors::PANE))
        .child(icon(markup, size::ICON_SMALL, colors::MUTED))
        .child(
            text(label)
                .flex_1()
                .text_size(px(text::BODY))
                .text_color(colors::TEXT),
        )
        .child(icon(icons::CHEVRON_DOWN, size::ICON_SMALL, colors::MUTED))
}

/// The widest a tooltip grows before it wraps: about 50 characters.
const TOOLTIP_MAX_W: f32 = 320.0;

/// An info bubble for `.tooltip(..)`: ink in light theme, light in dark
/// theme, so it reads over any surface. The trigger's own label speaks for
/// screen readers; this hint is for the pointer.
pub(crate) fn hint(label: impl IntoReactive<SharedString>) -> impl IntoTooltip {
    tooltip()
        .rounded(px(radius::CONTROL))
        .px(px(space::SM))
        .py(px(space::XS))
        .max_w(px(TOOLTIP_MAX_W))
        .bg(colors::TEXT)
        .child(
            text(label)
                .text_size(px(text::CAPTION))
                .text_color(colors::BACKGROUND),
        )
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
