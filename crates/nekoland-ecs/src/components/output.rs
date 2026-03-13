use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Broad output families exposed by the compositor.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputKind {
    Physical,
    Nested,
    #[default]
    Virtual,
}

/// Stable identity metadata for an output entity.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputDevice {
    pub name: String,
    pub kind: OutputKind,
    pub make: String,
    pub model: String,
}

/// Mutable output mode properties used by layout, rendering, and IPC snapshots.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputProperties {
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u32,
    pub scale: u32,
}
