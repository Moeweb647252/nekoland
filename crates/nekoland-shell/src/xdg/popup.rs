use std::collections::{BTreeMap, BTreeSet, HashMap};

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, With, Without};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{
    BufferState, PopupGrab, SurfaceContentVersion, SurfaceGeometry, WindowAnimation,
    WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{
    EntityIndex, PendingXdgRequests, PopupPlacement, WaylandIngress, WindowLifecycleAction,
    XdgSurfaceRole,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PopupManager;

type PopupParentGeometries<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static WlSurfaceHandle, &'static SurfaceGeometry),
    (With<XdgWindow>, Without<XdgPopup>, Allow<Disabled>),
>;
type PopupManagementQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static WlSurfaceHandle,
        &'static mut SurfaceGeometry,
        &'static mut BufferState,
        &'static mut SurfaceContentVersion,
        &'static mut XdgPopup,
        &'static mut PopupGrab,
        &'static ChildOf,
    ),
    (Without<XdgWindow>, Allow<Disabled>),
>;
type PopupProjectionParents<'w, 's> =
    Query<
        'w,
        's,
        (Entity, &'static SurfaceGeometry),
        (With<XdgWindow>, Without<XdgPopup>, Allow<Disabled>),
    >;
type PopupProjectionQuery<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut SurfaceGeometry, &'static XdgPopup, &'static ChildOf),
    (With<XdgPopup>, Without<XdgWindow>, Allow<Disabled>),
>;

/// Owns popup lifecycle requests after they have been bridged out of protocol callbacks.
///
/// Popup creation is deferred until the parent surface can be resolved so the popup enters the
/// ECS hierarchy with the correct `ChildOf` relationship from the start.
pub fn popup_management_system(
    mut commands: Commands,
    wayland_ingress: Option<Res<WaylandIngress>>,
    mut pending_xdg_requests: Option<ResMut<PendingXdgRequests>>,
    entity_index: bevy_ecs::prelude::Res<EntityIndex>,
    parent_geometries: PopupParentGeometries<'_, '_>,
    mut popups: PopupManagementQuery<'_, '_>,
) {
    let mut known_popups = BTreeSet::new();
    let mut parent_entities_by_surface = HashMap::new();
    let mut parent_geometries_by_entity = BTreeMap::new();
    for (entity, surface, geometry) in parent_geometries.iter() {
        parent_entities_by_surface.insert(surface.id, entity);
        parent_geometries_by_entity.insert(entity, geometry.clone());
    }
    for (entity, surface, geometry, ..) in popups.iter_mut() {
        known_popups.insert(surface.id);
        parent_entities_by_surface.insert(surface.id, entity);
        parent_geometries_by_entity.insert(entity, (*geometry).clone());
    }
    let mut deferred = Vec::new();
    let mut requests =
        pending_xdg_requests.as_deref_mut().map(PendingXdgRequests::take).unwrap_or_default();
    if let Some(wayland_ingress) = wayland_ingress.as_deref() {
        requests.extend(wayland_ingress.pending_xdg_requests.iter().cloned());
    }

    for request in requests {
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
                let Some(parent_entity) = popup_parent_entity(
                    parent_surface_id,
                    &entity_index,
                    &parent_entities_by_surface,
                )
                else {
                    known_popups.remove(&request.surface_id);
                    deferred.push(request);
                    continue;
                };
                let Some(geometry) =
                    popup_geometry_for(parent_entity, placement, &parent_geometries_by_entity)
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
                    popup_geometry_for(child_of.parent(), placement, &parent_geometries_by_entity)
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
                    queue_popup_despawn(&mut commands, entity);
                    known_popups.remove(&request.surface_id);
                } else if known_popups.contains(&request.surface_id) {
                    deferred.push(request);
                }
            }
            _ => deferred.push(request),
        }
    }

    if let Some(mut pending_xdg_requests) = pending_xdg_requests {
        pending_xdg_requests.replace(deferred);
    }
    tracing::trace!(count = known_popups.len(), "xdg popup system tick");
}

pub fn popup_projection_system(
    parent_geometries: PopupProjectionParents<'_, '_>,
    mut popups: PopupProjectionQuery<'_, '_>,
) {
    let mut resolved_geometries = parent_geometries
        .iter()
        .map(|(entity, geometry)| (entity, geometry.clone()))
        .collect::<BTreeMap<_, _>>();
    let pending = popups
        .iter_mut()
        .map(|(entity, _geometry, popup, child_of)| {
            (
                entity,
                child_of.parent(),
                popup.placement_x,
                popup.placement_y,
                popup.placement_width,
                popup.placement_height,
            )
        })
        .collect::<Vec<_>>();
    let mut updates = BTreeMap::new();

    loop {
        let mut progressed = false;
        for (index, (entity, parent_entity, placement_x, placement_y, placement_width, placement_height)) in
            pending.iter().enumerate()
        {
            if updates.contains_key(&index) {
                continue;
            }
            let Some(parent_geometry) = resolved_geometries.get(parent_entity) else {
                continue;
            };

            let next = SurfaceGeometry {
                x: parent_geometry.x.saturating_add(*placement_x),
                y: parent_geometry.y.saturating_add(*placement_y),
                width: (*placement_width).max(1),
                height: (*placement_height).max(1),
            };
            resolved_geometries.insert(*entity, next.clone());
            updates.insert(index, next);
            progressed = true;
        }

        if !progressed || updates.len() == pending.len() {
            break;
        }
    }

    for (index, (_, mut geometry, _, _)) in popups.iter_mut().enumerate() {
        if let Some(next) = updates.get(&index) {
            *geometry = next.clone();
        }
    }
}

