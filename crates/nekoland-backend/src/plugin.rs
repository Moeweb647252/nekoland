use std::collections::HashMap;
use std::marker::PhantomData;

use bevy_app::App;
use bevy_ecs::error::Result as BevyResult;
use bevy_ecs::prelude::{Entity, NonSend, NonSendMut, Query, Res, ResMut, Resource};
use bevy_ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy_ecs::system::SystemParam;
use nekoland_config::resources::CompositorConfig;
use nekoland_core::app::{
    WaylandApplySystems, WaylandCleanupSystems, WaylandExtractSystems, WaylandFeedbackSystems,
    WaylandNormalizeSystems, WaylandPresentSystems,
};
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::prelude::AppMetadata;
use nekoland_core::schedules::{ExtractSchedule, PresentSchedule, ProtocolSchedule};
use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
use nekoland_ecs::resources::{
    BackendOutputRegistry, CompiledOutputFrames, CompletedScreenshotFrames, CompositorClock,
    GlobalPointerPosition, PendingBackendInputEvents, PendingPlatformInputEvents,
    PendingProtocolInputEvents, PendingScreenshotRequests, PlatformImportDiagnosticsState,
    PresentAuditState,
    PresentSurfaceSnapshotState, PrimaryOutputState, RenderSurfaceRole, RenderSurfaceSnapshot,
    ShellRenderInput, SurfacePresentationRole, SurfacePresentationSnapshot,
    VirtualOutputCaptureState, WaylandFeedback, WaylandIngress,
};
use nekoland_ecs::views::{BackendPresentSurfaceRuntime, OutputRuntime};
use nekoland_protocol::{
    ProtocolDmabufSupport, ProtocolSeatDispatchSystems, ProtocolSurfaceRegistry,
    resources::{OutputPresentationState, PendingOutputPresentationEvents},
};

use crate::common::outputs::{
    BackendOutputMaterializationPlan, PendingBackendOutputEvents, PendingBackendOutputUpdates,
    RememberedOutputViewportState, apply_output_control_requests_system,
    apply_output_overlay_controls_system, apply_output_server_requests_system,
    collect_output_snapshots, remember_output_viewports_system, sync_configured_outputs_system,
    sync_output_layout_state_system, sync_output_snapshot_state_from_present_inputs_system,
    sync_primary_output_state_from_present_inputs_system,
};
use crate::common::presentation::apply_output_presentation_events_system;
use crate::common::render_order::snapshot_present_audit_outputs;
use crate::components::OutputBackend;
use crate::manager::{BackendManager, BackendStatus, SharedBackendManager};
use crate::traits::{BackendApplyCtx, BackendExtractCtx, BackendPresentCtx};

#[derive(Debug, Default, Clone, Copy)]
pub struct BackendPlugin;

#[derive(Debug, Default, Clone, Copy)]
pub struct BackendWaylandSubAppPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackendPresentSystems;

type BackendOutputQuery<'w, 's> =
    Query<'w, 's, (Entity, OutputRuntime, Option<&'static OutputBackend>)>;
type BackendPresentSurfaceQuery<'w, 's> = Query<'w, 's, (Entity, BackendPresentSurfaceRuntime)>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct BackendPresentInputs {
    outputs: Vec<crate::traits::OutputSnapshot>,
}

impl BackendPresentInputs {
    pub fn from_outputs(outputs: Vec<crate::traits::OutputSnapshot>) -> Self {
        Self { outputs }
    }

    pub fn outputs(&self) -> &[crate::traits::OutputSnapshot] {
        &self.outputs
    }
}

#[derive(SystemParam)]
struct BackendExtractState<'w, 's> {
    app_metadata: Option<Res<'w, AppMetadata>>,
    config: Option<Res<'w, CompositorConfig>>,
    outputs: Res<'w, BackendPresentInputs>,
    pending_backend_inputs: ResMut<'w, PendingBackendInputEvents>,
    pending_protocol_inputs: ResMut<'w, PendingProtocolInputEvents>,
    pending_output_events: ResMut<'w, PendingBackendOutputEvents>,
    pending_output_updates: ResMut<'w, PendingBackendOutputUpdates>,
    pending_presentation_events: ResMut<'w, PendingOutputPresentationEvents>,
    winit_window_state: Option<ResMut<'w, crate::winit::backend::WinitWindowState>>,
    _marker: PhantomData<&'s ()>,
}

#[derive(SystemParam)]
pub(crate) struct BackendPresentState<'w, 's> {
    config: Option<Res<'w, CompositorConfig>>,
    clock: Option<Res<'w, CompositorClock>>,
    pointer: Option<Res<'w, GlobalPointerPosition>>,
    present_inputs: Res<'w, BackendPresentInputs>,
    present_surfaces: Res<'w, PresentSurfaceSnapshotState>,
    compiled_frames: Res<'w, CompiledOutputFrames>,
    pending_screenshot_requests: ResMut<'w, PendingScreenshotRequests>,
    completed_screenshots: ResMut<'w, CompletedScreenshotFrames>,
    import_diagnostics: Option<ResMut<'w, PlatformImportDiagnosticsState>>,
    present_audit: ResMut<'w, PresentAuditState>,
    surface_registry: Option<NonSend<'w, ProtocolSurfaceRegistry>>,
    virtual_output_capture: ResMut<'w, VirtualOutputCaptureState>,
    _marker: PhantomData<&'s ()>,
}

impl NekolandPlugin for BackendPlugin {
    /// Register backend resources plus the extract/apply/present pipeline that
    /// keeps runtime backends in sync with ECS state.
    fn build(&self, app: &mut App) {
        app.insert_resource(BackendOutputRegistry::default())
            .insert_resource(RememberedOutputViewportState::default())
            .init_resource::<PendingOutputPresentationEvents>()
            .add_message::<OutputConnected>()
            .add_message::<OutputDisconnected>()
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    apply_output_overlay_controls_system,
                    sync_output_layout_state_system,
                    remember_output_viewports_system,
                )
                    .chain(),
            )
            .configure_sets(
                PresentSchedule,
                BackendPresentSystems.after(ProtocolSeatDispatchSystems),
            );
    }
}

