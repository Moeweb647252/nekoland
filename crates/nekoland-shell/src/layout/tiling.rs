use bevy_ecs::prelude::{Query, Res, With};
use nekoland_ecs::components::{LayoutSlot, SurfaceGeometry, WindowState, Workspace, XdgWindow};
use nekoland_ecs::resources::WorkArea;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TilingLayout;

pub fn tiling_layout_system(
    mut windows: Query<(&mut SurfaceGeometry, &mut LayoutSlot, &WindowState), With<XdgWindow>>,
    workspaces: Query<&Workspace>,
    work_area: Res<WorkArea>,
) {
    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.active)
        .map(|workspace| workspace.id.0)
        .or_else(|| {
            workspaces.iter().min_by_key(|workspace| workspace.id).map(|workspace| workspace.id.0)
        });
    let mut index = 0_usize;
    let available_width = work_area.width.max(1) as i32;
    let available_height = work_area.height.max(1) as i32;
    let column_width = (available_width / 2).max(240);

    for (mut geometry, mut slot, state) in windows.iter_mut() {
        if *state != WindowState::Tiled
            || active_workspace.is_some_and(|workspace| slot.workspace != workspace)
        {
            continue;
        }

        slot.column = index as u16;
        slot.row = 0;
        geometry.x = work_area.x + (index as i32) * column_width;
        geometry.y = work_area.y;
        geometry.width = (column_width.saturating_sub(40)).max(64) as u32;
        geometry.height = available_height.max(64) as u32;
        index += 1;
    }

    tracing::trace!("tiling layout system tick");
}
