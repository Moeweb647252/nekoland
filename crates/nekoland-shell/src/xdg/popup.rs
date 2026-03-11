use std::collections::BTreeSet;

use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Commands, Entity, Query, ResMut, With, Without};
use nekoland_ecs::components::{
    BufferState, PopupGrab, SurfaceGeometry, WindowAnimation, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::events::WindowClosed;
use nekoland_ecs::resources::{
    PendingPopupServerRequests, PendingXdgRequests, PopupPlacement, PopupServerAction,
    PopupServerRequest, WindowLifecycleAction, XdgSurfaceRole,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PopupManager;

pub fn popup_parent_close_system(
    mut window_closed: MessageReader<WindowClosed>,
    popups: Query<(&WlSurfaceHandle, &XdgPopup)>,
    mut pending_popup_requests: ResMut<PendingPopupServerRequests>,
) {
    let closed_surfaces =
        window_closed.read().map(|event| event.surface_id).collect::<BTreeSet<_>>();
    if closed_surfaces.is_empty() {
        return;
    }

    let mut dismissed = BTreeSet::new();
    for (surface, popup) in popups.iter() {
        if !closed_surfaces.contains(&popup.parent_surface) || !dismissed.insert(surface.id) {
            continue;
        }

        pending_popup_requests.items.push(PopupServerRequest {
            surface_id: surface.id,
            action: PopupServerAction::Dismiss,
        });
    }
}

pub fn popup_management_system(
    mut commands: Commands,
    mut pending_xdg_requests: ResMut<PendingXdgRequests>,
    parent_geometries: Query<
        (&WlSurfaceHandle, &SurfaceGeometry),
        (With<XdgWindow>, Without<XdgPopup>),
    >,
    mut popups: Query<
        (
            Entity,
            &WlSurfaceHandle,
            &mut SurfaceGeometry,
            &mut BufferState,
            &mut XdgPopup,
            &mut PopupGrab,
        ),
        Without<XdgWindow>,
    >,
) {
    let mut known_popups =
        popups.iter_mut().map(|(_, surface, _, _, _, _)| surface.id).collect::<BTreeSet<_>>();
    let mut deferred = Vec::new();

    for request in pending_xdg_requests.items.drain(..) {
        match request.action.clone() {
            WindowLifecycleAction::PopupCreated { parent_surface_id, placement }
                if known_popups.insert(request.surface_id) =>
            {
                let geometry = popup_geometry_for(parent_surface_id, placement, &parent_geometries)
                    .unwrap_or(SurfaceGeometry {
                        x: 96,
                        y: 96,
                        width: placement.width.max(1) as u32,
                        height: placement.height.max(1) as u32,
                    });

                commands.spawn((
                    WlSurfaceHandle { id: request.surface_id },
                    geometry,
                    BufferState { attached: false, scale: 1 },
                    XdgPopup {
                        parent_surface: parent_surface_id.unwrap_or_default(),
                        configure_serial: None,
                        grab_serial: None,
                        reposition_token: placement.reposition_token,
                    },
                    PopupGrab::default(),
                    WindowAnimation::default(),
                ));
            }
            WindowLifecycleAction::PopupRepositioned { placement } => {
                let mut handled = false;

                for (_, surface, mut geometry, _, mut popup, _) in &mut popups {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    if let Some(next_geometry) = popup_geometry_for(
                        Some(popup.parent_surface),
                        placement,
                        &parent_geometries,
                    ) {
                        *geometry = next_geometry;
                    }
                    popup.reposition_token = placement.reposition_token;
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::Committed { role: XdgSurfaceRole::Popup, size } => {
                let mut handled = false;

                for (_, surface, mut geometry, mut buffer, _, _) in &mut popups {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    if let Some(size) = size {
                        geometry.width = size.width.max(1);
                        geometry.height = size.height.max(1);
                    }
                    buffer.attached = size.is_some();
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::ConfigureRequested { role: XdgSurfaceRole::Popup } => {
                tracing::trace!(surface_id = request.surface_id, "popup configure requested");
            }
            WindowLifecycleAction::PopupGrab { seat_name, serial } => {
                let mut handled = false;

                for (_, surface, _, _, mut popup, mut grab) in &mut popups {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    popup.grab_serial = Some(serial);
                    grab.active = true;
                    grab.seat_name = seat_name.clone();
                    grab.serial = Some(serial);
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::Destroyed { role: XdgSurfaceRole::Popup } => {
                let mut handled = false;

                for (entity, surface, _, _, _, _) in &mut popups {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    commands.entity(entity).despawn();
                    known_popups.remove(&request.surface_id);
                    handled = true;
                    break;
                }

                if !handled && known_popups.contains(&request.surface_id) {
                    deferred.push(request);
                }
            }
            _ => deferred.push(request),
        }
    }

    pending_xdg_requests.items = deferred;
    tracing::trace!(count = known_popups.len(), "xdg popup system tick");
}

fn popup_geometry_for(
    parent_surface_id: Option<u64>,
    placement: PopupPlacement,
    parent_geometries: &Query<
        (&WlSurfaceHandle, &SurfaceGeometry),
        (With<XdgWindow>, Without<XdgPopup>),
    >,
) -> Option<SurfaceGeometry> {
    let parent_geometry = parent_surface_id.and_then(|parent_surface| {
        parent_geometries
            .iter()
            .find(|(surface, _)| surface.id == parent_surface)
            .map(|(_, geometry)| geometry.clone())
    })?;

    Some(SurfaceGeometry {
        x: parent_geometry.x + placement.x,
        y: parent_geometry.y + placement.y,
        width: placement.width.max(1) as u32,
        height: placement.height.max(1) as u32,
    })
}
