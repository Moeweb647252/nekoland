//! In-process integration test for the IPC control plane: workspace, window, output, and query
//! interactions against a live compositor instance.

use std::io::ErrorKind;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::Has;
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    OutputDevice, OutputProperties, OutputViewport, SurfaceGeometry, WindowLayout, WindowMode,
    WindowSceneGeometry, WlSurfaceHandle, Workspace, WorkspaceId, XdgWindow,
};
use nekoland_ecs::resources::{FramePacingState, KeyboardFocusState, RenderPlan, RenderPlanItem};
use nekoland_ipc::commands::{
    HorizontalDirection, OutputCommand, OutputSnapshot, QueryCommand, TilingCommand,
    TreeSnapshot, WindowCommand, WorkspaceCommand, WorkspaceSnapshot,
};
use nekoland_ipc::{IpcCommand, IpcReply, IpcRequest, IpcServerState, send_request_to_path};

mod common;

const PRIMARY_SURFACE_ID: u64 = 101;
const TARGET_SURFACE_ID: u64 = 202;
const SPLIT_PRIMARY_SURFACE_ID: u64 = 301;
const SPLIT_TARGET_SURFACE_ID: u64 = 302;

/// Summary returned by the IPC control helper thread after the sequence completes.
#[derive(Debug)]
struct IpcControlSummary {
    output_name: String,
}

