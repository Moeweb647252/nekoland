//! Render-list composition, visual-state projection, damage tracking, and compositor effects.

pub mod compositor_render;
pub mod cursor;
pub mod damage_tracker;
pub mod effects;
pub mod frame_callback;
pub mod material;
pub mod plugin;
pub mod presentation_feedback;
pub mod render_graph;
pub mod scene_source;
pub mod screenshot;
pub mod surface_visual;

pub use plugin::RenderPlugin;
