use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Entity, Query, Res, ResMut, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{WindowLayout, WindowMode, WindowRestoreState, XdgPopup, XdgWindow};
use nekoland_ecs::resources::{
    EntityIndex, GlobalPointerPosition, KeyboardFocusState, PendingXdgRequests,
    WindowLifecycleAction, XdgSurfaceRole,
};
use nekoland_ecs::views::{OutputRuntime, PopupConfigureRuntime, WindowRuntime};

use crate::interaction::{ActiveWindowGrab, WindowGrabMode, begin_window_grab};
use crate::window_policy::{lock_window_policy, restore_window_policy};

/// Sequences XDG configure-related requests after the toplevel entity exists.
///
/// This is where protocol acks, interactive move/resize grabs, and maximize/fullscreen state
/// transitions are projected into the ECS window model.
type XdgWindows<'w, 's> =
    Query<'w, 's, (Entity, WindowRuntime), (With<XdgWindow>, Allow<Disabled>)>;
type XdgPopups<'w, 's> =
    Query<'w, 's, (Entity, PopupConfigureRuntime), (With<XdgPopup>, Allow<Disabled>)>;

pub fn configure_sequence_system(
    mut pending_xdg_requests: ResMut<PendingXdgRequests>,
    entity_index: Res<EntityIndex>,
    pointer: Res<GlobalPointerPosition>,
    mut active_grab: ResMut<ActiveWindowGrab>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut windows: XdgWindows<'_, '_>,
    outputs: Query<OutputRuntime>,
    mut popups: XdgPopups<'_, '_>,
) {
    let mut deferred = Vec::new();
    let output_geometry = outputs
        .iter()
        .next()
        .map(|output| (output.properties.width.max(1), output.properties.height.max(1)));

    for request in pending_xdg_requests.drain() {
        match request.action.clone() {
            WindowLifecycleAction::ConfigureRequested { role: XdgSurfaceRole::Toplevel } => {
                tracing::trace!(surface_id = request.surface_id, "toplevel configure requested");
            }
            WindowLifecycleAction::AckConfigure { role: XdgSurfaceRole::Toplevel, serial } => {
                let Some(entity) =
                    resolve_xdg_window_entity(request.surface_id, &entity_index, &mut windows)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, window)) = windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                window
                    .xdg_window
                    .expect("xdg runtime should expose xdg metadata")
                    .last_acked_configure = Some(serial);
            }
            WindowLifecycleAction::AckConfigure { role: XdgSurfaceRole::Popup, serial } => {
                let Some(entity) =
                    resolve_xdg_popup_entity(request.surface_id, &entity_index, &mut popups)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut popup)) = popups.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                popup.popup.configure_serial = Some(serial);
            }
            WindowLifecycleAction::InteractiveMove { seat_name, serial } => {
                let Some(entity) =
                    resolve_xdg_window_entity(request.surface_id, &entity_index, &mut windows)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                *window.layout = WindowLayout::Floating;
                *window.mode = WindowMode::Normal;
                lock_window_policy(*window.layout, *window.mode, &mut window.policy_state);
                keyboard_focus.focused_surface = Some(window.surface_id());
                begin_window_grab(
                    &mut active_grab,
                    window.surface_id(),
                    WindowGrabMode::Move,
                    &pointer,
                    &window.scene_geometry,
                );
                tracing::trace!(
                    surface_id = window.surface_id(),
                    seat_name,
                    serial,
                    x = window.geometry.x,
                    y = window.geometry.y,
                    "started interactive move grab"
                );
            }
            WindowLifecycleAction::InteractiveResize { seat_name, serial, edges } => {
                let Some(entity) =
                    resolve_xdg_window_entity(request.surface_id, &entity_index, &mut windows)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                *window.layout = WindowLayout::Floating;
                *window.mode = WindowMode::Normal;
                lock_window_policy(*window.layout, *window.mode, &mut window.policy_state);
                keyboard_focus.focused_surface = Some(window.surface_id());
                begin_window_grab(
                    &mut active_grab,
                    window.surface_id(),
                    WindowGrabMode::Resize { edges: edges.clone() },
                    &pointer,
                    &window.scene_geometry,
                );
                tracing::trace!(
                    surface_id = window.surface_id(),
                    seat_name,
                    serial,
                    %edges,
                    width = window.geometry.width,
                    height = window.geometry.height,
                    "started interactive resize grab"
                );
            }
            WindowLifecycleAction::Maximize => {
                let Some(entity) =
                    resolve_xdg_window_entity(request.surface_id, &entity_index, &mut windows)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                window.restore.snapshot = Some(WindowRestoreState {
                    geometry: window.scene_geometry.clone(),
                    layout: (*window.layout).clone(),
                    mode: (*window.mode).clone(),
                });
                *window.mode = WindowMode::Maximized;
                keyboard_focus.focused_surface = Some(window.surface_id());
            }
            WindowLifecycleAction::UnMaximize => {
                let Some(entity) =
                    resolve_xdg_window_entity(request.surface_id, &entity_index, &mut windows)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                if let Some(restored) = window.restore.snapshot.take() {
                    *window.scene_geometry = restored.geometry;
                    *window.layout = restored.layout;
                    *window.mode = restored.mode;
                } else {
                    restore_window_policy(
                        &window.policy_state,
                        &mut window.layout,
                        &mut window.mode,
                    );
                }
            }
            WindowLifecycleAction::Fullscreen { output_name } => {
                let Some(entity) =
                    resolve_xdg_window_entity(request.surface_id, &entity_index, &mut windows)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                window.restore.snapshot = Some(WindowRestoreState {
                    geometry: window.scene_geometry.clone(),
                    layout: (*window.layout).clone(),
                    mode: (*window.mode).clone(),
                });
                *window.mode = WindowMode::Fullscreen;
                if let Some((width, height)) = output_geometry {
                    window.geometry.x = 0;
                    window.geometry.y = 0;
                    window.geometry.width = width;
                    window.geometry.height = height;
                }
                keyboard_focus.focused_surface = Some(window.surface_id());
                tracing::trace!(
                    surface_id = window.surface_id(),
                    output_name = ?output_name,
                    "applied fullscreen request"
                );
            }
            WindowLifecycleAction::UnFullscreen => {
                let Some(entity) =
                    resolve_xdg_window_entity(request.surface_id, &entity_index, &mut windows)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                if let Some(restored) = window.restore.snapshot.take() {
                    *window.scene_geometry = restored.geometry;
                    *window.layout = restored.layout;
                    *window.mode = restored.mode;
                } else {
                    restore_window_policy(
                        &window.policy_state,
                        &mut window.layout,
                        &mut window.mode,
                    );
                }
            }
            WindowLifecycleAction::Minimize => {
                let Some(entity) =
                    resolve_xdg_window_entity(request.surface_id, &entity_index, &mut windows)
                else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                *window.mode = WindowMode::Hidden;
                if keyboard_focus.focused_surface == Some(window.surface_id()) {
                    keyboard_focus.focused_surface = None;
                }
            }
            _ => deferred.push(request),
        }
    }

    pending_xdg_requests.replace(deferred);
    tracing::trace!("xdg configure sequencing system tick");
}

fn resolve_xdg_window_entity(
    surface_id: u64,
    entity_index: &EntityIndex,
    windows: &mut XdgWindows<'_, '_>,
) -> Option<Entity> {
    entity_index.entity_for_surface(surface_id).or_else(|| {
        windows
            .iter_mut()
            .find(|(_, window)| window.surface_id() == surface_id)
            .map(|(entity, _)| entity)
    })
}

fn resolve_xdg_popup_entity(
    surface_id: u64,
    entity_index: &EntityIndex,
    popups: &mut XdgPopups<'_, '_>,
) -> Option<Entity> {
    entity_index.entity_for_surface(surface_id).or_else(|| {
        popups
            .iter_mut()
            .find(|(_, popup)| popup.surface_id() == surface_id)
            .map(|(entity, _)| entity)
    })
}
