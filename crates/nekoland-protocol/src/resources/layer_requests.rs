use nekoland_ecs::components::{LayerAnchor, LayerLevel, LayerMargins};
use nekoland_ecs::kinds::ProtocolEventQueue;
use serde::{Deserialize, Serialize};

use super::pending_events::SurfaceExtent;

/// Payload needed to create a layer-shell entity from a protocol request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerSurfaceCreateSpec {
    /// Layer-shell namespace requested by the client.
    pub namespace: String,
    /// Optional target output chosen by the client.
    pub output_name: Option<String>,
    /// Requested layer level (`background`, `top`, etc.).
    pub layer: LayerLevel,
    /// Requested anchor mask.
    pub anchor: LayerAnchor,
    /// Desired logical width from the protocol request.
    pub desired_width: u32,
    /// Desired logical height from the protocol request.
    pub desired_height: u32,
    /// Exclusive zone requested by the client.
    pub exclusive_zone: i32,
    /// Requested layer margins.
    pub margins: LayerMargins,
}

/// Layer-shell lifecycle actions buffered between protocol callbacks and shell systems.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerLifecycleAction {
    /// Initial layer creation with its immutable creation-time metadata.
    Created { spec: LayerSurfaceCreateSpec },
    /// Commit-time refresh of size and anchoring data.
    Committed {
        size: Option<SurfaceExtent>,
        anchor: LayerAnchor,
        desired_width: u32,
        desired_height: u32,
        exclusive_zone: i32,
        margins: LayerMargins,
    },
    /// Final teardown notification.
    Destroyed,
}

/// One layer lifecycle request targeted at a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerLifecycleRequest {
    /// Protocol surface id associated with the layer surface.
    pub surface_id: u64,
    /// Lifecycle action to apply to the layer entity.
    pub action: LayerLifecycleAction,
}

/// Queue of pending layer-shell lifecycle requests.
pub type PendingLayerRequests = ProtocolEventQueue<LayerLifecycleRequest>;
