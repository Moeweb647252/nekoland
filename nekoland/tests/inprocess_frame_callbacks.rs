use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ipc::commands::WorkspaceCommand;
use nekoland_ipc::{IpcCommand, IpcReply, IpcRequest, IpcServerState, send_request_to_path};
use nekoland_protocol::ProtocolServerState;
use wayland_client::protocol::{wl_callback, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

#[derive(Debug)]
struct FrameCallbackSummary {
    total_callbacks: usize,
    inactive_callbacks: usize,
}

#[derive(Debug)]
struct FrameCallbackClientState {
    ipc_socket_path: PathBuf,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    stage: u8,
    total_callbacks: usize,
    inactive_callbacks: usize,
    inactive_since: Option<Instant>,
    terminal_error: Option<String>,
}

impl FrameCallbackClientState {
    fn new(ipc_socket_path: PathBuf) -> Self {
        Self {
            ipc_socket_path,
            compositor: None,
            wm_base: None,
            base_surface: None,
            xdg_surface: None,
            toplevel: None,
            stage: 0,
            total_callbacks: 0,
            inactive_callbacks: 0,
            inactive_since: None,
            terminal_error: None,
        }
    }

    fn maybe_create_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_some() || self.compositor.is_none() || self.wm_base.is_none() {
            return;
        }

        let compositor =
            self.compositor.as_ref().expect("compositor presence checked immediately above");
        let wm_base = self.wm_base.as_ref().expect("wm_base presence checked immediately above");

        let base_surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        base_surface.commit();

        self.base_surface = Some(base_surface);
        self.xdg_surface = Some(xdg_surface);
        self.toplevel = Some(toplevel);
    }

    fn request_frame_callback(&self, qh: &QueueHandle<Self>) {
        let surface =
            self.base_surface.as_ref().expect("frame callback scenario requires a wl_surface");
        let _ = surface.frame(qh, ());
        surface.commit();
    }

    fn switch_workspace(&self, workspace: &str) -> Result<IpcReply, std::io::Error> {
        send_ipc_request_with_retry(
            &self.ipc_socket_path,
            &IpcRequest {
                correlation_id: 400 + u64::from(self.stage),
                command: IpcCommand::Workspace(WorkspaceCommand::Switch {
                    workspace: workspace.to_owned(),
                }),
            },
        )
    }

    fn advance_timers(&mut self) {
        if self.stage != 2 {
            return;
        }

        let Some(inactive_since) = self.inactive_since else {
            return;
        };
        if inactive_since.elapsed() < Duration::from_millis(20) {
            return;
        }

        match self.switch_workspace("1") {
            Ok(reply) if reply.ok => {
                self.stage = 3;
                self.inactive_since = None;
            }
            Ok(reply) => {
                self.terminal_error =
                    Some(format!("workspace reactivation request was rejected: {}", reply.message));
            }
            Err(error) => {
                self.terminal_error =
                    Some(format!("workspace reactivation request failed: {error}"));
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.stage >= 4
    }
}

#[test]
fn inactive_workspace_surfaces_stop_receiving_frame_done_until_reactivated() {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-frame-callback-runtime");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(192),
    });

    let socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<ProtocolServerState>()
            .expect("protocol server state should be available immediately after build");

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping frame callback test in restricted environment: {error}");
                return;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let ipc_socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<IpcServerState>()
            .expect("IPC server state should be available immediately after build");

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping frame callback IPC test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let client_thread =
        thread::spawn(move || run_frame_callback_client(&socket_path, ipc_socket_path));
    app.run().expect("nekoland app should complete the configured frame budget");

    let summary = match client_thread.join().expect("frame callback client should exit cleanly") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping frame callback test in restricted environment: {reason}");
            return;
        }
        Err(common::TestControl::Fail(reason)) => {
            panic!("frame callback scenario failed: {reason}");
        }
    };

    assert!(
        summary.total_callbacks >= 2,
        "surface should receive one active callback and one reactivated callback: {summary:?}"
    );
    assert_eq!(
        summary.inactive_callbacks, 0,
        "inactive workspace should suppress frame done delivery: {summary:?}"
    );

    drop(runtime_dir);
}

