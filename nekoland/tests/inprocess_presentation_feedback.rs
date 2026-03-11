use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_backend::traits::SelectedBackend;
use nekoland_core::app::RunLoopSettings;
use nekoland_ipc::commands::{OutputCommand, OutputSnapshot, QueryCommand, WorkspaceCommand};
use nekoland_ipc::{IpcCommand, IpcReply, IpcRequest, IpcServerState, send_request_to_path};
use nekoland_protocol::ProtocolServerState;
use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

const TEST_OUTPUT_MODE: &str = "1600x900@75";
const TEST_OUTPUT_WIDTH: u32 = 1600;
const TEST_OUTPUT_HEIGHT: u32 = 900;
const TEST_OUTPUT_REFRESH_MILLIHZ: u32 = 75_000;
const INACTIVE_WORKSPACE_HOLD_MILLIS: u64 = 40;
const EXPECTED_PRESENTATION_REFRESH_NANOS: u32 =
    (1_000_000_000_000_u64 / TEST_OUTPUT_REFRESH_MILLIHZ as u64) as u32;

#[derive(Debug)]
struct PresentationSummary {
    total_presented: usize,
    inactive_presented: usize,
    presented_refreshes: Vec<u32>,
    presented_timestamps_nanos: Vec<u64>,
    presented_sequences: Vec<u64>,
}

#[derive(Debug)]
struct PresentationClientState {
    ipc_socket_path: PathBuf,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    presentation: Option<wp_presentation::WpPresentation>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    stage: u8,
    total_presented: usize,
    inactive_presented: usize,
    presented_refreshes: Vec<u32>,
    presented_timestamps_nanos: Vec<u64>,
    presented_sequences: Vec<u64>,
    inactive_since: Option<Instant>,
    terminal_error: Option<String>,
}

