use std::collections::BTreeSet;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Commands, Entity, Query, ResMut, With, Without};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{
    BufferState, PopupGrab, SurfaceContentVersion, SurfaceGeometry, WindowAnimation,
    WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{
    EntityIndex, PendingXdgRequests, PopupPlacement, WindowLifecycleAction, XdgSurfaceRole,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PopupManager;

/// Owns popup lifecycle requests after they have been bridged out of protocol callbacks.
///
/// Popup creation is deferred until the parent toplevel can be resolved so the popup enters the
/// ECS hierarchy with the correct `ChildOf` relationship from the start.
pub fn popup_management_system(
    mut commands: Commands,
    mut pending_xdg_requests: ResMut<PendingXdgRequests>,
    entity_index: bevy_ecs::prelude::Res<EntityIndex>,
    parent_geometries: Query<
        (Entity, &WlSurfaceHandle, &SurfaceGeometry),
        (With<XdgWindow>, Without<XdgPopup>, Allow<Disabled>),
    >,
    mut popups: Query<
        (
            Entity,
            &WlSurfaceHandle,
            &mut SurfaceGeometry,
            &mut BufferState,
            &mut SurfaceContentVersion,
            &mut XdgPopup,
            &mut PopupGrab,
            &ChildOf,
        ),
        (Without<XdgWindow>, Allow<Disabled>),
    >,
) {
    let mut known_popups =
        popups.iter_mut().map(|(_, surface, _, _, _, _, _, _)| surface.id).collect::<BTreeSet<_>>();
    let mut deferred = Vec::new();

    for request in pending_xdg_requests.drain() {
        match request.action.clone() {
            WindowLifecycleAction::PopupCreated { parent_surface_id, placement }
                if known_popups.insert(request.surface_id) =>
            {
                let Some(parent_surface_id) = parent_surface_id else {
                    tracing::warn!(
                        surface_id = request.surface_id,
                        "dropping popup create request without parent surface id"
                    );
                    known_popups.remove(&request.surface_id);
                    continue;
                };
                let Some(parent_entity) =
                    popup_parent_entity(parent_surface_id, &entity_index, &parent_geometries)
                else {
                    known_popups.remove(&request.surface_id);
                    deferred.push(request);
                    continue;
                };
                let Some(geometry) =
                    popup_geometry_for(parent_entity, placement, &parent_geometries)
                else {
                    known_popups.remove(&request.surface_id);
                    deferred.push(request);
                    continue;
                };

                let mut popup_entity = commands.spawn((
                    WlSurfaceHandle { id: request.surface_id },
                    geometry,
                    BufferState { attached: false, scale: 1 },
                    XdgPopup {
                        configure_serial: None,
                        grab_serial: None,
                        placement_x: placement.x,
                        placement_y: placement.y,
                        placement_width: placement.width.max(1) as u32,
                        placement_height: placement.height.max(1) as u32,
                        reposition_token: placement.reposition_token,
                    },
                    PopupGrab::default(),
                    WindowAnimation::default(),
                ));
                popup_entity.insert(ChildOf(parent_entity));
            }
            WindowLifecycleAction::PopupRepositioned { placement } => {
                let mut popup = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| popups.get_mut(entity).ok());
                if popup.is_none() {
                    popup =
                        popups.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id);
                }
                let Some((_, _, mut geometry, _, _, mut popup, _, child_of)) = popup else {
                    deferred.push(request);
                    continue;
                };

                if let Some(next_geometry) =
                    popup_geometry_for(child_of.parent(), placement, &parent_geometries)
                {
                    *geometry = next_geometry;
                }
                popup.placement_x = placement.x;
                popup.placement_y = placement.y;
                popup.placement_width = placement.width.max(1) as u32;
                popup.placement_height = placement.height.max(1) as u32;
                popup.reposition_token = placement.reposition_token;
            }
            WindowLifecycleAction::Committed { role: XdgSurfaceRole::Popup, size } => {
                let mut popup = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| popups.get_mut(entity).ok());
                if popup.is_none() {
                    popup =
                        popups.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id);
                }
                let Some((_, _, mut geometry, mut buffer, mut content_version, _, _, _)) = popup
                else {
                    deferred.push(request);
                    continue;
                };

                if let Some(size) = size
                    && (geometry.width == 0 || geometry.height == 0)
                {
                    geometry.width = size.width.max(1);
                    geometry.height = size.height.max(1);
                }
                buffer.attached = size.is_some();
                content_version.bump();
            }
            WindowLifecycleAction::ConfigureRequested { role: XdgSurfaceRole::Popup } => {
                tracing::trace!(surface_id = request.surface_id, "popup configure requested");
            }
            WindowLifecycleAction::PopupGrab { seat_name, serial } => {
                let mut popup = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| popups.get_mut(entity).ok());
                if popup.is_none() {
                    popup =
                        popups.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id);
                }
                let Some((_, _, _, _, _, mut popup, mut grab, _)) = popup else {
                    deferred.push(request);
                    continue;
                };

                popup.grab_serial = Some(serial);
                grab.active = true;
                grab.seat_name = seat_name.clone();
                grab.serial = Some(serial);
            }
            WindowLifecycleAction::Destroyed { role: XdgSurfaceRole::Popup } => {
                let mut popup = entity_index
                    .entity_for_surface(request.surface_id)
                    .and_then(|entity| popups.get_mut(entity).ok());
                if popup.is_none() {
                    popup =
                        popups.iter_mut().find(|(_, surface, ..)| surface.id == request.surface_id);
                }

                if let Some((entity, _, _, _, _, _, _, _)) = popup {
                    commands.entity(entity).despawn();
                    known_popups.remove(&request.surface_id);
                } else if known_popups.contains(&request.surface_id) {
                    deferred.push(request);
                }
            }
            _ => deferred.push(request),
        }
    }

    pending_xdg_requests.replace(deferred);
    tracing::trace!(count = known_popups.len(), "xdg popup system tick");
}

