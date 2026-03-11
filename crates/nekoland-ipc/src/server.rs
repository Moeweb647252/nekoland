use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy_ecs::prelude::{NonSendMut, Query, Res, ResMut, Resource};
use nekoland_config::LoadedConfigSource;
use serde::{Deserialize, Serialize};

use crate::commands::query::{
    ClipboardSnapshot, CommandSnapshot, CommandStatusSnapshot, ConfigCommandSnapshot,
    ConfigOutputSnapshot, ConfigSnapshot, OutputSnapshot, PopupSnapshot, PrimarySelectionSnapshot,
    QueryCommand, SelectionOwnerSnapshot, TreeSnapshot, WindowSnapshot, WorkspaceSnapshot,
};
use crate::commands::{OutputCommand, PopupCommand, WindowCommand, WorkspaceCommand};
use crate::subscribe::{
    IpcSubscription, IpcSubscriptionEvent, PendingSubscriptionEvents, SubscriptionTopic,
};
use crate::{IpcCommand, IpcReply, IpcRequest};
use nekoland_ecs::components::{
    LayoutSlot, OutputDevice, OutputProperties, PopupGrab, SurfaceGeometry, WindowState,
    WlSurfaceHandle, Workspace, X11Window, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{
    ClipboardSelectionState, CommandExecutionStatus, CommandHistoryState, CompositorClock,
    CompositorConfig, KeyboardFocusState, OutputServerAction, OutputServerRequest,
    PendingOutputServerRequests, PendingPopupServerRequests, PendingWindowServerRequests,
    PendingWorkspaceServerRequests, PopupServerAction, PopupServerRequest, PrimarySelectionState,
    RenderList, SelectionOwner, WindowServerAction, WindowServerRequest, WorkspaceServerAction,
    WorkspaceServerRequest,
};

const DEFAULT_IPC_SOCKET_NAME: &str = "nekoland-ipc.sock";
const IPC_IO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Resource, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcServerState {
    pub socket_path: PathBuf,
    pub listening: bool,
    pub startup_error: Option<String>,
    pub last_accept_error: Option<String>,
    pub last_client_error: Option<String>,
}

impl Default for IpcServerState {
    fn default() -> Self {
        Self {
            socket_path: default_socket_path(),
            listening: false,
            startup_error: None,
            last_accept_error: None,
            last_client_error: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct IpcServerRuntime {
    listener: Option<UnixListener>,
    connections: Vec<IpcConnection>,
    socket_path: PathBuf,
}

#[derive(Debug)]
struct IpcConnection {
    stream: UnixStream,
    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,
    request_processed: bool,
    peer_closed: bool,
    mode: ConnectionMode,
}

#[derive(Debug, Clone)]
enum ConnectionMode {
    RequestResponse,
    Subscription(IpcSubscription),
}

enum RequestDisposition {
    Reply(IpcReply),
    Subscribe(IpcSubscription),
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcQueryCache {
    pub tree: TreeSnapshot,
    pub outputs: Vec<OutputSnapshot>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub commands: Vec<CommandSnapshot>,
    pub config: ConfigSnapshot,
    pub clipboard: ClipboardSnapshot,
    pub primary_selection: PrimarySelectionSnapshot,
}

impl IpcQueryCache {
    pub fn build_reply(&self, command: &QueryCommand) -> IpcReply {
        let payload = match command {
            QueryCommand::GetTree => serde_json::to_value(&self.tree),
            QueryCommand::GetOutputs => serde_json::to_value(&self.outputs),
            QueryCommand::GetWorkspaces => serde_json::to_value(&self.workspaces),
            QueryCommand::GetCommands => serde_json::to_value(&self.commands),
            QueryCommand::GetConfig => serde_json::to_value(&self.config),
            QueryCommand::GetClipboard => serde_json::to_value(&self.clipboard),
            QueryCommand::GetPrimarySelection => serde_json::to_value(&self.primary_selection),
        }
        .ok();

        IpcReply {
            ok: payload.is_some(),
            message: format!("prepared reply for {command:?}"),
            payload,
        }
    }
}

impl IpcServerRuntime {
    pub(crate) fn new() -> (Self, IpcServerState) {
        let socket_path = default_socket_path();
        let mut server_state =
            IpcServerState { socket_path: socket_path.clone(), ..IpcServerState::default() };

        let listener = match bind_ipc_listener(&socket_path) {
            Ok(listener) => {
                tracing::info!(socket = %socket_path.display(), "IPC socket ready");
                server_state.listening = true;
                Some(listener)
            }
            Err(error) => {
                let error = error.to_string();
                tracing::warn!(
                    socket = %socket_path.display(),
                    error = %error,
                    "failed to create IPC socket"
                );
                server_state.startup_error = Some(error);
                None
            }
        };

        (Self { listener, connections: Vec::new(), socket_path }, server_state)
    }

    fn pump(
        &mut self,
        server_state: &mut IpcServerState,
        query_cache: &IpcQueryCache,
        pending_subscription_events: &mut PendingSubscriptionEvents,
        pending_popup_requests: &mut PendingPopupServerRequests,
        pending_window_requests: &mut PendingWindowServerRequests,
        pending_workspace_requests: &mut PendingWorkspaceServerRequests,
        pending_output_requests: &mut PendingOutputServerRequests,
    ) {
        self.accept_connections(server_state);
        self.process_connections(
            server_state,
            query_cache,
            pending_subscription_events,
            pending_popup_requests,
            pending_window_requests,
            pending_workspace_requests,
            pending_output_requests,
        );
        self.dispatch_subscription_events(pending_subscription_events);
        self.process_connections(
            server_state,
            query_cache,
            pending_subscription_events,
            pending_popup_requests,
            pending_window_requests,
            pending_workspace_requests,
            pending_output_requests,
        );
    }

    fn accept_connections(&mut self, server_state: &mut IpcServerState) {
        let Some(listener) = self.listener.as_ref() else {
            return;
        };

        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    if let Err(error) = stream.set_nonblocking(true) {
                        remember_server_error(
                            &mut server_state.last_accept_error,
                            error,
                            "failed to configure IPC client stream",
                        );
                        continue;
                    }
                    self.connections.push(IpcConnection::new(stream));
                    server_state.last_accept_error = None;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => {
                    remember_server_error(
                        &mut server_state.last_accept_error,
                        error,
                        "failed to accept IPC client",
                    );
                    break;
                }
            }
        }
    }

    fn process_connections(
        &mut self,
        server_state: &mut IpcServerState,
        query_cache: &IpcQueryCache,
        pending_subscription_events: &mut PendingSubscriptionEvents,
        pending_popup_requests: &mut PendingPopupServerRequests,
        pending_window_requests: &mut PendingWindowServerRequests,
        pending_workspace_requests: &mut PendingWorkspaceServerRequests,
        pending_output_requests: &mut PendingOutputServerRequests,
    ) {
        let mut keep = Vec::with_capacity(self.connections.len());

        for mut connection in self.connections.drain(..) {
            let should_close = connection.pump(
                server_state,
                query_cache,
                pending_subscription_events,
                pending_popup_requests,
                pending_window_requests,
                pending_workspace_requests,
                pending_output_requests,
            );
            if !should_close {
                keep.push(connection);
            }
        }

        self.connections = keep;
    }

    fn dispatch_subscription_events(
        &mut self,
        pending_subscription_events: &mut PendingSubscriptionEvents,
    ) {
        if pending_subscription_events.events.is_empty() {
            return;
        }

        let events = std::mem::take(&mut pending_subscription_events.events);
        for event in &events {
            for connection in &mut self.connections {
                connection.queue_subscription_event(event);
            }
        }
    }
}

impl Drop for IpcServerRuntime {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

impl IpcConnection {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            read_buffer: Vec::new(),
            write_buffer: Vec::new(),
            request_processed: false,
            peer_closed: false,
            mode: ConnectionMode::RequestResponse,
        }
    }

    fn pump(
        &mut self,
        server_state: &mut IpcServerState,
        query_cache: &IpcQueryCache,
        _pending_subscription_events: &mut PendingSubscriptionEvents,
        pending_popup_requests: &mut PendingPopupServerRequests,
        pending_window_requests: &mut PendingWindowServerRequests,
        pending_workspace_requests: &mut PendingWorkspaceServerRequests,
        pending_output_requests: &mut PendingOutputServerRequests,
    ) -> bool {
        if !self.request_processed {
            self.read_request(server_state);
            if let Some(frame) = take_request_frame(&mut self.read_buffer, self.peer_closed) {
                let disposition = match parse_request_frame(&frame) {
                    Ok(request) => reply_for_request(
                        request,
                        query_cache,
                        pending_popup_requests,
                        pending_window_requests,
                        pending_workspace_requests,
                        pending_output_requests,
                    ),
                    Err(error) => RequestDisposition::Reply(IpcReply {
                        ok: false,
                        message: format!("failed to decode request: {error}"),
                        payload: None,
                    }),
                };

                match disposition {
                    RequestDisposition::Reply(reply) => {
                        self.write_buffer = encode_reply(reply);
                    }
                    RequestDisposition::Subscribe(subscription) => {
                        self.mode = ConnectionMode::Subscription(subscription.clone());
                        self.write_buffer = encode_reply(IpcReply {
                            ok: true,
                            message: format!("subscribed to {:?} events", subscription.topic),
                            payload: None,
                        });
                    }
                }
                self.request_processed = true;
            }
        }

        if !self.write_buffer.is_empty() {
            let should_close = self.write_reply(server_state);
            if should_close || !self.write_buffer.is_empty() {
                return should_close;
            }
        }

        if !self.request_processed {
            return false;
        }

        match self.mode {
            ConnectionMode::RequestResponse => true,
            ConnectionMode::Subscription(_) => self.poll_subscription_peer(server_state),
        }
    }

    fn read_request(&mut self, server_state: &mut IpcServerState) {
        let mut buffer = [0_u8; 4096];

        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => {
                    self.peer_closed = true;
                    break;
                }
                Ok(read) => self.read_buffer.extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => {
                    self.peer_closed = true;
                    remember_server_error(
                        &mut server_state.last_client_error,
                        error,
                        "failed to read IPC request",
                    );
                    break;
                }
            }
        }
    }

    fn write_reply(&mut self, server_state: &mut IpcServerState) -> bool {
        while !self.write_buffer.is_empty() {
            match self.stream.write(&self.write_buffer) {
                Ok(0) => return true,
                Ok(written) => {
                    self.write_buffer.drain(..written);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return false,
                Err(error) => {
                    remember_server_error(
                        &mut server_state.last_client_error,
                        error,
                        "failed to write IPC reply",
                    );
                    return true;
                }
            }
        }

        false
    }

    fn poll_subscription_peer(&mut self, server_state: &mut IpcServerState) -> bool {
        let mut buffer = [0_u8; 512];

        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => {
                    self.peer_closed = true;
                    return true;
                }
                Ok(_) => continue,
                Err(error) if error.kind() == ErrorKind::WouldBlock => return false,
                Err(error) => {
                    remember_server_error(
                        &mut server_state.last_client_error,
                        error,
                        "failed while polling IPC subscription client",
                    );
                    return true;
                }
            }
        }
    }

    fn queue_subscription_event(&mut self, event: &IpcSubscriptionEvent) {
        let ConnectionMode::Subscription(subscription) = &self.mode else {
            return;
        };
        if !subscription_matches(subscription, event) {
            return;
        }

        let mut event = event.clone();
        if !subscription.include_payloads {
            event.payload = None;
        }
        self.write_buffer.extend(encode_subscription_event(&event));
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Some(path) = env::var_os("NEKOLAND_IPC_SOCKET") {
        return PathBuf::from(path);
    }

    if let Some(runtime_dir) =
        env::var_os("NEKOLAND_RUNTIME_DIR").or_else(|| env::var_os("XDG_RUNTIME_DIR"))
    {
        return PathBuf::from(runtime_dir).join(DEFAULT_IPC_SOCKET_NAME);
    }

    PathBuf::from("/tmp").join(DEFAULT_IPC_SOCKET_NAME)
}

