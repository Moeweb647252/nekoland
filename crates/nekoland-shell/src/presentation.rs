use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::{Local, Query, ResMut, With};
use nekoland_ecs::components::{SurfaceGeometry, Window, WindowMode, WindowSceneGeometry};
use nekoland_ecs::resources::{
    PendingWindowServerRequests, WindowServerAction, WindowServerRequest,
};
use nekoland_ecs::views::WindowRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WindowPresentationState {
    Sync {
        geometry: SurfaceGeometry,
        scene_geometry: Option<WindowSceneGeometry>,
        fullscreen: bool,
        maximized: bool,
        resizing: bool,
    },
}

pub fn window_presentation_sync_system(
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
    mut windows: Query<WindowRuntime, With<Window>>,
    mut synced: Local<BTreeMap<u64, WindowPresentationState>>,
) {
    let mut seen_surfaces = BTreeSet::new();

    for mut window in &mut windows {
        let surface_id = window.surface_id();
        seen_surfaces.insert(surface_id);

        let desired = desired_presentation_state(&mut window);
        if synced.get(&surface_id) == desired.as_ref() {
            continue;
        }

        if let Some(desired) = desired {
            if let Some(mut pending_resize) = window.pending_resize
                && pending_resize.inflight_geometry.is_none()
            {
                pending_resize.inflight_geometry = Some(pending_resize.requested_geometry.clone());
            }
            pending_window_requests.push(WindowServerRequest {
                surface_id,
                action: window_server_action_for_presentation_state(&desired),
            });
            synced.insert(surface_id, desired);
        } else if synced.remove(&surface_id).is_some() {
            pending_window_requests.push(WindowServerRequest {
                surface_id,
                action: WindowServerAction::SyncPresentation {
                    geometry: window.geometry.clone(),
                    scene_geometry: None,
                    fullscreen: false,
                    maximized: false,
                    resizing: false,
                },
            });
        }
    }

    synced.retain(|surface_id, _| seen_surfaces.contains(surface_id));
}

fn desired_presentation_state(
    window: &mut nekoland_ecs::views::WindowRuntimeItem<'_, '_>,
) -> Option<WindowPresentationState> {
    let fullscreen = window.role.is_output_background() || *window.mode == WindowMode::Fullscreen;
    let maximized = window.role.is_managed() && *window.mode == WindowMode::Maximized;
    let resizing = window.pending_resize.is_some();
    if fullscreen || maximized {
        Some(WindowPresentationState::Sync {
            geometry: window.geometry.clone(),
            scene_geometry: Some(window.scene_geometry.clone()),
            fullscreen,
            maximized,
            resizing,
        })
    } else if window.pending_resize.is_some() {
        let pending_resize = window.pending_resize.as_deref().expect("checked above");
        let target_geometry =
            pending_resize.inflight_geometry.as_ref().unwrap_or(&pending_resize.requested_geometry);
        Some(WindowPresentationState::Sync {
            geometry: SurfaceGeometry {
                x: window.geometry.x,
                y: window.geometry.y,
                width: target_geometry.width.max(1),
                height: target_geometry.height.max(1),
            },
            scene_geometry: Some(target_geometry.clone()),
            fullscreen: false,
            maximized: false,
            resizing: true,
        })
    } else if window.placement.floating_size.is_some() {
        Some(WindowPresentationState::Sync {
            geometry: window.geometry.clone(),
            scene_geometry: Some(window.scene_geometry.clone()),
            fullscreen: false,
            maximized: false,
            resizing: false,
        })
    } else if window.management_hints.client_driven_resize {
        (window.committed_size.width > 0
            && window.committed_size.height > 0
            && (window.committed_size.width != window.scene_geometry.width
                || window.committed_size.height != window.scene_geometry.height))
            .then(|| WindowPresentationState::Sync {
                geometry: window.geometry.clone(),
                scene_geometry: Some(window.scene_geometry.clone()),
                fullscreen: false,
                maximized: false,
                resizing: false,
            })
    } else {
        Some(WindowPresentationState::Sync {
            geometry: window.geometry.clone(),
            scene_geometry: Some(window.scene_geometry.clone()),
            fullscreen: false,
            maximized: false,
            resizing: false,
        })
    }
}

