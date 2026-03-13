use std::collections::BTreeSet;

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Local, Query, ResMut, With};
use nekoland_ecs::components::{
    BufferState, LayerShellSurface, OutputDevice, SurfaceGeometry, WindowMode, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{DamageRect, DamageState, OutputDamageRegions};
use nekoland_ecs::views::{PopupSnapshotRuntime, WindowSnapshotRuntime};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DamageTracker;

/// Derives coarse damage rectangles from the current visible scene graph.
///
/// This tracker is intentionally simple for now: any change in the number of visible elements
/// triggers a full redraw, and every visible surface contributes its full geometry as damage.
pub fn damage_tracking_system(
    layers: Query<(&SurfaceGeometry, &BufferState), With<LayerShellSurface>>,
    windows: Query<(Entity, WindowSnapshotRuntime), With<XdgWindow>>,
    popups: Query<PopupSnapshotRuntime, With<XdgPopup>>,
    outputs: Query<&OutputDevice>,
    mut damage_state: ResMut<DamageState>,
    mut output_damage_regions: ResMut<OutputDamageRegions>,
    mut previous_count: Local<usize>,
) {
    let active_window_surfaces = windows
        .iter()
        .filter(|(_, window)| *window.mode != WindowMode::Hidden)
        .map(|(_, window)| window.surface_id())
        .collect::<BTreeSet<_>>();
    let active_window_entities = windows
        .iter()
        .filter(|(_, window)| *window.mode != WindowMode::Hidden)
        .map(|(entity, _)| entity)
        .collect::<BTreeSet<_>>();
    let count = active_window_surfaces.len()
        + layers.iter().filter(|(_, buffer)| buffer.attached).count()
        + popups
            .iter()
            .filter(|popup| popup_parent_visible(popup.child_of, &active_window_entities))
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
                .filter(|(_, window)| active_window_surfaces.contains(&window.surface_id()))
                .map(|(_, window)| DamageRect {
                    x: window.geometry.x,
                    y: window.geometry.y,
                    width: window.geometry.width,
                    height: window.geometry.height,
                }),
        )
        .chain(
            popups
                .iter()
                .filter(|popup| popup_parent_visible(popup.child_of, &active_window_entities))
                .map(|popup| DamageRect {
                    x: popup.geometry.x,
                    y: popup.geometry.y,
                    width: popup.geometry.width,
                    height: popup.geometry.height,
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

/// Popups only contribute damage while their parent toplevel is still visible.
fn popup_parent_visible(child_of: &ChildOf, active_window_entities: &BTreeSet<Entity>) -> bool {
    active_window_entities.contains(&child_of.parent())
}
