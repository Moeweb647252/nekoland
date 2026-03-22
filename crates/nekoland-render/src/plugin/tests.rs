use bevy_ecs::system::{IntoSystem, System};
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{OutputId, SurfaceContentVersion, WlSurfaceHandle, XdgWindow};
use nekoland_ecs::resources::{
    CompiledOutputFrames, CompositorClock, CursorImageSnapshot, GlobalPointerPosition,
    OutputDamageRegions, OutputExecutionPlan, OutputGeometrySnapshot, OutputOverlayState,
    OutputRenderPlan, OutputSnapshotState, PlatformSurfaceSnapshotState, PreparedGpuResources,
    PreparedSceneResources, RenderFinalOutputPlan, RenderItemId, RenderItemIdentity,
    RenderItemInstance, RenderMaterialFrameState, RenderPassGraph, RenderPassId, RenderPassNode,
    RenderPlan, RenderPlanItem, RenderProcessPlan, RenderReadbackPlan, RenderRect, RenderSceneRole,
    RenderSourceId, RenderTargetAllocationPlan, RenderTargetId, RenderTargetKind, ShellRenderInput,
    SurfaceBufferAttachmentSnapshot, SurfaceContentVersionSnapshot, SurfacePresentationRole,
    SurfacePresentationSnapshot, SurfacePresentationState, SurfaceRenderItem,
    SurfaceTextureBridgePlan, WaylandIngress,
};

use crate::animation::{
    AnimationBindingKey, AnimationEasing, AnimationProperty, AnimationTimelineStore,
    AnimationTrack, AnimationValue, advance_animation_timelines_system,
};
use crate::compositor_render::RenderViewSnapshot;
use crate::scene_process::{AppearanceSnapshot, ProjectionSnapshot};
use crate::scene_source::{RenderInstanceKey, RenderSourceKey};

use super::extract::extract_render_subapp_inputs;
use super::sync_back::sync_compiled_output_frames_system;

#[test]
fn render_root_facade_exports_remain_usable() {
    let mut sub_app = bevy_app::SubApp::new();
    crate::configure_render_subapp(&mut sub_app);

    let mut main_world = bevy_ecs::world::World::default();
    let mut render_world = bevy_ecs::world::World::default();
    crate::sync_render_subapp_back(&mut main_world, &mut render_world, None);

    let _plugin = crate::RenderPlugin;
    let _subapp_plugin = crate::RenderSubAppPlugin;
}

#[test]
fn render_subapp_extract_syncs_shell_owned_inputs_from_shell_render_mailbox() {
    let mut main_world = bevy_ecs::world::World::default();
    let mut pending_screenshot_requests =
        nekoland_ecs::resources::PendingScreenshotRequests::default();
    let request_id = pending_screenshot_requests.request_output(OutputId(3));
    main_world.insert_resource(ShellRenderInput {
        pointer: GlobalPointerPosition { x: 33.0, y: 44.0 },
        cursor_image: CursorImageSnapshot::Named { icon_name: "default".to_owned() },
        surface_presentation: SurfacePresentationSnapshot {
            surfaces: std::collections::BTreeMap::from([(
                77,
                SurfacePresentationState {
                    visible: true,
                    target_output: Some(OutputId(3)),
                    geometry: nekoland_ecs::components::SurfaceGeometry {
                        x: 1,
                        y: 2,
                        width: 100,
                        height: 200,
                    },
                    input_enabled: true,
                    damage_enabled: true,
                    role: SurfacePresentationRole::Window,
                },
            )]),
        },
        output_overlays: OutputOverlayState::default(),
        pending_screenshot_requests,
    });

    let mut render_world = bevy_ecs::world::World::default();
    extract_render_subapp_inputs(&mut main_world, &mut render_world);

    assert_eq!(render_world.resource::<ShellRenderInput>().pointer.x, 33.0);
    assert_eq!(render_world.resource::<ShellRenderInput>().pointer.y, 44.0);
    assert_eq!(
        render_world.resource::<ShellRenderInput>().cursor_image,
        CursorImageSnapshot::Named { icon_name: "default".to_owned() }
    );
    assert!(
        render_world.resource::<ShellRenderInput>().surface_presentation.surfaces.contains_key(&77)
    );
    let requests = &render_world.resource::<ShellRenderInput>().pending_screenshot_requests;
    assert_eq!(requests.requests.len(), 1);
    assert_eq!(requests.requests[0].id, request_id);
    assert_eq!(requests.requests[0].output_id, OutputId(3));
}

