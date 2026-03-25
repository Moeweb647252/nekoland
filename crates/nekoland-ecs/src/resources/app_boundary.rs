use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::resources::{
    ClipboardSelectionState, CompletedScreenshotFrames, CursorImageSnapshot, DragAndDropState,
    GlobalPointerPosition, OutputOverlayState, OutputPresentationState, OutputSnapshotState,
    OverlayUiFrame, PendingLayerRequests, PendingOutputControls, PendingOutputEvents,
    PendingOutputOverlayControls, PendingOutputServerRequests, PendingPlatformInputEvents, PendingPopupEvents,
    PendingPopupServerRequests, PendingProtocolInputEvents, PendingScreenshotRequests,
    PendingWindowControls, PendingWindowEvents, PendingWindowServerRequests, PendingXdgRequests,
    PlatformBackendState, PlatformImportCapabilities, PlatformImportDiagnosticsState,
    PlatformOutputMaterializationPlan, PlatformSurfaceSnapshotState, PresentAuditState,
    PrimaryOutputState, PrimarySelectionState, ProtocolServerState, SeatRegistry,
    SurfacePresentationSnapshot, VirtualOutputCaptureState, XWaylandServerState,
};

/// Platform-to-shell boundary resource carrying normalized platform/runtime snapshots.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WaylandIngress {
    /// Public snapshot of the live Wayland server endpoint and startup state.
    pub protocol_server: ProtocolServerState,
    /// Public snapshot of the XWayland runtime.
    pub xwayland_server: XWaylandServerState,
    /// Current primary-output selection exported by the platform layer.
    pub primary_output: PrimaryOutputState,
    /// Surface currently under the pointer according to platform-side hit testing.
    pub pointer_focus_surface: Option<u64>,
    /// Normalized seat registry exported from protocol/runtime state.
    pub seat_registry: SeatRegistry,
    /// Cursor image selected by the protocol runtime.
    pub cursor_image: CursorImageSnapshot,
    /// Normalized platform input events for the current frame.
    pub platform_input_events: PendingPlatformInputEvents,
    /// Output geometry snapshots with no backend runtime handles.
    pub output_snapshots: OutputSnapshotState,
    /// Platform-owned surface snapshots and import metadata.
    pub surface_snapshots: PlatformSurfaceSnapshotState,
    /// Window lifecycle events emitted by protocol/runtime code.
    pub pending_window_events: PendingWindowEvents,
    /// Popup lifecycle events emitted by protocol/runtime code.
    pub pending_popup_events: PendingPopupEvents,
    /// XDG configure/lifecycle requests emitted by protocol callbacks.
    pub pending_xdg_requests: PendingXdgRequests,
    /// Layer-shell lifecycle requests emitted by protocol callbacks.
    pub pending_layer_requests: PendingLayerRequests,
    /// Protocol-originated high-level window controls.
    pub pending_window_controls: PendingWindowControls,
    /// Output lifecycle notifications emitted by platform/runtime code.
    pub pending_output_events: PendingOutputEvents,
    /// Backend-normalized output materialization plan for the main world.
    pub output_materialization: PlatformOutputMaterializationPlan,
    /// Platform import capabilities exported for render/resource preparation.
    pub import_capabilities: PlatformImportCapabilities,
}

/// Shell-to-render boundary resource carrying shell-owned presentation snapshots.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ShellRenderInput {
    /// Global pointer position after shell-side normalization.
    pub pointer: GlobalPointerPosition,
    /// Cursor image snapshot visible to the shell and render worlds.
    pub cursor_image: CursorImageSnapshot,
    /// Shell-owned surface presentation snapshot used by render and present.
    pub surface_presentation: SurfacePresentationSnapshot,
    /// Output overlay items emitted by shell policy.
    pub output_overlays: OutputOverlayState,
    /// Overlay UI frame emitted by compositor-owned overlay systems.
    pub overlay_ui: OverlayUiFrame,
    /// Screenshot requests that should be planned by the render/present pipeline.
    pub pending_screenshot_requests: PendingScreenshotRequests,
}

/// Shell-to-platform command boundary carrying protocol/backend-side requests.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WaylandCommands {
    /// Output control requests chosen by shell policy.
    pub pending_output_controls: PendingOutputControls,
    /// Output overlay control requests chosen by shell policy.
    pub pending_output_overlay_controls: PendingOutputOverlayControls,
    /// Protocol/backend-facing output server requests.
    pub pending_output_server_requests: PendingOutputServerRequests,
    /// Protocol/backend-facing window server requests.
    pub pending_window_server_requests: PendingWindowServerRequests,
    /// Protocol/backend-facing popup server requests.
    pub pending_popup_server_requests: PendingPopupServerRequests,
    /// Synthetic protocol input events injected by the main world.
    pub pending_protocol_input_events: PendingProtocolInputEvents,
}

/// Platform-to-shell/render feedback boundary carrying present-time and server-side results.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WaylandFeedback {
    /// Public snapshot of active backend runtimes.
    pub platform_backends: PlatformBackendState,
    /// Import diagnostics emitted during backend presentation.
    pub import_diagnostics: PlatformImportDiagnosticsState,
    /// Clipboard-selection snapshot exported from the protocol runtime.
    pub clipboard_selection: ClipboardSelectionState,
    /// Drag-and-drop snapshot exported from the protocol runtime.
    pub drag_and_drop: DragAndDropState,
    /// Primary-selection snapshot exported from the protocol runtime.
    pub primary_selection: PrimarySelectionState,
    /// Screenshot requests still pending completion in platform paths.
    pub pending_screenshot_requests: PendingScreenshotRequests,
    /// Screenshot frames completed by backend present paths.
    pub completed_screenshots: CompletedScreenshotFrames,
    /// Output presentation timeline snapshots.
    pub output_presentation: OutputPresentationState,
    /// Backend-generic present-audit snapshots.
    pub present_audit: PresentAuditState,
    /// Virtual-output capture frames exported by the virtual backend.
    pub virtual_output_capture: VirtualOutputCaptureState,
}
