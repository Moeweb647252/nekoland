use crate::plugin::feedback::WorkspaceVisibilitySnapshot;
use crate::plugin::server::{
    ForeignToplevelSnapshot, ForeignToplevelSnapshotState, ProtocolDmabufSupport,
};
use crate::plugin::{
    ProtocolSeatDispatchSystems, bootstrap, feedback, queue, seat, selection, server, surface,
    xwayland,
};
use bevy_app::{App, SubApp};
use bevy_ecs::prelude::{NonSendMut, Res, ResMut, Resource};
use bevy_ecs::schedule::{InternedScheduleLabel, IntoScheduleConfigs, ScheduleLabel};
use bevy_ecs::system::SystemParam;
use bevy_ecs::world::World;
use nekoland_config::resources::{CompositorConfig, KeyboardLayoutState};
use nekoland_core::app::{
    WaylandApplySystems, WaylandCleanupSystems, WaylandExtractSystems, WaylandFeedbackSystems,
    WaylandNormalizeSystems, WaylandPollSystems, WaylandPresentSystems,
};
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{ExtractSchedule, PresentSchedule, ProtocolSchedule};
use nekoland_ecs::resources::{
    ClipboardSelectionState, CompiledOutputFrames, CompletedScreenshotFrames, CompositorClock,
    CursorImageSnapshot, DragAndDropState, FramePacingState, GlobalPointerPosition,
    KeyboardFocusState,
    OutputPresentationState, OutputSnapshotState, PendingLayerRequests, PendingOutputControls,
    PendingOutputEvents, PendingOutputOverlayControls, PendingOutputServerRequests,
    PendingPlatformInputEvents, PendingPopupEvents, PendingPopupServerRequests,
    PendingProtocolInputEvents, PendingWindowControls, PendingWindowEvents,
    PendingWindowServerRequests, PendingXdgRequests, PlatformSurfaceSnapshotState,
    PresentAuditState, PrimarySelectionState, SeatRegistry, ProtocolServerState, RenderPlan,
    ShellRenderInput, SurfaceContentVersionSnapshot,
    VirtualOutputCaptureState, WaylandCommands, WaylandFeedback, WaylandIngress, XWaylandServerState,
};

/// Entrypoint for the dedicated wayland subapp boundary.
///
/// The subapp owns platform-facing boundary resource production plus the extracted
/// protocol/backend runtime
/// schedules that need non-send state, while a smaller set of ECS reconciliation systems still
/// remains in the main world until bootstrap, poll, and output discovery are fully migrated.
#[derive(Debug, Default, Clone, Copy)]
pub struct WaylandSubAppPlugin;

impl NekolandPlugin for WaylandSubAppPlugin {
    fn build(&self, app: &mut App) {
        bootstrap::bootstrap_protocol_runtime_in_subapp(app);

        app.init_resource::<crate::ProtocolState>()
            .init_resource::<ProtocolDmabufSupport>()
            .init_resource::<WaylandCommands>()
            .init_resource::<CompiledOutputFrames>()
            .init_resource::<ProtocolServerState>()
            .init_resource::<XWaylandServerState>()
            .init_resource::<CursorImageSnapshot>()
            .init_resource::<CompositorClock>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<PendingPlatformInputEvents>()
            .init_resource::<PendingProtocolInputEvents>()
            .init_resource::<FramePacingState>()
            .init_resource::<OutputSnapshotState>()
            .init_resource::<PlatformSurfaceSnapshotState>()
            .init_resource::<SurfaceContentVersionSnapshot>()
            .init_resource::<ForeignToplevelSnapshotState>()
            .init_resource::<RenderPlan>()
            .init_resource::<WorkspaceVisibilitySnapshot>()
            .init_resource::<PendingXdgRequests>()
            .init_resource::<PendingPopupEvents>()
            .init_resource::<PendingWindowEvents>()
            .init_resource::<PendingLayerRequests>()
            .init_resource::<PendingOutputEvents>()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<PendingPopupServerRequests>()
            .init_resource::<SeatRegistry>()
            .init_resource::<ClipboardSelectionState>()
            .init_resource::<DragAndDropState>()
            .init_resource::<PrimarySelectionState>()
            .init_resource::<CompletedScreenshotFrames>()
            .init_resource::<OutputPresentationState>()
            .init_resource::<PresentAuditState>()
            .init_resource::<WaylandIngress>()
            .init_resource::<WaylandFeedback>()
            .add_systems(
                ExtractSchedule,
                bootstrap::advance_compositor_clock_system
                    .in_set(WaylandExtractSystems)
                    .after(WaylandPollSystems),
            )
            .add_systems(
                ExtractSchedule,
                (ingest_backend_wayland_commands_system, ingest_protocol_wayland_commands_system)
                    .chain()
                    .in_set(WaylandApplySystems),
            )
            .add_systems(
                ProtocolSchedule,
                (
                    server::sync_protocol_dmabuf_support_system,
                    server::sync_keyboard_repeat_config_system,
                    server::sync_keyboard_layout_config_system,
                    server::sync_protocol_server_state_system,
                    xwayland::sync_xwayland_server_state_system,
                    server::sync_protocol_cursor_state_system,
                    server::sync_protocol_output_timing_system,
                )
                    .chain()
                    .in_set(WaylandNormalizeSystems),
            )
            .add_systems(
                ProtocolSchedule,
                (
                    xwayland::dispatch_xwayland_runtime_system,
                    xwayland::dispatch_window_server_requests_system,
                    xwayland::dispatch_popup_server_requests_system,
                    selection::process_selection_persistence_system,
                    server::collect_smithay_callbacks_system,
                    surface::sync_protocol_surface_registry_system,
                    surface::sync_platform_surface_snapshot_state_system,
                    queue::flush_protocol_queue_system,
                )
                    .chain()
                    .in_set(WaylandApplySystems),
            )
            .add_systems(
                ProtocolSchedule,
                sync_wayland_ingress_boundary_system.in_set(WaylandFeedbackSystems),
            )
            .add_systems(
                PresentSchedule,
                (
                    seat::dispatch_seat_input_system
                        .in_set(ProtocolSeatDispatchSystems)
                        .in_set(WaylandPresentSystems),
                    server::sync_foreign_toplevel_list_system.in_set(WaylandPresentSystems),
                    feedback::sync_workspace_visibility_system.in_set(WaylandPresentSystems),
                    feedback::dispatch_surface_frame_callbacks_system.in_set(WaylandPresentSystems),
                    sync_wayland_feedback_boundary_system.in_set(WaylandFeedbackSystems),
                )
                    .chain(),
            )
            .add_systems(
                PresentSchedule,
                clear_wayland_commands_boundary_system.in_set(WaylandCleanupSystems),
            );
    }
}

