use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

use crate::traits::{BackendId, BackendOutputId};

/// Explicit ownership metadata that binds one output entity to the backend runtime that owns it.
#[derive(Component, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputBackend {
    /// Installed backend runtime responsible for extracting/applying/presenting this output.
    pub backend_id: BackendId,
    /// Backend-scoped identity used for caches and reconnect-aware ownership.
    pub output_id: BackendOutputId,
}
