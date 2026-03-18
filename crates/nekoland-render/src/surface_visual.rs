use std::collections::BTreeMap;

use bevy_ecs::prelude::{Query, Res, ResMut, With};
use nekoland_ecs::components::{
    FadeState, LayerShellSurface, WindowAnimation, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{SurfaceVisualSnapshot, SurfaceVisualState};

use crate::animation::{
    AnimationBindingKey, AnimationProperty, AnimationTimelineStore, AnimationValue,
};
use crate::scene_source::RenderSourceKey;

type WindowVisualQuery<'w, 's> =
    Query<'w, 's, (&'static WlSurfaceHandle, &'static WindowAnimation), With<XdgWindow>>;
type PopupVisualQuery<'w, 's> =
    Query<'w, 's, (&'static WlSurfaceHandle, &'static WindowAnimation), With<XdgPopup>>;
type LayerVisualQuery<'w, 's> =
    Query<'w, 's, (&'static WlSurfaceHandle, &'static WindowAnimation), With<LayerShellSurface>>;

/// Projects animation/effect state into a narrow per-surface visual snapshot for later render use.
pub fn surface_visual_snapshot_system(
    windows: WindowVisualQuery<'_, '_>,
    popups: PopupVisualQuery<'_, '_>,
    layers: LayerVisualQuery<'_, '_>,
    timelines: Res<'_, AnimationTimelineStore>,
    mut snapshot: ResMut<'_, SurfaceVisualSnapshot>,
) {
    snapshot.surfaces = windows
        .iter()
        .chain(popups.iter())
        .chain(layers.iter())
        .map(|(surface, animation)| {
            let mut state = SurfaceVisualState {
                opacity: opacity_for_animation(animation),
                ..Default::default()
            };
            apply_source_animation_overrides(surface.id, &timelines, &mut state);
            (surface.id, state)
        })
        .collect::<BTreeMap<_, _>>();
}

fn apply_source_animation_overrides(
    surface_id: u64,
    timelines: &AnimationTimelineStore,
    state: &mut SurfaceVisualState,
) {
    let binding = AnimationBindingKey::Source(RenderSourceKey::surface(surface_id));

    if let Some(AnimationValue::Float(opacity)) =
        timelines.sampled_value(&binding, AnimationProperty::Opacity)
    {
        state.opacity = (*opacity).clamp(0.0, 1.0);
    }
    if let Some(AnimationValue::Rect(rect)) =
        timelines.sampled_value(&binding, AnimationProperty::Rect)
    {
        state.rect_override = Some(*rect);
    }
    if let Some(AnimationValue::Rect(rect)) =
        timelines.sampled_value(&binding, AnimationProperty::ClipRect)
    {
        state.clip_rect_override = Some(*rect);
    }
}

fn opacity_for_animation(animation: &WindowAnimation) -> f32 {
    if animation.progress == 0.0 && animation.fade == FadeState::Idle {
        1.0
    } else {
        animation.progress.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::PreRenderSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{FadeState, WlSurfaceHandle, XdgWindow};
    use nekoland_ecs::resources::{CompositorClock, RenderRect, SurfaceVisualSnapshot};

    use crate::animation::{
        AnimationBindingKey, AnimationEasing, AnimationProperty, AnimationTimelineStore,
        AnimationTrack, AnimationValue, advance_animation_timelines_system,
    };
    use crate::scene_source::RenderSourceKey;

    use super::surface_visual_snapshot_system;

    #[test]
    fn idle_zero_progress_defaults_to_full_opacity() {
        let mut app = NekolandApp::new("surface-visual-default-opacity-test");
        app.inner_mut()
            .init_resource::<SurfaceVisualSnapshot>()
            .init_resource::<AnimationTimelineStore>()
            .add_systems(
                PreRenderSchedule,
                (advance_animation_timelines_system, surface_visual_snapshot_system).chain(),
            );

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            window: XdgWindow {
                app_id: "org.nekoland.test".to_owned(),
                title: "window".to_owned(),
                last_acked_configure: None,
            },
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);

        let snapshot = app.inner().world().resource::<SurfaceVisualSnapshot>();
        assert_eq!(snapshot.surfaces.get(&11).map(|state| state.opacity), Some(1.0));
    }

    #[test]
    fn explicit_fade_progress_is_preserved_without_timeline_override() {
        let mut app = NekolandApp::new("surface-visual-fade-opacity-test");
        app.inner_mut()
            .init_resource::<SurfaceVisualSnapshot>()
            .init_resource::<AnimationTimelineStore>()
            .add_systems(
                PreRenderSchedule,
                (advance_animation_timelines_system, surface_visual_snapshot_system).chain(),
            );

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 12 },
            window: XdgWindow {
                app_id: "org.nekoland.test".to_owned(),
                title: "window".to_owned(),
                last_acked_configure: None,
            },
            animation: nekoland_ecs::components::WindowAnimation {
                progress: 0.35,
                fade: FadeState::Out,
                target_opacity: 0.0,
                duration_ms: 120,
                elapsed_ms: 42,
            },
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);

        let snapshot = app.inner().world().resource::<SurfaceVisualSnapshot>();
        assert_eq!(snapshot.surfaces.get(&12).map(|state| state.opacity), Some(0.35));
    }

    #[test]
    fn timeline_overrides_opacity_and_rect_fields() {
        let mut app = NekolandApp::new("surface-visual-animation-override-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 50 })
            .init_resource::<SurfaceVisualSnapshot>()
            .init_resource::<AnimationTimelineStore>()
            .add_systems(
                PreRenderSchedule,
                (advance_animation_timelines_system, surface_visual_snapshot_system).chain(),
            );

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 13 },
            window: XdgWindow {
                app_id: "org.nekoland.test".to_owned(),
                title: "window".to_owned(),
                last_acked_configure: None,
            },
            ..Default::default()
        });
        {
            let mut timelines =
                app.inner_mut().world_mut().resource_mut::<AnimationTimelineStore>();
            let binding = AnimationBindingKey::Source(RenderSourceKey::surface(13));
            timelines.upsert_track(
                binding.clone(),
                AnimationTrack {
                    property: AnimationProperty::Opacity,
                    from: AnimationValue::Float(0.0),
                    to: AnimationValue::Float(1.0),
                    start_uptime_millis: 0,
                    duration_millis: 100,
                    easing: AnimationEasing::Linear,
                },
            );
            timelines.upsert_track(
                binding,
                AnimationTrack {
                    property: AnimationProperty::Rect,
                    from: AnimationValue::Rect(RenderRect { x: 0, y: 0, width: 50, height: 50 }),
                    to: AnimationValue::Rect(RenderRect { x: 10, y: 20, width: 60, height: 70 }),
                    start_uptime_millis: 0,
                    duration_millis: 100,
                    easing: AnimationEasing::Linear,
                },
            );
        }

        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);

        let snapshot = app.inner().world().resource::<SurfaceVisualSnapshot>();
        let state = &snapshot.surfaces[&13];
        assert_eq!(state.opacity, 0.5);
        assert_eq!(state.rect_override, Some(RenderRect { x: 5, y: 10, width: 55, height: 60 }));
    }
}