pub fn configure_wayland_subapp(sub_app: &mut SubApp) {
    sub_app.set_extract(extract_wayland_subapp_inputs);
}

pub fn extract_wayland_subapp_inputs(main_world: &mut World, wayland_world: &mut World) {
    clone_resource_into::<WaylandCommands>(main_world, wayland_world);
    clone_resource_into::<CompiledOutputFrames>(main_world, wayland_world);
    clone_default_resource_into::<CompositorClock>(main_world, wayland_world);
    clone_resource_into::<GlobalPointerPosition>(main_world, wayland_world);
    let shell_render_input = main_world.resource::<ShellRenderInput>().clone();
    wayland_world.insert_resource(shell_render_input.surface_presentation.clone());
    clone_resource_into::<KeyboardFocusState>(main_world, wayland_world);
    clone_default_resource_into::<SeatRegistry>(main_world, wayland_world);
    clone_resource_into::<CompositorConfig>(main_world, wayland_world);
    clone_resource_into::<KeyboardLayoutState>(main_world, wayland_world);
    clone_resource_into::<FramePacingState>(main_world, wayland_world);
    extract_surface_content_versions_snapshot(main_world, wayland_world);
    if let Some(compiled_frames) = main_world.get_resource::<CompiledOutputFrames>() {
        wayland_world.insert_resource(compiled_frames.render_plan.clone());
    } else {
        clone_resource_into::<RenderPlan>(main_world, wayland_world);
    }
    extract_foreign_toplevel_snapshot(main_world, wayland_world);
    extract_workspace_visibility_snapshot(main_world, wayland_world);
}

pub fn sync_wayland_subapp_back(
    main_world: &mut World,
    wayland_world: &mut World,
    schedule: Option<InternedScheduleLabel>,
) {
    match schedule {
        Some(schedule) if schedule == ExtractSchedule.intern() => {
            clone_resource_into::<CompositorClock>(wayland_world, main_world);
            clear_main_world_wayland_command_boundary(main_world);
        }
        Some(schedule) if schedule == ProtocolSchedule.intern() => {
            clone_resource_into::<WaylandIngress>(wayland_world, main_world);
        }
        Some(schedule) if schedule == PresentSchedule.intern() => {
            clone_resource_into::<WaylandFeedback>(wayland_world, main_world);
        }
        _ => {
            clone_resource_into::<CompositorClock>(wayland_world, main_world);
            clone_resource_into::<WaylandIngress>(wayland_world, main_world);
            clone_resource_into::<WaylandFeedback>(wayland_world, main_world);
        }
    }
}