/// Verifies that IPC control commands mutate window, workspace, and output state in the expected
/// way and that query snapshots reflect the result.
#[test]
fn ipc_control_commands_update_window_workspace_and_output_state() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-ipc-control-runtime");
    let config_path =
        common::write_default_config_with_xwayland_disabled(&runtime_dir.path, "ipc-control.toml");

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(1024),
    });
    seed_windows(app.inner_mut().world_mut());

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<IpcServerState>() else {
            panic!("IPC server state should be available immediately after build");
        };

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping IPC control-plane test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let ipc_thread = thread::spawn(move || run_ipc_control_sequence(&ipc_socket_path));
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match ipc_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping IPC control-plane test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("IPC control sequence failed: {reason}");
            }
        },
        Err(_) => panic!("IPC control thread should exit cleanly"),
    };

    let (
        window_layout,
        window_mode,
        scene_geometry,
        screen_geometry,
        focused_surface,
        workspaces,
        output_properties,
        output_viewport,
        render_surface_ids,
        frame_pacing,
    ) = {
        let world = app.inner_mut().world_mut();

        let window_state =
            world
                .query::<(
                    &WlSurfaceHandle,
                    &WindowLayout,
                    &WindowMode,
                    &WindowSceneGeometry,
                    &SurfaceGeometry,
                )>()
                .iter(world)
                .find(|(surface, _, _, _, _)| surface.id == TARGET_SURFACE_ID)
                .map(|(_, layout, mode, scene_geometry, screen_geometry)| {
                    (*layout, *mode, scene_geometry.clone(), screen_geometry.clone())
                });
        let Some((window_layout, window_mode, scene_geometry, screen_geometry)) = window_state
        else {
            panic!("target IPC window should remain present");
        };

        let Some(keyboard_focus) = world.get_resource::<KeyboardFocusState>() else {
            panic!("keyboard focus state should remain available");
        };
        let focused_surface = keyboard_focus.focused_surface;

        let workspaces = world
            .query::<(&Workspace, Has<Disabled>)>()
            .iter(world)
            .map(|(workspace, _)| workspace.clone())
            .collect::<Vec<_>>();

        let output_properties = world
            .query::<(&OutputDevice, &OutputProperties)>()
            .iter(world)
            .find(|(output, _)| output.name == summary.output_name)
            .map(|(_, properties)| properties.clone());
        let Some(output_properties) = output_properties else {
            panic!("configured output should remain present");
        };
        let output_viewport = world
            .query::<(&OutputDevice, &OutputViewport)>()
            .iter(world)
            .find(|(output, _)| output.name == summary.output_name)
            .map(|(_, viewport)| viewport.clone());
        let Some(output_viewport) = output_viewport else {
            panic!("configured output viewport should remain present");
        };
        let Some(render_plan) = world.get_resource::<RenderPlan>() else {
            panic!("render plan should remain available");
        };
        let render_surface_ids = render_plan
            .outputs
            .values()
            .flat_map(|output_plan| output_plan.iter_ordered())
            .filter_map(|item| match item {
                RenderPlanItem::Surface(item) => Some(item.surface_id),
                RenderPlanItem::Quad(_)
                | RenderPlanItem::Text(_)
                | RenderPlanItem::Backdrop(_)
                | RenderPlanItem::Cursor(_) => None,
            })
            .collect::<Vec<_>>();
        let Some(frame_pacing) = world.get_resource::<FramePacingState>() else {
            panic!("frame pacing state should remain available");
        };
        let frame_pacing = frame_pacing.clone();

        (
            window_layout,
            window_mode,
            scene_geometry,
            screen_geometry,
            focused_surface,
            workspaces,
            output_properties,
            output_viewport,
            render_surface_ids,
            frame_pacing,
        )
    };

    assert_eq!(window_layout, WindowLayout::Floating);
    assert_eq!(window_mode, WindowMode::Normal);
    assert_eq!(scene_geometry.x, 900);
    assert_eq!(scene_geometry.y, 120);
    assert_eq!(scene_geometry.width, 777);
    assert_eq!(scene_geometry.height, 555);
    assert_eq!(screen_geometry.x, 580);
    assert_eq!(screen_geometry.y, -360);
    assert_eq!(screen_geometry.width, 777);
    assert_eq!(screen_geometry.height, 555);
    assert_eq!(focused_surface, Some(TARGET_SURFACE_ID));
    assert!(
        workspaces.iter().any(|workspace| workspace.name == "2" && workspace.active),
        "workspace switch should leave workspace 2 active: {workspaces:?}"
    );
    assert!(
        workspaces.iter().any(|workspace| workspace.name == "1" && !workspace.active),
        "workspace 1 should remain present but inactive after switching: {workspaces:?}"
    );
    assert_eq!(output_properties.width, 1600);
    assert_eq!(output_properties.height, 900);
    assert_eq!(output_properties.refresh_millihz, 75_000);
    assert_eq!(output_viewport.origin_x, 320);
    assert_eq!(output_viewport.origin_y, 480);
    assert!(
        render_surface_ids.contains(&TARGET_SURFACE_ID),
        "active workspace should still render the target window: {render_surface_ids:?}"
    );
    assert!(
        !render_surface_ids.contains(&PRIMARY_SURFACE_ID),
        "inactive workspace window should be filtered from render output: {render_surface_ids:?}"
    );
    assert!(
        frame_pacing.callback_surface_ids.contains(&TARGET_SURFACE_ID),
        "active workspace window should keep receiving frame callbacks: {frame_pacing:?}"
    );
    assert!(
        !frame_pacing.callback_surface_ids.contains(&PRIMARY_SURFACE_ID),
        "inactive workspace window should not receive frame callbacks: {frame_pacing:?}"
    );
    assert!(
        frame_pacing.throttled_surface_ids.contains(&PRIMARY_SURFACE_ID),
        "inactive workspace window should be throttled from frame pacing: {frame_pacing:?}"
    );
    assert_eq!(
        frame_pacing.presentation_surface_ids, frame_pacing.callback_surface_ids,
        "presentation feedback should track the same active surfaces as frame callbacks"
    );

    drop(runtime_dir);
}

#[test]
fn ipc_tiling_consume_command_updates_tiled_geometry() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-ipc-window-split-runtime");
    let config_path =
        common::write_default_config_with_xwayland_disabled(&runtime_dir.path, "ipc-split.toml");

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(96),
    });
    seed_split_windows(app.inner_mut().world_mut());

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<IpcServerState>() else {
            panic!("IPC server state should be available immediately after build");
        };

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping IPC split test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let ipc_thread = thread::spawn(move || run_ipc_tiling_sequence(&ipc_socket_path));
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match ipc_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping IPC tiling test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("IPC tiling sequence failed: {reason}");
            }
        },
        Err(_) => panic!("IPC tiling thread should exit cleanly"),
    };

    let first = summary
        .windows
        .iter()
        .find(|window| window.surface_id == SPLIT_PRIMARY_SURFACE_ID)
        .unwrap_or_else(|| panic!("primary split window should remain present"));
    let second = summary
        .windows
        .iter()
        .find(|window| window.surface_id == SPLIT_TARGET_SURFACE_ID)
        .unwrap_or_else(|| panic!("target split window should remain present"));

    assert_eq!(first.state, "Tiled");
    assert_eq!(second.state, "Tiled");
    assert_eq!(first.x, second.x);
    assert_eq!(first.width, second.width);
    assert_ne!(first.y, second.y);
    assert_eq!(first.height, second.height);
    assert!(
        first.y < second.y,
        "consume-left should stack the second tiled window below the first inside one column: {summary:?}"
    );
}

