use std::collections::BTreeSet;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, With, Without};
use bevy_ecs::query::Allow;
use bevy_ecs::system::SystemParam;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, BufferState, ServerDecoration, SurfaceContentVersion, SurfaceGeometry,
    WindowAnimation, WindowFullscreenTarget, WindowLayout, WindowMode, WindowPolicyState,
    WindowRole, WindowSceneGeometry, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::events::{WindowClosed, WindowCreated};
use nekoland_ecs::resources::{
    CompositorConfig, EntityIndex, FocusedOutputState, PendingPopupServerRequests,
    PendingXdgRequests, PopupServerAction, PopupServerRequest, PrimaryOutputState,
    WindowLifecycleAction, WindowLifecycleRequest, WorkArea, XdgSurfaceRole,
};
use nekoland_ecs::views::{OutputRuntime, PopupRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::{
    focused_or_primary_workspace_runtime_target, window_workspace_runtime_id,
};

use crate::layout::floating::{
    centre_x, centre_y, placement_work_area, should_auto_place_floating_window,
};
use crate::viewport::resolve_output_state_for_workspace;
use crate::window_policy::{
    WindowBackgroundState, refresh_window_policy, sync_window_background_role,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToplevelManager;

type ToplevelPopups<'w, 's> =
    Query<'w, 's, PopupRuntime, (With<XdgPopup>, Without<XdgWindow>, Allow<Disabled>)>;
type ToplevelSurfaces<'w, 's> =
    Query<'w, 's, &'static WlSurfaceHandle, (With<XdgWindow>, Allow<Disabled>)>;
type ToplevelOutputs<'w, 's> = Query<'w, 's, (Entity, OutputRuntime)>;
type ToplevelWindows<'w, 's> =
    Query<'w, 's, (Entity, WindowRuntime), (With<XdgWindow>, Allow<Disabled>)>;

#[derive(SystemParam)]
pub struct ToplevelLifecycleParams<'w, 's> {
    pending_xdg_requests: ResMut<'w, PendingXdgRequests>,
    pending_popup_requests: ResMut<'w, PendingPopupServerRequests>,
    entity_index: Res<'w, EntityIndex>,
    existing_surfaces: ToplevelSurfaces<'w, 's>,
    outputs: ToplevelOutputs<'w, 's>,
    primary_output: Option<Res<'w, PrimaryOutputState>>,
    focused_output: Option<Res<'w, FocusedOutputState>>,
    windows: ToplevelWindows<'w, 's>,
    popups: ToplevelPopups<'w, 's>,
    workspaces: Query<'w, 's, (Entity, WorkspaceRuntime)>,
    window_created: MessageWriter<'w, WindowCreated>,
    window_closed: MessageWriter<'w, WindowClosed>,
}

/// Owns the XDG toplevel lifecycle bridge from protocol requests into ECS window entities.
///
/// Requests that arrive before the corresponding entity exists are deferred and retried on later
/// ticks instead of being dropped.
pub fn toplevel_lifecycle_system(
    mut commands: Commands,
    config: Res<CompositorConfig>,
    work_area: Res<WorkArea>,
    toplevel: ToplevelLifecycleParams<'_, '_>,
) {
    let ToplevelLifecycleParams {
        mut pending_xdg_requests,
        mut pending_popup_requests,
        entity_index,
        existing_surfaces,
        outputs,
        primary_output,
        focused_output,
        mut windows,
        popups,
        workspaces,
        mut window_created,
        mut window_closed,
    } = toplevel;

    let mut known_surfaces =
        existing_surfaces.iter().map(|surface| surface.id).collect::<BTreeSet<_>>();
    let mut deferred = Vec::new();

    for request in pending_xdg_requests.drain() {
        match request.action.clone() {
            WindowLifecycleAction::Committed { role: XdgSurfaceRole::Toplevel, size }
                if known_surfaces.insert(request.surface_id) =>
            {
                let workspace_entity = focused_or_primary_workspace_runtime_target(
                    &outputs,
                    focused_output.as_deref(),
                    primary_output.as_deref(),
                    &entity_index,
                    1,
                );
                let geometry = size
                    .unwrap_or(nekoland_ecs::resources::SurfaceExtent { width: 960, height: 720 });
                let title = format!("Window {}", request.surface_id);
                let policy = config.resolve_window_policy("org.nekoland.demo", &title, false);
                let background =
                    config.resolve_window_background("org.nekoland.demo", &title, false);
                let mut role = WindowRole::Managed;
                let mut scene_geometry = WindowSceneGeometry {
                    x: 0,
                    y: 0,
                    width: geometry.width.max(1),
                    height: geometry.height.max(1),
                };
                let mut layout = policy.layout;
                let mut mode = policy.mode;
                let mut fullscreen_target = WindowFullscreenTarget::default();
                let window_entity = commands
                    .spawn((
                        WindowBundle {
                            surface: WlSurfaceHandle { id: request.surface_id },
                            geometry: SurfaceGeometry {
                                x: 0,
                                y: 0,
                                width: geometry.width.max(1),
                                height: geometry.height.max(1),
                            },
                            scene_geometry: scene_geometry.clone(),
                            viewport_visibility: Default::default(),
                            buffer: BufferState { attached: size.is_some(), scale: 1 },
                            content_version: SurfaceContentVersion { value: 1 },
                            window: XdgWindow {
                                app_id: "org.nekoland.demo".to_owned(),
                                title: title.clone(),
                                last_acked_configure: None,
                            },
                            layout,
                            mode,
                            decoration: ServerDecoration { enabled: true },
                            border_theme: BorderTheme {
                                width: policy.layout.border_width(),
                                color: config.border_color.clone(),
                            },
                            animation: WindowAnimation::default(),
                        },
                        WindowPolicyState { applied: policy, locked: false },
                    ))
                    .id();
                sync_window_background_role(
                    &mut commands,
                    window_entity,
                    background,
                    WindowBackgroundState::new(
                        &mut role,
                        &mut scene_geometry,
                        &mut fullscreen_target,
                        &mut layout,
                        &mut mode,
                    ),
                    None,
                );
                commands.entity(window_entity).insert((
                    scene_geometry.clone(),
                    fullscreen_target,
                    layout,
                    mode,
                    role,
                ));
                if let Some(workspace_entity) = workspace_entity {
                    commands.entity(window_entity).insert(ChildOf(workspace_entity));
                }
                window_created.write(WindowCreated { surface_id: request.surface_id, title });
            }
            WindowLifecycleAction::Committed {
                role: XdgSurfaceRole::Toplevel,
                size: Some(size),
            } => {
                let mut window = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| windows.get_mut(entity).ok());
                if window.is_none() {
                    window = windows
                        .iter_mut()
                        .find(|(_, window)| window.surface_id() == request.surface_id);
                }

                let Some((_, mut window)) = window else {
                    deferred.push(request);
                    continue;
                };

                let Some(buffer) = window.buffer.as_mut() else {
                    tracing::warn!(
                        surface_id = request.surface_id,
                        "skipping xdg commit for window without buffer state"
                    );
                    continue;
                };
                buffer.attached = true;
                window.content_version.bump();
                if !matches!(
                    *window.mode,
                    WindowMode::Maximized | WindowMode::Fullscreen | WindowMode::Hidden
                ) {
                    window.scene_geometry.width = size.width.max(1);
                    window.scene_geometry.height = size.height.max(1);
                } else if let Some(restored) = window.restore.snapshot.as_mut() {
                    restored.geometry.width = size.width.max(1);
                    restored.geometry.height = size.height.max(1);
                    if restored.layout == WindowLayout::Floating
                        && should_auto_place_floating_window(&window.placement, &restored.geometry)
                    {
                        let workspace_id =
                            window_workspace_runtime_id(window.child_of, &workspaces);
                        let placement_area = placement_area_for_workspace(
                            &outputs,
                            workspace_id,
                            primary_output.as_deref(),
                            &work_area,
                        );
                        restored.geometry.x = centre_x(&placement_area, 0, restored.geometry.width);
                        restored.geometry.y =
                            centre_y(&placement_area, 0, restored.geometry.height);
                    }
                }
            }
            WindowLifecycleAction::Committed { role: XdgSurfaceRole::Toplevel, size: None } => {
                if !known_surfaces.contains(&request.surface_id) {
                    deferred.push(request);
                    continue;
                }

                let mut window = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| windows.get_mut(entity).ok());
                if window.is_none() {
                    window = windows
                        .iter_mut()
                        .find(|(_, window)| window.surface_id() == request.surface_id);
                }
                if let Some((_, mut window)) = window {
                    window.content_version.bump();
                }
            }
            WindowLifecycleAction::ConfigureRequested { role: XdgSurfaceRole::Toplevel } => {
                tracing::trace!(surface_id = request.surface_id, "configure requested");
            }
            WindowLifecycleAction::Destroyed { role: XdgSurfaceRole::Toplevel } => {
                let mut window = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| windows.get_mut(entity).ok());
                if window.is_none() {
                    window = windows
                        .iter_mut()
                        .find(|(_, window)| window.surface_id() == request.surface_id);
                }

                let Some((entity, _)) = window else {
                    deferred.push(request);
                    continue;
                };

                enqueue_popup_dismissals(
                    request.surface_id,
                    &entity_index,
                    &popups,
                    &mut pending_popup_requests,
                );
                commands.entity(entity).despawn();
                known_surfaces.remove(&request.surface_id);
                window_closed.write(WindowClosed { surface_id: request.surface_id });
            }
            WindowLifecycleAction::MetadataChanged { title, app_id } => {
                let mut window = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| windows.get_mut(entity).ok());
                if window.is_none() {
                    window = windows
                        .iter_mut()
                        .find(|(_, window)| window.surface_id() == request.surface_id);
                }

                let Some((entity, mut window_runtime)) = window else {
                    deferred.push(request);
                    continue;
                };

                let (window_app_id, window_title) = {
                    let Some(window) = window_runtime.xdg_window.as_mut() else {
                        tracing::warn!(
                            surface_id = request.surface_id,
                            "skipping xdg metadata update for window without xdg metadata"
                        );
                        continue;
                    };
                    if let Some(title) = &title {
                        window.title = title.clone();
                    }
                    if let Some(app_id) = &app_id {
                        window.app_id = app_id.clone();
                    }
                    (window.app_id.clone(), window.title.clone())
                };
                let policy = config.resolve_window_policy(&window_app_id, &window_title, false);
                refresh_window_policy(
                    policy,
                    &mut window_runtime.layout,
                    &mut window_runtime.mode,
                    &mut window_runtime.restore,
                    &mut window_runtime.policy_state,
                );
                let background =
                    config.resolve_window_background(&window_app_id, &window_title, false);
                let current_background =
                    window_runtime.background.as_ref().map(|background| (*background).clone());
                sync_window_background_role(
                    &mut commands,
                    entity,
                    background,
                    WindowBackgroundState::new(
                        &mut window_runtime.role,
                        &mut window_runtime.scene_geometry,
                        &mut window_runtime.fullscreen_target,
                        &mut window_runtime.layout,
                        &mut window_runtime.mode,
                    ),
                    current_background,
                );
            }
            _ => deferred.push(request),
        }
    }

    pending_xdg_requests.replace(deferred);
}

