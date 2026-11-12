//! Design tokens: one calm, warm workspace system, light and dark.
//!
//! Surfaces are opaque solids separated by a soft hairline; depth comes from
//! the surface ramp itself (window → sidebar → card → pane → elevated) and
//! from the bezel: a tinted outer tray with the card surface nested inside it
//! on a concentric radius. The dark appearance is a deep warm near-black, the
//! light one a warm paper, and the accent is the app's blue per appearance.

use nsh_core::element::{AdaptiveColor, Rgba};

/// A translucent sRGB colour: the alpha is what makes a surface read as glass.
const fn veil(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: a as f32 / 255.0,
    }
}

const fn tone(light: Rgba, dark: Rgba) -> AdaptiveColor {
    AdaptiveColor {
        light: light.to_hsla(),
        dark: dark.to_hsla(),
    }
}

pub(crate) mod colors {
    use super::{AdaptiveColor, tone, veil};

    /// The window itself, behind everything else.
    pub(crate) const BACKGROUND: AdaptiveColor =
        tone(veil(0xf5, 0xf3, 0xef, 255), veil(0x0a, 0x0b, 0x0b, 255));
    /// List pane next to the window: one step up from the background.
    pub(crate) const SIDEBAR: AdaptiveColor =
        tone(veil(0xfb, 0xf9, 0xf5, 255), veil(0x10, 0x11, 0x12, 255));
    /// Card surface: group cards, the top bar, the status bar, dialogs.
    pub(crate) const CARD: AdaptiveColor =
        tone(veil(0xff, 0xff, 0xff, 255), veil(0x17, 0x19, 0x1a, 255));
    /// A row that sits on a card (list rows, secondary buttons): one step
    /// deeper so two stacked surfaces stay legible.
    pub(crate) const PANE: AdaptiveColor =
        tone(veil(0xf2, 0xef, 0xe9, 255), veil(0x1f, 0x21, 0x23, 255));
    /// The hover step: one more tint above a pane.
    pub(crate) const ELEVATED: AdaptiveColor =
        tone(veil(0xe9, 0xe5, 0xdd, 255), veil(0x29, 0x2b, 0x2d, 255));
    /// Wells and fields: punched in rather than raised.
    pub(crate) const INSET: AdaptiveColor =
        tone(veil(0xee, 0xeb, 0xe5, 255), veil(0x0d, 0x0e, 0x0f, 255));
    /// The outer tray of a bezelled surface: a veil over the background, so a
    /// card reads as a pane of glass seated in a tray.
    pub(crate) const SHELL: AdaptiveColor =
        tone(veil(0x00, 0x00, 0x00, 8), veil(0xff, 0xff, 0xff, 10));
    /// Separators and outlines: a soft hairline, never a shadow.
    pub(crate) const BORDER: AdaptiveColor =
        tone(veil(0xe4, 0xdf, 0xd6, 255), veil(0x2b, 0x2e, 0x30, 255));
    /// One step softer than [`BORDER`]: the seam of a tray and the edge of a
    /// raised control, so a container reads as a join, not a frame.
    pub(crate) const HAIRLINE: AdaptiveColor =
        tone(veil(0x00, 0x00, 0x00, 12), veil(0xff, 0xff, 0xff, 14));
    pub(crate) const TEXT: AdaptiveColor =
        tone(veil(0x1c, 0x1b, 0x18, 255), veil(0xed, 0xee, 0xef, 255));
    /// Secondary copy. Dark enough in the light appearance to hold 4.5:1 on
    /// the paper, the card and a pane: at the old `#74706a` captions measured
    /// 4.2:1 on `BACKGROUND` and 4.1:1 on `PANE`.
    pub(crate) const MUTED: AdaptiveColor =
        tone(veil(0x6b, 0x67, 0x61, 255), veil(0x8a, 0x8f, 0x8c, 255));
    /// A deep blue in light; a soft, bright blue in dark, with dark-on-blue
    /// contrast for filled controls. The light step is dark enough that white
    /// label text on a filled pill stays past 5:1.
    pub(crate) const ACCENT: AdaptiveColor =
        tone(veil(0x0b, 0x63, 0xd8, 255), veil(0x74, 0xb4, 0xff, 255));
    pub(crate) const ON_ACCENT: AdaptiveColor =
        tone(veil(0xff, 0xff, 0xff, 255), veil(0x08, 0x1c, 0x33, 255));
    pub(crate) const ACCENT_WASH: AdaptiveColor =
        tone(veil(0x0b, 0x63, 0xd8, 18), veil(0x74, 0xb4, 0xff, 34));
    pub(crate) const ACCENT_WASH_STRONG: AdaptiveColor =
        tone(veil(0x0b, 0x63, 0xd8, 32), veil(0x74, 0xb4, 0xff, 30));
    /// Text and glyphs on an accent tint. `ACCENT` itself only works on a
    /// plain surface, so a status pill or an active tab uses this instead: a
    /// deeper blue in light, a pale one in dark, both past 4.5:1 on
    /// `ACCENT_WASH_STRONG`.
    pub(crate) const ACCENT_INK: AdaptiveColor =
        tone(veil(0x0a, 0x46, 0x96, 255), veil(0xcf, 0xe6, 0xff, 255));
    /// The keeper of a group. Green, deliberately not the accent blue used
    /// for primary actions, so "this copy is kept" can never be confused
    /// with "this copy is ticked for deletion".
    pub(crate) const KEEP: AdaptiveColor =
        tone(veil(0x1d, 0x9a, 0x4a, 255), veil(0x32, 0xd7, 0x4b, 255));
    pub(crate) const KEEP_WASH: AdaptiveColor =
        tone(veil(0x1d, 0x9a, 0x4a, 22), veil(0x32, 0xd7, 0x4b, 40));
    pub(crate) const KEEP_WASH_STRONG: AdaptiveColor =
        tone(veil(0x1d, 0x9a, 0x4a, 44), veil(0x32, 0xd7, 0x4b, 72));
    /// `KEEP` as words on a keep wash. The tint is a backing, not a label
    /// colour: green under a 44% wash measured 3.3:1, so the KEEP badge takes
    /// this instead (6:1 on the wash).
    pub(crate) const KEEP_INK: AdaptiveColor =
        tone(veil(0x0f, 0x6b, 0x31, 255), veil(0x6f, 0xf0, 0x8b, 255));
    /// A copy ticked for deletion. Amber: a second decision, a second hue.
    pub(crate) const SELECT: AdaptiveColor =
        tone(veil(0xc2, 0x75, 0x00, 255), veil(0xff, 0x9f, 0x0a, 255));
    pub(crate) const SELECT_WASH: AdaptiveColor =
        tone(veil(0xff, 0x95, 0x00, 26), veil(0xff, 0x9f, 0x0a, 44));
    pub(crate) const SELECT_WASH_STRONG: AdaptiveColor =
        tone(veil(0xff, 0x95, 0x00, 50), veil(0xff, 0x9f, 0x0a, 76));
    /// `SELECT` as words on a tick wash: the amber is a fill and a border,
    /// never a label colour (it measured 3.4:1 on the light wash).
    pub(crate) const SELECT_INK: AdaptiveColor =
        tone(veil(0x8a, 0x52, 0x00, 255), veil(0xff, 0xc4, 0x66, 255));
    /// Error borders and marks. Not text: see [`DANGER_TEXT`].
    pub(crate) const DANGER: AdaptiveColor =
        tone(veil(0xff, 0x3b, 0x30, 255), veil(0xff, 0x45, 0x3a, 255));
    /// The fill of a destructive action: one deeper red in both appearances,
    /// so the label on it is white either way (5.8:1). The bright `DANGER`
    /// under white text measured 3.4:1, and carrying dark ink in the dark
    /// appearance made the one filled destructive control look disabled.
    pub(crate) const DANGER_SURFACE: AdaptiveColor =
        tone(veil(0xc4, 0x22, 0x1d, 255), veil(0xc4, 0x22, 0x1d, 255));
    /// The label on [`DANGER_SURFACE`].
    pub(crate) const ON_DANGER: AdaptiveColor =
        tone(veil(0xff, 0xff, 0xff, 255), veil(0xff, 0xff, 0xff, 255));
    /// Error copy on a plain surface: the fill colour is too light to read as
    /// words (3.3:1 on a card in light).
    pub(crate) const DANGER_TEXT: AdaptiveColor =
        tone(veil(0xc4, 0x22, 0x1d, 255), veil(0xff, 0x6b, 0x5e, 255));
    /// The veil behind a dialog: dims the app without hiding it.
    pub(crate) const SCRIM: AdaptiveColor =
        tone(veil(0x00, 0x00, 0x00, 64), veil(0x00, 0x00, 0x00, 150));
}

