use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::resources::{
    ClipboardSelectionState, CompletedScreenshotFrames, CursorImageSnapshot, DragAndDropState,
    GlobalPointerPosition, OutputOverlayState, OutputPresentationState, OutputSnapshotState,
    PendingLayerRequests, PendingOutputControls, PendingOutputEvents, PendingOutputOverlayControls,
    PendingOutputServerRequests, PendingPlatformInputEvents, PendingPopupServerRequests,
    PendingProtocolInputEvents, PendingScreenshotRequests, PendingWindowControls,
    PendingWindowServerRequests, PendingX11Requests, PendingXdgRequests, PlatformBackendState,
    PlatformImportCapabilities, PlatformImportDiagnosticsState, PlatformOutputMaterializationPlan,
    PlatformSurfaceSnapshotState, PresentAuditState, PrimaryOutputState, PrimarySelectionState, ProtocolServerState,
    SurfacePresentationSnapshot, VirtualOutputCaptureState,
    XWaylandServerState,
};

/// Platform-to-shell mailbox carrying normalized platform/runtime snapshots.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WaylandIngress {
    pub protocol_server: ProtocolServerState,
    pub xwayland_server: XWaylandServerState,
    pub primary_output: PrimaryOutputState,
    pub cursor_image: CursorImageSnapshot,
    pub platform_input_events: PendingPlatformInputEvents,
    pub output_snapshots: OutputSnapshotState,
    pub surface_snapshots: PlatformSurfaceSnapshotState,
    pub pending_xdg_requests: PendingXdgRequests,
    pub pending_layer_requests: PendingLayerRequests,
    pub pending_x11_requests: PendingX11Requests,
    pub pending_window_controls: PendingWindowControls,
    pub pending_output_events: PendingOutputEvents,
    pub output_materialization: PlatformOutputMaterializationPlan,
    pub import_capabilities: PlatformImportCapabilities,
}

/// Shell-to-render mailbox carrying shell-owned presentation snapshots.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ShellRenderInput {
    pub pointer: GlobalPointerPosition,
    pub cursor_image: CursorImageSnapshot,
    pub surface_presentation: SurfacePresentationSnapshot,
    pub output_overlays: OutputOverlayState,
    pub pending_screenshot_requests: PendingScreenshotRequests,
}

/// Shell-to-platform command mailbox carrying protocol/backend-side requests.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WaylandCommands {
    pub pending_output_controls: PendingOutputControls,
    pub pending_output_overlay_controls: PendingOutputOverlayControls,
    pub pending_output_server_requests: PendingOutputServerRequests,
    pub pending_window_server_requests: PendingWindowServerRequests,
    pub pending_popup_server_requests: PendingPopupServerRequests,
    pub pending_protocol_input_events: PendingProtocolInputEvents,
}

/// Platform-to-shell/render feedback mailbox carrying present-time and server-side results.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WaylandFeedback {
    pub platform_backends: PlatformBackendState,
    pub import_diagnostics: PlatformImportDiagnosticsState,
    pub clipboard_selection: ClipboardSelectionState,
    pub drag_and_drop: DragAndDropState,
    pub primary_selection: PrimarySelectionState,
    pub pending_screenshot_requests: PendingScreenshotRequests,
    pub completed_screenshots: CompletedScreenshotFrames,
    pub output_presentation: OutputPresentationState,
    pub present_audit: PresentAuditState,
    pub virtual_output_capture: VirtualOutputCaptureState,
}
