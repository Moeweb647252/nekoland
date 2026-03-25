//! ECS components that describe compositor entities such as windows, outputs, layers, and seats.

/// Animation components shared by shell policy and render extraction.
pub mod animation;
/// Decoration and border-related components.
pub mod decoration;
/// Layer-shell components and output binding markers.
pub mod layer;
/// Output identity, placement, and viewport components.
pub mod output;
/// Popup-surface and popup-grab components.
pub mod popup;
/// Seat identifiers and focus-related seat components.
pub mod seat;
/// Shared surface identifiers, geometry, and content-version components.
pub mod surface;
/// Window metadata, policy, placement, and mode components.
pub mod window;
/// Workspace identity and active-state components.
pub mod workspace;
/// XWayland-specific window metadata components.
pub mod x11;

pub use animation::*;
pub use decoration::*;
pub use layer::*;
pub use output::*;
pub use popup::*;
pub use seat::*;
pub use surface::*;
pub use window::*;
pub use workspace::*;
pub use x11::*;
