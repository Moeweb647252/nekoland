//! Shared backend helpers reused by multiple runtime implementations.

/// Cursor composition helpers shared by software and nested backends.
pub mod cursor;
/// GLES render-graph execution utilities reused by runtime backends.
pub mod gles_executor;
/// Output materialization, reconciliation, and viewport helpers.
pub mod outputs;
/// Presentation timeline helpers and completion event emission.
pub mod presentation;
/// Stable output-local render ordering helpers.
pub mod render_order;
