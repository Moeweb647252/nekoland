use std::marker::PhantomData;

use bevy_ecs::error::Result as BevyResult;
use bevy_ecs::prelude::{Entity, NonSendMut, Res, ResMut, Resource};
use bevy_ecs::system::SystemParam;
use bevy_ecs::world::World;
use nekoland_config::resources::CompositorConfig;
use nekoland_core::prelude::AppMetadata;
use nekoland_ecs::resources::{
    CompiledOutputFrames, CompositorClock, GlobalPointerPosition, PendingBackendInputEvents,
    PendingProtocolInputEvents,
};
use nekoland_ecs::views::{BackendPresentSurfaceRuntime, OutputRuntime};
use nekoland_protocol::ProtocolDmabufSupport;

use crate::components::OutputBackend;
use crate::manager::SharedBackendManager;
use crate::traits::{BackendExtractCtx, OutputSnapshot};

use super::BackendPresentInputs;

#[derive(SystemParam)]
pub(super) struct BackendExtractState<'w, 's> {
    pub app_metadata: Option<Res<'w, AppMetadata>>,
    pub config: Option<Res<'w, CompositorConfig>>,
    pub outputs: Res<'w, BackendPresentInputs>,
    pub pending_backend_inputs: ResMut<'w, PendingBackendInputEvents>,
    pub pending_protocol_inputs: ResMut<'w, PendingProtocolInputEvents>,
    pub pending_output_events: ResMut<'w, crate::common::outputs::PendingBackendOutputEvents>,
    pub pending_output_updates: ResMut<'w, crate::common::outputs::PendingBackendOutputUpdates>,
    pub pending_presentation_events:
        ResMut<'w, nekoland_ecs::resources::PendingOutputPresentationEvents>,
    pub winit_window_state: Option<ResMut<'w, crate::winit::backend::WinitWindowState>>,
    pub _marker: PhantomData<&'s ()>,
}

pub(super) fn sync_protocol_dmabuf_support_system(
    manager: Option<NonSendMut<SharedBackendManager>>,
    dmabuf_support: Option<ResMut<ProtocolDmabufSupport>>,
) -> BevyResult {
    let Some(manager) = manager else {
        return Ok(());
    };
    let Some(mut dmabuf_support) = dmabuf_support else {
        return Ok(());
    };

    let mut next = ProtocolDmabufSupport::default();
    manager.borrow_mut().collect_protocol_dmabuf_support(&mut next)?;

    if *dmabuf_support != next {
        *dmabuf_support = next;
    }

    Ok(())
}

/// Collect backend-originated events and state updates into ECS pending queues.
pub(super) fn backend_extract_system(
    manager: Option<NonSendMut<SharedBackendManager>>,
    state: BackendExtractState<'_, '_>,
) -> BevyResult {
    let Some(manager) = manager else {
        return Ok(());
    };
    let BackendExtractState {
        app_metadata,
        config,
        outputs,
        mut pending_backend_inputs,
        mut pending_protocol_inputs,
        mut pending_output_events,
        mut pending_output_updates,
        mut pending_presentation_events,
        mut winit_window_state,
        ..
    } = state;
    let mut ctx = BackendExtractCtx {
        app_metadata: app_metadata.as_deref(),
        config: config.as_deref(),
        outputs: &outputs.outputs,
        backend_input_events: &mut pending_backend_inputs,
        protocol_input_events: &mut pending_protocol_inputs,
        output_events: &mut pending_output_events,
        output_updates: &mut pending_output_updates,
        presentation_events: &mut pending_presentation_events,
        winit_window_state: winit_window_state.as_deref_mut(),
    };

    manager.borrow_mut().extract_all(&mut ctx).map_err(Into::into)
}

