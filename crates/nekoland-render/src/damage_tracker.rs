use std::collections::BTreeSet;

use bevy_ecs::prelude::{Local, Query, ResMut, With};
use nekoland_ecs::components::{
    BufferState, LayerShellSurface, LayoutSlot, OutputDevice, SurfaceGeometry, WindowState,
    Workspace, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{DamageRect, DamageState, OutputDamageRegions};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DamageTracker;

pub fn damage_tracking_system(
    layers: Query<(&SurfaceGeometry, &BufferState), With<LayerShellSurface>>,
    windows: Query<
        (&SurfaceGeometry, &WindowState, &LayoutSlot, &nekoland_ecs::components::WlSurfaceHandle),
        With<XdgWindow>,
    >,
    popups: Query<(&SurfaceGeometry, &XdgPopup), With<XdgPopup>>,
    outputs: Query<&OutputDevice>,
    workspaces: Query<&Workspace>,
    mut damage_state: ResMut<DamageState>,
    mut output_damage_regions: ResMut<OutputDamageRegions>,
    mut previous_count: Local<usize>,
) {
    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.active)
        .map(|workspace| workspace.id.0)
        .or_else(|| {
            workspaces.iter().min_by_key(|workspace| workspace.id).map(|workspace| workspace.id.0)
        });
    let active_window_surfaces = windows
        .iter()
        .filter(|(_, state, layout_slot, _)| {
            **state != WindowState::Hidden
                && active_workspace.is_none_or(|workspace| layout_slot.workspace == workspace)
        })
        .map(|(_, _, _, surface)| surface.id)
        .collect::<BTreeSet<_>>();
    let count = active_window_surfaces.len()
        + layers.iter().filter(|(_, buffer)| buffer.attached).count()
        + popups
            .iter()
            .filter(|(_, popup)| active_window_surfaces.contains(&popup.parent_surface))
            .count();
    damage_state.full_redraw = count != *previous_count;
    *previous_count = count;

    let regions = layers
        .iter()
        .filter(|(_, buffer)| buffer.attached)
        .map(|(geometry, _)| DamageRect {
            x: geometry.x,
            y: geometry.y,
            width: geometry.width,
            height: geometry.height,
        })
        .chain(
            windows
                .iter()
                .filter(|(_, _, _, surface)| active_window_surfaces.contains(&surface.id))
                .map(|(geometry, _, _, _)| DamageRect {
                    x: geometry.x,
                    y: geometry.y,
                    width: geometry.width,
                    height: geometry.height,
                }),
        )
        .chain(
            popups
                .iter()
                .filter(|(_, popup)| active_window_surfaces.contains(&popup.parent_surface))
                .map(|(geometry, _)| DamageRect {
                    x: geometry.x,
                    y: geometry.y,
                    width: geometry.width,
                    height: geometry.height,
                }),
        )
        .collect::<Vec<_>>();
    let output_names = outputs.iter().map(|output| output.name.clone()).collect::<Vec<_>>();
    output_damage_regions.regions.clear();
    if output_names.is_empty() {
        output_damage_regions.regions.insert("Virtual-1".to_owned(), regions);
    } else {
        for output_name in output_names {
            output_damage_regions.regions.insert(output_name, regions.clone());
        }
    }

    tracing::trace!(count, full_redraw = damage_state.full_redraw, "damage tracking tick");
}
