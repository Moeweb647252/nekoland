use std::collections::BTreeMap;

use bevy_ecs::prelude::{Query, ResMut, With};
use nekoland_ecs::components::{
    FadeState, LayerShellSurface, WindowAnimation, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{SurfaceVisualSnapshot, SurfaceVisualState};

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
    mut snapshot: ResMut<SurfaceVisualSnapshot>,
) {
    snapshot.surfaces = windows
        .iter()
        .chain(popups.iter())
        .chain(layers.iter())
        .map(|(surface, animation)| {
            (surface.id, SurfaceVisualState { opacity: opacity_for_animation(animation) })
        })
        .collect::<BTreeMap<_, _>>();
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
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::PreRenderSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{FadeState, WlSurfaceHandle, XdgWindow};
    use nekoland_ecs::resources::SurfaceVisualSnapshot;

    use super::surface_visual_snapshot_system;

    #[test]
    fn idle_zero_progress_defaults_to_full_opacity() {
        let mut app = NekolandApp::new("surface-visual-default-opacity-test");
        app.inner_mut()
            .init_resource::<SurfaceVisualSnapshot>()
            .add_systems(PreRenderSchedule, surface_visual_snapshot_system);

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
    fn explicit_fade_progress_is_preserved() {
        let mut app = NekolandApp::new("surface-visual-fade-opacity-test");
        app.inner_mut()
            .init_resource::<SurfaceVisualSnapshot>()
            .add_systems(PreRenderSchedule, surface_visual_snapshot_system);

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
}
