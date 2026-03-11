use std::io::ErrorKind;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    LayoutSlot, OutputDevice, OutputProperties, SurfaceGeometry, WindowState, WlSurfaceHandle,
    Workspace, XdgWindow,
};
use nekoland_ecs::resources::{FramePacingState, KeyboardFocusState, RenderList};
use nekoland_ipc::commands::{
    ConfigSnapshot, OutputCommand, OutputSnapshot, QueryCommand, TreeSnapshot, WindowCommand,
    WorkspaceCommand, WorkspaceSnapshot,
};
use nekoland_ipc::{IpcCommand, IpcReply, IpcRequest, IpcServerState, send_request_to_path};

mod common;

const PRIMARY_SURFACE_ID: u64 = 101;
const TARGET_SURFACE_ID: u64 = 202;

#[derive(Debug)]
struct IpcControlSummary {
    output_name: String,
    config: ConfigSnapshot,
}

#[test]
fn ipc_control_commands_update_window_workspace_and_output_state() {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-ipc-control-runtime");
    let config_path =
        common::write_default_config_with_xwayland_disabled(&runtime_dir.path, "ipc-control.toml");

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(96),
    });
    seed_windows(app.inner_mut().world_mut());

    let ipc_socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<IpcServerState>()
            .expect("IPC server state should be available immediately after build");

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
    app.run().expect("nekoland app should complete the configured frame budget");

    let summary = match ipc_thread.join().expect("IPC control thread should exit cleanly") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping IPC control-plane test in restricted environment: {reason}");
            return;
        }
        Err(common::TestControl::Fail(reason)) => {
            panic!("IPC control sequence failed: {reason}");
        }
    };

    let (
        window_state,
        geometry,
        focused_surface,
        workspaces,
        output_properties,
        render_list,
        frame_pacing,
    ) = {
        let world = app.inner_mut().world_mut();

        let (window_state, geometry) = world
            .query::<(&WlSurfaceHandle, &WindowState, &SurfaceGeometry)>()
            .iter(world)
            .find(|(surface, _, _)| surface.id == TARGET_SURFACE_ID)
            .map(|(_, state, geometry)| (state.clone(), geometry.clone()))
            .expect("target IPC window should remain present");

        let focused_surface = world
            .get_resource::<KeyboardFocusState>()
            .expect("keyboard focus state should remain available")
            .focused_surface;

        let workspaces = world.query::<&Workspace>().iter(world).cloned().collect::<Vec<_>>();

        let output_properties = world
            .query::<(&OutputDevice, &OutputProperties)>()
            .iter(world)
            .find(|(output, _)| output.name == summary.output_name)
            .map(|(_, properties)| properties.clone())
            .expect("configured output should remain present");
        let render_list = world
            .get_resource::<RenderList>()
            .expect("render list should remain available")
            .elements
            .clone();
        let frame_pacing = world
            .get_resource::<FramePacingState>()
            .expect("frame pacing state should remain available")
            .clone();

        (
            window_state,
            geometry,
            focused_surface,
            workspaces,
            output_properties,
            render_list,
            frame_pacing,
        )
    };

    assert_eq!(window_state, WindowState::Floating);
    assert_eq!(geometry.x, 900);
    assert_eq!(geometry.y, 120);
    assert_eq!(geometry.width, 777);
    assert_eq!(geometry.height, 555);
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
    assert_eq!(summary.config.default_layout, "tiling");
    assert_eq!(summary.config.command_history_limit, 64);
    assert!(!summary.config.xwayland_enabled);
    assert_eq!(summary.config.commands.terminal.as_deref(), Some("foot"));
    assert!(
        summary.config.keybindings.contains_key("Super+Q"),
        "runtime config query should expose active keybindings: {:?}",
        summary.config.keybindings
    );
    assert!(
        summary
            .config
            .outputs
            .iter()
            .any(|output| output.name == summary.output_name && output.mode == "1920x1080@60"),
        "runtime config query should expose configured outputs: {:?}",
        summary.config.outputs
    );
    assert!(
        render_list.iter().any(|element| element.surface_id == TARGET_SURFACE_ID),
        "active workspace should still render the target window: {render_list:?}"
    );
    assert!(
        !render_list.iter().any(|element| element.surface_id == PRIMARY_SURFACE_ID),
        "inactive workspace window should be filtered from render output: {render_list:?}"
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

fn seed_windows(world: &mut bevy_ecs::world::World) {
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
                    last_acked_configure: None,
                },
                state: WindowState::Tiled,
                ..Default::default()
            },
            LayoutSlot {
                workspace: if surface_id == TARGET_SURFACE_ID { 2 } else { 1 },
                column: 0,
                row: 0,
            },
        ));
    }
}

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

    let outputs = wait_for_outputs(socket_path, |outputs| !outputs.is_empty())?;
    let output_name = outputs[0].name.clone();

    send_command(
        socket_path,
        IpcCommand::Output(OutputCommand::Disable { output: output_name.clone() }),
    )?;
    let _ = wait_for_outputs(socket_path, |outputs| {
        outputs.iter().all(|output| output.name != output_name)
    })?;
    send_command(
        socket_path,
        IpcCommand::Output(OutputCommand::Enable { output: output_name.clone() }),
    )?;
    let _ = wait_for_outputs(socket_path, |outputs| {
        outputs.iter().any(|output| output.name == output_name)
    })?;
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
    })?;

    let config = wait_for_config(socket_path, |config| {
        config.default_layout == "tiling"
            && config.command_history_limit == 64
            && !config.xwayland_enabled
            && config.commands.terminal.as_deref() == Some("foot")
    })?;

    Ok(IpcControlSummary { output_name, config })
}

fn wait_for_tree(
    socket_path: &Path,
    predicate: impl Fn(&TreeSnapshot) -> bool,
) -> Result<TreeSnapshot, common::TestControl> {
    wait_for_payload(socket_path, QueryCommand::GetTree, predicate)
}

fn wait_for_outputs(
    socket_path: &Path,
    predicate: impl Fn(&[OutputSnapshot]) -> bool,
) -> Result<Vec<OutputSnapshot>, common::TestControl> {
    wait_for_payload(socket_path, QueryCommand::GetOutputs, |outputs: &Vec<OutputSnapshot>| {
        predicate(outputs)
    })
}

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

fn wait_for_config(
    socket_path: &Path,
    predicate: impl Fn(&ConfigSnapshot) -> bool,
) -> Result<ConfigSnapshot, common::TestControl> {
    wait_for_payload(socket_path, QueryCommand::GetConfig, predicate)
}

fn wait_for_payload<T>(
    socket_path: &Path,
    query: QueryCommand,
    predicate: impl Fn(&T) -> bool,
) -> Result<T, common::TestControl>
where
    T: serde::de::DeserializeOwned,
{
    let deadline = Instant::now() + Duration::from_secs(2);

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

fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
    )
}

fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}
