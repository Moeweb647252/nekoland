use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, NonSendMut, Query, Res, ResMut, Resource, With};
use bevy_ecs::query::Allow;
use bevy_ecs::system::SystemParam;
use nekoland_config::{
    ConfigReloadRequest, LoadedConfigSource,
    resources::{CompositorConfig, ConfiguredKeyboardLayout, KeyboardLayoutState},
};
use nekoland_core::lifecycle::AppLifecycleState;
use nekoland_ecs::control::{OutputControlApi, WindowControlApi, WorkspaceControlApi};
use nekoland_ecs::resources::SelectionOwner;
use nekoland_ecs::resources::{
    CommandExecutionStatus, CommandHistoryState, CompositorClock, EntityIndex,
    ExternalCommandRequest, KeyboardFocusState, PendingExternalCommandRequests,
    PendingOutputControls, PendingPopupServerRequests, PendingWindowControls,
    PendingWorkspaceControls, PopupServerAction, PopupServerRequest, PresentAuditElementKind,
    RenderPlan, RenderPlanItem, SeatRegistry, WaylandFeedback, WaylandIngress,
};
use nekoland_ecs::selectors::{
    OutputName, SurfaceId, WorkspaceLookup, WorkspaceName, WorkspaceSelector,
};
use serde::{Deserialize, Serialize};

use crate::commands::query::{
    ClipboardSnapshot, CommandSnapshot, CommandStatusSnapshot, ConfigOutputSnapshot,
    ConfigSnapshot, KeyboardLayoutEntrySnapshot, KeyboardLayoutsSnapshot, OutputSnapshot,
    PopupSnapshot, PresentAuditElementSnapshot, PresentAuditOutputSnapshot,
    PrimarySelectionSnapshot, QueryCommand, SelectionOwnerSnapshot, TreeSnapshot, WindowSnapshot,
    WorkspaceSnapshot,
};
use crate::commands::{
    ActionCommand, OutputCommand, PopupCommand, WindowCommand, WorkspaceCommand,
};
use crate::subscribe::{
    IpcSubscription, IpcSubscriptionEvent, PendingSubscriptionEvents, SubscriptionTopic,
};
use crate::{IpcCommand, IpcReply, IpcRequest};
use nekoland_ecs::components::{
    PopupSurface, SeatId, WindowDisplayState, WindowLayout, WindowMode, WlSurfaceHandle,
};
use nekoland_ecs::views::{
    OutputRuntime, PopupSnapshotRuntime, WindowSnapshotRuntime, WorkspaceRuntime,
};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

const DEFAULT_IPC_SOCKET_NAME: &str = "nekoland-ipc.sock";
const IPC_IO_TIMEOUT: Duration = Duration::from_secs(2);

/// Runtime health snapshot of the IPC server socket and its recent failures.
#[allow(missing_docs)]
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

type IpcWorkspaceQuery<'w, 's> = Query<'w, 's, (Entity, WorkspaceRuntime), Allow<Disabled>>;
type IpcWindowQuery<'w, 's> = Query<'w, 's, WindowSnapshotRuntime, Allow<Disabled>>;
type IpcPopupQuery<'w, 's> =
    Query<'w, 's, PopupSnapshotRuntime, (With<PopupSurface>, Allow<Disabled>)>;
type IpcSurfaceQuery<'w, 's> = Query<'w, 's, &'static WlSurfaceHandle, Allow<Disabled>>;

struct IpcRequestDispatchCtx<'a> {
    query_cache: &'a IpcQueryCache,
    app_lifecycle: &'a mut AppLifecycleState,
    config_reload: &'a mut ConfigReloadRequest,
    keyboard_layout_state: &'a mut KeyboardLayoutState,
    pending_external_commands: &'a mut PendingExternalCommandRequests,
    pending_popup_requests: &'a mut PendingPopupServerRequests,
    pending_window_controls: &'a mut PendingWindowControls,
    pending_workspace_controls: &'a mut PendingWorkspaceControls,
    pending_output_controls: &'a mut PendingOutputControls,
}

#[derive(SystemParam)]
pub(crate) struct IpcPumpParams<'w, 's> {
    server_state: ResMut<'w, IpcServerState>,
    query_cache: Res<'w, IpcQueryCache>,
    app_lifecycle: ResMut<'w, AppLifecycleState>,
    config_reload: Option<ResMut<'w, ConfigReloadRequest>>,
    keyboard_layout_state: ResMut<'w, KeyboardLayoutState>,
    pending_subscription_events: ResMut<'w, PendingSubscriptionEvents>,
    pending_external_commands: ResMut<'w, PendingExternalCommandRequests>,
    pending_popup_requests: ResMut<'w, PendingPopupServerRequests>,
    pending_window_controls: ResMut<'w, PendingWindowControls>,
    pending_workspace_controls: ResMut<'w, PendingWorkspaceControls>,
    pending_output_controls: ResMut<'w, PendingOutputControls>,
    _marker: std::marker::PhantomData<&'s ()>,
}

#[derive(SystemParam)]
pub(crate) struct IpcQuerySnapshotInputs<'w, 's> {
    outputs: Query<'w, 's, OutputRuntime>,
    workspaces: IpcWorkspaceQuery<'w, 's>,
    windows: IpcWindowQuery<'w, 's>,
    popups: IpcPopupQuery<'w, 's>,
    surfaces: IpcSurfaceQuery<'w, 's>,
    render_plan: Res<'w, RenderPlan>,
    keyboard_focus: Res<'w, KeyboardFocusState>,
    clock: Res<'w, CompositorClock>,
    command_history: Res<'w, CommandHistoryState>,
    config: Res<'w, CompositorConfig>,
    keyboard_layout_state: Res<'w, KeyboardLayoutState>,
    wayland_ingress: Option<Res<'w, WaylandIngress>>,
    wayland_feedback: Option<Res<'w, WaylandFeedback>>,
    config_source: Option<Res<'w, LoadedConfigSource>>,
    entity_index: Option<Res<'w, EntityIndex>>,
}

/// Materialized IPC query results reused by request/response handlers and subscriptions.
#[allow(missing_docs)]
#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IpcQueryCache {
    pub tree: TreeSnapshot,
    pub outputs: Vec<OutputSnapshot>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub keyboard_layouts: KeyboardLayoutsSnapshot,
    pub commands: Vec<CommandSnapshot>,
    pub config: ConfigSnapshot,
    pub clipboard: ClipboardSnapshot,
    pub primary_selection: PrimarySelectionSnapshot,
    pub present_audit: Vec<PresentAuditOutputSnapshot>,
}