fn clear_main_world_wayland_command_boundary(main_world: &mut World) {
    let Some(mut wayland_commands) = main_world.get_resource_mut::<WaylandCommands>() else {
        return;
    };
    wayland_commands.pending_window_server_requests.clear();
    wayland_commands.pending_protocol_input_events.clear();
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

fn extract_foreign_toplevel_snapshot(main_world: &mut World, wayland_world: &mut World) {
    let mut windows = main_world.query_filtered::<nekoland_ecs::views::WindowSnapshotRuntime, (
        bevy_ecs::query::With<nekoland_ecs::components::Window>,
        bevy_ecs::query::Allow<bevy_ecs::entity_disabling::Disabled>,
    )>();
    let windows = windows
        .iter(main_world)
        .filter(|window| window.role.is_managed() && !window.management_hints.helper_surface)
        .map(|window| ForeignToplevelSnapshot {
            surface_id: window.surface_id(),
            title: window.window.title.clone(),
            app_id: window.window.app_id.clone(),
        })
        .collect();
    wayland_world.insert_resource(ForeignToplevelSnapshotState { windows });
}

fn extract_workspace_visibility_snapshot(main_world: &mut World, wayland_world: &mut World) {
    let shell_render_input = main_world.resource::<ShellRenderInput>().clone();
    let mut workspaces =
        main_world.query::<(bevy_ecs::entity::Entity, nekoland_ecs::views::WorkspaceRuntime)>();
    let mut windows = main_world.query_filtered::<(
        bevy_ecs::entity::Entity,
        nekoland_ecs::views::WindowVisibilityRuntime,
        bevy_ecs::prelude::Has<bevy_ecs::entity_disabling::Disabled>,
    ), (
        bevy_ecs::query::With<nekoland_ecs::components::Window>,
        bevy_ecs::query::Allow<bevy_ecs::entity_disabling::Disabled>,
    )>();
    let mut popups = main_world.query_filtered::<(
        bevy_ecs::entity::Entity,
        nekoland_ecs::views::PopupRuntime,
        bevy_ecs::prelude::Has<bevy_ecs::entity_disabling::Disabled>,
    ), (
        bevy_ecs::query::With<nekoland_ecs::components::PopupSurface>,
        bevy_ecs::query::Allow<bevy_ecs::entity_disabling::Disabled>,
    )>();
    let surface_presentation = shell_render_input.surface_presentation;
    let active_workspace = workspaces
        .iter(main_world)
        .find(|(_, workspace)| workspace.is_active())
        .map(|(_, workspace)| workspace.id().0)
        .or_else(|| {
            workspaces
                .iter(main_world)
                .min_by_key(|(_, workspace)| workspace.id())
                .map(|(_, workspace)| workspace.id().0)
        });
    let visible_toplevels = windows
        .iter(main_world)
        .filter(|(_, window, disabled)| {
            !disabled
                && surface_presentation.surfaces.get(&window.surface_id()).is_some_and(|state| {
                    state.visible
                        && state.role == nekoland_ecs::resources::SurfacePresentationRole::Window
                })
        })
        .map(|(_, window, _)| window.surface_id())
        .collect::<std::collections::BTreeSet<_>>();
    let visible_toplevel_entities = windows
        .iter(main_world)
        .filter(|(_, window, disabled)| {
            !disabled
                && surface_presentation.surfaces.get(&window.surface_id()).is_some_and(|state| {
                    state.visible
                        && state.role == nekoland_ecs::resources::SurfacePresentationRole::Window
                })
        })
        .map(|(entity, _, _)| entity)
        .collect::<std::collections::BTreeSet<_>>();
    let visible_popup_entities = popups
        .iter(main_world)
        .filter(|(_, popup, disabled)| {
            !disabled
                && surface_presentation.surfaces.get(&popup.surface_id()).is_some_and(|state| {
                    state.visible
                        && state.role == nekoland_ecs::resources::SurfacePresentationRole::Popup
                })
        })
        .map(|(entity, _, _)| entity)
        .collect::<std::collections::BTreeSet<_>>();
    let visible_parent_entities = visible_toplevel_entities
        .iter()
        .copied()
        .chain(visible_popup_entities.iter().copied())
        .collect::<std::collections::BTreeSet<_>>();
    let visible_popups = popups
        .iter(main_world)
        .filter(|(_, popup, disabled)| {
            !disabled
                && surface_presentation.surfaces.get(&popup.surface_id()).is_some_and(|state| {
                    state.visible
                        && state.role == nekoland_ecs::resources::SurfacePresentationRole::Popup
                })
        })
        .map(|(_, popup, _)| popup.surface_id())
        .collect::<std::collections::BTreeSet<_>>();
    let hidden_parent_popups = popups
        .iter(main_world)
        .filter(|(_, popup, _)| !visible_parent_entities.contains(&popup.child_of.parent()))
        .map(|(_, popup, _)| popup.surface_id())
        .collect::<std::collections::BTreeSet<_>>();

    wayland_world.insert_resource(WorkspaceVisibilitySnapshot {
        active_workspace,
        visible_toplevels,
        visible_popups,
        hidden_parent_popups,
    });
}

fn extract_surface_content_versions_snapshot(main_world: &mut World, wayland_world: &mut World) {
    let mut surfaces = main_world.query::<(
        &nekoland_ecs::components::WlSurfaceHandle,
        &nekoland_ecs::components::SurfaceContentVersion,
    )>();
    let versions = surfaces
        .iter(main_world)
        .map(|(surface, content_version)| (surface.id, content_version.value))
        .collect();
    wayland_world.insert_resource(SurfaceContentVersionSnapshot { versions });
}

#[derive(SystemParam)]
struct WaylandIngressSyncParams<'w> {
    protocol_server: Res<'w, ProtocolServerState>,
    xwayland_server: Res<'w, XWaylandServerState>,
    dmabuf_support: Option<Res<'w, ProtocolDmabufSupport>>,
    pointer: Res<'w, GlobalPointerPosition>,
    render_plan: Res<'w, RenderPlan>,
    surface_presentation: Res<'w, nekoland_ecs::resources::SurfacePresentationSnapshot>,
    server: Option<NonSendMut<'w, server::SmithayProtocolServer>>,
    seat_registry: Res<'w, SeatRegistry>,
    cursor_image: Res<'w, CursorImageSnapshot>,
    platform_input_events: ResMut<'w, PendingPlatformInputEvents>,
    output_snapshots: Res<'w, OutputSnapshotState>,
    surface_snapshots: Res<'w, PlatformSurfaceSnapshotState>,
    pending_window_events: ResMut<'w, PendingWindowEvents>,
    pending_popup_events: ResMut<'w, PendingPopupEvents>,
    pending_xdg_requests: ResMut<'w, PendingXdgRequests>,
    pending_layer_requests: ResMut<'w, PendingLayerRequests>,
    pending_window_controls: ResMut<'w, PendingWindowControls>,
    pending_output_events: ResMut<'w, PendingOutputEvents>,
    wayland_ingress: ResMut<'w, WaylandIngress>,
}

fn sync_wayland_ingress_boundary_system(mut params: WaylandIngressSyncParams<'_>) {
    let output_materialization = params.wayland_ingress.output_materialization.clone();
    let primary_output = params.wayland_ingress.primary_output.clone();
    let pointer_focus_surface = seat::pointer_focus_target(
        params.pointer.x,
        params.pointer.y,
        params.server.as_deref(),
        (params.pointer.x, params.pointer.y).into(),
        &seat::PointerFocusInputs {
            render_plan: Some(&params.render_plan),
            surface_presentation: Some(&params.surface_presentation),
            output_snapshots: Some(&params.output_snapshots),
        },
    )
    .map(|focus| focus.surface_id);
    let mut pending_window_controls_boundary = PendingWindowControls::default();
    pending_window_controls_boundary.replace(params.pending_window_controls.take());
    *params.wayland_ingress = WaylandIngress {
        protocol_server: params.protocol_server.clone(),
        xwayland_server: params.xwayland_server.clone(),
        primary_output,
        pointer_focus_surface,
        seat_registry: params.seat_registry.clone(),
        cursor_image: params.cursor_image.clone(),
        platform_input_events: PendingPlatformInputEvents::from_items(params.platform_input_events.take()),
        output_snapshots: params.output_snapshots.clone(),
        surface_snapshots: params.surface_snapshots.clone(),
        pending_window_events: PendingWindowEvents::from_items(params.pending_window_events.take()),
        pending_popup_events: PendingPopupEvents::from_items(params.pending_popup_events.take()),
        pending_xdg_requests: PendingXdgRequests::from_items(params.pending_xdg_requests.take()),
        pending_layer_requests: PendingLayerRequests::from_items(params.pending_layer_requests.take()),
        pending_window_controls: pending_window_controls_boundary,
        pending_output_events: PendingOutputEvents::from_items(params.pending_output_events.take()),
        output_materialization,
        import_capabilities: nekoland_ecs::resources::PlatformImportCapabilities {
            dmabuf_importable: params
                .dmabuf_support
                .as_deref()
                .is_some_and(|support| support.importable),
        },
    };
}

