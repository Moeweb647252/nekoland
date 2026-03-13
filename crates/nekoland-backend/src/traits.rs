use bevy_ecs::entity::Entity;
use serde::{Deserialize, Serialize};

use nekoland_core::error::NekolandError;
use nekoland_core::prelude::AppMetadata;
use nekoland_ecs::components::{OutputDevice, OutputProperties, SurfaceGeometry};
use nekoland_ecs::resources::{
    CompositorClock, CompositorConfig, GlobalPointerPosition, PendingBackendInputEvents,
    PendingOutputPresentationEvents, PendingProtocolInputEvents, RenderList,
    VirtualOutputCaptureState,
};
use nekoland_protocol::ProtocolSurfaceRegistry;

use crate::common::outputs::{
    BackendOutputBlueprint, PendingBackendOutputEvents, PendingBackendOutputUpdates,
};
use crate::winit::backend::WinitWindowState;

/// Runtime-selectable backend families supported by the compositor.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BackendKind {
    Drm,
    Winit,
    Virtual,
    #[default]
    Auto,
}

/// Stable identity for one backend runtime instance.
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct BackendId(pub u64);

/// Intended runtime role for a backend instance.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackendRole {
    PrimaryDisplay,
    SecondaryDisplay,
    CaptureSink,
    DebugSink,
}

/// Capability bitset exposed by backend runtimes.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendCapabilities(u32);

impl BackendCapabilities {
    pub const INPUT: Self = Self(1 << 0);
    pub const OUTPUT_DISCOVERY: Self = Self(1 << 1);
    pub const OUTPUT_CONFIGURATION: Self = Self(1 << 2);
    pub const PRESENT: Self = Self(1 << 3);
    pub const PRESENT_TIMELINE: Self = Self(1 << 4);
    pub const CAPTURE: Self = Self(1 << 5);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for BackendCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for BackendCapabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Human-readable descriptor for one backend runtime instance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendDescriptor {
    pub id: BackendId,
    pub kind: BackendKind,
    pub role: BackendRole,
    pub label: String,
    pub description: String,
}

/// ECS-side output snapshot exposed to backend runtimes through constrained contexts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputSnapshot {
    pub entity: Entity,
    pub backend_id: Option<BackendId>,
    pub device: OutputDevice,
    pub properties: OutputProperties,
}

/// Minimal render-surface snapshot exposed to backends during present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderSurfaceRole {
    Window,
    Popup,
    Layer,
    Unknown,
}

/// Present-time surface metadata reconstructed from ECS and protocol state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSurfaceSnapshot {
    pub geometry: SurfaceGeometry,
    pub role: RenderSurfaceRole,
}

/// Constrained extract context that keeps backend runtimes ECS-native without handing them an
/// unrestricted `&mut World`.
pub struct BackendExtractCtx<'a> {
    pub app_metadata: Option<&'a AppMetadata>,
    pub config: Option<&'a CompositorConfig>,
    pub outputs: &'a [OutputSnapshot],
    pub backend_input_events: &'a mut PendingBackendInputEvents,
    pub protocol_input_events: &'a mut PendingProtocolInputEvents,
    pub output_events: &'a mut PendingBackendOutputEvents,
    pub output_updates: &'a mut PendingBackendOutputUpdates,
    pub presentation_events: &'a mut PendingOutputPresentationEvents,
    pub winit_window_state: Option<&'a mut WinitWindowState>,
}

/// Constrained apply context for backend-local reconciliation after common ECS state updates.
pub struct BackendApplyCtx<'a> {
    pub app_metadata: Option<&'a AppMetadata>,
    pub config: Option<&'a CompositorConfig>,
    pub outputs: &'a [OutputSnapshot],
    pub winit_window_state: Option<&'a mut WinitWindowState>,
}

/// Constrained present context that exposes normalized render inputs to backend runtimes.
pub struct BackendPresentCtx<'a> {
    pub config: Option<&'a CompositorConfig>,
    pub clock: Option<&'a CompositorClock>,
    pub pointer: Option<&'a GlobalPointerPosition>,
    pub outputs: &'a [OutputSnapshot],
    pub render_list: &'a RenderList,
    pub surfaces: &'a std::collections::HashMap<u64, RenderSurfaceSnapshot>,
    pub surface_registry: Option<&'a ProtocolSurfaceRegistry>,
    pub virtual_output_capture: Option<&'a mut VirtualOutputCaptureState>,
}

/// Constrained shutdown context. It currently exists to make the runtime contract explicit even
/// though the app lifecycle does not yet invoke it.
pub struct BackendShutdownCtx;

/// Functionally meaningful backend contract used by the manager.
pub trait Backend {
    fn id(&self) -> BackendId;
    fn descriptor(&self) -> BackendDescriptor;
    fn capabilities(&self) -> BackendCapabilities;
    fn seed_output(&self, output_name: &str) -> Option<BackendOutputBlueprint>;

    fn extract(&mut self, cx: &mut BackendExtractCtx<'_>) -> Result<(), NekolandError>;
    fn apply(&mut self, cx: &mut BackendApplyCtx<'_>) -> Result<(), NekolandError>;
    fn present(&mut self, cx: &mut BackendPresentCtx<'_>) -> Result<(), NekolandError>;
    fn shutdown(&mut self, _cx: &mut BackendShutdownCtx) -> Result<(), NekolandError> {
        Ok(())
    }
}