impl IpcQueryCache {
    /// Reuses the latest materialized query snapshots for request/response IPC so query handlers
    /// do not need to walk ECS state independently on every client request.
    pub fn build_reply(&self, command: &QueryCommand) -> IpcReply {
        let payload = match command {
            QueryCommand::GetTree => serde_json::to_value(&self.tree),
            QueryCommand::GetOutputs => serde_json::to_value(&self.outputs),
            QueryCommand::GetWorkspaces => serde_json::to_value(&self.workspaces),
            QueryCommand::GetWindows => serde_json::to_value(&self.tree.windows),
            QueryCommand::GetKeyboardLayouts => serde_json::to_value(&self.keyboard_layouts),
            QueryCommand::GetCommands => serde_json::to_value(&self.commands),
            QueryCommand::GetConfig => serde_json::to_value(&self.config),
            QueryCommand::GetClipboard => serde_json::to_value(&self.clipboard),
            QueryCommand::GetPrimarySelection => serde_json::to_value(&self.primary_selection),
            QueryCommand::GetPresentAudit => serde_json::to_value(&self.present_audit),
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
        request_ctx: &mut IpcRequestDispatchCtx<'_>,
        pending_subscription_events: &mut PendingSubscriptionEvents,
    ) {
        // Run the request/response state machine twice: once to accept/process new input, then a
        // second time after subscription fan-out so queued replies flush in the same frame.
        self.accept_connections(server_state);
        self.process_connections(server_state, request_ctx);
        self.dispatch_subscription_events(pending_subscription_events);
        self.process_connections(server_state, request_ctx);
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
        request_ctx: &mut IpcRequestDispatchCtx<'_>,
    ) {
        let mut keep = Vec::with_capacity(self.connections.len());

        for mut connection in self.connections.drain(..) {
            let should_close = connection.pump(server_state, request_ctx);
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
        if pending_subscription_events.is_empty() {
            return;
        }

        let events = pending_subscription_events.take();
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
        request_ctx: &mut IpcRequestDispatchCtx<'_>,
    ) -> bool {
        if !self.request_processed {
            self.read_request(server_state);
            if let Some(frame) = take_request_frame(&mut self.read_buffer, self.peer_closed) {
                let disposition = match parse_request_frame(&frame) {
                    Ok(request) => reply_for_request(request, request_ctx),
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

/// Returns the default Unix-domain socket path used by the compositor IPC server.
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

/// Sends one IPC request to the default compositor socket and waits for the reply.
pub fn send_request(request: &IpcRequest) -> io::Result<IpcReply> {
    send_request_to_path(&default_socket_path(), request)
}

/// Sends one IPC request to an explicit socket path and waits for the reply.
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
    params: IpcPumpParams<'_, '_>,
) {
    let IpcPumpParams {
        mut server_state,
        query_cache,
        mut app_lifecycle,
        mut config_reload,
        mut keyboard_layout_state,
        mut pending_subscription_events,
        mut pending_external_commands,
        mut pending_popup_requests,
        mut pending_window_controls,
        mut pending_workspace_controls,
        mut pending_output_controls,
        ..
    } = params;
    let Some(config_reload) = config_reload.as_deref_mut() else {
        tracing::warn!("config reload resource was unavailable; skipping ipc accept loop tick");
        return;
    };
    let mut request_ctx = IpcRequestDispatchCtx {
        query_cache: &query_cache,
        app_lifecycle: &mut app_lifecycle,
        config_reload,
        keyboard_layout_state: &mut keyboard_layout_state,
        pending_external_commands: &mut pending_external_commands,
        pending_popup_requests: &mut pending_popup_requests,
        pending_window_controls: &mut pending_window_controls,
        pending_workspace_controls: &mut pending_workspace_controls,
        pending_output_controls: &mut pending_output_controls,
    };

    runtime.pump(&mut server_state, &mut request_ctx, &mut pending_subscription_events);

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

pub(crate) fn refresh_query_cache_system(
    inputs: IpcQuerySnapshotInputs<'_, '_>,
    mut query_cache: ResMut<IpcQueryCache>,
) {
    let IpcQuerySnapshotInputs {
        outputs,
        workspaces,
        windows,
        popups,
        surfaces,
        render_plan,
        keyboard_focus,
        clock,
        command_history,
        config,
        keyboard_layout_state,
        wayland_ingress,
        wayland_feedback,
        config_source,
        entity_index,
    } = inputs;

    let connected_output_names = outputs
        .iter()
        .map(|output| output.device.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let enabled_output_names = outputs
        .iter()
        .map(|output| output.device.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut output_snapshots = outputs
        .iter()
        .map(|output| OutputSnapshot {
            name: output.device.name.clone(),
            kind: output.device.kind.clone(),
            make: output.device.make.clone(),
            model: output.device.model.clone(),
            connected: connected_output_names.contains(&output.device.name),
            enabled: enabled_output_names.contains(&output.device.name),
            width: output.properties.width,
            height: output.properties.height,
            refresh_millihz: output.properties.refresh_millihz,
            scale: output.properties.scale,
            x: output.placement.x,
            y: output.placement.y,
            viewport_origin_x: output.viewport.origin_x as i64,
            viewport_origin_y: output.viewport.origin_y as i64,
            work_area_x: output.work_area.x,
            work_area_y: output.work_area.y,
            work_area_width: output.work_area.width,
            work_area_height: output.work_area.height,
            mode: format_output_mode(
                output.properties.width,
                output.properties.height,
                output.properties.refresh_millihz,
            ),
            current_workspace: output
                .current_workspace
                .as_ref()
                .map(|current_workspace| current_workspace.workspace.0),
        })
        .collect::<Vec<_>>();
    for configured_output in config.outputs.iter() {
        if output_snapshots.iter().any(|output| output.name == configured_output.name) {
            continue;
        }

        let (width, height, refresh_millihz) = parse_output_mode_string(&configured_output.mode);
        output_snapshots.push(OutputSnapshot {
            name: configured_output.name.clone(),
            kind: nekoland_ecs::components::OutputKind::Physical,
            make: String::new(),
            model: String::new(),
            connected: connected_output_names.contains(&configured_output.name),
            enabled: enabled_output_names.contains(&configured_output.name),
            width,
            height,
            refresh_millihz,
            scale: configured_output.scale,
            x: 0,
            y: 0,
            viewport_origin_x: 0,
            viewport_origin_y: 0,
            work_area_x: 0,
            work_area_y: 0,
            work_area_width: width,
            work_area_height: height,
            mode: configured_output.mode.clone(),
            current_workspace: None,
        });
    }
    output_snapshots.sort_by(|left, right| {
        (!left.connected, !left.enabled, left.y, left.x, left.name.as_str()).cmp(&(
            !right.connected,
            !right.enabled,
            right.y,
            right.x,
            right.name.as_str(),
        ))
    });

    let render_order = flattened_render_plan_surface_order(
        &outputs
            .iter()
            .map(|output| {
                (output.placement.y, output.placement.x, output.device.name.clone(), output.id())
            })
            .collect::<Vec<_>>(),
        &render_plan,
    );
    let render_indices = render_order
        .iter()
        .enumerate()
        .map(|(index, surface_id)| (*surface_id, index))
        .collect::<HashMap<_, _>>();
    let focused_workspace = keyboard_focus.focused_surface.and_then(|focused_surface| {
        windows
            .iter()
            .find(|window| window.surface_id() == focused_surface)
            .and_then(|window| window_workspace_runtime_id(window.child_of, &workspaces))
    });
    let workspace_output_names = output_snapshots
        .iter()
        .filter_map(|output| {
            output.current_workspace.map(|workspace| (workspace, output.name.clone()))
        })
        .collect::<HashMap<_, _>>();
    let output_names_by_id = outputs
        .iter()
        .map(|output| (output.id(), output.name().to_owned()))
        .collect::<HashMap<_, _>>();

    let mut window_snapshots = windows
        .iter()
        .map(|window| WindowSnapshot {
            surface_id: window.surface_id(),
            title: window.window.title.clone(),
            app_id: window.window.app_id.clone(),
            xwayland: window.x11_window.is_some(),
            x11_window_id: window.x11_window.map(|x11_window| x11_window.window_id),
            override_redirect: window
                .x11_window
                .is_some_and(|x11_window| x11_window.override_redirect),
            role: window_role_label(*window.role),
            layout: window_layout_label(*window.layout),
            x: window.geometry.x,
            y: window.geometry.y,
            scene_x: window.scene_geometry.x as i64,
            scene_y: window.scene_geometry.y as i64,
            screen_x: window.geometry.x,
            screen_y: window.geometry.y,
            width: window.geometry.width,
            height: window.geometry.height,
            state: window_state_label(*window.layout, *window.mode),
            workspace: window_workspace_runtime_id(window.child_of, &workspaces),
            output: window
                .viewport_visibility
                .output
                .and_then(|output_id| output_names_by_id.get(&output_id).cloned()),
            focused: keyboard_focus.focused_surface == Some(window.surface_id()),
            visible_in_viewport: window.viewport_visibility.visible,
            render_index: render_indices.get(&window.surface_id()).copied(),
        })
        .collect::<Vec<_>>();
    window_snapshots.sort_by(|left, right| {
        (
            left.workspace.unwrap_or(u32::MAX),
            left.output.as_deref().unwrap_or(""),
            left.render_index.unwrap_or(usize::MAX),
            left.scene_y,
            left.scene_x,
            left.surface_id,
        )
            .cmp(&(
                right.workspace.unwrap_or(u32::MAX),
                right.output.as_deref().unwrap_or(""),
                right.render_index.unwrap_or(usize::MAX),
                right.scene_y,
                right.scene_x,
                right.surface_id,
            ))
    });

    let occupied_workspaces = window_snapshots
        .iter()
        .filter_map(|window| window.workspace)
        .collect::<std::collections::BTreeSet<_>>();
    let mut workspace_rows = workspaces
        .iter()
        .map(|(_, workspace)| {
            (workspace.id().0, workspace.name().to_owned(), workspace.is_active())
        })
        .collect::<Vec<_>>();
    workspace_rows.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    query_cache.workspaces = workspace_rows
        .iter()
        .enumerate()
        .map(|(index, (id, name, active))| WorkspaceSnapshot {
            id: *id,
            idx: index as u32,
            name: name.clone(),
            active: *active,
            focused: focused_workspace == Some(*id),
            occupied: occupied_workspaces.contains(id),
            urgent: false,
            output: workspace_output_names.get(id).cloned(),
        })
        .collect();
    let seat_registry =
        wayland_ingress.as_deref().map(|ingress| ingress.seat_registry.clone()).unwrap_or_default();
    let primary_seat_id = seat_registry.primary_seat_id();
    let primary_seat_name =
        seat_name(&seat_registry, primary_seat_id).unwrap_or_default().to_owned();
    query_cache.outputs = output_snapshots;
    query_cache.keyboard_layouts = KeyboardLayoutsSnapshot {
        seat_id: primary_seat_id,
        seat_name: primary_seat_name,
        active_index: keyboard_layout_state.active_index(),
        active_name: keyboard_layout_state.active_name().to_owned(),
        layouts: keyboard_layout_state
            .layouts()
            .iter()
            .map(keyboard_layout_entry_snapshot)
            .collect(),
    };

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
        default_layout: config.default_layout.to_string(),
        focus_follows_mouse: config.focus_follows_mouse,
        repeat_rate: config.repeat_rate,
        configured_keyboard_layout: config.current_keyboard_layout.clone(),
        keyboard_layouts: config
            .keyboard_layouts
            .iter()
            .map(keyboard_layout_entry_snapshot)
            .collect(),
        viewport_pan_modifiers: config.viewport_pan_modifiers.config_tokens(),
        command_history_limit: config.command_history_limit,
        startup_actions: config.startup_actions.clone(),
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
        keybindings: config.keybindings.clone(),
    };

    let clipboard_selection = wayland_feedback
        .as_deref()
        .map(|feedback| feedback.clipboard_selection.clone())
        .unwrap_or_default();
    let primary_selection = wayland_feedback
        .as_deref()
        .map(|feedback| feedback.primary_selection.clone())
        .unwrap_or_default();

    query_cache.clipboard = ClipboardSnapshot {
        seat_id: clipboard_selection.selection.as_ref().map(|selection| selection.seat_id),
        seat_name: clipboard_selection
            .selection
            .as_ref()
            .and_then(|selection| seat_name(&seat_registry, selection.seat_id).map(str::to_owned)),
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
        seat_id: primary_selection.selection.as_ref().map(|selection| selection.seat_id),
        seat_name: primary_selection
            .selection
            .as_ref()
            .and_then(|selection| seat_name(&seat_registry, selection.seat_id).map(str::to_owned)),
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
    query_cache.present_audit = wayland_feedback
        .as_deref()
        .map(|feedback| {
            let present_audit = &feedback.present_audit;
            let mut outputs = present_audit
                .outputs
                .values()
                .map(|output| PresentAuditOutputSnapshot {
                    output_name: output.output_name.clone(),
                    frame: output.frame,
                    uptime_millis: output.uptime_millis,
                    elements: output.elements.iter().map(present_audit_element_snapshot).collect(),
                })
                .collect::<Vec<_>>();
            outputs.sort_by(|left, right| left.output_name.cmp(&right.output_name));
            outputs
        })
        .unwrap_or_default();

    let popups = popups
        .iter()
        .map(|popup| PopupSnapshot {
            surface_id: popup.surface_id(),
            parent_surface_id: popup_parent_surface_id(
                popup.child_of,
                &surfaces,
                entity_index.as_deref(),
            ),
            x: popup.geometry.x,
            y: popup.geometry.y,
            width: popup.geometry.width,
            height: popup.geometry.height,
            grab_active: popup.grab.is_some_and(|grab| grab.active),
            grab_serial: popup.grab.and_then(|grab| grab.serial),
        })
        .collect::<Vec<_>>();

    query_cache.tree = TreeSnapshot {
        frame: clock.frame,
        focused_surface: keyboard_focus.focused_surface,
        outputs: query_cache.outputs.clone(),
        workspaces: query_cache.workspaces.clone(),
        windows: window_snapshots,
        popups,
        render_order,
    };
}

fn seat_name(seat_registry: &SeatRegistry, seat_id: SeatId) -> Option<&str> {
    seat_registry.seat_name(seat_id)
}

fn selection_owner_snapshot(owner: SelectionOwner) -> SelectionOwnerSnapshot {
    match owner {
        SelectionOwner::Client => SelectionOwnerSnapshot::Client,
        SelectionOwner::Compositor => SelectionOwnerSnapshot::Compositor,
    }
}

fn present_audit_element_snapshot(
    element: &nekoland_ecs::resources::PresentAuditElement,
) -> PresentAuditElementSnapshot {
    PresentAuditElementSnapshot {
        surface_id: element.surface_id,
        kind: match element.kind {
            PresentAuditElementKind::Window => "window",
            PresentAuditElementKind::Popup => "popup",
            PresentAuditElementKind::Layer => "layer",
            PresentAuditElementKind::Quad => "quad",
            PresentAuditElementKind::Backdrop => "backdrop",
            PresentAuditElementKind::Compositor => "compositor",
            PresentAuditElementKind::Cursor => "cursor",
            PresentAuditElementKind::Unknown => "unknown",
        }
        .to_owned(),
        x: element.x,
        y: element.y,
        width: element.width,
        height: element.height,
        z_index: element.z_index,
        opacity: element.opacity,
    }
}

fn flattened_render_plan_surface_order(
    output_scene_order: &[(i32, i32, String, nekoland_ecs::components::OutputId)],
    render_plan: &RenderPlan,
) -> Vec<u64> {
    let mut output_scene_order = output_scene_order.to_vec();
    output_scene_order.sort();
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = Vec::new();

    for (_, _, _, output_id) in output_scene_order {
        let Some(output_plan) = render_plan.outputs.get(&output_id) else { continue };
        for item in output_plan.iter_ordered() {
            let RenderPlanItem::Surface(item) = item else {
                continue;
            };
            if seen.insert(item.surface_id) {
                ordered.push(item.surface_id);
            }
        }
    }

    ordered
}

fn keyboard_layout_entry_snapshot(
    layout: &ConfiguredKeyboardLayout,
) -> KeyboardLayoutEntrySnapshot {
    KeyboardLayoutEntrySnapshot {
        name: layout.name.clone(),
        rules: layout.rules.clone(),
        model: layout.model.clone(),
        layout: layout.layout.clone(),
        variant: layout.variant.clone(),
        options: layout.options.clone(),
    }
}

fn window_state_label(layout: WindowLayout, mode: WindowMode) -> String {
    WindowDisplayState::from_layout_mode(layout, mode).label().to_owned()
}

fn window_layout_label(layout: WindowLayout) -> String {
    match layout {
        WindowLayout::Tiled => "tiled".to_owned(),
        WindowLayout::Floating => "floating".to_owned(),
    }
}

fn window_role_label(role: nekoland_ecs::components::WindowRole) -> String {
    match role {
        nekoland_ecs::components::WindowRole::Managed => "managed".to_owned(),
        nekoland_ecs::components::WindowRole::OutputBackground => "output_background".to_owned(),
    }
}

fn format_output_mode(width: u32, height: u32, refresh_millihz: u32) -> String {
    if refresh_millihz.is_multiple_of(1000) {
        format!("{}x{}@{}", width, height, refresh_millihz / 1000)
    } else {
        format!("{}x{}@{:.1}", width, height, refresh_millihz as f64 / 1000.0)
    }
}

fn parse_output_mode_string(mode: &str) -> (u32, u32, u32) {
    let Some((size, refresh)) = mode.split_once('@') else {
        return (0, 0, 0);
    };
    let Some((width, height)) = size.split_once('x') else {
        return (0, 0, 0);
    };
    let width = width.parse::<u32>().unwrap_or(0);
    let height = height.parse::<u32>().unwrap_or(0);
    let refresh_millihz = if let Some((whole, frac)) = refresh.split_once('.') {
        let whole = whole.parse::<u32>().unwrap_or(0);
        let mut frac = frac.chars().take(3).collect::<String>();
        while frac.len() < 3 {
            frac.push('0');
        }
        whole.saturating_mul(1000).saturating_add(frac.parse::<u32>().unwrap_or(0))
    } else {
        refresh.parse::<u32>().unwrap_or(0).saturating_mul(1000)
    };

    (width, height, refresh_millihz)
}

fn popup_parent_surface_id(
    child_of: &ChildOf,
    surfaces: &Query<&WlSurfaceHandle, Allow<Disabled>>,
    entity_index: Option<&EntityIndex>,
) -> u64 {
    entity_index
        .and_then(|entity_index| entity_index.surface_id_for_entity(child_of.parent()))
        .or_else(|| surfaces.get(child_of.parent()).ok().map(|surface| surface.id))
        .unwrap_or_default()
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
    request_ctx: &mut IpcRequestDispatchCtx<'_>,
) -> RequestDisposition {
    let IpcRequestDispatchCtx {
        query_cache,
        app_lifecycle,
        config_reload,
        keyboard_layout_state,
        pending_external_commands,
        pending_popup_requests,
        pending_window_controls,
        pending_workspace_controls,
        pending_output_controls,
    } = request_ctx;
    let keyboard_focus = KeyboardFocusState::default();
    let mut windows = WindowControlApi::new(&keyboard_focus, pending_window_controls);
    let mut workspaces = WorkspaceControlApi::new(pending_workspace_controls);
    let mut outputs = OutputControlApi::new(pending_output_controls);
    match request.command {
        IpcCommand::Query(command) => RequestDisposition::Reply(query_cache.build_reply(&command)),
        IpcCommand::Subscribe(subscription) => RequestDisposition::Subscribe(subscription),
        IpcCommand::Action(ActionCommand::FocusWorkspace { workspace }) => {
            workspaces.switch_or_create(WorkspaceLookup::parse(&workspace));
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued focus workspace action for {workspace}"),
                payload: None,
            })
        }
        IpcCommand::Action(ActionCommand::FocusWindow { id }) => {
            windows.surface(SurfaceId(id)).focus();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued focus window action for surface {id}"),
                payload: None,
            })
        }
        IpcCommand::Action(ActionCommand::CloseWindow { id }) => {
            windows.surface(SurfaceId(id)).close();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued close window action for surface {id}"),
                payload: None,
            })
        }
        IpcCommand::Action(ActionCommand::Spawn { command }) => {
            if command.is_empty() {
                RequestDisposition::Reply(IpcReply {
                    ok: false,
                    message: "spawn action requires at least one argv entry".to_owned(),
                    payload: None,
                })
            } else {
                pending_external_commands.push(ExternalCommandRequest {
                    origin: "ipc action spawn".to_owned(),
                    candidates: vec![command.clone()],
                });
                RequestDisposition::Reply(IpcReply {
                    ok: true,
                    message: format!("queued spawn action for `{}`", command.join(" ")),
                    payload: None,
                })
            }
        }
        IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutNext) => {
            let changed = keyboard_layout_state.activate_next();
            let active_name = keyboard_layout_state.active_name().to_owned();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: if changed {
                    format!("switched keyboard layout to `{active_name}`")
                } else {
                    format!("keyboard layout already on `{active_name}`")
                },
                payload: None,
            })
        }
        IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutPrev) => {
            let changed = keyboard_layout_state.activate_prev();
            let active_name = keyboard_layout_state.active_name().to_owned();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: if changed {
                    format!("switched keyboard layout to `{active_name}`")
                } else {
                    format!("keyboard layout already on `{active_name}`")
                },
                payload: None,
            })
        }
        IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutByName { name }) => {
            if keyboard_layout_state.activate_name(&name) {
                RequestDisposition::Reply(IpcReply {
                    ok: true,
                    message: format!("switched keyboard layout to `{name}`"),
                    payload: None,
                })
            } else if keyboard_layout_state.contains_name(&name) {
                RequestDisposition::Reply(IpcReply {
                    ok: true,
                    message: format!("keyboard layout already on `{name}`"),
                    payload: None,
                })
            } else {
                RequestDisposition::Reply(IpcReply {
                    ok: false,
                    message: format!("unknown keyboard layout `{name}`"),
                    payload: None,
                })
            }
        }
        IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutByIndex { index }) => {
            if index >= keyboard_layout_state.layouts().len() {
                RequestDisposition::Reply(IpcReply {
                    ok: false,
                    message: format!("keyboard layout index {index} is out of range"),
                    payload: None,
                })
            } else {
                let changed = keyboard_layout_state.activate_index(index);
                let active_name = keyboard_layout_state.active_name().to_owned();
                RequestDisposition::Reply(IpcReply {
                    ok: true,
                    message: if changed {
                        format!("switched keyboard layout to `{active_name}`")
                    } else {
                        format!("keyboard layout already on `{active_name}`")
                    },
                    payload: None,
                })
            }
        }
        IpcCommand::Action(ActionCommand::ReloadConfig) => {
            config_reload.requested = true;
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: "queued config reload request".to_owned(),
                payload: None,
            })
        }
        IpcCommand::Action(ActionCommand::Quit) => {
            app_lifecycle.quit_requested = true;
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: "queued compositor quit request".to_owned(),
                payload: None,
            })
        }
        IpcCommand::Action(ActionCommand::PowerOffMonitors) => {
            let output_names = query_cache
                .outputs
                .iter()
                .filter(|output| output.enabled)
                .map(|output| output.name.clone())
                .collect::<Vec<_>>();
            for output in &output_names {
                outputs.select(nekoland_ecs::selectors::OutputSelector::parse(output)).disable();
            }
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued power-off for {} output(s)", output_names.len()),
                payload: None,
            })
        }
        IpcCommand::Action(ActionCommand::PowerOnMonitors) => {
            let output_names = query_cache
                .config
                .outputs
                .iter()
                .filter(|output| output.enabled)
                .map(|output| output.name.clone())
                .collect::<Vec<_>>();
            for output in &output_names {
                outputs.select(nekoland_ecs::selectors::OutputSelector::parse(output)).enable();
            }
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued power-on for {} output(s)", output_names.len()),
                payload: None,
            })
        }
        IpcCommand::Popup(PopupCommand::Dismiss { surface_id }) => {
            pending_popup_requests
                .push(PopupServerRequest { surface_id, action: PopupServerAction::Dismiss });
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued popup dismiss request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Close { surface_id }) => {
            windows.surface(SurfaceId(surface_id)).close();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued close request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Focus { surface_id }) => {
            windows.surface(SurfaceId(surface_id)).focus();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued focus request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Move { surface_id, x, y }) => {
            windows.surface(SurfaceId(surface_id)).move_to(x as isize, y as isize);
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued move request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Resize { surface_id, width, height }) => {
            windows.surface(SurfaceId(surface_id)).resize_to(width, height);
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued resize request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Split { surface_id, axis }) => {
            windows.surface(SurfaceId(surface_id)).split(axis);
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued {axis:?} split request for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::Background { surface_id, output }) => {
            windows.surface(SurfaceId(surface_id)).background_on(OutputName::from(output.clone()));
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued background role for surface {surface_id} on {output}"),
                payload: None,
            })
        }
        IpcCommand::Window(WindowCommand::ClearBackground { surface_id }) => {
            windows.surface(SurfaceId(surface_id)).clear_background();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued background clear for surface {surface_id}"),
                payload: None,
            })
        }
        IpcCommand::Workspace(WorkspaceCommand::Switch { workspace }) => {
            workspaces.switch_or_create(WorkspaceLookup::parse(&workspace));
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued workspace switch to {workspace}"),
                payload: None,
            })
        }
        IpcCommand::Workspace(WorkspaceCommand::Create { workspace }) => {
            workspaces.create_named(WorkspaceName::from(workspace.clone()));
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued workspace create for {workspace}"),
                payload: None,
            })
        }
        IpcCommand::Workspace(WorkspaceCommand::Destroy { workspace }) => {
            workspaces.destroy(WorkspaceSelector::parse(&workspace));
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued workspace destroy for {workspace}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::Configure { output, mode, scale }) => {
            outputs
                .select(nekoland_ecs::selectors::OutputSelector::parse(&output))
                .configure(mode.clone(), scale);
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued output configure for {output}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::Enable { output }) => {
            outputs.select(nekoland_ecs::selectors::OutputSelector::parse(&output)).enable();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued output enable for {output}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::Disable { output }) => {
            outputs.select(nekoland_ecs::selectors::OutputSelector::parse(&output)).disable();
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued output disable for {output}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::ViewportMove { output, x, y }) => {
            outputs
                .select(nekoland_ecs::selectors::OutputSelector::parse(&output))
                .move_viewport_to(x as isize, y as isize);
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued viewport move for {output}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::ViewportPan { output, dx, dy }) => {
            outputs
                .select(nekoland_ecs::selectors::OutputSelector::parse(&output))
                .pan_viewport_by(dx as isize, dy as isize);
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued viewport pan for {output}"),
                payload: None,
            })
        }
        IpcCommand::Output(OutputCommand::CenterViewportOnWindow { output, surface_id }) => {
            outputs
                .select(nekoland_ecs::selectors::OutputSelector::parse(&output))
                .center_viewport_on_window(SurfaceId(surface_id));
            RequestDisposition::Reply(IpcReply {
                ok: true,
                message: format!("queued viewport centering for {output} on window {surface_id}"),
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
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::schedule::Schedule;
    use nekoland_config::{
        ConfigReloadRequest,
        resources::{CompositorConfig, ConfiguredKeyboardLayout, KeyboardLayoutState},
    };
    use nekoland_core::lifecycle::AppLifecycleState;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        BufferState, SurfaceGeometry, WindowAnimation, WindowLayout, WindowMode, WlSurfaceHandle,
        Workspace, WorkspaceId, XdgWindow,
    };
    use nekoland_ecs::resources::PendingPopupServerRequests;
    use nekoland_ecs::resources::SplitAxis;
    use nekoland_ecs::resources::{
        CompositorClock, EntityIndex, KeyboardFocusState, PendingExternalCommandRequests,
        PendingOutputControls, PendingWindowControls, PendingWorkspaceControls,
        PresentAuditElement, PresentAuditElementKind, RenderPlan, WaylandFeedback,
    };

    use super::{
        IpcRequestDispatchCtx, RequestDisposition, refresh_query_cache_system, reply_for_request,
    };
    use super::{event_filter_matches, subscription_matches};
    use crate::commands::{ActionCommand, WindowCommand};
    use crate::subscribe::{IpcSubscription, IpcSubscriptionEvent, SubscriptionTopic};
    use crate::{IpcCommand, IpcQueryCache, IpcReply, IpcRequest};

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

    #[test]
    fn refresh_query_cache_derives_workspace_from_relationship() {
        let mut world = bevy_ecs::world::World::new();
        world.insert_resource(RenderPlan::default());
        world.insert_resource(KeyboardFocusState::default());
        world.insert_resource(CompositorClock::default());
        world.insert_resource(nekoland_ecs::resources::CommandHistoryState::default());
        world.insert_resource(CompositorConfig::default());
        world.insert_resource(KeyboardLayoutState::default());
        world.insert_resource(EntityIndex::default());
        world.insert_resource({
            let mut feedback = WaylandFeedback::default();
            feedback.present_audit.outputs.insert(
                nekoland_ecs::components::OutputId(7),
                nekoland_ecs::resources::OutputPresentAudit {
                    output_name: "Virtual-1".to_owned(),
                    frame: 3,
                    uptime_millis: 33,
                    elements: vec![PresentAuditElement {
                        surface_id: 42,
                        kind: PresentAuditElementKind::Window,
                        x: 10,
                        y: 20,
                        width: 800,
                        height: 600,
                        z_index: 0,
                        opacity: 1.0,
                    }],
                },
            );
            feedback
        });
        world.insert_resource(IpcQueryCache::default());

        let workspace_entity =
            world.spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true }).id();
        world.spawn((
            nekoland_ecs::components::OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: nekoland_ecs::components::OutputKind::Virtual,
                make: "test".to_owned(),
                model: "output".to_owned(),
            },
            nekoland_ecs::components::OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            nekoland_ecs::components::OutputCurrentWorkspace { workspace: WorkspaceId(1) },
        ));
        world.spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 42 },
                geometry: SurfaceGeometry { x: 10, y: 20, width: 800, height: 600 },
                scene_geometry: nekoland_ecs::components::WindowSceneGeometry {
                    x: 10,
                    y: 20,
                    width: 800,
                    height: 600,
                },
                viewport_visibility: Default::default(),
                buffer: BufferState { attached: true, scale: 1 },
                content_version: nekoland_ecs::components::SurfaceContentVersion::default(),
                window: XdgWindow { app_id: "test.app".to_owned(), title: "Test".to_owned() },
                management_hints: Default::default(),
                layout: WindowLayout::Tiled,
                mode: WindowMode::Normal,
                decoration: Default::default(),
                border_theme: Default::default(),
                animation: WindowAnimation::default(),
            },
            ChildOf(workspace_entity),
        ));

        let mut schedule = Schedule::default();
        schedule.add_systems(refresh_query_cache_system);
        schedule.run(&mut world);

        let cache = world.resource::<IpcQueryCache>();
        assert_eq!(cache.tree.windows.len(), 1, "expected one window snapshot");
        assert_eq!(cache.keyboard_layouts.active_name, "us");
        assert_eq!(cache.keyboard_layouts.layouts.len(), 1);
        assert_eq!(
            cache.tree.windows[0].workspace,
            Some(1),
            "window snapshot should derive workspace from ChildOf",
        );
        let Some(live_output) = cache.outputs.iter().find(|output| output.name == "Virtual-1")
        else {
            panic!("expected live Virtual-1 output snapshot");
        };
        assert!(live_output.connected);
        assert_eq!(live_output.current_workspace, Some(1));
        assert_eq!(cache.present_audit.len(), 1, "expected one present-audit snapshot");
        assert_eq!(cache.present_audit[0].output_name, "Virtual-1");
        assert_eq!(cache.present_audit[0].elements.len(), 1);
        assert_eq!(cache.present_audit[0].elements[0].kind, "window");
    }

    #[test]
    fn reply_for_request_stages_window_split_control() {
        let mut pending_popup_requests = PendingPopupServerRequests::default();
        let mut app_lifecycle = AppLifecycleState::default();
        let mut config_reload = ConfigReloadRequest::default();
        let mut keyboard_layout_state = KeyboardLayoutState::default();
        let mut pending_external_commands = PendingExternalCommandRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_workspace_controls = PendingWorkspaceControls::default();
        let mut pending_output_controls = PendingOutputControls::default();
        let query_cache = IpcQueryCache::default();
        let mut request_ctx = IpcRequestDispatchCtx {
            query_cache: &query_cache,
            app_lifecycle: &mut app_lifecycle,
            config_reload: &mut config_reload,
            keyboard_layout_state: &mut keyboard_layout_state,
            pending_external_commands: &mut pending_external_commands,
            pending_popup_requests: &mut pending_popup_requests,
            pending_window_controls: &mut pending_window_controls,
            pending_workspace_controls: &mut pending_workspace_controls,
            pending_output_controls: &mut pending_output_controls,
        };

        let disposition = reply_for_request(
            IpcRequest {
                correlation_id: 1,
                command: IpcCommand::Window(WindowCommand::Split {
                    surface_id: 42,
                    axis: SplitAxis::Vertical,
                }),
            },
            &mut request_ctx,
        );

        assert!(matches!(disposition, RequestDisposition::Reply(_)));
        assert!(!app_lifecycle.quit_requested);
        assert!(!config_reload.requested);
        assert!(pending_external_commands.is_empty());
        assert!(pending_popup_requests.is_empty());
        assert!(pending_workspace_controls.is_empty());
        assert!(pending_output_controls.is_empty());
        assert_eq!(pending_window_controls.as_slice().len(), 1);
        assert_eq!(pending_window_controls.as_slice()[0].surface_id.0, 42);
        assert_eq!(pending_window_controls.as_slice()[0].split_axis, Some(SplitAxis::Vertical));
    }

    #[test]
    fn reply_for_request_stages_window_background_control() {
        let mut pending_popup_requests = PendingPopupServerRequests::default();
        let mut app_lifecycle = AppLifecycleState::default();
        let mut config_reload = ConfigReloadRequest::default();
        let mut keyboard_layout_state = KeyboardLayoutState::default();
        let mut pending_external_commands = PendingExternalCommandRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_workspace_controls = PendingWorkspaceControls::default();
        let mut pending_output_controls = PendingOutputControls::default();
        let query_cache = IpcQueryCache::default();
        let mut request_ctx = IpcRequestDispatchCtx {
            query_cache: &query_cache,
            app_lifecycle: &mut app_lifecycle,
            config_reload: &mut config_reload,
            keyboard_layout_state: &mut keyboard_layout_state,
            pending_external_commands: &mut pending_external_commands,
            pending_popup_requests: &mut pending_popup_requests,
            pending_window_controls: &mut pending_window_controls,
            pending_workspace_controls: &mut pending_workspace_controls,
            pending_output_controls: &mut pending_output_controls,
        };

        let disposition = reply_for_request(
            IpcRequest {
                correlation_id: 2,
                command: IpcCommand::Window(WindowCommand::Background {
                    surface_id: 77,
                    output: "Virtual-1".to_owned(),
                }),
            },
            &mut request_ctx,
        );

        assert!(matches!(disposition, RequestDisposition::Reply(_)));
        assert!(!app_lifecycle.quit_requested);
        assert!(!config_reload.requested);
        assert!(pending_external_commands.is_empty());
        assert!(pending_popup_requests.is_empty());
        assert!(pending_workspace_controls.is_empty());
        assert!(pending_output_controls.is_empty());
        assert_eq!(pending_window_controls.as_slice().len(), 1);
        assert!(matches!(
            pending_window_controls.as_slice()[0].background,
            Some(nekoland_ecs::resources::WindowBackgroundControl::Set { ref output })
                if output.as_str() == "Virtual-1"
        ));
    }

    #[test]
    fn reply_for_request_stages_spawn_action() {
        let mut pending_popup_requests = PendingPopupServerRequests::default();
        let mut app_lifecycle = AppLifecycleState::default();
        let mut config_reload = ConfigReloadRequest::default();
        let mut keyboard_layout_state = KeyboardLayoutState::default();
        let mut pending_external_commands = PendingExternalCommandRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_workspace_controls = PendingWorkspaceControls::default();
        let mut pending_output_controls = PendingOutputControls::default();
        let query_cache = IpcQueryCache::default();
        let mut request_ctx = IpcRequestDispatchCtx {
            query_cache: &query_cache,
            app_lifecycle: &mut app_lifecycle,
            config_reload: &mut config_reload,
            keyboard_layout_state: &mut keyboard_layout_state,
            pending_external_commands: &mut pending_external_commands,
            pending_popup_requests: &mut pending_popup_requests,
            pending_window_controls: &mut pending_window_controls,
            pending_workspace_controls: &mut pending_workspace_controls,
            pending_output_controls: &mut pending_output_controls,
        };

        let disposition = reply_for_request(
            IpcRequest {
                correlation_id: 3,
                command: IpcCommand::Action(ActionCommand::Spawn {
                    command: vec!["foot".to_owned(), "--server".to_owned()],
                }),
            },
            &mut request_ctx,
        );

        assert!(matches!(disposition, RequestDisposition::Reply(_)));
        assert_eq!(pending_external_commands.len(), 1);
        assert_eq!(
            pending_external_commands.as_slice()[0].candidates,
            vec![vec!["foot".to_owned(), "--server".to_owned()]]
        );
        assert!(pending_window_controls.is_empty());
        assert!(pending_workspace_controls.is_empty());
        assert!(pending_output_controls.is_empty());
        assert!(pending_popup_requests.is_empty());
        assert!(!app_lifecycle.quit_requested);
        assert!(!config_reload.requested);
    }

    #[test]
    fn reply_for_request_marks_reload_and_quit_actions() {
        let mut pending_popup_requests = PendingPopupServerRequests::default();
        let mut app_lifecycle = AppLifecycleState::default();
        let mut config_reload = ConfigReloadRequest::default();
        let mut keyboard_layout_state = KeyboardLayoutState::default();
        let mut pending_external_commands = PendingExternalCommandRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_workspace_controls = PendingWorkspaceControls::default();
        let mut pending_output_controls = PendingOutputControls::default();
        let query_cache = IpcQueryCache::default();
        let reload = {
            let mut request_ctx = IpcRequestDispatchCtx {
                query_cache: &query_cache,
                app_lifecycle: &mut app_lifecycle,
                config_reload: &mut config_reload,
                keyboard_layout_state: &mut keyboard_layout_state,
                pending_external_commands: &mut pending_external_commands,
                pending_popup_requests: &mut pending_popup_requests,
                pending_window_controls: &mut pending_window_controls,
                pending_workspace_controls: &mut pending_workspace_controls,
                pending_output_controls: &mut pending_output_controls,
            };
            reply_for_request(
                IpcRequest {
                    correlation_id: 4,
                    command: IpcCommand::Action(ActionCommand::ReloadConfig),
                },
                &mut request_ctx,
            )
        };
        assert!(matches!(reload, RequestDisposition::Reply(_)));
        assert!(config_reload.requested);
        assert!(!app_lifecycle.quit_requested);

        let quit = {
            let mut request_ctx = IpcRequestDispatchCtx {
                query_cache: &query_cache,
                app_lifecycle: &mut app_lifecycle,
                config_reload: &mut config_reload,
                keyboard_layout_state: &mut keyboard_layout_state,
                pending_external_commands: &mut pending_external_commands,
                pending_popup_requests: &mut pending_popup_requests,
                pending_window_controls: &mut pending_window_controls,
                pending_workspace_controls: &mut pending_workspace_controls,
                pending_output_controls: &mut pending_output_controls,
            };
            reply_for_request(
                IpcRequest { correlation_id: 5, command: IpcCommand::Action(ActionCommand::Quit) },
                &mut request_ctx,
            )
        };
        assert!(matches!(quit, RequestDisposition::Reply(_)));
        assert!(app_lifecycle.quit_requested);
    }

    #[test]
    fn reply_for_request_switches_keyboard_layout_state() {
        let mut pending_popup_requests = PendingPopupServerRequests::default();
        let mut app_lifecycle = AppLifecycleState::default();
        let mut config_reload = ConfigReloadRequest::default();
        let mut keyboard_layout_state = KeyboardLayoutState::from_config(
            &[
                ConfiguredKeyboardLayout::default(),
                ConfiguredKeyboardLayout {
                    name: "de".to_owned(),
                    layout: "de".to_owned(),
                    ..ConfiguredKeyboardLayout::default()
                },
            ],
            "us",
        );
        let mut pending_external_commands = PendingExternalCommandRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_workspace_controls = PendingWorkspaceControls::default();
        let mut pending_output_controls = PendingOutputControls::default();
        let query_cache = IpcQueryCache::default();
        let next = {
            let mut request_ctx = IpcRequestDispatchCtx {
                query_cache: &query_cache,
                app_lifecycle: &mut app_lifecycle,
                config_reload: &mut config_reload,
                keyboard_layout_state: &mut keyboard_layout_state,
                pending_external_commands: &mut pending_external_commands,
                pending_popup_requests: &mut pending_popup_requests,
                pending_window_controls: &mut pending_window_controls,
                pending_workspace_controls: &mut pending_workspace_controls,
                pending_output_controls: &mut pending_output_controls,
            };
            reply_for_request(
                IpcRequest {
                    correlation_id: 6,
                    command: IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutNext),
                },
                &mut request_ctx,
            )
        };
        assert!(matches!(next, RequestDisposition::Reply(IpcReply { ok: true, .. })));
        assert_eq!(keyboard_layout_state.active_name(), "de");

        let by_name = {
            let mut request_ctx = IpcRequestDispatchCtx {
                query_cache: &query_cache,
                app_lifecycle: &mut app_lifecycle,
                config_reload: &mut config_reload,
                keyboard_layout_state: &mut keyboard_layout_state,
                pending_external_commands: &mut pending_external_commands,
                pending_popup_requests: &mut pending_popup_requests,
                pending_window_controls: &mut pending_window_controls,
                pending_workspace_controls: &mut pending_workspace_controls,
                pending_output_controls: &mut pending_output_controls,
            };
            reply_for_request(
                IpcRequest {
                    correlation_id: 7,
                    command: IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutByName {
                        name: "us".to_owned(),
                    }),
                },
                &mut request_ctx,
            )
        };
        assert!(matches!(by_name, RequestDisposition::Reply(IpcReply { ok: true, .. })));
        assert_eq!(keyboard_layout_state.active_name(), "us");

        let invalid = {
            let mut request_ctx = IpcRequestDispatchCtx {
                query_cache: &query_cache,
                app_lifecycle: &mut app_lifecycle,
                config_reload: &mut config_reload,
                keyboard_layout_state: &mut keyboard_layout_state,
                pending_external_commands: &mut pending_external_commands,
                pending_popup_requests: &mut pending_popup_requests,
                pending_window_controls: &mut pending_window_controls,
                pending_workspace_controls: &mut pending_workspace_controls,
                pending_output_controls: &mut pending_output_controls,
            };
            reply_for_request(
                IpcRequest {
                    correlation_id: 8,
                    command: IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutByIndex {
                        index: 9,
                    }),
                },
                &mut request_ctx,
            )
        };
        assert!(matches!(invalid, RequestDisposition::Reply(IpcReply { ok: false, .. })));
        assert_eq!(keyboard_layout_state.active_name(), "us");
    }
}
