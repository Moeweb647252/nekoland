use serde::{Deserialize, Serialize};

use crate::components::WorkspaceId;

/// Typed wrapper around compositor surface ids at the control-plane boundary.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct SurfaceId(pub u64);

impl From<u64> for SurfaceId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<SurfaceId> for u64 {
    fn from(value: SurfaceId) -> Self {
        value.0
    }
}

/// Typed workspace display name used at user-facing boundaries.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct WorkspaceName(pub String);

impl WorkspaceName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for WorkspaceName {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for WorkspaceName {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Typed output display name used at user-facing boundaries.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct OutputName(pub String);

impl OutputName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for OutputName {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for OutputName {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Runtime-facing selection of a window target.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowSelector {
    Focused,
    Surface(SurfaceId),
}

impl From<SurfaceId> for WindowSelector {
    fn from(value: SurfaceId) -> Self {
        Self::Surface(value)
    }
}

/// Boundary-facing workspace lookup by stable id or display name.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged, rename_all = "lowercase")]
pub enum WorkspaceLookup {
    Id(WorkspaceId),
    Name(WorkspaceName),
}

impl WorkspaceLookup {
    pub fn parse(boundary: &str) -> Self {
        boundary
            .parse::<u32>()
            .map(WorkspaceId)
            .map(Self::Id)
            .unwrap_or_else(|_| Self::Name(WorkspaceName::from(boundary)))
    }
}

impl From<WorkspaceId> for WorkspaceLookup {
    fn from(value: WorkspaceId) -> Self {
        Self::Id(value)
    }
}

impl From<WorkspaceName> for WorkspaceLookup {
    fn from(value: WorkspaceName) -> Self {
        Self::Name(value)
    }
}

/// Runtime-facing workspace selection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged, rename_all = "lowercase")]
pub enum WorkspaceSelector {
    Active,
    Id(WorkspaceId),
    Name(WorkspaceName),
}

impl WorkspaceSelector {
    pub fn parse(boundary: &str) -> Self {
        match WorkspaceLookup::parse(boundary) {
            WorkspaceLookup::Id(id) => Self::Id(id),
            WorkspaceLookup::Name(name) => Self::Name(name),
        }
    }
}

impl From<WorkspaceId> for WorkspaceSelector {
    fn from(value: WorkspaceId) -> Self {
        Self::Id(value)
    }
}

impl From<WorkspaceName> for WorkspaceSelector {
    fn from(value: WorkspaceName) -> Self {
        Self::Name(value)
    }
}

/// Runtime-facing output selection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputSelector {
    Primary,
    Focused,
    Name(OutputName),
}

impl OutputSelector {
    pub fn parse(boundary: &str) -> Self {
        if boundary.eq_ignore_ascii_case("primary") {
            Self::Primary
        } else if boundary.eq_ignore_ascii_case("focused") {
            Self::Focused
        } else {
            Self::Name(OutputName::from(boundary))
        }
    }
}

impl From<OutputName> for OutputSelector {
    fn from(value: OutputName) -> Self {
        Self::Name(value)
    }
}
