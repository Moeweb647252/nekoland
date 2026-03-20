use std::collections::BTreeSet;

use bevy_ecs::lifecycle::RemovedComponents;
use bevy_ecs::prelude::{Commands, Entity, Query, ResMut, With};
use bevy_ecs::query::{Added, Changed, Or};
use nekoland_ecs::bundles::{LayerSurfaceBundle, LayerSurfaceBundleSpec};
use nekoland_ecs::components::{
    BufferState, DesiredOutputName, LayerAnchor, LayerOnOutput, LayerShellSurface, OutputDevice,
    SurfaceContentVersion, SurfaceGeometry, WlSurfaceHandle,
};
use nekoland_ecs::resources::{EntityIndex, PrimaryOutputState, WorkArea};
use nekoland_ecs::views::{LayerOutputBindingRuntime, OutputRuntime};
use nekoland_protocol::resources::{LayerLifecycleAction, PendingLayerRequests};

type LayerLifecycleSurfaces<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static WlSurfaceHandle,
        &'static mut SurfaceGeometry,
        &'static mut BufferState,
        &'static mut SurfaceContentVersion,
        &'static mut LayerShellSurface,
    ),
    With<LayerShellSurface>,
>;
type LayerOutputDevices<'w, 's> = Query<'w, 's, (Entity, &'static OutputDevice)>;
type LayerRelationshipQuery<'w, 's> =
    Query<'w, 's, (Entity, LayerOutputBindingRuntime), With<LayerShellSurface>>;
type LayerOutputRelationshipChanges<'w, 's> = Query<
    'w,
    's,
    (),
    (With<LayerShellSurface>, Or<(Added<DesiredOutputName>, Changed<DesiredOutputName>)>),
>;
type LayerOutputDeviceChanges<'w, 's> =
    Query<'w, 's, (), Or<(Added<OutputDevice>, Changed<OutputDevice>)>>;
type LayerOutputs<'w, 's> = Query<'w, 's, (Entity, OutputRuntime)>;
type ArrangedLayers<'w, 's> = Query<
    'w,
    's,
    (
        &'static LayerShellSurface,
        Option<&'static DesiredOutputName>,
        &'static LayerAnchor,
        &'static mut SurfaceGeometry,
        &'static BufferState,
        Option<&'static LayerOnOutput>,
    ),
>;
type WorkAreaLayers<'w, 's> = Query<
    'w,
    's,
    (
        &'static LayerShellSurface,
        Option<&'static DesiredOutputName>,
        &'static LayerAnchor,
        &'static SurfaceGeometry,
        &'static BufferState,
        Option<&'static LayerOnOutput>,
    ),
>;

/// Materializes layer-shell lifecycle requests into ECS entities and keeps the authoritative layer
/// geometry/buffer attachment state updated from protocol commits.
pub fn layer_lifecycle_system(
    mut commands: Commands,
    mut pending_layer_requests: ResMut<PendingLayerRequests>,
    entity_index: bevy_ecs::prelude::Res<EntityIndex>,
    existing_layers: Query<&WlSurfaceHandle, With<LayerShellSurface>>,
    mut layers: LayerLifecycleSurfaces<'_, '_>,
) {
    let mut known_surface_ids =
        existing_layers.iter().map(|surface| surface.id).collect::<BTreeSet<_>>();
    let mut deferred = Vec::new();

    for request in pending_layer_requests.drain() {
        match request.action {
            LayerLifecycleAction::Created { spec }
                if known_surface_ids.insert(request.surface_id) =>
            {
                commands.spawn(LayerSurfaceBundle::new(LayerSurfaceBundleSpec {
                    surface_id: request.surface_id,
                    namespace: spec.namespace,
                    output: spec.output_name,
                    layer: spec.layer,
                    anchor: spec.anchor,
                    desired_width: spec.desired_width,
                    desired_height: spec.desired_height,
                    exclusive_zone: spec.exclusive_zone,
                    margins: spec.margins,
                }));
            }
            LayerLifecycleAction::Committed {
                size,
                anchor,
                desired_width,
                desired_height,
                exclusive_zone,
                margins,
            } => {
                let mut layer = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| layers.get_mut(entity).ok());
                if layer.is_none() {
                    layer = layers
                        .iter_mut()
                        .find(|(_, surface, _, _, _, _)| surface.id == request.surface_id);
                }

                let Some((
                    entity,
                    _surface,
                    mut geometry,
                    mut buffer,
                    mut content_version,
                    mut layer_surface,
                )) = layer
                else {
                    deferred.push(request);
                    continue;
                };

                commands.entity(entity).insert(anchor);
                layer_surface.desired_width = desired_width;
                layer_surface.desired_height = desired_height;
                layer_surface.exclusive_zone = exclusive_zone;
                layer_surface.margins = margins;
                if let Some(size) = size {
                    geometry.width = size.width.max(1);
                    geometry.height = size.height.max(1);
                }
                buffer.attached = size.is_some();
                content_version.bump();
            }
            LayerLifecycleAction::Destroyed => {
                let mut layer = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| layers.get_mut(entity).ok());
                if layer.is_none() {
                    layer = layers
                        .iter_mut()
                        .find(|(_, surface, _, _, _, _)| surface.id == request.surface_id);
                }

                let handled = if let Some((entity, _, _, _, _, _)) = layer {
                    commands.entity(entity).despawn();
                    known_surface_ids.remove(&request.surface_id);
                    true
                } else {
                    false
                };

                if !handled {
                    deferred.push(request);
                }
            }
            _ => deferred.push(request),
        }
    }

    pending_layer_requests.replace(deferred);
}

