//! Primary-output selection state used when routing should not depend on iteration order.

#![allow(missing_docs)]

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Names the compositor's current primary output for layout decisions that should not silently
/// depend on query iteration order.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimaryOutputState {
    pub id: Option<OutputId>,
}
