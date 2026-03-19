use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Per-window animation state.
///
/// `progress` is the primary hook used by the render compositor to set the
/// surface opacity once the render-time appearance projection has sampled it
/// into the frame-local appearance snapshot (0.0 = transparent, 1.0 = fully
/// opaque). The
/// remaining fields are reserved for the future animation driver in
/// `nekoland-render/src/effects/fade.rs`:
///
/// - `target_opacity`: the opacity value the animation is moving towards.
/// - `duration_ms`:    total animation length in milliseconds.
/// - `elapsed_ms`:     how many milliseconds have elapsed in the current animation.
///
/// When `fade` is `Idle`, neither `elapsed_ms` nor `duration_ms` are meaningful
/// and the render pass should treat the surface as fully opaque (`progress = 1.0
/// or 0.0 on spawn` is handled by the spawner).
#[derive(Component, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WindowAnimation {
    /// Current render opacity used by the compositor (driven by the effect system).
    pub progress: f32,
    /// Current fade direction.
    pub fade: FadeState,
    /// Target opacity once the animation completes (FUTURE: used by fade system).
    pub target_opacity: f32,
    /// Total duration of the animation in milliseconds (FUTURE: used by fade system).
    pub duration_ms: u32,
    /// Elapsed time of the current animation in milliseconds (FUTURE: driven by render tick).
    pub elapsed_ms: u32,
}

impl Default for WindowAnimation {
    fn default() -> Self {
        Self {
            progress: 0.0,
            fade: FadeState::Idle,
            target_opacity: 1.0,
            duration_ms: 0,
            elapsed_ms: 0,
        }
    }
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum FadeState {
    /// Surface is fading in toward full opacity.
    In,
    /// Surface is fading out toward transparency.
    Out,
    /// No fade is currently progressing.
    #[default]
    Idle,
}