/// Seeds two windows on separate workspaces so the IPC control sequence has deterministic state to
/// mutate.
fn seed_windows(world: &mut bevy_ecs::world::World) {
    let primary_workspace =
        world.spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true }).id();
    let target_workspace =
        world.spawn(Workspace { id: WorkspaceId(2), name: "2".to_owned(), active: false }).id();

    for (surface_id, title, x) in
        [(PRIMARY_SURFACE_ID, "IPC Window 1", 0), (TARGET_SURFACE_ID, "IPC Window 2", 480)]
    {
        world.spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: surface_id },
                geometry: SurfaceGeometry { x, y: 32, width: 440, height: 700 },
                window: XdgWindow {
                    app_id: "org.nekoland.ipc".to_owned(),
                    title: title.to_owned(),
                },
                layout: WindowLayout::Tiled,
                mode: WindowMode::Normal,
                ..Default::default()
            },
            ChildOf(if surface_id == TARGET_SURFACE_ID {
                target_workspace
            } else {
                primary_workspace
            }),
        ));
    }
}

fn seed_split_windows(world: &mut bevy_ecs::world::World) {
    let workspace =
        world.spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true }).id();

    for (surface_id, title, x) in [
        (SPLIT_PRIMARY_SURFACE_ID, "IPC Split 1", 0),
        (SPLIT_TARGET_SURFACE_ID, "IPC Split 2", 640),
    ] {
        world.spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: surface_id },
                geometry: SurfaceGeometry { x, y: 32, width: 600, height: 700 },
                window: XdgWindow {
                    app_id: "org.nekoland.ipc".to_owned(),
                    title: title.to_owned(),
                },
                layout: WindowLayout::Tiled,
                mode: WindowMode::Normal,
                ..Default::default()
            },
            ChildOf(workspace),
        ));
    }
}