#[test]
fn render_subapp_extract_builds_view_and_surface_snapshots_from_mailboxes() {
    let mut main_world = bevy_ecs::world::World::default();
    main_world.insert_resource(nekoland_ecs::resources::WindowStackingState {
        workspaces: std::collections::BTreeMap::from([(
            nekoland_ecs::resources::UNASSIGNED_WORKSPACE_STACK_ID,
            vec![42],
        )]),
    });
    main_world.insert_resource(WaylandIngress {
        output_snapshots: OutputSnapshotState {
            outputs: vec![OutputGeometrySnapshot {
                output_id: OutputId(1),
                name: "DP-1".to_owned(),
                x: 100,
                y: 200,
                width: 2560,
                height: 1440,
                scale: 2,
                refresh_millihz: 60_000,
            }],
        },
        surface_snapshots: PlatformSurfaceSnapshotState {
            surfaces: std::collections::BTreeMap::from([(
                42,
                nekoland_ecs::resources::PlatformSurfaceSnapshot {
                    surface_id: 42,
                    kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                    buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                    dmabuf_format: None,
                    import_strategy:
                        nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                    attached: true,
                    scale: 2,
                    content_version: 7,
                },
            )]),
        },
        ..Default::default()
    });
    main_world.insert_resource(ShellRenderInput {
        surface_presentation: SurfacePresentationSnapshot {
            surfaces: std::collections::BTreeMap::from([(
                42,
                nekoland_ecs::resources::SurfacePresentationState {
                    visible: true,
                    target_output: None,
                    geometry: nekoland_ecs::components::SurfaceGeometry {
                        x: 0,
                        y: 0,
                        width: 100,
                        height: 100,
                    },
                    input_enabled: true,
                    damage_enabled: true,
                    role: nekoland_ecs::resources::SurfacePresentationRole::Window,
                },
            )]),
        },
        pending_screenshot_requests: nekoland_ecs::resources::PendingScreenshotRequests::default(),
        ..Default::default()
    });
    main_world.spawn(WindowBundle {
        surface: WlSurfaceHandle { id: 42 },
        content_version: SurfaceContentVersion { value: 7 },
        ..Default::default()
    });

    let mut render_world = bevy_ecs::world::World::default();
    extract_render_subapp_inputs(&mut main_world, &mut render_world);

    let views = &render_world.resource::<RenderViewSnapshot>().views;
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].x, 100);
    assert_eq!(views[0].y, 200);
    assert_eq!(views[0].width, 2560);
    assert_eq!(views[0].height, 1440);
    assert_eq!(views[0].scale, 2);

    let versions = &render_world.resource::<SurfaceContentVersionSnapshot>().versions;
    assert_eq!(versions.get(&42), Some(&7));

    let attachments = &render_world.resource::<SurfaceBufferAttachmentSnapshot>().surfaces;
    let attachment = attachments.get(&42).expect("surface attachment snapshot");
    assert!(attachment.attached);
    assert_eq!(attachment.scale, 2);

    let ordered =
        &render_world.resource::<crate::compositor_render::DesktopSurfaceOrderSnapshot>().outputs;
    assert_eq!(ordered.len(), 1);
    assert_eq!(ordered.values().next().expect("ordered surfaces"), &vec![42]);
}

