use std::collections::{BTreeMap, BTreeSet, HashMap};

use bevy_ecs::prelude::{Entity, Query, Res, ResMut, With};
use nekoland_ecs::components::{
    BufferState, DesiredOutputName, LayerOnOutput, LayerShellSurface, SurfaceGeometry,
    WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::presentation_logic::{
    layer_visible, managed_window_visible, output_background_window_visible, popup_visible,
};
use nekoland_ecs::resources::{
    SurfacePresentationRole, SurfacePresentationSnapshot, SurfacePresentationState, WaylandIngress,
};
use nekoland_ecs::views::{OutputRuntime, PopupSnapshotRuntime, WindowSnapshotRuntime};

use crate::viewport::preferred_primary_output_id;

type LayerPresentationQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static WlSurfaceHandle,
        &'static SurfaceGeometry,
        &'static BufferState,
        Option<&'static LayerOnOutput>,
        Option<&'static DesiredOutputName>,
    ),
    With<LayerShellSurface>,
>;

pub fn surface_presentation_snapshot_system(
    outputs: Query<(Entity, OutputRuntime)>,
    wayland_ingress: Res<WaylandIngress>,
    windows: Query<(Entity, WindowSnapshotRuntime), With<XdgWindow>>,
    popups: Query<(Entity, PopupSnapshotRuntime), With<XdgPopup>>,
    layers: LayerPresentationQuery<'_, '_>,
    mut snapshot: ResMut<SurfacePresentationSnapshot>,
) {
    let live_output_ids = outputs.iter().map(|(_, output)| output.id()).collect::<BTreeSet<_>>();
    let output_ids_by_name = outputs
        .iter()
        .map(|(_, output)| (output.name().to_owned(), output.id()))
        .collect::<HashMap<_, _>>();
    let primary_output_id = preferred_primary_output_id(Some(&wayland_ingress))
        .or_else(|| live_output_ids.iter().next().copied());

    let mut surfaces = BTreeMap::new();
    let mut presentation_by_entity = HashMap::new();

    for (entity, window) in windows.iter() {
        let role = if window.role.is_output_background() {
            SurfacePresentationRole::OutputBackground
        } else {
            SurfacePresentationRole::Window
        };
        let target_output = window
            .background
            .map(|background| background.output)
            .or_else(|| window.viewport_visibility.output.clone())
            .filter(|output_id| live_output_ids.contains(output_id));
        let visible = match role {
            SurfacePresentationRole::OutputBackground => output_background_window_visible(
                *window.mode,
                target_output.is_some(),
                *window.role,
            ),
            SurfacePresentationRole::Window => managed_window_visible(
                *window.mode,
                window.viewport_visibility.visible,
                *window.role,
            ),
            _ => false,
        };
        let state = SurfacePresentationState {
            visible,
            target_output: target_output.clone(),
            geometry: window.geometry.clone(),
            input_enabled: visible
                && window.role.is_managed()
                && window.x11_window.is_none_or(|window| !window.is_helper_surface()),
            damage_enabled: visible,
            role,
        };
        presentation_by_entity.insert(entity, state.clone());
        surfaces.insert(window.surface_id(), state);
    }

    let mut pending_popups = popups
        .iter()
        .map(|(entity, popup)| {
            (
                entity,
                popup.surface_id(),
                popup.child_of.parent(),
                popup.geometry.clone(),
                popup.buffer.attached,
            )
        })
        .collect::<Vec<_>>();
    while !pending_popups.is_empty() {
        let mut progressed = false;
        let mut unresolved = Vec::new();
        for (entity, surface_id, parent_entity, geometry, attached) in pending_popups {
            let Some(parent_state) = presentation_by_entity.get(&parent_entity) else {
                unresolved.push((entity, surface_id, parent_entity, geometry, attached));
                continue;
            };

            let visible = popup_visible(attached, parent_state.visible);
            let state = SurfacePresentationState {
                visible,
                target_output: parent_state.target_output,
                geometry,
                input_enabled: visible,
                damage_enabled: visible,
                role: SurfacePresentationRole::Popup,
            };
            presentation_by_entity.insert(entity, state.clone());
            surfaces.insert(surface_id, state);
            progressed = true;
        }

        if progressed {
            pending_popups = unresolved;
            continue;
        }

        for (entity, surface_id, _, geometry, _) in unresolved {
            let state = SurfacePresentationState {
                visible: false,
                target_output: None,
                geometry,
                input_enabled: false,
                damage_enabled: false,
                role: SurfacePresentationRole::Popup,
            };
            presentation_by_entity.insert(entity, state.clone());
            surfaces.insert(surface_id, state);
        }
        break;
    }

    for (surface, geometry, buffer, layer_output, desired_output_name) in layers.iter() {
        let target_output = layer_output
            .and_then(|layer_output| {
                outputs
                    .iter()
                    .find(|(entity, _)| *entity == layer_output.0)
                    .map(|(_, output)| output.id())
            })
            .or_else(|| {
                desired_output_name
                    .and_then(|desired_output_name| desired_output_name.0.as_deref())
                    .and_then(|output_name| output_ids_by_name.get(output_name).copied())
            })
            .or(primary_output_id)
            .filter(|output_id| live_output_ids.contains(output_id));
        let visible = layer_visible(buffer.attached, target_output.is_some());
        surfaces.insert(
            surface.id,
            SurfacePresentationState {
                visible,
                target_output,
                geometry: (*geometry).clone(),
                input_enabled: visible,
                damage_enabled: visible,
                role: SurfacePresentationRole::Layer,
            },
        );
    }

    snapshot.surfaces = surfaces;
}