fn run_frame_callback_client(
    socket_path: &Path,
    ipc_socket_path: PathBuf,
) -> Result<FrameCallbackSummary, common::TestControl> {
    let stream = std::os::unix::net::UnixStream::connect(socket_path)
        .map_err(|error| common::TestControl::Fail(error.to_string()))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| common::TestControl::Fail(format!("set_read_timeout failed: {error}")))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| common::TestControl::Fail(format!("set_write_timeout failed: {error}")))?;

    let conn = Connection::from_socket(stream)
        .map_err(|error| common::TestControl::Fail(format!("from_socket failed: {error}")))?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    conn.display().get_registry(&qh, ());

    let mut state = FrameCallbackClientState::new(ipc_socket_path);
    let deadline = Instant::now() + Duration::from_secs(2);

    while !state.is_complete() {
        event_queue.dispatch_pending(&mut state).map_err(|error| {
            common::TestControl::Fail(format!("dispatch_pending before read failed: {error}"))
        })?;
        event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;

        if let Some(read_guard) = event_queue.prepare_read() {
            read_guard.read().map_err(|error| common::TestControl::Fail(error.to_string()))?;
            event_queue.dispatch_pending(&mut state).map_err(|error| {
                common::TestControl::Fail(format!("dispatch_pending after read failed: {error}"))
            })?;
        }

        state.advance_timers();

        if let Some(error) = state.terminal_error.take() {
            return Err(common::TestControl::Fail(error));
        }
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for frame callback scenario completion".to_owned(),
            ));
        }
    }

    Ok(FrameCallbackSummary {
        total_callbacks: state.total_callbacks,
        inactive_callbacks: state.inactive_callbacks,
    })
}

fn send_ipc_request_with_retry(
    socket_path: &Path,
    request: &IpcRequest,
) -> Result<IpcReply, std::io::Error> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        match send_request_to_path(socket_path, request) {
            Ok(reply) => return Ok(reply),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock
                        | ErrorKind::TimedOut
                        | ErrorKind::NotFound
                        | ErrorKind::ConnectionRefused
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::other(format!(
                        "timed out waiting for IPC request {:?}: {error}",
                        request.command
                    )));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

impl Dispatch<wl_registry::WlRegistry, ()> for FrameCallbackClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()));
                    state.maybe_create_toplevel(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                    state.maybe_create_toplevel(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for FrameCallbackClientState {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for FrameCallbackClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            if let Some(surface) = state.base_surface.as_ref() {
                surface.commit();
            }

            if state.stage == 0 {
                state.request_frame_callback(qh);
                state.stage = 1;
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for FrameCallbackClientState {
    fn event(
        state: &mut Self,
        _callback: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.total_callbacks = state.total_callbacks.saturating_add(1);

            match state.stage {
                1 => {
                    state.request_frame_callback(qh);
                    match state.switch_workspace("2") {
                        Ok(reply) if reply.ok => {
                            state.stage = 2;
                            state.inactive_since = Some(Instant::now());
                        }
                        Ok(reply) => {
                            state.terminal_error = Some(format!(
                                "workspace deactivation request was rejected: {}",
                                reply.message
                            ));
                        }
                        Err(error) => {
                            state.terminal_error =
                                Some(format!("workspace deactivation request failed: {error}"));
                        }
                    }
                }
                2 => {
                    state.inactive_callbacks = state.inactive_callbacks.saturating_add(1);
                    state.terminal_error = Some(
                        "received wl_surface.frame done while the window was on an inactive workspace"
                            .to_owned(),
                    );
                }
                3 => {
                    state.stage = 4;
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for FrameCallbackClientState {
    fn event(
        _state: &mut Self,
        _toplevel: &xdg_toplevel::XdgToplevel,
        _event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(FrameCallbackClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(FrameCallbackClientState: ignore wl_surface::WlSurface);
