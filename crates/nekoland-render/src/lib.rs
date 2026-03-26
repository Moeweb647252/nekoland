#![warn(missing_docs)]

//! Render-list composition, visual-state projection, damage tracking, and compositor effects.

/// Animation-state helpers used while projecting visual snapshots into the render world.
pub mod animation;
/// Desktop scene extraction and render-plan assembly.
pub mod compositor_render;
/// Cursor snapshotting and cursor-scene contribution emission.
pub mod cursor;
/// Output-local damage computation based on render-plan and content-version changes.
pub mod damage_tracker;
/// Effect feature plugins such as blur, shadow, rounded corners, and fade.
pub mod effects;
/// Final-output target selection derived from the compiled render graph.
pub mod final_output_plan;
/// Frame-callback selection for surfaces that should receive `wl_surface.frame` notifications.
pub mod frame_callback;
/// Typed material registration, request queuing, and frame-state projection.
pub mod material;
/// Output overlay scene extraction.
pub mod output_overlay;
/// Overlay-UI scene extraction and compositor-owned UI rendering.
pub mod overlay_ui;
/// Render-phase planning derived from ordered scene contributions and effect requests.
pub mod phase_plan;
/// Pipeline specialization and cache-key projection.
pub mod pipeline_cache;
/// Plugin entrypoints, render schedule wiring, and render/main sync-back.
pub mod plugin;
/// Prepared scene and GPU resource descriptors derived from render plans.
pub mod prepare_resources;
/// Presentation-feedback bookkeeping that feeds the platform boundary.
pub mod presentation_feedback;
/// Post-process plan compilation.
pub mod process_plan;
/// Screenshot and readback planning.
pub mod readback_plan;
/// Backend-neutral render graph compilation.
pub mod render_graph;
/// Scene-process snapshot extraction for opacity and projection adjustments.
pub mod scene_process;
/// Stable scene-contribution keys and render-plan item conversion.
pub mod scene_source;
/// Screenshot request bookkeeping exported from the render cleanup phase.
pub mod screenshot;
/// Shared text shaping and atlas caching for compositor-owned text items.
pub mod text;

pub use plugin::{
    RenderCleanupSystems, RenderExecuteSystems, RenderPlugin, RenderPrepareSystems,
    RenderQueueSystems, RenderSubAppPlugin, configure_render_subapp, sync_render_subapp_back,
};