fn ingest_backend_wayland_commands_system(
    wayland_commands: Res<'_, WaylandCommands>,
    mut pending_output_controls: ResMut<'_, PendingOutputControls>,
    mut pending_output_overlay_controls: ResMut<'_, PendingOutputOverlayControls>,
    mut pending_output_server_requests: ResMut<'_, PendingOutputServerRequests>,
) {
    *pending_output_controls = wayland_commands.pending_output_controls.clone();
    *pending_output_overlay_controls = wayland_commands.pending_output_overlay_controls.clone();
    *pending_output_server_requests = wayland_commands.pending_output_server_requests.clone();
}

fn ingest_protocol_wayland_commands_system(
    wayland_commands: Res<'_, WaylandCommands>,
    mut pending_protocol_inputs: ResMut<'_, PendingProtocolInputEvents>,
    mut pending_window_requests: ResMut<'_, PendingWindowServerRequests>,
    mut pending_popup_requests: ResMut<'_, PendingPopupServerRequests>,
) {
    pending_protocol_inputs.extend(
        wayland_commands.pending_protocol_input_events.as_slice().iter().cloned(),
    );
    *pending_window_requests = wayland_commands.pending_window_server_requests.clone();
    *pending_popup_requests = wayland_commands.pending_popup_server_requests.clone();
}

fn clear_wayland_commands_boundary_system(mut wayland_commands: ResMut<'_, WaylandCommands>) {
    *wayland_commands = WaylandCommands::default();
}

fn sync_wayland_feedback_boundary_system(
    clipboard_selection: Res<'_, ClipboardSelectionState>,
    drag_and_drop: Res<'_, DragAndDropState>,
    primary_selection: Res<'_, PrimarySelectionState>,
    completed_screenshots: Res<'_, CompletedScreenshotFrames>,
    output_presentation: Res<'_, OutputPresentationState>,
    present_audit: Res<'_, PresentAuditState>,
    virtual_output_capture: Option<Res<'_, VirtualOutputCaptureState>>,
    mut wayland_feedback: ResMut<'_, WaylandFeedback>,
) {
    *wayland_feedback = WaylandFeedback {
        platform_backends: wayland_feedback.platform_backends.clone(),
        import_diagnostics: wayland_feedback.import_diagnostics.clone(),
        clipboard_selection: clipboard_selection.clone(),
        drag_and_drop: drag_and_drop.clone(),
        primary_selection: primary_selection.clone(),
        pending_screenshot_requests: wayland_feedback.pending_screenshot_requests.clone(),
        completed_screenshots: completed_screenshots.clone(),
        output_presentation: output_presentation.clone(),
        present_audit: present_audit.clone(),
        virtual_output_capture: virtual_output_capture.as_deref().cloned().unwrap_or_default(),
    };
}

#[cfg(test)]
mod tests {
    use bevy_app::SubApp;
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::schedule::ScheduleLabel;
    use bevy_ecs::world::World;
    use nekoland_core::plugin::NekolandAppPlugin;
    use nekoland_core::schedules::{
        ExtractSchedule, PresentSchedule, ProtocolSchedule, install_core_schedules_sub_app,
    };
    use nekoland_ecs::components::{
        BufferState, PopupSurface, SeatId, SurfaceGeometry, WindowMode, WindowRole,
        WindowViewportVisibility, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, ClipboardSelection, ClipboardSelectionState,
        CompiledOutputFrames, CompletedScreenshotFrames, CompositorClock, CursorImageSnapshot,
        DragAndDropDrop, DragAndDropSession, DragAndDropState, GlobalPointerPosition,
        OutputGeometrySnapshot, OutputPresentationState, OutputPresentationTimeline,
        OutputSnapshotState, PendingLayerRequests, PendingOutputControls, PendingOutputEvents,
        PendingOutputOverlayControls, PendingOutputServerRequests, PendingPopupServerRequests,
        PendingPopupEvents, PendingProtocolInputEvents, PendingScreenshotRequests,
        PendingWindowControls, PendingWindowServerRequests,
        PendingXdgRequests, PlatformSurfaceKind,
        PlatformSurfaceSnapshot, PlatformSurfaceSnapshotState, PresentAuditElement,
        PresentAuditElementKind, PresentAuditState, PrimarySelection, PrimarySelectionState,
        ProtocolServerState, ScreenshotFrame, SelectionOwner, ShellRenderInput, SurfaceExtent,
        SurfacePresentationRole, SurfacePresentationSnapshot,
        SurfacePresentationState, VirtualOutputCaptureState, VirtualOutputElement,
        VirtualOutputElementKind, VirtualOutputFrame, WaylandCommands, WaylandFeedback,
        WaylandIngress, WindowServerAction, WindowServerRequest, XWaylandServerState,
    };
    use crate::plugin::feedback::WorkspaceVisibilitySnapshot;

    use super::{
        WaylandSubAppPlugin, configure_wayland_subapp, extract_workspace_visibility_snapshot,
        sync_wayland_subapp_back,
    };

