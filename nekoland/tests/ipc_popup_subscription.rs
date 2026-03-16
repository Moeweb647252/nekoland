//! In-process integration test for popup geometry/grab subscription events.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Query, Res, ResMut, Resource, With};
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_core::schedules::LayoutSchedule;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    PopupGrab, SurfaceGeometry, WindowLayout, WindowMode, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::CompositorClock;
use nekoland_ipc::{
    IpcServerState, IpcSubscription, PopupGeometryChangeSnapshot, PopupGrabChangeSnapshot,
    SubscriptionTopic, subscribe_to_path,
};

mod common;

/// Surface id of the parent toplevel window.
const PARENT_SURFACE_ID: u64 = 101;
/// Surface id of the popup child whose changes are observed over IPC.
const POPUP_SURFACE_ID: u64 = 202;

/// Planned popup mutation applied by the test system once the scenario starts.
#[derive(Debug, Clone, Resource)]
struct PopupMutationPlan {
    /// Target x coordinate written by the mutation system.
    target_x: i32,
    /// Target y coordinate written by the mutation system.
    target_y: i32,
    /// Target width written by the mutation system.
    target_width: u32,
    /// Target height written by the mutation system.
    target_height: u32,
    /// Target popup-grab active flag written by the mutation system.
    target_grab_active: bool,
    /// Target popup-grab serial written by the mutation system.
    target_grab_serial: Option<u32>,
    /// Armed after the subscription thread has had time to connect.
    ready: Arc<AtomicBool>,
    /// Set once the mutation has been applied so it only runs once.
    applied: bool,
}

/// Pair of popup subscription events that the scenario waits for.
#[derive(Debug)]
struct PopupChangeEvents {
    geometry: PopupGeometryChangeSnapshot,
    grab: PopupGrabChangeSnapshot,
}

/// Verifies that popup geometry and grab mutations are surfaced through popup subscription events.
#[test]
fn popup_subscription_reports_geometry_and_grab_transitions() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _runtime_dir = common::RuntimeDirGuard::new("nekoland-popup-subscription");
    let config_path = workspace_config_path();
    let mutation_ready = Arc::new(AtomicBool::new(false));

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(160),
    });
    app.insert_resource(PopupMutationPlan {
        target_x: 96,
        target_y: 124,
        target_width: 320,
        target_height: 180,
        target_grab_active: true,
        target_grab_serial: Some(77),
        ready: mutation_ready.clone(),
        applied: false,
    });
    app.inner_mut().add_systems(LayoutSchedule, apply_popup_mutation_system);
    seed_popup_tree(app.inner_mut().world_mut());

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<IpcServerState>() else {
            panic!("IPC server state should be available immediately after build");
        };

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping popup subscription test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let subscription_path = ipc_socket_path.clone();
    let subscription_thread = thread::spawn(move || {
        wait_for_popup_change_events(
            &subscription_path,
            IpcSubscription {
                topic: SubscriptionTopic::Popup,
                include_payloads: true,
                events: vec!["popup_geometry_changed".to_owned(), "popup_grab_changed".to_owned()],
            },
            POPUP_SURFACE_ID,
        )
    });
    let mutation_arm_thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        mutation_ready.store(true, Ordering::SeqCst);
    });

    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }
    if mutation_arm_thread.join().is_err() {
        panic!("popup mutation arm thread should exit cleanly");
    }

    let events = match subscription_thread.join() {
        Ok(result) => match result {
            Ok(events) => events,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping popup subscription test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("popup subscription failed: {reason}");
            }
        },
        Err(_) => panic!("popup subscription thread should exit cleanly"),
    };

    assert_eq!(events.geometry.surface_id, POPUP_SURFACE_ID);
    assert_eq!(events.geometry.parent_surface_id, PARENT_SURFACE_ID);
    assert_eq!(events.geometry.previous_x, 24);
    assert_eq!(events.geometry.previous_y, 48);
    assert_eq!(events.geometry.previous_width, 220);
    assert_eq!(events.geometry.previous_height, 120);
    assert_eq!(events.geometry.x, 96);
    assert_eq!(events.geometry.y, 124);
    assert_eq!(events.geometry.width, 320);
    assert_eq!(events.geometry.height, 180);

    assert_eq!(events.grab.surface_id, POPUP_SURFACE_ID);
    assert_eq!(events.grab.parent_surface_id, PARENT_SURFACE_ID);
    assert!(!events.grab.previous_grab_active);
    assert_eq!(events.grab.previous_grab_serial, None);
    assert!(events.grab.grab_active);
    assert_eq!(events.grab.grab_serial, Some(77));
}