fn window_server_action_for_presentation_state(
    state: &WindowPresentationState,
) -> WindowServerAction {
    match state {
        WindowPresentationState::Sync {
            geometry,
            scene_geometry,
            fullscreen,
            maximized,
            resizing,
        } => WindowServerAction::SyncPresentation {
            geometry: geometry.clone(),
            scene_geometry: scene_geometry.clone(),
            fullscreen: *fullscreen,
            maximized: *maximized,
            resizing: *resizing,
        },
    }
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{WindowBundle, X11WindowBundle};
    use nekoland_ecs::components::{
        OutputBackgroundWindow, OutputId, PendingInteractiveResize, WindowCommittedSize,
        WindowLayout, WindowPlacement, WindowRestoreState, WindowRole, WindowSceneGeometry,
        WindowSize, WlSurfaceHandle, X11Window,
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
                x11_window: X11Window {
                    window_id: 7,
                    override_redirect: false,
                    popup: false,
                    transient_for: None,
                    window_type: None,
                },
                ..Default::default()
            },
            WindowRole::OutputBackground,
            OutputBackgroundWindow {
                output: OutputId(7),
                restore: WindowRestoreState {
                    geometry: WindowSceneGeometry { x: 200, y: 300, width: 640, height: 480 },
                    layout: WindowLayout::Floating,
                    mode: nekoland_ecs::components::WindowMode::Normal,
                    fullscreen_output: None,
                    previous: None,
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
            WindowServerAction::SyncPresentation {
                geometry,
                scene_geometry,
                fullscreen,
                maximized,
                resizing,
            } => {
                assert_eq!(
                    geometry,
                    &nekoland_ecs::components::SurfaceGeometry {
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    }
                );
                assert_eq!(
                    *scene_geometry,
                    Some(WindowSceneGeometry { x: 200, y: 300, width: 640, height: 480 })
                );
                assert!(*fullscreen);
                assert!(!maximized);
                assert!(!resizing);
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
            x11_window: X11Window {
                window_id: 8,
                override_redirect: false,
                popup: false,
                transient_for: None,
                window_type: None,
            },
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
            WindowServerAction::SyncPresentation {
                geometry,
                scene_geometry,
                fullscreen,
                maximized,
                resizing,
            } => {
                assert_eq!(
                    geometry,
                    &nekoland_ecs::components::SurfaceGeometry {
                        x: 10,
                        y: 20,
                        width: 800,
                        height: 600,
                    }
                );
                assert_eq!(
                    *scene_geometry,
                    Some(WindowSceneGeometry { x: 1200, y: 2100, width: 800, height: 600 })
                );
                assert!(!fullscreen);
                assert!(!maximized);
                assert!(!resizing);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn normal_xdg_windows_do_not_emit_presentation_sync() {
        let mut app = NekolandApp::new("window-presentation-xdg-normal-test");
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
        assert!(requests.is_empty());
    }

    #[test]
    fn fullscreen_xdg_windows_sync_state_without_geometry() {
        let mut app = NekolandApp::new("window-presentation-xdg-fullscreen-test");
        app.inner_mut()
            .init_resource::<PendingWindowServerRequests>()
            .add_systems(LayoutSchedule, window_presentation_sync_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 44 },
            geometry: nekoland_ecs::components::SurfaceGeometry {
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
            },
            scene_geometry: WindowSceneGeometry { x: 4000, y: 5000, width: 1024, height: 768 },
            layout: WindowLayout::Floating,
            mode: nekoland_ecs::components::WindowMode::Fullscreen,
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
            WindowServerAction::SyncPresentation {
                geometry,
                scene_geometry,
                fullscreen,
                maximized,
                resizing,
            } => {
                assert_eq!(
                    geometry,
                    &nekoland_ecs::components::SurfaceGeometry {
                        x: 0,
                        y: 0,
                        width: 1280,
                        height: 720,
                    }
                );
                assert_eq!(
                    *scene_geometry,
                    Some(WindowSceneGeometry { x: 4000, y: 5000, width: 1024, height: 768 })
                );
                assert!(*fullscreen);
                assert!(!maximized);
                assert!(!resizing);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn resized_floating_xdg_windows_sync_explicit_size() {
        let mut app = NekolandApp::new("window-presentation-xdg-resize-test");
        app.inner_mut()
            .init_resource::<PendingWindowServerRequests>()
            .add_systems(LayoutSchedule, window_presentation_sync_system);

        app.inner_mut().world_mut().spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 55 },
                geometry: nekoland_ecs::components::SurfaceGeometry {
                    x: 50,
                    y: 60,
                    width: 1024,
                    height: 768,
                },
                scene_geometry: WindowSceneGeometry { x: 4000, y: 5000, width: 1440, height: 900 },
                layout: WindowLayout::Floating,
                mode: nekoland_ecs::components::WindowMode::Normal,
                ..Default::default()
            },
            WindowPlacement {
                floating_size: Some(WindowSize { width: 1440, height: 900 }),
                ..WindowPlacement::default()
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
            WindowServerAction::SyncPresentation {
                geometry,
                scene_geometry,
                fullscreen,
                maximized,
                resizing,
            } => {
                assert_eq!(
                    geometry,
                    &nekoland_ecs::components::SurfaceGeometry {
                        x: 50,
                        y: 60,
                        width: 1024,
                        height: 768,
                    }
                );
                assert_eq!(
                    *scene_geometry,
                    Some(WindowSceneGeometry { x: 4000, y: 5000, width: 1440, height: 900 })
                );
                assert!(!fullscreen);
                assert!(!maximized);
                assert!(!resizing);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn client_driven_xdg_windows_sync_when_scene_size_differs_from_committed_size() {
        let mut app = NekolandApp::new("window-presentation-xdg-initial-fit-test");
        app.inner_mut()
            .init_resource::<PendingWindowServerRequests>()
            .add_systems(LayoutSchedule, window_presentation_sync_system);

        app.inner_mut().world_mut().spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 56 },
                geometry: nekoland_ecs::components::SurfaceGeometry {
                    x: 50,
                    y: 60,
                    width: 1280,
                    height: 720,
                },
                scene_geometry: WindowSceneGeometry { x: 4000, y: 5000, width: 1280, height: 720 },
                layout: WindowLayout::Floating,
                mode: nekoland_ecs::components::WindowMode::Normal,
                ..Default::default()
            },
            WindowCommittedSize { width: 1600, height: 900 },
            WindowPlacement {
                floating_position: Some(nekoland_ecs::components::FloatingPosition::Auto(
                    nekoland_ecs::components::WindowPosition { x: 4000, y: 5000 },
                )),
                ..WindowPlacement::default()
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
            WindowServerAction::SyncPresentation {
                geometry,
                scene_geometry,
                fullscreen,
                maximized,
                resizing,
            } => {
                assert_eq!(geometry.width, 1280);
                assert_eq!(geometry.height, 720);
                assert_eq!(
                    *scene_geometry,
                    Some(WindowSceneGeometry { x: 4000, y: 5000, width: 1280, height: 720 })
                );
                assert!(!fullscreen);
                assert!(!maximized);
                assert!(!resizing);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }

    #[test]
    fn pending_interactive_resize_prefers_requested_size_over_committed_geometry() {
        let mut app = NekolandApp::new("window-presentation-xdg-pending-resize-test");
        app.inner_mut()
            .init_resource::<PendingWindowServerRequests>()
            .add_systems(LayoutSchedule, window_presentation_sync_system);

        app.inner_mut().world_mut().spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 66 },
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
            },
            PendingInteractiveResize {
                requested_geometry: WindowSceneGeometry {
                    x: 4010,
                    y: 5010,
                    width: 1440,
                    height: 900,
                },
                inflight_geometry: None,
                edges: nekoland_ecs::resources::ResizeEdges::BottomRight,
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
            WindowServerAction::SyncPresentation {
                geometry,
                scene_geometry,
                fullscreen,
                maximized,
                resizing,
            } => {
                assert_eq!(
                    geometry,
                    &nekoland_ecs::components::SurfaceGeometry {
                        x: 50,
                        y: 60,
                        width: 1440,
                        height: 900,
                    }
                );
                assert_eq!(
                    *scene_geometry,
                    Some(WindowSceneGeometry { x: 4010, y: 5010, width: 1440, height: 900 })
                );
                assert!(!fullscreen);
                assert!(!maximized);
                assert!(*resizing);
            }
            action => panic!("unexpected action: {action:?}"),
        }
    }
}