#[cfg(test)]
mod tests {
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::RunSystemOnce;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        BufferState, OutputDevice, OutputId, OutputKind, OutputProperties, SurfaceGeometry,
        WlSurfaceHandle, XdgPopup,
    };
    use nekoland_ecs::resources::{
        OutputGeometrySnapshot, OutputSnapshotState, SurfacePresentationSnapshot, WaylandIngress,
    };

    use super::surface_presentation_snapshot_system;

    #[test]
    fn nested_popups_inherit_visibility_from_popup_parents() {
        let mut world = World::default();
        world.insert_resource(SurfacePresentationSnapshot::default());
        world.insert_resource(WaylandIngress {
            output_snapshots: OutputSnapshotState {
                outputs: vec![OutputGeometrySnapshot {
                    output_id: OutputId(1),
                    name: "Virtual-1".to_owned(),
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                    scale: 1,
                    refresh_millihz: 60_000,
                }],
            },
            ..WaylandIngress::default()
        });
        world.spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "Nekoland".to_owned(),
                model: "test".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });

        let window = world
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 10 },
                geometry: SurfaceGeometry { x: 20, y: 30, width: 320, height: 180 },
                viewport_visibility: nekoland_ecs::components::WindowViewportVisibility {
                    visible: true,
                    output: Some(OutputId(1)),
                },
                buffer: BufferState { attached: true, scale: 1 },
                ..Default::default()
            })
            .id();
        let popup = world
            .spawn((
                WlSurfaceHandle { id: 11 },
                SurfaceGeometry { x: 40, y: 50, width: 160, height: 90 },
                BufferState { attached: true, scale: 1 },
                XdgPopup::default(),
                ChildOf(window),
            ))
            .id();
        world.spawn((
            WlSurfaceHandle { id: 12 },
            SurfaceGeometry { x: 60, y: 70, width: 120, height: 60 },
            BufferState { attached: true, scale: 1 },
            XdgPopup::default(),
            ChildOf(popup),
        ));

        let Ok(()) = world.run_system_once(surface_presentation_snapshot_system) else {
            panic!("surface presentation snapshot system should run");
        };

        let snapshot = world.resource::<SurfacePresentationSnapshot>();
        assert!(snapshot.surfaces[&11].visible);
        assert!(snapshot.surfaces[&12].visible);
        assert_eq!(snapshot.surfaces[&12].target_output, Some(OutputId(1)));
    }
}
