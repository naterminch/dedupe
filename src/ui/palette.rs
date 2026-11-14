//! Design tokens: an achromatic workspace, light and dark.
//!
//! The chrome is ink on a stone ground: no hue on navigation, buttons or
//! selection. Colour is spent on three meanings only — the one data hue on
//! the figure a screen answers with (the space a delete gives back), the
//! keep/tick pair on a group's copies, and red on destruction. Surfaces are
//! flat: a white card on the ground, separated by one line, never a shadow
//! (only what floats — a menu, a dialog, a toast — casts one).

use nsh_core::element::{AdaptiveColor, Rgba};

/// An sRGB colour with alpha, from bytes.
const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba {
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
    use super::{AdaptiveColor, rgba, tone};

    /// The window ground behind everything: a stone one level off white
    /// (saturation under 10%, so it reads as paper, not cream), and a warm
    /// near-black in dark.
    pub(crate) const BACKGROUND: AdaptiveColor =
        tone(rgba(0xf4, 0xf3, 0xf1, 255), rgba(0x0b, 0x0c, 0x0c, 255));
    /// The folder pane and the settings nav: the page's own ground, so the
    /// chrome recedes and only a rule separates it from the work.
    pub(crate) const SIDEBAR: AdaptiveColor = BACKGROUND;
    /// Cards, dialogs, menus, the status bar and text fields.
    pub(crate) const CARD: AdaptiveColor =
        tone(rgba(0xff, 0xff, 0xff, 255), rgba(0x15, 0x17, 0x18, 255));
    /// A surface that sits on a card or the ground: member tiles, secondary
    /// buttons, a hovered row.
    pub(crate) const PANE: AdaptiveColor =
        tone(rgba(0xee, 0xec, 0xe8, 255), rgba(0x1f, 0x21, 0x23, 255));
    /// One step above a pane: its hover, and the active navigation pill.
    pub(crate) const ELEVATED: AdaptiveColor =
        tone(rgba(0xe4, 0xe1, 0xdc, 255), rgba(0x2a, 0x2c, 0x2e, 255));
    /// Tracks: the segmented control's groove, the progress bar's rail,
    /// preview boxes before their image lands.
    pub(crate) const INSET: AdaptiveColor =
        tone(rgba(0xe9, 0xe7, 0xe3, 255), rgba(0x0f, 0x10, 0x11, 255));
    /// Card edges and outlines: the one line that separates two surfaces.
    pub(crate) const BORDER: AdaptiveColor =
        tone(rgba(0xe3, 0xe0, 0xdb, 255), rgba(0x27, 0x2a, 0x2c, 255));
    /// Rules inside a card, one step lighter than its edge, so a row reads
    /// as part of the card and not as a second box.
    pub(crate) const HAIRLINE: AdaptiveColor =
        tone(rgba(0x00, 0x00, 0x00, 15), rgba(0xff, 0xff, 0xff, 16));
    /// The outline of a field or a select trigger: one step darker than a
    /// card edge, so an input reads as something to type into.
    pub(crate) const CONTROL_BORDER: AdaptiveColor =
        tone(rgba(0xd3, 0xcf, 0xc9, 255), rgba(0x3a, 0x3d, 0x40, 255));
    pub(crate) const TEXT: AdaptiveColor =
        tone(rgba(0x1a, 0x1a, 0x18, 255), rgba(0xed, 0xed, 0xee, 255));
    /// Secondary copy. 5.5:1 on the ground and 6:1 on a card in light;
    /// 6.8:1 on the dark ground.
    pub(crate) const MUTED: AdaptiveColor =
        tone(rgba(0x65, 0x62, 0x5d, 255), rgba(0x9b, 0x9e, 0xa0, 255));
    /// The one action that moves the work forward: near-black in light,
    /// near-white in dark. The chrome carries no brand hue.
    pub(crate) const PRIMARY: AdaptiveColor = TEXT;
    /// The label on [`PRIMARY`].
    pub(crate) const ON_PRIMARY: AdaptiveColor =
        tone(rgba(0xff, 0xff, 0xff, 255), rgba(0x11, 0x12, 0x13, 255));
    /// The one hue: the screen's answer figure (space reclaimable) and the
    /// keyboard's focus ring. 6.1:1 on white; 8.4:1 on the dark card.
    pub(crate) const ACCENT: AdaptiveColor =
        tone(rgba(0x1d, 0x5f, 0xbf, 255), rgba(0x74, 0xb4, 0xff, 255));
    /// The keeper of a group: its border and its mark. Green, so "this copy
    /// stays" can never be read as "this copy is ticked".
    pub(crate) const KEEP: AdaptiveColor =
        tone(rgba(0x1d, 0x8a, 0x47, 255), rgba(0x32, 0xd7, 0x4b, 255));
    /// The pale backing of a keeper's badge; words on it take [`KEEP_INK`].
    pub(crate) const KEEP_WASH: AdaptiveColor =
        tone(rgba(0xdc, 0xf3, 0xe3, 255), rgba(0x14, 0x2e, 0x1b, 255));
    pub(crate) const KEEP_INK: AdaptiveColor =
        tone(rgba(0x0f, 0x5c, 0x2c, 255), rgba(0x8b, 0xf0, 0x9f, 255));
    /// A copy ticked for deletion: its border and its mark.
    pub(crate) const SELECT: AdaptiveColor =
        tone(rgba(0xc2, 0x75, 0x00, 255), rgba(0xff, 0x9f, 0x0a, 255));
    pub(crate) const SELECT_WASH: AdaptiveColor =
        tone(rgba(0xfd, 0xee, 0xd5, 255), rgba(0x33, 0x23, 0x0a, 255));
    pub(crate) const SELECT_INK: AdaptiveColor =
        tone(rgba(0x7a, 0x48, 0x00, 255), rgba(0xff, 0xc8, 0x70, 255));
    /// Error borders and marks. Not text: see [`DANGER_TEXT`].
    pub(crate) const DANGER: AdaptiveColor =
        tone(rgba(0xe0, 0x3a, 0x2f, 255), rgba(0xff, 0x45, 0x3a, 255));
    /// The fill of a destructive action, white label either way (5.8:1).
    pub(crate) const DANGER_SURFACE: AdaptiveColor =
        tone(rgba(0xc4, 0x22, 0x1d, 255), rgba(0xc4, 0x22, 0x1d, 255));
    /// The label on [`DANGER_SURFACE`].
    pub(crate) const ON_DANGER: AdaptiveColor =
        tone(rgba(0xff, 0xff, 0xff, 255), rgba(0xff, 0xff, 0xff, 255));
    /// Error copy on a plain surface.
    pub(crate) const DANGER_TEXT: AdaptiveColor =
        tone(rgba(0xb3, 0x1f, 0x1a, 255), rgba(0xff, 0x6b, 0x5e, 255));
    /// The veil behind a dialog: dims the app without hiding it.
    pub(crate) const SCRIM: AdaptiveColor =
        tone(rgba(0x00, 0x00, 0x00, 56), rgba(0x00, 0x00, 0x00, 150));
}

