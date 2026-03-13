use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Opaque workspace identifier used in snapshots and relationships.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct WorkspaceId(pub u32);

/// Workspace entity metadata tracked by shell systems.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub active: bool,
}

/// Query marker for the workspace currently treated as active by shell scheduling.
#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveWorkspace;