pub fn send_request(request: &IpcRequest) -> io::Result<IpcReply> {
    send_request_to_path(&default_socket_path(), request)
}

pub fn send_request_to_path(socket_path: &Path, request: &IpcRequest) -> io::Result<IpcReply> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(IPC_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_IO_TIMEOUT))?;

    let mut request_bytes = serde_json::to_vec(request).map_err(io::Error::other)?;
    request_bytes.push(b'\n');
    stream.write_all(&request_bytes)?;
    stream.shutdown(Shutdown::Write)?;

    let mut reply = String::new();
    let mut reader = BufReader::new(stream);
    let bytes_read = reader.read_line(&mut reply)?;
    if bytes_read == 0 {
        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "IPC server closed the connection without replying",
        ));
    }

    serde_json::from_str(reply.trim_end()).map_err(io::Error::other)
}

pub(crate) fn accept_connections_system(
    mut runtime: NonSendMut<IpcServerRuntime>,
    mut server_state: ResMut<IpcServerState>,
    query_cache: Res<IpcQueryCache>,
    mut pending_subscription_events: ResMut<PendingSubscriptionEvents>,
    mut pending_popup_requests: ResMut<PendingPopupServerRequests>,
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
    mut pending_workspace_requests: ResMut<PendingWorkspaceServerRequests>,
    mut pending_output_requests: ResMut<PendingOutputServerRequests>,
) {
    runtime.pump(
        &mut server_state,
        &query_cache,
        &mut pending_subscription_events,
        &mut pending_popup_requests,
        &mut pending_window_requests,
        &mut pending_workspace_requests,
        &mut pending_output_requests,
    );

    tracing::trace!(
        socket = %server_state.socket_path.display(),
        listening = server_state.listening,
        outputs = query_cache.outputs.len(),
        windows = query_cache.tree.windows.len(),
        popups = query_cache.tree.popups.len(),
        clients = runtime.connections.len(),
        "ipc accept loop system tick"
    );
}

