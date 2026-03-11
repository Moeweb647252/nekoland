use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputKind {
    Physical,
    Nested,
    #[default]
    Virtual,
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputDevice {
    pub name: String,
    pub kind: OutputKind,
    pub make: String,
    pub model: String,
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputProperties {
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u32,
    pub scale: u32,
}
