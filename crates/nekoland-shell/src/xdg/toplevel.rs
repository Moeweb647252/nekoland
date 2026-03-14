use std::collections::BTreeSet;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, BufferState, ServerDecoration, SurfaceContentVersion, SurfaceGeometry,
    WindowAnimation, WindowLayout, WindowMode, WindowPolicyState, WindowSceneGeometry,
    WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::events::{WindowClosed, WindowCreated};
use nekoland_ecs::resources::{
    CompositorConfig, EntityIndex, FocusedOutputState, PendingPopupServerRequests,
    PendingXdgRequests, PopupServerAction, PopupServerRequest, PrimaryOutputState,
    WindowLifecycleAction, WindowLifecycleRequest, WorkArea, XdgSurfaceRole,
};
use nekoland_ecs::views::{OutputRuntime, PopupRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::focused_or_primary_workspace_runtime_target;

use crate::layout::floating::{
    centre_x, centre_y, placement_work_area, should_auto_place_floating_window,
};
use crate::window_policy::refresh_window_policy;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToplevelManager;

/// Owns the XDG toplevel lifecycle bridge from protocol requests into ECS window entities.
///
/// Requests that arrive before the corresponding entity exists are deferred and retried on later
/// ticks instead of being dropped.
pub fn toplevel_lifecycle_system(
    mut commands: Commands,
    config: Res<CompositorConfig>,
    work_area: Res<WorkArea>,
    mut pending_xdg_requests: ResMut<PendingXdgRequests>,
    mut pending_popup_requests: ResMut<PendingPopupServerRequests>,
    entity_index: Res<EntityIndex>,
    existing_surfaces: Query<&WlSurfaceHandle, (With<XdgWindow>, Allow<Disabled>)>,
    outputs: Query<(Entity, OutputRuntime)>,
    primary_output: Option<Res<PrimaryOutputState>>,
    focused_output: Option<Res<FocusedOutputState>>,
    mut windows: Query<(Entity, WindowRuntime), (With<XdgWindow>, Allow<Disabled>)>,
    popups: Query<PopupRuntime, (With<XdgPopup>, Allow<Disabled>)>,
    _workspaces: Query<(Entity, WorkspaceRuntime)>,
    mut window_created: MessageWriter<WindowCreated>,
    mut window_closed: MessageWriter<WindowClosed>,
) {
    let mut known_surfaces =
        existing_surfaces.iter().map(|surface| surface.id).collect::<BTreeSet<_>>();
    let mut deferred = Vec::new();
    let placement_area =
        placement_work_area(&work_area, outputs.iter().next().map(|(_, output)| output.properties));

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
                let mut window_entity = commands.spawn((
                    WindowBundle {
                        surface: WlSurfaceHandle { id: request.surface_id },
                        geometry: SurfaceGeometry {
                            x: 0,
                            y: 0,
                            width: geometry.width.max(1),
                            height: geometry.height.max(1),
                        },
                        scene_geometry: WindowSceneGeometry {
                            x: 0,
                            y: 0,
                            width: geometry.width.max(1),
                            height: geometry.height.max(1),
                        },
                        viewport_visibility: Default::default(),
                        buffer: BufferState { attached: size.is_some(), scale: 1 },
                        content_version: SurfaceContentVersion { value: 1 },
                        window: XdgWindow {
                            app_id: "org.nekoland.demo".to_owned(),
                            title: title.clone(),
                            last_acked_configure: None,
                        },
                        layout: policy.layout,
                        mode: policy.mode,
                        decoration: ServerDecoration { enabled: true },
                        border_theme: BorderTheme {
                            width: policy.layout.border_width(),
                            color: config.border_color.clone(),
                        },
                        animation: WindowAnimation::default(),
                    },
                    WindowPolicyState { applied: policy, locked: false },
                ));
                if let Some(workspace_entity) = workspace_entity {
                    window_entity.insert(ChildOf(workspace_entity));
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

                window.buffer.expect("xdg toplevel should have buffer state").attached = true;
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

                let Some((_, mut window_runtime)) = window else {
                    deferred.push(request);
                    continue;
                };

                let window = window_runtime.xdg_window.as_mut().expect("xdg window should exist");
                if let Some(title) = &title {
                    window.title = title.clone();
                }
                if let Some(app_id) = &app_id {
                    window.app_id = app_id.clone();
                }
                let policy = config.resolve_window_policy(&window.app_id, &window.title, false);
                refresh_window_policy(
                    policy,
                    &mut window_runtime.layout,
                    &mut window_runtime.mode,
                    &mut window_runtime.restore,
                    &mut window_runtime.policy_state,
                );
            }
            _ => deferred.push(request),
        }
    }

    pending_xdg_requests.replace(deferred);
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
    popups: &Query<PopupRuntime, (With<XdgPopup>, Allow<Disabled>)>,
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
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        BufferState, SurfaceGeometry, WindowAnimation, WindowLayout, WindowMode,
        WindowSceneGeometry, WlSurfaceHandle, Workspace, WorkspaceId, XdgPopup, XdgWindow,
    };
    use nekoland_ecs::events::{WindowClosed, WindowCreated};
    use nekoland_ecs::resources::{
        CompositorConfig, ConfiguredWindowRule, EntityIndex, PendingPopupServerRequests,
        PendingXdgRequests, PopupServerAction, WindowLifecycleAction, WindowLifecycleRequest,
        WorkArea, XdgSurfaceRole, rebuild_entity_index_system,
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
            .expect("created toplevel window should exist");

        let child_of = world.get::<ChildOf>(window_entity).expect("window should have ChildOf");
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
            XdgPopup { configure_serial: None, grab_serial: None, reposition_token: None },
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
    fn metadata_change_reapplies_matching_window_rule() {
        let mut app = NekolandApp::new("toplevel-policy-test");
        let mut config = CompositorConfig::default();
        config.window_rules.push(ConfiguredWindowRule {
            app_id: Some("org.nekoland.rules".to_owned()),
            title: None,
            layout: Some(WindowLayout::Tiled),
            mode: None,
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
        let (_, layout, mode) = windows
            .iter(world)
            .find(|(surface, _, _)| surface.id == 21)
            .expect("toplevel window should still exist");
        assert_eq!(*layout, WindowLayout::Tiled);
        assert_eq!(*mode, WindowMode::Normal);
    }
}
