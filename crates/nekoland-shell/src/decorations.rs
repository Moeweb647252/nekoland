use bevy_ecs::prelude::{Query, Res};
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::components::{BorderTheme, WindowLayout};

/// Sync server-side border styling from compositor config into ECS decoration components.
pub fn server_decoration_system(
    config: Res<CompositorConfig>,
    mut borders: Query<(&WindowLayout, &mut BorderTheme)>,
) {
    for (layout, mut border) in &mut borders {
        // The current policy is intentionally small: only the configured color
        // and a layout-dependent width are propagated into each border theme.
        border.color = config.border_color.clone();
        border.width = layout.border_width();
    }

    tracing::trace!(border_color = %config.border_color, "server decoration system tick");
}
