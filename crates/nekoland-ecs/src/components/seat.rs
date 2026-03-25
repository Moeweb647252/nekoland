//! Seat identity and seat-facing components.
#![allow(missing_docs)]

use std::sync::atomic::{AtomicU64, Ordering};

use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

static NEXT_SEAT_ID: AtomicU64 = AtomicU64::new(2);

/// Runtime-stable identity for one logical input seat.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct SeatId(pub u64);

impl SeatId {
    pub const PRIMARY: Self = Self(1);

    pub fn fresh() -> Self {
        Self(NEXT_SEAT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for SeatId {
    fn default() -> Self {
        Self::PRIMARY
    }
}

/// Named logical input seat entity.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputSeat {
    pub id: SeatId,
    pub name: String,
}

/// Pointer position component for seat entities or related input state entities.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PointerPosition {
    pub x: f64,
    pub y: f64,
}

/// Focus target associated with a seat.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardFocus {
    pub surface_id: Option<u64>,
}
