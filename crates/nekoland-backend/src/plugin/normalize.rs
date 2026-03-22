use std::collections::HashMap;

use crate::common::outputs::{
    BackendOutputMaterializationPlan, PendingBackendOutputEvents, PendingBackendOutputUpdates,
    collect_output_snapshots,
};
use bevy_ecs::prelude::{Entity, Res, ResMut};
use nekoland_ecs::resources::{
    PendingBackendInputEvents, PendingPlatformInputEvents, PrimaryOutputState, RenderSurfaceRole,
    RenderSurfaceSnapshot, ShellRenderInput, SurfacePresentationRole, SurfacePresentationSnapshot,
    WaylandIngress,
};

use super::{BackendOutputQuery, BackendPresentInputs, BackendPresentSurfaceQuery};

fn collect_render_surface_snapshots(
    outputs: &BackendOutputQuery<'_, '_>,
    surfaces: &BackendPresentSurfaceQuery<'_, '_>,
    primary_output: Option<&PrimaryOutputState>,
    surface_presentation: Option<&SurfacePresentationSnapshot>,
) -> std::collections::BTreeMap<u64, RenderSurfaceSnapshot> {
    if let Some(surface_presentation) = surface_presentation {
        return surfaces
            .iter()
            .filter_map(|(_, surface)| {
                surface_presentation.surfaces.get(&surface.surface_id()).map(|state| {
                    (
                        surface.surface_id(),
                        RenderSurfaceSnapshot {
                            geometry: state.geometry.clone(),
                            role: render_surface_role_from_presentation(state.role),
                            target_output: state.target_output,
                        },
                    )
                })
            })
            .collect();
    }

    let output_ids =
        outputs.iter().map(|(entity, output, _)| (entity, output.id())).collect::<HashMap<_, _>>();
    let output_ids_by_name = outputs
        .iter()
        .map(|(_, output, _)| (output.name().to_owned(), output.id()))
        .collect::<HashMap<_, _>>();
    let primary_output_id = primary_output.and_then(|primary_output| primary_output.id);
    let window_target_outputs = surfaces
        .iter()
        .filter_map(|(entity, surface)| {
            surface.window.map(|_| {
                (
                    entity,
                    surface.background.map(|background| background.output).or_else(|| {
                        surface
                            .viewport_visibility
                            .and_then(|viewport_visibility| viewport_visibility.output)
                    }),
                    surface.surface_id(),
                )
            })
        })
        .collect::<Vec<(Entity, Option<nekoland_ecs::components::OutputId>, u64)>>();
    let window_entity_target_outputs = window_target_outputs
        .iter()
        .map(|(entity, target_output, _)| (*entity, *target_output))
        .collect::<HashMap<_, _>>();
    let window_surface_target_outputs = window_target_outputs
        .iter()
        .map(|(_, target_output, surface_id)| (*surface_id, *target_output))
        .collect::<HashMap<_, _>>();
    surfaces
        .iter()
        .map(|(_entity, surface)| {
            let role = if surface.window.is_some() {
                RenderSurfaceRole::Window
            } else if surface.popup.is_some() {
                RenderSurfaceRole::Popup
            } else if surface.layer.is_some() {
                RenderSurfaceRole::Layer
            } else {
                RenderSurfaceRole::Unknown
            };
            let target_output = if surface.window.is_some() {
                surface.background.map(|background| background.output).or_else(|| {
                    surface
                        .viewport_visibility
                        .and_then(|viewport_visibility| viewport_visibility.output)
                })
            } else if surface.popup.is_some() {
                surface.child_of.and_then(|child_of| {
                    window_entity_target_outputs.get(&child_of.parent()).copied().flatten()
                })
            } else if surface.layer.is_some() {
                surface
                    .layer_output
                    .and_then(|layer_output| output_ids.get(&layer_output.0).copied())
                    .or_else(|| {
                        surface
                            .desired_output_name
                            .and_then(|desired_output_name| desired_output_name.0.as_deref())
                            .and_then(|output_name| output_ids_by_name.get(output_name).copied())
                    })
                    .or(primary_output_id)
            } else {
                window_surface_target_outputs.get(&surface.surface_id()).copied().flatten()
            };
            (
                surface.surface_id(),
                RenderSurfaceSnapshot { geometry: surface.geometry.clone(), role, target_output },
            )
        })
        .collect()
}

pub(super) fn render_surface_role_from_presentation(
    role: SurfacePresentationRole,
) -> RenderSurfaceRole {
    match role {
        SurfacePresentationRole::Window | SurfacePresentationRole::OutputBackground => {
            RenderSurfaceRole::Window
        }
        SurfacePresentationRole::Popup => RenderSurfaceRole::Popup,
        SurfacePresentationRole::Layer => RenderSurfaceRole::Layer,
    }
}

pub(super) fn sync_backend_wayland_ingress_system(
    pending_output_events: Res<'_, PendingBackendOutputEvents>,
    pending_output_updates: Res<'_, PendingBackendOutputUpdates>,
    mut wayland_ingress: ResMut<'_, WaylandIngress>,
) {
    wayland_ingress.output_materialization = BackendOutputMaterializationPlan::from_pending_queues(
        &pending_output_events,
        &pending_output_updates,
    )
    .into();
}

pub(super) fn sync_platform_input_events_from_backend_inputs_system(
    pending_backend_inputs: Res<'_, PendingBackendInputEvents>,
    mut platform_input_events: ResMut<'_, PendingPlatformInputEvents>,
) {
    *platform_input_events =
        PendingPlatformInputEvents::from_items(pending_backend_inputs.as_slice().to_vec());
}

pub fn sync_backend_present_inputs_system(
    outputs: BackendOutputQuery<'_, '_>,
    surfaces: BackendPresentSurfaceQuery<'_, '_>,
    wayland_ingress: Option<Res<'_, WaylandIngress>>,
    shell_render_input: Option<Res<'_, ShellRenderInput>>,
    mut present_surface_snapshots: ResMut<'_, nekoland_ecs::resources::PresentSurfaceSnapshotState>,
    mut present_inputs: ResMut<'_, BackendPresentInputs>,
) {
    present_inputs.outputs = collect_output_snapshots(&outputs);
    let primary_output =
        wayland_ingress.as_deref().map(|wayland_ingress| &wayland_ingress.primary_output);
    let surface_presentation = shell_render_input
        .as_deref()
        .map(|shell_render_input| &shell_render_input.surface_presentation);
    present_surface_snapshots.surfaces =
        collect_render_surface_snapshots(&outputs, &surfaces, primary_output, surface_presentation);
}
