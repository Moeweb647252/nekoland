//! Normalized runtime resources derived from disk config and hot-reload state.

/// Normalized compositor policy and action configuration consumed by runtime systems.
pub mod compositor_config;
/// Runtime keyboard-layout state derived from normalized config data.
pub mod keyboard_layout;

pub use compositor_config::*;
pub use keyboard_layout::*;