/// The type scale: five steps, 12/14/16/24/30. Text is set in points and the
/// engine applies the user's text size to all of it.
pub(crate) mod text {
    /// Meta: paths, counts, hints, badges.
    pub(crate) const CAPTION: f32 = 12.0;
    /// Body, rows, controls.
    pub(crate) const BODY: f32 = 14.0;
    /// Card and section titles, the wordmark.
    pub(crate) const TITLE: f32 = 16.0;
    /// The page title (one per screen).
    pub(crate) const HEADING: f32 = 24.0;
    /// The screen's answer figure, and nothing else.
    pub(crate) const FIGURE: f32 = 30.0;
}

/// Corner radii: the tight pair. Cards and floating surfaces take 8, the
/// controls inside them 6, so nested curves step down instead of competing.
pub(crate) mod radius {
    /// Cards, dialogs, menus, toasts, preview tiles.
    pub(crate) const CARD: f32 = 8.0;
    /// Buttons, fields, rows, badges.
    pub(crate) const CONTROL: f32 = 6.0;
    /// A surface inset 2–4 pt inside a card or a control: a segment in its
    /// groove, a group header's hover, a preview inside its tile. Keeps the
    /// two curves concentric.
    pub(crate) const INSET: f32 = 4.0;
}

/// Spacing scale, the relationships the guide names: optical correction,
/// parts of one control, closely related controls, one content group,
/// separate groups, separate sections, region boundaries.
pub(crate) mod space {
    pub(crate) const XXS: f32 = 2.0;
    pub(crate) const XS: f32 = 4.0;
    pub(crate) const SM: f32 = 8.0;
    pub(crate) const MD: f32 = 12.0;
    pub(crate) const LG: f32 = 16.0;
    pub(crate) const XL: f32 = 24.0;
    pub(crate) const XXL: f32 = 32.0;
}

/// Control frames, in points: every button, field, select and segmented
/// track in a row shares one height, so a toolbar lines up by construction.
pub(crate) mod size {
    /// The desktop default control height.
    pub(crate) const CONTROL: f32 = 32.0;
    /// A dense control inside a row (a member's tick, a row action).
    pub(crate) const COMPACT: f32 = 26.0;
    /// Glyphs: beside a label, in a dense control, in the rail and an empty
    /// preview.
    pub(crate) const ICON: f32 = 16.0;
    pub(crate) const ICON_SMALL: f32 = 14.0;
    pub(crate) const ICON_LARGE: f32 = 20.0;
    /// How far a small target's hit area reaches past its drawn edge.
    pub(crate) const HIT_SLOP: f32 = 4.0;
    /// The keyboard focus ring's width.
    pub(crate) const FOCUS_RING: f32 = 2.0;
}