pub fn popup_projection_system(
    parent_geometries: Query<
        &SurfaceGeometry,
        (With<XdgWindow>, Without<XdgPopup>, Allow<Disabled>),
    >,
    mut popups: Query<
        (&mut SurfaceGeometry, &XdgPopup, &ChildOf),
        (With<XdgPopup>, Without<XdgWindow>, Allow<Disabled>),
    >,
) {
    for (mut geometry, popup, child_of) in &mut popups {
        let Ok(parent_geometry) = parent_geometries.get(child_of.parent()) else {
            continue;
        };

        *geometry = SurfaceGeometry {
            x: parent_geometry.x.saturating_add(popup.placement_x),
            y: parent_geometry.y.saturating_add(popup.placement_y),
            width: popup.placement_width.max(1),
            height: popup.placement_height.max(1),
        };
    }
}

/// Converts popup-relative placement coordinates into global surface geometry using the resolved
/// parent toplevel rectangle.
fn popup_geometry_for(
    parent_entity: Entity,
    placement: PopupPlacement,
    parent_geometries: &Query<
        (Entity, &WlSurfaceHandle, &SurfaceGeometry),
        (With<XdgWindow>, Without<XdgPopup>, Allow<Disabled>),
    >,
) -> Option<SurfaceGeometry> {
    let (_, _, parent_geometry) = parent_geometries.get(parent_entity).ok()?;

    Some(SurfaceGeometry {
        x: parent_geometry.x + placement.x,
        y: parent_geometry.y + placement.y,
        width: placement.width.max(1) as u32,
        height: placement.height.max(1) as u32,
    })
}

