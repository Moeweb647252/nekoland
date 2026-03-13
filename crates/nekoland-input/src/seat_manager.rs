use bevy_ecs::prelude::{ResMut, Resource};

/// Minimal seat registry used by the current input pipeline.
///
/// The model is intentionally lightweight for now: one or more seat names are enough to keep the
/// rest of the compositor plumbing alive until richer seat state is introduced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct SeatManager {
    pub seats: Vec<String>,
}

/// Ensures there is always at least one default seat available for subsystems that assume a seat
/// exists even before real backend device discovery becomes richer.
pub fn seat_management_system(mut seat_manager: ResMut<SeatManager>) {
    if seat_manager.seats.is_empty() {
        seat_manager.seats.push("seat0".to_owned());
    }

    tracing::trace!(seats = seat_manager.seats.len(), "seat management system tick");
}
