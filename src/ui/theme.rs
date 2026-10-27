use gpui_kit::component::Theme;
use gpui_kit::{App, Hsla, px, rgb};

pub(crate) trait ThemeReadability {
    fn text_xs_px(&self) -> f32;
    fn text_sm_px(&self) -> f32;
    fn text_heading_px(&self) -> f32;
    fn card_surface(&self) -> Hsla;
    fn accent_wash(&self) -> Hsla;
    fn accent_wash_strong(&self) -> Hsla;
}

impl ThemeReadability for Theme {
    fn text_xs_px(&self) -> f32 {
        12.
    }

    fn text_sm_px(&self) -> f32 {
        14.
    }

    fn text_heading_px(&self) -> f32 {
        23.
    }

    fn card_surface(&self) -> Hsla {
        if self.is_dark() {
            rgb(0x171717).into()
        } else {
            rgb(0xffffff).into()
        }
    }

    fn accent_wash(&self) -> Hsla {
        if self.is_dark() {
            rgb(0x0a1624).into()
        } else {
            rgb(0xf2f7fd).into()
        }
    }

    fn accent_wash_strong(&self) -> Hsla {
        if self.is_dark() {
            rgb(0x0e2033).into()
        } else {
            rgb(0xe5f0fc).into()
        }
    }
}

pub(crate) fn apply_readable_theme(cx: &mut App) {
    Theme::update(cx, |theme| {
        if theme.is_dark() {
            let accent = rgb(0x8cc4fa);
            theme.font_size = px(14.);
            theme.foreground = rgb(0xf2f2f2).into();
            theme.muted_foreground = rgb(0xbdbdbd).into();
            theme.group_box_foreground = rgb(0xf2f2f2).into();
            theme.sidebar_foreground = rgb(0xf2f2f2).into();
            theme.border = rgb(0x343434).into();
            theme.group_box = rgb(0x121212).into();
            theme.accent = accent.into();
            theme.accent_foreground = rgb(0x07233f).into();
            theme.ring = accent.into();
            theme.shadow = false;
        } else {
            let accent = rgb(0x073b87);
            theme.font_size = px(14.);
            theme.foreground = rgb(0x171717).into();
            theme.muted_foreground = rgb(0x565656).into();
            theme.group_box_foreground = rgb(0x171717).into();
            theme.sidebar_foreground = rgb(0x171717).into();
            theme.border = rgb(0xdedede).into();
            theme.group_box = rgb(0xf2f2f2).into();
            theme.accent = accent.into();
            theme.accent_foreground = rgb(0xffffff).into();
            theme.ring = accent.into();
            theme.shadow = true;
        }
    });
}