impl PresentationClientState {
    fn new(ipc_socket_path: PathBuf) -> Self {
        Self {
            ipc_socket_path,
            compositor: None,
            wm_base: None,
            presentation: None,
            base_surface: None,
            xdg_surface: None,
            toplevel: None,
            stage: 0,
            total_presented: 0,
            inactive_presented: 0,
            presented_refreshes: Vec::new(),
            presented_timestamps_nanos: Vec::new(),
            presented_sequences: Vec::new(),
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

    fn request_feedback(&self, qh: &QueueHandle<Self>) {
        let presentation =
            self.presentation.as_ref().expect("presentation scenario requires wp_presentation");
        let surface =
            self.base_surface.as_ref().expect("presentation scenario requires a wl_surface");
        let _ = presentation.feedback(surface, qh, ());
        surface.commit();
    }

    fn switch_workspace(&self, workspace: &str) -> Result<IpcReply, std::io::Error> {
        send_ipc_request_with_retry(
            &self.ipc_socket_path,
            &IpcRequest {
                correlation_id: 500 + u64::from(self.stage),
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
        if inactive_since.elapsed() < Duration::from_millis(INACTIVE_WORKSPACE_HOLD_MILLIS) {
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
fn inactive_workspace_surfaces_delay_presentation_feedback_until_reactivated() {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-presentation-runtime");
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
                eprintln!("skipping presentation feedback test in restricted environment: {error}");
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
                eprintln!(
                    "skipping presentation feedback IPC test in restricted environment: {error}"
                );
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let client_thread =
        thread::spawn(move || run_presentation_client(&socket_path, ipc_socket_path));
    app.run().expect("nekoland app should complete the configured frame budget");

    let summary = match client_thread.join().expect("presentation client should exit cleanly") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping presentation feedback test in restricted environment: {reason}");
            return;
        }
        Err(common::TestControl::Fail(reason)) => {
            panic!("presentation feedback scenario failed: {reason}");
        }
    };

    let backend_description = app
        .inner()
        .world()
        .get_resource::<SelectedBackend>()
        .map(|backend| backend.description.clone())
        .unwrap_or_default();
    if backend_description.contains("timer fallback") {
        eprintln!(
            "skipping presentation refresh assertions because the test environment forced {backend_description}"
        );
        return;
    }

    assert!(
        summary.total_presented >= 2,
        "surface should receive one active presented event and one reactivated presented event: {summary:?}"
    );
    assert_eq!(
        summary.inactive_presented, 0,
        "inactive workspace should suppress presented delivery until reactivation: {summary:?}"
    );
    assert!(
        summary.presented_refreshes.len() >= 2,
        "expected at least one active and one reactivated presentation feedback: {summary:?}"
    );
    assert!(
        summary
            .presented_refreshes
            .iter()
            .all(|refresh| *refresh == EXPECTED_PRESENTATION_REFRESH_NANOS),
        "presentation feedback should reflect the configured output refresh: {summary:?}"
    );
    assert!(
        summary.presented_timestamps_nanos.len() >= 2,
        "expected presentation timestamps for both visible feedback deliveries: {summary:?}"
    );
    assert!(
        summary.presented_timestamps_nanos.windows(2).all(|timestamps| {
            let delta = timestamps[1].saturating_sub(timestamps[0]);
            delta >= u64::from(EXPECTED_PRESENTATION_REFRESH_NANOS)
                && delta % u64::from(EXPECTED_PRESENTATION_REFRESH_NANOS) == 0
        }),
        "presentation timestamps should advance on the output refresh cadence: {summary:?}"
    );
    assert!(
        summary.presented_sequences.len() >= 2,
        "expected backend presentation sequence values for both visible deliveries: {summary:?}"
    );
    assert!(
        summary.presented_sequences.windows(2).all(|sequences| {
            let delta = sequences[1].saturating_sub(sequences[0]);
            delta > 1
        }),
        "backend presentation sequence should keep advancing while the surface is throttled: {summary:?}"
    );

    drop(runtime_dir);
}

fn run_presentation_client(
    socket_path: &Path,
    ipc_socket_path: PathBuf,
) -> Result<PresentationSummary, common::TestControl> {
    configure_output_mode(&ipc_socket_path)?;

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

    let mut state = PresentationClientState::new(ipc_socket_path);
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
                "timed out waiting for presentation feedback scenario completion".to_owned(),
            ));
        }
    }

    Ok(PresentationSummary {
        total_presented: state.total_presented,
        inactive_presented: state.inactive_presented,
        presented_refreshes: state.presented_refreshes,
        presented_timestamps_nanos: state.presented_timestamps_nanos,
        presented_sequences: state.presented_sequences,
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

fn configure_output_mode(socket_path: &Path) -> Result<(), common::TestControl> {
    let output_name = wait_for_outputs(socket_path, |outputs| !outputs.is_empty())?
        .into_iter()
        .next()
        .expect("wait_for_outputs predicate should ensure at least one output is present")
        .name;

    let reply = send_ipc_request_with_retry(
        socket_path,
        &IpcRequest {
            correlation_id: 601,
            command: IpcCommand::Output(OutputCommand::Configure {
                output: output_name.clone(),
                mode: TEST_OUTPUT_MODE.to_owned(),
                scale: None,
            }),
        },
    )
    .map_err(|error| {
        common::TestControl::Fail(format!("output configure request failed: {error}"))
    })?;

    if !reply.ok {
        return Err(common::TestControl::Fail(format!(
            "output configure request was rejected: {}",
            reply.message
        )));
    }

    let _ = wait_for_outputs(socket_path, |outputs| {
        outputs.iter().any(|output| {
            output.name == output_name
                && output.width == TEST_OUTPUT_WIDTH
                && output.height == TEST_OUTPUT_HEIGHT
                && output.refresh_millihz == TEST_OUTPUT_REFRESH_MILLIHZ
        })
    })?;

    Ok(())
}

fn wait_for_outputs(
    socket_path: &Path,
    predicate: impl Fn(&[OutputSnapshot]) -> bool,
) -> Result<Vec<OutputSnapshot>, common::TestControl> {
    wait_for_payload(socket_path, QueryCommand::GetOutputs, |outputs: &Vec<OutputSnapshot>| {
        predicate(outputs)
    })
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
            Err(error) => return Err(common::TestControl::Fail(error.to_string())),
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

fn query_payload<T>(socket_path: &Path, command: IpcCommand) -> Result<T, std::io::Error>
where
    T: serde::de::DeserializeOwned,
{
    let reply = send_request_to_path(socket_path, &IpcRequest { correlation_id: 602, command })?;

    if !reply.ok {
        return Err(std::io::Error::other(format!("IPC query failed: {}", reply.message)));
    }

    let payload =
        reply.payload.ok_or_else(|| std::io::Error::other("IPC query returned no payload"))?;

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

fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

impl Dispatch<wl_registry::WlRegistry, ()> for PresentationClientState {
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
                "wp_presentation" => {
                    state.presentation = Some(
                        registry.bind::<wp_presentation::WpPresentation, _, _>(name, 1, qh, ()),
                    );
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for PresentationClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for PresentationClientState {
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
                state.request_feedback(qh);
                state.stage = 1;
            }
        }
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, ()> for PresentationClientState {
    fn event(
        state: &mut Self,
        _feedback: &wp_presentation_feedback::WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                seq_hi,
                seq_lo,
                ..
            } => {
                state.presented_refreshes.push(refresh);
                state
                    .presented_timestamps_nanos
                    .push(presentation_timestamp_nanos(tv_sec_hi, tv_sec_lo, tv_nsec));
                state.presented_sequences.push(presentation_sequence(seq_hi, seq_lo));
                match state.stage {
                    1 => {
                        state.total_presented = state.total_presented.saturating_add(1);
                        state.request_feedback(qh);
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
                        state.total_presented = state.total_presented.saturating_add(1);
                        state.inactive_presented = state.inactive_presented.saturating_add(1);
                        state.terminal_error = Some(
                        "received wp_presentation.presented while the surface was on an inactive workspace"
                            .to_owned(),
                    );
                    }
                    3 => {
                        state.total_presented = state.total_presented.saturating_add(1);
                        state.stage = 4;
                    }
                    _ => {}
                }
            }
            wp_presentation_feedback::Event::Discarded => {
                state.terminal_error =
                    Some("presentation feedback was discarded unexpectedly".to_owned());
            }
            _ => {}
        }
    }
}

fn presentation_timestamp_nanos(tv_sec_hi: u32, tv_sec_lo: u32, tv_nsec: u32) -> u64 {
    let tv_sec = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
    tv_sec.saturating_mul(1_000_000_000).saturating_add(u64::from(tv_nsec))
}

fn presentation_sequence(seq_hi: u32, seq_lo: u32) -> u64 {
    (u64::from(seq_hi) << 32) | u64::from(seq_lo)
}

impl Dispatch<wp_presentation::WpPresentation, ()> for PresentationClientState {
    fn event(
        _state: &mut Self,
        _presentation: &wp_presentation::WpPresentation,
        _event: wp_presentation::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for PresentationClientState {
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

delegate_noop!(PresentationClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(PresentationClientState: ignore wl_surface::WlSurface);