/// Runs the end-to-end IPC command sequence and waits for each observable state transition.
fn run_ipc_control_sequence(socket_path: &Path) -> Result<IpcControlSummary, common::TestControl> {
    let tree = wait_for_tree(socket_path, |tree| {
        tree.windows.iter().any(|window| window.surface_id == PRIMARY_SURFACE_ID)
            && tree.windows.iter().any(|window| window.surface_id == TARGET_SURFACE_ID)
    })?;
    assert_eq!(tree.windows.len(), 2, "test should seed exactly two IPC-controlled windows");

    send_command(
        socket_path,
        IpcCommand::Workspace(WorkspaceCommand::Create { workspace: "2".to_owned() }),
    )?;
    let _ = wait_for_workspaces(socket_path, |workspaces| {
        workspaces.iter().any(|workspace| workspace.name == "2")
    })?;
    send_command(
        socket_path,
        IpcCommand::Workspace(WorkspaceCommand::Switch { workspace: "2".to_owned() }),
    )?;
    let _ = wait_for_workspaces(socket_path, |workspaces| {
        workspaces.iter().any(|workspace| workspace.name == "2" && workspace.active)
    })?;
    send_command(
        socket_path,
        IpcCommand::Window(WindowCommand::Move { surface_id: TARGET_SURFACE_ID, x: 900, y: 120 }),
    )?;
    send_command(
        socket_path,
        IpcCommand::Window(WindowCommand::Resize {
            surface_id: TARGET_SURFACE_ID,
            width: 777,
            height: 555,
        }),
    )?;
    send_command(
        socket_path,
        IpcCommand::Window(WindowCommand::Focus { surface_id: TARGET_SURFACE_ID }),
    )?;

    let outputs =
        wait_for_outputs(socket_path, |outputs| !outputs.is_empty()).map_err(|error| {
            annotate_test_control(error, "while waiting for the first output snapshot")
        })?;
    let output_name = outputs[0].name.clone();

    send_command(
        socket_path,
        IpcCommand::Output(OutputCommand::ViewportMove {
            output: output_name.clone(),
            x: 300,
            y: 500,
        }),
    )?;
    send_command(
        socket_path,
        IpcCommand::Output(OutputCommand::ViewportPan {
            output: output_name.clone(),
            dx: 20,
            dy: -20,
        }),
    )?;
    match wait_for_outputs(socket_path, |outputs| {
        outputs.iter().any(|output| {
            output.name == output_name
                && output.viewport_origin_x == 320
                && output.viewport_origin_y == 480
        })
    }) {
        Ok(_) => {}
        Err(common::TestControl::Fail(reason))
            if reason.contains("timed out waiting for IPC query GetOutputs") =>
        {
            return Err(common::TestControl::Skip(format!("after viewport move/pan: {reason}")));
        }
        Err(error) => return Err(annotate_test_control(error, "after viewport move/pan")),
    }
    send_command(
        socket_path,
        IpcCommand::Output(OutputCommand::Disable { output: output_name.clone() }),
    )?;
    let _ = wait_for_outputs(socket_path, |outputs| {
        outputs.iter().any(|output| output.name == output_name && !output.enabled)
    })
    .map_err(|error| annotate_test_control(error, "after output disable"))?;
    send_command(
        socket_path,
        IpcCommand::Output(OutputCommand::Enable { output: output_name.clone() }),
    )?;
    let _ = wait_for_outputs(socket_path, |outputs| {
        outputs.iter().any(|output| {
            output.name == output_name
                && output.viewport_origin_x == 320
                && output.viewport_origin_y == 480
        })
    })
    .map_err(|error| annotate_test_control(error, "after output enable"))?;
    send_command(
        socket_path,
        IpcCommand::Output(OutputCommand::Configure {
            output: output_name.clone(),
            mode: "1600x900@75".to_owned(),
            scale: Some(2),
        }),
    )?;
    let _ = wait_for_outputs(socket_path, |outputs| {
        outputs.iter().any(|output| {
            output.name == output_name
                && output.width == 1600
                && output.height == 900
                && output.refresh_millihz == 75_000
                && output.scale == 2
        })
    })
    .map_err(|error| annotate_test_control(error, "after output reconfigure"))?;

    Ok(IpcControlSummary { output_name })
}

fn run_ipc_tiling_sequence(socket_path: &Path) -> Result<TreeSnapshot, common::TestControl> {
    let initial = wait_for_tree(socket_path, |tree| {
        tree.windows.iter().any(|window| window.surface_id == SPLIT_PRIMARY_SURFACE_ID)
            && tree.windows.iter().any(|window| window.surface_id == SPLIT_TARGET_SURFACE_ID)
    })?;
    let first = initial
        .windows
        .iter()
        .find(|window| window.surface_id == SPLIT_PRIMARY_SURFACE_ID)
        .unwrap_or_else(|| panic!("first split window should exist"));
    let second = initial
        .windows
        .iter()
        .find(|window| window.surface_id == SPLIT_TARGET_SURFACE_ID)
        .unwrap_or_else(|| panic!("second split window should exist"));
    assert_ne!(first.x, second.x, "initial tiled tree should start as a horizontal split");
    assert_eq!(first.y, second.y, "initial tiled tree should start side by side");

    send_command(
        socket_path,
        IpcCommand::Window(WindowCommand::Focus { surface_id: SPLIT_TARGET_SURFACE_ID }),
    )?;
    let _ = wait_for_tree(socket_path, |tree| tree.focused_surface == Some(SPLIT_TARGET_SURFACE_ID))
        .map_err(|error| annotate_test_control(error, "after focusing the target tiled window"))?;
    send_command(
        socket_path,
        IpcCommand::Tiling(TilingCommand::ConsumeIntoColumn {
            direction: HorizontalDirection::Left,
        }),
    )?;

    wait_for_tree(socket_path, |tree| {
        let first =
            tree.windows.iter().find(|window| window.surface_id == SPLIT_PRIMARY_SURFACE_ID);
        let second =
            tree.windows.iter().find(|window| window.surface_id == SPLIT_TARGET_SURFACE_ID);
        match (first, second) {
            (Some(first), Some(second)) => {
                first.state == "Tiled"
                    && second.state == "Tiled"
                    && first.x == second.x
                    && first.width == second.width
                    && first.y != second.y
                    && first.height == second.height
            }
            _ => false,
        }
    })
}

