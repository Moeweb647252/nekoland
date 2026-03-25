//! Protocol-owned resource modules and re-exports shared with the main compositor world.

/// Clipboard selection resources and helpers.
pub mod clipboard;
/// Drag-and-drop resources and event snapshots.
pub mod dnd;
/// Output presentation snapshots mirrored out of protocol feedback.
pub mod output_presentation;

pub use clipboard::*;
pub use dnd::*;
pub use nekoland_ecs::resources::{
    InputEventRecord, LayerLifecycleAction, LayerLifecycleRequest, LayerSurfaceCreateSpec,
    OutputEventRecord, PendingInputEvents, PendingLayerRequests, PendingOutputEvents,
    PendingPopupEvents, PendingPopupServerRequests, PendingWindowEvents,
    PendingWindowServerRequests, PendingX11Requests, PendingXdgRequests, PopupEvent,
    PopupEventRequest, PopupPlacement, PopupServerAction, PopupServerRequest, ProtocolServerState,
    ResizeEdges, SurfaceExtent, WindowEvent, WindowEventRequest, WindowLifecycleAction,
    WindowLifecycleRequest, WindowManagerRequest, WindowServerAction, WindowServerRequest,
    X11LifecycleAction, X11LifecycleRequest, X11WindowGeometry, XWaylandServerState,
    XdgSurfaceRole,
};
pub use output_presentation::*;