fn placement_area_for_workspace(
    outputs: &Query<(Entity, OutputRuntime)>,
    workspace_id: Option<u32>,
    primary_output: Option<&PrimaryOutputState>,
    work_area: &WorkArea,
) -> WorkArea {
    resolve_output_state_for_workspace(outputs, workspace_id, primary_output).map_or(
        *work_area,
        |(_, output, _, output_work_area)| {
            placement_work_area(
                &WorkArea {
                    x: output_work_area.x,
                    y: output_work_area.y,
                    width: output_work_area.width,
                    height: output_work_area.height,
                },
                Some(output),
            )
        },
    )
}

/// Debug helper kept around for tracing deferred protocol requests while the lifecycle systems are
/// still growing new request kinds.
#[allow(dead_code)]
fn _trace_unhandled_request(request: &WindowLifecycleRequest) {
    match &request.action {
        WindowLifecycleAction::Committed { role, size } => {
            tracing::trace!(surface_id = request.surface_id, ?role, ?size, "deferred xdg request");
        }
        WindowLifecycleAction::ConfigureRequested { role } => {
            tracing::trace!(surface_id = request.surface_id, ?role, "deferred xdg request");
        }
        WindowLifecycleAction::AckConfigure { role, serial } => {
            tracing::trace!(
                surface_id = request.surface_id,
                ?role,
                serial,
                "deferred xdg configure ack"
            );
        }
        WindowLifecycleAction::MetadataChanged { title, app_id } => {
            tracing::trace!(
                surface_id = request.surface_id,
                title = ?title,
                app_id = ?app_id,
                "deferred xdg metadata change"
            );
        }
        WindowLifecycleAction::InteractiveMove { seat_name, serial } => {
            tracing::trace!(
                surface_id = request.surface_id,
                seat_name,
                serial,
                "deferred interactive move request"
            );
        }
        WindowLifecycleAction::InteractiveResize { seat_name, serial, edges } => {
            tracing::trace!(
                surface_id = request.surface_id,
                seat_name,
                serial,
                %edges,
                "deferred interactive resize request"
            );
        }
        WindowLifecycleAction::Maximize => {
            tracing::trace!(surface_id = request.surface_id, "deferred maximize request");
        }
        WindowLifecycleAction::UnMaximize => {
            tracing::trace!(surface_id = request.surface_id, "deferred unmaximize request");
        }
        WindowLifecycleAction::Fullscreen { output_name } => {
            tracing::trace!(
                surface_id = request.surface_id,
                output_name = ?output_name,
                "deferred fullscreen request"
            );
        }
        WindowLifecycleAction::UnFullscreen => {
            tracing::trace!(surface_id = request.surface_id, "deferred unfullscreen request");
        }
        WindowLifecycleAction::Minimize => {
            tracing::trace!(surface_id = request.surface_id, "deferred minimize request");
        }
        WindowLifecycleAction::PopupCreated { parent_surface_id, placement } => {
            tracing::trace!(
                surface_id = request.surface_id,
                parent_surface_id = ?parent_surface_id,
                placement = ?placement,
                "deferred popup create request"
            );
        }
        WindowLifecycleAction::PopupRepositioned { placement } => {
            tracing::trace!(
                surface_id = request.surface_id,
                placement = ?placement,
                "deferred popup reposition request"
            );
        }
        WindowLifecycleAction::PopupGrab { seat_name, serial } => {
            tracing::trace!(
                surface_id = request.surface_id,
                seat_name,
                serial,
                "deferred popup grab request"
            );
        }
        WindowLifecycleAction::Destroyed { role } => {
            tracing::trace!(surface_id = request.surface_id, ?role, "deferred destroy request");
        }
    }
}

