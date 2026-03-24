use std::collections::BTreeSet;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, With, Without};
use bevy_ecs::query::Allow;
use bevy_ecs::system::SystemParam;
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, BufferState, PendingInteractiveResize, ServerDecoration, SurfaceContentVersion,
    SurfaceGeometry, WindowAnimation, WindowFullscreenTarget, WindowLayout,
    WindowManagementHints, WindowMode, WindowPolicyState, WindowPosition, WindowRole,
    WindowSceneGeometry, WindowSize, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::events::{WindowClosed, WindowCreated};
use nekoland_ecs::resources::{
    EntityIndex, FocusedOutputState, PendingPopupServerRequests, PopupServerAction,
    PopupServerRequest, ResizeEdges, SurfaceExtent, WaylandIngress, WindowLifecycleAction,
    WindowLifecycleRequest, WorkArea, XdgSurfaceRole,
};
use nekoland_ecs::views::{OutputRuntime, PopupRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::{
    focused_or_primary_workspace_runtime_target, window_workspace_runtime_id,
};

use crate::layout::floating::{
    centre_x, centre_y, placement_work_area, should_auto_place_floating_window,
};
use crate::viewport::{preferred_primary_output_id, resolve_output_state_for_workspace};
use crate::window_policy::{
    WindowBackgroundState, refresh_window_policy, resolve_background_output_id,
    sync_window_background_role,
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
    pending_xdg_requests: ResMut<'w, crate::xdg::DeferredXdgRequests>,
    pending_popup_requests: ResMut<'w, PendingPopupServerRequests>,
    entity_index: Res<'w, EntityIndex>,
    existing_surfaces: ToplevelSurfaces<'w, 's>,
    outputs: ToplevelOutputs<'w, 's>,
    wayland_ingress: Res<'w, WaylandIngress>,
    focused_output: Res<'w, FocusedOutputState>,
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
pub(crate) fn toplevel_lifecycle_system(
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
        wayland_ingress,
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
    let primary_output_id = preferred_primary_output_id(Some(wayland_ingress.as_ref()));
    let focused_output_id = focused_output.id;
    let mut requests = pending_xdg_requests.take();
    requests.extend(wayland_ingress.pending_xdg_requests.iter().cloned());

    for request in requests {
        match request.action.clone() {
            WindowLifecycleAction::Committed { role: XdgSurfaceRole::Toplevel, size }
                if known_surfaces.insert(request.surface_id) =>
            {
                let workspace_entity = focused_or_primary_workspace_runtime_target(
                    &outputs,
                    focused_output_id,
                    primary_output_id,
                    &entity_index,
                    1,
                );
                let geometry = size.unwrap_or(SurfaceExtent { width: 960, height: 720 });
                let title = format!("Window {}", request.surface_id);
                let policy = config.resolve_window_policy("org.nekoland.demo", &title, false);
                let background =
                    config.resolve_window_background("org.nekoland.demo", &title, false);
                let background_output_id =
                    resolve_background_output_id(&outputs, background.as_ref());
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
                            },
                            management_hints: WindowManagementHints::native_wayland(),
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
                    background_output_id,
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

                let Some((entity, mut window)) = window else {
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
                    let committed_width = size.width.max(1);
                    let committed_height = size.height.max(1);
                    if let Some(mut pending_resize) = window.pending_resize {
                        let inflight = pending_resize.inflight_geometry.clone();
                        if let Some(inflight) = inflight {
                            let committed_geometry = anchored_resize_commit_geometry(
                                &inflight,
                                committed_width,
                                committed_height,
                                pending_resize.edges,
                            );
                            window.scene_geometry.x = committed_geometry.x;
                            window.scene_geometry.y = committed_geometry.y;
                            window.scene_geometry.width = committed_geometry.width;
                            window.scene_geometry.height = committed_geometry.height;
                            window.placement.set_explicit_position(WindowPosition {
                                x: committed_geometry.x,
                                y: committed_geometry.y,
                            });
                            window.placement.floating_size = Some(WindowSize {
                                width: committed_geometry.width,
                                height: committed_geometry.height,
                            });
                            if pending_resize.requested_geometry == inflight {
                                commands.entity(entity).remove::<PendingInteractiveResize>();
                            } else {
                                pending_resize.inflight_geometry = None;
                            }
                        } else {
                            let committed_geometry = anchored_resize_commit_geometry(
                                &pending_resize.requested_geometry,
                                committed_width,
                                committed_height,
                                pending_resize.edges,
                            );
                            window.scene_geometry.x = committed_geometry.x;
                            window.scene_geometry.y = committed_geometry.y;
                            window.scene_geometry.width = committed_geometry.width;
                            window.scene_geometry.height = committed_geometry.height;
                            window.placement.set_explicit_position(WindowPosition {
                                x: committed_geometry.x,
                                y: committed_geometry.y,
                            });
                            window.placement.floating_size = Some(WindowSize {
                                width: committed_geometry.width,
                                height: committed_geometry.height,
                            });
                            if pending_resize.requested_geometry.width == committed_width
                                && pending_resize.requested_geometry.height == committed_height
                            {
                                commands.entity(entity).remove::<PendingInteractiveResize>();
                            }
                        }
                    } else {
                        window.scene_geometry.width = committed_width;
                        window.scene_geometry.height = committed_height;
                        if *window.layout == WindowLayout::Floating
                            && window.placement.floating_size.is_some()
                        {
                            window.placement.floating_size = Some(WindowSize {
                                width: committed_width,
                                height: committed_height,
                            });
                        }
                    }
                } else if let Some(restored) = window.restore.snapshot.as_mut() {
                    restored.geometry.width = size.width.max(1);
                    restored.geometry.height = size.height.max(1);
                    if restored.layout == WindowLayout::Floating
                        && should_auto_place_floating_window(&window.placement, &restored.geometry)
                    {
                        let workspace_id =
                            window_workspace_runtime_id(window.child_of, &workspaces);
                let placement_area =
                    placement_area_for_workspace(&outputs, workspace_id, primary_output_id, &work_area);
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

                let window = &mut *window_runtime.window;
                if let Some(title) = &title {
                    window.title = title.clone();
                }
                if let Some(app_id) = &app_id {
                    window.app_id = app_id.clone();
                }
                let (window_app_id, window_title) =
                    (window.app_id.clone(), window.title.clone());
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
                let background_output_id =
                    resolve_background_output_id(&outputs, background.as_ref());
                let current_background =
                    window_runtime.background.as_ref().map(|background| (*background).clone());
                sync_window_background_role(
                    &mut commands,
                    entity,
                    background_output_id,
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
    primary_output_id: Option<nekoland_ecs::components::OutputId>,
    work_area: &WorkArea,
) -> WorkArea {
    resolve_output_state_for_workspace(outputs, workspace_id, primary_output_id).map_or(
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

fn anchored_resize_commit_geometry(
    target_geometry: &WindowSceneGeometry,
    committed_width: u32,
    committed_height: u32,
    edges: ResizeEdges,
) -> WindowSceneGeometry {
    let right = target_geometry.x.saturating_add(target_geometry.width as isize);
    let bottom = target_geometry.y.saturating_add(target_geometry.height as isize);
    let x = if edges.has_left() {
        right.saturating_sub(committed_width as isize)
    } else {
        target_geometry.x
    };
    let y = if edges.has_top() {
        bottom.saturating_sub(committed_height as isize)
    } else {
        target_geometry.y
    };

    WindowSceneGeometry { x, y, width: committed_width, height: committed_height }
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
    use nekoland_config::resources::{CompositorConfig, ConfiguredWindowRule};
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        BufferState, OutputCurrentWorkspace, OutputDevice, OutputKind, OutputProperties,
        OutputWorkArea, PendingInteractiveResize, SurfaceGeometry, WindowAnimation, WindowLayout,
        WindowManagementHints, WindowMode, WindowPlacement, WindowPosition,
        WindowRestoreSnapshot, WindowRestoreState, WindowSceneGeometry, WindowSize,
        WlSurfaceHandle, Workspace, WorkspaceId, XdgPopup, XdgWindow,
    };
    use nekoland_ecs::events::{WindowClosed, WindowCreated};
    use nekoland_ecs::resources::{
        EntityIndex, FocusedOutputState, PendingPopupServerRequests, PopupServerAction,
        ResizeEdges, SurfaceExtent, WaylandIngress, WindowLifecycleAction,
        WindowLifecycleRequest, WorkArea, XdgSurfaceRole, rebuild_entity_index_system,
    };

    use crate::xdg::DeferredXdgRequests;

    use super::toplevel_lifecycle_system;

    #[test]
    fn toplevel_create_inserts_child_of_active_workspace() {
        let mut app = NekolandApp::new("toplevel-lifecycle-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(WorkArea::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        let workspace_entity = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 21,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(SurfaceExtent { width: 800, height: 600 }),
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
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(WaylandIngress::default());
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
                management_hints: WindowManagementHints::native_wayland(),
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

        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
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
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(WaylandIngress::default());
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
        let virtual_output = app
            .inner_mut()
            .world_mut()
            .spawn((
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
            ))
            .id();
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
        let virtual_output_id = *app
            .inner()
            .world()
            .get::<nekoland_ecs::components::OutputId>(virtual_output)
            .expect("virtual output id");
        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().primary_output.id =
            Some(virtual_output_id);
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
                    management_hints: WindowManagementHints::native_wayland(),
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
                        previous: None,
                    }),
                },
                ChildOf(workspace_2),
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 51,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(SurfaceExtent { width: 200, height: 100 }),
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
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 21,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(SurfaceExtent { width: 800, height: 600 }),
                },
            },
        );
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
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

    #[test]
    fn mismatched_committed_resize_does_not_leave_resize_stuck() {
        let mut app = NekolandApp::new("toplevel-resize-mismatch-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(WorkArea::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 61 },
                    geometry: SurfaceGeometry { x: 10, y: 20, width: 800, height: 600 },
                    scene_geometry: WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 },
                    viewport_visibility: Default::default(),
                    buffer: BufferState { attached: true, scale: 1 },
                    content_version: Default::default(),
                    window: XdgWindow::default(),
                    management_hints: WindowManagementHints::native_wayland(),
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Normal,
                    decoration: Default::default(),
                    border_theme: Default::default(),
                    animation: WindowAnimation::default(),
                },
                PendingInteractiveResize {
                    requested_geometry: WindowSceneGeometry { x: 10, y: 20, width: 1200, height: 900 },
                    inflight_geometry: Some(WindowSceneGeometry {
                        x: 10,
                        y: 20,
                        width: 1200,
                        height: 900,
                    }),
                    edges: ResizeEdges::BottomRight,
                },
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 61,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(SurfaceExtent { width: 1024, height: 768 }),
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner();
        assert!(world.world().get::<PendingInteractiveResize>(entity).is_none());
        assert_eq!(
            world.world().get::<WindowSceneGeometry>(entity),
            Some(&WindowSceneGeometry { x: 10, y: 20, width: 1024, height: 768 })
        );
    }

    #[test]
    fn mismatched_committed_resize_keeps_newer_requested_target() {
        let mut app = NekolandApp::new("toplevel-resize-mismatch-newer-request-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(WorkArea::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 62 },
                    geometry: SurfaceGeometry { x: 10, y: 20, width: 800, height: 600 },
                    scene_geometry: WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 },
                    viewport_visibility: Default::default(),
                    buffer: BufferState { attached: true, scale: 1 },
                    content_version: Default::default(),
                    window: XdgWindow::default(),
                    management_hints: WindowManagementHints::native_wayland(),
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Normal,
                    decoration: Default::default(),
                    border_theme: Default::default(),
                    animation: WindowAnimation::default(),
                },
                PendingInteractiveResize {
                    requested_geometry: WindowSceneGeometry { x: 10, y: 20, width: 1400, height: 1000 },
                    inflight_geometry: Some(WindowSceneGeometry {
                        x: 10,
                        y: 20,
                        width: 1200,
                        height: 900,
                    }),
                    edges: ResizeEdges::BottomRight,
                },
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 62,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(SurfaceExtent { width: 1024, height: 768 }),
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner();
        assert_eq!(
            world.world().get::<PendingInteractiveResize>(entity),
            Some(&PendingInteractiveResize {
                requested_geometry: WindowSceneGeometry { x: 10, y: 20, width: 1400, height: 1000 },
                inflight_geometry: None,
                edges: ResizeEdges::BottomRight,
            })
        );
        assert_eq!(
            world.world().get::<WindowSceneGeometry>(entity),
            Some(&WindowSceneGeometry { x: 10, y: 20, width: 1024, height: 768 })
        );
    }

    #[test]
    fn top_edge_resize_reanchors_position_when_client_commits_constrained_height() {
        let mut app = NekolandApp::new("toplevel-top-resize-anchor-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(WorkArea::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredXdgRequests::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, toplevel_lifecycle_system).chain(),
        );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 63 },
                    geometry: SurfaceGeometry { x: 10, y: 20, width: 800, height: 600 },
                    scene_geometry: WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 },
                    viewport_visibility: Default::default(),
                    buffer: BufferState { attached: true, scale: 1 },
                    content_version: Default::default(),
                    window: XdgWindow::default(),
                    management_hints: WindowManagementHints::native_wayland(),
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Normal,
                    decoration: Default::default(),
                    border_theme: Default::default(),
                    animation: WindowAnimation::default(),
                },
                PendingInteractiveResize {
                    requested_geometry: WindowSceneGeometry { x: 10, y: 120, width: 800, height: 500 },
                    inflight_geometry: Some(WindowSceneGeometry {
                        x: 10,
                        y: 120,
                        width: 800,
                        height: 500,
                    }),
                    edges: ResizeEdges::Top,
                },
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 63,
                action: WindowLifecycleAction::Committed {
                    role: XdgSurfaceRole::Toplevel,
                    size: Some(SurfaceExtent { width: 800, height: 520 }),
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner();
        assert_eq!(
            world.world().get::<WindowSceneGeometry>(entity),
            Some(&WindowSceneGeometry { x: 10, y: 100, width: 800, height: 520 })
        );
        assert_eq!(
            world.world().get::<WindowPlacement>(entity),
            Some(&WindowPlacement {
                floating_position: Some(
                    nekoland_ecs::components::FloatingPosition::Explicit(WindowPosition {
                        x: 10,
                        y: 100,
                    }),
                ),
                floating_size: Some(WindowSize { width: 800, height: 520 }),
            })
        );
    }
}
