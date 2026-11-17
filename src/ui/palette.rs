//! Design tokens: washi and sumi, light and dark.
//!
//! The window is a sheet of unbleached paper (kinari) with real fibre grain;
//! cards are lighter sheets laid on it, text and the primary action are sumi
//! ink. One hue, shu vermilion — the colour of a hanko seal — goes on the
//! figure a screen answers with, the seal, the keyboard's focus and
//! destruction. The decision pair is earth: matcha on the copy a group keeps,
//! kuchiba ochre on a copy ticked to go. Surfaces are flat paper: one soft
//! edge, never a shadow (only what floats — a menu, a dialog, a toast — casts
//! one).

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
    use super::{AdaptiveColor, Rgba, rgba, tone};

    /// The window ground: kinari, unbleached paper, under the fibre grain;
    /// at night the warm black of an inkstone.
    pub(crate) const BACKGROUND: AdaptiveColor =
        tone(rgba(0xf2, 0xec, 0xe0, 255), rgba(0x16, 0x15, 0x13, 255));
    /// The folder pane and the settings nav: the page's own paper, so the
    /// chrome recedes and only a rule separates it from the work.
    pub(crate) const SIDEBAR: AdaptiveColor = BACKGROUND;
    /// Cards, dialogs, menus and text fields: a lighter sheet (shiro washi)
    /// laid on the ground.
    pub(crate) const CARD: AdaptiveColor =
        tone(rgba(0xfb, 0xf8, 0xf1, 255), rgba(0x1e, 0x1c, 0x19, 255));
    /// A surface that sits on a card or the ground: member tiles, secondary
    /// buttons, a hovered row.
    pub(crate) const PANE: AdaptiveColor =
        tone(rgba(0xec, 0xe5, 0xd7, 255), rgba(0x27, 0x24, 0x21, 255));
    /// One step above a pane: its hover, and the active navigation pill.
    pub(crate) const ELEVATED: AdaptiveColor =
        tone(rgba(0xe3, 0xda, 0xca, 255), rgba(0x31, 0x2d, 0x29, 255));
    /// Tracks: the segmented control's groove, the progress bar's rail,
    /// preview boxes before their image lands.
    pub(crate) const INSET: AdaptiveColor =
        tone(rgba(0xe8, 0xe1, 0xd3, 255), rgba(0x12, 0x11, 0x10, 255));
    /// Card edges and outlines: the deckle of a sheet, one soft line.
    pub(crate) const BORDER: AdaptiveColor =
        tone(rgba(0xe0, 0xd7, 0xc7, 255), rgba(0x2e, 0x2a, 0x26, 255));
    /// Rules inside a card, one step lighter than its edge, so a row reads
    /// as part of the sheet and not as a second box.
    pub(crate) const HAIRLINE: AdaptiveColor =
        tone(rgba(0x3a, 0x2c, 0x18, 18), rgba(0xff, 0xf4, 0xe0, 16));
    /// The outline of a field or a select trigger: one step darker than a
    /// card edge, so an input reads as something to write into.
    pub(crate) const CONTROL_BORDER: AdaptiveColor =
        tone(rgba(0xcf, 0xc4, 0xb1, 255), rgba(0x42, 0x3c, 0x36, 255));
    /// Sumi: the reading ink.
    pub(crate) const TEXT: AdaptiveColor =
        tone(rgba(0x1f, 0x1c, 0x18, 255), rgba(0xed, 0xe6, 0xd8, 255));
    /// Diluted ink for secondary copy. 5.2:1 on the ground and 5.6:1 on a
    /// card in light; 6.8:1 on the dark ground.
    pub(crate) const MUTED: AdaptiveColor =
        tone(rgba(0x6a, 0x62, 0x57, 255), rgba(0xa6, 0x9d, 0x8f, 255));
    /// The one action that moves the work forward: sumi in light, paper in
    /// dark. The chrome carries no hue.
    pub(crate) const PRIMARY: AdaptiveColor = TEXT;
    /// The label on [`PRIMARY`].
    pub(crate) const ON_PRIMARY: AdaptiveColor =
        tone(rgba(0xfb, 0xf8, 0xf1, 255), rgba(0x16, 0x15, 0x13, 255));
    /// Shu vermilion, the hanko's red: the screen's answer figure, the seal
    /// and the keyboard's focus ring. 4.7:1 on a light card, 5:1 on the
    /// dark one.
    pub(crate) const ACCENT: AdaptiveColor =
        tone(rgba(0xc8, 0x3e, 0x2b, 255), rgba(0xe5, 0x65, 0x4f, 255));
    /// What is printed on a seal's vermilion: the paper itself.
    pub(crate) const ON_ACCENT: AdaptiveColor =
        tone(rgba(0xfb, 0xf8, 0xf1, 255), rgba(0x16, 0x15, 0x13, 255));
    /// The keeper of a group: its edge and its mark. Matcha, so "this copy
    /// stays" can never be read as "this copy is ticked".
    pub(crate) const KEEP: AdaptiveColor =
        tone(rgba(0x3c, 0x6e, 0x47, 255), rgba(0x8c, 0xc4, 0x9a, 255));
    /// The pale backing of a keeper's badge; words on it take [`KEEP_INK`].
    pub(crate) const KEEP_WASH: AdaptiveColor =
        tone(rgba(0xe0, 0xea, 0xdb, 255), rgba(0x1d, 0x2a, 0x20, 255));
    pub(crate) const KEEP_INK: AdaptiveColor =
        tone(rgba(0x24, 0x50, 0x2f, 255), rgba(0xb5, 0xe0, 0xbe, 255));
    /// A copy ticked to go: kuchiba, fallen-leaf ochre, on its edge and mark.
    pub(crate) const SELECT: AdaptiveColor =
        tone(rgba(0xa8, 0x6a, 0x12, 255), rgba(0xe3, 0xa5, 0x4e, 255));
    pub(crate) const SELECT_WASH: AdaptiveColor =
        tone(rgba(0xf4, 0xe5, 0xc9, 255), rgba(0x33, 0x27, 0x0f, 255));
    pub(crate) const SELECT_INK: AdaptiveColor =
        tone(rgba(0x6e, 0x45, 0x08, 255), rgba(0xf2, 0xcc, 0x8a, 255));
    /// Error borders and marks. Not text: see [`DANGER_TEXT`].
    pub(crate) const DANGER: AdaptiveColor = ACCENT;
    /// The fill of a destructive action — the seal pressed — with a paper
    /// label either way (5.9:1).
    pub(crate) const DANGER_SURFACE: AdaptiveColor =
        tone(rgba(0xb8, 0x36, 0x1f, 255), rgba(0xb8, 0x36, 0x1f, 255));
    /// The label on [`DANGER_SURFACE`].
    pub(crate) const ON_DANGER: AdaptiveColor =
        tone(rgba(0xfb, 0xf8, 0xf1, 255), rgba(0xfb, 0xf8, 0xf1, 255));
    /// Error copy on a plain surface.
    pub(crate) const DANGER_TEXT: AdaptiveColor =
        tone(rgba(0xa8, 0x30, 0x1e, 255), rgba(0xf0, 0x84, 0x70, 255));
    /// The veil behind a dialog: a wash of ink that dims the app without
    /// hiding it.
    pub(crate) const SCRIM: AdaptiveColor =
        tone(rgba(0x1f, 0x1c, 0x18, 64), rgba(0x00, 0x00, 0x00, 150));

    /// A traditional dye for a category of thing — a file's kind, a cleanup
    /// finding's kind — never for a state or an action: the chrome stays sumi,
    /// the decisions stay matcha and kuchiba, and vermilion stays the answer's.
    /// A dye is a pale wash behind a tag and the deep ink its words take
    /// (6.7:1 or more either way, both appearances).
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Dye {
        pub(crate) wash: AdaptiveColor,
        pub(crate) ink: AdaptiveColor,
    }

    const fn dye(wash: (Rgba, Rgba), ink: (Rgba, Rgba)) -> Dye {
        Dye {
            wash: tone(wash.0, wash.1),
            ink: tone(ink.0, ink.1),
        }
    }

    /// Ai, indigo: documents, and the largest files.
    pub(crate) const AI: Dye = dye(
        (rgba(0xdf, 0xe5, 0xee, 255), rgba(0x1a, 0x21, 0x30, 255)),
        (rgba(0x22, 0x38, 0x5e, 255), rgba(0xae, 0xbf, 0xdc, 255)),
    );
    /// Fuji, wisteria: video, and files that are damaged or mislabelled.
    pub(crate) const FUJI: Dye = dye(
        (rgba(0xe8, 0xe1, 0xee, 255), rgba(0x25, 0x1f, 0x2d, 255)),
        (rgba(0x4f, 0x3a, 0x6b, 255), rgba(0xcd, 0xb9, 0xe3, 255)),
    );
    /// Yamabuki, kerria gold: audio. Kept clear of kuchiba's brown so a tag
    /// never reads as a copy ticked to go.
    pub(crate) const YAMABUKI: Dye = dye(
        (rgba(0xf5, 0xeb, 0xc2, 255), rgba(0x2c, 0x26, 0x0f, 255)),
        (rgba(0x5f, 0x4b, 0x00, 255), rgba(0xec, 0xd4, 0x86, 255)),
    );
    /// Sabi-asagi, rusted pale teal: archives, and leftover temp files.
    pub(crate) const ASAGI: Dye = dye(
        (rgba(0xd9, 0xe9, 0xe6, 255), rgba(0x16, 0x26, 0x2a, 255)),
        (rgba(0x1f, 0x54, 0x50, 255), rgba(0x9f, 0xd3, 0xcc, 255)),
    );
    /// Sakura, cherry: photos, and the metadata photos carry.
    pub(crate) const SAKURA: Dye = dye(
        (rgba(0xf3, 0xe0, 0xe2, 255), rgba(0x2e, 0x1c, 0x20, 255)),
        (rgba(0x7a, 0x34, 0x46, 255), rgba(0xeb, 0xb3, 0xbe, 255)),
    );
    /// Nezumi, mouse grey: anything else, and what holds nothing at all.
    pub(crate) const NEZUMI: Dye = dye(
        (rgba(0xe6, 0xe1, 0xd8, 255), rgba(0x24, 0x22, 0x1f, 255)),
        (rgba(0x4f, 0x4a, 0x43, 255), rgba(0xc2, 0xba, 0xae, 255)),
    );
}

