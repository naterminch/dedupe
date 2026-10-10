//! Application asset source: our own bundled files (app logo, ...) with a
//! fallback to the gpui-kit bundle (Lucide icons used across the UI).
//!
//! Register [`AppAssets`] via `application().with_assets(AppAssets)`.
//! `with_assets` takes a single source, so without the fallback every
//! `IconName::...` in the UI would stop resolving. Images load by embedded
//! path, e.g. `img("icons/logo.png")` (a `&str` becomes an embedded
//! resource, never a disk lookup).

use std::borrow::Cow;

use anyhow::Result;
use gpui_kit::{AssetSource, SharedString};

/// Files under the crate `assets/` dir, embedded in the binary.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/"]
struct Embedded;

/// Combined source: ours first, gpui-kit second.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(file) = Embedded::get(path) {
            return Ok(Some(file.data));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names: Vec<SharedString> = Embedded::iter()
            .filter_map(|name| name.starts_with(path).then(|| name.into()))
            .collect();
        names.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_is_embedded() {
        let data = AppAssets
            .load("icons/logo.png")
            .expect("load must not fail")
            .expect("logo must be embedded");
        assert!(!data.is_empty(), "embedded logo must have bytes");
        // PNG magic.
        assert_eq!(&data[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn kit_icons_still_resolve_through_fallback() {
        // The UI uses Lucide icons everywhere; the fallback must keep them
        // working. `list` on the kit prefix must be non-empty.
        let names = AppAssets.list("icons/").expect("list must not fail");
        assert!(
            names.len() > 1,
            "fallback should contribute kit icons, got {names:?}"
        );
    }

    #[test]
    fn empty_path_loads_nothing() {
        assert!(AppAssets.load("").expect("load must not fail").is_none());
    }
}