/// Queues popup dismissals before removing a parent toplevel so popup teardown follows the same
/// server-request path as explicit popup dismiss actions.
fn enqueue_popup_dismissals(
    parent_surface_id: u64,
    entity_index: &EntityIndex,
    popups: &ToplevelPopups<'_, '_>,
    pending_popup_requests: &mut PendingPopupServerRequests,
) {
    let Some(parent_entity) = entity_index.entity_for_surface(parent_surface_id) else {
        return;
    };
    let mut dismissed = BTreeSet::new();
    for popup in popups.iter() {
        if popup.child_of.parent() != parent_entity || !dismissed.insert(popup.surface_id()) {
            continue;
        }

        pending_popup_requests.push(PopupServerRequest {
            surface_id: popup.surface_id(),
            action: PopupServerAction::Dismiss,
        });
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::Entity;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        BufferState, OutputCurrentWorkspace, OutputDevice, OutputKind, OutputProperties,
        OutputWorkArea, SurfaceGeometry, WindowAnimation, WindowLayout, WindowMode,
        WindowRestoreSnapshot, WindowRestoreState, WindowSceneGeometry, WlSurfaceHandle, Workspace,
        WorkspaceId, XdgPopup, XdgWindow,
    };
    use nekoland_ecs::events::{WindowClosed, WindowCreated};
    use nekoland_ecs::resources::{
        CompositorConfig, ConfiguredWindowRule, EntityIndex, PendingPopupServerRequests,
        PendingXdgRequests, PopupServerAction, PrimaryOutputState, WindowLifecycleAction,
        WindowLifecycleRequest, WorkArea, XdgSurfaceRole, rebuild_entity_index_system,
    };

    use super::toplevel_lifecycle_system;

    #[test]
    fn toplevel_create_inserts_child_of_active_workspace() {
        let mut app = NekolandApp::new("toplevel-lifecycle-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(WorkArea::default())
            .insert_resource(PendingXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        let workspace_entity = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 21,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(nekoland_ecs::resources::SurfaceExtent { width: 800, height: 600 }),
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let mut windows = world.query::<(Entity, &WlSurfaceHandle)>();
        let window_entity = windows
            .iter(world)
            .find(|(_, surface)| surface.id == 21)
            .map(|(entity, _)| entity)
            .unwrap_or_else(|| panic!("created toplevel window should exist"));

        let Some(child_of) = world.get::<ChildOf>(window_entity) else {
            panic!("window should have ChildOf");
        };
        assert_eq!(
            child_of.parent(),
            workspace_entity,
            "new toplevel should attach to the active workspace entity",
        );
    }

    #[test]
    fn toplevel_destroy_queues_popup_dismiss_before_window_despawn() {
        let mut app = NekolandApp::new("toplevel-lifecycle-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(WorkArea::default())
            .insert_resource(PendingXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        let parent = app
            .inner_mut()
            .world_mut()
            .spawn((WindowBundle {
                surface: WlSurfaceHandle { id: 11 },
                geometry: SurfaceGeometry { x: 0, y: 0, width: 800, height: 600 },
                scene_geometry: WindowSceneGeometry { x: 0, y: 0, width: 800, height: 600 },
                viewport_visibility: Default::default(),
                buffer: BufferState { attached: true, scale: 1 },
                content_version: Default::default(),
                window: XdgWindow::default(),
                layout: WindowLayout::Tiled,
                mode: WindowMode::Normal,
                decoration: Default::default(),
                border_theme: Default::default(),
                animation: WindowAnimation::default(),
            },))
            .id();
        app.inner_mut().world_mut().spawn((
            WlSurfaceHandle { id: 12 },
            XdgPopup {
                configure_serial: None,
                grab_serial: None,
                reposition_token: None,
                placement_x: 0,
                placement_y: 0,
                placement_width: 1,
                placement_height: 1,
            },
            ChildOf(parent),
        ));

        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 11,
                action: WindowLifecycleAction::Destroyed { role: XdgSurfaceRole::Toplevel },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let pending = app.inner().world().resource::<PendingPopupServerRequests>().as_slice();
        assert_eq!(pending.len(), 1, "destroying a toplevel should queue one popup dismiss");
        assert_eq!(pending[0].surface_id, 12);
        assert!(matches!(pending[0].action, PopupServerAction::Dismiss));
    }

    #[test]
    fn committed_size_repositions_restore_geometry_using_workspace_output_area() {
        let mut app = NekolandApp::new("toplevel-restore-placement-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 640, height: 480 })
            .insert_resource(PrimaryOutputState { name: Some("Virtual-1".to_owned()) })
            .insert_resource(PendingXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        let workspace_2 = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(2), name: "2".to_owned(), active: false })
            .id();
        app.inner_mut().world_mut().spawn((
            OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "one".to_owned(),
                },
                properties: OutputProperties {
                    width: 640,
                    height: 480,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                work_area: OutputWorkArea { x: 0, y: 0, width: 640, height: 480 },
                ..Default::default()
            },
            OutputCurrentWorkspace { workspace: WorkspaceId(1) },
        ));
        app.inner_mut().world_mut().spawn((
            OutputBundle {
                output: OutputDevice {
                    name: "HDMI-A-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "two".to_owned(),
                },
                properties: OutputProperties {
                    width: 1600,
                    height: 900,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                work_area: OutputWorkArea { x: 100, y: 200, width: 800, height: 500 },
                ..Default::default()
            },
            OutputCurrentWorkspace { workspace: WorkspaceId(2) },
        ));
        let window_entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 51 },
                    geometry: SurfaceGeometry { x: 0, y: 0, width: 300, height: 200 },
                    scene_geometry: WindowSceneGeometry { x: 0, y: 0, width: 300, height: 200 },
                    viewport_visibility: Default::default(),
                    buffer: BufferState { attached: false, scale: 1 },
                    content_version: Default::default(),
                    window: XdgWindow::default(),
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Fullscreen,
                    decoration: Default::default(),
                    border_theme: Default::default(),
                    animation: WindowAnimation::default(),
                },
                WindowRestoreSnapshot {
                    snapshot: Some(WindowRestoreState {
                        geometry: WindowSceneGeometry { x: 0, y: 0, width: 300, height: 200 },
                        layout: WindowLayout::Floating,
                        mode: WindowMode::Normal,
                        fullscreen_output: None,
                    }),
                },
                ChildOf(workspace_2),
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 51,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(nekoland_ecs::resources::SurfaceExtent { width: 200, height: 100 }),
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(restore) = app
            .inner()
            .world()
            .get::<WindowRestoreSnapshot>(window_entity)
            .and_then(|restore| restore.snapshot.as_ref())
        else {
            panic!("restore snapshot should remain present");
        };

        assert_eq!(
            restore.geometry,
            WindowSceneGeometry { x: 400, y: 400, width: 200, height: 100 }
        );
    }

    #[test]
    fn metadata_change_reapplies_matching_window_rule() {
        let mut app = NekolandApp::new("toplevel-policy-test");
        let mut config = CompositorConfig::default();
        config.window_rules.push(ConfiguredWindowRule {
            app_id: Some("org.nekoland.rules".to_owned()),
            title: None,
            layout: Some(WindowLayout::Tiled),
            mode: None,
            background: None,
        });
        app.insert_resource(config)
            .insert_resource(WorkArea::default())
            .insert_resource(PendingXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 21,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(nekoland_ecs::resources::SurfaceExtent { width: 800, height: 600 }),
                },
            },
        );
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 21,
                action: WindowLifecycleAction::MetadataChanged {
                    title: Some("Rules".to_owned()),
                    app_id: Some("org.nekoland.rules".to_owned()),
                },
            },
        );
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let mut windows = world.query::<(&WlSurfaceHandle, &WindowLayout, &WindowMode)>();
        let window_state = windows.iter(world).find(|(surface, _, _)| surface.id == 21);
        let Some((_, layout, mode)) = window_state else {
            panic!("toplevel window should still exist");
        };
        assert_eq!(*layout, WindowLayout::Tiled);
        assert_eq!(*mode, WindowMode::Normal);
    }
}