pub fn refresh_query_cache_system(
    outputs: Query<(&OutputDevice, &OutputProperties)>,
    workspaces: Query<&Workspace>,
    windows: Query<(
        &WlSurfaceHandle,
        &XdgWindow,
        Option<&X11Window>,
        &SurfaceGeometry,
        &WindowState,
        Option<&LayoutSlot>,
    )>,
    popups: Query<(&WlSurfaceHandle, &XdgPopup, &SurfaceGeometry, Option<&PopupGrab>)>,
    render_list: Res<RenderList>,
    keyboard_focus: Res<KeyboardFocusState>,
    clock: Res<CompositorClock>,
    command_history: Res<CommandHistoryState>,
    config: Res<CompositorConfig>,
    clipboard_selection: Res<ClipboardSelectionState>,
    primary_selection: Res<PrimarySelectionState>,
    config_source: Option<Res<LoadedConfigSource>>,
    mut query_cache: ResMut<IpcQueryCache>,
) {
    query_cache.outputs = outputs
        .iter()
        .map(|(output, properties)| OutputSnapshot {
            name: output.name.clone(),
            kind: output.kind.clone(),
            make: output.make.clone(),
            model: output.model.clone(),
            width: properties.width,
            height: properties.height,
            refresh_millihz: properties.refresh_millihz,
            scale: properties.scale,
        })
        .collect();

    query_cache.workspaces = workspaces
        .iter()
        .map(|workspace| WorkspaceSnapshot {
            id: workspace.id.0,
            name: workspace.name.clone(),
            active: workspace.active,
        })
        .collect();

    query_cache.commands = command_history
        .items
        .iter()
        .cloned()
        .map(|record| CommandSnapshot {
            frame: record.frame,
            uptime_millis: record.uptime_millis,
            origin: record.origin,
            command: record.command,
            candidates: record.candidates,
            status: record.status.map(|status| match status {
                CommandExecutionStatus::Launched { pid } => CommandStatusSnapshot::Launched { pid },
                CommandExecutionStatus::Failed { error } => CommandStatusSnapshot::Failed { error },
            }),
        })
        .collect();

    query_cache.config = ConfigSnapshot {
        path: config_source.as_ref().map(|source| source.path.display().to_string()),
        loaded_from_disk: config_source.as_ref().is_some_and(|source| source.loaded_from_disk),
        successful_reloads: config_source.as_ref().map_or(0, |source| source.successful_reloads),
        last_reload_error: config_source
            .as_ref()
            .and_then(|source| source.last_reload_error.clone()),
        theme: config.theme.clone(),
        cursor_theme: config.cursor_theme.clone(),
        border_color: config.border_color.clone(),
        background_color: config.background_color.clone(),
        default_layout: config.default_layout.clone(),
        focus_follows_mouse: config.focus_follows_mouse,
        repeat_rate: config.repeat_rate,
        command_history_limit: config.command_history_limit,
        startup_commands: config.startup_commands.clone(),
        xwayland_enabled: config.xwayland.enabled,
        outputs: config
            .outputs
            .iter()
            .cloned()
            .map(|output| ConfigOutputSnapshot {
                name: output.name,
                mode: output.mode,
                scale: output.scale,
                enabled: output.enabled,
            })
            .collect(),
        commands: ConfigCommandSnapshot {
            terminal: config.commands.terminal.clone(),
            launcher: config.commands.launcher.clone(),
            power_menu: config.commands.power_menu.clone(),
        },
        keybindings: config.keybindings.clone(),
    };

    query_cache.clipboard = ClipboardSnapshot {
        seat_name: clipboard_selection
            .selection
            .as_ref()
            .map(|selection| selection.seat_name.clone()),
        mime_types: clipboard_selection
            .selection
            .as_ref()
            .map(|selection| selection.mime_types.clone())
            .unwrap_or_default(),
        owner: clipboard_selection
            .selection
            .as_ref()
            .map(|selection| selection_owner_snapshot(selection.owner)),
        persisted_mime_types: clipboard_selection
            .selection
            .as_ref()
            .map(|selection| selection.persisted_mime_types.clone())
            .unwrap_or_default(),
    };
    query_cache.primary_selection = PrimarySelectionSnapshot {
        seat_name: primary_selection
            .selection
            .as_ref()
            .map(|selection| selection.seat_name.clone()),
        mime_types: primary_selection
            .selection
            .as_ref()
            .map(|selection| selection.mime_types.clone())
            .unwrap_or_default(),
        owner: primary_selection
            .selection
            .as_ref()
            .map(|selection| selection_owner_snapshot(selection.owner)),
        persisted_mime_types: primary_selection
            .selection
            .as_ref()
            .map(|selection| selection.persisted_mime_types.clone())
            .unwrap_or_default(),
    };

    let windows = windows
        .iter()
        .map(|(surface, window, x11_window, geometry, state, slot)| WindowSnapshot {
            surface_id: surface.id,
            title: window.title.clone(),
            app_id: window.app_id.clone(),
            xwayland: x11_window.is_some(),
            x11_window_id: x11_window.map(|window| window.window_id),
            override_redirect: x11_window.is_some_and(|window| window.override_redirect),
            x: geometry.x,
            y: geometry.y,
            width: geometry.width,
            height: geometry.height,
            state: format!("{state:?}"),
            workspace: slot.map(|slot| slot.workspace),
            focused: keyboard_focus.focused_surface == Some(surface.id),
        })
        .collect::<Vec<_>>();
    let popups = popups
        .iter()
        .map(|(surface, popup, geometry, grab)| PopupSnapshot {
            surface_id: surface.id,
            parent_surface_id: popup.parent_surface,
            x: geometry.x,
            y: geometry.y,
            width: geometry.width,
            height: geometry.height,
            grab_active: grab.is_some_and(|grab| grab.active),
            grab_serial: grab.and_then(|grab| grab.serial),
        })
        .collect::<Vec<_>>();

    query_cache.tree = TreeSnapshot {
        frame: clock.frame,
        focused_surface: keyboard_focus.focused_surface,
        outputs: query_cache.outputs.clone(),
        workspaces: query_cache.workspaces.clone(),
        windows,
        popups,
        render_order: render_list.elements.iter().map(|element| element.surface_id).collect(),
    };
}

