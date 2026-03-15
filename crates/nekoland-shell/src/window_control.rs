use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Query, ResMut, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{WindowLayout, WindowMode, WindowPosition, WindowSize, XdgWindow};
use nekoland_ecs::events::WindowMoved;
use nekoland_ecs::resources::{
    EntityIndex, KeyboardFocusState, PendingWindowControls, PendingWindowServerRequests,
    PrimaryOutputState, UNASSIGNED_WORKSPACE_STACK_ID, UNASSIGNED_WORKSPACE_TILING_ID,
    WindowServerAction, WindowServerRequest, WindowStackingState, WorkspaceTilingState,
};
use nekoland_ecs::views::{OutputRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::viewport::{project_scene_geometry, resolve_output_state_for_workspace};
use crate::window_policy::{lock_window_policy, sync_window_background_role};

/// Applies high-level staged window controls to the current window set.
///
/// Geometry-related controls update `WindowPlacement`, while close requests are translated into
/// the lower-level protocol-close queue that already exists.
pub fn window_control_request_system(
    mut commands: Commands,
    mut pending_window_controls: ResMut<PendingWindowControls>,
    entity_index: bevy_ecs::prelude::Res<EntityIndex>,
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut stacking: ResMut<WindowStackingState>,
    mut tiling: ResMut<WorkspaceTilingState>,
    mut windows: Query<WindowRuntime, (With<XdgWindow>, Allow<Disabled>)>,
    outputs: Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    primary_output: Option<bevy_ecs::prelude::Res<PrimaryOutputState>>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
    mut window_moved: MessageWriter<WindowMoved>,
) {
    if pending_window_controls.is_empty() {
        return;
    }

    let mut deferred = Vec::new();

    for control in pending_window_controls.take() {
        let Some(entity) = entity_index.entity_for_surface(control.surface_id.0) else {
            deferred.push(control);
            continue;
        };
        let Some(mut window) = windows.get_mut(entity).ok() else {
            deferred.push(control);
            continue;
        };
        let workspace_id = window_workspace_runtime_id(window.child_of, &workspaces)
            .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID);
        let output_state = resolve_output_state_for_workspace(
            &outputs,
            Some(workspace_id),
            primary_output.as_deref(),
        );
        let is_background = window.background.is_some();

        if is_background {
            if control.close {
                pending_window_requests.push(WindowServerRequest {
                    surface_id: window.surface_id(),
                    action: WindowServerAction::Close,
                });
            }

            if let Some(background_control) = control.background {
                match background_control {
                    nekoland_ecs::resources::WindowBackgroundControl::Set { output } => {
                        sync_window_background_role(
                            &mut commands,
                            entity,
                            Some(output),
                            &mut window.scene_geometry,
                            &mut window.layout,
                            &mut window.mode,
                            window.background.as_ref().map(|background| (*background).clone()),
                        );
                        if keyboard_focus.focused_surface == Some(window.surface_id()) {
                            keyboard_focus.focused_surface = None;
                        }
                    }
                    nekoland_ecs::resources::WindowBackgroundControl::Clear => {
                        sync_window_background_role(
                            &mut commands,
                            entity,
                            None,
                            &mut window.scene_geometry,
                            &mut window.layout,
                            &mut window.mode,
                            window.background.as_ref().map(|background| (*background).clone()),
                        );
                    }
                }
            }

            continue;
        }

        if let Some(position) = control.position {
            window.placement.set_explicit_position(WindowPosition { x: position.x, y: position.y });
            window.scene_geometry.x = position.x;
            window.scene_geometry.y = position.y;
            *window.layout = WindowLayout::Floating;
            *window.mode = WindowMode::Normal;
            lock_window_policy(*window.layout, *window.mode, &mut window.policy_state);
            stacking.raise(workspace_id, window.surface_id());
            if let Some((_, _, viewport, _)) = output_state.as_ref() {
                *window.geometry = project_scene_geometry(&window.scene_geometry, viewport);
            }
            window_moved.write(WindowMoved {
                surface_id: window.surface_id(),
                x: position.x as i64,
                y: position.y as i64,
            });
        }

        if let Some(size) = control.size {
            window.placement.floating_size =
                Some(WindowSize { width: size.width.max(64), height: size.height.max(64) });
            window.scene_geometry.width = size.width.max(64);
            window.scene_geometry.height = size.height.max(64);
            *window.layout = WindowLayout::Floating;
            *window.mode = WindowMode::Normal;
            lock_window_policy(*window.layout, *window.mode, &mut window.policy_state);
            stacking.raise(workspace_id, window.surface_id());
            if control.position.is_none() {
                window.placement.set_explicit_position(WindowPosition {
                    x: window.scene_geometry.x,
                    y: window.scene_geometry.y,
                });
            }
            if let Some((_, _, viewport, _)) = output_state.as_ref() {
                *window.geometry = project_scene_geometry(&window.scene_geometry, viewport);
            }
        }

        if let Some(axis) = control.split_axis {
            *window.layout = WindowLayout::Tiled;
            *window.mode = WindowMode::Normal;
            lock_window_policy(*window.layout, *window.mode, &mut window.policy_state);
            let tiling_workspace_id = if workspace_id == UNASSIGNED_WORKSPACE_STACK_ID {
                UNASSIGNED_WORKSPACE_TILING_ID
            } else {
                workspace_id
            };
            tiling.ensure_surface(tiling_workspace_id, window.surface_id());
            tiling.set_surface_split_axis(tiling_workspace_id, window.surface_id(), axis);
        }

        if control.focus
            && *window.mode != WindowMode::Hidden
            && window.x11_window.is_none_or(|window| !window.override_redirect)
        {
            keyboard_focus.focused_surface = Some(window.surface_id());
            stacking.raise(workspace_id, window.surface_id());
        }

        if control.close {
            pending_window_requests.push(WindowServerRequest {
                surface_id: window.surface_id(),
                action: WindowServerAction::Close,
            });
        }

        if let Some(background_control) = control.background {
            match background_control {
                nekoland_ecs::resources::WindowBackgroundControl::Set { output } => {
                    sync_window_background_role(
                        &mut commands,
                        entity,
                        Some(output),
                        &mut window.scene_geometry,
                        &mut window.layout,
                        &mut window.mode,
                        window.background.as_ref().map(|background| (*background).clone()),
                    );
                    if keyboard_focus.focused_surface == Some(window.surface_id()) {
                        keyboard_focus.focused_surface = None;
                    }
                }
                nekoland_ecs::resources::WindowBackgroundControl::Clear => {
                    sync_window_background_role(
                        &mut commands,
                        entity,
                        None,
                        &mut window.scene_geometry,
                        &mut window.layout,
                        &mut window.mode,
                        window.background.as_ref().map(|background| (*background).clone()),
                    );
                }
            }
        }
    }

    pending_window_controls.replace(deferred);
}