/// The type scale: five steps, 12/14/16/26/36. Text is set in points and the
/// engine applies the user's text size to all of it.
pub(crate) mod text {
    /// Meta: paths, counts, hints, badges.
    pub(crate) const CAPTION: f32 = 12.0;
    /// Body, rows, controls.
    pub(crate) const BODY: f32 = 14.0;
    /// Card and section titles, the wordmark.
    pub(crate) const TITLE: f32 = 16.0;
    /// The page title (one per screen).
    pub(crate) const HEADING: f32 = 26.0;
    /// The screen's answer figure, and nothing else: large and light, the
    /// way a brush number sits on a page.
    pub(crate) const FIGURE: f32 = 36.0;
}

/// The face, bundled in `assets/fonts` (Inter, four weights) and registered
/// at start-up; the ground sets it for everything under it.
pub(crate) mod faces {
    pub(crate) const SANS: &str = "Inter";
}

/// Corner radii: paper is cut nearly square. Sheets take 6, the controls on
/// them 4, so nested curves step down instead of competing.
pub(crate) mod radius {
    /// Cards, dialogs, menus, toasts, preview tiles.
    pub(crate) const CARD: f32 = 6.0;
    /// Buttons, fields, rows, badges.
    pub(crate) const CONTROL: f32 = 4.0;
    /// A surface inset 2–4 pt inside a card or a control: a segment in its
    /// groove, a group header's hover, a preview inside its tile. Keeps the
    /// two curves concentric.
    pub(crate) const INSET: f32 = 2.0;
}

/// Spacing scale, the relationships the guide names: optical correction,
/// parts of one control, closely related controls, one content group,
/// separate groups, separate sections, region boundaries. `MA` is the empty
/// margin around the work itself: space as a part of the page, not left over.
pub(crate) mod space {
    pub(crate) const XXS: f32 = 2.0;
    pub(crate) const XS: f32 = 4.0;
    pub(crate) const SM: f32 = 8.0;
    pub(crate) const MD: f32 = 12.0;
    pub(crate) const LG: f32 = 16.0;
    pub(crate) const XL: f32 = 24.0;
    pub(crate) const XXL: f32 = 32.0;
    pub(crate) const MA: f32 = 48.0;
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
