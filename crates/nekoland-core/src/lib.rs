//! Core application loop, schedule definitions, and runtime glue shared by all nekoland crates.
#![warn(missing_docs)]

/// Application runner types, sub-app labels, and the outer compositor frame loop.
pub mod app;
/// FIFO bridge primitives used when callback-driven code must hand work into ECS schedules.
pub mod bridge;
/// Calloop integration shared by protocol and backend runtime installers.
pub mod calloop;
/// Common error type used across the workspace's runtime infrastructure.
pub mod error;
/// Process-lifetime state shared by plugins that can request shutdown.
pub mod lifecycle;
/// Minimal plugin abstractions used by the root app and its sub-apps.
pub mod plugin;
/// Canonical frame schedule labels shared across the workspace.
pub mod schedules;

/// Frequently used core types re-exported for plugin and app wiring code.
pub mod prelude {
    pub use crate::app::{AppMetadata, NekolandApp, RenderSubApp, RunLoopSettings, WaylandSubApp};
    pub use crate::bridge::{EventBridge, WaylandBridge};
    pub use crate::calloop::CalloopSourceRegistry;
    pub use crate::error::NekolandError;
    pub use crate::lifecycle::AppLifecycleState;
    pub use crate::plugin::NekolandPlugin;
    pub use crate::schedules::{
        ExtractSchedule, InputSchedule, LayoutSchedule, PostRenderSchedule, PreRenderSchedule,
        PresentSchedule, ProtocolSchedule, RenderSchedule,
    };
}