    #[test]
    fn wayland_subapp_extracts_inputs_from_main_world() {
        let mut main_world = World::default();
        main_world.insert_resource(GlobalPointerPosition { x: 48.0, y: 64.0 });
        main_world.insert_resource(WaylandCommands {
            pending_output_controls: PendingOutputControls::default(),
            pending_protocol_input_events: PendingProtocolInputEvents::from_items(vec![
                BackendInputEvent {
                    device: "test-seat".to_owned(),
                    action: BackendInputAction::PointerButton {
                        button_code: 0x110,
                        pressed: true,
                    },
                },
            ]),
            ..Default::default()
        });
        main_world.insert_resource(CompiledOutputFrames::default());
        main_world.insert_resource(ShellRenderInput {
            pointer: GlobalPointerPosition { x: 1.0, y: 2.0 },
            ..ShellRenderInput::default()
        });
        main_world.init_resource::<CompletedScreenshotFrames>();

        let mut sub_app = SubApp::new();
        install_core_schedules_sub_app(&mut sub_app);
        sub_app.add_plugins(NekolandAppPlugin::new(WaylandSubAppPlugin));
        configure_wayland_subapp(&mut sub_app);
        sub_app.extract(&mut main_world);
        assert_eq!(
            sub_app.world().get_resource::<GlobalPointerPosition>(),
            Some(&GlobalPointerPosition { x: 48.0, y: 64.0 }),
            "wayland extract should mirror the latest main-world pointer resource",
        );
        assert_eq!(
            sub_app
                .world()
                .resource::<WaylandCommands>()
                .pending_protocol_input_events
                .as_slice()
                .len(),
            1,
            "wayland extract should forward one-shot protocol input events into the subapp",
        );
        assert_eq!(
            sub_app.world().get_resource::<SurfacePresentationSnapshot>(),
            Some(&SurfacePresentationSnapshot::default()),
            "wayland extract should mirror the shell boundary presentation snapshot",
        );
        sub_app.world_mut().insert_resource(ProtocolServerState {
            socket_name: Some("wayland-1".to_owned()),
            ..Default::default()
        });
        sub_app.world_mut().insert_resource(XWaylandServerState {
            enabled: true,
            ready: true,
            ..Default::default()
        });
        sub_app.world_mut().resource_mut::<WaylandIngress>().primary_output =
            nekoland_ecs::resources::PrimaryOutputState {
                id: Some(nekoland_ecs::components::OutputId(7)),
            };
        sub_app
            .world_mut()
            .insert_resource(CursorImageSnapshot::Named { icon_name: "default".to_owned() });
        sub_app.world_mut().insert_resource(OutputSnapshotState {
            outputs: vec![OutputGeometrySnapshot {
                output_id: nekoland_ecs::components::OutputId(7),
                name: "DP-1".to_owned(),
                x: 10,
                y: 20,
                width: 1920,
                height: 1080,
                scale: 2,
                refresh_millihz: 60_000,
            }],
        });
        sub_app.world_mut().insert_resource(PlatformSurfaceSnapshotState {
            surfaces: std::collections::BTreeMap::from([(
                91,
                PlatformSurfaceSnapshot {
                    surface_id: 91,
                    kind: PlatformSurfaceKind::Toplevel,
                    attached: true,
                    scale: 1,
                    content_version: 0,
                    dmabuf_format: None,
                    ..Default::default()
                },
            )]),
        });
        sub_app.world_mut().init_resource::<PendingXdgRequests>();
        sub_app.world_mut().init_resource::<PendingLayerRequests>();
        sub_app.world_mut().init_resource::<PendingPopupEvents>();
        sub_app.world_mut().run_schedule(ExtractSchedule);
        sub_app.world_mut().run_schedule(ProtocolSchedule);

        assert!(sub_app.world().get_resource::<WaylandCommands>().is_some());
        assert!(sub_app.world().get_resource::<CompiledOutputFrames>().is_some());
        let protocol_server = sub_app.world().resource::<ProtocolServerState>().clone();
        assert_eq!(sub_app.world().resource::<WaylandIngress>().protocol_server, protocol_server);
        assert!(sub_app.world().resource::<WaylandIngress>().xwayland_server.enabled);
        assert_eq!(
            sub_app.world().resource::<WaylandIngress>().primary_output.id,
            Some(nekoland_ecs::components::OutputId(7))
        );
        assert_eq!(
            sub_app.world().resource::<WaylandIngress>().cursor_image,
            CursorImageSnapshot::Named { icon_name: "default".to_owned() }
        );
        assert!(sub_app.world().resource::<WaylandIngress>().platform_input_events.is_empty());
        assert_eq!(sub_app.world().resource::<WaylandIngress>().output_snapshots.outputs.len(), 1);
        assert_eq!(
            sub_app.world().resource::<WaylandIngress>().surface_snapshots.kind(91),
            PlatformSurfaceKind::Toplevel
        );
        assert!(sub_app.world().resource::<WaylandIngress>().pending_window_controls.is_empty());
    }

    #[test]
    fn wayland_subapp_syncs_feedback_back_to_main_world() {
        let mut main_world = World::default();
        let mut wayland_world = World::default();
        wayland_world.insert_resource(WaylandIngress::default());
        wayland_world.insert_resource(WaylandFeedback::default());

        sync_wayland_subapp_back(&mut main_world, &mut wayland_world, None);

        assert!(main_world.get_resource::<WaylandIngress>().is_some());
        assert!(main_world.get_resource::<WaylandFeedback>().is_some());
    }

    #[test]
    fn wayland_subapp_protocol_sync_back_mirrors_compat_resources_from_ingress() {
        let mut main_world = World::default();
        let mut wayland_world = World::default();
        let mut pending_xdg_requests = PendingXdgRequests::default();
        pending_xdg_requests.push(nekoland_ecs::resources::WindowLifecycleRequest {
            surface_id: 7,
            action: nekoland_ecs::resources::WindowLifecycleAction::ConfigureRequested {
                role: nekoland_ecs::resources::XdgSurfaceRole::Toplevel,
            },
        });
        wayland_world.insert_resource(WaylandIngress {
            protocol_server: ProtocolServerState {
                socket_name: Some("wayland-compat".to_owned()),
                ..Default::default()
            },
            xwayland_server: XWaylandServerState {
                enabled: true,
                ready: true,
                display_number: Some(1),
                ..Default::default()
            },
            cursor_image: CursorImageSnapshot::Named { icon_name: "left_ptr".to_owned() },
            surface_snapshots: PlatformSurfaceSnapshotState {
                surfaces: std::collections::BTreeMap::from([(
                    42,
                    PlatformSurfaceSnapshot {
                        surface_id: 42,
                        kind: PlatformSurfaceKind::Popup,
                        attached: true,
                        scale: 2,
                        content_version: 9,
                        dmabuf_format: None,
                        ..Default::default()
                    },
                )]),
            },
            pending_xdg_requests: pending_xdg_requests.clone(),
            pending_layer_requests: PendingLayerRequests::default(),
            pending_popup_events: PendingPopupEvents::default(),
            ..Default::default()
        });

        sync_wayland_subapp_back(
            &mut main_world,
            &mut wayland_world,
            Some(ProtocolSchedule.intern()),
        );

        let ingress = main_world.resource::<WaylandIngress>();
        assert_eq!(ingress.protocol_server.socket_name.as_deref(), Some("wayland-compat"));
        assert!(ingress.xwayland_server.ready);
        assert_eq!(
            ingress.cursor_image,
            CursorImageSnapshot::Named { icon_name: "left_ptr".to_owned() }
        );
        assert_eq!(ingress.surface_snapshots.kind(42), PlatformSurfaceKind::Popup);
        assert_eq!(ingress.pending_xdg_requests, pending_xdg_requests);
        assert!(main_world.get_resource::<PendingWindowControls>().is_none());
        assert!(main_world.get_resource::<PendingOutputEvents>().is_none());
    }

