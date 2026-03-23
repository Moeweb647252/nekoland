use bevy_app::App;
use bevy_ecs::prelude::{ResMut, Resource};
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::system::RunSystemOnce;
use bevy_ecs::world::World;

use nekoland_core::schedules::{ExtractSchedule, PresentSchedule, install_core_schedules};
use nekoland_ecs::bundles::OutputBundle;
use nekoland_ecs::components::{
    OutputDevice, OutputId, OutputKind, OutputProperties, SurfaceGeometry,
    WindowViewportVisibility, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, CompiledOutputFrames, CompletedScreenshotFrames,
    CompositorClock, OutputDamageRegions, OutputExecutionPlan, OutputPresentationState,
    OutputProcessPlan, OutputRenderPlan, PendingBackendInputEvents, PendingPlatformInputEvents,
    PendingProtocolInputEvents, PendingScreenshotRequests, PlatformImportDiagnostic,
    PlatformImportDiagnosticsState, PlatformImportFailureStage, PresentAuditState,
    PresentSurfaceSnapshotState, RenderItemId, RenderItemIdentity, RenderItemInstance,
    RenderMaterialFrameState, RenderPassGraph, RenderPassId, RenderPassNode, RenderPlan,
    RenderPlanItem, RenderProcessPlan, RenderRect, RenderSceneRole, RenderSourceId, RenderTargetId,
    RenderTargetKind, ShellRenderInput, SurfacePresentationSnapshot, SurfacePresentationState,
    SurfaceRenderItem, VirtualOutputCaptureState, WaylandFeedback, WaylandIngress,
};
use nekoland_protocol::ProtocolSeatDispatchSystems;

use crate::common::outputs::{
    BackendOutputBlueprint, BackendOutputChange, BackendOutputEventRecord,
    BackendOutputMaterializationPlan, BackendOutputPropertyUpdate, PendingBackendOutputEvents,
    PendingBackendOutputUpdates,
};
use crate::manager::{BackendManager, BackendStatus, SharedBackendManager};

use super::feedback::{
    clear_backend_frame_local_queues_system, sync_backend_wayland_feedback_system,
};
use super::normalize::{
    sync_backend_present_inputs_system, sync_backend_wayland_ingress_system,
    sync_platform_input_events_from_backend_inputs_system,
};
use super::present::backend_present_system;
use super::{BackendPresentInputs, BackendPresentSystems};

fn identity(id: u64) -> RenderItemIdentity {
    RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
}

#[test]
fn backend_root_facade_exports_remain_usable() {
    let mut main_world = World::default();
    main_world.insert_resource(ShellRenderInput::default());
    let mut wayland_world = World::default();

    crate::extract_backend_wayland_subapp_inputs(&mut main_world, &mut wayland_world);

    let _plugin = crate::BackendPlugin;
    let _subapp_plugin = crate::BackendWaylandSubAppPlugin;
}

#[test]
fn backend_extract_requires_shell_render_boundary_for_present_surface_snapshots() {
    let mut main_world = World::default();
    main_world.insert_resource(ShellRenderInput::default());
    let output = main_world
        .spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "Virtual".to_owned(),
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
    let output_id = *main_world.get::<OutputId>(output).expect("output id");
    main_world.spawn((
        WlSurfaceHandle { id: 77 },
        SurfaceGeometry { x: 40, y: 30, width: 400, height: 300 },
        WindowViewportVisibility { visible: true, output: Some(output_id) },
        XdgWindow::default(),
    ));

    let mut wayland_world = World::default();
    crate::extract_backend_wayland_subapp_inputs(&mut main_world, &mut wayland_world);

    assert!(
        wayland_world.resource::<PresentSurfaceSnapshotState>().surfaces.is_empty(),
        "present surfaces should come from ShellRenderInput rather than reconstructed live state",
    );
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
        .configure_sets(PresentSchedule, BackendPresentSystems.after(ProtocolSeatDispatchSystems))
        .add_systems(PresentSchedule, record_protocol_present.in_set(ProtocolSeatDispatchSystems))
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
    app.world_mut().resource_mut::<PendingBackendOutputEvents>().push(BackendOutputEventRecord {
        backend_id: crate::traits::BackendId(1),
        output_name: "DP-1".to_owned(),
        local_id: "nested-0".to_owned(),
        change: BackendOutputChange::Disconnected,
    });
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
fn backend_normalize_mirrors_backend_input_events_into_platform_input_queue() {
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

    assert_eq!(app.world().resource::<WaylandFeedback>().import_diagnostics.entries.len(), 1);
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
        .init_resource::<ShellRenderInput>()
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
    app.world_mut().insert_resource(ShellRenderInput {
        surface_presentation: SurfacePresentationSnapshot {
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
        },
        ..Default::default()
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
                    prepared_scene: nekoland_ecs::resources::OutputPreparedSceneResources::default(
                    ),
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
                    prepared_scene: nekoland_ecs::resources::OutputPreparedSceneResources::default(
                    ),
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
        render_target_allocation: nekoland_ecs::resources::RenderTargetAllocationPlan::default(),
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