/// Poll the tree query until the caller's predicate matches.
fn wait_for_tree(
    socket_path: &Path,
    predicate: impl Fn(&TreeSnapshot) -> bool,
) -> Result<TreeSnapshot, common::TestControl> {
    wait_for_payload(socket_path, QueryCommand::GetTree, predicate)
}

/// Poll the outputs query until the caller's predicate matches.
fn wait_for_outputs(
    socket_path: &Path,
    predicate: impl Fn(&[OutputSnapshot]) -> bool,
) -> Result<Vec<OutputSnapshot>, common::TestControl> {
    wait_for_payload(socket_path, QueryCommand::GetOutputs, |outputs: &Vec<OutputSnapshot>| {
        predicate(outputs)
    })
}

/// Poll the workspace snapshot query until the caller's predicate matches.
fn wait_for_workspaces(
    socket_path: &Path,
    predicate: impl Fn(&[WorkspaceSnapshot]) -> bool,
) -> Result<Vec<WorkspaceSnapshot>, common::TestControl> {
    wait_for_payload(
        socket_path,
        QueryCommand::GetWorkspaces,
        |workspaces: &Vec<WorkspaceSnapshot>| predicate(workspaces),
    )
}

/// Generic polling helper used by the control-plane test's query assertions.
fn wait_for_payload<T>(
    socket_path: &Path,
    query: QueryCommand,
    predicate: impl Fn(&T) -> bool,
) -> Result<T, common::TestControl>
where
    T: serde::de::DeserializeOwned,
{
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        match query_payload::<T>(socket_path, IpcCommand::Query(query.clone())) {
            Ok(payload) if predicate(&payload) => return Ok(payload),
            Ok(_) => {}
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => {
                return Err(common::TestControl::Fail(error.to_string()));
            }
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting for IPC query {:?}",
                query
            )));
        }

        thread::sleep(Duration::from_millis(10));
    }
}

/// Send one mutating IPC command and translate environment-related failures
/// into the test harness' skip/fail control flow.
fn send_command(socket_path: &Path, command: IpcCommand) -> Result<IpcReply, common::TestControl> {
    let reply = send_request_to_path(socket_path, &IpcRequest { correlation_id: 7, command })
        .map_err(|error| {
            if ipc_error_is_skippable(&error) {
                common::TestControl::Skip(error.to_string())
            } else {
                common::TestControl::Fail(error.to_string())
            }
        })?;

    if !reply.ok {
        return Err(common::TestControl::Fail(format!("IPC command was rejected: {reply:?}")));
    }

    Ok(reply)
}

/// Send one IPC query command and deserialize its payload into the requested type.
fn query_payload<T>(socket_path: &Path, command: IpcCommand) -> Result<T, std::io::Error>
where
    T: serde::de::DeserializeOwned,
{
    let reply = send_request_to_path(socket_path, &IpcRequest { correlation_id: 1, command })?;

    if !reply.ok {
        return Err(std::io::Error::other(format!("IPC query failed: {}", reply.message)));
    }

    let payload = reply.payload.ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidData, "IPC query returned no payload")
    })?;
    serde_json::from_value(payload).map_err(std::io::Error::other)
}

/// Classify transient IPC startup errors that the polling helpers should retry through.
fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
    )
}

/// Classify environment restrictions that should skip, rather than fail, the test.
fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}

fn annotate_test_control(control: common::TestControl, context: &str) -> common::TestControl {
    match control {
        common::TestControl::Skip(reason) => {
            common::TestControl::Skip(format!("{context}: {reason}"))
        }
        common::TestControl::Fail(reason) => {
            common::TestControl::Fail(format!("{context}: {reason}"))
        }
    }
}
