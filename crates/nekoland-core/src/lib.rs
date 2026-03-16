//! Core application loop, schedule definitions, and runtime glue shared by all nekoland crates.

pub mod app;
pub mod bridge;
pub mod calloop;
pub mod error;
pub mod lifecycle;
pub mod plugin;
pub mod schedules;

pub mod prelude {
    pub use crate::app::{AppMetadata, NekolandApp, RunLoopSettings};
    pub use crate::bridge::{EventBridge, WaylandBridge};
    pub use crate::calloop::CalloopSourceRegistry;
    pub use crate::error::NekolandError;
    pub use crate::lifecycle::AppLifecycleState;
    pub use crate::plugin::NekolandPlugin;
    pub use crate::schedules::{
        ExtractSchedule, InputSchedule, LayoutSchedule, PresentSchedule, ProtocolSchedule,
        RenderSchedule,
    };
}
