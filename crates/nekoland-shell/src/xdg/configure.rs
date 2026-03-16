use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Entity, Query, Res, ResMut, With};
use bevy_ecs::query::Allow;
use bevy_ecs::system::SystemParam;
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

#[derive(SystemParam)]
pub struct ConfigureSequenceParams<'w, 's> {
    pending_xdg_requests: ResMut<'w, PendingXdgRequests>,
    entity_index: Res<'w, EntityIndex>,
    pointer: Res<'w, GlobalPointerPosition>,
    active_grab: ResMut<'w, ActiveWindowGrab>,
    keyboard_focus: ResMut<'w, KeyboardFocusState>,
    windows: XdgWindows<'w, 's>,
    outputs: Query<'w, 's, OutputRuntime>,
    popups: XdgPopups<'w, 's>,
}

pub fn configure_sequence_system(mut configure: ConfigureSequenceParams<'_, '_>) {
    let mut deferred = Vec::new();
    let output_geometry = configure
        .outputs
        .iter()
        .next()
        .map(|output| (output.properties.width.max(1), output.properties.height.max(1)));

    for request in configure.pending_xdg_requests.drain() {
        match request.action.clone() {
            WindowLifecycleAction::ConfigureRequested { role: XdgSurfaceRole::Toplevel } => {
                tracing::trace!(surface_id = request.surface_id, "toplevel configure requested");
            }
            WindowLifecycleAction::AckConfigure { role: XdgSurfaceRole::Toplevel, serial } => {
                let Some(entity) = resolve_xdg_window_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.windows,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = configure.windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                let Some(xdg_window) = window.xdg_window.as_mut() else {
                    tracing::warn!(
                        surface_id = request.surface_id,
                        "skipping ack_configure for xdg window without xdg metadata"
                    );
                    continue;
                };
                xdg_window.last_acked_configure = Some(serial);
            }
            WindowLifecycleAction::AckConfigure { role: XdgSurfaceRole::Popup, serial } => {
                let Some(entity) = resolve_xdg_popup_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.popups,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut popup)) = configure.popups.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                popup.popup.configure_serial = Some(serial);
            }
            WindowLifecycleAction::InteractiveMove { seat_name, serial } => {
                let Some(entity) = resolve_xdg_window_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.windows,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = configure.windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };
                if window.role.is_output_background() {
                    continue;
                }

                *window.layout = WindowLayout::Floating;
                *window.mode = WindowMode::Normal;
                lock_window_policy(*window.layout, *window.mode, &mut window.policy_state);
                configure.keyboard_focus.focused_surface = Some(window.surface_id());
                begin_window_grab(
                    &mut configure.active_grab,
                    window.surface_id(),
                    WindowGrabMode::Move,
                    &configure.pointer,
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
                let Some(entity) = resolve_xdg_window_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.windows,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = configure.windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };
                if window.role.is_output_background() {
                    continue;
                }

                *window.layout = WindowLayout::Floating;
                *window.mode = WindowMode::Normal;
                lock_window_policy(*window.layout, *window.mode, &mut window.policy_state);
                configure.keyboard_focus.focused_surface = Some(window.surface_id());
                begin_window_grab(
                    &mut configure.active_grab,
                    window.surface_id(),
                    WindowGrabMode::Resize { edges },
                    &configure.pointer,
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
                let Some(entity) = resolve_xdg_window_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.windows,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = configure.windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                window.restore.snapshot = Some(WindowRestoreState {
                    geometry: window.scene_geometry.clone(),
                    layout: *window.layout,
                    mode: *window.mode,
                });
                *window.mode = WindowMode::Maximized;
                configure.keyboard_focus.focused_surface = Some(window.surface_id());
            }
            WindowLifecycleAction::UnMaximize => {
                let Some(entity) = resolve_xdg_window_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.windows,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = configure.windows.get_mut(entity) else {
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
                let Some(entity) = resolve_xdg_window_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.windows,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = configure.windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                window.restore.snapshot = Some(WindowRestoreState {
                    geometry: window.scene_geometry.clone(),
                    layout: *window.layout,
                    mode: *window.mode,
                });
                *window.mode = WindowMode::Fullscreen;
                if let Some((width, height)) = output_geometry {
                    window.geometry.x = 0;
                    window.geometry.y = 0;
                    window.geometry.width = width;
                    window.geometry.height = height;
                }
                configure.keyboard_focus.focused_surface = Some(window.surface_id());
                tracing::trace!(
                    surface_id = window.surface_id(),
                    output_name = ?output_name,
                    "applied fullscreen request"
                );
            }
            WindowLifecycleAction::UnFullscreen => {
                let Some(entity) = resolve_xdg_window_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.windows,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = configure.windows.get_mut(entity) else {
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
                let Some(entity) = resolve_xdg_window_entity(
                    request.surface_id,
                    &configure.entity_index,
                    &mut configure.windows,
                ) else {
                    deferred.push(request);
                    continue;
                };
                let Ok((_, mut window)) = configure.windows.get_mut(entity) else {
                    deferred.push(request);
                    continue;
                };

                *window.mode = WindowMode::Hidden;
                if configure.keyboard_focus.focused_surface == Some(window.surface_id()) {
                    configure.keyboard_focus.focused_surface = None;
                }
            }
            _ => deferred.push(request),
        }
    }

    configure.pending_xdg_requests.replace(deferred);
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