/// Seeds one parent toplevel and one popup child for the subscription scenario.
fn seed_popup_tree(world: &mut bevy_ecs::world::World) {
    let parent = world
        .spawn((WindowBundle {
            surface: WlSurfaceHandle { id: PARENT_SURFACE_ID },
            geometry: SurfaceGeometry { x: 0, y: 32, width: 640, height: 480 },
            window: XdgWindow {
                app_id: "org.nekoland.popup-subscription".to_owned(),
                title: "Popup Parent".to_owned(),
                last_acked_configure: None,
            },
            layout: WindowLayout::Tiled,
            mode: WindowMode::Normal,
            ..Default::default()
        },))
        .id();

    world.spawn((
        WlSurfaceHandle { id: POPUP_SURFACE_ID },
        XdgPopup {
            configure_serial: Some(1),
            grab_serial: None,
            reposition_token: None,
            placement_x: 24,
            placement_y: 48,
            placement_width: 220,
            placement_height: 120,
        },
        SurfaceGeometry { x: 24, y: 48, width: 220, height: 120 },
        PopupGrab { active: false, seat_name: "seat-0".to_owned(), serial: None },
        ChildOf(parent),
    ));
}

/// Returns the default config path used by this integration test.
fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

/// Applies the planned popup mutation once the compositor has advanced a few frames.
fn apply_popup_mutation_system(
    clock: Res<CompositorClock>,
    mut plan: ResMut<PopupMutationPlan>,
    mut popups: Query<
        (&WlSurfaceHandle, &mut SurfaceGeometry, &mut XdgPopup, &mut PopupGrab),
        With<XdgPopup>,
    >,
) {
    if plan.applied || clock.frame < 2 || !plan.ready.load(Ordering::SeqCst) {
        return;
    }

    let Some((_, mut geometry, mut popup, mut grab)) =
        popups.iter_mut().find(|(surface, ..)| surface.id == POPUP_SURFACE_ID)
    else {
        return;
    };

    popup.placement_x = plan.target_x;
    popup.placement_y = plan.target_y;
    popup.placement_width = plan.target_width;
    popup.placement_height = plan.target_height;
    geometry.x = plan.target_x;
    geometry.y = plan.target_y;
    geometry.width = plan.target_width;
    geometry.height = plan.target_height;
    grab.active = plan.target_grab_active;
    grab.serial = plan.target_grab_serial;
    plan.applied = true;
}

/// Waits for both popup geometry and popup grab change events targeting the expected popup.
fn wait_for_popup_change_events(
    socket_path: &Path,
    subscription: IpcSubscription,
    expected_surface: u64,
) -> Result<PopupChangeEvents, common::TestControl> {
    let mut stream = subscribe_to_path(socket_path, &subscription).map_err(classify_ipc_error)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut geometry = None;
    let mut grab = None;

    loop {
        match stream.read_event() {
            Ok(event) => {
                let Some(payload) = event.payload else {
                    continue;
                };

                // The popup topic can interleave geometry and grab updates, so
                // hold onto whichever side arrives first until both are present.
                match event.event.as_str() {
                    "popup_geometry_changed" => {
                        let change = serde_json::from_value::<PopupGeometryChangeSnapshot>(payload)
                            .map_err(|error| {
                                common::TestControl::Fail(format!(
                                    "failed to decode popup_geometry_changed payload: {error}"
                                ))
                            })?;
                        if change.surface_id == expected_surface {
                            geometry = Some(change);
                        }
                    }
                    "popup_grab_changed" => {
                        let change = serde_json::from_value::<PopupGrabChangeSnapshot>(payload)
                            .map_err(|error| {
                                common::TestControl::Fail(format!(
                                    "failed to decode popup_grab_changed payload: {error}"
                                ))
                            })?;
                        if change.surface_id == expected_surface {
                            grab = Some(change);
                        }
                    }
                    _ => {}
                }

                if let (Some(geometry), Some(grab)) = (geometry.clone(), grab.clone()) {
                    return Ok(PopupChangeEvents { geometry, grab });
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) => return Err(classify_ipc_error(error)),
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for popup change subscription events".to_owned(),
            ));
        }
    }
}

/// Maps IPC failures into the test's skip/fail control flow.
fn classify_ipc_error(error: std::io::Error) -> common::TestControl {
    if ipc_error_is_skippable(&error) {
        return common::TestControl::Skip(error.to_string());
    }

    common::TestControl::Fail(error.to_string())
}

/// Identifies retryable transient IPC errors.
fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
    )
}

/// Identifies IPC errors that should skip the test in restricted environments.
fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}
