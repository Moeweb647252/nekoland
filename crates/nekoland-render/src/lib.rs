//! Render-list composition, visual-state projection, damage tracking, and compositor effects.

pub mod animation;
pub mod compositor_render;
pub mod cursor;
pub mod damage_tracker;
pub mod effects;
pub mod final_output_plan;
pub mod frame_callback;
pub mod material;
pub mod overlay_ui;
pub mod output_overlay;
pub mod phase_plan;
pub mod pipeline_cache;
pub mod plugin;
pub mod prepare_resources;
pub mod presentation_feedback;
pub mod process_plan;
pub mod readback_plan;
pub mod render_graph;
pub mod scene_process;
pub mod scene_source;
pub mod screenshot;

pub use plugin::{
    RenderCleanupSystems, RenderExecuteSystems, RenderPlugin, RenderPrepareSystems,
    RenderQueueSystems, RenderSubAppPlugin, configure_render_subapp, sync_render_subapp_back,
};