/// Keeps each layer surface attached to the output entity named in its protocol state.
pub(crate) fn layer_output_relationship_reconciliation_needed(
    layer_output_changes: LayerOutputRelationshipChanges<'_, '_>,
    output_changes: LayerOutputDeviceChanges<'_, '_>,
    removed_output_names: RemovedComponents<DesiredOutputName>,
    removed_outputs: RemovedComponents<OutputDevice>,
) -> bool {
    !layer_output_changes.is_empty()
        || !output_changes.is_empty()
        || !removed_output_names.is_empty()
        || !removed_outputs.is_empty()
}

pub fn sync_layer_output_relationships_system(
    mut commands: Commands,
    entity_index: bevy_ecs::prelude::Res<EntityIndex>,
    outputs: LayerOutputDevices<'_, '_>,
    layers: LayerRelationshipQuery<'_, '_>,
) {
    for (entity, binding) in &layers {
        let desired_output = resolve_output_entity(
            binding
                .desired_output_name
                .and_then(|desired_output_name| desired_output_name.0.as_deref()),
            &entity_index,
            &outputs,
        );
        match (desired_output, binding.layer_output.map(|layer_output| layer_output.0)) {
            (Some(desired_output), Some(current_output)) if desired_output == current_output => {}
            (Some(desired_output), _) => {
                commands.entity(entity).insert(LayerOnOutput(desired_output));
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<LayerOnOutput>();
            }
            (None, None) => {}
        }
    }
}

/// Resolves a protocol-level output name into an ECS entity, preferring the index when it is
/// already up to date but falling back to a linear scan during early startup.
fn resolve_output_entity(
    output_name: Option<&str>,
    entity_index: &EntityIndex,
    outputs: &LayerOutputDevices<'_, '_>,
) -> Option<Entity> {
    let output_name = output_name?;
    entity_index.entity_for_output_name(output_name).or_else(|| {
        outputs.iter().find(|(_, output)| output.name == output_name).map(|(entity, _)| entity)
    })
}