#[cfg(test)]
mod tests {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        OutputBackgroundWindow, OutputProperties, WindowLayout, WindowMode, WindowPlacement,
        WlSurfaceHandle, Workspace, WorkspaceId,
    };
    use nekoland_ecs::events::WindowMoved;
    use nekoland_ecs::resources::{
        EntityIndex, KeyboardFocusState, PendingWindowControls, PendingWindowServerRequests,
        WindowStackingState, WorkArea, WorkspaceTilingState, rebuild_entity_index_system,
    };
    use nekoland_ecs::selectors::SurfaceId;

    use crate::layout::{floating::floating_layout_system, tiling::tiling_layout_system};

    use super::window_control_request_system;

    #[test]
    fn move_and_resize_controls_update_window_placement_and_geometry() {
        let mut app = NekolandApp::new("window-control-test");
        app.inner_mut()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<EntityIndex>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WorkspaceTilingState>()
            .insert_resource(WorkArea { x: 0, y: 0, width: 1280, height: 720 })
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (
                    rebuild_entity_index_system,
                    window_control_request_system,
                    tiling_layout_system,
                    floating_layout_system,
                )
                    .chain(),
            );

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 7 },
                geometry: nekoland_ecs::components::SurfaceGeometry {
                    x: 0,
                    y: 0,
                    width: 320,
                    height: 240,
                },
                buffer: nekoland_ecs::components::BufferState { attached: true, scale: 1 },
                layout: WindowLayout::Tiled,
                mode: WindowMode::Normal,
                ..Default::default()
            })
            .id();
        app.inner_mut().world_mut().spawn((
            nekoland_ecs::components::OutputDevice {
                name: "Winit-1".to_owned(),
                kind: nekoland_ecs::components::OutputKind::Nested,
                make: "Winit".to_owned(),
                model: "test".to_owned(),
            },
            OutputProperties { width: 1280, height: 720, refresh_millihz: 60_000, scale: 1 },
        ));

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWindowControls>()
            .surface(SurfaceId(7))
            .move_to(100, 120)
            .resize_to(800, 600);

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let placement =
            world.get::<WindowPlacement>(entity).expect("window placement should exist");
        let geometry =
            world.get::<nekoland_ecs::components::SurfaceGeometry>(entity).expect("geometry");
        let layout = world.get::<WindowLayout>(entity).expect("window layout should exist");
        let mode = world.get::<WindowMode>(entity).expect("window mode should exist");

        assert_eq!(placement.resolved_floating_position().expect("position").x, 100);
        assert_eq!(placement.resolved_floating_position().expect("position").y, 120);
        assert_eq!(placement.floating_size.expect("size").width, 800);
        assert_eq!(placement.floating_size.expect("size").height, 600);
        assert_eq!((geometry.x, geometry.y, geometry.width, geometry.height), (100, 120, 800, 600));
        assert_eq!(*layout, WindowLayout::Floating);
        assert_eq!(*mode, WindowMode::Normal);
    }

    #[test]
    fn split_controls_switch_window_to_tiled_geometry() {
        let mut app = NekolandApp::new("window-control-split-test");
        app.inner_mut()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<EntityIndex>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WorkspaceTilingState>()
            .insert_resource(WorkArea { x: 0, y: 0, width: 1200, height: 800 })
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (rebuild_entity_index_system, window_control_request_system, tiling_layout_system)
                    .chain(),
            );

        let workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        let left = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 17 },
                    geometry: nekoland_ecs::components::SurfaceGeometry {
                        x: 80,
                        y: 60,
                        width: 320,
                        height: 240,
                    },
                    buffer: nekoland_ecs::components::BufferState { attached: true, scale: 1 },
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                bevy_ecs::hierarchy::ChildOf(workspace),
            ))
            .id();
        let right = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 18 },
                    geometry: nekoland_ecs::components::SurfaceGeometry {
                        x: 420,
                        y: 60,
                        width: 320,
                        height: 240,
                    },
                    buffer: nekoland_ecs::components::BufferState { attached: true, scale: 1 },
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                bevy_ecs::hierarchy::ChildOf(workspace),
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWindowControls>()
            .surface(SurfaceId(17))
            .split_vertical();
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let left_geometry = world
            .get::<nekoland_ecs::components::SurfaceGeometry>(left)
            .expect("left tiled window should exist");
        let right_geometry = world
            .get::<nekoland_ecs::components::SurfaceGeometry>(right)
            .expect("right tiled window should exist");

        assert_eq!(
            (left_geometry.x, left_geometry.y, left_geometry.width, left_geometry.height),
            (0, 0, 1200, 400)
        );
        assert_eq!(
            (right_geometry.x, right_geometry.y, right_geometry.width, right_geometry.height),
            (0, 400, 1200, 400)
        );
    }

    #[test]
    fn close_controls_translate_to_low_level_close_requests() {
        let mut app = NekolandApp::new("window-control-close-test");
        app.inner_mut()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<EntityIndex>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WorkspaceTilingState>()
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (rebuild_entity_index_system, window_control_request_system).chain(),
            );

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 9 },
            buffer: nekoland_ecs::components::BufferState { attached: true, scale: 1 },
            ..Default::default()
        });

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWindowControls>()
            .surface(SurfaceId(9))
            .close();
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let requests = app
            .inner()
            .world()
            .get_resource::<PendingWindowServerRequests>()
            .expect("window request queue should exist");
        assert_eq!(requests.len(), 1);
        assert!(matches!(
            requests.as_slice()[0].action,
            nekoland_ecs::resources::WindowServerAction::Close
        ));
    }

    #[test]
    fn focus_controls_update_keyboard_focus() {
        let mut app = NekolandApp::new("window-control-focus-test");
        app.inner_mut()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<EntityIndex>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WorkspaceTilingState>()
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (rebuild_entity_index_system, window_control_request_system).chain(),
            );

        app.inner_mut()
            .world_mut()
            .spawn(WindowBundle { surface: WlSurfaceHandle { id: 11 }, ..Default::default() });

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWindowControls>()
            .surface(SurfaceId(11))
            .focus();
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let focus = app
            .inner()
            .world()
            .get_resource::<KeyboardFocusState>()
            .expect("keyboard focus should exist");
        assert_eq!(focus.focused_surface, Some(11));
    }

    #[test]
    fn background_controls_insert_and_clear_output_background_role() {
        let mut app = NekolandApp::new("window-control-background-test");
        app.inner_mut()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<EntityIndex>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WorkspaceTilingState>()
            .insert_resource(WorkArea { x: 0, y: 0, width: 1280, height: 720 })
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (rebuild_entity_index_system, window_control_request_system).chain(),
            );

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 13 },
                buffer: nekoland_ecs::components::BufferState { attached: true, scale: 1 },
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                ..Default::default()
            })
            .id();

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWindowControls>()
            .surface(SurfaceId(13))
            .background_on("Virtual-1");
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let background = world
            .get::<OutputBackgroundWindow>(entity)
            .expect("background role should be inserted");
        assert_eq!(background.output, "Virtual-1");

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWindowControls>()
            .surface(SurfaceId(13))
            .clear_background();
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        assert!(
            app.inner().world().get::<OutputBackgroundWindow>(entity).is_none(),
            "background role should be removable",
        );
    }
}
