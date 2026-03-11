use std::collections::BTreeSet;

use bevy_ecs::prelude::{Commands, Entity, Query, ResMut, With};
use nekoland_ecs::bundles::LayerSurfaceBundle;
use nekoland_ecs::components::{
    BufferState, LayerAnchor, LayerShellSurface, OutputProperties, SurfaceGeometry, WlSurfaceHandle,
};
use nekoland_ecs::resources::{LayerLifecycleAction, PendingLayerRequests, WorkArea};

pub fn layer_lifecycle_system(
    mut commands: Commands,
    mut pending_layer_requests: ResMut<PendingLayerRequests>,
    existing_layers: Query<&WlSurfaceHandle, With<LayerShellSurface>>,
    mut layers: Query<
        (Entity, &WlSurfaceHandle, &mut SurfaceGeometry, &mut BufferState, &mut LayerShellSurface),
        With<LayerShellSurface>,
    >,
) {
    let mut known_surface_ids =
        existing_layers.iter().map(|surface| surface.id).collect::<BTreeSet<_>>();
    let mut deferred = Vec::new();

    for request in pending_layer_requests.items.drain(..) {
        match request.action {
            LayerLifecycleAction::Created { spec }
                if known_surface_ids.insert(request.surface_id) =>
            {
                commands.spawn(LayerSurfaceBundle::new(
                    request.surface_id,
                    spec.namespace,
                    spec.output_name,
                    spec.layer,
                    spec.anchor,
                    spec.desired_width,
                    spec.desired_height,
                    spec.exclusive_zone,
                    spec.margins,
                ));
            }
            LayerLifecycleAction::Committed {
                size,
                anchor,
                desired_width,
                desired_height,
                exclusive_zone,
                margins,
            } => {
                let Some((entity, _surface, mut geometry, mut buffer, mut layer_surface)) = layers
                    .iter_mut()
                    .find(|(_, surface, _, _, _)| surface.id == request.surface_id)
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
            }
            LayerLifecycleAction::Destroyed => {
                let mut handled = false;

                for (entity, surface, _, _, _) in &mut layers {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    commands.entity(entity).despawn();
                    known_surface_ids.remove(&request.surface_id);
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            _ => deferred.push(request),
        }
    }

    pending_layer_requests.items = deferred;
}

pub fn layer_arrangement_system(
    outputs: Query<&OutputProperties>,
    mut layers: Query<(&LayerShellSurface, &LayerAnchor, &mut SurfaceGeometry, &BufferState)>,
) {
    let Some(output) = outputs.iter().next() else {
        return;
    };
    let output_width = output.width.max(1) as i32;
    let output_height = output.height.max(1) as i32;

    for (layer_surface, anchor, mut geometry, buffer) in &mut layers {
        if !buffer.attached {
            geometry.width = layer_surface.desired_width.max(1);
            geometry.height = layer_surface.desired_height.max(1);
        }

        let mut width = geometry.width.max(layer_surface.desired_width).max(1) as i32;
        let mut height = geometry.height.max(layer_surface.desired_height).max(1) as i32;
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

pub fn work_area_system(
    outputs: Query<&OutputProperties>,
    layers: Query<(&LayerShellSurface, &LayerAnchor, &SurfaceGeometry, &BufferState)>,
    mut work_area: ResMut<WorkArea>,
) {
    let Some(output) = outputs.iter().next() else {
        return;
    };

    let mut left = 0_i32;
    let mut top = 0_i32;
    let mut right = output.width.max(1) as i32;
    let mut bottom = output.height.max(1) as i32;

    for (layer_surface, anchor, geometry, buffer) in &layers {
        if !buffer.attached || layer_surface.exclusive_zone <= 0 {
            continue;
        }

        let zone = layer_surface.exclusive_zone;
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
    }

    work_area.x = left;
    work_area.y = top;
    work_area.width = right.saturating_sub(left).max(1) as u32;
    work_area.height = bottom.saturating_sub(top).max(1) as u32;

    tracing::trace!(
        x = work_area.x,
        y = work_area.y,
        width = work_area.width,
        height = work_area.height,
        "updated shell work area"
    );
}