    #[test]
    fn extract_schedule_sync_back_drains_main_world_one_shot_wayland_commands() {
        let mut main_world = World::default();
        let mut wayland_world = World::default();
        let mut pending_window_server_requests = PendingWindowServerRequests::default();
        pending_window_server_requests.push(WindowServerRequest {
            surface_id: 9,
            action: WindowServerAction::SyncXdgToplevelState {
                size: Some(SurfaceExtent { width: 800, height: 600 }),
                fullscreen: false,
                maximized: false,
                resizing: false,
            },
        });
        main_world.insert_resource(WaylandCommands {
            pending_window_server_requests,
            pending_protocol_input_events: PendingProtocolInputEvents::from_items(vec![
                BackendInputEvent {
                    device: "test-seat".to_owned(),
                    action: BackendInputAction::PointerButton {
                        button_code: 0x110,
                        pressed: true,
                    },
                },
            ]),
            ..WaylandCommands::default()
        });

        sync_wayland_subapp_back(
            &mut main_world,
            &mut wayland_world,
            Some(ExtractSchedule.intern()),
        );

        assert!(
            main_world
                .resource::<WaylandCommands>()
                .pending_window_server_requests
                .is_empty(),
            "extract schedule sync-back should clear one-shot window server commands after handoff",
        );
        assert!(
            main_world
                .resource::<WaylandCommands>()
                .pending_protocol_input_events
                .is_empty(),
            "extract schedule sync-back should clear one-shot protocol input commands after handoff",
        );
    }

    #[test]
    fn wayland_subapp_present_sync_back_mirrors_compat_resources_from_feedback() {
        let mut main_world = World::default();
        let mut wayland_world = World::default();
        let mut completed_screenshots = CompletedScreenshotFrames::default();
        let mut pending_screenshot_requests = PendingScreenshotRequests::default();
        let pending_request_id =
            pending_screenshot_requests.request_output(nekoland_ecs::components::OutputId(5));
        completed_screenshots.push_frame(ScreenshotFrame {
            request_id: pending_request_id,
            output_id: nekoland_ecs::components::OutputId(5),
            frame: 8,
            uptime_millis: 12,
            width: 1920,
            height: 1080,
            scale: 2,
            pixels_rgba: vec![255, 0, 0, 255],
        });
        let mut virtual_output_capture = VirtualOutputCaptureState::default();
        virtual_output_capture.push_frame(VirtualOutputFrame {
            output_name: "Virtual-1".to_owned(),
            frame: 8,
            uptime_millis: 12,
            width: 640,
            height: 480,
            scale: 1,
            background_color: "#000000".to_owned(),
            elements: vec![VirtualOutputElement {
                surface_id: 99,
                kind: VirtualOutputElementKind::Window,
                x: 0,
                y: 0,
                width: 640,
                height: 480,
                z_index: 1,
                opacity: 1.0,
            }],
        });
        wayland_world.insert_resource(WaylandFeedback {
            clipboard_selection: ClipboardSelectionState {
                selection: Some(ClipboardSelection {
                    seat_id: SeatId::PRIMARY,
                    mime_types: vec!["text/plain".to_owned()],
                    owner: SelectionOwner::Client,
                    persisted_mime_types: vec!["text/plain".to_owned()],
                }),
            },
            drag_and_drop: DragAndDropState {
                active_session: Some(DragAndDropSession {
                    seat_id: SeatId::PRIMARY,
                    source_surface_id: Some(99),
                    icon_surface_id: None,
                    mime_types: vec!["text/plain".to_owned()],
                    accepted_mime_type: Some("text/plain".to_owned()),
                    chosen_action: Some("copy".to_owned()),
                }),
                last_drop: Some(DragAndDropDrop {
                    seat_id: SeatId::PRIMARY,
                    source_surface_id: Some(99),
                    target_surface_id: Some(100),
                    validated: true,
                    mime_types: vec!["text/plain".to_owned()],
                }),
            },
            primary_selection: PrimarySelectionState {
                selection: Some(PrimarySelection {
                    seat_id: SeatId::PRIMARY,
                    mime_types: vec!["text/plain".to_owned()],
                    owner: SelectionOwner::Compositor,
                    persisted_mime_types: vec![],
                }),
            },
            output_presentation: OutputPresentationState {
                outputs: vec![OutputPresentationTimeline {
                    output_id: nekoland_ecs::components::OutputId(5),
                    refresh_interval_nanos: 16_666_667,
                    present_time_nanos: 8_000_000,
                    sequence: 2,
                }],
            },
            present_audit: PresentAuditState {
                outputs: std::collections::BTreeMap::from([(
                    nekoland_ecs::components::OutputId(5),
                    nekoland_ecs::resources::OutputPresentAudit {
                        output_name: "Virtual-1".to_owned(),
                        frame: 8,
                        uptime_millis: 12,
                        elements: vec![PresentAuditElement {
                            surface_id: 99,
                            kind: PresentAuditElementKind::Window,
                            x: 0,
                            y: 0,
                            width: 640,
                            height: 480,
                            z_index: 1,
                            opacity: 1.0,
                        }],
                    },
                )]),
            },
            pending_screenshot_requests: pending_screenshot_requests.clone(),
            completed_screenshots: completed_screenshots.clone(),
            virtual_output_capture: virtual_output_capture.clone(),
            ..Default::default()
        });

        sync_wayland_subapp_back(
            &mut main_world,
            &mut wayland_world,
            Some(PresentSchedule.intern()),
        );

        let feedback = main_world.resource::<WaylandFeedback>();
        assert_eq!(feedback.output_presentation.outputs.len(), 1);
        assert_eq!(feedback.present_audit.outputs.len(), 1);
        assert_eq!(feedback.pending_screenshot_requests, pending_screenshot_requests);
        assert!(main_world.get_resource::<PendingScreenshotRequests>().is_none());
        assert_eq!(feedback.completed_screenshots, completed_screenshots);
        assert_eq!(feedback.virtual_output_capture, virtual_output_capture);
        assert!(feedback.clipboard_selection.selection.is_some());
        assert!(feedback.drag_and_drop.active_session.is_some());
        assert!(feedback.primary_selection.selection.is_some());
        assert!(main_world.get_resource::<ClipboardSelectionState>().is_none());
        assert!(main_world.get_resource::<DragAndDropState>().is_none());
        assert!(main_world.get_resource::<PrimarySelectionState>().is_none());
    }

