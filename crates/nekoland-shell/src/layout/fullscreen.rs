use bevy_ecs::prelude::{Query, Res, With};
use nekoland_ecs::components::{WindowMode, XdgWindow};
use nekoland_ecs::resources::WorkArea;
use nekoland_ecs::views::{OutputRuntime, WindowRuntime};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullscreenLayout;

/// Applies fullscreen and maximized geometry after layout/work-area state has been updated.
pub fn fullscreen_layout_system(
    outputs: Query<OutputRuntime>,
    mut windows: Query<WindowRuntime, With<XdgWindow>>,
    work_area: Res<WorkArea>,
) {
    let Some(output) = outputs.iter().next() else {
        tracing::trace!("fullscreen layout system tick");
        return;
    };
    for mut window in &mut windows {
        match *window.mode {
            WindowMode::Fullscreen => {
                window.geometry.x = 0;
                window.geometry.y = 0;
                window.geometry.width = output.properties.width.max(1);
                window.geometry.height = output.properties.height.max(1);
            }
            WindowMode::Maximized => {
                // Keep a small inset so maximized windows still leave room for compositor-side
                // borders and do not visually merge into the output edge.
                window.geometry.x = work_area.x + 16;
                window.geometry.y = work_area.y + 16;
                window.geometry.width = work_area.width.saturating_sub(32).max(1);
                window.geometry.height = work_area.height.saturating_sub(32).max(1);
            }
            _ => {}
        }
    }

    tracing::trace!("fullscreen layout system tick");
}