/// Computes layer surface rectangles from anchors, desired size, margins, and bound output size.
pub fn layer_arrangement_system(
    primary_output: Option<bevy_ecs::prelude::Res<PrimaryOutputState>>,
    outputs: LayerOutputs<'_, '_>,
    mut layers: ArrangedLayers<'_, '_>,
) {
    let output_sizes = outputs
        .iter()
        .map(|(entity, output)| {
            (entity, output.properties.width.max(1) as i32, output.properties.height.max(1) as i32)
        })
        .collect::<Vec<_>>();
    let Some(primary_output) =
        primary_output_from_state_or_sizes(primary_output.as_deref(), &output_sizes, &outputs)
    else {
        return;
    };

    for (layer_surface, desired_output_name, anchor, mut geometry, buffer, layer_output) in
        &mut layers
    {
        let Some((_, output_width, output_height)) =
            output_size_for_layer(desired_output_name, layer_output, primary_output, &output_sizes)
        else {
            continue;
        };
        if !buffer.attached {
            geometry.width = layer_surface.desired_width.max(1);
            geometry.height = layer_surface.desired_height.max(1);
        }

        let mut width = if layer_surface.desired_width > 0 {
            layer_surface.desired_width as i32
        } else {
            geometry.width.max(1) as i32
        };
        let mut height = if layer_surface.desired_height > 0 {
            layer_surface.desired_height as i32
        } else {
            geometry.height.max(1) as i32
        };
        let horizontal_margins =
            layer_surface.margins.left.saturating_add(layer_surface.margins.right);
        let vertical_margins =
            layer_surface.margins.top.saturating_add(layer_surface.margins.bottom);

        if anchor.left && anchor.right {
            width = output_width.saturating_sub(horizontal_margins);
        }
        if anchor.top && anchor.bottom {
            height = output_height.saturating_sub(vertical_margins);
        }

        geometry.width = width.max(1) as u32;
        geometry.height = height.max(1) as u32;
        geometry.x = match (anchor.left, anchor.right) {
            (true, _) => layer_surface.margins.left,
            (false, true) => {
                output_width.saturating_sub(width).saturating_sub(layer_surface.margins.right)
            }
            (false, false) => {
                (output_width.saturating_sub(width) + layer_surface.margins.left
                    - layer_surface.margins.right)
                    / 2
            }
        };
        geometry.y = match (anchor.top, anchor.bottom) {
            (true, _) => layer_surface.margins.top,
            (false, true) => {
                output_height.saturating_sub(height).saturating_sub(layer_surface.margins.bottom)
            }
            (false, false) => {
                (output_height.saturating_sub(height) + layer_surface.margins.top
                    - layer_surface.margins.bottom)
                    / 2
            }
        };
    }

    tracing::trace!("layer arrangement system tick");
}

