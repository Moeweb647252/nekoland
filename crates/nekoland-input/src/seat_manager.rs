use bevy_ecs::prelude::ResMut;

/// Ensures the shared seat registry always exposes a primary seat entry.
pub fn seat_management_system(mut seat_registry: ResMut<nekoland_ecs::resources::SeatRegistry>) {
    seat_registry.ensure_wayland_name(nekoland_ecs::resources::DEFAULT_WAYLAND_SEAT_NAME);

    tracing::trace!(
        primary_seat_id = seat_registry.primary_seat_id().0,
        "seat management system tick"
    );
}
