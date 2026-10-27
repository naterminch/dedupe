use super::*;

use gpui_kit::component::slider::{SliderScale, SliderState};

/// Regression: the size sliders must build without panicking. The state
/// clamps on every builder call, so max() has to precede min() —
/// min(1024) with the default max(100) panics inside f32::clamp.
#[test]
fn size_sliders_build_without_panic() {
    for default in [SIZE_SLIDER_MIN, SIZE_SLIDER_MAX] {
        let s = SliderState::new()
            .max(SIZE_SLIDER_MAX)
            .min(SIZE_SLIDER_MIN)
            .scale(SliderScale::Logarithmic)
            .default_value(default);
        assert_eq!(s.min_value(), SIZE_SLIDER_MIN);
        assert_eq!(s.max_value(), SIZE_SLIDER_MAX);
    }
}