/// Derives the layout work area by subtracting exclusive-zone layers anchored to the primary
/// output from the full output rectangle.
pub fn work_area_system(
    primary_output: Option<bevy_ecs::prelude::Res<PrimaryOutputState>>,
    mut outputs: LayerOutputs<'_, '_>,
    layers: WorkAreaLayers<'_, '_>,
    mut work_area: ResMut<WorkArea>,
) {
    let output_sizes = outputs
        .iter()
        .map(|(entity, output)| {
            (entity, output.properties.width.max(1) as i32, output.properties.height.max(1) as i32)
        })
        .collect::<Vec<_>>();
    let Some((primary_output, output_width, output_height)) =
        primary_output_from_state_or_sizes(primary_output.as_deref(), &output_sizes, &outputs)
    else {
        return;
    };

    let mut output_work_areas = output_sizes
        .iter()
        .map(|(entity, width, height)| {
            (
                *entity,
                WorkArea {
                    x: 0,
                    y: 0,
                    width: (*width).max(1) as u32,
                    height: (*height).max(1) as u32,
                },
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    for (layer_surface, desired_output_name, anchor, geometry, buffer, layer_output) in &layers {
        if !buffer.attached || layer_surface.exclusive_zone <= 0 {
            continue;
        }
        let Some((target_output, _, _)) = output_size_for_layer(
            desired_output_name,
            layer_output,
            (primary_output, output_width, output_height),
            &output_sizes,
        ) else {
            continue;
        };

        let zone = layer_surface.exclusive_zone;
        let Some(area) = output_work_areas.get_mut(&target_output) else {
            continue;
        };
        let mut left = area.x;
        let mut top = area.y;
        let mut right = area.x.saturating_add(area.width as i32);
        let mut bottom = area.y.saturating_add(area.height as i32);

        if anchor.top && anchor.left && anchor.right && !anchor.bottom {
            top = top.max(geometry.y.saturating_add(zone));
        } else if anchor.bottom && anchor.left && anchor.right && !anchor.top {
            bottom =
                bottom.min(geometry.y.saturating_add(geometry.height as i32).saturating_sub(zone));
        } else if anchor.left && anchor.top && anchor.bottom && !anchor.right {
            left = left.max(geometry.x.saturating_add(zone));
        } else if anchor.right && anchor.top && anchor.bottom && !anchor.left {
            right =
                right.min(geometry.x.saturating_add(geometry.width as i32).saturating_sub(zone));
        }

        area.x = left;
        area.y = top;
        area.width = right.saturating_sub(left).max(1) as u32;
        area.height = bottom.saturating_sub(top).max(1) as u32;
    }

    for (entity, mut output) in &mut outputs {
        if let Some(next_work_area) = output_work_areas.get(&entity) {
            output.work_area.x = next_work_area.x;
            output.work_area.y = next_work_area.y;
            output.work_area.width = next_work_area.width;
            output.work_area.height = next_work_area.height;
        }
    }

    let primary_area = output_work_areas.get(&primary_output).copied().unwrap_or(WorkArea {
        x: 0,
        y: 0,
        width: output_width.max(1) as u32,
        height: output_height.max(1) as u32,
    });
    work_area.x = primary_area.x;
    work_area.y = primary_area.y;
    work_area.width = primary_area.width;
    work_area.height = primary_area.height;

    tracing::trace!(
        x = work_area.x,
        y = work_area.y,
        width = work_area.width,
        height = work_area.height,
        "updated shell work area"
    );
}

/// Chooses the target output size for one layer, defaulting to the primary output when the layer
/// did not bind itself to a specific output.
fn output_size_for_layer(
    desired_output_name: Option<&DesiredOutputName>,
    layer_output: Option<&LayerOnOutput>,
    primary_output: (Entity, i32, i32),
    output_sizes: &[(Entity, i32, i32)],
) -> Option<(Entity, i32, i32)> {
    if let Some(layer_output) = layer_output {
        return output_sizes.iter().find(|(entity, _, _)| *entity == layer_output.0).copied();
    }

    desired_output_name
        .and_then(|desired_output_name| desired_output_name.0.as_deref())
        .is_none()
        .then_some(primary_output)
}

/// Treats layers without an explicit output binding as targeting the primary output, while layers
/// with an unresolved desired output stay detached until the relationship can be restored.
#[allow(dead_code)]
fn layer_targets_output(
    desired_output_name: Option<&DesiredOutputName>,
    layer_output: Option<&LayerOnOutput>,
    output_entity: Entity,
) -> bool {
    if let Some(layer_output) = layer_output {
        return layer_output.0 == output_entity;
    }

    desired_output_name.and_then(|desired_output_name| desired_output_name.0.as_deref()).is_none()
}

/// Picks the largest output as the primary layout target, breaking ties by entity id for
/// deterministic tests.
fn primary_output_from_sizes(output_sizes: &[(Entity, i32, i32)]) -> Option<(Entity, i32, i32)> {
    output_sizes
        .iter()
        .copied()
        .max_by_key(|(entity, width, height)| ((i64::from(*width) * i64::from(*height)), *entity))
}

fn primary_output_from_state_or_sizes(
    primary_output_state: Option<&PrimaryOutputState>,
    output_sizes: &[(Entity, i32, i32)],
    outputs: &LayerOutputs<'_, '_>,
) -> Option<(Entity, i32, i32)> {
    let Some(primary_output_id) =
        primary_output_state.and_then(|primary_output_state| primary_output_state.id)
    else {
        return primary_output_from_sizes(output_sizes);
    };

    outputs
        .iter()
        .find(|(_, output)| output.id() == primary_output_id)
        .map(|(entity, output)| {
            (entity, output.properties.width.max(1) as i32, output.properties.height.max(1) as i32)
        })
        .or_else(|| primary_output_from_sizes(output_sizes))
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::Entity;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{LayerSurfaceBundle, LayerSurfaceBundleSpec, OutputBundle};
    use nekoland_ecs::components::{
        BufferState, DesiredOutputName, LayerAnchor, LayerLevel, LayerMargins, LayerOnOutput,
        LayerShellSurface, OutputDevice, OutputKind, OutputProperties, SurfaceGeometry,
        WlSurfaceHandle,
    };
    use nekoland_ecs::resources::{PrimaryOutputState, WorkArea, register_entity_index_hooks};
    use nekoland_protocol::resources::{
        LayerLifecycleAction, LayerLifecycleRequest, LayerSurfaceCreateSpec, PendingLayerRequests,
    };

    use super::{
        layer_arrangement_system, layer_lifecycle_system,
        layer_output_relationship_reconciliation_needed, sync_layer_output_relationships_system,
        work_area_system,
    };

    #[test]
    fn layer_creation_inserts_output_relationship_when_output_exists() {
        let mut app = NekolandApp::new("layer-output-relationship-test");
        app.insert_resource(PendingLayerRequests::default());
        register_entity_index_hooks(app.inner_mut().world_mut());
        app.inner_mut().add_systems(
            LayoutSchedule,
            (
                layer_lifecycle_system,
                sync_layer_output_relationships_system
                    .run_if(layer_output_relationship_reconciliation_needed),
            )
                .chain(),
        );

        let output_entity = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();

        app.inner_mut().world_mut().resource_mut::<PendingLayerRequests>().push(
            LayerLifecycleRequest {
                surface_id: 91,
                action: LayerLifecycleAction::Created {
                    spec: LayerSurfaceCreateSpec {
                        namespace: "panel".to_owned(),
                        output_name: Some("Virtual-1".to_owned()),
                        layer: LayerLevel::Top,
                        anchor: LayerAnchor { top: true, bottom: false, left: true, right: true },
                        desired_width: 1280,
                        desired_height: 32,
                        exclusive_zone: 32,
                        margins: LayerMargins::default(),
                    },
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let Some(layer_entity) = world
            .query::<(Entity, &WlSurfaceHandle, &LayerShellSurface)>()
            .iter(world)
            .find_map(|(entity, surface, _)| (surface.id == 91).then_some(entity))
        else {
            panic!("layer entity should be spawned");
        };
        let Some(on_output) = world.get::<LayerOnOutput>(layer_entity) else {
            panic!("created layer should resolve the output relationship");
        };

        assert_eq!(on_output.0, output_entity, "layer should point at the resolved output entity");
    }

    #[test]
    fn layer_arrangement_uses_target_output_dimensions() {
        let mut app = NekolandApp::new("layer-arrangement-output-target-test");
        app.inner_mut().add_systems(LayoutSchedule, layer_arrangement_system);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "primary".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let secondary_output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "HDMI-A-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "secondary".to_owned(),
                },
                properties: OutputProperties {
                    width: 800,
                    height: 600,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let layer = app
            .inner_mut()
            .world_mut()
            .spawn((
                LayerShellSurface {
                    namespace: "panel".to_owned(),
                    layer: LayerLevel::Top,
                    desired_width: 0,
                    desired_height: 32,
                    exclusive_zone: 0,
                    margins: LayerMargins::default(),
                },
                DesiredOutputName(Some("HDMI-A-1".to_owned())),
                LayerAnchor { top: true, bottom: false, left: true, right: true },
                SurfaceGeometry { x: 0, y: 0, width: 1, height: 32 },
                BufferState { attached: true, scale: 1 },
                LayerOnOutput(secondary_output),
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(layer) else {
            panic!("layer should keep geometry after arrangement");
        };
        assert_eq!(
            geometry.width, 800,
            "stretch layer should size itself against the targeted output, not the primary output"
        );
    }

    #[test]
    fn work_area_ignores_layers_targeting_non_primary_outputs() {
        let mut app = NekolandApp::new("layer-work-area-output-target-test");
        app.insert_resource(WorkArea::default());
        app.inner_mut().add_systems(LayoutSchedule, work_area_system);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "primary".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let secondary_output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "HDMI-A-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "secondary".to_owned(),
                },
                properties: OutputProperties {
                    width: 800,
                    height: 600,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        app.inner_mut().world_mut().spawn((
            LayerShellSurface {
                namespace: "panel".to_owned(),
                layer: LayerLevel::Top,
                desired_width: 800,
                desired_height: 32,
                exclusive_zone: 32,
                margins: LayerMargins::default(),
            },
            DesiredOutputName(Some("HDMI-A-1".to_owned())),
            LayerAnchor { top: true, bottom: false, left: true, right: true },
            SurfaceGeometry { x: 0, y: 0, width: 800, height: 32 },
            BufferState { attached: true, scale: 1 },
            LayerOnOutput(secondary_output),
        ));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(work_area) = app.inner().world().get_resource::<WorkArea>() else {
            panic!("work area resource should be present");
        };
        let work_area = *work_area;
        assert_eq!(
            work_area,
            WorkArea { x: 0, y: 0, width: 1280, height: 720 },
            "layers targeting a non-primary output should not shrink the global work area"
        );
    }

    #[test]
    fn detached_named_layer_does_not_fall_back_to_primary_output() {
        let mut app = NekolandApp::new("layer-detached-output-test");
        app.inner_mut().add_systems(LayoutSchedule, layer_arrangement_system);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "primary".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let layer = app
            .inner_mut()
            .world_mut()
            .spawn((
                LayerShellSurface {
                    namespace: "panel".to_owned(),
                    layer: LayerLevel::Top,
                    desired_width: 0,
                    desired_height: 32,
                    exclusive_zone: 0,
                    margins: LayerMargins::default(),
                },
                DesiredOutputName(Some("Missing-1".to_owned())),
                LayerAnchor { top: true, bottom: false, left: true, right: true },
                SurfaceGeometry { x: 17, y: 19, width: 123, height: 32 },
                BufferState { attached: true, scale: 1 },
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(layer) else {
            panic!("detached layer should keep geometry");
        };
        assert_eq!(
            *geometry,
            SurfaceGeometry { x: 17, y: 19, width: 123, height: 32 },
            "a layer with a named but unresolved output should stay detached instead of falling back to the primary output"
        );
    }

    #[test]
    fn detached_named_layer_does_not_shrink_work_area() {
        let mut app = NekolandApp::new("layer-detached-work-area-test");
        app.insert_resource(WorkArea::default());
        app.inner_mut().add_systems(LayoutSchedule, work_area_system);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "primary".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn((
            LayerShellSurface {
                namespace: "panel".to_owned(),
                layer: LayerLevel::Top,
                desired_width: 1280,
                desired_height: 32,
                exclusive_zone: 32,
                margins: LayerMargins::default(),
            },
            DesiredOutputName(Some("Missing-1".to_owned())),
            LayerAnchor { top: true, bottom: false, left: true, right: true },
            SurfaceGeometry { x: 0, y: 0, width: 1280, height: 32 },
            BufferState { attached: true, scale: 1 },
        ));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(work_area) = app.inner().world().get_resource::<WorkArea>() else {
            panic!("work area resource should be present");
        };
        let work_area = *work_area;
        assert_eq!(
            work_area,
            WorkArea { x: 0, y: 0, width: 1280, height: 720 },
            "a detached layer should not reserve space on the primary output"
        );
    }

    #[test]
    fn explicit_primary_output_state_overrides_largest_output_fallback() {
        let mut app = NekolandApp::new("layer-primary-output-state-test");
        app.insert_resource(PrimaryOutputState::default());
        app.insert_resource(WorkArea::default());
        app.inner_mut()
            .add_systems(LayoutSchedule, (layer_arrangement_system, work_area_system).chain());

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "largest".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let hdmi_output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "HDMI-A-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "selected".to_owned(),
                },
                properties: OutputProperties {
                    width: 800,
                    height: 600,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let hdmi_output_id = *app
            .inner()
            .world()
            .get::<nekoland_ecs::components::OutputId>(hdmi_output)
            .expect("hdmi output id");
        app.inner_mut().world_mut().resource_mut::<PrimaryOutputState>().id = Some(hdmi_output_id);
        let layer = app
            .inner_mut()
            .world_mut()
            .spawn((
                LayerShellSurface {
                    namespace: "panel".to_owned(),
                    layer: LayerLevel::Top,
                    desired_width: 0,
                    desired_height: 32,
                    exclusive_zone: 32,
                    margins: LayerMargins::default(),
                },
                DesiredOutputName(None),
                LayerAnchor { top: true, bottom: false, left: true, right: true },
                SurfaceGeometry { x: 0, y: 0, width: 1, height: 32 },
                BufferState { attached: true, scale: 1 },
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(layer) else {
            panic!("unbound layer should be arranged against the selected primary output");
        };
        assert_eq!(geometry.width, 800);

        let Some(work_area) = app.inner().world().get_resource::<WorkArea>() else {
            panic!("work area resource should be present");
        };
        let work_area = *work_area;
        assert_eq!(work_area, WorkArea { x: 0, y: 32, width: 800, height: 568 });
    }

    #[test]
    fn explicit_layer_height_overrides_oversized_committed_geometry() {
        let mut app = NekolandApp::new("layer-explicit-height-test");
        app.inner_mut().add_systems(LayoutSchedule, layer_arrangement_system);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "primary".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let layer = app
            .inner_mut()
            .world_mut()
            .spawn((
                LayerShellSurface {
                    namespace: "panel".to_owned(),
                    layer: LayerLevel::Top,
                    desired_width: 0,
                    desired_height: 32,
                    exclusive_zone: 0,
                    margins: LayerMargins::default(),
                },
                DesiredOutputName(None),
                LayerAnchor { top: true, bottom: false, left: true, right: true },
                SurfaceGeometry { x: 0, y: 0, width: 1280, height: 720 },
                BufferState { attached: true, scale: 1 },
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(layer) else {
            panic!("layer should keep geometry after arrangement");
        };
        assert_eq!(geometry.width, 1280);
        assert_eq!(
            geometry.height, 32,
            "explicit layer height should win over a stale oversized committed size",
        );
    }

    #[test]
    fn sync_layer_output_relationships_insert_when_output_appears_late() {
        let mut app = NekolandApp::new("layer-output-sync-insert-test");
        register_entity_index_hooks(app.inner_mut().world_mut());
        app.inner_mut().add_systems(
            LayoutSchedule,
            sync_layer_output_relationships_system
                .run_if(layer_output_relationship_reconciliation_needed),
        );

        let layer = app
            .inner_mut()
            .world_mut()
            .spawn(LayerSurfaceBundle::new(LayerSurfaceBundleSpec {
                surface_id: 77,
                namespace: "panel".to_owned(),
                output: Some("Virtual-1".to_owned()),
                layer: LayerLevel::Top,
                anchor: LayerAnchor { top: true, bottom: false, left: true, right: true },
                desired_width: 1280,
                desired_height: 32,
                exclusive_zone: 32,
                margins: LayerMargins::default(),
            }))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        assert!(
            app.inner().world().get::<LayerOnOutput>(layer).is_none(),
            "without a matching output entity the relationship should stay absent"
        );

        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "primary".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(relationship) = app.inner().world().get::<LayerOnOutput>(layer) else {
            panic!("relationship should be inserted once the output appears");
        };
        assert_eq!(relationship.0, output);
    }

    #[test]
    fn sync_layer_output_relationships_remove_when_output_disappears() {
        let mut app = NekolandApp::new("layer-output-sync-remove-test");
        register_entity_index_hooks(app.inner_mut().world_mut());
        app.inner_mut().add_systems(
            LayoutSchedule,
            sync_layer_output_relationships_system
                .run_if(layer_output_relationship_reconciliation_needed),
        );

        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "primary".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let layer = app
            .inner_mut()
            .world_mut()
            .spawn((
                LayerSurfaceBundle::new(LayerSurfaceBundleSpec {
                    surface_id: 77,
                    namespace: "panel".to_owned(),
                    output: Some("Virtual-1".to_owned()),
                    layer: LayerLevel::Top,
                    anchor: LayerAnchor { top: true, bottom: false, left: true, right: true },
                    desired_width: 1280,
                    desired_height: 32,
                    exclusive_zone: 32,
                    margins: LayerMargins::default(),
                }),
                LayerOnOutput(output),
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().entity_mut(output).despawn();
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        assert!(
            app.inner().world().get::<LayerOnOutput>(layer).is_none(),
            "relationship should be removed when the named output no longer exists"
        );
    }
}
