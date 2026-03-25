//! Shell-side state transitions: workspace management, focus, layout, decorations, and protocol
//! lifecycle handling for XDG/X11 surfaces.
#![warn(missing_docs)]

/// Startup actions, external command dispatch, and command-history bookkeeping.
pub mod commands;
/// Server-side decoration projection and border-theme updates.
pub mod decorations;
/// Keyboard and pointer focus policy derived from visible shell state.
pub mod focus;
mod fps_hud;
/// Interactive move/resize grabs driven by pointer input.
pub mod interaction;
/// Layer-shell lifecycle and arrangement policy.
pub mod layer;
/// Window layout strategies such as tiling, floating, stacking, and fullscreen.
pub mod layout;
/// Main-world plugin entrypoint for shell policy and boundary synchronization.
pub mod plugin;
mod presentation;
mod surface_presentation;
/// Output viewport projection and scene-to-output coordinate helpers.
pub mod viewport;
/// High-level control-plane handling for focused window actions.
pub mod window_control;
mod window_lifecycle;
mod window_policy;
/// Workspace creation, switching, and output/workspace routing.
pub mod workspace;
/// XDG-shell request handling and popup/configure sequencing.
pub mod xdg;

pub use plugin::ShellPlugin;