impl NekolandPlugin for BackendWaylandSubAppPlugin {
    fn build(&self, app: &mut App) {
        let manager = SharedBackendManager::new(BackendManager::bootstrap(app));
        app.insert_non_send_resource(manager)
            .init_resource::<BackendStatus>()
            .init_resource::<BackendPresentInputs>()
            .init_resource::<PlatformImportDiagnosticsState>()
            .init_resource::<ProtocolDmabufSupport>()
            .init_resource::<PendingBackendInputEvents>()
            .init_resource::<PendingProtocolInputEvents>()
            .init_resource::<PendingBackendOutputEvents>()
            .init_resource::<PendingBackendOutputUpdates>()
            .init_resource::<PendingOutputPresentationEvents>()
            .init_resource::<PresentAuditState>()
            .init_resource::<PresentSurfaceSnapshotState>()
            .init_resource::<PendingScreenshotRequests>()
            .init_resource::<CompletedScreenshotFrames>()
            .init_resource::<VirtualOutputCaptureState>()
            .init_resource::<CompiledOutputFrames>()
            .init_resource::<crate::winit::backend::WinitWindowState>()
            .add_systems(
                ExtractSchedule,
                (sync_protocol_dmabuf_support_system, backend_extract_system)
                    .chain()
                    .in_set(WaylandExtractSystems),
            )
            .add_systems(
                ExtractSchedule,
                (
                    sync_platform_input_events_from_backend_inputs_system,
                    sync_output_snapshot_state_from_present_inputs_system,
                    sync_primary_output_state_from_present_inputs_system,
                )
                    .chain()
                    .in_set(WaylandNormalizeSystems),
            )
            .add_systems(
                ProtocolSchedule,
                (backend_apply_system, sync_backend_status_system)
                    .chain()
                    .in_set(WaylandApplySystems),
            )
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_presentation_events_system,
                    sync_configured_outputs_system,
                    apply_output_server_requests_system,
                    sync_backend_wayland_ingress_system,
                )
                    .chain()
                    .in_set(WaylandApplySystems),
            )
            .add_systems(
                PresentSchedule,
                backend_present_system.in_set(BackendPresentSystems).in_set(WaylandPresentSystems),
            )
            .add_systems(
                PresentSchedule,
                sync_backend_wayland_feedback_system
                    .after(BackendPresentSystems)
                    .in_set(WaylandFeedbackSystems),
            )
            .add_systems(
                PresentSchedule,
                clear_backend_frame_local_queues_system.in_set(WaylandCleanupSystems),
            );
    }
}

