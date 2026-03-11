use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, With};
use nekoland_ecs::bundles::X11WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, BufferState, LayoutSlot, OutputProperties, ServerDecoration, SurfaceGeometry,
    WindowAnimation, WindowState, WlSurfaceHandle, Workspace, X11Window, XdgWindow,
};
use nekoland_ecs::events::{WindowClosed, WindowCreated, WindowMoved};
use nekoland_ecs::resources::{
    CompositorConfig, GlobalPointerPosition, KeyboardFocusState, PendingX11Requests,
    X11LifecycleAction,
};

pub fn xwayland_bridge_system(
    mut commands: Commands,
    config: Res<CompositorConfig>,
    mut pending_x11_requests: ResMut<PendingX11Requests>,
    pointer: Res<GlobalPointerPosition>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    workspaces: Query<&Workspace>,
    outputs: Query<&OutputProperties>,
    mut windows: Query<
        (
            Entity,
            &WlSurfaceHandle,
            &mut SurfaceGeometry,
            &mut BufferState,
            &mut XdgWindow,
            &X11Window,
            &mut WindowState,
            &mut LayoutSlot,
        ),
        With<X11Window>,
    >,
    mut window_created: MessageWriter<WindowCreated>,
    mut window_closed: MessageWriter<WindowClosed>,
    mut window_moved: MessageWriter<WindowMoved>,
) {
    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.active)
        .map(|workspace| workspace.id.0)
        .unwrap_or(1);
    let output_geometry =
        outputs.iter().next().map(|properties| (properties.width.max(1), properties.height.max(1)));
    let mut deferred = Vec::new();

    for request in pending_x11_requests.items.drain(..) {
        match request.action.clone() {
            X11LifecycleAction::Mapped {
                window_id,
                override_redirect,
                title,
                app_id,
                geometry,
            } => {
                if let Some((
                    _,
                    _,
                    mut existing_geometry,
                    mut buffer,
                    mut window,
                    _,
                    mut state,
                    mut layout_slot,
                )) = windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                {
                    let moved =
                        existing_geometry.x != geometry.x || existing_geometry.y != geometry.y;
                    *existing_geometry = SurfaceGeometry {
                        x: geometry.x,
                        y: geometry.y,
                        width: geometry.width.max(1),
                        height: geometry.height.max(1),
                    };
                    buffer.attached = true;
                    window.title = title.clone();
                    window.app_id = app_id.clone();
                    *state = restored_window_state(&config, override_redirect);
                    layout_slot.workspace = active_workspace;
                    if moved {
                        window_moved.write(WindowMoved {
                            surface_id: request.surface_id,
                            x: geometry.x,
                            y: geometry.y,
                        });
                    }
                    continue;
                }

                commands.spawn((
                    X11WindowBundle {
                        surface: WlSurfaceHandle { id: request.surface_id },
                        geometry: SurfaceGeometry {
                            x: geometry.x,
                            y: geometry.y,
                            width: geometry.width.max(1),
                            height: geometry.height.max(1),
                        },
                        buffer: BufferState { attached: true, scale: 1 },
                        window: XdgWindow {
                            app_id: app_id.clone(),
                            title: title.clone(),
                            last_acked_configure: None,
                        },
                        x11_window: X11Window { window_id, override_redirect },
                        state: restored_window_state(&config, override_redirect),
                        decoration: ServerDecoration { enabled: true },
                        border_theme: BorderTheme {
                            width: border_width(&config.default_layout),
                            color: config.border_color.clone(),
                        },
                        animation: WindowAnimation::default(),
                    },
                    LayoutSlot { workspace: active_workspace, column: 0, row: 0 },
                ));
                window_created.write(WindowCreated { surface_id: request.surface_id, title });
            }
            X11LifecycleAction::Reconfigured { title, app_id, geometry } => {
                let Some((_, surface, mut existing_geometry, mut buffer, mut window, _, state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                let moved = existing_geometry.x != geometry.x || existing_geometry.y != geometry.y;
                let resizable = !matches!(*state, WindowState::Fullscreen | WindowState::Maximized);
                existing_geometry.x = geometry.x;
                existing_geometry.y = geometry.y;
                if resizable {
                    existing_geometry.width = geometry.width.max(1);
                    existing_geometry.height = geometry.height.max(1);
                }
                buffer.attached = true;
                window.title = title;
                window.app_id = app_id;
                if moved {
                    window_moved.write(WindowMoved {
                        surface_id: surface.id,
                        x: existing_geometry.x,
                        y: existing_geometry.y,
                    });
                }
            }
            X11LifecycleAction::Maximize => {
                let Some((_, _, mut geometry, _, _, _, mut state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = WindowState::Maximized;
                if let Some((width, height)) = output_geometry {
                    geometry.x = 0;
                    geometry.y = 0;
                    geometry.width = width;
                    geometry.height = height;
                }
            }
            X11LifecycleAction::UnMaximize => {
                let Some((_, _, _, _, _, x11_window, mut state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = restored_window_state(&config, x11_window.override_redirect);
            }
            X11LifecycleAction::Fullscreen => {
                let Some((_, _, mut geometry, _, _, _, mut state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = WindowState::Fullscreen;
                if let Some((width, height)) = output_geometry {
                    geometry.x = 0;
                    geometry.y = 0;
                    geometry.width = width;
                    geometry.height = height;
                }
            }
            X11LifecycleAction::UnFullscreen => {
                let Some((_, _, _, _, _, x11_window, mut state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = restored_window_state(&config, x11_window.override_redirect);
            }
            X11LifecycleAction::Minimize => {
                let Some((_, _, _, _, _, _, mut state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = WindowState::Hidden;
            }
            X11LifecycleAction::UnMinimize => {
                let Some((_, _, _, _, _, x11_window, mut state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = restored_window_state(&config, x11_window.override_redirect);
            }
            X11LifecycleAction::InteractiveMove { button: _button } => {
                let Some((_, surface, mut geometry, _, _, x11_window, mut state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = restored_window_state(&config, x11_window.override_redirect);
                if !x11_window.override_redirect {
                    *state = WindowState::Floating;
                }
                geometry.x = pointer.x.round() as i32 - 32;
                geometry.y = pointer.y.round() as i32 - 16;
                keyboard_focus.focused_surface = Some(surface.id);
                window_moved.write(WindowMoved {
                    surface_id: surface.id,
                    x: geometry.x,
                    y: geometry.y,
                });
            }
            X11LifecycleAction::InteractiveResize { button: _button, edges } => {
                let Some((_, surface, mut geometry, _, _, x11_window, mut state, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = restored_window_state(&config, x11_window.override_redirect);
                if !x11_window.override_redirect {
                    *state = WindowState::Floating;
                }
                apply_interactive_resize(&mut geometry, &edges);
                keyboard_focus.focused_surface = Some(surface.id);
            }
            X11LifecycleAction::Unmapped | X11LifecycleAction::Destroyed => {
                let Some((entity, _, _, _, _, _, _, _)) =
                    windows.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id)
                else {
                    continue;
                };

                commands.entity(entity).despawn();
                window_closed.write(WindowClosed { surface_id: request.surface_id });
            }
        }
    }

    pending_x11_requests.items = deferred;
}

fn restored_window_state(config: &CompositorConfig, override_redirect: bool) -> WindowState {
    if override_redirect {
        WindowState::Floating
    } else {
        match config.default_layout.as_str() {
            "floating" => WindowState::Floating,
            "maximized" => WindowState::Maximized,
            "fullscreen" => WindowState::Fullscreen,
            "tiling" | "stacking" => WindowState::Floating,
            _ => WindowState::Floating,
        }
    }
}

fn apply_interactive_resize(geometry: &mut SurfaceGeometry, edges: &str) {
    if edges.contains("Left") {
        geometry.x -= 32;
        geometry.width = geometry.width.saturating_add(32);
    }
    if edges.contains("Right") {
        geometry.width = geometry.width.saturating_add(64);
    }
    if edges.contains("Top") {
        geometry.y -= 24;
        geometry.height = geometry.height.saturating_add(24);
    }
    if edges.contains("Bottom") {
        geometry.height = geometry.height.saturating_add(48);
    }

    geometry.width = geometry.width.max(64);
    geometry.height = geometry.height.max(64);
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::Entity;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::X11WindowBundle;
    use nekoland_ecs::components::{
        BorderTheme, BufferState, LayoutSlot, ServerDecoration, SurfaceGeometry, WindowAnimation,
        WindowState, WlSurfaceHandle, X11Window, XdgWindow,
    };
    use nekoland_ecs::events::{WindowClosed, WindowCreated, WindowMoved};
    use nekoland_ecs::resources::{
        CompositorConfig, GlobalPointerPosition, KeyboardFocusState, PendingX11Requests,
        X11LifecycleAction, X11LifecycleRequest,
    };

    use super::xwayland_bridge_system;

    fn setup_app_with_window() -> (NekolandApp, Entity) {
        let mut app = NekolandApp::new("xwayland-bridge-test");
        app.insert_resource(CompositorConfig::default())
            .insert_resource(GlobalPointerPosition { x: 320.0, y: 180.0 })
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(PendingX11Requests::default());
        app.inner_mut()
            .add_message::<WindowCreated>()
            .add_message::<WindowClosed>()
            .add_message::<WindowMoved>()
            .add_systems(LayoutSchedule, xwayland_bridge_system);

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                X11WindowBundle {
                    surface: WlSurfaceHandle { id: 42 },
                    geometry: SurfaceGeometry { x: 32, y: 48, width: 640, height: 480 },
                    buffer: BufferState { attached: true, scale: 1 },
                    window: XdgWindow {
                        app_id: "x11-test".to_owned(),
                        title: "X11 Test".to_owned(),
                        last_acked_configure: None,
                    },
                    x11_window: X11Window { window_id: 7, override_redirect: false },
                    state: WindowState::Tiled,
                    decoration: ServerDecoration { enabled: true },
                    border_theme: BorderTheme::default(),
                    animation: WindowAnimation::default(),
                },
                LayoutSlot { workspace: 1, column: 0, row: 0 },
            ))
            .id();

        (app, entity)
    }

    #[test]
    fn x11_interactive_move_request_updates_geometry_and_focus() {
        let (mut app, entity) = setup_app_with_window();
        app.inner_mut().world_mut().resource_mut::<PendingX11Requests>().items.push(
            X11LifecycleRequest {
                surface_id: 42,
                action: X11LifecycleAction::InteractiveMove { button: 1 },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let geometry = world
            .get::<SurfaceGeometry>(entity)
            .expect("x11 window geometry should still exist after move");
        let state = world.get::<WindowState>(entity).expect("x11 window state should exist");
        let focus = world
            .get_resource::<KeyboardFocusState>()
            .expect("keyboard focus state should be initialized");

        assert_eq!((geometry.x, geometry.y), (288, 164));
        assert_eq!(*state, WindowState::Floating);
        assert_eq!(focus.focused_surface, Some(42));
    }

    #[test]
    fn x11_interactive_resize_request_updates_geometry_and_focus() {
        let (mut app, entity) = setup_app_with_window();
        app.inner_mut().world_mut().resource_mut::<PendingX11Requests>().items.push(
            X11LifecycleRequest {
                surface_id: 42,
                action: X11LifecycleAction::InteractiveResize {
                    button: 1,
                    edges: "BottomRight".to_owned(),
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let geometry = world
            .get::<SurfaceGeometry>(entity)
            .expect("x11 window geometry should still exist after resize");
        let state = world.get::<WindowState>(entity).expect("x11 window state should exist");
        let focus = world
            .get_resource::<KeyboardFocusState>()
            .expect("keyboard focus state should be initialized");

        assert_eq!((geometry.width, geometry.height), (704, 528));
        assert_eq!(*state, WindowState::Floating);
        assert_eq!(focus.focused_surface, Some(42));
    }
}

fn border_width(default_layout: &str) -> u32 {
    if matches!(default_layout, "tiling" | "stacking") { 2 } else { 1 }
}