fn selection_owner_snapshot(owner: SelectionOwner) -> SelectionOwnerSnapshot {
    match owner {
        SelectionOwner::Client => SelectionOwnerSnapshot::Client,
        SelectionOwner::Compositor => SelectionOwnerSnapshot::Compositor,
    }
}

fn bind_ipc_listener(socket_path: &Path) -> io::Result<UnixListener> {
    let Some(parent) = socket_path.parent() else {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("IPC socket path has no parent directory: {}", socket_path.display()),
        ));
    };
    fs::create_dir_all(parent)?;

    match fs::symlink_metadata(socket_path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(socket_path)?;
        }
        Ok(_) => {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "IPC socket path already exists and is not a socket: {}",
                    socket_path.display()
                ),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let listener = UnixListener::bind(socket_path)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn take_request_frame(buffer: &mut Vec<u8>, peer_closed: bool) -> Option<Vec<u8>> {
    if let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
        return Some(buffer.drain(..=index).collect());
    }

    if peer_closed && !buffer.is_empty() {
        return Some(std::mem::take(buffer));
    }

    None
}

fn parse_request_frame(frame: &[u8]) -> io::Result<IpcRequest> {
    let frame = frame.strip_suffix(b"\n").unwrap_or(frame).strip_suffix(b"\r").unwrap_or(frame);
    serde_json::from_slice(frame).map_err(io::Error::other)
}