    #[test]
    fn wayland_subapp_fans_out_wayland_commands_to_platform_pending_queues() {
        let mut main_world = World::default();
        main_world.insert_resource(ShellRenderInput::default());
        let pending_output_controls = PendingOutputControls::default();
        let mut pending_output_overlay_controls = PendingOutputOverlayControls::default();
        pending_output_overlay_controls
            .output(nekoland_ecs::components::OutputId(7))
            .clear_overlays();
        let mut pending_output_server_requests = PendingOutputServerRequests::default();
        pending_output_server_requests.push(nekoland_ecs::resources::OutputServerRequest {
            action: nekoland_ecs::resources::OutputServerAction::Disable {
                output: "Virtual-1".to_owned(),
            },
        });
        let mut pending_protocol_input_events = PendingProtocolInputEvents::default();
        pending_protocol_input_events.push(BackendInputEvent {
            device: "seat-0".to_owned(),
            action: BackendInputAction::FocusChanged { focused: true },
        });
        main_world.insert_resource(WaylandCommands {
            pending_output_controls: pending_output_controls.clone(),
            pending_output_overlay_controls: pending_output_overlay_controls.clone(),
            pending_output_server_requests: pending_output_server_requests.clone(),
            pending_protocol_input_events: pending_protocol_input_events.clone(),
            pending_window_server_requests: PendingWindowServerRequests::default(),
            pending_popup_server_requests: PendingPopupServerRequests::default(),
            ..Default::default()
        });

        let mut sub_app = SubApp::new();
        install_core_schedules_sub_app(&mut sub_app);
        sub_app.add_plugins(NekolandAppPlugin::new(WaylandSubAppPlugin));
        configure_wayland_subapp(&mut sub_app);
        sub_app.extract(&mut main_world);
        sub_app.world_mut().run_schedule(ExtractSchedule);

        assert_eq!(
            *sub_app.world().resource::<PendingWindowServerRequests>(),
            main_world.resource::<WaylandCommands>().pending_window_server_requests
        );
        assert_eq!(
            *sub_app.world().resource::<PendingPopupServerRequests>(),
            main_world.resource::<WaylandCommands>().pending_popup_server_requests
        );
        assert_eq!(*sub_app.world().resource::<PendingOutputControls>(), pending_output_controls);
        assert_eq!(
            *sub_app.world().resource::<PendingOutputOverlayControls>(),
            pending_output_overlay_controls
        );
        assert_eq!(
            *sub_app.world().resource::<PendingOutputServerRequests>(),
            pending_output_server_requests
        );
        assert_eq!(
            *sub_app.world().resource::<PendingProtocolInputEvents>(),
            pending_protocol_input_events
        );
    }

    #[test]
    fn wayland_subapp_keeps_backend_protocol_inputs_when_applying_wayland_commands() {
        let mut main_world = World::default();
        main_world.insert_resource(ShellRenderInput::default());

        let backend_event = BackendInputEvent {
            device: "backend-seat".to_owned(),
            action: BackendInputAction::PointerMoved { x: 32.0, y: 48.0 },
        };
        let shell_event = BackendInputEvent {
            device: "shell-seat".to_owned(),
            action: BackendInputAction::PointerButton { button_code: 0x110, pressed: true },
        };

        main_world.insert_resource(WaylandCommands {
            pending_protocol_input_events: PendingProtocolInputEvents::from_items(vec![
                shell_event.clone(),
            ]),
            ..Default::default()
        });

        let mut sub_app = SubApp::new();
        install_core_schedules_sub_app(&mut sub_app);
        sub_app.add_plugins(NekolandAppPlugin::new(WaylandSubAppPlugin));
        configure_wayland_subapp(&mut sub_app);
        sub_app.extract(&mut main_world);
        sub_app
            .world_mut()
            .resource_mut::<PendingProtocolInputEvents>()
            .push(backend_event.clone());

        sub_app.world_mut().run_schedule(ExtractSchedule);

        assert_eq!(
            sub_app.world().resource::<PendingProtocolInputEvents>().as_slice(),
            &[backend_event, shell_event],
        );
    }

