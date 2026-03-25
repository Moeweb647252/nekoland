//! Focus-oriented default output selection state.

#![allow(missing_docs)]

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Names the output currently targeted by pointer/keyboard-oriented default routing.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusedOutputState {
    pub id: Option<OutputId>,
}
