use nsh_core::element::{AdaptiveColor, Rgba, rgb};

const fn tone(light: Rgba, dark: Rgba) -> AdaptiveColor {
    AdaptiveColor {
        light: light.to_hsla(),
        dark: dark.to_hsla(),
    }
}

pub(crate) mod colors {
    use super::{AdaptiveColor, rgb, tone};

    pub(crate) const BACKGROUND: AdaptiveColor = tone(rgb(0xffffff), rgb(0x0a0a0a));
    pub(crate) const CARD: AdaptiveColor = tone(rgb(0xffffff), rgb(0x171717));
    pub(crate) const INSET: AdaptiveColor = tone(rgb(0xf2f2f2), rgb(0x121212));
    pub(crate) const BORDER: AdaptiveColor = tone(rgb(0xdedede), rgb(0x343434));
    pub(crate) const TEXT: AdaptiveColor = tone(rgb(0x171717), rgb(0xf2f2f2));
    pub(crate) const MUTED: AdaptiveColor = tone(rgb(0x565656), rgb(0xbdbdbd));
    pub(crate) const ACCENT: AdaptiveColor = tone(rgb(0x073b87), rgb(0x8cc4fa));
    pub(crate) const ON_ACCENT: AdaptiveColor = tone(rgb(0xffffff), rgb(0x07233f));
    pub(crate) const ACCENT_WASH: AdaptiveColor = tone(rgb(0xf2f7fd), rgb(0x0a1624));
    pub(crate) const ACCENT_WASH_STRONG: AdaptiveColor = tone(rgb(0xe5f0fc), rgb(0x0e2033));
    pub(crate) const DANGER: AdaptiveColor = tone(rgb(0xe7000b), rgb(0xff6467));
}

pub(crate) mod text {
    pub(crate) const CAPTION: f32 = 12.0;
    pub(crate) const BODY: f32 = 14.0;
    pub(crate) const HEADING: f32 = 23.0;
}

pub(crate) mod radius {
    pub(crate) const CARD: f32 = 10.0;
    pub(crate) const CONTROL: f32 = 8.0;
}
