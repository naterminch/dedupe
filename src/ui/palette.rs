//! Design tokens: one glass system, light and dark.
//!
//! Every surface is a translucent tone over the window background rather than
//! an opaque grey, so stacked layers read as depth (a pane in a tray) instead
//! of adjacent rectangles. Hairlines are alpha black/white, never a solid grey,
//! and the accent is Apple's system blue per appearance.

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
        tone(veil(0xf2, 0xf2, 0xf7, 255), veil(0x0b, 0x0b, 0x0d, 255));
    /// Frosted surface: cards, the top bar, the status bar, floating toasts.
    pub(crate) const GLASS: AdaptiveColor =
        tone(veil(0xff, 0xff, 0xff, 214), veil(0x1c, 0x1c, 0x1e, 196));
    /// A pane that sits on glass (secondary buttons, list rows): one step
    /// deeper so two stacked surfaces stay legible.
    pub(crate) const PANE: AdaptiveColor =
        tone(veil(0xff, 0xff, 0xff, 150), veil(0x2c, 0x2c, 0x2e, 160));
    /// Wells and fields: punched in rather than raised.
    pub(crate) const INSET: AdaptiveColor =
        tone(veil(0x00, 0x00, 0x00, 20), veil(0xff, 0xff, 0xff, 18));
    /// Separators and outlines: a hairline, never a hard grey rule.
    pub(crate) const BORDER: AdaptiveColor =
        tone(veil(0x00, 0x00, 0x00, 26), veil(0xff, 0xff, 0xff, 34));
    pub(crate) const TEXT: AdaptiveColor =
        tone(veil(0x1d, 0x1d, 0x1f, 255), veil(0xf5, 0xf5, 0xf7, 255));
    pub(crate) const MUTED: AdaptiveColor =
        tone(veil(0x6e, 0x6e, 0x73, 255), veil(0x9a, 0x9a, 0xa0, 255));
    /// System blue.
    pub(crate) const ACCENT: AdaptiveColor =
        tone(veil(0x00, 0x71, 0xe3, 255), veil(0x0a, 0x84, 0xff, 255));
    pub(crate) const ON_ACCENT: AdaptiveColor =
        tone(veil(0xff, 0xff, 0xff, 255), veil(0xff, 0xff, 0xff, 255));
    pub(crate) const ACCENT_WASH: AdaptiveColor =
        tone(veil(0x00, 0x71, 0xe3, 20), veil(0x0a, 0x84, 0xff, 38));
    pub(crate) const ACCENT_WASH_STRONG: AdaptiveColor =
        tone(veil(0x00, 0x71, 0xe3, 36), veil(0x0a, 0x84, 0xff, 64));
    pub(crate) const DANGER: AdaptiveColor =
        tone(veil(0xff, 0x3b, 0x30, 255), veil(0xff, 0x45, 0x3a, 255));
    /// Ambient light under raised surfaces. Translucent, so a shadow never
    /// turns into a dark smear on the glass.
    pub(crate) const SHADOW: AdaptiveColor =
        tone(veil(0x00, 0x00, 0x00, 40), veil(0x00, 0x00, 0x00, 150));
}

pub(crate) mod text {
    pub(crate) const CAPTION: f32 = 12.0;
    pub(crate) const BODY: f32 = 14.0;
    /// Page headings. Text scales with the user's text size by itself; only
    /// the step value lives here.
    pub(crate) const HEADING: f32 = 20.0;
}

/// Corner radii: one finite scale instead of a value per element. A large
/// surface gets the softer tier, and the controls inside it take the tighter
/// one, so the nesting stays legible.
pub(crate) mod radius {
    /// Cards, bars, floating panels and media cards.
    pub(crate) const CARD: f32 = 16.0;
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