pub(crate) mod text {
    /// Eyebrows: the smallest step, for kickers and the upper-case name above
    /// a heading.
    pub(crate) const MICRO: f32 = 11.0;
    pub(crate) const CAPTION: f32 = 12.0;
    pub(crate) const BODY: f32 = 14.0;
    /// Page headings. Text scales with the user's text size by itself; only
    /// the step value lives here.
    pub(crate) const HEADING: f32 = 22.0;
    /// Page titles ("General", "Preferences"), one step above the heading.
    pub(crate) const PAGE: f32 = 30.0;
}

/// Corner radii: one finite scale instead of a value per element. A large
/// surface gets the softer tier, and the controls inside it take the tighter
/// one, so the nesting stays legible.
pub(crate) mod radius {
    /// The outer tray of a bezelled surface; its core takes the difference.
    pub(crate) const SHELL: f32 = 22.0;
    /// Cards, bars, floating panels and media cards.
    pub(crate) const CARD: f32 = 18.0;
    /// Inputs, rows, previews and small controls.
    pub(crate) const CONTROL: f32 = 12.0;
}

/// Spacing scale, the same relationships the guide names: optical correction,
/// parts of one control, closely related controls, one content group, separate
/// groups, separate sections, region boundaries.
pub(crate) mod space {
    pub(crate) const XXS: f32 = 2.0;
    pub(crate) const XS: f32 = 4.0;
    pub(crate) const SM: f32 = 8.0;
    pub(crate) const MD: f32 = 12.0;
    pub(crate) const LG: f32 = 16.0;
    pub(crate) const XL: f32 = 24.0;
    pub(crate) const XXL: f32 = 32.0;
    /// The gap between sections of one page: deliberately larger than a
    /// content group so the page breathes instead of stacking.
    pub(crate) const XXXL: f32 = 44.0;
}