fn encode_reply(reply: IpcReply) -> Vec<u8> {
    match serde_json::to_vec(&reply) {
        Ok(mut encoded) => {
            encoded.push(b'\n');
            encoded
        }
        Err(error) => format!(
            "{{\"ok\":false,\"message\":\"failed to encode IPC reply: {error}\",\"payload\":null}}\n"
        )
        .into_bytes(),
    }
}

fn encode_subscription_event(event: &IpcSubscriptionEvent) -> Vec<u8> {
    match serde_json::to_vec(event) {
        Ok(mut encoded) => {
            encoded.push(b'\n');
            encoded
        }
        Err(error) => format!(
            "{{\"topic\":\"All\",\"event\":\"subscription_encode_failed\",\"payload\":{{\"error\":\"{error}\"}}}}\n"
        )
        .into_bytes(),
    }
}

fn subscription_matches(subscription: &IpcSubscription, event: &IpcSubscriptionEvent) -> bool {
    let topic_matches =
        matches!(subscription.topic, SubscriptionTopic::All) || subscription.topic == event.topic;
    if !topic_matches {
        return false;
    }

    subscription.events.is_empty()
        || subscription.events.iter().any(|pattern| event_filter_matches(pattern, &event.event))
}

fn event_filter_matches(pattern: &str, event_name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => event_name.starts_with(prefix),
        None => pattern == event_name,
    }
}