pub fn extract_backend_wayland_subapp_inputs(main_world: &mut World, wayland_world: &mut World) {
    clone_resource_into::<AppMetadata>(main_world, wayland_world);
    clone_resource_into::<CompiledOutputFrames>(main_world, wayland_world);
    clone_default_resource_into::<CompositorClock>(main_world, wayland_world);
    clone_resource_into::<CompositorConfig>(main_world, wayland_world);
    if let Some(shell_render_input) =
        main_world.get_resource::<nekoland_ecs::resources::ShellRenderInput>()
    {
        wayland_world.insert_resource(shell_render_input.pointer.clone());
    } else {
        wayland_world.insert_resource(GlobalPointerPosition::default());
    }

    let mut outputs = main_world.query::<(Entity, OutputRuntime, Option<&OutputBackend>)>();
    let output_snapshots = outputs
        .iter(main_world)
        .map(|(_, output, owner)| OutputSnapshot {
            output_id: output.id(),
            backend_id: owner.map(|owner| owner.backend_id),
            backend_output_id: owner.map(|owner| owner.output_id.clone()),
            device: output.device.clone(),
            properties: output.properties.clone(),
        })
        .collect();
    wayland_world.insert_resource(BackendPresentInputs { outputs: output_snapshots });

    let mut surfaces = main_world.query::<(Entity, BackendPresentSurfaceRuntime)>();
    let primary_output = main_world
        .get_resource::<nekoland_ecs::resources::WaylandIngress>()
        .map(|wayland_ingress| wayland_ingress.primary_output.clone());
    let surface_presentation = main_world
        .get_resource::<nekoland_ecs::resources::ShellRenderInput>()
        .map(|shell_render_input| &shell_render_input.surface_presentation)
        .cloned()
        .unwrap_or_default();
    let output_ids = outputs
        .iter(main_world)
        .map(|(entity, output, _)| (entity, output.id()))
        .collect::<std::collections::HashMap<_, _>>();
    let output_ids_by_name = outputs
        .iter(main_world)
        .map(|(_, output, _)| (output.name().to_owned(), output.id()))
        .collect::<std::collections::HashMap<_, _>>();
    let primary_output_id = primary_output.and_then(|primary_output| primary_output.id);

    let present_surfaces: std::collections::BTreeMap<
        u64,
        nekoland_ecs::resources::RenderSurfaceSnapshot,
    > = {
        surfaces
            .iter(main_world)
            .filter_map(|(_, surface)| {
                surface_presentation.surfaces.get(&surface.surface_id()).map(|state| {
                    (
                        surface.surface_id(),
                        nekoland_ecs::resources::RenderSurfaceSnapshot {
                            geometry: state.geometry.clone(),
                            role: super::normalize::render_surface_role_from_presentation(
                                state.role,
                            ),
                            target_output: state.target_output,
                        },
                    )
                })
            })
            .collect()
    };
    let present_surfaces = if present_surfaces.is_empty() {
        let window_target_outputs = surfaces
            .iter(main_world)
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
            .collect::<std::collections::HashMap<_, _>>();
        let window_surface_target_outputs = window_target_outputs
            .iter()
            .map(|(_, target_output, surface_id)| (*surface_id, *target_output))
            .collect::<std::collections::HashMap<_, _>>();

        surfaces
            .iter(main_world)
            .map(|(_entity, surface)| {
                let role = if surface.window.is_some() {
                    nekoland_ecs::resources::RenderSurfaceRole::Window
                } else if surface.popup.is_some() {
                    nekoland_ecs::resources::RenderSurfaceRole::Popup
                } else if surface.layer.is_some() {
                    nekoland_ecs::resources::RenderSurfaceRole::Layer
                } else {
                    nekoland_ecs::resources::RenderSurfaceRole::Unknown
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
                                .and_then(|output_name| {
                                    output_ids_by_name.get(output_name).copied()
                                })
                        })
                        .or(primary_output_id)
                } else {
                    window_surface_target_outputs.get(&surface.surface_id()).copied().flatten()
                };
                (
                    surface.surface_id(),
                    nekoland_ecs::resources::RenderSurfaceSnapshot {
                        geometry: surface.geometry.clone(),
                        role,
                        target_output,
                    },
                )
            })
            .collect()
    } else {
        present_surfaces
    };
    wayland_world.insert_resource(nekoland_ecs::resources::PresentSurfaceSnapshotState {
        surfaces: present_surfaces,
    });
}

fn clone_resource_into<R>(source: &World, dest: &mut World)
where
    R: Resource + Clone,
{
    if let Some(resource) = source.get_resource::<R>() {
        dest.insert_resource(resource.clone());
    }
}

fn clone_default_resource_into<R>(source: &World, dest: &mut World)
where
    R: Resource + Clone + Default + PartialEq,
{
    let should_seed = dest.get_resource::<R>().is_none_or(|existing| *existing == R::default());
    if should_seed {
        clone_resource_into::<R>(source, dest);
    }
}
