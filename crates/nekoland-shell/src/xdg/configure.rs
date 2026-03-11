use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Query, Res, ResMut, With};
use nekoland_ecs::components::{
    OutputProperties, PopupGrab, SurfaceGeometry, WindowState, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::events::WindowMoved;
use nekoland_ecs::resources::{
    GlobalPointerPosition, KeyboardFocusState, PendingWindowServerRequests, PendingXdgRequests,
    WindowLifecycleAction, WindowServerAction, XdgSurfaceRole,
};

pub fn window_geometry_request_system(
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
    mut windows: Query<(&WlSurfaceHandle, &mut SurfaceGeometry, &mut WindowState), With<XdgWindow>>,
    mut window_moved: MessageWriter<WindowMoved>,
) {
    let mut deferred = Vec::new();

    for request in pending_window_requests.items.drain(..) {
        match request.action {
            WindowServerAction::Move { x, y } => {
                let Some((surface, mut geometry, mut state)) =
                    windows.iter_mut().find(|(surface, _, _)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = WindowState::Floating;
                geometry.x = x;
                geometry.y = y;
                window_moved.write(WindowMoved { surface_id: surface.id, x, y });
            }
            WindowServerAction::Resize { width, height } => {
                let Some((_, mut geometry, mut state)) =
                    windows.iter_mut().find(|(surface, _, _)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                *state = WindowState::Floating;
                geometry.width = width.max(64);
                geometry.height = height.max(64);
            }
            _ => deferred.push(request),
        }
    }

    pending_window_requests.items = deferred;
}

pub fn configure_sequence_system(
    mut pending_xdg_requests: ResMut<PendingXdgRequests>,
    pointer: Res<GlobalPointerPosition>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut windows: Query<
        (&WlSurfaceHandle, &mut XdgWindow, &mut SurfaceGeometry, &mut WindowState),
        With<XdgWindow>,
    >,
    outputs: Query<&OutputProperties>,
    mut popups: Query<(&WlSurfaceHandle, &mut XdgPopup, Option<&mut PopupGrab>)>,
    mut window_moved: MessageWriter<WindowMoved>,
) {
    let mut deferred = Vec::new();
    let output_geometry =
        outputs.iter().next().map(|properties| (properties.width.max(1), properties.height.max(1)));

    for request in pending_xdg_requests.items.drain(..) {
        match request.action.clone() {
            WindowLifecycleAction::ConfigureRequested { role: XdgSurfaceRole::Toplevel } => {
                tracing::trace!(surface_id = request.surface_id, "toplevel configure requested");
            }
            WindowLifecycleAction::AckConfigure { role: XdgSurfaceRole::Toplevel, serial } => {
                let mut handled = false;
                for (surface, mut window, _, _) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    window.last_acked_configure = Some(serial);
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::AckConfigure { role: XdgSurfaceRole::Popup, serial } => {
                let mut handled = false;
                for (surface, mut popup, _) in &mut popups {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    popup.configure_serial = Some(serial);
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::InteractiveMove { seat_name, serial } => {
                let mut handled = false;
                for (surface, _, mut geometry, mut state) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    *state = WindowState::Floating;
                    geometry.x = pointer.x.round() as i32 - 32;
                    geometry.y = pointer.y.round() as i32 - 16;
                    keyboard_focus.focused_surface = Some(surface.id);
                    window_moved.write(WindowMoved {
                        surface_id: surface.id,
                        x: geometry.x,
                        y: geometry.y,
                    });
                    tracing::trace!(
                        surface_id = surface.id,
                        seat_name,
                        serial,
                        x = geometry.x,
                        y = geometry.y,
                        "applied interactive move request"
                    );
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::InteractiveResize { seat_name, serial, edges } => {
                let mut handled = false;
                for (surface, _, mut geometry, mut state) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    *state = WindowState::Floating;
                    apply_interactive_resize(&mut geometry, &edges);
                    tracing::trace!(
                        surface_id = surface.id,
                        seat_name,
                        serial,
                        edges,
                        width = geometry.width,
                        height = geometry.height,
                        "applied interactive resize request"
                    );
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::Maximize => {
                let mut handled = false;
                for (surface, _, _, mut state) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    *state = WindowState::Maximized;
                    keyboard_focus.focused_surface = Some(surface.id);
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::UnMaximize => {
                let mut handled = false;
                for (surface, _, _, mut state) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    *state = WindowState::Tiled;
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::Fullscreen { output_name } => {
                let mut handled = false;
                for (surface, _, mut geometry, mut state) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    *state = WindowState::Fullscreen;
                    if let Some((width, height)) = output_geometry {
                        geometry.x = 0;
                        geometry.y = 0;
                        geometry.width = width;
                        geometry.height = height;
                    }
                    keyboard_focus.focused_surface = Some(surface.id);
                    tracing::trace!(
                        surface_id = surface.id,
                        output_name = ?output_name,
                        "applied fullscreen request"
                    );
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::UnFullscreen => {
                let mut handled = false;
                for (surface, _, _, mut state) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    *state = WindowState::Tiled;
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::Minimize => {
                let mut handled = false;
                for (surface, _, _, mut state) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    *state = WindowState::Hidden;
                    if keyboard_focus.focused_surface == Some(surface.id) {
                        keyboard_focus.focused_surface = None;
                    }
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            _ => deferred.push(request),
        }
    }

    pending_xdg_requests.items = deferred;
    tracing::trace!("xdg configure sequencing system tick");
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
