use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Snapshot of surface content versions keyed by compositor surface id.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceContentVersionSnapshot {
    pub versions: BTreeMap<u64, u64>,
}