#[test]
fn render_subapp_extract_builds_scene_process_snapshots_from_mailboxes() {
    let mut main_world = bevy_ecs::world::World::default();
    main_world.insert_resource(CompositorClock { frame: 1, uptime_millis: 50 });
    main_world.insert_resource(AnimationTimelineStore::default());
    main_world.insert_resource(WaylandIngress {
        output_snapshots: OutputSnapshotState {
            outputs: vec![OutputGeometrySnapshot {
                output_id: OutputId(3),
                name: "Virtual-1".to_owned(),
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
                scale: 1,
                refresh_millihz: 60_000,
            }],
        },
        ..Default::default()
    });
    main_world.insert_resource(ShellRenderInput {
        surface_presentation: SurfacePresentationSnapshot {
            surfaces: std::collections::BTreeMap::from([(
                13,
                SurfacePresentationState {
                    visible: true,
                    target_output: Some(OutputId(3)),
                    geometry: nekoland_ecs::components::SurfaceGeometry {
                        x: 0,
                        y: 0,
                        width: 50,
                        height: 50,
                    },
                    input_enabled: true,
                    damage_enabled: true,
                    role: SurfacePresentationRole::Window,
                },
            )]),
        },
        pending_screenshot_requests: nekoland_ecs::resources::PendingScreenshotRequests::default(),
        ..Default::default()
    });
    main_world.spawn(WindowBundle {
        surface: WlSurfaceHandle { id: 13 },
        window: XdgWindow::default(),
        ..Default::default()
    });
    main_world.resource_mut::<AnimationTimelineStore>().upsert_track(
        AnimationBindingKey::Source(RenderSourceKey::window(13)),
        AnimationTrack {
            property: AnimationProperty::Opacity,
            from: AnimationValue::Float(0.0),
            to: AnimationValue::Float(1.0),
            start_uptime_millis: 0,
            duration_millis: 100,
            easing: AnimationEasing::Linear,
        },
    );
    main_world.resource_mut::<AnimationTimelineStore>().upsert_track(
        AnimationBindingKey::Instance(RenderInstanceKey::new(
            RenderSourceKey::window(13),
            OutputId(3),
            0,
        )),
        AnimationTrack {
            property: AnimationProperty::Rect,
            from: AnimationValue::Rect(RenderRect { x: 0, y: 0, width: 50, height: 50 }),
            to: AnimationValue::Rect(RenderRect { x: 10, y: 20, width: 60, height: 70 }),
            start_uptime_millis: 0,
            duration_millis: 100,
            easing: AnimationEasing::Linear,
        },
    );

    let mut advance = bevy_ecs::system::IntoSystem::into_system(advance_animation_timelines_system);
    advance.initialize(&mut main_world);
    let _ = advance.run((), &mut main_world);

    let mut render_world = bevy_ecs::world::World::default();
    extract_render_subapp_inputs(&mut main_world, &mut render_world);

    let appearance = render_world.resource::<AppearanceSnapshot>();
    let projection = render_world.resource::<ProjectionSnapshot>();
    assert_eq!(
        appearance.sources.get(&RenderSourceKey::window(13)).map(|state| state.opacity),
        Some(0.5)
    );
    assert_eq!(
        projection
            .instances
            .get(&RenderInstanceKey::new(RenderSourceKey::window(13), OutputId(3), 0))
            .and_then(|state| state.rect_override),
        Some(RenderRect { x: 5, y: 10, width: 55, height: 60 })
    );
}

#[test]
fn compiled_output_frames_mirror_render_outputs() {
    let mut world = bevy_ecs::world::World::default();
    world.insert_resource(OutputDamageRegions::default());
    world.insert_resource(PreparedSceneResources::default());
    world.insert_resource(RenderMaterialFrameState::default());
    world.insert_resource(RenderPassGraph::default());
    world.insert_resource(RenderPlan::default());
    world.insert_resource(RenderProcessPlan::default());
    world.insert_resource(RenderFinalOutputPlan::default());
    world.insert_resource(RenderReadbackPlan::default());
    world.insert_resource(RenderTargetAllocationPlan::default());
    world.insert_resource(SurfaceTextureBridgePlan::default());
    world.insert_resource(PreparedGpuResources::default());
    world.init_resource::<CompiledOutputFrames>();

    let mut system = IntoSystem::into_system(sync_compiled_output_frames_system);
    system.initialize(&mut world);
    let _ = system.run((), &mut world);

    let compiled = world.resource::<CompiledOutputFrames>();
    assert!(compiled.outputs.is_empty());
    assert_eq!(compiled.output_damage_regions, OutputDamageRegions::default());
    assert_eq!(compiled.prepared_scene, PreparedSceneResources::default());
    assert_eq!(compiled.materials, RenderMaterialFrameState::default());
    assert_eq!(compiled.render_graph, RenderPassGraph::default());
    assert_eq!(compiled.render_plan, RenderPlan::default());
    assert_eq!(compiled.process_plan, RenderProcessPlan::default());
    assert_eq!(compiled.final_output_plan, RenderFinalOutputPlan::default());
    assert_eq!(compiled.render_target_allocation, RenderTargetAllocationPlan::default());
    assert_eq!(compiled.surface_texture_bridge, SurfaceTextureBridgePlan::default());
}

