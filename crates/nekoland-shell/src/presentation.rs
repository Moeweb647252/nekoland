use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::{Local, Query, ResMut, With};
use nekoland_ecs::components::{SurfaceGeometry, WindowMode, XdgWindow};
use nekoland_ecs::resources::{
    PendingWindowServerRequests, WindowServerAction, WindowServerRequest,
};
use nekoland_ecs::views::{WindowSnapshotRuntime, WindowSnapshotRuntimeItem};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowPresentationState {
    geometry: SurfaceGeometry,
    fullscreen: bool,
    maximized: bool,
}

pub fn window_presentation_sync_system(
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
    windows: Query<WindowSnapshotRuntime, With<XdgWindow>>,
    mut synced: Local<BTreeMap<u64, WindowPresentationState>>,
) {
    let mut seen_surfaces = BTreeSet::new();

    for window in windows.iter() {
        let surface_id = window.surface_id();
        seen_surfaces.insert(surface_id);

        let desired = desired_presentation_state(&window);
        if synced.get(&surface_id) == Some(&desired) {
            continue;
        }

        pending_window_requests.push(WindowServerRequest {
            surface_id,
            action: WindowServerAction::SyncPresentation {
                geometry: desired.geometry.clone(),
                fullscreen: desired.fullscreen,
                maximized: desired.maximized,
            },
        });
        synced.insert(surface_id, desired);
    }

    synced.retain(|surface_id, _| seen_surfaces.contains(surface_id));
}

fn desired_presentation_state(
    window: &WindowSnapshotRuntimeItem<'_, '_>,
) -> WindowPresentationState {
    let fullscreen = window.background.is_some() || *window.mode == WindowMode::Fullscreen;
    let maximized = window.background.is_none() && *window.mode == WindowMode::Maximized;
    let geometry = match window.x11_window {
        Some(_) if fullscreen || maximized => window.geometry.clone(),
        Some(_) => SurfaceGeometry {
            x: saturating_isize_to_i32(window.scene_geometry.x),
            y: saturating_isize_to_i32(window.scene_geometry.y),
            width: window.scene_geometry.width.max(1),
            height: window.scene_geometry.height.max(1),
        },
        None if fullscreen || maximized => SurfaceGeometry {
            x: 0,
            y: 0,
            width: window.geometry.width.max(1),
            height: window.geometry.height.max(1),
        },
        None => SurfaceGeometry {
            x: 0,
            y: 0,
            width: window.scene_geometry.width.max(1),
            height: window.scene_geometry.height.max(1),
        },
    };

    WindowPresentationState { geometry, fullscreen, maximized }
}

fn saturating_isize_to_i32(value: isize) -> i32 {
    value.clamp(i32::MIN as isize, i32::MAX as isize) as i32
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{WindowBundle, X11WindowBundle};
    use nekoland_ecs::components::{
        OutputBackgroundWindow, WindowLayout, WindowRestoreState, WindowSceneGeometry,
        WlSurfaceHandle, X11Window,
    };
    use nekoland_ecs::resources::{PendingWindowServerRequests, WindowServerAction};

    use super::window_presentation_sync_system;

    #[test]
    fn background_windows_sync_fullscreen_output_local_presentation() {
        let mut app = NekolandApp::new("window-presentation-background-test");
        app.inner_mut()
            .init_resource::<PendingWindowServerRequests>()
            .add_systems(LayoutSchedule, window_presentation_sync_system);

        app.inner_mut().world_mut().spawn((
            X11WindowBundle {
                surface: WlSurfaceHandle { id: 11 },
                geometry: nekoland_ecs::components::SurfaceGeometry {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                scene_geometry: WindowSceneGeometry { x: 200, y: 300, width: 640, height: 480 },
                layout: WindowLayout::Floating,
                mode: nekoland_ecs::components::WindowMode::Fullscreen,
                x11_window: X11Window { window_id: 7, override_redirect: false },
                ..Default::default()
            },
            OutputBackgroundWindow {
                output: "Virtual-1".to_owned(),
                restore: WindowRestoreState {
                    geometry: WindowSceneGeometry { x: 200, y: 300, width: 640, height: 480 },
                    layout: WindowLayout::Floating,
                    mode: nekoland_ecs::components::WindowMode::Normal,
                },
            },
        ));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let requests = app
            .inner()
            .world()
            .resource::<PendingWindowServerRequests>()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        match &requests[0].action {
            WindowServerAction::SyncPresentation { geometry, fullscreen, maximized } => {
                assert_eq!(
                    *geometry,
                    nekoland_ecs::components::SurfaceGeometry {
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    }
                );
                assert!(*fullscreen);
                assert!(!maximized);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn normal_x11_windows_sync_scene_geometry() {
        let mut app = NekolandApp::new("window-presentation-x11-scene-test");
        app.inner_mut()
            .init_resource::<PendingWindowServerRequests>()
            .add_systems(LayoutSchedule, window_presentation_sync_system);

        app.inner_mut().world_mut().spawn(X11WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: nekoland_ecs::components::SurfaceGeometry {
                x: 10,
                y: 20,
                width: 800,
                height: 600,
            },
            scene_geometry: WindowSceneGeometry { x: 1200, y: 2100, width: 800, height: 600 },
            layout: WindowLayout::Floating,
            mode: nekoland_ecs::components::WindowMode::Normal,
            x11_window: X11Window { window_id: 8, override_redirect: false },
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let requests = app
            .inner()
            .world()
            .resource::<PendingWindowServerRequests>()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        match &requests[0].action {
            WindowServerAction::SyncPresentation { geometry, fullscreen, maximized } => {
                assert_eq!(
                    *geometry,
                    nekoland_ecs::components::SurfaceGeometry {
                        x: 1200,
                        y: 2100,
                        width: 800,
                        height: 600,
                    }
                );
                assert!(!fullscreen);
                assert!(!maximized);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn xdg_windows_ignore_scene_position_and_only_sync_size() {
        let mut app = NekolandApp::new("window-presentation-xdg-size-test");
        app.inner_mut()
            .init_resource::<PendingWindowServerRequests>()
            .add_systems(LayoutSchedule, window_presentation_sync_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 33 },
            geometry: nekoland_ecs::components::SurfaceGeometry {
                x: 50,
                y: 60,
                width: 1024,
                height: 768,
            },
            scene_geometry: WindowSceneGeometry { x: 4000, y: 5000, width: 1024, height: 768 },
            layout: WindowLayout::Floating,
            mode: nekoland_ecs::components::WindowMode::Normal,
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let requests = app
            .inner()
            .world()
            .resource::<PendingWindowServerRequests>()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        match &requests[0].action {
            WindowServerAction::SyncPresentation { geometry, .. } => {
                assert_eq!(
                    *geometry,
                    nekoland_ecs::components::SurfaceGeometry {
                        x: 0,
                        y: 0,
                        width: 1024,
                        height: 768,
                    }
                );
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }
}
