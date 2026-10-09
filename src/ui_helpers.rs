//! Pure UI helpers extracted from `ui.rs`: text capping, hash captions,
//! progress-bar reads, and size slider/text syncing.
//!
//! No view state lives here — only functions. `DedupeView` appears solely
//! as the `Context` generic so slider/input sync can notify correctly.

use crate::ui::{DedupeView, SIZE_SLIDER_MAX, SIZE_SLIDER_MIN};
use crate::util;
use gpui_kit::component::input::InputState;
use gpui_kit::component::slider::SliderState;
use gpui_kit::{Context, Entity, SharedString, Window};

/// Read (position, length) off a headless progress bar.
pub(crate) fn bar_pos(bar: &indicatif::ProgressBar) -> (u64, u64) {
    (bar.position(), bar.length().unwrap_or(0))
}

/// Cap a display string, keeping the tail — filenames live at the end of
/// paths, so a truncated middle would hide the useful part. Paired with
/// `.truncate()` (ellipsis) on the element for pixel-level clipping.
pub(crate) fn fit_text(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    format!(
        "…{}",
        s.chars().skip(count - max_chars + 1).collect::<String>()
    )
}

/// One-line explanation for each hash algorithm, shown under the selector.
pub(crate) fn hash_caption(index: Option<usize>) -> &'static str {
    match index.unwrap_or(0) {
        1 => "SHA-256 — widely supported standard; pick it to match other tools.",
        2 => "MD5 — legacy and collision-prone; only to match old hashes.",
        _ => "BLAKE3 — fastest modern hash; the right default for scans.",
    }
}

/// Display text for "no size limit". The boxes never look empty-broken:
/// an unbounded end reads `unlimited`, and the same word (case-insensitive)
/// is accepted as input wherever a size is read.
pub(crate) const NO_LIMIT_LABEL: &str = "unlimited";

/// True when a size box means "no limit": blank (legacy) or `unlimited`.
pub(crate) fn is_no_limit(raw: &str) -> bool {
    let raw = raw.trim();
    raw.is_empty() || raw.eq_ignore_ascii_case(NO_LIMIT_LABEL)
}

/// Parse a size box into an optional byte bound. Blank or `unlimited`
/// means unbounded (`None`); anything else goes through [`util::parse_size`].
pub(crate) fn parse_size_limit(raw: &str) -> anyhow::Result<Option<u64>> {
    if is_no_limit(raw) {
        return Ok(None);
    }
    util::parse_size(raw.trim()).map(Some)
}

/// Mirror a size slider position into its text box. `empty_at_min`: the
/// min-size slider clears the box at the left end (no limit); the max-size
/// slider clears it at the right end. The empty end writes `unlimited`
/// so the box never looks broken.
pub(crate) fn size_slider_to_text(
    value: f32,
    empty_at_min: bool,
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<DedupeView>,
) {
    let at_empty_end = if empty_at_min {
        value <= SIZE_SLIDER_MIN * 1.001
    } else {
        value >= SIZE_SLIDER_MAX / 1.001
    };
    let text = if at_empty_end {
        NO_LIMIT_LABEL.to_string()
    } else {
        util::human_bytes(value.max(1.0) as u64)
    };
    input.update(cx, |state, cx| {
        state.set_value(SharedString::from(text), window, cx);
    });
}

/// Mirror a typed size into its slider. Blank or `unlimited` means "no
/// limit" (slider to the empty end); unparsable text leaves the slider alone.
pub(crate) fn size_text_to_slider(
    raw: &str,
    empty_at_min: bool,
    slider: &Entity<SliderState>,
    window: &mut Window,
    cx: &mut Context<DedupeView>,
) {
    let raw = raw.trim();
    let target: f32 = if is_no_limit(raw) {
        if empty_at_min {
            SIZE_SLIDER_MIN
        } else {
            SIZE_SLIDER_MAX
        }
    } else {
        match util::parse_size(raw) {
            Ok(b) => (b as f32).clamp(SIZE_SLIDER_MIN, SIZE_SLIDER_MAX),
            Err(_) => return,
        }
    };
    slider.update(cx, |state, cx| {
        state.set_value(target, window, cx);
    });
}

/// Initial slider position for a restored prefs size string. Blank,
/// `unlimited`, or unparsable means "no limit" (slider to the empty end).
pub(crate) fn prefs_size_to_slider(raw: &str, empty_at_min: bool) -> f32 {
    let raw = raw.trim();
    if is_no_limit(raw) {
        return if empty_at_min {
            SIZE_SLIDER_MIN
        } else {
            SIZE_SLIDER_MAX
        };
    }
    match util::parse_size(raw) {
        Ok(b) => (b as f32).clamp(SIZE_SLIDER_MIN, SIZE_SLIDER_MAX),
        Err(_) => {
            if empty_at_min {
                SIZE_SLIDER_MIN
            } else {
                SIZE_SLIDER_MAX
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_text_keeps_short_strings_and_truncates_tails() {
        assert_eq!(fit_text("abc", 10), "abc");
        assert_eq!(fit_text("abcdef", 6), "abcdef");
        // Long paths keep the filename tail, prefixed with an ellipsis.
        let capped = fit_text("C:\\very\\long\\directory\\file.txt", 12);
        assert_eq!(capped, "…ry\\file.txt");
        assert_eq!(capped.chars().count(), 12);
    }

    #[test]
    fn size_limit_sentinel_parses() {
        assert_eq!(parse_size_limit("").unwrap(), None);
        assert_eq!(parse_size_limit("unlimited").unwrap(), None);
        assert_eq!(parse_size_limit("Unlimited").unwrap(), None);
        assert_eq!(parse_size_limit("  UNLIMITED  ").unwrap(), None);
        assert_eq!(parse_size_limit("100KB").unwrap(), Some(100_000));
        assert!(parse_size_limit("abc").is_err());
    }

    #[test]
    fn hash_caption_names_all_three_algorithms() {
        assert!(hash_caption(Some(0)).contains("BLAKE3"));
        assert!(hash_caption(None).contains("BLAKE3"));
        assert!(hash_caption(Some(1)).contains("SHA-256"));
        assert!(hash_caption(Some(2)).contains("MD5"));
    }

    #[test]
    fn prefs_slider_positions_clamp_to_slider_bounds() {
        assert_eq!(prefs_size_to_slider("", true), SIZE_SLIDER_MIN);
        assert_eq!(prefs_size_to_slider("", false), SIZE_SLIDER_MAX);
        assert_eq!(prefs_size_to_slider("unlimited", true), SIZE_SLIDER_MIN);
        // Below the floor clamps up; astronomically large clamps down.
        assert_eq!(prefs_size_to_slider("1", true), SIZE_SLIDER_MIN);
        assert_eq!(prefs_size_to_slider("99999TB", false), SIZE_SLIDER_MAX);
        // A sane middle value lands strictly inside the range.
        let mid = prefs_size_to_slider("500MB", true);
        assert!(mid > SIZE_SLIDER_MIN && mid < SIZE_SLIDER_MAX);
    }
}