/// Converts popup-relative placement coordinates into global surface geometry using the resolved
/// parent surface rectangle.
fn popup_geometry_for(
    parent_entity: Entity,
    placement: PopupPlacement,
    parent_geometries_by_entity: &BTreeMap<Entity, SurfaceGeometry>,
) -> Option<SurfaceGeometry> {
    let parent_geometry = parent_geometries_by_entity.get(&parent_entity)?;

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
    parent_entities_by_surface: &HashMap<u64, Entity>,
) -> Option<Entity> {
    entity_index
        .entity_for_surface(parent_surface_id)
        .or_else(|| parent_entities_by_surface.get(&parent_surface_id).copied())
}

fn queue_popup_despawn(commands: &mut Commands, entity: Entity) {
    commands.queue(move |world: &mut bevy_ecs::world::World| {
        let mut to_despawn = Vec::new();
        collect_popup_descendants(world, entity, &mut to_despawn);
        to_despawn.push(entity);

        for entity in to_despawn {
            if let Ok(entity) = world.get_entity_mut(entity) {
                entity.despawn();
            }
        }
    });
}

fn collect_popup_descendants(
    world: &bevy_ecs::world::World,
    entity: Entity,
    to_despawn: &mut Vec<Entity>,
) {
    let Some(children) = world.get::<Children>(entity) else {
        return;
    };

    for child in children.iter() {
        collect_popup_descendants(world, *child, to_despawn);
        to_despawn.push(*child);
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::Entity;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::components::{
        BufferState, PopupGrab, SurfaceContentVersion, SurfaceGeometry, WlSurfaceHandle, XdgPopup,
        XdgWindow,
    };
    use nekoland_ecs::resources::{
        EntityIndex, PendingXdgRequests, PopupPlacement, WindowLifecycleAction,
        WindowLifecycleRequest, XdgSurfaceRole, rebuild_entity_index_system,
    };
    use nekoland_ecs::components::WindowAnimation;

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

    #[test]
    fn popup_creation_accepts_popup_parent_surfaces() {
        let mut app = NekolandApp::new("nested-popup-management-test");
        app.insert_resource(EntityIndex::default()).insert_resource(PendingXdgRequests::default());
        app.inner_mut().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, popup_management_system).chain(),
        );

        let window = app
            .inner_mut()
            .world_mut()
            .spawn((
                WlSurfaceHandle { id: 42 },
                SurfaceGeometry { x: 20, y: 30, width: 640, height: 480 },
                XdgWindow::default(),
            ))
            .id();
        let popup_parent = app
            .inner_mut()
            .world_mut()
            .spawn((
                WlSurfaceHandle { id: 100 },
                SurfaceGeometry { x: 120, y: 150, width: 240, height: 120 },
                XdgPopup::default(),
                ChildOf(window),
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 200,
                action: WindowLifecycleAction::PopupCreated {
                    parent_surface_id: Some(100),
                    placement: PopupPlacement {
                        x: 10,
                        y: 12,
                        width: 160,
                        height: 80,
                        reposition_token: Some(9),
                    },
                },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let child_entity = world
            .query::<(Entity, &WlSurfaceHandle)>()
            .iter(world)
            .find_map(|(entity, surface)| (surface.id == 200).then_some(entity))
            .unwrap_or_else(|| panic!("nested popup entity should be spawned"));
        let child_parent = world
            .get::<ChildOf>(child_entity)
            .unwrap_or_else(|| panic!("nested popup should have ChildOf"));

        assert_eq!(child_parent.parent(), popup_parent);
    }

    #[test]
    fn popup_destroy_is_idempotent_when_duplicate_requests_arrive() {
        let mut app = NekolandApp::new("popup-destroy-idempotent-test");
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
        app.inner_mut().world_mut().spawn((
            WlSurfaceHandle { id: 100 },
            SurfaceGeometry { x: 40, y: 50, width: 160, height: 90 },
            BufferState { attached: true, scale: 1 },
            SurfaceContentVersion::default(),
            XdgPopup::default(),
            PopupGrab::default(),
            WindowAnimation::default(),
            ChildOf(parent),
        ));
        let popup_parent = {
            let world = app.inner_mut().world_mut();
            world
                .query::<(Entity, &WlSurfaceHandle)>()
                .iter(world)
                .find_map(|(entity, surface)| (surface.id == 100).then_some(entity))
                .unwrap_or_else(|| panic!("popup parent should exist"))
        };
        app.inner_mut().world_mut().spawn((
            WlSurfaceHandle { id: 101 },
            SurfaceGeometry { x: 60, y: 70, width: 120, height: 60 },
            BufferState { attached: true, scale: 1 },
            SurfaceContentVersion::default(),
            XdgPopup::default(),
            PopupGrab::default(),
            WindowAnimation::default(),
            ChildOf(popup_parent),
        ));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 100,
                action: WindowLifecycleAction::Destroyed { role: XdgSurfaceRole::Popup },
            },
        );
        app.inner_mut().world_mut().resource_mut::<PendingXdgRequests>().push(
            WindowLifecycleRequest {
                surface_id: 100,
                action: WindowLifecycleAction::Destroyed { role: XdgSurfaceRole::Popup },
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let popup_count = world.query::<&XdgPopup>().iter(world).count();
        assert_eq!(popup_count, 0);
    }
}
