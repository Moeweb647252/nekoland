//! Backend selection and runtime integrations for DRM, nested winit, and virtual output modes.

/// Shared backend-side helpers such as output materialization and presentation timelines.
pub mod common;
/// ECS components owned by backend reconciliation paths.
pub mod components;
/// DRM backend runtime and supporting device/session/input modules.
pub mod drm;
/// Runtime backend manager and status snapshots.
pub mod manager;
/// Main-world and Wayland-subapp plugin wiring for backend extraction and present.
pub mod plugin;
/// Backend contract types shared by `winit`, `drm`, and `virtual` runtimes.
pub mod traits;
/// Offscreen backend used for capture, soak, and render-audit scenarios.
pub mod virtual_output;
/// Nested development backend built on Smithay's `winit` integration.
pub mod winit;

pub use manager::{BackendStatus, SharedBackendManager, set_requested_backend_override};
pub use plugin::extract::extract_backend_wayland_subapp_inputs;
pub use plugin::{BackendPlugin, BackendWaylandSubAppPlugin};
pub use winit::backend::WinitWindowState;
