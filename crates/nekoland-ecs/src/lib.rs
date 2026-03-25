#![warn(missing_docs)]

//! Shared ECS-facing data model used across the workspace.
//!
//! This crate intentionally stays close to pure data definitions so backend, protocol, shell, and
//! render crates can communicate through stable component/resource/message types.

/// Canonical bundles used when materializing outputs, windows, and layer-shell entities.
pub mod bundles;
/// ECS components for compositor-owned entities.
pub mod components;
/// High-level control facades for staging shell actions from systems and IPC.
pub mod control;
/// ECS messages emitted by shell, backend, and protocol systems.
pub mod events;
/// Marker traits and queue abstractions shared across event families.
pub mod kinds;
/// Shared visibility and presentation semantics used across shell and render extraction.
pub mod presentation_logic;
/// Shared resources carrying frame state, boundaries, render plans, and control queues.
pub mod resources;
/// Stable selectors used by IPC, config actions, and shell control paths.
pub mod selectors;
/// QueryData views that bundle commonly co-accessed ECS state.
pub mod views;
/// Workspace/output relationship helpers shared by shell, IPC, and render extraction.
pub mod workspace_membership;

/// Convenience re-exports for most shared ECS-facing types.
pub mod prelude {
    pub use crate::bundles::{OutputBundle, WindowBundle, X11WindowBundle};
    pub use crate::components::*;
    pub use crate::control::*;
    pub use crate::events::*;
    pub use crate::kinds::*;
    pub use crate::presentation_logic::*;
    pub use crate::resources::*;
    pub use crate::selectors::*;
    pub use crate::views::*;
    pub use crate::workspace_membership::*;
}
