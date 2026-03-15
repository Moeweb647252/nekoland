//! Shell-side state transitions: workspace management, focus, layout, decorations, and protocol
//! lifecycle handling for XDG/X11 surfaces.

pub mod commands;
pub mod decorations;
pub mod focus;
pub mod interaction;
pub mod layer;
pub mod layout;
pub mod plugin;
mod presentation;
pub mod viewport;
pub mod window_control;
mod window_policy;
pub mod workspace;
pub mod x11;
pub mod xdg;

pub use plugin::ShellPlugin;
