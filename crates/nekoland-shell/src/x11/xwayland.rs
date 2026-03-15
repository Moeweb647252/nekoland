use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, With};
use bevy_ecs::query::Allow;
use bevy_ecs::system::SystemParam;
use nekoland_ecs::bundles::X11WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, BufferState, FloatingPosition, ServerDecoration, SurfaceGeometry, WindowAnimation,
    WindowLayout, WindowMode, WindowPlacement, WindowPolicyState, WindowPosition,
    WindowRestoreState, WindowSceneGeometry, WindowSize, WlSurfaceHandle, X11Window, XdgPopup,
    XdgWindow,
};
use nekoland_ecs::events::{WindowClosed, WindowCreated, WindowMoved};
use nekoland_ecs::resources::{
    CompositorConfig, EntityIndex, FocusedOutputState, GlobalPointerPosition, KeyboardFocusState,
    PendingPopupServerRequests, PendingX11Requests, PopupServerAction, PopupServerRequest,
    PrimaryOutputState, X11LifecycleAction,
};
use nekoland_ecs::views::{OutputRuntime, PopupRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::focused_or_primary_workspace_runtime_target;

use crate::interaction::{ActiveWindowGrab, WindowGrabMode, begin_window_grab};
use crate::window_policy::{
    apply_window_policy, lock_window_policy, refresh_window_policy, restore_window_policy,
    sync_window_background_role,
};

/// Bridges XWayland lifecycle requests into the same ECS window model used by native XDG windows.
///
/// The system mirrors the XDG configure logic closely so X11 windows participate in the same
/// focus, layout, restore-state, and popup-dismiss flows.
type X11Windows<'w, 's> =
    Query<'w, 's, (Entity, WindowRuntime), (With<X11Window>, Allow<Disabled>)>;

#[derive(SystemParam)]
pub struct X11PopupDismissals<'w, 's> {
    popups: Query<'w, 's, PopupRuntime, (With<XdgPopup>, Allow<Disabled>)>,
    requests: ResMut<'w, PendingPopupServerRequests>,
}

pub fn xwayland_bridge_system(
    mut commands: Commands,
    config: Res<CompositorConfig>,
    mut pending_x11_requests: ResMut<PendingX11Requests>,
    entity_index: Res<EntityIndex>,
    pointer: Res<GlobalPointerPosition>,
    mut active_grab: ResMut<ActiveWindowGrab>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    _workspaces: Query<(Entity, WorkspaceRuntime)>,
    outputs: Query<(Entity, OutputRuntime)>,
    primary_output: Option<Res<PrimaryOutputState>>,
    focused_output: Option<Res<FocusedOutputState>>,
    mut windows: X11Windows<'_, '_>,
    mut popup_dismissals: X11PopupDismissals<'_, '_>,
    mut window_created: MessageWriter<WindowCreated>,
    mut window_closed: MessageWriter<WindowClosed>,
    mut window_moved: MessageWriter<WindowMoved>,
) {
    let active_workspace_entity = focused_or_primary_workspace_runtime_target(
        &outputs,
        focused_output.as_deref(),
        primary_output.as_deref(),
        &entity_index,
        1,
    );
    let output_geometry = outputs
        .iter()
        .next()
        .map(|(_, output)| (output.properties.width.max(1), output.properties.height.max(1)));
    let mut deferred = Vec::new();

    for request in pending_x11_requests.drain() {
        match request.action.clone() {
            X11LifecycleAction::Mapped {
                window_id,
                override_redirect,
                title,
                app_id,
                geometry,
            } => {
                map_x11_window(
                    request.surface_id,
                    window_id,
                    override_redirect,
                    title,
                    app_id,
                    geometry,
                    active_workspace_entity,
                    &config,
                    &entity_index,
                    &mut windows,
                    &mut commands,
                    &mut window_created,
                    &mut window_moved,
                );
            }
            X11LifecycleAction::Reconfigured { title, app_id, geometry } => {
                if !reconfigure_x11_window(
                    request.surface_id,
                    title,
                    app_id,
                    geometry,
                    &config,
                    &entity_index,
                    &mut windows,
                    &mut commands,
                    &mut window_moved,
                ) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::Maximize => {
                if !enter_x11_window_state(
                    request.surface_id,
                    WindowMode::Maximized,
                    output_geometry,
                    &entity_index,
                    &mut windows,
                ) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::UnMaximize => {
                if !restore_or_default_x11_window_state(
                    request.surface_id,
                    &entity_index,
                    &mut windows,
                ) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::Fullscreen => {
                if !enter_x11_window_state(
                    request.surface_id,
                    WindowMode::Fullscreen,
                    output_geometry,
                    &entity_index,
                    &mut windows,
                ) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::UnFullscreen => {
                if !restore_or_default_x11_window_state(
                    request.surface_id,
                    &entity_index,
                    &mut windows,
                ) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::Minimize => {
                if !minimize_x11_window(request.surface_id, &entity_index, &mut windows) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::UnMinimize => {
                if !restore_or_default_x11_window_state(
                    request.surface_id,
                    &entity_index,
                    &mut windows,
                ) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::InteractiveMove { button: _button } => {
                if !start_x11_window_grab(
                    request.surface_id,
                    WindowGrabMode::Move,
                    &pointer,
                    &entity_index,
                    &mut windows,
                    &mut active_grab,
                    &mut keyboard_focus,
                ) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::InteractiveResize { button: _button, edges } => {
                if !start_x11_window_grab(
                    request.surface_id,
                    WindowGrabMode::Resize { edges },
                    &pointer,
                    &entity_index,
                    &mut windows,
                    &mut active_grab,
                    &mut keyboard_focus,
                ) {
                    deferred.push(request);
                }
            }
            X11LifecycleAction::Unmapped | X11LifecycleAction::Destroyed => {
                destroy_x11_window(
                    request.surface_id,
                    &entity_index,
                    &mut windows,
                    &mut popup_dismissals,
                    &mut commands,
                    &mut window_closed,
                );
            }
        }
    }

    pending_x11_requests.replace(deferred);
}

fn resolve_x11_window_entity(
    surface_id: u64,
    entity_index: &EntityIndex,
    windows: &mut X11Windows<'_, '_>,
) -> Option<Entity> {
    entity_index.entity_for_surface(surface_id).or_else(|| {
        windows
            .iter_mut()
            .find(|(_, window)| window.surface_id() == surface_id)
            .map(|(entity, _)| entity)
    })
}

fn map_x11_window(
    surface_id: u64,
    window_id: u32,
    override_redirect: bool,
    title: String,
    app_id: String,
    geometry: nekoland_ecs::resources::X11WindowGeometry,
    active_workspace_entity: Option<Entity>,
    config: &CompositorConfig,
    entity_index: &EntityIndex,
    windows: &mut X11Windows<'_, '_>,
    commands: &mut Commands,
    window_created: &mut MessageWriter<WindowCreated>,
    window_moved: &mut MessageWriter<WindowMoved>,
) {
    let policy = config.resolve_window_policy(&app_id, &title, override_redirect);
    let background = config.resolve_window_background(&app_id, &title, override_redirect);
    if let Some(entity) = resolve_x11_window_entity(surface_id, entity_index, windows) {
        if let Ok((entity, mut window)) = windows.get_mut(entity) {
            let moved = window.geometry.x != geometry.x || window.geometry.y != geometry.y;
            *window.geometry = SurfaceGeometry {
                x: geometry.x,
                y: geometry.y,
                width: geometry.width.max(1),
                height: geometry.height.max(1),
            };
            *window.scene_geometry = WindowSceneGeometry {
                x: geometry.x as isize,
                y: geometry.y as isize,
                width: geometry.width.max(1),
                height: geometry.height.max(1),
            };
            window.buffer.expect("x11 window should have buffer state").attached = true;
            let xdg_window =
                window.xdg_window.as_mut().expect("x11 runtime should expose xdg metadata");
            xdg_window.title = title.clone();
            xdg_window.app_id = app_id.clone();
            apply_window_policy(
                policy,
                &mut window.layout,
                &mut window.mode,
                &mut window.policy_state,
            );
            sync_window_background_role(
                commands,
                entity,
                background,
                &mut window.scene_geometry,
                &mut window.layout,
                &mut window.mode,
                window.background.as_ref().map(|background| (*background).clone()),
            );
            if matches!(*window.layout, WindowLayout::Floating)
                && *window.mode == WindowMode::Normal
            {
                window.placement.set_explicit_position(WindowPosition {
                    x: geometry.x as isize,
                    y: geometry.y as isize,
                });
                window.placement.floating_size = Some(WindowSize {
                    width: geometry.width.max(1),
                    height: geometry.height.max(1),
                });
            }
            if let Some(active_workspace_entity) = active_workspace_entity {
                commands.entity(entity).insert(ChildOf(active_workspace_entity));
            }
            if moved {
                window_moved.write(WindowMoved {
                    surface_id,
                    x: geometry.x as i64,
                    y: geometry.y as i64,
                });
            }
            return;
        }
    }

    let window_entity = commands
        .spawn((
            X11WindowBundle {
                surface: WlSurfaceHandle { id: surface_id },
                geometry: SurfaceGeometry {
                    x: geometry.x,
                    y: geometry.y,
                    width: geometry.width.max(1),
                    height: geometry.height.max(1),
                },
                scene_geometry: WindowSceneGeometry {
                    x: geometry.x as isize,
                    y: geometry.y as isize,
                    width: geometry.width.max(1),
                    height: geometry.height.max(1),
                },
                viewport_visibility: Default::default(),
                buffer: BufferState { attached: true, scale: 1 },
                content_version: Default::default(),
                window: XdgWindow {
                    app_id: app_id.clone(),
                    title: title.clone(),
                    last_acked_configure: None,
                },
                x11_window: X11Window { window_id, override_redirect },
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
            WindowPlacement {
                floating_position: Some(FloatingPosition::Explicit(WindowPosition {
                    x: geometry.x as isize,
                    y: geometry.y as isize,
                })),
                floating_size: Some(WindowSize {
                    width: geometry.width.max(1),
                    height: geometry.height.max(1),
                }),
            },
        ))
        .id();
    let mut scene_geometry = WindowSceneGeometry {
        x: geometry.x as isize,
        y: geometry.y as isize,
        width: geometry.width.max(1),
        height: geometry.height.max(1),
    };
    let mut layout = policy.layout;
    let mut mode = policy.mode;
    sync_window_background_role(
        commands,
        window_entity,
        background,
        &mut scene_geometry,
        &mut layout,
        &mut mode,
        None,
    );
    commands.entity(window_entity).insert((scene_geometry.clone(), layout, mode));
    if let Some(active_workspace_entity) = active_workspace_entity {
        commands.entity(window_entity).insert(ChildOf(active_workspace_entity));
    }
    window_created.write(WindowCreated { surface_id, title });
}

fn reconfigure_x11_window(
    surface_id: u64,
    title: String,
    app_id: String,
    geometry: nekoland_ecs::resources::X11WindowGeometry,
    config: &CompositorConfig,
    entity_index: &EntityIndex,
    windows: &mut X11Windows<'_, '_>,
    commands: &mut Commands,
    window_moved: &mut MessageWriter<WindowMoved>,
) -> bool {
    let Some(entity) = resolve_x11_window_entity(surface_id, entity_index, windows) else {
        return false;
    };
    let Ok((_, mut window)) = windows.get_mut(entity) else {
        return false;
    };

    window.buffer.expect("x11 window should have buffer state").attached = true;
    let xdg_window = window.xdg_window.as_mut().expect("x11 runtime should expose xdg metadata");
    xdg_window.title = title;
    xdg_window.app_id = app_id;
    let override_redirect =
        window.x11_window.expect("x11 runtime should expose x11 metadata").override_redirect;
    let policy =
        config.resolve_window_policy(&xdg_window.app_id, &xdg_window.title, override_redirect);
    let background =
        config.resolve_window_background(&xdg_window.app_id, &xdg_window.title, override_redirect);
    refresh_window_policy(
        policy,
        &mut window.layout,
        &mut window.mode,
        &mut window.restore,
        &mut window.policy_state,
    );
    sync_window_background_role(
        commands,
        entity,
        background,
        &mut window.scene_geometry,
        &mut window.layout,
        &mut window.mode,
        window.background.as_ref().map(|background| (*background).clone()),
    );

    let moved = window.geometry.x != geometry.x || window.geometry.y != geometry.y;
    let resizable = !matches!(*window.mode, WindowMode::Fullscreen | WindowMode::Maximized);
    window.geometry.x = geometry.x;
    window.geometry.y = geometry.y;
    window.scene_geometry.x = geometry.x as isize;
    window.scene_geometry.y = geometry.y as isize;
    if resizable {
        window.geometry.width = geometry.width.max(1);
        window.geometry.height = geometry.height.max(1);
        window.scene_geometry.width = geometry.width.max(1);
        window.scene_geometry.height = geometry.height.max(1);
    }
    if matches!(*window.layout, WindowLayout::Floating) && *window.mode == WindowMode::Normal {
        window.placement.set_explicit_position(WindowPosition {
            x: geometry.x as isize,
            y: geometry.y as isize,
        });
        if resizable {
            window.placement.floating_size =
                Some(WindowSize { width: geometry.width.max(1), height: geometry.height.max(1) });
        }
    }
    if moved {
        window_moved.write(WindowMoved {
            surface_id,
            x: window.scene_geometry.x as i64,
            y: window.scene_geometry.y as i64,
        });
    }
    true
}

fn enter_x11_window_state(
    surface_id: u64,
    target_mode: WindowMode,
    output_geometry: Option<(u32, u32)>,
    entity_index: &EntityIndex,
    windows: &mut X11Windows<'_, '_>,
) -> bool {
    let Some(entity) = resolve_x11_window_entity(surface_id, entity_index, windows) else {
        return false;
    };
    let Ok((_, mut window)) = windows.get_mut(entity) else {
        return false;
    };

    window.restore.snapshot = Some(WindowRestoreState {
        geometry: window.scene_geometry.clone(),
        layout: (*window.layout).clone(),
        mode: (*window.mode).clone(),
    });
    *window.mode = target_mode;
    if let Some((width, height)) = output_geometry {
        window.geometry.x = 0;
        window.geometry.y = 0;
        window.geometry.width = width;
        window.geometry.height = height;
    }
    true
}

fn restore_or_default_x11_window_state(
    surface_id: u64,
    entity_index: &EntityIndex,
    windows: &mut X11Windows<'_, '_>,
) -> bool {
    let Some(entity) = resolve_x11_window_entity(surface_id, entity_index, windows) else {
        return false;
    };
    let Ok((_, mut window)) = windows.get_mut(entity) else {
        return false;
    };

    if let Some(restored) = window.restore.snapshot.take() {
        *window.scene_geometry = restored.geometry;
        *window.layout = restored.layout;
        *window.mode = restored.mode;
    } else {
        restore_window_policy(&window.policy_state, &mut window.layout, &mut window.mode);
    }
    true
}

fn minimize_x11_window(
    surface_id: u64,
    entity_index: &EntityIndex,
    windows: &mut X11Windows<'_, '_>,
) -> bool {
    let Some(entity) = resolve_x11_window_entity(surface_id, entity_index, windows) else {
        return false;
    };
    let Ok((_, mut window)) = windows.get_mut(entity) else {
        return false;
    };

    *window.mode = WindowMode::Hidden;
    true
}

fn start_x11_window_grab(
    surface_id: u64,
    mode: WindowGrabMode,
    pointer: &GlobalPointerPosition,
    entity_index: &EntityIndex,
    windows: &mut X11Windows<'_, '_>,
    active_grab: &mut ActiveWindowGrab,
    keyboard_focus: &mut KeyboardFocusState,
) -> bool {
    let Some(entity) = resolve_x11_window_entity(surface_id, entity_index, windows) else {
        return false;
    };
    let Ok((_, mut window)) = windows.get_mut(entity) else {
        return false;
    };
    if window.background.is_some() {
        return true;
    }

    let override_redirect =
        window.x11_window.expect("x11 runtime should expose x11 metadata").override_redirect;
    restore_window_policy(&window.policy_state, &mut window.layout, &mut window.mode);
    if !override_redirect {
        *window.layout = WindowLayout::Floating;
        *window.mode = WindowMode::Normal;
        lock_window_policy(*window.layout, *window.mode, &mut window.policy_state);
    }
    keyboard_focus.focused_surface = Some(surface_id);
    begin_window_grab(active_grab, surface_id, mode, pointer, &window.scene_geometry);
    true
}

fn destroy_x11_window(
    surface_id: u64,
    entity_index: &EntityIndex,
    windows: &mut X11Windows<'_, '_>,
    popup_dismissals: &mut X11PopupDismissals<'_, '_>,
    commands: &mut Commands,
    window_closed: &mut MessageWriter<WindowClosed>,
) {
    let Some(entity) = resolve_x11_window_entity(surface_id, entity_index, windows) else {
        return;
    };

    let popup_surface_ids =
        popup_dismissal_surface_ids(surface_id, entity_index, &popup_dismissals.popups);
    for popup_surface_id in popup_surface_ids {
        popup_dismissals.requests.push(PopupServerRequest {
            surface_id: popup_surface_id,
            action: PopupServerAction::Dismiss,
        });
    }
    commands.entity(entity).despawn();
    window_closed.write(WindowClosed { surface_id });
}

fn popup_dismissal_surface_ids(
    parent_surface_id: u64,
    entity_index: &EntityIndex,
    popups: &Query<PopupRuntime, (With<XdgPopup>, Allow<Disabled>)>,
) -> Vec<u64> {
    let Some(parent_entity) = entity_index.entity_for_surface(parent_surface_id) else {
        return Vec::new();
    };
    let mut dismissed = std::collections::BTreeSet::new();
    let mut surface_ids = Vec::new();
    for popup in popups.iter() {
        if popup.child_of.parent() != parent_entity || !dismissed.insert(popup.surface_id()) {
            continue;
        }

        surface_ids.push(popup.surface_id());
    }

    surface_ids
}

#[cfg(test)]
mod tests {
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::Entity;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::X11WindowBundle;
    use nekoland_ecs::components::{
        BorderTheme, BufferState, ServerDecoration, SurfaceGeometry, WindowAnimation, WindowLayout,
        WindowMode, WindowSceneGeometry, WlSurfaceHandle, Workspace, WorkspaceId, X11Window,
        XdgWindow,
    };
    use nekoland_ecs::events::{PointerButton, WindowClosed, WindowCreated, WindowMoved};
    use nekoland_ecs::resources::{
        CompositorConfig, ConfiguredWindowRule, EntityIndex, GlobalPointerPosition,
        KeyboardFocusState, PendingX11Requests, ResizeEdges, WindowStackingState,
        X11LifecycleAction, X11LifecycleRequest, X11WindowGeometry, rebuild_entity_index_system,
    };

    use crate::interaction::{self, ActiveWindowGrab};

    use super::xwayland_bridge_system;

    fn setup_app_with_window() -> (NekolandApp, Entity) {
        let mut app = NekolandApp::new("xwayland-bridge-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(GlobalPointerPosition { x: 320.0, y: 180.0 })
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(ActiveWindowGrab::default())
            .insert_resource(WindowStackingState::default())
            .insert_resource(PendingX11Requests::default())
            .insert_resource(nekoland_ecs::resources::PendingPopupServerRequests::default());
        app.inner_mut()
            .add_message::<PointerButton>()
            .add_message::<WindowCreated>()
            .add_message::<WindowClosed>()
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (
                    rebuild_entity_index_system,
                    xwayland_bridge_system,
                    interaction::window_grab_system,
                )
                    .chain(),
            );

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((X11WindowBundle {
                surface: WlSurfaceHandle { id: 42 },
                geometry: SurfaceGeometry { x: 32, y: 48, width: 640, height: 480 },
                scene_geometry: WindowSceneGeometry { x: 32, y: 48, width: 640, height: 480 },
                viewport_visibility: Default::default(),
                buffer: BufferState { attached: true, scale: 1 },
                content_version: Default::default(),
                window: XdgWindow {
                    app_id: "x11-test".to_owned(),
                    title: "X11 Test".to_owned(),
                    last_acked_configure: None,
                },
                x11_window: X11Window { window_id: 7, override_redirect: false },
                layout: WindowLayout::Tiled,
                mode: WindowMode::Normal,
                decoration: ServerDecoration { enabled: true },
                border_theme: BorderTheme::default(),
                animation: WindowAnimation::default(),
            },))
            .id();

        (app, entity)
    }

    #[test]
    fn x11_interactive_move_request_updates_geometry_and_focus() {
        let (mut app, entity) = setup_app_with_window();
        app.inner_mut().world_mut().resource_mut::<PendingX11Requests>().push(
            X11LifecycleRequest {
                surface_id: 42,
                action: X11LifecycleAction::InteractiveMove { button: 1 },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>().x = 352.0;
        app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>().y = 196.0;
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let geometry = world
            .get::<SurfaceGeometry>(entity)
            .expect("x11 window geometry should still exist after move");
        let layout = world.get::<WindowLayout>(entity).expect("x11 window layout should exist");
        let mode = world.get::<WindowMode>(entity).expect("x11 window mode should exist");
        let focus = world
            .get_resource::<KeyboardFocusState>()
            .expect("keyboard focus state should be initialized");

        assert_eq!((geometry.x, geometry.y), (64, 64));
        assert_eq!(*layout, WindowLayout::Floating);
        assert_eq!(*mode, WindowMode::Normal);
        assert_eq!(focus.focused_surface, Some(42));
    }

    #[test]
    fn x11_interactive_resize_request_updates_geometry_and_focus() {
        let (mut app, entity) = setup_app_with_window();
        app.inner_mut().world_mut().resource_mut::<PendingX11Requests>().push(
            X11LifecycleRequest {
                surface_id: 42,
                action: X11LifecycleAction::InteractiveResize {
                    button: 1,
                    edges: ResizeEdges::BottomRight,
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>().x = 352.0;
        app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>().y = 196.0;
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let geometry = world
            .get::<SurfaceGeometry>(entity)
            .expect("x11 window geometry should still exist after resize");
        let layout = world.get::<WindowLayout>(entity).expect("x11 window layout should exist");
        let mode = world.get::<WindowMode>(entity).expect("x11 window mode should exist");
        let focus = world
            .get_resource::<KeyboardFocusState>()
            .expect("keyboard focus state should be initialized");

        assert_eq!((geometry.width, geometry.height), (672, 496));
        assert_eq!(*layout, WindowLayout::Floating);
        assert_eq!(*mode, WindowMode::Normal);
        assert_eq!(focus.focused_surface, Some(42));
    }

    #[test]
    fn mapped_window_inserts_child_of_active_workspace() {
        let mut app = NekolandApp::new("xwayland-workspace-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(GlobalPointerPosition::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(ActiveWindowGrab::default())
            .insert_resource(WindowStackingState::default())
            .insert_resource(PendingX11Requests::default())
            .insert_resource(nekoland_ecs::resources::PendingPopupServerRequests::default());
        app.inner_mut()
            .add_message::<PointerButton>()
            .add_message::<WindowCreated>()
            .add_message::<WindowClosed>()
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (
                    rebuild_entity_index_system,
                    xwayland_bridge_system,
                    interaction::window_grab_system,
                )
                    .chain(),
            );

        let workspace_entity = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        app.inner_mut().world_mut().resource_mut::<PendingX11Requests>().push(
            X11LifecycleRequest {
                surface_id: 77,
                action: X11LifecycleAction::Mapped {
                    window_id: 5,
                    override_redirect: false,
                    title: "mapped".to_owned(),
                    app_id: "mapped.app".to_owned(),
                    geometry: X11WindowGeometry { x: 16, y: 24, width: 640, height: 480 },
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let mut windows = world.query::<(Entity, &WlSurfaceHandle)>();
        let window_entity = windows
            .iter(world)
            .find(|(_, surface)| surface.id == 77)
            .map(|(entity, _)| entity)
            .expect("mapped X11 window should exist");
        let child_of =
            world.get::<ChildOf>(window_entity).expect("mapped X11 window should have ChildOf");
        assert_eq!(
            child_of.parent(),
            workspace_entity,
            "mapped X11 window should attach to the active workspace entity",
        );
    }

    #[test]
    fn mapped_window_applies_matching_window_rule() {
        let mut app = NekolandApp::new("xwayland-policy-test");
        let mut config = CompositorConfig::default();
        config.window_rules.push(ConfiguredWindowRule {
            app_id: Some("mapped.app".to_owned()),
            title: None,
            layout: Some(WindowLayout::Tiled),
            mode: None,
            background: None,
        });
        app.insert_resource(config)
            .insert_resource(EntityIndex::default())
            .insert_resource(GlobalPointerPosition::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(ActiveWindowGrab::default())
            .insert_resource(WindowStackingState::default())
            .insert_resource(PendingX11Requests::default())
            .insert_resource(nekoland_ecs::resources::PendingPopupServerRequests::default());
        app.inner_mut()
            .add_message::<PointerButton>()
            .add_message::<WindowCreated>()
            .add_message::<WindowClosed>()
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (
                    rebuild_entity_index_system,
                    xwayland_bridge_system,
                    interaction::window_grab_system,
                )
                    .chain(),
            );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        app.inner_mut().world_mut().resource_mut::<PendingX11Requests>().push(
            X11LifecycleRequest {
                surface_id: 88,
                action: X11LifecycleAction::Mapped {
                    window_id: 9,
                    override_redirect: false,
                    title: "mapped".to_owned(),
                    app_id: "mapped.app".to_owned(),
                    geometry: X11WindowGeometry { x: 40, y: 56, width: 640, height: 480 },
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let mut windows = world.query::<(&WlSurfaceHandle, &WindowLayout, &WindowMode)>();
        let (_, layout, mode) = windows
            .iter(world)
            .find(|(surface, _, _)| surface.id == 88)
            .expect("mapped X11 window should exist");
        assert_eq!(*layout, WindowLayout::Tiled);
        assert_eq!(*mode, WindowMode::Normal);
    }
}