#[test]
fn compiled_output_frames_include_per_output_frames() {
    let mut world = bevy_ecs::world::World::default();
    world.insert_resource(OutputDamageRegions {
        regions: std::collections::BTreeMap::from([(
            nekoland_ecs::components::OutputId(1),
            vec![nekoland_ecs::resources::DamageRect { x: 0, y: 0, width: 10, height: 10 }],
        )]),
    });
    world.insert_resource(PreparedSceneResources::default());
    world.insert_resource(RenderMaterialFrameState::default());
    world.insert_resource(RenderPassGraph {
        outputs: std::collections::BTreeMap::from([(
            nekoland_ecs::components::OutputId(1),
            OutputExecutionPlan {
                targets: std::collections::BTreeMap::from([(
                    RenderTargetId(1),
                    RenderTargetKind::OutputSwapchain(nekoland_ecs::components::OutputId(1)),
                )]),
                passes: std::collections::BTreeMap::from([(
                    RenderPassId(1),
                    RenderPassNode::scene(
                        RenderSceneRole::Desktop,
                        RenderTargetId(1),
                        Vec::new(),
                        vec![RenderItemId(1)],
                    ),
                )]),
                ordered_passes: vec![RenderPassId(1)],
                terminal_passes: vec![RenderPassId(1)],
            },
        )]),
    });
    world.insert_resource(RenderPlan {
        outputs: std::collections::BTreeMap::from([(
            nekoland_ecs::components::OutputId(1),
            OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                identity: RenderItemIdentity::new(RenderSourceId(1), RenderItemId(1)),
                surface_id: 11,
                instance: RenderItemInstance {
                    rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                    opacity: 1.0,
                    clip_rect: None,
                    z_index: 0,
                    scene_role: RenderSceneRole::Desktop,
                },
            })]),
        )]),
    });
    world.insert_resource(RenderProcessPlan::default());
    world.insert_resource(RenderFinalOutputPlan::default());
    world.insert_resource(RenderReadbackPlan::default());
    world.insert_resource(RenderTargetAllocationPlan::default());
    world.insert_resource(SurfaceTextureBridgePlan::default());
    world.insert_resource(PreparedGpuResources::default());
    world.init_resource::<CompiledOutputFrames>();

    let mut system = IntoSystem::into_system(sync_compiled_output_frames_system);
    system.initialize(&mut world);
    let _ = system.run((), &mut world);

    let compiled = world.resource::<CompiledOutputFrames>();
    let output = compiled
        .outputs
        .get(&nekoland_ecs::components::OutputId(1))
        .expect("compiled output frame");
    assert_eq!(output.render_plan.ordered_item_ids(), &[RenderItemId(1)]);
    assert!(output.prepared_scene.items.is_empty());
    assert_eq!(output.execution_plan.ordered_passes, vec![RenderPassId(1)]);
    assert_eq!(output.damage_regions.len(), 1);
}

