//! Projection of backend-owned queues and snapshots into shared ECS boundary resources.

use crate::common::outputs::{
    BackendOutputMaterializationPlan, PendingBackendOutputEvents, PendingBackendOutputUpdates,
    collect_output_snapshots,
};
use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    PendingBackendInputEvents, PendingPlatformInputEvents, RenderSurfaceRole,
    RenderSurfaceSnapshot, ShellRenderInput, SurfacePresentationRole, SurfacePresentationSnapshot,
    WaylandIngress,
};

use super::{BackendOutputQuery, BackendPresentInputs, BackendPresentSurfaceQuery};

fn collect_render_surface_snapshots(
    surfaces: &BackendPresentSurfaceQuery<'_, '_>,
    surface_presentation: &SurfacePresentationSnapshot,
) -> std::collections::BTreeMap<u64, RenderSurfaceSnapshot> {
    surfaces
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

/// Rebuilds backend present inputs and present-surface snapshots from live ECS state.
pub fn sync_backend_present_inputs_system(
    outputs: BackendOutputQuery<'_, '_>,
    surfaces: BackendPresentSurfaceQuery<'_, '_>,
    shell_render_input: Res<'_, ShellRenderInput>,
    mut present_surface_snapshots: ResMut<'_, nekoland_ecs::resources::PresentSurfaceSnapshotState>,
    mut present_inputs: ResMut<'_, BackendPresentInputs>,
) {
    present_inputs.outputs = collect_output_snapshots(&outputs);
    present_surface_snapshots.surfaces =
        collect_render_surface_snapshots(&surfaces, &shell_render_input.surface_presentation);
}
