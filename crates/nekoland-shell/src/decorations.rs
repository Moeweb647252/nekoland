use bevy_ecs::prelude::{Query, Res};
use nekoland_ecs::components::BorderTheme;
use nekoland_ecs::resources::CompositorConfig;

pub fn server_decoration_system(
    config: Res<CompositorConfig>,
    mut borders: Query<&mut BorderTheme>,
) {
    for mut border in &mut borders {
        border.color = config.border_color.clone();
        border.width = if config.default_layout == "tiling" { 2 } else { 1 };
    }

    tracing::trace!(border_color = %config.border_color, "server decoration system tick");
}
