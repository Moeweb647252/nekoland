use bevy_ecs::prelude::{Query, Res, With};
use nekoland_ecs::components::{
    LayoutSlot, OutputProperties, SurfaceGeometry, WindowState, Workspace, XdgWindow,
};
use nekoland_ecs::resources::WorkArea;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullscreenLayout;

pub fn fullscreen_layout_system(
    outputs: Query<&OutputProperties>,
    mut windows: Query<(&mut SurfaceGeometry, &WindowState, &LayoutSlot), With<XdgWindow>>,
    workspaces: Query<&Workspace>,
    work_area: Res<WorkArea>,
) {
    let Some(output) = outputs.iter().next() else {
        tracing::trace!("fullscreen layout system tick");
        return;
    };
    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.active)
        .map(|workspace| workspace.id.0)
        .or_else(|| {
            workspaces.iter().min_by_key(|workspace| workspace.id).map(|workspace| workspace.id.0)
        });

    for (mut geometry, state, layout_slot) in &mut windows {
        if active_workspace.is_some_and(|workspace| layout_slot.workspace != workspace) {
            continue;
        }

        match state {
            WindowState::Fullscreen => {
                geometry.x = 0;
                geometry.y = 0;
                geometry.width = output.width.max(1);
                geometry.height = output.height.max(1);
            }
            WindowState::Maximized => {
                geometry.x = work_area.x + 16;
                geometry.y = work_area.y + 16;
                geometry.width = work_area.width.saturating_sub(32).max(1);
                geometry.height = work_area.height.saturating_sub(32).max(1);
            }
            _ => {}
        }
    }

    tracing::trace!("fullscreen layout system tick");
}
