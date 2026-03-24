use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Entity, Query, Res, ResMut, With};
use bevy_ecs::query::Allow;
use bevy_ecs::system::SystemParam;
use nekoland_ecs::components::{WindowLayout, WindowMode, XdgPopup, XdgWindow};
use nekoland_ecs::resources::{
    EntityIndex, GlobalPointerPosition, KeyboardFocusState, WaylandIngress, WindowLifecycleAction,
    XdgSurfaceRole,
};
use nekoland_ecs::selectors::OutputName;
use nekoland_ecs::views::{PopupConfigureRuntime, WindowRuntime};

use crate::interaction::{ActiveWindowGrab, WindowGrabMode, begin_window_grab};
use crate::window_policy::{enter_temporary_window_mode, lock_window_policy, restore_window_state};

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
    pending_xdg_requests: ResMut<'w, crate::xdg::DeferredXdgRequests>,
    wayland_ingress: Res<'w, WaylandIngress>,
    entity_index: Res<'w, EntityIndex>,
    pointer: Res<'w, GlobalPointerPosition>,
    active_grab: ResMut<'w, ActiveWindowGrab>,
    keyboard_focus: ResMut<'w, KeyboardFocusState>,
    windows: XdgWindows<'w, 's>,
    popups: XdgPopups<'w, 's>,
}

pub(crate) fn configure_sequence_system(mut configure: ConfigureSequenceParams<'_, '_>) {
    let mut deferred = Vec::new();
    let mut requests = configure.pending_xdg_requests.take();
    requests.extend(configure.wayland_ingress.pending_xdg_requests.iter().cloned());

    for request in requests {
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
            WindowLifecycleAction::InteractiveMove { seat_id, serial } => {
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
                    seat_id = seat_id.0,
                    serial,
                    x = window.geometry.x,
                    y = window.geometry.y,
                    "started interactive move grab"
                );
            }
            WindowLifecycleAction::InteractiveResize { seat_id, serial, edges } => {
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
                    seat_id = seat_id.0,
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

                enter_temporary_window_mode(
                    &window.scene_geometry,
                    &mut window.fullscreen_target,
                    &mut window.restore,
                    *window.layout,
                    &mut window.mode,
                    WindowMode::Maximized,
                    None,
                );
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

                restore_window_state(
                    &window.policy_state,
                    &mut window.scene_geometry,
                    &mut window.fullscreen_target,
                    &mut window.restore,
                    &mut window.layout,
                    &mut window.mode,
                );
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

                enter_temporary_window_mode(
                    &window.scene_geometry,
                    &mut window.fullscreen_target,
                    &mut window.restore,
                    *window.layout,
                    &mut window.mode,
                    WindowMode::Fullscreen,
                    output_name.as_ref().map(|output_name| OutputName::from(output_name.as_str())),
                );
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

                restore_window_state(
                    &window.policy_state,
                    &mut window.scene_geometry,
                    &mut window.fullscreen_target,
                    &mut window.restore,
                    &mut window.layout,
                    &mut window.mode,
                );
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

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        BufferState, SurfaceGeometry, WindowAnimation, WindowFullscreenTarget, WindowLayout,
        WindowMode, WindowRestoreSnapshot, WindowSceneGeometry, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::resources::{
        EntityIndex, GlobalPointerPosition, KeyboardFocusState, WaylandIngress,
        WindowLifecycleAction, WindowLifecycleRequest,
    };

    use crate::interaction::ActiveWindowGrab;
    use crate::xdg::DeferredXdgRequests;

    use super::configure_sequence_system;

    fn setup_app_with_window() -> (NekolandApp, bevy_ecs::entity::Entity) {
        let mut app = NekolandApp::new("xdg-configure-test");
        app.insert_resource(EntityIndex::default())
            .insert_resource(DeferredXdgRequests::default())
            .insert_resource(GlobalPointerPosition::default())
            .insert_resource(ActiveWindowGrab::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_systems(LayoutSchedule, configure_sequence_system);

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 42 },
                geometry: SurfaceGeometry { x: 12, y: 24, width: 640, height: 480 },
                scene_geometry: WindowSceneGeometry { x: 12, y: 24, width: 640, height: 480 },
                viewport_visibility: Default::default(),
                buffer: BufferState { attached: true, scale: 1 },
                content_version: Default::default(),
                window: XdgWindow::default(),
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                decoration: Default::default(),
                border_theme: Default::default(),
                animation: WindowAnimation::default(),
            })
            .id();

        (app, entity)
    }

    #[test]
    fn repeated_maximize_restores_original_state() {
        let (mut app, entity) = setup_app_with_window();
        let mut requests = app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>();
        requests.push(WindowLifecycleRequest {
            surface_id: 42,
            action: WindowLifecycleAction::Maximize,
        });
        requests.push(WindowLifecycleRequest {
            surface_id: 42,
            action: WindowLifecycleAction::Maximize,
        });
        requests.push(WindowLifecycleRequest {
            surface_id: 42,
            action: WindowLifecycleAction::UnMaximize,
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        assert_eq!(
            world.get::<WindowSceneGeometry>(entity),
            Some(&WindowSceneGeometry { x: 12, y: 24, width: 640, height: 480 })
        );
        assert_eq!(world.get::<WindowLayout>(entity), Some(&WindowLayout::Floating));
        assert_eq!(world.get::<WindowMode>(entity), Some(&WindowMode::Normal));
        assert_eq!(
            world.get::<WindowFullscreenTarget>(entity),
            Some(&WindowFullscreenTarget::default())
        );
        assert_eq!(
            world.get::<WindowRestoreSnapshot>(entity),
            Some(&WindowRestoreSnapshot::default())
        );
    }

    #[test]
    fn fullscreen_restore_returns_to_previous_temporary_state() {
        let (mut app, entity) = setup_app_with_window();
        {
            let mut requests = app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>();
            requests.push(WindowLifecycleRequest {
                surface_id: 42,
                action: WindowLifecycleAction::Maximize,
            });
            requests.push(WindowLifecycleRequest {
                surface_id: 42,
                action: WindowLifecycleAction::Fullscreen {
                    output_name: Some("HDMI-A-1".to_owned()),
                },
            });
            requests.push(WindowLifecycleRequest {
                surface_id: 42,
                action: WindowLifecycleAction::UnFullscreen,
            });
        }

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        assert_eq!(world.get::<WindowMode>(entity), Some(&WindowMode::Maximized));
        assert_eq!(
            world
                .get::<WindowRestoreSnapshot>(entity)
                .and_then(|restore| restore.snapshot.as_ref()),
            Some(&nekoland_ecs::components::WindowRestoreState {
                geometry: WindowSceneGeometry { x: 12, y: 24, width: 640, height: 480 },
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                fullscreen_output: None,
                previous: None,
            })
        );
        assert_eq!(
            world.get::<WindowFullscreenTarget>(entity),
            Some(&WindowFullscreenTarget::default())
        );

        app.inner_mut().world_mut().resource_mut::<DeferredXdgRequests>().push(
            WindowLifecycleRequest { surface_id: 42, action: WindowLifecycleAction::UnMaximize },
        );
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        assert_eq!(world.get::<WindowMode>(entity), Some(&WindowMode::Normal));
        assert_eq!(
            world.get::<WindowSceneGeometry>(entity),
            Some(&WindowSceneGeometry { x: 12, y: 24, width: 640, height: 480 })
        );
        assert_eq!(
            world.get::<WindowRestoreSnapshot>(entity),
            Some(&WindowRestoreSnapshot::default())
        );
    }
}