#[test]
fn stable_ids_flow_from_platform_mailboxes_into_compiled_output_frames() {
    let output_id = OutputId(3);
    let surface_id = 13_u64;

    let mut main_world = bevy_ecs::world::World::default();
    main_world.insert_resource(nekoland_ecs::resources::WindowStackingState {
        workspaces: std::collections::BTreeMap::from([(
            nekoland_ecs::resources::UNASSIGNED_WORKSPACE_STACK_ID,
            vec![surface_id],
        )]),
    });
    main_world.insert_resource(WaylandIngress {
        output_snapshots: OutputSnapshotState {
            outputs: vec![OutputGeometrySnapshot {
                output_id,
                name: "Virtual-1".to_owned(),
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
                scale: 1,
                refresh_millihz: 60_000,
            }],
        },
        surface_snapshots: PlatformSurfaceSnapshotState {
            surfaces: std::collections::BTreeMap::from([(
                surface_id,
                nekoland_ecs::resources::PlatformSurfaceSnapshot {
                    surface_id,
                    kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                    buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                    dmabuf_format: None,
                    import_strategy:
                        nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                    attached: true,
                    scale: 1,
                    content_version: 4,
                },
            )]),
        },
        ..Default::default()
    });
    main_world.insert_resource(ShellRenderInput {
        surface_presentation: SurfacePresentationSnapshot {
            surfaces: std::collections::BTreeMap::from([(
                surface_id,
                SurfacePresentationState {
                    visible: true,
                    target_output: Some(output_id),
                    geometry: nekoland_ecs::components::SurfaceGeometry {
                        x: 10,
                        y: 20,
                        width: 100,
                        height: 80,
                    },
                    input_enabled: true,
                    damage_enabled: true,
                    role: SurfacePresentationRole::Window,
                },
            )]),
        },
        ..Default::default()
    });
    main_world.spawn(WindowBundle {
        surface: WlSurfaceHandle { id: surface_id },
        content_version: SurfaceContentVersion { value: 4 },
        ..Default::default()
    });

    let mut render_world = bevy_ecs::world::World::default();
    extract_render_subapp_inputs(&mut main_world, &mut render_world);

    assert_eq!(
        render_world.resource::<RenderViewSnapshot>().view(output_id).map(|view| view.output_id),
        Some(output_id)
    );
    assert_eq!(
        render_world
            .resource::<crate::compositor_render::DesktopSurfaceOrderSnapshot>()
            .outputs
            .get(&output_id),
        Some(&vec![surface_id])
    );

    render_world.insert_resource(OutputDamageRegions::default());
    render_world.insert_resource(PreparedSceneResources::default());
    render_world.insert_resource(RenderMaterialFrameState::default());
    render_world.insert_resource(RenderPassGraph {
        outputs: std::collections::BTreeMap::from([(
            output_id,
            OutputExecutionPlan {
                targets: std::collections::BTreeMap::from([(
                    RenderTargetId(1),
                    RenderTargetKind::OutputSwapchain(output_id),
                )]),
                passes: std::collections::BTreeMap::from([(
                    RenderPassId(1),
                    RenderPassNode::scene(
                        RenderSceneRole::Desktop,
                        RenderTargetId(1),
                        Vec::new(),
                        vec![RenderItemId(1)],
                    ),
                )]),
                ordered_passes: vec![RenderPassId(1)],
                terminal_passes: vec![RenderPassId(1)],
            },
        )]),
    });
    render_world.insert_resource(RenderPlan {
        outputs: std::collections::BTreeMap::from([(
            output_id,
            OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                identity: RenderItemIdentity::new(RenderSourceId(surface_id), RenderItemId(1)),
                surface_id,
                instance: RenderItemInstance {
                    rect: RenderRect { x: 10, y: 20, width: 100, height: 80 },
                    opacity: 1.0,
                    clip_rect: None,
                    z_index: 0,
                    scene_role: RenderSceneRole::Desktop,
                },
            })]),
        )]),
    });
    render_world.insert_resource(RenderProcessPlan::default());
    render_world.insert_resource(RenderFinalOutputPlan::default());
    render_world.insert_resource(RenderReadbackPlan::default());
    render_world.insert_resource(RenderTargetAllocationPlan::default());
    render_world.insert_resource(SurfaceTextureBridgePlan::default());
    render_world.insert_resource(PreparedGpuResources::default());
    render_world.init_resource::<CompiledOutputFrames>();

    let mut sync = bevy_ecs::system::IntoSystem::into_system(sync_compiled_output_frames_system);
    sync.initialize(&mut render_world);
    let _ = sync.run((), &mut render_world);

    let compiled = render_world.resource::<CompiledOutputFrames>();
    let compiled_output = compiled.output(output_id).expect("compiled output should exist");
    let compiled_surface = compiled_output.render_plan.iter_ordered().find_map(|item| match item {
        RenderPlanItem::Surface(item) => Some(item.surface_id),
        _ => None,
    });
    assert_eq!(compiled_surface, Some(surface_id));
}
