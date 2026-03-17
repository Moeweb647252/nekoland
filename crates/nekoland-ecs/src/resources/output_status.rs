use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Tracks the output names currently materialized in ECS by any backend runtime.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputRegistry {
    /// Runtime output ids that are currently known to be physically/backend connected.
    pub connected_by_id: std::collections::BTreeMap<OutputId, String>,
    /// Runtime output ids that are currently materialized as enabled ECS entities.
    pub enabled_by_id: std::collections::BTreeMap<OutputId, String>,
    /// Name-to-id lookup for the currently connected output set.
    pub ids_by_name: std::collections::BTreeMap<String, OutputId>,
}

impl BackendOutputRegistry {
    pub fn remember_connected(&mut self, output_id: OutputId, output_name: String) {
        self.ids_by_name.insert(output_name.clone(), output_id);
        self.connected_by_id.insert(output_id, output_name);
    }

    pub fn remember_enabled(&mut self, output_id: OutputId, output_name: String) {
        self.ids_by_name.insert(output_name.clone(), output_id);
        self.enabled_by_id.insert(output_id, output_name);
    }

    pub fn forget_enabled_name(&mut self, output_name: &str) {
        if let Some(output_id) = self.ids_by_name.get(output_name).copied() {
            self.enabled_by_id.remove(&output_id);
        } else {
            self.enabled_by_id.retain(|_, candidate_name| candidate_name != output_name);
        }
    }

    pub fn forget_connected_name(&mut self, output_name: &str) {
        if let Some(output_id) = self.ids_by_name.remove(output_name) {
            self.connected_by_id.remove(&output_id);
            self.enabled_by_id.remove(&output_id);
        } else {
            self.connected_by_id.retain(|_, candidate_name| candidate_name != output_name);
            self.enabled_by_id.retain(|_, candidate_name| candidate_name != output_name);
        }
    }

    pub fn has_enabled_name(&self, output_name: &str) -> bool {
        self.ids_by_name
            .get(output_name)
            .is_some_and(|output_id| self.enabled_by_id.contains_key(output_id))
            || self.enabled_by_id.values().any(|candidate_name| candidate_name == output_name)
    }

    pub fn has_connected_name(&self, output_name: &str) -> bool {
        self.ids_by_name
            .get(output_name)
            .is_some_and(|output_id| self.connected_by_id.contains_key(output_id))
            || self.connected_by_id.values().any(|candidate_name| candidate_name == output_name)
    }
}
