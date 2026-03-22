use bevy_app::App;
use bevy_ecs::prelude::{Query, Res, ResMut};
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::PreRenderSchedule;
use nekoland_ecs::components::{FadeState, WindowAnimation, WlSurfaceHandle};
use nekoland_ecs::resources::CompositorClock;

use crate::animation::{
    AnimationBindingKey, AnimationEasing, AnimationProperty, AnimationTimelineStore,
    AnimationTrack, AnimationValue,
};
use crate::scene_source::RenderSourceKey;

#[derive(Debug, Default, Clone, Copy)]
pub struct FadeEffectPlugin;

impl NekolandPlugin for FadeEffectPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreRenderSchedule, fade_effect_system);
    }
}

/// Fade-in/fade-out animation driver.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FadeEffect {
    pub duration_ms: u32,
}

/// Global config for fade animations.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct FadeEffectConfig {
    pub enabled: bool,
    pub open_duration_ms: u32,
    pub close_duration_ms: u32,
}

impl Default for FadeEffectConfig {
    fn default() -> Self {
        Self { enabled: false, open_duration_ms: 200, close_duration_ms: 150 }
    }
}

type FadeQuery<'w, 's> = Query<'w, 's, (&'static WlSurfaceHandle, &'static mut WindowAnimation)>;

/// Adapts legacy `WindowAnimation` state onto the generic timeline runtime.
pub fn fade_effect_system(
    clock: Option<Res<'_, CompositorClock>>,
    mut timelines: ResMut<'_, AnimationTimelineStore>,
    mut animations: FadeQuery<'_, '_>,
) {
    let current_uptime_millis =
        clock.as_deref().map(|clock| clock.uptime_millis).unwrap_or_default();
    let delta_millis = timelines.delta_millis(current_uptime_millis);

    for (surface, mut animation) in &mut animations {
        drive_window_animation(
            surface.id,
            &mut animation,
            current_uptime_millis,
            delta_millis,
            &mut timelines,
        );
    }
}

fn drive_window_animation(
    surface_id: u64,
    animation: &mut WindowAnimation,
    current_uptime_millis: u128,
    delta_millis: u32,
    timelines: &mut AnimationTimelineStore,
) {
    let binding = AnimationBindingKey::Source(RenderSourceKey::surface(surface_id));

    match animation.fade {
        FadeState::Idle => {
            timelines.remove_track(&binding, AnimationProperty::Opacity);
            animation.progress = animation.progress.clamp(0.0, 1.0);
            animation.elapsed_ms = animation.elapsed_ms.min(animation.duration_ms);
        }
        FadeState::In | FadeState::Out => {
            let total_duration = animation.duration_ms.max(1);
            animation.elapsed_ms =
                animation.elapsed_ms.saturating_add(delta_millis).min(total_duration);
            let target = animation.target_opacity.clamp(0.0, 1.0);
            let start = match animation.fade {
                FadeState::In => 0.0,
                FadeState::Out => 1.0,
                FadeState::Idle => animation.progress.clamp(0.0, 1.0),
            };
            let elapsed = u128::from(animation.elapsed_ms.min(total_duration));
            let start_uptime_millis = current_uptime_millis.saturating_sub(elapsed);
            timelines.upsert_track(
                binding,
                AnimationTrack {
                    property: AnimationProperty::Opacity,
                    from: AnimationValue::Float(start),
                    to: AnimationValue::Float(target),
                    start_uptime_millis,
                    duration_millis: total_duration,
                    easing: AnimationEasing::EaseInOut,
                },
            );
            animation.progress = fade_progress(start, target, animation.elapsed_ms, total_duration);
            if animation.elapsed_ms >= total_duration {
                animation.fade = FadeState::Idle;
                animation.progress = target;
            }
        }
    }
}

fn fade_progress(start: f32, target: f32, elapsed_ms: u32, duration_ms: u32) -> f32 {
    let progress = (elapsed_ms as f32 / duration_ms.max(1) as f32).clamp(0.0, 1.0);
    let eased = if progress <= 0.0 {
        0.0
    } else if progress >= 1.0 {
        1.0
    } else {
        progress * progress * (3.0 - 2.0 * progress)
    };
    start + (target - start) * eased
}

#[cfg(test)]
mod tests {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::PreRenderSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{FadeState, WlSurfaceHandle, XdgWindow};
    use nekoland_ecs::resources::CompositorClock;

    use crate::animation::{
        AnimationProperty, AnimationTimelineStore, AnimationValue,
        advance_animation_timelines_system,
    };
    use crate::scene_source::RenderSourceKey;

    use super::fade_effect_system;

    #[test]
    fn fade_in_updates_progress_and_timeline_sample() {
        let mut app = NekolandApp::new("fade-effect-runtime-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 40 })
            .init_resource::<AnimationTimelineStore>()
            .add_systems(
                PreRenderSchedule,
                (fade_effect_system, advance_animation_timelines_system).chain(),
            );

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 11 },
                window: XdgWindow::default(),
                animation: nekoland_ecs::components::WindowAnimation {
                    progress: 0.0,
                    fade: FadeState::In,
                    target_opacity: 1.0,
                    duration_ms: 100,
                    elapsed_ms: 0,
                },
                ..Default::default()
            })
            .id();

        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);

        let animation = app
            .inner()
            .world()
            .get::<nekoland_ecs::components::WindowAnimation>(entity)
            .expect("window animation");
        assert_eq!(animation.elapsed_ms, 40);
        assert!(animation.progress > 0.0);
        assert_eq!(
            app.inner().world().resource::<AnimationTimelineStore>().sampled_value(
                &crate::animation::AnimationBindingKey::Source(RenderSourceKey::surface(11)),
                AnimationProperty::Opacity,
            ),
            Some(&AnimationValue::Float(animation.progress))
        );
    }

    #[test]
    fn completed_fade_out_transitions_to_idle() {
        let mut app = NekolandApp::new("fade-effect-complete-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 100 })
            .init_resource::<AnimationTimelineStore>()
            .add_systems(
                PreRenderSchedule,
                (fade_effect_system, advance_animation_timelines_system).chain(),
            );

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 12 },
                window: XdgWindow::default(),
                animation: nekoland_ecs::components::WindowAnimation {
                    progress: 1.0,
                    fade: FadeState::Out,
                    target_opacity: 0.0,
                    duration_ms: 100,
                    elapsed_ms: 80,
                },
                ..Default::default()
            })
            .id();

        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);

        let animation = app
            .inner()
            .world()
            .get::<nekoland_ecs::components::WindowAnimation>(entity)
            .expect("window animation");
        assert_eq!(animation.fade, FadeState::Idle);
        assert_eq!(animation.progress, 0.0);
    }
}
