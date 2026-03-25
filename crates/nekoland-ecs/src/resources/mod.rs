//! ECS resources that carry frame-to-frame compositor state, pending requests, and queryable
//! runtime snapshots.

/// Explicit boundaries exchanged between the shell world and sub-app worlds.
pub mod app_boundary;
/// Normalized backend and platform input queues.
pub mod backend_input;
/// Monotonic compositor clock mirrored across worlds.
pub mod clock;
/// External command history and execution-status snapshots.
pub mod command_history;
/// Queues of shell-facing command requests.
pub mod command_requests;
/// Render-to-platform compiled output frame payloads.
pub mod compiled_output;
/// Compositor-owned scene items emitted alongside desktop surfaces.
pub mod compositor_scene;
/// Cursor snapshots and cursor-image state.
pub mod cursor_state;
/// Damage tracking state and output-local damage regions.
pub mod damage;
/// Transitional stable-id to entity lookup index.
pub mod entity_index;
/// Focused-output selection state.
pub mod focused_output;
/// Runtime state for the compositor-owned FPS HUD.
pub mod fps_hud;
/// Frame pacing, callback selection, and presentation bookkeeping.
pub mod frame_pacing;
/// Keyboard focus, modifiers, and pressed-key state.
pub mod keyboard_state;
/// Typed render-material frame state.
pub mod material_frame;
/// Output viewport animation state mirrored into shell policy.
pub mod output_animation;
/// High-level output control queues and staged handles.
pub mod output_control;
/// Output overlay scene state and control queues.
pub mod output_overlay;
/// Output presentation timelines and feedback snapshots.
pub mod output_presentation;
/// Protocol-facing output server requests.
pub mod output_requests;
/// Normalized output geometry snapshots.
pub mod output_snapshot;
/// Backend/public output status snapshots.
pub mod output_status;
/// Overlay UI state used by compositor-owned status or debug overlays.
pub mod overlay_ui;
/// Backend descriptors and import-capability snapshots.
pub mod platform_backend;
/// Pointer position, deltas, and pan-mode state.
pub mod pointer_state;
/// Backend present-audit snapshots for debug and IPC.
pub mod present_audit;
/// Backend-facing present-surface snapshots.
pub mod present_surface;
/// Primary-output selection state.
pub mod primary_output;
/// Protocol bridge resources and runtime snapshots.
pub mod protocol_bridge;
/// Backend-neutral render graph structures.
pub mod render_graph;
/// Render output data exported by the render sub-app.
pub mod render_output;
/// Render phase-planning resources.
pub mod render_phase;
/// Render plan items and output-local ordered scene plans.
pub mod render_plan;
/// Prepared scene and GPU resource descriptors.
pub mod render_prepare;
/// Post-process and material execution plans.
pub mod render_process;
/// Screenshot and readback planning resources.
pub mod render_readback;
/// Screenshot requests and completed frames.
pub mod screenshot;
/// Stable seat registry snapshots.
pub mod seat_registry;
/// Clipboard, drag-and-drop, and primary-selection state.
pub mod selection_state;
/// Surface content-version snapshots.
pub mod surface_content;
/// Shell-owned surface presentation snapshots.
pub mod surface_presentation;
/// Platform-owned surface snapshots and import descriptors.
pub mod surface_snapshot;
/// Virtual-output capture snapshots.
pub mod virtual_output;
/// High-level window control queues and server requests.
pub mod window_control;
/// Window stacking order state.
pub mod window_stacking;
/// Output work-area state derived from layer-shell exclusivity.
pub mod work_area;
/// Workspace control queues.
pub mod workspace_control;
/// Workspace-local tiling-tree state.
pub mod workspace_tiling;

pub use app_boundary::*;
pub use backend_input::*;
pub use clock::*;
pub use command_history::*;
pub use command_requests::*;
pub use compiled_output::*;
pub use compositor_scene::*;
pub use cursor_state::*;
pub use damage::*;
pub use entity_index::*;
pub use focused_output::*;
pub use fps_hud::*;
pub use frame_pacing::*;
pub use keyboard_state::*;
pub use material_frame::*;
pub use output_animation::*;
pub use output_control::*;
pub use output_overlay::*;
pub use output_presentation::*;
pub use output_requests::*;
pub use output_snapshot::*;
pub use output_status::*;
pub use overlay_ui::*;
pub use platform_backend::*;
pub use pointer_state::*;
pub use present_audit::*;
pub use present_surface::*;
pub use primary_output::*;
pub use protocol_bridge::*;
pub use render_graph::*;
pub use render_output::*;
pub use render_phase::*;
pub use render_plan::*;
pub use render_prepare::*;
pub use render_process::*;
pub use render_readback::*;
pub use screenshot::*;
pub use seat_registry::*;
pub use selection_state::*;
pub use surface_content::*;
pub use surface_presentation::*;
pub use surface_snapshot::*;
pub use virtual_output::*;
pub use window_control::*;
pub use window_stacking::*;
pub use work_area::*;
pub use workspace_control::*;
pub use workspace_tiling::*;
