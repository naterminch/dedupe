//! Design tokens: one flat dark workspace system, light and dark.
//!
//! Surfaces are opaque solids separated by a one-pixel hairline; depth comes
//! from the surface ramp itself (window → card → pane → elevated) rather than
//! shadows or blur. The ramp is hue-neutral near-black in the dark appearance
//! and paper-grey in the light one, and the accent is a warm orange per
//! appearance.

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
        tone(veil(0xf4, 0xf4, 0xf5, 255), veil(0x0e, 0x0f, 0x10, 255));
    /// List pane next to the window: one step up from the background.
    pub(crate) const SIDEBAR: AdaptiveColor =
        tone(veil(0xfa, 0xfa, 0xfa, 255), veil(0x12, 0x13, 0x13, 255));
    /// Card surface: group cards, the top bar, the status bar, dialogs.
    pub(crate) const CARD: AdaptiveColor =
        tone(veil(0xff, 0xff, 0xff, 255), veil(0x1d, 0x1f, 0x20, 255));
    /// A row that sits on a card (list rows, secondary buttons): one step
    /// deeper so two stacked surfaces stay legible.
    pub(crate) const PANE: AdaptiveColor =
        tone(veil(0xf0, 0xf0, 0xf2, 255), veil(0x24, 0x26, 0x28, 255));
    /// The hover step: one more tint above a pane.
    pub(crate) const ELEVATED: AdaptiveColor =
        tone(veil(0xe8, 0xe8, 0xea, 255), veil(0x2c, 0x2e, 0x30, 255));
    /// Wells and fields: punched in rather than raised.
    pub(crate) const INSET: AdaptiveColor =
        tone(veil(0xf0, 0xf0, 0xf2, 255), veil(0x14, 0x15, 0x16, 255));
    /// Separators and outlines: a solid hairline grey, never a shadow.
    pub(crate) const BORDER: AdaptiveColor =
        tone(veil(0xe2, 0xe2, 0xe4, 255), veil(0x3a, 0x3c, 0x3a, 255));
    pub(crate) const TEXT: AdaptiveColor =
        tone(veil(0x1d, 0x1d, 0x1f, 255), veil(0xe4, 0xe4, 0xe6, 255));
    pub(crate) const MUTED: AdaptiveColor =
        tone(veil(0x6e, 0x6e, 0x73, 255), veil(0x7f, 0x81, 0x81, 255));
    /// A deep orange in light; a soft, warm orange in dark, with
    /// dark-on-orange contrast for filled controls. The light step is dark
    /// enough that white label text on a filled pill stays legible.
    pub(crate) const ACCENT: AdaptiveColor =
        tone(veil(0xc0, 0x56, 0x0f, 255), veil(0xf2, 0xa6, 0x5f, 255));
    pub(crate) const ON_ACCENT: AdaptiveColor =
        tone(veil(0xff, 0xff, 0xff, 255), veil(0x1a, 0x14, 0x10, 255));
    pub(crate) const ACCENT_WASH: AdaptiveColor =
        tone(veil(0xc0, 0x56, 0x0f, 20), veil(0xf2, 0xa6, 0x5f, 38));
    pub(crate) const ACCENT_WASH_STRONG: AdaptiveColor =
        tone(veil(0xc0, 0x56, 0x0f, 36), veil(0xf2, 0xa6, 0x5f, 64));
    /// The keeper of a group. Green, deliberately not the accent blue used
    /// for primary actions, so "this copy is kept" can never be confused
    /// with "this copy is ticked for deletion".
    pub(crate) const KEEP: AdaptiveColor =
        tone(veil(0x1d, 0x9a, 0x4a, 255), veil(0x32, 0xd7, 0x4b, 255));
    pub(crate) const KEEP_WASH: AdaptiveColor =
        tone(veil(0x1d, 0x9a, 0x4a, 22), veil(0x32, 0xd7, 0x4b, 40));
    pub(crate) const KEEP_WASH_STRONG: AdaptiveColor =
        tone(veil(0x1d, 0x9a, 0x4a, 44), veil(0x32, 0xd7, 0x4b, 72));
    /// A copy ticked for deletion. Amber: a second decision, a second hue.
    pub(crate) const SELECT: AdaptiveColor =
        tone(veil(0xc2, 0x75, 0x00, 255), veil(0xff, 0x9f, 0x0a, 255));
    pub(crate) const SELECT_WASH: AdaptiveColor =
        tone(veil(0xff, 0x95, 0x00, 26), veil(0xff, 0x9f, 0x0a, 44));
    pub(crate) const SELECT_WASH_STRONG: AdaptiveColor =
        tone(veil(0xff, 0x95, 0x00, 50), veil(0xff, 0x9f, 0x0a, 76));
    pub(crate) const DANGER: AdaptiveColor =
        tone(veil(0xff, 0x3b, 0x30, 255), veil(0xff, 0x45, 0x3a, 255));
    /// The veil behind a dialog: dims the app without hiding it.
    pub(crate) const SCRIM: AdaptiveColor =
        tone(veil(0x00, 0x00, 0x00, 64), veil(0x00, 0x00, 0x00, 150));
}

pub(crate) mod text {
    pub(crate) const CAPTION: f32 = 12.0;
    pub(crate) const BODY: f32 = 14.0;
    /// Page headings. Text scales with the user's text size by itself; only
    /// the step value lives here.
    pub(crate) const HEADING: f32 = 20.0;
    /// Page titles ("General", "Preferences"), one step above the heading.
    pub(crate) const PAGE: f32 = 28.0;
}

/// Corner radii: one finite scale instead of a value per element. A large
/// surface gets the softer tier, and the controls inside it take the tighter
/// one, so the nesting stays legible.
pub(crate) mod radius {
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
}
