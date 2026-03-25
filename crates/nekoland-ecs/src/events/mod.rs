//! ECS messages emitted during input, output, and window lifecycle processing.

/// Input-facing ECS messages emitted by the normalized input pipeline.
pub mod input_events;
/// Output lifecycle messages emitted when outputs are materialized or removed.
pub mod output_events;
/// Window lifecycle messages emitted by shell policy systems.
pub mod window_events;

pub use input_events::*;
pub use output_events::*;
pub use window_events::*;
