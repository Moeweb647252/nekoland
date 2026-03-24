use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::SeatId;

pub const DEFAULT_WAYLAND_SEAT_NAME: &str = "seat-0";

/// Human-readable names associated with one logical seat.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeatMetadata {
    pub wayland_name: Option<String>,
    pub backend_name: Option<String>,
}

impl SeatMetadata {
    pub fn display_name(&self) -> Option<&str> {
        self.wayland_name.as_deref().or(self.backend_name.as_deref())
    }
}

/// Registry that resolves stable seat ids to protocol/backend-facing seat names.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeatRegistry {
    primary_seat_id: SeatId,
    seats: BTreeMap<SeatId, SeatMetadata>,
    wayland_names: BTreeMap<String, SeatId>,
    backend_names: BTreeMap<String, SeatId>,
}

impl Default for SeatRegistry {
    fn default() -> Self {
        let mut registry = Self {
            primary_seat_id: SeatId::PRIMARY,
            seats: BTreeMap::new(),
            wayland_names: BTreeMap::new(),
            backend_names: BTreeMap::new(),
        };
        registry.bind_wayland_name(SeatId::PRIMARY, DEFAULT_WAYLAND_SEAT_NAME);
        registry
    }
}

impl SeatRegistry {
    pub fn primary_seat_id(&self) -> SeatId {
        self.primary_seat_id
    }

    pub fn seat(&self, seat_id: SeatId) -> Option<&SeatMetadata> {
        self.seats.get(&seat_id)
    }

    pub fn seat_name(&self, seat_id: SeatId) -> Option<&str> {
        self.seat(seat_id).and_then(SeatMetadata::display_name)
    }

    pub fn wayland_name(&self, seat_id: SeatId) -> Option<&str> {
        self.seat(seat_id).and_then(|seat| seat.wayland_name.as_deref())
    }

    pub fn backend_name(&self, seat_id: SeatId) -> Option<&str> {
        self.seat(seat_id).and_then(|seat| seat.backend_name.as_deref())
    }

    pub fn seat_id_for_wayland_name(&self, wayland_name: &str) -> Option<SeatId> {
        self.wayland_names.get(wayland_name).copied()
    }

    pub fn seat_id_for_backend_name(&self, backend_name: &str) -> Option<SeatId> {
        self.backend_names.get(backend_name).copied()
    }

    pub fn ensure_wayland_name(&mut self, wayland_name: impl AsRef<str>) -> SeatId {
        let wayland_name = wayland_name.as_ref();
        if let Some(seat_id) = self.seat_id_for_wayland_name(wayland_name) {
            return seat_id;
        }

        let seat_id = if self.seats.is_empty() { self.primary_seat_id } else { SeatId::fresh() };
        self.bind_wayland_name(seat_id, wayland_name);
        seat_id
    }

    pub fn bind_backend_name(&mut self, seat_id: SeatId, backend_name: impl Into<String>) {
        let backend_name = backend_name.into();
        if let Some(previous) = self
            .seats
            .entry(seat_id)
            .or_default()
            .backend_name
            .replace(backend_name.clone())
        {
            self.backend_names.remove(&previous);
        }
        self.backend_names.insert(backend_name, seat_id);
    }

    pub fn bind_wayland_name(&mut self, seat_id: SeatId, wayland_name: impl Into<String>) {
        let wayland_name = wayland_name.into();
        if let Some(previous) = self
            .seats
            .entry(seat_id)
            .or_default()
            .wayland_name
            .replace(wayland_name.clone())
        {
            self.wayland_names.remove(&previous);
        }
        self.wayland_names.insert(wayland_name, seat_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_WAYLAND_SEAT_NAME, SeatRegistry};
    use crate::components::SeatId;

    #[test]
    fn default_registry_exposes_primary_wayland_seat() {
        let registry = SeatRegistry::default();

        assert_eq!(registry.primary_seat_id(), SeatId::PRIMARY);
        assert_eq!(registry.seat_name(SeatId::PRIMARY), Some(DEFAULT_WAYLAND_SEAT_NAME));
        assert_eq!(
            registry.seat_id_for_wayland_name(DEFAULT_WAYLAND_SEAT_NAME),
            Some(SeatId::PRIMARY)
        );
    }

    #[test]
    fn ensure_wayland_name_returns_same_id_for_existing_name() {
        let mut registry = SeatRegistry::default();

        assert_eq!(
            registry.ensure_wayland_name(DEFAULT_WAYLAND_SEAT_NAME),
            SeatId::PRIMARY
        );
    }

    #[test]
    fn bind_backend_name_preserves_display_name_priority() {
        let mut registry = SeatRegistry::default();
        registry.bind_backend_name(SeatId::PRIMARY, "seat0");

        assert_eq!(registry.backend_name(SeatId::PRIMARY), Some("seat0"));
        assert_eq!(registry.seat_name(SeatId::PRIMARY), Some(DEFAULT_WAYLAND_SEAT_NAME));
        assert_eq!(registry.seat_id_for_backend_name("seat0"), Some(SeatId::PRIMARY));
    }
}