fn reply_for_request(
    request: IpcRequest,
    query_cache: &IpcQueryCache,
    pending_popup_requests: &mut PendingPopupServerRequests,
    pending_window_requests: &mut PendingWindowServerRequests,
    pending_workspace_requests: &mut PendingWorkspaceServerRequests,
    pending_output_requests: &mut PendingOutputServerRequests,
) -> RequestDisposition {
    match request.command {
        IpcCommand::Query(command) => RequestDisposition::Reply(query_cache.build_reply(&command)),
        IpcCommand::Subscribe(subscription) => RequestDisposition::Subscribe(subscription),
        IpcCommand::Popup(PopupCommand::Dismiss { surface_id }) => {
            pending_popup_requests
                .items
                .push(PopupServerRequest { surface_id, action: PopupServerAction::Dismiss });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued popup dismiss request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Close { surface_id }) => {
            pending_window_requests
                .items
                .push(WindowServerRequest { surface_id, action: WindowServerAction::Close });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued close request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Focus { surface_id }) => {
            pending_window_requests
                .items
                .push(WindowServerRequest { surface_id, action: WindowServerAction::Focus });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued focus request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Move { surface_id, x, y }) => {
            pending_window_requests.items.push(WindowServerRequest {
                surface_id,
                action: WindowServerAction::Move { x, y },
            });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued move request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Resize { surface_id, width, height }) => {
            pending_window_requests.items.push(WindowServerRequest {
                surface_id,
                action: WindowServerAction::Resize { width, height },
            });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued resize request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Workspace(WorkspaceCommand::Switch { workspace }) => {
            pending_workspace_requests.items.push(WorkspaceServerRequest {
                action: WorkspaceServerAction::Switch { workspace: workspace.clone() },
            });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued workspace switch to {workspace}"),
                payload: None,
            })
        }
        IpcCommand::Workspace(WorkspaceCommand::Create { workspace }) => {
            pending_workspace_requests.items.push(WorkspaceServerRequest {
                action: WorkspaceServerAction::Create { workspace: workspace.clone() },
            });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued workspace create for {workspace}"),
                payload: None,
            })
        }
        IpcCommand::Workspace(WorkspaceCommand::Destroy { workspace }) => {
            pending_workspace_requests.items.push(WorkspaceServerRequest {
                action: WorkspaceServerAction::Destroy { workspace: workspace.clone() },
            });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued workspace destroy for {workspace}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::Configure { output, mode, scale }) => {
            pending_output_requests.items.push(OutputServerRequest {
                action: OutputServerAction::Configure {
                    output: output.clone(),
                    mode: mode.clone(),
                    scale,
                },
            });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued output configure for {output}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::Enable { output }) => {
            pending_output_requests.items.push(OutputServerRequest {
                action: OutputServerAction::Enable { output: output.clone() },
            });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued output enable for {output}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::Disable { output }) => {
            pending_output_requests.items.push(OutputServerRequest {
                action: OutputServerAction::Disable { output: output.clone() },
            });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued output disable for {output}"),
                payload: None,
            })
        }
        IpcCommand::Raw(command) => RequestDisposition::Reply(IpcReply {
            ok: false,
            message: format!("unsupported raw IPC command: {command}"),
            payload: None,
        }),
    }
}

