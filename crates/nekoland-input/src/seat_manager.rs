use bevy_ecs::prelude::{ResMut, Resource};

#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct SeatManager {
    pub seats: Vec<String>,
}

pub fn seat_management_system(mut seat_manager: ResMut<SeatManager>) {
    if seat_manager.seats.is_empty() {
        seat_manager.seats.push("seat0".to_owned());
    }

    tracing::trace!(seats = seat_manager.seats.len(), "seat management system tick");
}
