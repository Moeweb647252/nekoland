use crate::components::{LayerLevel, WindowMode, WindowRole};

/// Returns whether a managed window should contribute to the visible desktop scene.
pub fn managed_window_visible(mode: WindowMode, viewport_visible: bool, role: WindowRole) -> bool {
    mode != WindowMode::Hidden && viewport_visible && role.is_managed()
}

/// Returns whether an output-background window should be treated as visible.
pub fn output_background_window_visible(
    mode: WindowMode,
    has_target_output: bool,
    role: WindowRole,
) -> bool {
    mode != WindowMode::Hidden && has_target_output && role.is_output_background()
}

/// Returns whether a popup should be considered visible given attachment and parent visibility.
pub fn popup_visible(attached: bool, parent_visible: bool) -> bool {
    attached && parent_visible
}

/// Returns whether a layer-shell surface should be considered visible for layout/render.
pub fn layer_visible(attached: bool, has_target_output: bool) -> bool {
    attached && has_target_output
}

/// Returns whether a layer belongs to the background half of the layer stack.
pub fn is_background_band_layer(level: LayerLevel) -> bool {
    matches!(level, LayerLevel::Background | LayerLevel::Bottom)
}

/// Returns whether a layer belongs to the foreground half of the layer stack.
pub fn is_foreground_band_layer(level: LayerLevel) -> bool {
    matches!(level, LayerLevel::Top | LayerLevel::Overlay)
}

#[cfg(test)]
mod tests {
    use crate::components::{LayerLevel, WindowMode, WindowRole};

    use super::{
        is_background_band_layer, is_foreground_band_layer, layer_visible, managed_window_visible,
        output_background_window_visible, popup_visible,
    };

    #[test]
    fn window_visibility_helpers_follow_window_role() {
        assert!(managed_window_visible(WindowMode::Normal, true, WindowRole::Managed));
        assert!(!managed_window_visible(WindowMode::Normal, true, WindowRole::OutputBackground,));
        assert!(output_background_window_visible(
            WindowMode::Fullscreen,
            true,
            WindowRole::OutputBackground,
        ));
        assert!(!output_background_window_visible(
            WindowMode::Hidden,
            true,
            WindowRole::OutputBackground,
        ));
    }

    #[test]
    fn popup_and_layer_helpers_match_attachment_requirements() {
        assert!(popup_visible(true, true));
        assert!(!popup_visible(false, true));
        assert!(layer_visible(true, true));
        assert!(!layer_visible(true, false));
    }

    #[test]
    fn layer_band_helpers_partition_levels() {
        assert!(is_background_band_layer(LayerLevel::Background));
        assert!(is_background_band_layer(LayerLevel::Bottom));
        assert!(is_foreground_band_layer(LayerLevel::Top));
        assert!(is_foreground_band_layer(LayerLevel::Overlay));
        assert!(!is_background_band_layer(LayerLevel::Top));
        assert!(!is_foreground_band_layer(LayerLevel::Bottom));
    }
}