fn remember_server_error(slot: &mut Option<String>, error: impl std::fmt::Display, message: &str) {
    let error = error.to_string();
    if slot.as_deref() != Some(error.as_str()) {
        tracing::warn!(error = %error, "{message}");
    }
    *slot = Some(error);
}

#[cfg(test)]
mod tests {
    use super::{event_filter_matches, subscription_matches};
    use crate::subscribe::{IpcSubscription, IpcSubscriptionEvent, SubscriptionTopic};

    #[test]
    fn event_filter_matches_exact_names() {
        assert!(event_filter_matches("tree_changed", "tree_changed"));
        assert!(!event_filter_matches("tree_changed", "workspaces_changed"));
    }

    #[test]
    fn event_filter_matches_prefix_wildcards() {
        assert!(event_filter_matches("window_*", "window_created"));
        assert!(event_filter_matches("window_*", "window_closed"));
        assert!(!event_filter_matches("window_*", "tree_changed"));
    }

    #[test]
    fn subscription_matches_combines_topic_and_wildcard_event_filters() {
        let subscription = IpcSubscription {
            topic: SubscriptionTopic::Window,
            include_payloads: true,
            events: vec!["window_*".to_owned()],
        };
        let matching_event = IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "window_moved".to_owned(),
            payload: None,
        };
        let filtered_event = IpcSubscriptionEvent {
            topic: SubscriptionTopic::Tree,
            event: "tree_changed".to_owned(),
            payload: None,
        };

        assert!(subscription_matches(&subscription, &matching_event));
        assert!(!subscription_matches(&subscription, &filtered_event));
    }
}