/// Resolves the popup parent by surface id, preferring the entity index but falling back to a
/// query scan while startup bookkeeping is still catching up.
fn popup_parent_entity(
    parent_surface_id: u64,
    entity_index: &EntityIndex,
    parent_geometries: &Query<
        (Entity, &WlSurfaceHandle, &SurfaceGeometry),
        (With<XdgWindow>, Without<XdgPopup>, Allow<Disabled>),
    >,
) -> Option<Entity> {
    entity_index
        .entity_for_surface(parent_surface_id)
        .and_then(|entity| parent_geometries.get(entity).ok().map(|(entity, _, _)| entity))
        .or_else(|| {
            parent_geometries
                .iter()
                .find(|(_, surface, _)| surface.id == parent_surface_id)
                .map(|(entity, _, _)| entity)
        })
}

#[cfg(test)]
mod tests {
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::components::{SurfaceGeometry, WlSurfaceHandle, XdgWindow};
    use nekoland_ecs::resources::{
        EntityIndex, PendingXdgRequests, PopupPlacement, WindowLifecycleAction,
        WindowLifecycleRequest, rebuild_entity_index_system,
    };

    use super::{popup_management_system, popup_projection_system};

    #[test]
    fn popup_creation_inserts_child_of_parent_when_parent_exists() {
        let mut app = NekolandApp::new("popup-management-test");
        app.insert_resource(EntityIndex::default()).insert_resource(PendingXdgRequests::default());
        app.inner_mut().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, popup_management_system).chain(),
        );

        let parent = app
            .inner_mut()
            .world_mut()
            .spawn((
                WlSurfaceHandle { id: 42 },
                SurfaceGeometry { x: 20, y: 30, width: 640, height: 480 },
                XdgWindow::default(),
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 100,
                action: WindowLifecycleAction::PopupCreated {
                    parent_surface_id: Some(42),
                    placement: PopupPlacement {
                        x: 10,
                        y: 12,
                        width: 200,
                        height: 120,
                        reposition_token: Some(7),
                    },
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let popup_entity = world
            .query::<(bevy_ecs::entity::Entity, &WlSurfaceHandle)>()
            .iter(world)
            .find_map(|(entity, surface)| (surface.id == 100).then_some(entity))
            .unwrap_or_else(|| panic!("popup entity should be spawned"));
        let Some(popup_parent) = world.get::<ChildOf>(popup_entity) else {
            panic!("popup should have ChildOf relationship");
        };

        assert_eq!(popup_parent.parent(), parent);
    }

    #[test]
    fn popup_projection_tracks_parent_geometry_changes() {
        let mut app = NekolandApp::new("popup-projection-test");
        app.insert_resource(EntityIndex::default()).insert_resource(PendingXdgRequests::default());
        app.inner_mut().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, popup_management_system, popup_projection_system).chain(),
        );

        let parent = app
            .inner_mut()
            .world_mut()
            .spawn((
                WlSurfaceHandle { id: 42 },
                SurfaceGeometry { x: 20, y: 30, width: 640, height: 480 },
                XdgWindow::default(),
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 100,
                action: WindowLifecycleAction::PopupCreated {
                    parent_surface_id: Some(42),
                    placement: PopupPlacement {
                        x: 10,
                        y: 12,
                        width: 200,
                        height: 120,
                        reposition_token: Some(7),
                    },
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        {
            let Some(mut geometry) = app.inner_mut().world_mut().get_mut::<SurfaceGeometry>(parent)
            else {
                panic!("parent geometry");
            };
            geometry.x = 120;
            geometry.y = 230;
        }
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let popup_geometry = world
            .query::<(&WlSurfaceHandle, &SurfaceGeometry)>()
            .iter(world)
            .find(|(surface, _)| surface.id == 100)
            .map(|(_, geometry)| geometry.clone());
        let Some(popup_geometry) = popup_geometry else {
            panic!("popup geometry should exist");
        };
        assert_eq!(popup_geometry.x, 130);
        assert_eq!(popup_geometry.y, 242);
        assert_eq!(popup_geometry.width, 200);
        assert_eq!(popup_geometry.height, 120);
    }
}