fn sync_protocol_dmabuf_support_system(
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
fn backend_extract_system(
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

/// Let backends consume already-normalized ECS state before presentation.
fn backend_apply_system(
    manager: Option<NonSendMut<SharedBackendManager>>,
    app_metadata: Option<Res<AppMetadata>>,
    config: Option<Res<CompositorConfig>>,
    outputs: Res<'_, BackendPresentInputs>,
    winit_window_state: Option<ResMut<crate::winit::backend::WinitWindowState>>,
) -> BevyResult {
    let Some(manager) = manager else {
        return Ok(());
    };
    let mut winit_window_state = winit_window_state;
    let mut ctx = BackendApplyCtx {
        app_metadata: app_metadata.as_deref(),
        config: config.as_deref(),
        outputs: &outputs.outputs,
        winit_window_state: winit_window_state.as_deref_mut(),
    };

    manager.borrow_mut().apply_all(&mut ctx).map_err(Into::into)
}

/// Let backends present the current render plan using backend-specific surfaces.
pub(crate) fn backend_present_system(
    manager: Option<NonSendMut<SharedBackendManager>>,
    state: BackendPresentState<'_, '_>,
) -> BevyResult {
    let Some(manager) = manager else {
        return Ok(());
    };
    let BackendPresentState {
        config,
        clock,
        pointer,
        present_inputs,
        present_surfaces,
        compiled_frames,
        mut pending_screenshot_requests,
        mut completed_screenshots,
        import_diagnostics,
        mut present_audit,
        surface_registry,
        mut virtual_output_capture,
        ..
    } = state;

    let mut import_diagnostics = import_diagnostics;
    if let Some(diagnostics) = import_diagnostics.as_deref_mut() {
        diagnostics.clear();
    }

    let mut ctx = BackendPresentCtx {
        config: config.as_deref(),
        clock: clock.as_deref(),
        pointer: pointer.as_deref(),
        outputs: &present_inputs.outputs,
        compiled_frames: &compiled_frames,
        pending_screenshot_requests: &mut pending_screenshot_requests,
        completed_screenshots: &mut completed_screenshots,
        surfaces: &present_surfaces.surfaces,
        surface_registry: surface_registry.as_deref(),
        virtual_output_capture: Some(&mut virtual_output_capture),
        import_diagnostics: import_diagnostics.as_deref_mut(),
    };

    let (frame, uptime_millis) = clock
        .as_deref()
        .map(|clock| (clock.frame, clock.uptime_millis.min(u128::from(u64::MAX)) as u64))
        .unwrap_or((0, 0));
    present_audit.outputs = snapshot_present_audit_outputs(
        frame,
        uptime_millis,
        &present_inputs.outputs,
        &compiled_frames,
        &present_surfaces.surfaces,
    );

    manager.borrow_mut().present_all(&mut ctx).map_err(Into::into)
}

fn sync_backend_wayland_feedback_system(
    pending_screenshot_requests: Res<'_, PendingScreenshotRequests>,
    completed_screenshots: Res<'_, CompletedScreenshotFrames>,
    backend_status: Res<'_, BackendStatus>,
    import_diagnostics: Res<'_, PlatformImportDiagnosticsState>,
    output_presentation: Res<'_, OutputPresentationState>,
    present_audit: Res<'_, PresentAuditState>,
    virtual_output_capture: Res<'_, VirtualOutputCaptureState>,
    mut wayland_feedback: ResMut<'_, WaylandFeedback>,
) {
    wayland_feedback.platform_backends = backend_status.platform_state();
    wayland_feedback.import_diagnostics = import_diagnostics.clone();
    wayland_feedback.pending_screenshot_requests = pending_screenshot_requests.clone();
    wayland_feedback.completed_screenshots = completed_screenshots.clone();
    wayland_feedback.output_presentation = output_presentation.clone();
    wayland_feedback.present_audit = present_audit.clone();
    wayland_feedback.virtual_output_capture = virtual_output_capture.clone();
}

fn sync_backend_wayland_ingress_system(
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

fn sync_platform_input_events_from_backend_inputs_system(
    pending_backend_inputs: Res<'_, PendingBackendInputEvents>,
    mut platform_input_events: ResMut<'_, PendingPlatformInputEvents>,
) {
    *platform_input_events =
        PendingPlatformInputEvents::from_items(pending_backend_inputs.as_slice().to_vec());
}

fn clear_backend_frame_local_queues_system(
    mut pending_backend_inputs: ResMut<'_, PendingBackendInputEvents>,
    mut pending_protocol_inputs: ResMut<'_, PendingProtocolInputEvents>,
    mut pending_output_events: ResMut<'_, PendingBackendOutputEvents>,
    mut pending_output_updates: ResMut<'_, PendingBackendOutputUpdates>,
) {
    *pending_backend_inputs = PendingBackendInputEvents::default();
    *pending_protocol_inputs = PendingProtocolInputEvents::default();
    *pending_output_events = PendingBackendOutputEvents::default();
    *pending_output_updates = PendingBackendOutputUpdates::default();
}

pub fn sync_backend_present_inputs_system(
    outputs: BackendOutputQuery<'_, '_>,
    surfaces: BackendPresentSurfaceQuery<'_, '_>,
    wayland_ingress: Option<Res<'_, WaylandIngress>>,
    shell_render_input: Option<Res<'_, ShellRenderInput>>,
    mut present_surface_snapshots: ResMut<'_, PresentSurfaceSnapshotState>,
    mut present_inputs: ResMut<'_, BackendPresentInputs>,
) {
    present_inputs.outputs = collect_output_snapshots(&outputs);
    let primary_output = wayland_ingress
        .as_deref()
        .map(|wayland_ingress| &wayland_ingress.primary_output);
    let surface_presentation = shell_render_input
        .as_deref()
        .map(|shell_render_input| &shell_render_input.surface_presentation);
    present_surface_snapshots.surfaces = collect_render_surface_snapshots(
        &outputs,
        &surfaces,
        primary_output,
        surface_presentation,
    );
}

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

fn render_surface_role_from_presentation(role: SurfacePresentationRole) -> RenderSurfaceRole {
    match role {
        SurfacePresentationRole::Window | SurfacePresentationRole::OutputBackground => {
            RenderSurfaceRole::Window
        }
        SurfacePresentationRole::Popup => RenderSurfaceRole::Popup,
        SurfacePresentationRole::Layer => RenderSurfaceRole::Layer,
    }
}

/// Refresh the public backend-status resource from the installed backend manager.
fn sync_backend_status_system(
    manager: Option<NonSend<SharedBackendManager>>,
    mut status: ResMut<BackendStatus>,
) {
    let Some(manager) = manager else {
        return;
    };
    status.refresh_from_manager(&manager.borrow());
}

pub fn extract_backend_wayland_subapp_inputs(
    main_world: &mut bevy_ecs::world::World,
    wayland_world: &mut bevy_ecs::world::World,
) {
    clone_resource_into::<AppMetadata>(main_world, wayland_world);
    clone_resource_into::<CompiledOutputFrames>(main_world, wayland_world);
    clone_default_resource_into::<CompositorClock>(main_world, wayland_world);
    clone_resource_into::<CompositorConfig>(main_world, wayland_world);
    if let Some(shell_render_input) = main_world.get_resource::<ShellRenderInput>() {
        wayland_world.insert_resource(shell_render_input.pointer.clone());
    } else {
        wayland_world.insert_resource(GlobalPointerPosition::default());
    }

    let mut outputs = main_world.query::<(Entity, OutputRuntime, Option<&OutputBackend>)>();
    let output_snapshots = outputs
        .iter(main_world)
        .map(|(_, output, owner)| crate::traits::OutputSnapshot {
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
        .get_resource::<WaylandIngress>()
        .map(|wayland_ingress| wayland_ingress.primary_output.clone());
    let surface_presentation = main_world
        .get_resource::<ShellRenderInput>()
        .map(|shell_render_input| &shell_render_input.surface_presentation)
        .cloned()
        .unwrap_or_default();
    let output_ids = outputs
        .iter(main_world)
        .map(|(entity, output, _)| (entity, output.id()))
        .collect::<HashMap<_, _>>();
    let output_ids_by_name = outputs
        .iter(main_world)
        .map(|(_, output, _)| (output.name().to_owned(), output.id()))
        .collect::<HashMap<_, _>>();
    let primary_output_id = primary_output.and_then(|primary_output| primary_output.id);

    let present_surfaces: std::collections::BTreeMap<u64, RenderSurfaceSnapshot> = {
        surfaces
            .iter(main_world)
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
            .collect::<HashMap<_, _>>();
        let window_surface_target_outputs = window_target_outputs
            .iter()
            .map(|(_, target_output, surface_id)| (*surface_id, *target_output))
            .collect::<HashMap<_, _>>();

        surfaces
            .iter(main_world)
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
                    RenderSurfaceSnapshot {
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

    wayland_world.insert_resource(PresentSurfaceSnapshotState { surfaces: present_surfaces });
}

fn clone_resource_into<R>(source: &bevy_ecs::world::World, dest: &mut bevy_ecs::world::World)
where
    R: Resource + Clone,
{
    if let Some(resource) = source.get_resource::<R>() {
        dest.insert_resource(resource.clone());
    }
}

fn clone_default_resource_into<R>(
    source: &bevy_ecs::world::World,
    dest: &mut bevy_ecs::world::World,
) where
    R: Resource + Clone + Default + PartialEq,
{
    let should_seed = dest.get_resource::<R>().is_none_or(|existing| *existing == R::default());
    if should_seed {
        clone_resource_into::<R>(source, dest);
    }
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::prelude::{ResMut, Resource};
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use bevy_ecs::system::RunSystemOnce;
    use bevy_ecs::world::World;

    use nekoland_core::schedules::{ExtractSchedule, PresentSchedule, install_core_schedules};
    use nekoland_ecs::bundles::OutputBundle;
    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputKind, OutputProperties, SurfaceGeometry, WlSurfaceHandle,
    };
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, CompiledOutputFrames, CompletedScreenshotFrames,
        CompositorClock, OutputDamageRegions, OutputExecutionPlan, OutputPresentationState,
        OutputProcessPlan, OutputRenderPlan, PendingBackendInputEvents,
        PendingPlatformInputEvents, PendingProtocolInputEvents, PendingScreenshotRequests,
        PlatformImportDiagnostic, PlatformImportDiagnosticsState, PlatformImportFailureStage,
        PresentAuditState, PresentSurfaceSnapshotState, RenderItemId, RenderItemIdentity,
        RenderItemInstance, RenderMaterialFrameState, RenderPassGraph, RenderPassId,
        RenderPassNode, RenderPlan, RenderPlanItem, RenderProcessPlan, RenderRect,
        RenderSceneRole, RenderSourceId, RenderTargetId, RenderTargetKind,
        SurfacePresentationSnapshot, SurfacePresentationState, SurfaceRenderItem,
        VirtualOutputCaptureState, WaylandFeedback, WaylandIngress,
    };
    use nekoland_protocol::ProtocolSeatDispatchSystems;

    use crate::common::outputs::{
        BackendOutputBlueprint, BackendOutputChange, BackendOutputEventRecord,
        BackendOutputMaterializationPlan, BackendOutputPropertyUpdate, PendingBackendOutputEvents,
        PendingBackendOutputUpdates,
    };
    use crate::manager::{BackendManager, BackendStatus, SharedBackendManager};

    use super::{
        BackendPresentInputs, BackendPresentSystems, backend_present_system,
        clear_backend_frame_local_queues_system, sync_backend_present_inputs_system,
        sync_backend_wayland_feedback_system, sync_backend_wayland_ingress_system,
        sync_platform_input_events_from_backend_inputs_system,
    };

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
    }

    #[derive(Debug, Default, Resource)]
    struct PresentOrderAudit(Vec<&'static str>);

    fn record_protocol_present(mut audit: ResMut<PresentOrderAudit>) {
        audit.0.push("protocol");
    }

    fn record_backend_present(mut audit: ResMut<PresentOrderAudit>) {
        audit.0.push("backend");
    }

    #[test]
    fn backend_present_systems_run_after_protocol_seat_dispatch_systems() {
        let mut app = App::new();
        install_core_schedules(&mut app);
        app.init_resource::<PresentOrderAudit>()
            .configure_sets(
                PresentSchedule,
                BackendPresentSystems.after(ProtocolSeatDispatchSystems),
            )
            .add_systems(
                PresentSchedule,
                record_protocol_present.in_set(ProtocolSeatDispatchSystems),
            )
            .add_systems(PresentSchedule, record_backend_present.in_set(BackendPresentSystems));

        app.world_mut().run_schedule(PresentSchedule);

        let Some(audit) = app.world().get_resource::<PresentOrderAudit>() else {
            panic!("present order audit should exist");
        };
        assert_eq!(audit.0, vec!["protocol", "backend"]);
    }

    #[test]
    fn backend_wayland_ingress_sync_exports_output_materialization_plan() {
        let mut world = World::default();
        let mut pending_output_events = PendingBackendOutputEvents::default();
        pending_output_events.push(BackendOutputEventRecord {
            backend_id: crate::traits::BackendId(1),
            output_name: "DP-1".to_owned(),
            local_id: "nested-0".to_owned(),
            change: BackendOutputChange::Connected(BackendOutputBlueprint {
                local_id: "nested-0".to_owned(),
                device: OutputDevice {
                    name: "DP-1".to_owned(),
                    kind: OutputKind::Nested,
                    make: "Nekoland".to_owned(),
                    model: "dp".to_owned(),
                },
                properties: OutputProperties {
                    width: 2560,
                    height: 1440,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
            }),
        });
        let mut pending_output_updates = PendingBackendOutputUpdates::default();
        pending_output_updates.push(BackendOutputPropertyUpdate {
            backend_id: crate::traits::BackendId(1),
            output_name: "DP-1".to_owned(),
            local_id: "nested-0".to_owned(),
            properties: OutputProperties {
                width: 1920,
                height: 1080,
                refresh_millihz: 59_940,
                scale: 2,
            },
        });
        let materialization = BackendOutputMaterializationPlan::from_pending_queues(
            &pending_output_events,
            &pending_output_updates,
        );
        world.insert_resource(pending_output_events);
        world.insert_resource(pending_output_updates);
        world.insert_resource(WaylandIngress::default());

        let Ok(()) = world.run_system_once(sync_backend_wayland_ingress_system) else {
            panic!("backend ingress sync should run");
        };

        let ingress = world.resource::<WaylandIngress>();
        assert_eq!(ingress.output_materialization, materialization.into());
    }

    #[test]
    fn backend_wayland_cleanup_clears_frame_local_runtime_queues() {
        let mut app = App::new();
        install_core_schedules(&mut app);
        app.init_resource::<PendingBackendInputEvents>()
            .init_resource::<PendingProtocolInputEvents>()
            .init_resource::<PendingBackendOutputEvents>()
            .init_resource::<PendingBackendOutputUpdates>()
            .add_systems(PresentSchedule, clear_backend_frame_local_queues_system);

        app.world_mut().resource_mut::<PendingBackendInputEvents>().push(BackendInputEvent {
            device: "seat-0".to_owned(),
            action: BackendInputAction::FocusChanged { focused: true },
        });
        app.world_mut().resource_mut::<PendingProtocolInputEvents>().push(BackendInputEvent {
            device: "seat-0".to_owned(),
            action: BackendInputAction::Key { keycode: 1, pressed: true },
        });
        app.world_mut().resource_mut::<PendingBackendOutputEvents>().push(
            BackendOutputEventRecord {
                backend_id: crate::traits::BackendId(1),
                output_name: "DP-1".to_owned(),
                local_id: "nested-0".to_owned(),
                change: BackendOutputChange::Disconnected,
            },
        );
        app.world_mut().resource_mut::<PendingBackendOutputUpdates>().push(
            BackendOutputPropertyUpdate {
                backend_id: crate::traits::BackendId(1),
                output_name: "DP-1".to_owned(),
                local_id: "nested-0".to_owned(),
                properties: OutputProperties {
                    width: 1920,
                    height: 1080,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
            },
        );

        app.world_mut().run_schedule(PresentSchedule);

        assert!(app.world().resource::<PendingBackendInputEvents>().is_empty());
        assert!(app.world().resource::<PendingProtocolInputEvents>().is_empty());
        assert!(app.world().resource::<PendingBackendOutputEvents>().is_empty());
        assert!(app.world().resource::<PendingBackendOutputUpdates>().is_empty());
    }

    #[test]
    fn backend_normalize_mirrors_backend_input_events_into_platform_input_mailbox() {
        let mut app = App::new();
        install_core_schedules(&mut app);
        app.init_resource::<PendingBackendInputEvents>()
            .init_resource::<PendingPlatformInputEvents>()
            .add_systems(ExtractSchedule, sync_platform_input_events_from_backend_inputs_system);

        app.world_mut().resource_mut::<PendingBackendInputEvents>().push(BackendInputEvent {
            device: "seat-0".to_owned(),
            action: BackendInputAction::PointerMoved { x: 128.0, y: 64.0 },
        });

        app.world_mut().run_schedule(ExtractSchedule);

        assert_eq!(
            app.world().resource::<PendingPlatformInputEvents>().as_slice(),
            &[BackendInputEvent {
                device: "seat-0".to_owned(),
                action: BackendInputAction::PointerMoved { x: 128.0, y: 64.0 },
            }]
        );
    }

    #[test]
    fn backend_feedback_mirrors_import_diagnostics_into_wayland_feedback() {
        let mut app = App::new();
        install_core_schedules(&mut app);
        app.init_resource::<PendingScreenshotRequests>()
            .init_resource::<CompletedScreenshotFrames>()
            .init_resource::<BackendStatus>()
            .init_resource::<PlatformImportDiagnosticsState>()
            .init_resource::<OutputPresentationState>()
            .init_resource::<PresentAuditState>()
            .init_resource::<VirtualOutputCaptureState>()
            .init_resource::<WaylandFeedback>()
            .add_systems(PresentSchedule, sync_backend_wayland_feedback_system);

        app.world_mut().resource_mut::<PlatformImportDiagnosticsState>().entries.push(
            PlatformImportDiagnostic {
                output_name: "DP-1".to_owned(),
                surface_id: Some(44),
                strategy: None,
                stage: PlatformImportFailureStage::Present,
                message: "backend advertised dma-buf import but present failed".to_owned(),
            },
        );

        app.world_mut().run_schedule(PresentSchedule);

        assert_eq!(
            app.world().resource::<WaylandFeedback>().import_diagnostics.entries.len(),
            1
        );
        assert_eq!(
            app.world().resource::<WaylandFeedback>().import_diagnostics.entries[0].surface_id,
            Some(44)
        );
    }

    #[test]
    fn backend_present_system_populates_multi_output_present_audit() {
        let mut app = App::new();
        install_core_schedules(&mut app);
        app.insert_non_send_resource(SharedBackendManager::new(BackendManager::default()))
            .insert_resource(CompositorClock { frame: 7, uptime_millis: 1234 })
            .init_resource::<CompiledOutputFrames>()
            .init_resource::<OutputDamageRegions>()
            .init_resource::<PresentAuditState>()
            .init_resource::<VirtualOutputCaptureState>()
            .init_resource::<BackendPresentInputs>()
            .init_resource::<PresentSurfaceSnapshotState>()
            .init_resource::<PendingScreenshotRequests>()
            .init_resource::<CompletedScreenshotFrames>()
            .init_resource::<RenderMaterialFrameState>()
            .init_resource::<RenderPassGraph>()
            .add_systems(
                PresentSchedule,
                (sync_backend_present_inputs_system, backend_present_system).chain(),
            );

        let hdmi = app
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "HDMI-A-1".to_owned(),
                    kind: OutputKind::Nested,
                    make: "Nekoland".to_owned(),
                    model: "hdmi".to_owned(),
                },
                properties: OutputProperties {
                    width: 1920,
                    height: 1080,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let dp = app
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "DP-1".to_owned(),
                    kind: OutputKind::Nested,
                    make: "Nekoland".to_owned(),
                    model: "dp".to_owned(),
                },
                properties: OutputProperties {
                    width: 2560,
                    height: 1440,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let hdmi_id = app.world().get::<OutputId>(hdmi).copied().expect("hdmi output id");
        let dp_id = app.world().get::<OutputId>(dp).copied().expect("dp output id");
        app.world_mut().insert_resource(SurfacePresentationSnapshot {
            surfaces: std::collections::BTreeMap::from([
                (
                    11,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(hdmi_id),
                        geometry: SurfaceGeometry { x: 10, y: 20, width: 300, height: 200 },
                        input_enabled: true,
                        damage_enabled: true,
                        role: nekoland_ecs::resources::SurfacePresentationRole::Window,
                    },
                ),
                (
                    22,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(dp_id),
                        geometry: SurfaceGeometry { x: 40, y: 50, width: 320, height: 240 },
                        input_enabled: true,
                        damage_enabled: true,
                        role: nekoland_ecs::resources::SurfacePresentationRole::Window,
                    },
                ),
                (
                    33,
                    SurfacePresentationState {
                        visible: true,
                        target_output: None,
                        geometry: SurfaceGeometry { x: 70, y: 80, width: 128, height: 96 },
                        input_enabled: true,
                        damage_enabled: true,
                        role: nekoland_ecs::resources::SurfacePresentationRole::Layer,
                    },
                ),
            ]),
        });

        app.world_mut().spawn((
            WlSurfaceHandle { id: 11 },
            SurfaceGeometry { x: 10, y: 20, width: 300, height: 200 },
        ));
        app.world_mut().spawn((
            WlSurfaceHandle { id: 22 },
            SurfaceGeometry { x: 40, y: 50, width: 320, height: 240 },
        ));
        app.world_mut().spawn((
            WlSurfaceHandle { id: 33 },
            SurfaceGeometry { x: 70, y: 80, width: 128, height: 96 },
        ));

        app.world_mut().insert_resource(RenderPlan {
            outputs: std::collections::BTreeMap::from([
                (
                    hdmi_id,
                    OutputRenderPlan::from_items([
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            identity: identity(11),
                            surface_id: 11,
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 10, y: 20, width: 300, height: 200 },
                                opacity: 1.0,
                                clip_rect: None,
                                z_index: 0,
                                scene_role: RenderSceneRole::Desktop,
                            },
                        }),
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            identity: identity(33),
                            surface_id: 33,
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 70, y: 80, width: 128, height: 96 },
                                opacity: 0.5,
                                clip_rect: None,
                                z_index: 1,
                                scene_role: RenderSceneRole::Desktop,
                            },
                        }),
                    ]),
                ),
                (
                    dp_id,
                    OutputRenderPlan::from_items([
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            identity: identity(34),
                            surface_id: 33,
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 70, y: 80, width: 128, height: 96 },
                                opacity: 0.5,
                                clip_rect: None,
                                z_index: 0,
                                scene_role: RenderSceneRole::Desktop,
                            },
                        }),
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            identity: identity(22),
                            surface_id: 22,
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 40, y: 50, width: 320, height: 240 },
                                opacity: 0.7,
                                clip_rect: None,
                                z_index: 2,
                                scene_role: RenderSceneRole::Desktop,
                            },
                        }),
                    ]),
                ),
            ]),
        });
        app.world_mut().insert_resource(RenderPassGraph {
            outputs: std::collections::BTreeMap::from([
                (
                    hdmi_id,
                    OutputExecutionPlan {
                        targets: std::collections::BTreeMap::from([(
                            RenderTargetId(1),
                            RenderTargetKind::OutputSwapchain(hdmi_id),
                        )]),
                        passes: std::collections::BTreeMap::from([(
                            RenderPassId(1),
                            RenderPassNode::scene(
                                RenderSceneRole::Desktop,
                                RenderTargetId(1),
                                Vec::new(),
                                vec![RenderItemId(11), RenderItemId(33)],
                            ),
                        )]),
                        ordered_passes: vec![RenderPassId(1)],
                        terminal_passes: vec![RenderPassId(1)],
                    },
                ),
                (
                    dp_id,
                    OutputExecutionPlan {
                        targets: std::collections::BTreeMap::from([(
                            RenderTargetId(2),
                            RenderTargetKind::OutputSwapchain(dp_id),
                        )]),
                        passes: std::collections::BTreeMap::from([(
                            RenderPassId(2),
                            RenderPassNode::scene(
                                RenderSceneRole::Desktop,
                                RenderTargetId(2),
                                Vec::new(),
                                vec![RenderItemId(34), RenderItemId(22)],
                            ),
                        )]),
                        ordered_passes: vec![RenderPassId(2)],
                        terminal_passes: vec![RenderPassId(2)],
                    },
                ),
            ]),
        });
        app.world_mut().insert_resource(RenderProcessPlan {
            outputs: std::collections::BTreeMap::from([
                (hdmi_id, OutputProcessPlan::default()),
                (dp_id, OutputProcessPlan::default()),
            ]),
        });
        let render_graph = app.world().resource::<RenderPassGraph>().clone();
        let render_plan = app.world().resource::<RenderPlan>().clone();
        let process_plan = app.world().resource::<RenderProcessPlan>().clone();
        app.world_mut().insert_resource(CompiledOutputFrames {
            outputs: std::collections::BTreeMap::from([
                (
                    hdmi_id,
                    nekoland_ecs::resources::CompiledOutputFrame {
                        render_plan: render_plan.outputs[&hdmi_id].clone(),
                        prepared_scene:
                            nekoland_ecs::resources::OutputPreparedSceneResources::default(),
                        execution_plan: render_graph.outputs[&hdmi_id].clone(),
                        process_plan: process_plan.outputs[&hdmi_id].clone(),
                        final_output: None,
                        readback: None,
                        target_allocation: None,
                        gpu_prep: None,
                        damage_regions: Vec::new(),
                    },
                ),
                (
                    dp_id,
                    nekoland_ecs::resources::CompiledOutputFrame {
                        render_plan: render_plan.outputs[&dp_id].clone(),
                        prepared_scene:
                            nekoland_ecs::resources::OutputPreparedSceneResources::default(),
                        execution_plan: render_graph.outputs[&dp_id].clone(),
                        process_plan: process_plan.outputs[&dp_id].clone(),
                        final_output: None,
                        readback: None,
                        target_allocation: None,
                        gpu_prep: None,
                        damage_regions: Vec::new(),
                    },
                ),
            ]),
            output_damage_regions: OutputDamageRegions::default(),
            prepared_scene: nekoland_ecs::resources::PreparedSceneResources::default(),
            materials: RenderMaterialFrameState::default(),
            render_graph,
            render_plan,
            process_plan,
            final_output_plan: nekoland_ecs::resources::RenderFinalOutputPlan::default(),
            readback_plan: nekoland_ecs::resources::RenderReadbackPlan::default(),
            render_target_allocation: nekoland_ecs::resources::RenderTargetAllocationPlan::default(
            ),
            surface_texture_bridge: nekoland_ecs::resources::SurfaceTextureBridgePlan::default(),
            prepared_gpu: nekoland_ecs::resources::PreparedGpuResources::default(),
        });

        app.world_mut().run_schedule(PresentSchedule);

        let audit = app.world().resource::<PresentAuditState>();
        assert_eq!(audit.outputs.len(), 2);

        let hdmi_audit = &audit.outputs[&hdmi_id];
        assert_eq!(hdmi_audit.output_name, "HDMI-A-1");
        assert_eq!(hdmi_audit.frame, 7);
        assert_eq!(hdmi_audit.uptime_millis, 1234);
        assert_eq!(
            hdmi_audit.elements.iter().map(|element| element.surface_id).collect::<Vec<_>>(),
            vec![11, 33]
        );

        let dp_audit = &audit.outputs[&dp_id];
        assert_eq!(dp_audit.output_name, "DP-1");
        assert_eq!(dp_audit.frame, 7);
        assert_eq!(dp_audit.uptime_millis, 1234);
        assert_eq!(
            dp_audit.elements.iter().map(|element| element.surface_id).collect::<Vec<_>>(),
            vec![33, 22]
        );
    }
}
