use std::collections::HashMap;
use std::marker::PhantomData;

use bevy_app::App;
use bevy_ecs::error::Result as BevyResult;
use bevy_ecs::prelude::{Entity, NonSend, NonSendMut, Query, Res, ResMut};
use bevy_ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy_ecs::system::SystemParam;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::prelude::AppMetadata;
use nekoland_core::schedules::{ExtractSchedule, PresentSchedule};
use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
use nekoland_ecs::resources::{
    BackendOutputRegistry, CompositorClock, CompositorConfig, FocusedOutputState,
    GlobalPointerPosition, OutputDamageRegions, OutputPresentationState, PendingBackendInputEvents,
    PendingOutputControls, PendingOutputPresentationEvents, PendingOutputServerRequests,
    PendingProtocolInputEvents, PresentAuditState, PrimaryOutputState, RenderPassGraph, RenderPlan,
    SurfacePresentationRole, SurfacePresentationSnapshot, VirtualOutputCaptureState,
};
use nekoland_ecs::views::{BackendPresentSurfaceRuntime, OutputRuntime};
use nekoland_protocol::{
    ProtocolDmabufSupport, ProtocolSeatDispatchSystems, ProtocolSurfaceRegistry,
};

use crate::common::outputs::{
    PendingBackendOutputEvents, PendingBackendOutputUpdates, RememberedOutputViewportState,
    apply_backend_output_updates_system, apply_output_control_requests_system,
    apply_output_server_requests_system, collect_output_snapshots,
    remember_output_viewports_system, sync_configured_outputs_system,
    sync_output_layout_state_system, sync_primary_output_state_system,
    synchronize_backend_outputs_system,
};
use crate::common::presentation::apply_output_presentation_events_system;
use crate::common::render_order::snapshot_present_audit_outputs;
use crate::components::OutputBackend;
use crate::manager::{BackendManager, BackendStatus};
use crate::traits::{
    BackendApplyCtx, BackendExtractCtx, BackendPresentCtx, RenderSurfaceRole, RenderSurfaceSnapshot,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct BackendPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BackendPresentSystems;

type BackendOutputQuery<'w, 's> =
    Query<'w, 's, (Entity, OutputRuntime, Option<&'static OutputBackend>)>;
type BackendPresentSurfaceQuery<'w, 's> = Query<'w, 's, (Entity, BackendPresentSurfaceRuntime)>;

#[derive(SystemParam)]
struct BackendExtractState<'w, 's> {
    app_metadata: Option<Res<'w, AppMetadata>>,
    config: Option<Res<'w, CompositorConfig>>,
    pending_backend_inputs: ResMut<'w, PendingBackendInputEvents>,
    pending_protocol_inputs: ResMut<'w, PendingProtocolInputEvents>,
    pending_output_events: ResMut<'w, PendingBackendOutputEvents>,
    pending_output_updates: ResMut<'w, PendingBackendOutputUpdates>,
    pending_presentation_events: ResMut<'w, PendingOutputPresentationEvents>,
    winit_window_state: Option<ResMut<'w, crate::winit::backend::WinitWindowState>>,
    _marker: PhantomData<&'s ()>,
}

#[derive(SystemParam)]
struct BackendPresentState<'w, 's> {
    config: Option<Res<'w, CompositorConfig>>,
    clock: Option<Res<'w, CompositorClock>>,
    pointer: Option<Res<'w, GlobalPointerPosition>>,
    primary_output: Option<Res<'w, PrimaryOutputState>>,
    output_damage_regions: Res<'w, OutputDamageRegions>,
    surface_presentation: Option<Res<'w, SurfacePresentationSnapshot>>,
    render_graph: Res<'w, RenderPassGraph>,
    render_plan: Res<'w, RenderPlan>,
    present_audit: ResMut<'w, PresentAuditState>,
    surface_registry: Option<NonSend<'w, ProtocolSurfaceRegistry>>,
    virtual_output_capture: ResMut<'w, VirtualOutputCaptureState>,
    _marker: PhantomData<&'s ()>,
}

impl NekolandPlugin for BackendPlugin {
    /// Register backend resources plus the extract/apply/present pipeline that
    /// keeps runtime backends in sync with ECS state.
    fn build(&self, app: &mut App) {
        let manager = BackendManager::bootstrap(app);

        app.insert_resource(BackendStatus::default())
            .insert_resource(BackendOutputRegistry::default())
            .insert_resource(RememberedOutputViewportState::default())
            .init_resource::<PresentAuditState>()
            .init_resource::<VirtualOutputCaptureState>()
            .init_resource::<PendingBackendInputEvents>()
            .init_resource::<PendingProtocolInputEvents>()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<PendingOutputPresentationEvents>()
            .init_resource::<OutputPresentationState>()
            .init_resource::<PrimaryOutputState>()
            .init_resource::<FocusedOutputState>()
            .init_resource::<PendingBackendOutputEvents>()
            .init_resource::<PendingBackendOutputUpdates>()
            .insert_non_send_resource(manager)
            .add_message::<OutputConnected>()
            .add_message::<OutputDisconnected>()
            .add_systems(
                ExtractSchedule,
                (
                    sync_configured_outputs_system,
                    sync_protocol_dmabuf_support_system,
                    backend_extract_system,
                    synchronize_backend_outputs_system,
                    apply_backend_output_updates_system,
                    apply_output_control_requests_system,
                    apply_output_server_requests_system,
                    sync_output_layout_state_system,
                    remember_output_viewports_system,
                    sync_primary_output_state_system,
                    backend_apply_system,
                    sync_backend_status_system,
                    apply_output_presentation_events_system,
                )
                    .chain(),
            )
            .configure_sets(
                PresentSchedule,
                BackendPresentSystems.after(ProtocolSeatDispatchSystems),
            )
            .add_systems(PresentSchedule, backend_present_system.in_set(BackendPresentSystems));
    }
}

fn sync_protocol_dmabuf_support_system(
    mut manager: NonSendMut<BackendManager>,
    dmabuf_support: Option<ResMut<ProtocolDmabufSupport>>,
) -> BevyResult {
    let Some(mut dmabuf_support) = dmabuf_support else {
        return Ok(());
    };

    let mut next = ProtocolDmabufSupport::default();
    manager.collect_protocol_dmabuf_support(&mut next)?;

    if *dmabuf_support != next {
        *dmabuf_support = next;
    }

    Ok(())
}

/// Collect backend-originated events and state updates into ECS pending queues.
fn backend_extract_system(
    mut manager: NonSendMut<BackendManager>,
    outputs: BackendOutputQuery<'_, '_>,
    state: BackendExtractState<'_, '_>,
) -> BevyResult {
    let BackendExtractState {
        app_metadata,
        config,
        mut pending_backend_inputs,
        mut pending_protocol_inputs,
        mut pending_output_events,
        mut pending_output_updates,
        mut pending_presentation_events,
        mut winit_window_state,
        ..
    } = state;
    let output_snapshots = collect_output_snapshots(&outputs);
    let mut ctx = BackendExtractCtx {
        app_metadata: app_metadata.as_deref(),
        config: config.as_deref(),
        outputs: &output_snapshots,
        backend_input_events: &mut pending_backend_inputs,
        protocol_input_events: &mut pending_protocol_inputs,
        output_events: &mut pending_output_events,
        output_updates: &mut pending_output_updates,
        presentation_events: &mut pending_presentation_events,
        winit_window_state: winit_window_state.as_deref_mut(),
    };

    manager.extract_all(&mut ctx).map_err(Into::into)
}

/// Let backends consume already-normalized ECS state before presentation.
fn backend_apply_system(
    mut manager: NonSendMut<BackendManager>,
    app_metadata: Option<Res<AppMetadata>>,
    config: Option<Res<CompositorConfig>>,
    outputs: BackendOutputQuery<'_, '_>,
    winit_window_state: Option<ResMut<crate::winit::backend::WinitWindowState>>,
) -> BevyResult {
    let output_snapshots = collect_output_snapshots(&outputs);
    let mut winit_window_state = winit_window_state;
    let mut ctx = BackendApplyCtx {
        app_metadata: app_metadata.as_deref(),
        config: config.as_deref(),
        outputs: &output_snapshots,
        winit_window_state: winit_window_state.as_deref_mut(),
    };

    manager.apply_all(&mut ctx).map_err(Into::into)
}

/// Let backends present the current render plan using backend-specific surfaces.
fn backend_present_system(
    mut manager: NonSendMut<BackendManager>,
    outputs: BackendOutputQuery<'_, '_>,
    surfaces: BackendPresentSurfaceQuery<'_, '_>,
    state: BackendPresentState<'_, '_>,
) -> BevyResult {
    let BackendPresentState {
        config,
        clock,
        pointer,
        primary_output,
        output_damage_regions,
        surface_presentation,
        render_graph,
        render_plan,
        mut present_audit,
        surface_registry,
        mut virtual_output_capture,
        ..
    } = state;
    let output_snapshots = collect_output_snapshots(&outputs);
    let primary_output = primary_output.as_deref();
    let surface_snapshots = if let Some(surface_presentation) = surface_presentation.as_deref() {
        surfaces
            .iter()
            .filter_map(|(_, surface)| {
                surface_presentation.surfaces.get(&surface.surface_id()).map(|state| {
                    (
                        surface.surface_id(),
                        RenderSurfaceSnapshot {
                            geometry: state.geometry.clone(),
                            role: render_surface_role_from_presentation(state.role),
                            target_output: state.target_output.clone(),
                        },
                    )
                })
            })
            .collect::<HashMap<_, _>>()
    } else {
        let output_ids = outputs
            .iter()
            .map(|(entity, output, _)| (entity, output.id()))
            .collect::<HashMap<_, _>>();
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
                                .and_then(|viewport_visibility| viewport_visibility.output.clone())
                        }),
                        surface.surface_id(),
                    )
                })
            })
            .collect::<Vec<(Entity, Option<nekoland_ecs::components::OutputId>, u64)>>();
        let window_entity_target_outputs = window_target_outputs
            .iter()
            .map(|(entity, target_output, _)| (*entity, target_output.clone()))
            .collect::<HashMap<_, _>>();
        let window_surface_target_outputs = window_target_outputs
            .iter()
            .map(|(_, target_output, surface_id)| (*surface_id, target_output.clone()))
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
                            .and_then(|viewport_visibility| viewport_visibility.output.clone())
                    })
                } else if surface.popup.is_some() {
                    surface.child_of.and_then(|child_of| {
                        window_entity_target_outputs.get(&child_of.parent()).cloned().flatten()
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
                    window_surface_target_outputs.get(&surface.surface_id()).cloned().flatten()
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
            .collect::<HashMap<_, _>>()
    };

    let mut ctx = BackendPresentCtx {
        config: config.as_deref(),
        clock: clock.as_deref(),
        pointer: pointer.as_deref(),
        output_damage_regions: &output_damage_regions,
        outputs: &output_snapshots,
        render_graph: &render_graph,
        render_plan: &render_plan,
        surfaces: &surface_snapshots,
        surface_registry: surface_registry.as_deref(),
        virtual_output_capture: Some(&mut virtual_output_capture),
    };

    let (frame, uptime_millis) = clock
        .as_deref()
        .map(|clock| (clock.frame, clock.uptime_millis.min(u128::from(u64::MAX)) as u64))
        .unwrap_or((0, 0));
    present_audit.outputs = snapshot_present_audit_outputs(
        frame,
        uptime_millis,
        &output_snapshots,
        &render_graph,
        &render_plan,
        &surface_snapshots,
    );

    manager.present_all(&mut ctx).map_err(Into::into)
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
fn sync_backend_status_system(manager: NonSend<BackendManager>, mut status: ResMut<BackendStatus>) {
    status.refresh_from_manager(&manager);
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::prelude::{ResMut, Resource};
    use bevy_ecs::schedule::IntoScheduleConfigs;

    use nekoland_core::schedules::{PresentSchedule, install_core_schedules};
    use nekoland_ecs::bundles::OutputBundle;
    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputKind, OutputProperties, SurfaceGeometry, WlSurfaceHandle,
    };
    use nekoland_ecs::resources::{
        CompositorClock, OutputDamageRegions, OutputExecutionPlan, OutputRenderPlan,
        PresentAuditState, RenderItemId, RenderItemIdentity, RenderItemInstance, RenderPassGraph,
        RenderPassId, RenderPassNode, RenderPlan, RenderPlanItem, RenderRect, RenderSceneRole,
        RenderSourceId, RenderTargetId, RenderTargetKind, SurfacePresentationSnapshot,
        SurfacePresentationState, SurfaceRenderItem, VirtualOutputCaptureState,
    };
    use nekoland_protocol::ProtocolSeatDispatchSystems;

    use crate::manager::BackendManager;

    use super::{BackendPresentSystems, backend_present_system};

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
    fn backend_present_system_populates_multi_output_present_audit() {
        let mut app = App::new();
        install_core_schedules(&mut app);
        app.insert_non_send_resource(BackendManager::default())
            .insert_resource(CompositorClock { frame: 7, uptime_millis: 1234 })
            .init_resource::<OutputDamageRegions>()
            .init_resource::<PresentAuditState>()
            .init_resource::<VirtualOutputCaptureState>()
            .init_resource::<RenderPassGraph>()
            .add_systems(PresentSchedule, backend_present_system);

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