    #[test]
    fn workspace_visibility_does_not_dismiss_nested_visible_popups() {
        let mut main_world = World::default();
        let mut wayland_world = World::default();

        let window = main_world
            .spawn((
                WlSurfaceHandle { id: 10 },
                WindowViewportVisibility { visible: true, output: None },
                WindowRole::default(),
                WindowMode::default(),
                XdgWindow::default(),
            ))
            .id();
        let popup = main_world
            .spawn((
                WlSurfaceHandle { id: 11 },
                BufferState { attached: true, scale: 1 },
                PopupSurface::default(),
                ChildOf(window),
            ))
            .id();
        main_world.spawn((
            WlSurfaceHandle { id: 12 },
            BufferState { attached: true, scale: 1 },
            PopupSurface::default(),
            ChildOf(popup),
        ));
        main_world.insert_resource(ShellRenderInput {
            surface_presentation: SurfacePresentationSnapshot {
                surfaces: std::collections::BTreeMap::from([
                    (
                        10,
                        SurfacePresentationState {
                            visible: true,
                            target_output: None,
                            geometry: SurfaceGeometry::default(),
                            input_enabled: true,
                            damage_enabled: true,
                            role: SurfacePresentationRole::Window,
                        },
                    ),
                    (
                        11,
                        SurfacePresentationState {
                            visible: true,
                            target_output: None,
                            geometry: SurfaceGeometry::default(),
                            input_enabled: true,
                            damage_enabled: true,
                            role: SurfacePresentationRole::Popup,
                        },
                    ),
                    (
                        12,
                        SurfacePresentationState {
                            visible: true,
                            target_output: None,
                            geometry: SurfaceGeometry::default(),
                            input_enabled: true,
                            damage_enabled: true,
                            role: SurfacePresentationRole::Popup,
                        },
                    ),
                ]),
            },
            ..Default::default()
        });

        extract_workspace_visibility_snapshot(&mut main_world, &mut wayland_world);

        let snapshot = wayland_world.resource::<WorkspaceVisibilitySnapshot>();
        assert!(snapshot.visible_popups.contains(&11));
        assert!(snapshot.visible_popups.contains(&12));
        assert!(!snapshot.hidden_parent_popups.contains(&12));
    }

    #[test]
    fn wayland_subapp_extract_phase_advances_and_syncs_compositor_clock() {
        let mut main_world = World::default();
        main_world.insert_resource(CompositorClock::default());
        main_world.insert_resource(ShellRenderInput::default());

        let mut sub_app = SubApp::new();
        install_core_schedules_sub_app(&mut sub_app);
        sub_app.add_plugins(NekolandAppPlugin::new(WaylandSubAppPlugin));
        configure_wayland_subapp(&mut sub_app);
        sub_app.extract(&mut main_world);
        sub_app.world_mut().run_schedule(ExtractSchedule);
        sync_wayland_subapp_back(
            &mut main_world,
            sub_app.world_mut(),
            Some(ExtractSchedule.intern()),
        );

        let clock = main_world.resource::<CompositorClock>();
        assert_eq!(clock.frame, 1);
    }

    #[test]
    fn wayland_subapp_extract_keeps_compositor_clock_subapp_owned_after_bootstrap() {
        let mut main_world = World::default();
        main_world.insert_resource(CompositorClock::default());
        main_world.insert_resource(ShellRenderInput::default());

        let mut sub_app = SubApp::new();
        install_core_schedules_sub_app(&mut sub_app);
        sub_app.add_plugins(NekolandAppPlugin::new(WaylandSubAppPlugin));
        configure_wayland_subapp(&mut sub_app);

        sub_app.extract(&mut main_world);
        sub_app.world_mut().run_schedule(ExtractSchedule);
        sync_wayland_subapp_back(
            &mut main_world,
            sub_app.world_mut(),
            Some(ExtractSchedule.intern()),
        );
        assert_eq!(main_world.resource::<CompositorClock>().frame, 1);

        main_world.insert_resource(CompositorClock::default());
        sub_app.extract(&mut main_world);
        sub_app.world_mut().run_schedule(ExtractSchedule);
        sync_wayland_subapp_back(
            &mut main_world,
            sub_app.world_mut(),
            Some(ExtractSchedule.intern()),
        );

        let clock = main_world.resource::<CompositorClock>();
        assert_eq!(clock.frame, 2);
    }

    #[test]
    fn wayland_subapp_builds_feedback_from_present_phase_state() {
        let mut main_world = World::default();
        main_world.insert_resource(GlobalPointerPosition::default());
        main_world.insert_resource(ShellRenderInput::default());

        let mut sub_app = SubApp::new();
        install_core_schedules_sub_app(&mut sub_app);
        sub_app.add_plugins(NekolandAppPlugin::new(WaylandSubAppPlugin));
        configure_wayland_subapp(&mut sub_app);
        sub_app.extract(&mut main_world);
        sub_app.world_mut().init_resource::<CompletedScreenshotFrames>();
        sub_app.world_mut().insert_resource(OutputPresentationState {
            outputs: vec![OutputPresentationTimeline {
                output_id: nekoland_ecs::components::OutputId(7),
                refresh_interval_nanos: 16_666_666,
                present_time_nanos: 100_000_000,
                sequence: 3,
            }],
        });
        sub_app.world_mut().insert_resource(PresentAuditState {
            outputs: std::collections::BTreeMap::from([(
                nekoland_ecs::components::OutputId(7),
                nekoland_ecs::resources::OutputPresentAudit {
                    output_name: "DP-1".to_owned(),
                    frame: 12,
                    uptime_millis: 34,
                    elements: vec![PresentAuditElement {
                        surface_id: 91,
                        kind: PresentAuditElementKind::Window,
                        x: 1,
                        y: 2,
                        width: 3,
                        height: 4,
                        z_index: 5,
                        opacity: 1.0,
                    }],
                },
            )]),
        });
        sub_app.world_mut().run_schedule(ProtocolSchedule);
        sub_app.world_mut().run_schedule(PresentSchedule);

        let feedback = sub_app.world().resource::<WaylandFeedback>();
        assert_eq!(feedback.output_presentation.outputs.len(), 1);
        assert_eq!(feedback.output_presentation.outputs[0].sequence, 3);
        assert_eq!(feedback.present_audit.outputs.len(), 1);
        assert_eq!(
            feedback.present_audit.outputs[&nekoland_ecs::components::OutputId(7)].elements.len(),
            1
        );
    }
}
