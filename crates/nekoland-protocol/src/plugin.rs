//! Smithay integration layer that turns Wayland/XWayland callbacks into `ProtocolEvent`s and
//! synchronized compositor-side resources.
//!
//! The file is intentionally large because it owns the runtime glue for globals, seats, outputs,
//! selection handling, presentation feedback, and XWayland bridging.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::process::Stdio;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy_app::App;
use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Has, Local, NonSend, NonSendMut, Query, Res, ResMut, Resource, With};
use bevy_ecs::query::Allow;
use bevy_ecs::schedule::{IntoScheduleConfigs, SystemSet};
use calloop::generic::{FdWrapper, Generic};
use calloop::{Interest, Mode, PostAction};
use nekoland_core::bridge::WaylandBridge;
use nekoland_core::calloop::CalloopSourceRegistry;
use nekoland_core::error::NekolandError;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{ExtractSchedule, PresentSchedule, ProtocolSchedule, RenderSchedule};
use nekoland_ecs::components::{
    DesiredOutputName, LayerOnOutput, LayerShellSurface, OutputBackgroundWindow,
    OutputDevice, OutputPlacement, SurfaceGeometry, WindowMode, WindowViewportVisibility,
    XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{
    BackendInputAction, ClipboardSelectionState, CompositorClock, CompositorConfig,
    DragAndDropState, FramePacingState, GlobalPointerPosition, KeyboardFocusState,
    OutputPresentationState, PendingLayerRequests, PendingOutputEvents,
    PendingPopupServerRequests, PendingProtocolInputEvents, PendingWindowServerRequests,
    PendingX11Requests, PendingXdgRequests, PopupPlacement, PopupServerAction, PrimaryOutputState,
    ResizeEdges, PrimarySelectionState, RenderList, SurfaceExtent, SurfacePresentationRole,
    SurfacePresentationSnapshot, WindowServerAction, X11WindowGeometry, XdgSurfaceRole,
};
use nekoland_ecs::views::{
    OutputRuntime, PopupRuntime, SurfaceRuntime, WindowVisibilityRuntime, WorkspaceRuntime,
};
use smithay::backend::input::{Axis as InputAxis, AxisSource, ButtonState, KeyState};
use smithay::backend::allocator::{Format as DmabufFormat, dmabuf::Dmabuf};
use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::delegate_data_device;
use smithay::delegate_dmabuf;
use smithay::delegate_fractional_scale;
use smithay::delegate_output;
use smithay::delegate_primary_selection;
use smithay::delegate_presentation;
use smithay::delegate_seat;
use smithay::delegate_shm;
use smithay::delegate_viewporter;
use smithay::desktop::utils::{
    SurfacePresentationFeedback, send_frames_surface_tree, under_from_surface_tree,
};
use smithay::desktop::{
    PopupKeyboardGrab, PopupKind as DesktopPopupKind, PopupManager as DesktopPopupManager,
    PopupPointerGrab, WindowSurfaceType, find_popup_root_surface,
};
use smithay::input::keyboard::FilterResult;
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, CursorImageSurfaceData, Focus,
    MotionEvent,
};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as XdgDecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, Display, ListeningSocket, Resource as WaylandResource,
};
use smithay::utils::Serial;
use smithay::utils::{
    Clock, ClockSource, Logical, Monotonic, Point, Rectangle, SERIAL_COUNTER, Size, Time,
    Transform,
};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    self, CompositorClientState, CompositorHandler, CompositorState as SmithayCompositorState,
};
use smithay::wayland::dmabuf::{
    DmabufGlobal, DmabufHandler, DmabufState as SmithayDmabufState, ImportNotifier,
};
use smithay::wayland::fractional_scale::{
    FractionalScaleHandler, FractionalScaleManagerState, with_fractional_scale,
};
use smithay::wayland::output::{OutputHandler, OutputManagerState as SmithayOutputManagerState};
use smithay::wayland::presentation::{PresentationState as SmithayPresentationState, Refresh};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState as SmithayDataDeviceState,
    ServerDndGrabHandler, clear_data_device_selection, request_data_device_client_selection,
    set_data_device_focus, set_data_device_selection, with_source_metadata,
};
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState as SmithayPrimarySelectionState,
    clear_primary_selection, request_primary_client_selection, set_primary_focus,
    set_primary_selection,
};
use smithay::wayland::shell::wlr_layer::{
    Anchor as SmithayLayerAnchor, ExclusiveZone as SmithayExclusiveZone, Layer as SmithayLayer,
    LayerSurface as SmithayLayerSurface, LayerSurfaceCachedState, Margins as SmithayMargins,
    WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shell::xdg::{
    Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
    XdgToplevelSurfaceData,
    XdgShellState as SmithayXdgShellState,
};
use smithay::wayland::shell::xdg::decoration::{
    XdgDecorationHandler, XdgDecorationState as SmithayXdgDecorationState,
};
use smithay::wayland::shm::{ShmHandler, ShmState as SmithayShmState};
use smithay::wayland::viewporter::ViewporterState as SmithayViewporterState;
use smithay::wayland::xwayland_shell::{
    XWaylandShellHandler, XWaylandShellState as SmithayXWaylandShellState,
};
use smithay::xwayland::xwm::{
    Reorder, ResizeEdge, WmWindowProperty, WmWindowType, X11Surface, X11Wm, XwmHandler, XwmId,
};
use smithay::xwayland::{XWayland, XWaylandClientData, XWaylandEvent};
use smithay::{
    delegate_compositor, delegate_layer_shell, delegate_xdg_decoration, delegate_xdg_shell,
    delegate_xwayland_shell,
};

use crate::{
    ProtocolEvent, ProtocolRegistry, ProtocolState, ProtocolSurfaceEntry, ProtocolSurfaceKind,
    ProtocolSurfaceRegistry,
};

type PresentationKind = wp_presentation_feedback::Kind;

const MONOTONIC_CLOCK_ID: u32 = Monotonic::ID as u32;
const DEFAULT_KEYBOARD_REPEAT_DELAY_MS: i32 = 200;
const DEFAULT_KEYBOARD_REPEAT_RATE: u16 = 25;
const MAX_PERSISTED_SELECTION_BYTES: usize = 1024 * 1024;

/// Installs the Smithay runtime and bridges its callback-driven world into the compositor's ECS
/// schedules.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProtocolPlugin;

/// Present-phase system set that updates Smithay seat focus/hit-test state from the current frame.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtocolSeatDispatchSet;

#[derive(Debug, Clone)]
struct SurfaceIdentity(u64);

#[derive(Debug, Clone, Copy)]
struct XdgSurfaceMarker(XdgSurfaceRole);

#[derive(Debug)]
struct SmithayProtocolServer {
    runtime: Option<SharedProtocolRuntime>,
}

type SharedProtocolRuntime = Rc<RefCell<SmithayProtocolRuntime>>;

#[derive(Debug)]
struct SmithayProtocolRuntime {
    display: Display<ProtocolRuntimeState>,
    state: ProtocolRuntimeState,
    xwayland_event_loop: Option<calloop::EventLoop<'static, ProtocolRuntimeState>>,
    socket: Option<ListeningSocket>,
    clients: Vec<Client>,
    last_accept_error: Option<String>,
    last_dispatch_error: Option<String>,
    last_xwayland_error: Option<String>,
}

#[derive(Debug)]
struct ProtocolRuntimeState {
    compositor_state: SmithayCompositorState,
    xdg_shell_state: SmithayXdgShellState,
    _xdg_decoration_state: SmithayXdgDecorationState,
    xwayland_shell_state: SmithayXWaylandShellState,
    layer_shell_state: WlrLayerShellState,
    data_device_state: SmithayDataDeviceState,
    _primary_selection_state: SmithayPrimarySelectionState,
    dmabuf_state: SmithayDmabufState,
    _dmabuf_global: DmabufGlobal,
    _viewporter_state: SmithayViewporterState,
    _fractional_scale_state: FractionalScaleManagerState,
    shm_state: SmithayShmState,
    _presentation_state: SmithayPresentationState,
    _output_manager_state: SmithayOutputManagerState,
    seat_state: SeatState<Self>,
    seat: Seat<Self>,
    primary_output: Output,
    popup_manager: DesktopPopupManager,
    toplevels: HashMap<u64, ToplevelSurface>,
    popups: HashMap<u64, PopupSurface>,
    layers: HashMap<u64, SmithayLayerSurface>,
    xwms: HashMap<XwmId, X11Wm>,
    x11_windows: HashMap<u32, X11Surface>,
    x11_surface_ids_by_window: HashMap<u32, u64>,
    x11_window_ids_by_surface: HashMap<u64, u32>,
    mapped_x11_windows: BTreeSet<u32>,
    published_x11_windows: BTreeSet<u32>,
    xwayland_client: Option<Client>,
    _xwm_connection: Option<UnixStream>,
    mapped_primary_output_name: String,
    event_queue: VecDeque<ProtocolEvent>,
    next_surface_id: u64,
    presentation_sequence: u64,
    synthetic_pointer_grab: Option<SyntheticPointerGrab>,
    selection_persistence: SelectionPersistenceState,
    xwayland_state: XWaylandRuntimeState,
    cursor_state: ProtocolCursorState,
}

#[derive(Debug, Default)]
struct ProtocolClientState {
    compositor_state: CompositorClientState,
}

/// Public status snapshot for the compositor's Wayland protocol server socket.
#[derive(Debug, Clone, Default, Resource)]
pub struct ProtocolServerState {
    pub socket_name: Option<String>,
    pub runtime_dir: Option<String>,
    pub startup_error: Option<String>,
    pub last_accept_error: Option<String>,
    pub last_dispatch_error: Option<String>,
}

/// Public status snapshot for the compositor's XWayland server integration.
#[derive(Debug, Clone, Default, Resource)]
pub struct XWaylandServerState {
    pub enabled: bool,
    pub ready: bool,
    pub display_number: Option<u32>,
    pub display_name: Option<String>,
    pub startup_error: Option<String>,
    pub last_error: Option<String>,
}

/// Protocol-originated cursor image state exposed to present backends.
#[derive(Debug, Clone)]
pub enum ProtocolCursorImage {
    Hidden,
    Named(CursorIcon),
    Surface { surface: WlSurface, hotspot_x: i32, hotspot_y: i32 },
}

/// Latest cursor image requested by the focused client seat.
#[derive(Debug, Clone)]
pub struct ProtocolCursorState {
    pub image: ProtocolCursorImage,
}

impl Default for ProtocolCursorState {
    fn default() -> Self {
        Self { image: ProtocolCursorImage::Named(CursorIcon::Default) }
    }
}

#[derive(Debug, Clone, Copy)]
struct RegisteredRawFd(RawFd);

#[derive(Debug, Clone, Default)]
struct XWaylandRuntimeState {
    enabled: bool,
    ready: bool,
    display_number: Option<u32>,
    display_name: Option<String>,
    startup_error: Option<String>,
}

#[derive(Debug, Default)]
struct WorkspaceVisibilityState {
    initialized: bool,
    active_workspace: Option<u32>,
    visible_toplevels: BTreeSet<u64>,
    visible_popups: BTreeSet<u64>,
}

#[derive(Debug, Clone, Copy)]
struct PointerSurfaceFocus {
    surface_id: u64,
    surface_origin: Point<f64, Logical>,
}

#[derive(Debug, Clone, Copy)]
struct SyntheticPointerGrab {
    serial: u32,
    surface_id: u64,
}

#[derive(Debug, Clone, Copy)]
struct SeatInputSyncState {
    initialized: bool,
    host_focused: bool,
    keyboard_focus: Option<u64>,
    pointer_focus: Option<u64>,
    pointer_location: Point<f64, Logical>,
}

impl Default for SeatInputSyncState {
    fn default() -> Self {
        Self {
            initialized: false,
            host_focused: true,
            keyboard_focus: None,
            pointer_focus: None,
            pointer_location: (0.0, 0.0).into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputTiming {
    output_name: String,
    width: u32,
    height: u32,
    refresh_millihz: u32,
    scale: u32,
}

#[derive(Debug, Clone, Copy)]
struct PresentationFeedbackTiming {
    frame_time: Time<Monotonic>,
    refresh: Refresh,
    sequence: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct PersistedSelectionData {
    mime_data: Arc<HashMap<String, Vec<u8>>>,
}

#[derive(Debug)]
struct PendingSelectionCapture {
    mime_type: String,
    reader: UnixStream,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct SelectionCaptureRequest {
    generation: u64,
    mime_types: Vec<String>,
}

#[derive(Debug, Default)]
struct SelectionCaptureState {
    generation: u64,
    installed_generation: Option<u64>,
    pending_request: Option<SelectionCaptureRequest>,
    active_captures: Vec<PendingSelectionCapture>,
    captured_mime_data: HashMap<String, Vec<u8>>,
}

impl SelectionCaptureState {
    fn note_selection_change(&mut self, mime_types: Vec<String>) {
        self.generation = self.generation.saturating_add(1);
        self.installed_generation = None;
        self.pending_request =
            Some(SelectionCaptureRequest { generation: self.generation, mime_types });
        self.active_captures.clear();
        self.captured_mime_data.clear();
    }
}

#[derive(Debug, Default)]
struct SelectionPersistenceState {
    clipboard: SelectionCaptureState,
    primary: SelectionCaptureState,
}

impl SelectionPersistenceState {
    fn note_selection_change(&mut self, target: SelectionTarget, mime_types: Vec<String>) {
        match target {
            SelectionTarget::Clipboard => self.clipboard.note_selection_change(mime_types),
            SelectionTarget::Primary => self.primary.note_selection_change(mime_types),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum InteractiveRequestKind {
    Move,
    Resize,
    PopupGrab,
}

impl InteractiveRequestKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Move => "xdg_toplevel.move",
            Self::Resize => "xdg_toplevel.resize",
            Self::PopupGrab => "xdg_popup.grab",
        }
    }
}

impl NekolandPlugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        let state = ProtocolState::default();
        let registry = ProtocolRegistry { globals: state.supported_globals() };
        let repeat_rate = app
            .world()
            .get_resource::<CompositorConfig>()
            .map(|config| config.repeat_rate)
            .unwrap_or(DEFAULT_KEYBOARD_REPEAT_RATE);
        let xwayland_enabled = app
            .world()
            .get_resource::<CompositorConfig>()
            .map(|config| config.xwayland.enabled)
            .unwrap_or(true);
        let (server, server_state) = SmithayProtocolServer::new(repeat_rate, xwayland_enabled);
        register_calloop_sources(app, &server);

        app.insert_resource(state)
            .insert_resource(registry)
            .insert_resource(server_state)
            .init_resource::<XWaylandServerState>()
            .insert_non_send_resource(server)
            .insert_non_send_resource(ProtocolSurfaceRegistry::default())
            .insert_non_send_resource(ProtocolCursorState::default())
            .init_resource::<CompositorClock>()
            .init_resource::<PendingProtocolInputEvents>()
            .init_resource::<PendingXdgRequests>()
            .init_resource::<PendingX11Requests>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<PendingPopupServerRequests>()
            .init_resource::<PendingOutputEvents>()
            .init_resource::<ClipboardSelectionState>()
            .init_resource::<DragAndDropState>()
            .init_resource::<PrimarySelectionState>()
            .add_systems(ExtractSchedule, advance_compositor_clock_system)
            .add_systems(
                ProtocolSchedule,
                (
                    sync_protocol_server_state_system,
                    sync_xwayland_server_state_system,
                    sync_keyboard_repeat_config_system,
                    dispatch_xwayland_runtime_system,
                    dispatch_window_server_requests_system,
                    dispatch_popup_server_requests_system,
                    dispatch_surface_frame_callbacks_system,
                    sync_protocol_output_timing_system,
                    process_selection_persistence_system,
                    collect_smithay_callbacks_system,
                    sync_protocol_surface_registry_system,
                    sync_protocol_cursor_state_system,
                    flush_protocol_queue_system,
                )
                    .chain(),
            )
            .add_systems(RenderSchedule, sync_workspace_visibility_system)
            .add_systems(
                PresentSchedule,
                dispatch_seat_input_system.in_set(ProtocolSeatDispatchSet),
            );
    }
}

fn advance_compositor_clock_system(
    mut clock: ResMut<CompositorClock>,
    mut started_at: Local<Option<Instant>>,
) {
    let started_at = started_at.get_or_insert_with(Instant::now);
    clock.frame = clock.frame.saturating_add(1);
    clock.uptime_millis = started_at.elapsed().as_millis();
}

fn sync_protocol_server_state_system(
    server: NonSendMut<SmithayProtocolServer>,
    mut server_state: ResMut<ProtocolServerState>,
) {
    let Some(runtime) = server.runtime.as_ref() else {
        return;
    };

    runtime.borrow().sync_server_state(&mut server_state);
}

fn sync_xwayland_server_state_system(
    server: NonSendMut<SmithayProtocolServer>,
    mut xwayland_state: ResMut<XWaylandServerState>,
) {
    server.sync_xwayland_state(&mut xwayland_state);
}

fn sync_protocol_cursor_state_system(
    server: NonSend<SmithayProtocolServer>,
    mut cursor_state: NonSendMut<ProtocolCursorState>,
) {
    server.sync_cursor_state(&mut cursor_state);
}

fn dispatch_xwayland_runtime_system(mut server: NonSendMut<SmithayProtocolServer>) {
    server.dispatch_xwayland();
}

fn collect_smithay_callbacks_system(
    mut protocol_state: ResMut<ProtocolState>,
    mut server: NonSendMut<SmithayProtocolServer>,
) {
    for event in server.drain_events() {
        protocol_state.queue_event(event);
    }
}

fn process_selection_persistence_system(mut server: NonSendMut<SmithayProtocolServer>) {
    server.process_selection_persistence();
}

fn sync_protocol_surface_registry_system(
    server: NonSendMut<SmithayProtocolServer>,
    mut registry: NonSendMut<ProtocolSurfaceRegistry>,
) {
    server.sync_surface_registry(&mut registry);
}

fn dispatch_window_server_requests_system(
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
    mut server: NonSendMut<SmithayProtocolServer>,
) {
    let mut deferred = Vec::new();

    for request in pending_window_requests.drain() {
        let handled = match request.action {
            WindowServerAction::Close => server.send_close(request.surface_id),
            WindowServerAction::SyncXdgToplevelState { size, fullscreen, maximized } => {
                server.sync_xdg_toplevel_state(request.surface_id, size, fullscreen, maximized)
            }
            WindowServerAction::SyncX11WindowPresentation { geometry, fullscreen, maximized } => {
                server.sync_x11_window_presentation(
                    request.surface_id,
                    geometry,
                    fullscreen,
                    maximized,
                )
            }
        };

        if !handled {
            deferred.push(request);
        }
    }

    pending_window_requests.replace(deferred);
}

fn dispatch_popup_server_requests_system(
    mut pending_popup_requests: ResMut<PendingPopupServerRequests>,
    mut server: NonSendMut<SmithayProtocolServer>,
) {
    let mut deferred = Vec::new();

    for request in pending_popup_requests.drain() {
        let handled = match request.action {
            PopupServerAction::Dismiss => server.dismiss_popup(request.surface_id),
        };

        if !handled {
            deferred.push(request);
        }
    }

    pending_popup_requests.replace(deferred);
}

fn dispatch_surface_frame_callbacks_system(
    outputs: Query<OutputRuntime>,
    output_presentation: Option<Res<OutputPresentationState>>,
    frame_pacing: Res<FramePacingState>,
    mut server: NonSendMut<SmithayProtocolServer>,
) {
    if frame_pacing.callback_surface_ids.is_empty()
        && frame_pacing.presentation_surface_ids.is_empty()
    {
        return;
    }

    let timing = current_output_presentation(&outputs, output_presentation.as_deref())
        .unwrap_or_else(|| {
            let frame_time = Clock::<Monotonic>::new().now();
            let refresh = current_output_timing(&outputs)
                .map(refresh_from_output_timing)
                .unwrap_or(Refresh::Unknown);
            PresentationFeedbackTiming { frame_time, refresh, sequence: None }
        });
    server.send_frame_callbacks(&frame_pacing.callback_surface_ids, timing.frame_time);
    server.send_presentation_feedback(
        &frame_pacing.presentation_surface_ids,
        timing.frame_time,
        timing.refresh,
        timing.sequence,
    );
}

fn flush_protocol_queue_system(
    mut protocol_state: ResMut<ProtocolState>,
    mut pending_xdg_requests: ResMut<PendingXdgRequests>,
    mut pending_layer_requests: ResMut<PendingLayerRequests>,
    mut pending_x11_requests: ResMut<PendingX11Requests>,
    mut pending_output_events: ResMut<PendingOutputEvents>,
    mut clipboard_selection: ResMut<ClipboardSelectionState>,
    mut drag_and_drop: ResMut<DragAndDropState>,
    mut primary_selection: ResMut<PrimarySelectionState>,
) {
    protocol_state.flush_into_ecs(
        &mut pending_xdg_requests,
        &mut pending_layer_requests,
        &mut pending_x11_requests,
        &mut pending_output_events,
        &mut clipboard_selection,
        &mut drag_and_drop,
        &mut primary_selection,
    );
}

fn sync_workspace_visibility_system(
    workspaces: Query<(Entity, WorkspaceRuntime)>,
    windows: Query<
        (Entity, WindowVisibilityRuntime, Has<Disabled>),
        (With<XdgWindow>, Allow<Disabled>),
    >,
    popups: Query<(PopupRuntime, Has<Disabled>), (With<XdgPopup>, Allow<Disabled>)>,
    surface_presentation: Option<Res<SurfacePresentationSnapshot>>,
    mut visibility: Local<WorkspaceVisibilityState>,
    mut server: NonSendMut<SmithayProtocolServer>,
) {
    let (_, active_workspace) =
        nekoland_ecs::workspace_membership::active_workspace_runtime_target(&workspaces);
    let surface_presentation = surface_presentation.as_deref();
    let visible_toplevels = windows
        .iter()
        .filter(|(_, window, disabled)| {
            !disabled
                && surface_presentation.map_or_else(
                    || {
                        *window.mode != WindowMode::Hidden
                            && window.viewport_visibility.visible
                            && window.background.is_none()
                    },
                    |snapshot| {
                        snapshot.surfaces.get(&window.surface_id()).is_some_and(|state| {
                            state.visible && state.role == SurfacePresentationRole::Window
                        })
                    },
                )
        })
        .map(|(_, window, _)| window.surface_id())
        .collect::<BTreeSet<_>>();
    let visible_toplevel_entities = windows
        .iter()
        .filter(|(_, window, disabled)| {
            !disabled
                && surface_presentation.map_or_else(
                    || {
                        *window.mode != WindowMode::Hidden
                            && window.viewport_visibility.visible
                            && window.background.is_none()
                    },
                    |snapshot| {
                        snapshot.surfaces.get(&window.surface_id()).is_some_and(|state| {
                            state.visible && state.role == SurfacePresentationRole::Window
                        })
                    },
                )
        })
        .map(|(entity, _, _)| entity)
        .collect::<BTreeSet<_>>();
    let visible_popups = popups
        .iter()
        .filter(|(popup, disabled)| {
            !disabled
                && surface_presentation.map_or_else(
                    || {
                        popup.buffer.attached
                            && popup_parent_visible(popup.child_of, &visible_toplevel_entities)
                    },
                    |snapshot| {
                        snapshot.surfaces.get(&popup.surface_id()).is_some_and(|state| {
                            state.visible && state.role == SurfacePresentationRole::Popup
                        })
                    },
                )
        })
        .map(|(popup, _)| popup.surface_id())
        .collect::<BTreeSet<_>>();
    let hidden_parent_popups = popups
        .iter()
        .filter(|(popup, disabled)| {
            !disabled && !visible_toplevel_entities.contains(&popup.child_of.parent())
        })
        .map(|(popup, _)| popup.surface_id())
        .collect::<BTreeSet<_>>();

    if !visibility.initialized {
        visibility.initialized = true;
        visibility.active_workspace = active_workspace;
        visibility.visible_toplevels = visible_toplevels;
        visibility.visible_popups = visible_popups;
        return;
    }

    let dismissed_popups = visibility
        .visible_popups
        .difference(&visible_popups)
        .copied()
        .chain(hidden_parent_popups.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let activated_toplevels =
        visible_toplevels.difference(&visibility.visible_toplevels).copied().collect::<Vec<_>>();

    if visibility.active_workspace != active_workspace
        || visibility.visible_toplevels != visible_toplevels
        || visibility.visible_popups != visible_popups
    {
        server.sync_workspace_visibility(&activated_toplevels, &dismissed_popups);
    }

    visibility.active_workspace = active_workspace;
    visibility.visible_toplevels = visible_toplevels;
    visibility.visible_popups = visible_popups;
}

fn popup_parent_visible(child_of: &ChildOf, visible_toplevel_entities: &BTreeSet<Entity>) -> bool {
    visible_toplevel_entities.contains(&child_of.parent())
}

fn dispatch_seat_input_system(
    clock: Res<CompositorClock>,
    keyboard_focus: Option<Res<KeyboardFocusState>>,
    pointer: Option<Res<GlobalPointerPosition>>,
    render_list: Option<Res<RenderList>>,
    surface_presentation: Option<Res<SurfacePresentationSnapshot>>,
    primary_output: Option<Res<PrimaryOutputState>>,
    mut pending_protocol_input_events: ResMut<PendingProtocolInputEvents>,
    outputs: Query<(Entity, &OutputDevice, &OutputPlacement)>,
    windows: Query<
        (
            Entity,
            SurfaceRuntime,
            Option<&WindowViewportVisibility>,
            Option<&OutputBackgroundWindow>,
        ),
        With<XdgWindow>,
    >,
    popups: Query<(SurfaceRuntime, &ChildOf), With<XdgPopup>>,
    layers: Query<
        (SurfaceRuntime, Option<&LayerOnOutput>, Option<&DesiredOutputName>),
        With<LayerShellSurface>,
    >,
    mut seat_sync: Local<SeatInputSyncState>,
    mut server: NonSendMut<SmithayProtocolServer>,
) {
    if !seat_sync.initialized {
        seat_sync.initialized = true;
        seat_sync.host_focused = true;
    }

    let time = compositor_time_millis(&clock);
    let keyboard_focus = keyboard_focus.as_deref();
    let pointer = pointer.as_deref();
    let render_list = render_list.as_deref();
    let surface_presentation = surface_presentation.as_deref();

    for event in pending_protocol_input_events.drain() {
        sync_keyboard_focus_if_needed(&mut server, &mut seat_sync, keyboard_focus);

        match event.action {
            BackendInputAction::FocusChanged { focused } => {
                seat_sync.host_focused = focused;
                sync_keyboard_focus_if_needed(&mut server, &mut seat_sync, keyboard_focus);
                sync_pointer_focus_if_needed(
                    &mut server,
                    &mut seat_sync,
                    pointer,
                    render_list,
                    surface_presentation,
                    primary_output.as_deref(),
                    &outputs,
                    &windows,
                    &popups,
                    &layers,
                    time,
                );
            }
            BackendInputAction::Key { keycode, pressed } => {
                sync_keyboard_focus_if_needed(&mut server, &mut seat_sync, keyboard_focus);
                if seat_sync.host_focused {
                    server.dispatch_keyboard_input(keycode, pressed, time);
                }
            }
            BackendInputAction::PointerMoved { .. } | BackendInputAction::PointerDelta { .. } => {
                sync_pointer_focus_if_needed(
                    &mut server,
                    &mut seat_sync,
                    pointer,
                    render_list,
                    surface_presentation,
                    primary_output.as_deref(),
                    &outputs,
                    &windows,
                    &popups,
                    &layers,
                    time,
                );
            }
            BackendInputAction::PointerButton { button_code, pressed } => {
                sync_pointer_focus_if_needed(
                    &mut server,
                    &mut seat_sync,
                    pointer,
                    render_list,
                    surface_presentation,
                    primary_output.as_deref(),
                    &outputs,
                    &windows,
                    &popups,
                    &layers,
                    time,
                );
                if seat_sync.host_focused {
                    server.dispatch_pointer_button(
                        button_code,
                        pressed,
                        time,
                        seat_sync.pointer_focus,
                    );
                }
            }
            BackendInputAction::PointerAxis { horizontal, vertical } => {
                sync_pointer_focus_if_needed(
                    &mut server,
                    &mut seat_sync,
                    pointer,
                    render_list,
                    surface_presentation,
                    primary_output.as_deref(),
                    &outputs,
                    &windows,
                    &popups,
                    &layers,
                    time,
                );
                if seat_sync.host_focused {
                    server.dispatch_pointer_axis(horizontal, vertical, time);
                }
            }
        }
    }

    sync_keyboard_focus_if_needed(&mut server, &mut seat_sync, keyboard_focus);
    sync_pointer_focus_if_needed(
        &mut server,
        &mut seat_sync,
        pointer,
        render_list,
        surface_presentation,
        primary_output.as_deref(),
        &outputs,
        &windows,
        &popups,
        &layers,
        time,
    );
}

impl SmithayProtocolServer {
    fn new(repeat_rate: u16, xwayland_enabled: bool) -> (Self, ProtocolServerState) {
        let mut server_state = ProtocolServerState::default();

        let runtime = match Display::new() {
            Ok(display) => {
                let display_handle = display.handle();
                let state = ProtocolRuntimeState::new(&display_handle, repeat_rate);
                let socket = match bind_wayland_socket() {
                    Ok((socket, socket_name)) => {
                        let socket_name = socket_name.to_string_lossy().into_owned();
                        tracing::info!(socket = %socket_name, "Wayland display socket ready");
                        server_state.socket_name = Some(socket_name);
                        server_state.runtime_dir = current_wayland_runtime_dir();
                        Some(socket)
                    }
                    Err(error) => {
                        let error = error.to_string();
                        tracing::warn!(error = %error, "failed to create Wayland display socket");
                        server_state.startup_error = Some(error);
                        None
                    }
                };

                let runtime = Rc::new(RefCell::new(SmithayProtocolRuntime {
                    display,
                    state,
                    xwayland_event_loop: None,
                    socket,
                    clients: Vec::new(),
                    last_accept_error: None,
                    last_dispatch_error: None,
                    last_xwayland_error: None,
                }));
                runtime.borrow_mut().initialize_xwayland(xwayland_enabled);
                Some(runtime)
            }
            Err(error) => {
                let error = error.to_string();
                tracing::warn!(error = %error, "failed to initialize Wayland display");
                server_state.startup_error = Some(error);
                None
            }
        };

        (Self { runtime }, server_state)
    }

    fn drain_events(&mut self) -> Vec<ProtocolEvent> {
        self.runtime.as_ref().map(|runtime| runtime.borrow_mut().drain_events()).unwrap_or_default()
    }

    fn sync_surface_registry(&self, registry: &mut ProtocolSurfaceRegistry) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow().state.sync_surface_registry(registry);
        } else {
            registry.surfaces.clear();
        }
    }

    fn sync_cursor_state(&self, cursor_state: &mut ProtocolCursorState) {
        if let Some(runtime) = self.runtime.as_ref() {
            *cursor_state = runtime.borrow().state.cursor_state.clone();
        } else {
            *cursor_state = ProtocolCursorState::default();
        }
    }

    fn sync_xwayland_state(&self, state: &mut XWaylandServerState) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow().sync_xwayland_state(state);
        } else {
            *state = XWaylandServerState::default();
        }
    }

    fn process_selection_persistence(&mut self) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().process_selection_persistence();
        }
    }

    fn dispatch_xwayland(&mut self) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_xwayland();
        }
    }

    fn send_close(&mut self, surface_id: u64) -> bool {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.borrow_mut().send_close(surface_id))
            .unwrap_or(false)
    }

    fn pointer_focus_candidate_accepts(
        &self,
        surface_id: u64,
        location: Point<f64, Logical>,
        surface_origin: Point<f64, Logical>,
    ) -> bool {
        self.runtime
            .as_ref()
            .map(|runtime| {
                runtime.borrow().pointer_focus_candidate_accepts(
                    surface_id,
                    location,
                    surface_origin,
                )
            })
            .unwrap_or(true)
    }

    fn sync_xdg_toplevel_state(
        &mut self,
        surface_id: u64,
        size: Option<SurfaceExtent>,
        fullscreen: bool,
        maximized: bool,
    ) -> bool {
        self.runtime
            .as_ref()
            .map(|runtime| {
                runtime
                    .borrow_mut()
                    .sync_xdg_toplevel_state(surface_id, size, fullscreen, maximized)
            })
            .unwrap_or(false)
    }

    fn sync_x11_window_presentation(
        &mut self,
        surface_id: u64,
        geometry: X11WindowGeometry,
        fullscreen: bool,
        maximized: bool,
    ) -> bool {
        self.runtime
            .as_ref()
            .map(|runtime| {
                runtime
                    .borrow_mut()
                    .sync_x11_window_presentation(surface_id, geometry, fullscreen, maximized)
            })
            .unwrap_or(false)
    }

    fn dismiss_popup(&mut self, surface_id: u64) -> bool {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.borrow_mut().dismiss_popup(surface_id))
            .unwrap_or(false)
    }

    fn sync_keyboard_focus(&mut self, surface_id: Option<u64>) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().sync_keyboard_focus(surface_id);
        }
    }

    fn dispatch_keyboard_input(&mut self, keycode: u32, pressed: bool, time: u32) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_keyboard_input(keycode, pressed, time);
        }
    }

    fn dispatch_pointer_motion(
        &mut self,
        focus: Option<PointerSurfaceFocus>,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_pointer_motion(focus, location, time);
        }
    }

    fn dispatch_pointer_button(
        &mut self,
        button_code: u32,
        pressed: bool,
        time: u32,
        focus_surface_id: Option<u64>,
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_pointer_button(
                button_code,
                pressed,
                time,
                focus_surface_id,
            );
        }
    }

    fn dispatch_pointer_axis(&mut self, horizontal: f64, vertical: f64, time: u32) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_pointer_axis(horizontal, vertical, time);
        }
    }

    fn sync_workspace_visibility(&mut self, activated_toplevels: &[u64], dismissed_popups: &[u64]) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().sync_workspace_visibility(activated_toplevels, dismissed_popups);
        }
    }

    fn send_frame_callbacks(&mut self, surface_ids: &[u64], frame_time: Time<Monotonic>) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().send_frame_callbacks(surface_ids, frame_time);
        }
    }

    fn send_presentation_feedback(
        &mut self,
        surface_ids: &[u64],
        frame_time: Time<Monotonic>,
        refresh: Refresh,
        sequence: Option<u64>,
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().send_presentation_feedback(
                surface_ids,
                frame_time,
                refresh,
                sequence,
            );
        }
    }

    fn sync_output_timing(&mut self, output_timing: OutputTiming) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().sync_output_timing(output_timing);
        }
    }
}

fn sync_protocol_output_timing_system(
    outputs: Query<OutputRuntime>,
    mut last_output_timing: Local<Option<OutputTiming>>,
    mut server: NonSendMut<SmithayProtocolServer>,
) {
    if let Some(output_timing) = current_output_timing(&outputs) {
        if last_output_timing.as_ref() != Some(&output_timing) {
            server.sync_output_timing(output_timing.clone());
            *last_output_timing = Some(output_timing);
        }
    }
}

enum SelectionCapturePoll {
    Pending(PendingSelectionCapture),
    Complete { mime_type: String, bytes: Vec<u8> },
    Drop,
}

fn poll_selection_capture(mut capture: PendingSelectionCapture) -> SelectionCapturePoll {
    loop {
        let mut buffer = [0_u8; 4096];
        match capture.reader.read(&mut buffer) {
            Ok(0) => {
                return SelectionCapturePoll::Complete {
                    mime_type: capture.mime_type,
                    bytes: capture.bytes,
                };
            }
            Ok(read) => {
                capture.bytes.extend_from_slice(&buffer[..read]);
                if capture.bytes.len() > MAX_PERSISTED_SELECTION_BYTES {
                    tracing::warn!(
                        %capture.mime_type,
                        limit = MAX_PERSISTED_SELECTION_BYTES,
                        "dropping oversized persisted selection payload"
                    );
                    return SelectionCapturePoll::Drop;
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                return SelectionCapturePoll::Pending(capture);
            }
            Err(error) => {
                tracing::warn!(
                    %capture.mime_type,
                    %error,
                    "failed while reading persisted selection payload"
                );
                return SelectionCapturePoll::Drop;
            }
        }
    }
}

fn selection_target_name(target: SelectionTarget) -> &'static str {
    match target {
        SelectionTarget::Clipboard => "clipboard",
        SelectionTarget::Primary => "primary-selection",
    }
}

fn sync_keyboard_repeat_config_system(
    server: NonSendMut<SmithayProtocolServer>,
    config: Res<CompositorConfig>,
    mut last_repeat_rate: Local<Option<u16>>,
) {
    if *last_repeat_rate == Some(config.repeat_rate) {
        return;
    }

    let Some(runtime) = server.runtime.as_ref() else {
        return;
    };

    runtime.borrow_mut().sync_keyboard_repeat_info(config.repeat_rate);
    *last_repeat_rate = Some(config.repeat_rate);
}

impl SmithayProtocolRuntime {
    fn initialize_xwayland(&mut self, enabled: bool) {
        if !enabled {
            tracing::info!("XWayland startup disabled by config");
            return;
        }

        let event_loop = match calloop::EventLoop::<ProtocolRuntimeState>::try_new() {
            Ok(event_loop) => event_loop,
            Err(error) => {
                self.state.xwayland_state.startup_error = Some(error.to_string());
                return;
            }
        };

        let (xwayland, client) = match XWayland::spawn(
            &self.display.handle(),
            None,
            std::iter::empty::<(&str, &str)>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| {},
        ) {
            Ok(spawned) => spawned,
            Err(error) => {
                self.state.xwayland_state.startup_error = Some(error.to_string());
                return;
            }
        };

        self.state.xwayland_state.enabled = true;
        self.state.xwayland_client = Some(client);
        let event_loop_handle = event_loop.handle();
        let callback_handle = event_loop_handle.clone();
        match event_loop_handle.insert_source(xwayland, move |event, _, state| {
            state.handle_xwayland_event(callback_handle.clone(), event);
        }) {
            Ok(_) => {
                self.xwayland_event_loop = Some(event_loop);
            }
            Err(error) => {
                self.state.xwayland_state.startup_error = Some(error.error.to_string());
            }
        }
    }

    fn on_socket_ready(&mut self) {
        self.accept_pending_clients();
        self.dispatch_clients();
    }

    fn on_display_ready(&mut self) {
        self.dispatch_clients();
    }

    fn accept_pending_clients(&mut self) {
        let Some(socket) = self.socket.as_ref() else {
            return;
        };

        loop {
            match socket.accept() {
                Ok(Some(stream)) => {
                    let mut handle = self.display.handle();
                    match handle.insert_client(stream, Arc::new(ProtocolClientState::default())) {
                        Ok(client) => {
                            self.clients.push(client);
                            self.last_accept_error = None;
                        }
                        Err(error) => {
                            remember_protocol_error(
                                &mut self.last_accept_error,
                                error,
                                "failed to register Wayland client",
                            );
                            break;
                        }
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    remember_protocol_error(
                        &mut self.last_accept_error,
                        error,
                        "failed to accept Wayland client",
                    );
                    break;
                }
            }
        }
    }

    fn dispatch_clients(&mut self) {
        match self.display.dispatch_clients(&mut self.state) {
            Ok(_) => match self.display.flush_clients() {
                Ok(()) => {
                    self.last_dispatch_error = None;
                }
                Err(error) => {
                    remember_protocol_error(
                        &mut self.last_dispatch_error,
                        error,
                        "failed to flush Wayland clients",
                    );
                }
            },
            Err(error) => {
                remember_protocol_error(
                    &mut self.last_dispatch_error,
                    error,
                    "failed to dispatch Wayland clients",
                );
            }
        }
        self.state.popup_manager.cleanup();
    }

    fn process_selection_persistence(&mut self) {
        self.process_selection_capture_requests(SelectionTarget::Clipboard);
        self.process_selection_capture_requests(SelectionTarget::Primary);
        self.poll_selection_captures(SelectionTarget::Clipboard);
        self.poll_selection_captures(SelectionTarget::Primary);
    }

    fn process_selection_capture_requests(&mut self, target: SelectionTarget) {
        let Some(request) = self.capture_state_mut(target).pending_request.take() else {
            return;
        };

        let capture_state = self.capture_state_mut(target);
        capture_state.active_captures.clear();
        capture_state.captured_mime_data.clear();
        capture_state.installed_generation = None;

        if request.mime_types.is_empty() {
            self.clear_persisted_selection(target);
            return;
        }

        let mut scheduled = Vec::new();
        for mime_type in request.mime_types.into_iter().collect::<BTreeSet<_>>() {
            let Ok((reader, writer)) = UnixStream::pair() else {
                tracing::warn!(selection = selection_target_name(target), %mime_type, "failed to allocate selection persistence pipe");
                continue;
            };
            if let Err(error) = reader.set_nonblocking(true) {
                tracing::warn!(
                    selection = selection_target_name(target),
                    %mime_type,
                    %error,
                    "failed to configure selection persistence reader"
                );
                continue;
            }

            let request_failed = match target {
                SelectionTarget::Clipboard => request_data_device_client_selection::<
                    ProtocolRuntimeState,
                >(
                    &self.state.seat, mime_type.clone(), writer.into()
                )
                .map_err(|error| error.to_string()),
                SelectionTarget::Primary => {
                    request_primary_client_selection::<ProtocolRuntimeState>(
                        &self.state.seat,
                        mime_type.clone(),
                        writer.into(),
                    )
                    .map_err(|error| error.to_string())
                }
            };

            if let Err(error) = request_failed {
                tracing::debug!(
                    selection = selection_target_name(target),
                    %mime_type,
                    %error,
                    "selection persistence request was not accepted"
                );
                continue;
            }

            scheduled.push(PendingSelectionCapture { mime_type, reader, bytes: Vec::new() });
        }

        self.capture_state_mut(target).active_captures = scheduled;
        self.capture_state_mut(target).generation = request.generation;

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after scheduling selection persistence",
            );
        }
    }

    fn poll_selection_captures(&mut self, target: SelectionTarget) {
        let generation = self.capture_state_mut(target).generation;
        let captures = std::mem::take(&mut self.capture_state_mut(target).active_captures);
        let mut pending = Vec::new();

        for capture in captures {
            match poll_selection_capture(capture) {
                SelectionCapturePoll::Pending(capture) => pending.push(capture),
                SelectionCapturePoll::Complete { mime_type, bytes } => {
                    self.capture_state_mut(target).captured_mime_data.insert(mime_type, bytes);
                }
                SelectionCapturePoll::Drop => {}
            }
        }

        self.capture_state_mut(target).active_captures = pending;
        let should_install = {
            let state = self.capture_state_mut(target);
            state.active_captures.is_empty()
                && !state.captured_mime_data.is_empty()
                && state.installed_generation != Some(generation)
        };

        if !should_install {
            return;
        }

        let persisted = PersistedSelectionData {
            mime_data: Arc::new(self.capture_state_mut(target).captured_mime_data.clone()),
        };
        self.install_persisted_selection(target, persisted);
        self.capture_state_mut(target).installed_generation = Some(generation);
    }

    fn install_persisted_selection(
        &mut self,
        target: SelectionTarget,
        persisted: PersistedSelectionData,
    ) {
        let mime_types = persisted.mime_data.keys().cloned().collect::<Vec<_>>();
        match target {
            SelectionTarget::Clipboard => set_data_device_selection::<ProtocolRuntimeState>(
                &self.display.handle(),
                &self.state.seat,
                mime_types.clone(),
                persisted,
            ),
            SelectionTarget::Primary => set_primary_selection::<ProtocolRuntimeState>(
                &self.display.handle(),
                &self.state.seat,
                mime_types.clone(),
                persisted,
            ),
        }
        match target {
            SelectionTarget::Clipboard => {
                self.state.event_queue.push_back(ProtocolEvent::ClipboardSelectionPersisted {
                    persisted_mime_types: mime_types,
                });
            }
            SelectionTarget::Primary => {
                self.state.event_queue.push_back(ProtocolEvent::PrimarySelectionPersisted {
                    persisted_mime_types: mime_types,
                });
            }
        }

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after installing persisted selection",
            );
        }
    }

    fn clear_persisted_selection(&mut self, target: SelectionTarget) {
        match target {
            SelectionTarget::Clipboard => {
                clear_data_device_selection::<ProtocolRuntimeState>(
                    &self.display.handle(),
                    &self.state.seat,
                );
            }
            SelectionTarget::Primary => {
                clear_primary_selection::<ProtocolRuntimeState>(
                    &self.display.handle(),
                    &self.state.seat,
                );
            }
        }

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after clearing persisted selection",
            );
        }
    }

    fn capture_state_mut(&mut self, target: SelectionTarget) -> &mut SelectionCaptureState {
        match target {
            SelectionTarget::Clipboard => &mut self.state.selection_persistence.clipboard,
            SelectionTarget::Primary => &mut self.state.selection_persistence.primary,
        }
    }

    fn dispatch_xwayland(&mut self) {
        let Some(event_loop) = self.xwayland_event_loop.as_mut() else {
            return;
        };

        match event_loop.dispatch(Duration::ZERO, &mut self.state) {
            Ok(()) => {
                self.last_xwayland_error = None;
            }
            Err(error) => {
                remember_protocol_error(
                    &mut self.last_xwayland_error,
                    error,
                    "failed to dispatch XWayland runtime",
                );
            }
        }
    }

    fn drain_events(&mut self) -> Vec<ProtocolEvent> {
        self.state.event_queue.drain(..).collect()
    }

    fn send_close(&mut self, surface_id: u64) -> bool {
        let handled = if let Some(toplevel) = self.state.toplevels.get(&surface_id).cloned() {
            toplevel.send_close();
            true
        } else if let Some(window_id) =
            self.state.x11_window_ids_by_surface.get(&surface_id).copied()
        {
            let Some(window) = self.state.x11_windows.get(&window_id).cloned() else {
                return false;
            };

            if let Err(error) = window.close() {
                remember_protocol_error(
                    &mut self.last_xwayland_error,
                    error,
                    "failed to send X11 close request",
                );
                return false;
            }
            true
        } else {
            false
        };

        if handled {
            if let Err(error) = self.display.flush_clients() {
                remember_protocol_error(
                    &mut self.last_dispatch_error,
                    error,
                    "failed to flush Wayland clients after sending close",
                );
            }
        }

        handled
    }

    fn sync_xdg_toplevel_state(
        &mut self,
        surface_id: u64,
        size: Option<SurfaceExtent>,
        fullscreen: bool,
        maximized: bool,
    ) -> bool {
        let handled = if let Some(toplevel) = self.state.toplevels.get(&surface_id).cloned() {
            toplevel.with_pending_state(|state| {
                state.size = size.map(|size| {
                    Size::<i32, Logical>::from((
                        size.width.max(1) as i32,
                        size.height.max(1) as i32,
                    ))
                });
                if fullscreen {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                }
                if maximized {
                    state.states.set(xdg_toplevel::State::Maximized);
                } else {
                    state.states.unset(xdg_toplevel::State::Maximized);
                }
            });
            toplevel.send_configure();
            true
        } else {
            false
        };

        if handled {
            if let Err(error) = self.display.flush_clients() {
                remember_protocol_error(
                    &mut self.last_dispatch_error,
                    error,
                    "failed to flush Wayland clients after syncing XDG toplevel state",
                );
            }
        }

        handled
    }

    fn sync_x11_window_presentation(
        &mut self,
        surface_id: u64,
        geometry: X11WindowGeometry,
        fullscreen: bool,
        maximized: bool,
    ) -> bool {
        let handled = if let Some(window_id) =
            self.state.x11_window_ids_by_surface.get(&surface_id).copied()
        {
            let Some(window) = self.state.x11_windows.get(&window_id).cloned() else {
                return false;
            };

            if let Err(error) = window.set_fullscreen(fullscreen) {
                remember_protocol_error(
                    &mut self.last_xwayland_error,
                    error,
                    "failed to sync X11 fullscreen state",
                );
                return false;
            }
            if let Err(error) = window.set_maximized(maximized) {
                remember_protocol_error(
                    &mut self.last_xwayland_error,
                    error,
                    "failed to sync X11 maximized state",
                );
                return false;
            }
            if !window.is_override_redirect() {
                let rect = Rectangle::new(
                    Point::from((geometry.x, geometry.y)),
                    Size::from((geometry.width.max(1) as i32, geometry.height.max(1) as i32)),
                );
                if let Err(error) = window.configure(rect) {
                    remember_protocol_error(
                        &mut self.last_xwayland_error,
                        error,
                        "failed to sync X11 window geometry",
                    );
                    return false;
                }
            }
            true
        } else {
            false
        };

        if handled {
            if let Err(error) = self.display.flush_clients() {
                remember_protocol_error(
                    &mut self.last_dispatch_error,
                    error,
                    "failed to flush Wayland clients after syncing X11 window presentation",
                );
            }
        }

        handled
    }

    fn dismiss_popup(&mut self, surface_id: u64) -> bool {
        let Some(popup) = self.state.popups.get(&surface_id).cloned() else {
            return false;
        };

        let dismissed = self.dismiss_popup_surface(surface_id, &popup);
        if dismissed {
            self.state.queue_event(ProtocolEvent::SurfaceDestroyed {
                surface_id,
                role: XdgSurfaceRole::Popup,
            });
            if let Err(error) = self.display.flush_clients() {
                remember_protocol_error(
                    &mut self.last_dispatch_error,
                    error,
                    "failed to flush Wayland clients after dismissing popup",
                );
            }
            self.state.popup_manager.cleanup();
        }

        dismissed
    }

    fn sync_keyboard_focus(&mut self, surface_id: Option<u64>) {
        let focus = surface_id.and_then(|surface_id| self.surface_for_id(surface_id));
        let seat = self.state.seat.clone();
        let client = focus.as_ref().and_then(WaylandResource::client);
        set_data_device_focus::<ProtocolRuntimeState>(
            &self.display.handle(),
            &seat,
            client.clone(),
        );
        set_primary_focus::<ProtocolRuntimeState>(&self.display.handle(), &seat, client);
        let Some(keyboard) = seat.get_keyboard() else {
            return;
        };

        keyboard.set_focus(&mut self.state, focus, SERIAL_COUNTER.next_serial());
        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after syncing keyboard focus",
            );
        }
    }

    fn dispatch_keyboard_input(&mut self, keycode: u32, pressed: bool, time: u32) {
        let seat = self.state.seat.clone();
        let Some(keyboard) = seat.get_keyboard() else {
            return;
        };

        keyboard.input::<(), _>(
            &mut self.state,
            keycode.into(),
            if pressed { KeyState::Pressed } else { KeyState::Released },
            SERIAL_COUNTER.next_serial(),
            time,
            |_, _, _| FilterResult::Forward,
        );

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after dispatching keyboard input",
            );
        }
    }

    fn dispatch_pointer_motion(
        &mut self,
        focus: Option<PointerSurfaceFocus>,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        let seat = self.state.seat.clone();
        let Some(pointer) = seat.get_pointer() else {
            return;
        };

        let focus = focus.and_then(|focus| {
            let root_surface = self.surface_for_id(focus.surface_id)?;
            under_from_surface_tree(
                &root_surface,
                location,
                focus.surface_origin.to_i32_round(),
                WindowSurfaceType::ALL,
            )
            .map(|(surface, origin)| (surface, origin.to_f64()))
        });
        pointer.motion(
            &mut self.state,
            focus,
            &MotionEvent { location, serial: SERIAL_COUNTER.next_serial(), time },
        );
        pointer.frame(&mut self.state);

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after dispatching pointer motion",
            );
        }
    }

    fn dispatch_pointer_button(
        &mut self,
        button_code: u32,
        pressed: bool,
        time: u32,
        focus_surface_id: Option<u64>,
    ) {
        let seat = self.state.seat.clone();
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        if pressed {
            self.state.synthetic_pointer_grab = focus_surface_id
                .map(|surface_id| SyntheticPointerGrab { serial: u32::from(serial), surface_id });
        } else {
            self.state.synthetic_pointer_grab = None;
        }

        pointer.button(
            &mut self.state,
            &ButtonEvent {
                serial,
                time,
                button: button_code,
                state: if pressed { ButtonState::Pressed } else { ButtonState::Released },
            },
        );
        pointer.frame(&mut self.state);

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after dispatching pointer button",
            );
        }
    }

    fn dispatch_pointer_axis(&mut self, horizontal: f64, vertical: f64, time: u32) {
        let seat = self.state.seat.clone();
        let Some(pointer) = seat.get_pointer() else {
            return;
        };

        let mut axis = AxisFrame::new(time).source(AxisSource::Continuous);
        if horizontal != 0.0 {
            axis = axis.value(InputAxis::Horizontal, horizontal);
        }
        if vertical != 0.0 {
            axis = axis.value(InputAxis::Vertical, vertical);
        }
        pointer.axis(&mut self.state, axis);
        pointer.frame(&mut self.state);

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after dispatching pointer axis",
            );
        }
    }

    fn sync_workspace_visibility(&mut self, activated_toplevels: &[u64], dismissed_popups: &[u64]) {
        let mut sent_protocol_update = false;

        for surface_id in dismissed_popups {
            if self.dismiss_popup(*surface_id) {
                sent_protocol_update = true;
            }
        }

        for surface_id in activated_toplevels {
            let Some(toplevel) = self.state.toplevels.get(surface_id).cloned() else {
                continue;
            };

            toplevel.send_configure();
            sent_protocol_update = true;
        }

        if sent_protocol_update {
            if let Err(error) = self.display.flush_clients() {
                remember_protocol_error(
                    &mut self.last_dispatch_error,
                    error,
                    "failed to flush Wayland clients after syncing workspace visibility",
                );
            }
        }
    }

    fn sync_keyboard_repeat_info(&mut self, repeat_rate: u16) {
        let seat = self.state.seat.clone();
        let Some(keyboard) = seat.get_keyboard() else {
            return;
        };

        keyboard.change_repeat_info(i32::from(repeat_rate), DEFAULT_KEYBOARD_REPEAT_DELAY_MS);
        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after updating keyboard repeat info",
            );
        }
    }

    fn send_frame_callbacks(&mut self, surface_ids: &[u64], frame_time: Time<Monotonic>) {
        let mut sent_callbacks = false;

        for surface_id in surface_ids {
            let Some(surface) = self.surface_for_id(*surface_id) else {
                continue;
            };

            let output = self.state.primary_output.clone();
            send_frames_surface_tree(&surface, &output, frame_time, None, |_, _| {
                Some(output.clone())
            });
            sent_callbacks = true;
        }

        if sent_callbacks {
            if let Err(error) = self.display.flush_clients() {
                remember_protocol_error(
                    &mut self.last_dispatch_error,
                    error,
                    "failed to flush Wayland clients after sending frame callbacks",
                );
            }
        }
    }

    fn send_presentation_feedback(
        &mut self,
        surface_ids: &[u64],
        frame_time: Time<Monotonic>,
        refresh: Refresh,
        sequence: Option<u64>,
    ) {
        let mut sent_feedback = false;
        let sequence = sequence.unwrap_or_else(|| {
            self.state.presentation_sequence = self.state.presentation_sequence.saturating_add(1);
            self.state.presentation_sequence
        });

        for surface_id in surface_ids {
            let Some(mut feedback) = self.presentation_feedback_for_id(*surface_id) else {
                continue;
            };

            let output = self.state.primary_output.clone();
            feedback.presented(
                &output,
                MONOTONIC_CLOCK_ID,
                frame_time,
                refresh,
                sequence,
                PresentationKind::Vsync,
            );
            sent_feedback = true;
        }

        if sent_feedback {
            if let Err(error) = self.display.flush_clients() {
                remember_protocol_error(
                    &mut self.last_dispatch_error,
                    error,
                    "failed to flush Wayland clients after sending presentation feedback",
                );
            }
        }
    }

    fn sync_output_timing(&mut self, output_timing: OutputTiming) {
        let mode = OutputMode {
            size: (output_timing.width as i32, output_timing.height as i32).into(),
            refresh: output_timing.refresh_millihz as i32,
        };
        self.state.mapped_primary_output_name = output_timing.output_name.clone();

        self.state.primary_output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(output_timing.scale.max(1) as i32)),
            Some((0, 0).into()),
        );
        self.state.primary_output.set_preferred(mode);
        self.state.update_all_fractional_scales();
    }

    fn surface_for_id(&self, surface_id: u64) -> Option<WlSurface> {
        self.state
            .toplevels
            .get(&surface_id)
            .map(|surface| surface.wl_surface().clone())
            .or_else(|| {
                self.state.popups.get(&surface_id).map(|surface| surface.wl_surface().clone())
            })
            .or_else(|| {
                self.state.layers.get(&surface_id).map(|surface| surface.wl_surface().clone())
            })
            .or_else(|| {
                self.state
                    .x11_window_ids_by_surface
                    .get(&surface_id)
                    .and_then(|window_id| self.state.x11_windows.get(window_id))
                    .and_then(X11Surface::wl_surface)
            })
    }

    fn presentation_feedback_for_id(&self, surface_id: u64) -> Option<SurfacePresentationFeedback> {
        let surface = self.surface_for_id(surface_id)?;
        compositor::with_states(&surface, |states| {
            SurfacePresentationFeedback::from_states(states, PresentationKind::empty())
        })
    }

    fn pointer_focus_candidate_accepts(
        &self,
        surface_id: u64,
        location: Point<f64, Logical>,
        surface_origin: Point<f64, Logical>,
    ) -> bool {
        let Some(root_surface) = self.surface_for_id(surface_id) else {
            return false;
        };

        under_from_surface_tree(
            &root_surface,
            location,
            surface_origin.to_i32_round(),
            WindowSurfaceType::ALL,
        )
        .is_some()
    }

    fn sync_server_state(&self, server_state: &mut ProtocolServerState) {
        server_state.last_accept_error = self.last_accept_error.clone();
        server_state.last_dispatch_error = self.last_dispatch_error.clone();
    }

    fn sync_xwayland_state(&self, state: &mut XWaylandServerState) {
        *state = XWaylandServerState {
            enabled: self.state.xwayland_state.enabled,
            ready: self.state.xwayland_state.ready,
            display_number: self.state.xwayland_state.display_number,
            display_name: self.state.xwayland_state.display_name.clone(),
            startup_error: self.state.xwayland_state.startup_error.clone(),
            last_error: self.last_xwayland_error.clone(),
        };
    }

    fn dismiss_popup_surface(&mut self, surface_id: u64, popup: &PopupSurface) -> bool {
        let popup_kind = DesktopPopupKind::from(popup.clone());
        match find_popup_root_surface(&popup_kind) {
            Ok(root_surface) => {
                if let Err(error) = DesktopPopupManager::dismiss_popup(&root_surface, &popup_kind) {
                    tracing::warn!(
                        surface_id,
                        error = %error,
                        "failed to dismiss popup through Smithay popup manager"
                    );
                    popup.send_popup_done();
                }
            }
            Err(error) => {
                tracing::warn!(
                    surface_id,
                    error = %error,
                    "popup root surface disappeared before server-side dismissal"
                );
                popup.send_popup_done();
            }
        }

        true
    }
}

impl ProtocolRuntimeState {
    fn new(
        display_handle: &smithay::reexports::wayland_server::DisplayHandle,
        repeat_rate: u16,
    ) -> Self {
        let compositor_state = SmithayCompositorState::new::<Self>(display_handle);
        let xdg_shell_state = SmithayXdgShellState::new::<Self>(display_handle);
        let xdg_decoration_state = SmithayXdgDecorationState::new::<Self>(display_handle);
        let xwayland_shell_state = SmithayXWaylandShellState::new::<Self>(display_handle);
        let layer_shell_state = WlrLayerShellState::new::<Self>(display_handle);
        let data_device_state = SmithayDataDeviceState::new::<Self>(display_handle);
        let primary_selection_state = SmithayPrimarySelectionState::new::<Self>(display_handle);
        let mut dmabuf_state = SmithayDmabufState::new();
        let dmabuf_global =
            dmabuf_state.create_global::<Self>(display_handle, Vec::<DmabufFormat>::new());
        let viewporter_state = SmithayViewporterState::new::<Self>(display_handle);
        let fractional_scale_state = FractionalScaleManagerState::new::<Self>(display_handle);
        let shm_state = SmithayShmState::new::<Self>(
            display_handle,
            vec![wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888, wl_shm::Format::Rgb565],
        );
        let presentation_state =
            SmithayPresentationState::new::<Self>(display_handle, MONOTONIC_CLOCK_ID);
        let output_manager_state =
            SmithayOutputManagerState::new_with_xdg_output::<Self>(display_handle);
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(display_handle, "seat-0");
        seat.add_pointer();
        let _ = seat.add_keyboard(
            XkbConfig::default(),
            DEFAULT_KEYBOARD_REPEAT_DELAY_MS,
            i32::from(repeat_rate),
        );

        let primary_output = Output::new(
            "Nekoland-1".into(),
            PhysicalProperties {
                size: (344, 194).into(),
                subpixel: Subpixel::Unknown,
                make: "Nekoland".into(),
                model: "Virtual Output".into(),
            },
        );
        primary_output.create_global::<Self>(display_handle);
        let mode = OutputMode { size: (1280, 720).into(), refresh: 60_000 };
        primary_output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            Some((0, 0).into()),
        );
        primary_output.set_preferred(mode);

        let mut state = Self {
            compositor_state,
            xdg_shell_state,
            _xdg_decoration_state: xdg_decoration_state,
            xwayland_shell_state,
            layer_shell_state,
            data_device_state,
            _primary_selection_state: primary_selection_state,
            dmabuf_state,
            _dmabuf_global: dmabuf_global,
            _viewporter_state: viewporter_state,
            _fractional_scale_state: fractional_scale_state,
            shm_state,
            _presentation_state: presentation_state,
            _output_manager_state: output_manager_state,
            seat_state,
            seat,
            primary_output: primary_output.clone(),
            popup_manager: DesktopPopupManager::default(),
            toplevels: HashMap::new(),
            popups: HashMap::new(),
            layers: HashMap::new(),
            xwms: HashMap::new(),
            x11_windows: HashMap::new(),
            x11_surface_ids_by_window: HashMap::new(),
            x11_window_ids_by_surface: HashMap::new(),
            mapped_x11_windows: BTreeSet::new(),
            published_x11_windows: BTreeSet::new(),
            xwayland_client: None,
            _xwm_connection: None,
            mapped_primary_output_name: primary_output.name(),
            event_queue: VecDeque::new(),
            next_surface_id: 1,
            presentation_sequence: 0,
            synthetic_pointer_grab: None,
            selection_persistence: SelectionPersistenceState::default(),
            xwayland_state: XWaylandRuntimeState::default(),
            cursor_state: ProtocolCursorState::default(),
        };

        state.queue_event(ProtocolEvent::OutputAnnounced { output_name: primary_output.name() });

        state
    }

    fn surface_id(&mut self, surface: &WlSurface) -> u64 {
        surface_identity(surface, &mut self.next_surface_id)
    }

    fn validate_interactive_request(
        &mut self,
        wl_seat: &WlSeat,
        serial: Serial,
        expected_focus_surface_id: u64,
        kind: InteractiveRequestKind,
    ) -> bool {
        let seat = self.seat.clone();
        let Some(pointer) = seat.get_pointer() else {
            tracing::warn!(
                request = kind.as_str(),
                expected_focus_surface_id,
                serial = u32::from(serial),
                "rejecting interactive xdg request because the seat has no pointer capability"
            );
            return false;
        };

        if !pointer.has_grab(serial) {
            if self.synthetic_pointer_grab.is_some_and(|grab| {
                grab.serial == u32::from(serial)
                    && (grab.surface_id == expected_focus_surface_id
                        || matches!(kind, InteractiveRequestKind::PopupGrab))
            }) {
                return true;
            }
            tracing::warn!(
                request = kind.as_str(),
                seat_name = seat.name(),
                seat_resource = %seat_name(wl_seat),
                expected_focus_surface_id,
                serial = u32::from(serial),
                "rejecting interactive xdg request without a matching implicit pointer grab"
            );
            return false;
        }

        let Some(grab_start) = pointer.grab_start_data() else {
            tracing::warn!(
                request = kind.as_str(),
                seat_name = seat.name(),
                seat_resource = %seat_name(wl_seat),
                expected_focus_surface_id,
                serial = u32::from(serial),
                "rejecting interactive xdg request because the pointer grab has no start data"
            );
            return false;
        };
        let Some((focused_surface, _)) = grab_start.focus else {
            tracing::warn!(
                request = kind.as_str(),
                seat_name = seat.name(),
                seat_resource = %seat_name(wl_seat),
                expected_focus_surface_id,
                serial = u32::from(serial),
                "rejecting interactive xdg request because the implicit grab did not start on a surface"
            );
            return false;
        };

        let focused_surface_id = self.surface_id(&focused_surface);
        if focused_surface_id != expected_focus_surface_id {
            if self.synthetic_pointer_grab.is_some_and(|grab| {
                grab.serial == u32::from(serial)
                    && (grab.surface_id == expected_focus_surface_id
                        || matches!(kind, InteractiveRequestKind::PopupGrab))
            }) {
                return true;
            }
            tracing::warn!(
                request = kind.as_str(),
                seat_name = seat.name(),
                seat_resource = %seat_name(wl_seat),
                expected_focus_surface_id,
                focused_surface_id,
                serial = u32::from(serial),
                "rejecting interactive xdg request because the implicit grab belongs to a different surface"
            );
            return false;
        }

        true
    }

    fn queue_event(&mut self, event: ProtocolEvent) {
        self.event_queue.push_back(event);
    }

    fn sync_x11_surface_mapping(&mut self, window: &X11Surface) -> Option<u64> {
        let surface = window.wl_surface()?;
        let surface_id = self.surface_id(&surface);
        let window_id = window.window_id();
        self.x11_surface_ids_by_window.insert(window_id, surface_id);
        self.x11_window_ids_by_surface.insert(surface_id, window_id);
        self.update_surface_fractional_scale(&surface);
        Some(surface_id)
    }

    fn queue_toplevel_metadata_changed(&mut self, surface: &ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        let (title, app_id) = compositor::with_states(surface.wl_surface(), |states| {
            let Some(attributes) = states.data_map.get::<XdgToplevelSurfaceData>() else {
                return (None, None);
            };
            let attributes = attributes.lock().unwrap();
            (attributes.title.clone(), attributes.app_id.clone())
        });

        self.queue_event(ProtocolEvent::ToplevelMetadataChanged { surface_id, title, app_id });
    }

    fn x11_app_id(window: &X11Surface) -> String {
        let class = window.class();
        if class.is_empty() { window.instance() } else { class }
    }

    fn x11_geometry(window: &X11Surface) -> X11WindowGeometry {
        let geometry = window.geometry();
        X11WindowGeometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w.max(1) as u32,
            height: geometry.size.h.max(1) as u32,
        }
    }

    fn should_publish_managed_x11_window(
        window: &X11Surface,
        title: &str,
        app_id: &str,
        geometry: X11WindowGeometry,
    ) -> bool {
        if title.is_empty() && app_id.is_empty() && geometry.width <= 1 && geometry.height <= 1 {
            return false;
        }

        if window.is_popup() {
            return false;
        }

        !matches!(
            window.window_type(),
            Some(
                WmWindowType::DropdownMenu
                    | WmWindowType::Menu
                    | WmWindowType::Notification
                    | WmWindowType::PopupMenu
                    | WmWindowType::Tooltip
            )
        )
    }

    fn remember_x11_window(&mut self, window: &X11Surface) {
        self.x11_windows.insert(window.window_id(), window.clone());
        let _ = self.sync_x11_surface_mapping(window);
    }

    fn publish_x11_window_if_ready(&mut self, window_id: u32) {
        if !self.mapped_x11_windows.contains(&window_id) {
            return;
        }

        let Some(window) = self.x11_windows.get(&window_id).cloned() else {
            return;
        };
        let Some(surface_id) = self.sync_x11_surface_mapping(&window) else {
            return;
        };
        let title = window.title();
        let app_id = Self::x11_app_id(&window);
        let geometry = Self::x11_geometry(&window);

        if !Self::should_publish_managed_x11_window(&window, &title, &app_id, geometry) {
            tracing::trace!(
                window_id,
                surface_id,
                window_type = ?window.window_type(),
                popup = window.is_popup(),
                override_redirect = window.is_override_redirect(),
                "ignoring XWayland helper surface"
            );
            return;
        }

        let event = if self.published_x11_windows.insert(window_id) {
            ProtocolEvent::X11WindowMapped {
                surface_id,
                window_id,
                override_redirect: window.is_override_redirect(),
                title,
                app_id,
                geometry,
            }
        } else {
            ProtocolEvent::X11WindowReconfigured { surface_id, title, app_id, geometry }
        };
        self.queue_event(event);
    }

    fn queue_x11_reconfigured(&mut self, window_id: u32) {
        if !self.published_x11_windows.contains(&window_id) {
            return;
        }

        let Some(window) = self.x11_windows.get(&window_id).cloned() else {
            return;
        };
        let Some(surface_id) = self.sync_x11_surface_mapping(&window) else {
            return;
        };

        self.queue_event(ProtocolEvent::X11WindowReconfigured {
            surface_id,
            title: window.title(),
            app_id: Self::x11_app_id(&window),
            geometry: Self::x11_geometry(&window),
        });
    }

    fn unpublish_x11_window(&mut self, window_id: u32) -> Option<u64> {
        self.mapped_x11_windows.remove(&window_id);
        self.published_x11_windows.remove(&window_id);
        let surface_id = self.x11_surface_ids_by_window.remove(&window_id);
        if let Some(surface_id) = surface_id {
            self.x11_window_ids_by_surface.remove(&surface_id);
        }
        surface_id
    }

    fn handle_xwayland_event(
        &mut self,
        handle: calloop::LoopHandle<'static, ProtocolRuntimeState>,
        event: XWaylandEvent,
    ) {
        match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                let Some(client) = self.xwayland_client.clone() else {
                    self.xwayland_state.startup_error =
                        Some("XWayland client handle disappeared before startup".to_owned());
                    self.xwayland_state.ready = false;
                    return;
                };

                let xwm_socket = x11_socket.try_clone().ok();
                match X11Wm::start_wm(handle, x11_socket, client) {
                    Ok(xwm) => {
                        self.xwms.insert(xwm.id(), xwm);
                        self._xwm_connection = xwm_socket;
                    }
                    Err(error) => {
                        self.xwayland_state.startup_error = Some(error.to_string());
                        self.xwayland_state.ready = false;
                        tracing::warn!(error = %error, "failed to attach XWayland window manager");
                        return;
                    }
                }
                self.xwayland_state.ready = true;
                self.xwayland_state.display_number = Some(display_number);
                self.xwayland_state.display_name = Some(format!(":{display_number}"));
                tracing::info!(
                    display_number,
                    display_name = self.xwayland_state.display_name.as_deref().unwrap_or(""),
                    "XWayland runtime is ready"
                );
            }
            XWaylandEvent::Error => {
                self.xwayland_state.ready = false;
                self.xwayland_state.display_number = None;
                self.xwayland_state.display_name = None;
                tracing::warn!("XWayland failed during startup");
            }
        }
    }

    fn preferred_fractional_scale(&self) -> f64 {
        self.primary_output.current_scale().fractional_scale().max(1.0)
    }

    fn update_surface_fractional_scale(&self, surface: &WlSurface) {
        let preferred_scale = self.preferred_fractional_scale();
        compositor::with_states(surface, |states| {
            with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(preferred_scale);
            });
        });
    }

    fn update_all_fractional_scales(&self) {
        for surface in self.toplevels.values() {
            self.update_surface_fractional_scale(surface.wl_surface());
        }
        for surface in self.popups.values() {
            self.update_surface_fractional_scale(surface.wl_surface());
        }
        for surface in self.layers.values() {
            self.update_surface_fractional_scale(surface.wl_surface());
        }
        for surface in self.x11_windows.values().filter_map(X11Surface::wl_surface) {
            self.update_surface_fractional_scale(&surface);
        }
    }

    fn sync_surface_registry(&self, registry: &mut ProtocolSurfaceRegistry) {
        registry.surfaces.clear();
        registry.surfaces.extend(self.toplevels.iter().map(|(surface_id, surface)| {
            (
                *surface_id,
                ProtocolSurfaceEntry {
                    kind: ProtocolSurfaceKind::Toplevel,
                    surface: surface.wl_surface().clone(),
                },
            )
        }));
        registry.surfaces.extend(self.popups.iter().map(|(surface_id, surface)| {
            (
                *surface_id,
                ProtocolSurfaceEntry {
                    kind: ProtocolSurfaceKind::Popup,
                    surface: surface.wl_surface().clone(),
                },
            )
        }));
        registry.surfaces.extend(self.layers.iter().map(|(surface_id, surface)| {
            (
                *surface_id,
                ProtocolSurfaceEntry {
                    kind: ProtocolSurfaceKind::Layer,
                    surface: surface.wl_surface().clone(),
                },
            )
        }));
        registry.surfaces.extend(self.x11_window_ids_by_surface.iter().filter_map(
            |(surface_id, window_id)| {
                self.x11_windows.get(window_id).and_then(|window| {
                    window.wl_surface().map(|surface| {
                        (
                            *surface_id,
                            ProtocolSurfaceEntry { kind: ProtocolSurfaceKind::Toplevel, surface },
                        )
                    })
                })
            },
        ));
    }
}

impl ClientData for ProtocolClientState {
    fn initialized(&self, client_id: ClientId) {
        tracing::debug!(?client_id, "Wayland client initialized");
    }

    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        tracing::debug!(?client_id, ?reason, "Wayland client disconnected");
    }
}

impl CompositorHandler for ProtocolRuntimeState {
    fn compositor_state(&mut self) -> &mut SmithayCompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(client_state) = client.get_data::<ProtocolClientState>() {
            &client_state.compositor_state
        } else if let Some(client_state) = client.get_data::<XWaylandClientData>() {
            &client_state.compositor_state
        } else {
            panic!("Wayland clients are created with ProtocolClientState or XWaylandClientData");
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<ProtocolRuntimeState>(surface);
        let surface_id = self.surface_id(surface);
        self.popup_manager.commit(surface);
        if let Some(role) = tracked_xdg_role(surface) {
            self.queue_event(ProtocolEvent::SurfaceCommitted {
                surface_id,
                role,
                size: committed_surface_extent(surface),
            });
        } else if self.layers.contains_key(&surface_id) {
            let cached_state = layer_cached_state(surface);
            self.queue_event(ProtocolEvent::LayerSurfaceCommitted {
                surface_id,
                size: committed_surface_extent(surface),
                anchor: map_layer_anchor(cached_state.anchor),
                desired_width: u32::try_from(cached_state.size.w.max(0)).unwrap_or_default(),
                desired_height: u32::try_from(cached_state.size.h.max(0)).unwrap_or_default(),
                exclusive_zone: map_exclusive_zone(cached_state.exclusive_zone),
                margins: map_layer_margins(cached_state.margin),
            });
        }
    }

    fn destroyed(&mut self, surface: &WlSurface) {
        let Some(role) = tracked_xdg_role(surface) else {
            return;
        };

        let surface_id = self.surface_id(surface);
        match role {
            XdgSurfaceRole::Toplevel => {
                self.toplevels.remove(&surface_id);
            }
            XdgSurfaceRole::Popup => {
                self.popups.remove(&surface_id);
            }
        }
        self.queue_event(ProtocolEvent::SurfaceDestroyed { surface_id, role });
    }
}

impl XdgShellHandler for ProtocolRuntimeState {
    fn xdg_shell_state(&mut self) -> &mut SmithayXdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface().clone();
        let surface_id = self.surface_id(&wl_surface);
        mark_xdg_surface(&wl_surface, XdgSurfaceRole::Toplevel);
        self.update_surface_fractional_scale(&wl_surface);
        self.toplevels.insert(surface_id, surface.clone());
        surface.send_configure();
        self.queue_event(ProtocolEvent::ConfigureRequested {
            surface_id,
            role: XdgSurfaceRole::Toplevel,
        });
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        let wl_surface = surface.wl_surface().clone();
        let surface_id = self.surface_id(&wl_surface);
        let parent_surface_id = surface.get_parent_surface().map(|parent| self.surface_id(&parent));
        let placement = popup_placement(positioner, None);
        let popup_kind = DesktopPopupKind::from(surface.clone());

        mark_xdg_surface(&wl_surface, XdgSurfaceRole::Popup);
        self.update_surface_fractional_scale(&wl_surface);
        if let Err(error) = self.popup_manager.track_popup(popup_kind) {
            tracing::warn!(
                surface_id,
                error = %error,
                "failed to register popup with Smithay popup manager"
            );
        }
        self.popups.insert(surface_id, surface.clone());
        let _ = surface.send_configure();
        self.queue_event(ProtocolEvent::PopupCreated { surface_id, parent_surface_id, placement });
        self.queue_event(ProtocolEvent::ConfigureRequested {
            surface_id,
            role: XdgSurfaceRole::Popup,
        });
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        let surface_id = self.surface_id(surface.wl_surface());
        if !self.validate_interactive_request(
            &seat,
            serial,
            surface_id,
            InteractiveRequestKind::Move,
        ) {
            return;
        }

        self.queue_event(ProtocolEvent::MoveRequested {
            surface_id,
            seat_name: self.seat.name().to_owned(),
            serial: serial.into(),
        });
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        if !self.validate_interactive_request(
            &seat,
            serial,
            surface_id,
            InteractiveRequestKind::Resize,
        ) {
            return;
        }

        self.queue_event(ProtocolEvent::ResizeRequested {
            surface_id,
            seat_name: self.seat.name().to_owned(),
            serial: serial.into(),
            edges: map_xdg_resize_edge(edges),
        });
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        let surface_id = self.surface_id(surface.wl_surface());
        let Some(parent_surface) = surface.get_parent_surface() else {
            tracing::warn!(
                request = InteractiveRequestKind::PopupGrab.as_str(),
                surface_id,
                serial = u32::from(serial),
                "rejecting popup grab because the popup has no parent surface"
            );
            return;
        };
        let parent_surface_id = self.surface_id(&parent_surface);
        if !self.validate_interactive_request(
            &seat,
            serial,
            parent_surface_id,
            InteractiveRequestKind::PopupGrab,
        ) {
            surface.send_popup_done();
            return;
        }

        let popup_kind = DesktopPopupKind::from(surface.clone());
        let root_surface = match find_popup_root_surface(&popup_kind) {
            Ok(root_surface) => root_surface,
            Err(error) => {
                tracing::warn!(
                    request = InteractiveRequestKind::PopupGrab.as_str(),
                    surface_id,
                    serial = u32::from(serial),
                    error = %error,
                    "rejecting popup grab because the popup root surface is no longer alive"
                );
                return;
            }
        };

        let popup_grab = match self.popup_manager.grab_popup::<Self>(
            root_surface,
            popup_kind,
            &self.seat,
            serial,
        ) {
            Ok(popup_grab) => popup_grab,
            Err(error) => {
                tracing::warn!(
                    request = InteractiveRequestKind::PopupGrab.as_str(),
                    surface_id,
                    serial = u32::from(serial),
                    error = %error,
                    "popup grab request was denied by Smithay popup manager; falling back to compositor-side popup grab state"
                );
                self.queue_event(ProtocolEvent::PopupGrabRequested {
                    surface_id,
                    seat_name: self.seat.name().to_owned(),
                    serial: serial.into(),
                });
                return;
            }
        };

        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_grab(self, PopupKeyboardGrab::new(&popup_grab), serial);
            keyboard.set_focus(self, Some(surface.wl_surface().clone()), serial);
        }
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.set_grab(self, PopupPointerGrab::new(&popup_grab), serial, Focus::Keep);
        }

        self.queue_event(ProtocolEvent::PopupGrabRequested {
            surface_id,
            seat_name: self.seat.name().to_owned(),
            serial: serial.into(),
        });
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        surface.send_configure();
        self.queue_event(ProtocolEvent::MaximizeRequested { surface_id });
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.queue_event(ProtocolEvent::UnMaximizeRequested { surface_id });
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        surface.send_configure();
        self.queue_event(ProtocolEvent::FullscreenRequested {
            surface_id,
            output_name: output.map(|output| format!("wl_output@{:?}", output.id())),
        });
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.queue_event(ProtocolEvent::UnFullscreenRequested { surface_id });
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.queue_event(ProtocolEvent::MinimizeRequested { surface_id });
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        let surface_id = self.surface_id(&surface);
        let (role, serial) = match configure {
            Configure::Toplevel(configure) => (XdgSurfaceRole::Toplevel, configure.serial.into()),
            Configure::Popup(configure) => (XdgSurfaceRole::Popup, configure.serial.into()),
        };

        self.queue_event(ProtocolEvent::AckConfigure { surface_id, role, serial });
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.send_repositioned(token);
        if surface.send_configure().is_ok() {
            let surface_id = self.surface_id(surface.wl_surface());
            self.queue_event(ProtocolEvent::PopupRepositionRequested {
                surface_id,
                placement: popup_placement(positioner, Some(token)),
            });
            self.queue_event(ProtocolEvent::ConfigureRequested {
                surface_id,
                role: XdgSurfaceRole::Popup,
            });
        }
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.popups.remove(&surface_id);
        self.queue_event(ProtocolEvent::SurfaceDestroyed {
            surface_id,
            role: XdgSurfaceRole::Popup,
        });
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.queue_toplevel_metadata_changed(&surface);
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.queue_toplevel_metadata_changed(&surface);
    }
}

impl XdgDecorationHandler for ProtocolRuntimeState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(XdgDecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: XdgDecorationMode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(XdgDecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(XdgDecorationMode::ServerSide);
        });
        toplevel.send_configure();
    }
}

impl SelectionHandler for ProtocolRuntimeState {
    type SelectionUserData = PersistedSelectionData;

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        seat: Seat<Self>,
    ) {
        let seat_name = seat.name().to_owned();
        let mime_types = source.map(|source| source.mime_types()).unwrap_or_default();
        self.selection_persistence.note_selection_change(ty, mime_types.clone());

        match ty {
            SelectionTarget::Clipboard => {
                self.event_queue
                    .push_back(ProtocolEvent::ClipboardSelectionChanged { seat_name, mime_types });
            }
            SelectionTarget::Primary => {
                self.event_queue
                    .push_back(ProtocolEvent::PrimarySelectionChanged { seat_name, mime_types });
            }
        }
    }

    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        mime_type: String,
        fd: std::os::unix::io::OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        let Some(bytes) = user_data.mime_data.get(&mime_type) else {
            tracing::warn!(%mime_type, "requested persisted selection mime type is unavailable");
            return;
        };

        let mut file = fs::File::from(fd);
        if let Err(error) = file.write_all(bytes) {
            tracing::warn!(%mime_type, %error, "failed to write persisted selection payload");
        }
    }
}

impl PrimarySelectionHandler for ProtocolRuntimeState {
    fn primary_selection_state(&self) -> &SmithayPrimarySelectionState {
        &self._primary_selection_state
    }
}

impl ClientDndGrabHandler for ProtocolRuntimeState {
    fn started(
        &mut self,
        source: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
    ) {
        let source_surface_id = seat
            .get_pointer()
            .and_then(|pointer| pointer.grab_start_data())
            .and_then(|start_data| start_data.focus.map(|(surface, _)| self.surface_id(&surface)));
        let icon_surface_id = icon.as_ref().map(|surface| self.surface_id(surface));
        let mime_types = source
            .as_ref()
            .and_then(|source| {
                with_source_metadata(source, |metadata| metadata.mime_types.clone()).ok()
            })
            .unwrap_or_default();

        self.queue_event(ProtocolEvent::DragStarted {
            seat_name: seat.name().to_owned(),
            source_surface_id,
            icon_surface_id,
            mime_types,
        });
    }

    fn dropped(&mut self, target: Option<WlSurface>, validated: bool, seat: Seat<Self>) {
        let target_surface_id = target.as_ref().map(|surface| self.surface_id(surface));
        self.queue_event(ProtocolEvent::DragDropped {
            seat_name: seat.name().to_owned(),
            target_surface_id,
            validated,
        });
    }
}

impl ServerDndGrabHandler for ProtocolRuntimeState {
    fn accept(&mut self, mime_type: Option<String>, seat: Seat<Self>) {
        self.queue_event(ProtocolEvent::DragAccepted {
            seat_name: seat.name().to_owned(),
            mime_type,
        });
    }

    fn action(&mut self, action: DndAction, seat: Seat<Self>) {
        self.queue_event(ProtocolEvent::DragActionSelected {
            seat_name: seat.name().to_owned(),
            action: format!("{action:?}"),
        });
    }
}

impl DataDeviceHandler for ProtocolRuntimeState {
    fn data_device_state(&self) -> &SmithayDataDeviceState {
        &self.data_device_state
    }
}

impl FractionalScaleHandler for ProtocolRuntimeState {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        self.update_surface_fractional_scale(&surface);
    }
}

impl SeatHandler for ProtocolRuntimeState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_state.image = match image {
            CursorImageStatus::Hidden => ProtocolCursorImage::Hidden,
            CursorImageStatus::Named(icon) => ProtocolCursorImage::Named(icon),
            CursorImageStatus::Surface(surface) => {
                let hotspot = compositor::with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .map(|attributes| attributes.lock().unwrap().hotspot)
                        .unwrap_or_default()
                });
                ProtocolCursorImage::Surface { surface, hotspot_x: hotspot.x, hotspot_y: hotspot.y }
            }
        };
    }
}

impl BufferHandler for ProtocolRuntimeState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl DmabufHandler for ProtocolRuntimeState {
    fn dmabuf_state(&mut self) -> &mut SmithayDmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        _dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        notifier.failed();
    }
}

impl WlrLayerShellHandler for ProtocolRuntimeState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: SmithayLayerSurface,
        output: Option<WlOutput>,
        layer: SmithayLayer,
        namespace: String,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.update_surface_fractional_scale(surface.wl_surface());
        let cached_state = layer_cached_state(surface.wl_surface());
        let suggested_size = suggested_layer_surface_size(cached_state.size, &self.primary_output);
        surface.with_pending_state(|state| {
            state.size = Some(suggested_size);
        });
        surface.send_configure();

        self.layers.insert(surface_id, surface);
        self.queue_event(ProtocolEvent::LayerSurfaceCreated {
            surface_id,
            namespace,
            output_name: output.map(|_| self.mapped_primary_output_name.clone()),
            layer: map_layer_level(layer),
            anchor: map_layer_anchor(cached_state.anchor),
            desired_width: u32::try_from(cached_state.size.w.max(0)).unwrap_or_default(),
            desired_height: u32::try_from(cached_state.size.h.max(0)).unwrap_or_default(),
            exclusive_zone: map_exclusive_zone(cached_state.exclusive_zone),
            margins: map_layer_margins(cached_state.margin),
        });
    }

    fn layer_destroyed(&mut self, surface: SmithayLayerSurface) {
        let surface_id = self.surface_id(surface.wl_surface());
        self.layers.remove(&surface_id);
        self.queue_event(ProtocolEvent::LayerSurfaceDestroyed { surface_id });
    }
}

impl ShmHandler for ProtocolRuntimeState {
    fn shm_state(&self) -> &SmithayShmState {
        &self.shm_state
    }
}

impl OutputHandler for ProtocolRuntimeState {
    fn output_bound(&mut self, _output: Output, _wl_output: WlOutput) {}
}

impl XWaylandShellHandler for ProtocolRuntimeState {
    fn xwayland_shell_state(&mut self) -> &mut SmithayXWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(&mut self, xwm: XwmId, wl_surface: WlSurface, surface: X11Surface) {
        let _ = xwm;
        let window_id = surface.window_id();
        let surface_id = self.surface_id(&wl_surface);
        self.x11_windows.insert(window_id, surface.clone());
        self.x11_surface_ids_by_window.insert(window_id, surface_id);
        self.x11_window_ids_by_surface.insert(surface_id, window_id);
        self.update_surface_fractional_scale(&wl_surface);
        self.publish_x11_window_if_ready(window_id);
    }
}

impl XwmHandler for ProtocolRuntimeState {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        self.xwms.get_mut(&xwm).expect("XWayland WM callback referenced an unknown XWM")
    }

    fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remember_x11_window(&window);
    }

    fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remember_x11_window(&window);
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let window_id = window.window_id();
        self.remember_x11_window(&window);
        if let Err(error) = window.set_mapped(true) {
            tracing::warn!(window_id, error = %error, "failed to map XWayland window");
            return;
        }
        self.mapped_x11_windows.insert(window_id);
        self.publish_x11_window_if_ready(window_id);
    }

    fn map_window_notify(&mut self, _xwm: XwmId, window: X11Surface) {
        let window_id = window.window_id();
        self.remember_x11_window(&window);
        self.mapped_x11_windows.insert(window_id);
        self.publish_x11_window_if_ready(window_id);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let window_id = window.window_id();
        self.remember_x11_window(&window);
        self.mapped_x11_windows.insert(window_id);
        self.publish_x11_window_if_ready(window_id);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let window_id = window.window_id();
        if let Some(surface_id) = self.unpublish_x11_window(window_id) {
            self.queue_event(ProtocolEvent::X11WindowUnmapped { surface_id });
        }
        self.x11_windows.insert(window_id, window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let window_id = window.window_id();
        if let Some(surface_id) = self.unpublish_x11_window(window_id) {
            self.queue_event(ProtocolEvent::X11WindowDestroyed { surface_id });
        }
        self.x11_windows.remove(&window_id);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let mut geometry = window.geometry();
        if let Some(x) = x {
            geometry.loc.x = x;
        }
        if let Some(y) = y {
            geometry.loc.y = y;
        }
        if let Some(w) = w {
            geometry.size.w = w.max(1) as i32;
        }
        if let Some(h) = h {
            geometry.size.h = h.max(1) as i32;
        }

        if let Err(error) = window.configure(geometry) {
            tracing::warn!(
                window_id = window.window_id(),
                error = %error,
                "failed to configure XWayland window"
            );
            return;
        }

        self.remember_x11_window(&window);
        self.queue_x11_reconfigured(window.window_id());
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _geometry: smithay::utils::Rectangle<i32, Logical>,
        _above: Option<smithay::xwayland::xwm::X11Window>,
    ) {
        self.remember_x11_window(&window);
        self.queue_x11_reconfigured(window.window_id());
    }

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, _property: WmWindowProperty) {
        self.remember_x11_window(&window);
        self.queue_x11_reconfigured(window.window_id());
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(ProtocolEvent::X11WindowMaximizeRequested { surface_id });
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(ProtocolEvent::X11WindowUnMaximizeRequested { surface_id });
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(ProtocolEvent::X11WindowFullscreenRequested { surface_id });
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(ProtocolEvent::X11WindowUnFullscreenRequested { surface_id });
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(ProtocolEvent::X11WindowMinimizeRequested { surface_id });
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(ProtocolEvent::X11WindowUnMinimizeRequested { surface_id });
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        button: u32,
        resize_edge: ResizeEdge,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(ProtocolEvent::X11WindowResizeRequested {
                surface_id,
                button,
                edges: map_x11_resize_edge(resize_edge),
            });
        }
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(ProtocolEvent::X11WindowMoveRequested { surface_id, button });
        }
    }

    fn disconnected(&mut self, xwm: XwmId) {
        self.xwms.remove(&xwm);
    }
}

fn surface_identity(surface: &WlSurface, next_surface_id: &mut u64) -> u64 {
    compositor::with_states(surface, |states| {
        if let Some(identity) = states.data_map.get::<SurfaceIdentity>() {
            return identity.0;
        }

        let surface_id = *next_surface_id;
        *next_surface_id = next_surface_id.saturating_add(1);
        states.data_map.insert_if_missing_threadsafe(|| SurfaceIdentity(surface_id));
        surface_id
    })
}

fn committed_surface_extent(surface: &WlSurface) -> Option<SurfaceExtent> {
    with_renderer_surface_state(surface, |state| {
        state.surface_size().or_else(|| state.buffer_size())
    })
    .flatten()
    .and_then(|size| {
        let width = u32::try_from(size.w).ok()?.max(1);
        let height = u32::try_from(size.h).ok()?.max(1);
        Some(SurfaceExtent { width, height })
    })
}

fn layer_cached_state(surface: &WlSurface) -> LayerSurfaceCachedState {
    compositor::with_states(surface, |states| {
        *states.cached_state.get::<LayerSurfaceCachedState>().current()
    })
}

fn suggested_layer_surface_size(
    requested_size: smithay::utils::Size<i32, Logical>,
    output: &Output,
) -> smithay::utils::Size<i32, Logical> {
    let output_size = output.current_mode().map(|mode| mode.size).unwrap_or((1280, 720).into());
    let width = if requested_size.w > 0 { requested_size.w } else { output_size.w.max(1) };
    let height = if requested_size.h > 0 { requested_size.h } else { output_size.h.max(1) };
    (width.max(1), height.max(1)).into()
}

fn map_layer_level(layer: SmithayLayer) -> nekoland_ecs::components::LayerLevel {
    match layer {
        SmithayLayer::Background => nekoland_ecs::components::LayerLevel::Background,
        SmithayLayer::Bottom => nekoland_ecs::components::LayerLevel::Bottom,
        SmithayLayer::Top => nekoland_ecs::components::LayerLevel::Top,
        SmithayLayer::Overlay => nekoland_ecs::components::LayerLevel::Overlay,
    }
}

fn map_layer_anchor(anchor: SmithayLayerAnchor) -> nekoland_ecs::components::LayerAnchor {
    nekoland_ecs::components::LayerAnchor {
        top: anchor.contains(SmithayLayerAnchor::TOP),
        bottom: anchor.contains(SmithayLayerAnchor::BOTTOM),
        left: anchor.contains(SmithayLayerAnchor::LEFT),
        right: anchor.contains(SmithayLayerAnchor::RIGHT),
    }
}

fn map_exclusive_zone(exclusive_zone: SmithayExclusiveZone) -> i32 {
    match exclusive_zone {
        SmithayExclusiveZone::Exclusive(value) => i32::try_from(value).unwrap_or(i32::MAX),
        SmithayExclusiveZone::Neutral => 0,
        SmithayExclusiveZone::DontCare => -1,
    }
}

fn map_layer_margins(margins: SmithayMargins) -> nekoland_ecs::components::LayerMargins {
    nekoland_ecs::components::LayerMargins {
        top: margins.top,
        right: margins.right,
        bottom: margins.bottom,
        left: margins.left,
    }
}

fn mark_xdg_surface(surface: &WlSurface, role: XdgSurfaceRole) {
    compositor::with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(|| XdgSurfaceMarker(role));
    });
}

fn popup_placement(positioner: PositionerState, reposition_token: Option<u32>) -> PopupPlacement {
    let geometry = positioner.get_geometry();
    PopupPlacement {
        x: geometry.loc.x,
        y: geometry.loc.y,
        width: geometry.size.w,
        height: geometry.size.h,
        reposition_token,
    }
}

fn tracked_xdg_role(surface: &WlSurface) -> Option<XdgSurfaceRole> {
    compositor::with_states(surface, |states| {
        states.data_map.get::<XdgSurfaceMarker>().map(|marker| marker.0)
    })
}

fn compositor_time_millis(clock: &CompositorClock) -> u32 {
    clock.uptime_millis.min(u128::from(u32::MAX)) as u32
}

fn sync_keyboard_focus_if_needed(
    server: &mut SmithayProtocolServer,
    seat_sync: &mut SeatInputSyncState,
    keyboard_focus: Option<&KeyboardFocusState>,
) {
    let desired_focus = seat_sync
        .host_focused
        .then(|| keyboard_focus.and_then(|focus| focus.focused_surface))
        .flatten();

    if seat_sync.keyboard_focus == desired_focus {
        return;
    }

    server.sync_keyboard_focus(desired_focus);
    seat_sync.keyboard_focus = desired_focus;
}

fn sync_pointer_focus_if_needed(
    server: &mut SmithayProtocolServer,
    seat_sync: &mut SeatInputSyncState,
    pointer: Option<&GlobalPointerPosition>,
    render_list: Option<&RenderList>,
    surface_presentation: Option<&SurfacePresentationSnapshot>,
    primary_output: Option<&PrimaryOutputState>,
    outputs: &Query<(Entity, &OutputDevice, &OutputPlacement)>,
    windows: &Query<
        (
            Entity,
            SurfaceRuntime,
            Option<&WindowViewportVisibility>,
            Option<&OutputBackgroundWindow>,
        ),
        With<XdgWindow>,
    >,
    popups: &Query<(SurfaceRuntime, &ChildOf), With<XdgPopup>>,
    layers: &Query<
        (SurfaceRuntime, Option<&LayerOnOutput>, Option<&DesiredOutputName>),
        With<LayerShellSurface>,
    >,
    time: u32,
) {
    let location = pointer
        .map(|pointer| Point::<f64, Logical>::from((pointer.x, pointer.y)))
        .unwrap_or(seat_sync.pointer_location);
    let desired_focus = if seat_sync.host_focused {
        pointer.and_then(|pointer| {
            pointer_focus_target(
                pointer.x,
                pointer.y,
                Some(&*server),
                location,
                render_list,
                surface_presentation,
                primary_output,
                outputs,
                windows,
                popups,
                layers,
            )
        })
    } else {
        None
    };
    let desired_focus_id = desired_focus.map(|focus| focus.surface_id);

    if seat_sync.pointer_focus == desired_focus_id && seat_sync.pointer_location == location {
        return;
    }

    server.dispatch_pointer_motion(desired_focus, location, time);
    seat_sync.pointer_focus = desired_focus_id;
    seat_sync.pointer_location = location;
}

fn pointer_focus_target(
    pointer_x: f64,
    pointer_y: f64,
    server: Option<&SmithayProtocolServer>,
    location: Point<f64, Logical>,
    render_list: Option<&RenderList>,
    surface_presentation: Option<&SurfacePresentationSnapshot>,
    primary_output: Option<&PrimaryOutputState>,
    outputs: &Query<(Entity, &OutputDevice, &OutputPlacement)>,
    windows: &Query<
        (
            Entity,
            SurfaceRuntime,
            Option<&WindowViewportVisibility>,
            Option<&OutputBackgroundWindow>,
        ),
        With<XdgWindow>,
    >,
    popups: &Query<(SurfaceRuntime, &ChildOf), With<XdgPopup>>,
    layers: &Query<
        (SurfaceRuntime, Option<&LayerOnOutput>, Option<&DesiredOutputName>),
        With<LayerShellSurface>,
    >,
) -> Option<PointerSurfaceFocus> {
    let render_list = render_list?;
    if let Some(surface_presentation) = surface_presentation {
        let output_offsets = outputs
            .iter()
            .map(|(_, output, placement)| (output.name.clone(), (placement.x, placement.y)))
            .collect::<HashMap<_, _>>();

        for element in render_list.elements.iter().rev() {
            let Some(state) = surface_presentation.surfaces.get(&element.surface_id) else {
                continue;
            };
            if !state.visible || !state.input_enabled {
                continue;
            }
            let bounds = global_surface_bounds(
                &state.geometry,
                state.target_output.as_deref(),
                &output_offsets,
            );
            if pointer_x < bounds.x
                || pointer_x >= bounds.x + bounds.width
                || pointer_y < bounds.y
                || pointer_y >= bounds.y + bounds.height
            {
                continue;
            }
            let surface_origin = Point::<f64, Logical>::from((bounds.x, bounds.y));
            if server.is_some_and(|server| {
                !server.pointer_focus_candidate_accepts(
                    element.surface_id,
                    location,
                    surface_origin,
                )
            }) {
                continue;
            }
            return Some(PointerSurfaceFocus { surface_id: element.surface_id, surface_origin });
        }

        return None;
    }
    let primary_output_name =
        primary_output.and_then(|primary_output| primary_output.name.as_deref());
    let output_names = outputs
        .iter()
        .map(|(entity, output, _)| (entity, output.name.clone()))
        .collect::<HashMap<_, _>>();
    let output_offsets = outputs
        .iter()
        .map(|(_, output, placement)| (output.name.clone(), (placement.x, placement.y)))
        .collect::<HashMap<_, _>>();
    let mut surface_bounds = HashMap::new();
    let mut window_target_outputs_by_entity = HashMap::new();

    for (entity, surface, viewport_visibility, background) in windows.iter() {
        if background.is_some() {
            continue;
        }
        let target_output = background
            .map(|background| background.output.clone())
            .or_else(|| viewport_visibility.and_then(|visibility| visibility.output.clone()));
        window_target_outputs_by_entity.insert(entity, target_output.clone());
        surface_bounds.insert(
            surface.surface_id(),
            global_surface_bounds(surface.geometry, target_output.as_deref(), &output_offsets),
        );
    }

    for (surface, child_of) in popups.iter() {
        let target_output = window_target_outputs_by_entity
            .get(&child_of.parent())
            .and_then(|target_output| target_output.as_deref());
        surface_bounds.insert(
            surface.surface_id(),
            global_surface_bounds(surface.geometry, target_output, &output_offsets),
        );
    }

    for (surface, layer_output, desired_output_name) in layers.iter() {
        let target_output = layer_output
            .and_then(|layer_output| output_names.get(&layer_output.0).map(String::as_str))
            .or_else(|| {
                desired_output_name.and_then(|desired_output_name| desired_output_name.0.as_deref())
            })
            .or(primary_output_name);
        surface_bounds.insert(
            surface.surface_id(),
            global_surface_bounds(surface.geometry, target_output, &output_offsets),
        );
    }

    for element in render_list.elements.iter().rev() {
        let Some(bounds) = surface_bounds.get(&element.surface_id) else {
            continue;
        };

        if pointer_x >= bounds.x
            && pointer_x < bounds.x + bounds.width
            && pointer_y >= bounds.y
            && pointer_y < bounds.y + bounds.height
        {
            let surface_origin = Point::<f64, Logical>::from((bounds.x, bounds.y));
            if server.is_some_and(|server| {
                !server.pointer_focus_candidate_accepts(
                    element.surface_id,
                    location,
                    surface_origin,
                )
            }) {
                continue;
            }
            return Some(PointerSurfaceFocus { surface_id: element.surface_id, surface_origin });
        }
    }

    None
}

#[derive(Debug, Clone, Copy)]
struct GlobalSurfaceBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn global_surface_bounds(
    geometry: &SurfaceGeometry,
    target_output: Option<&str>,
    output_offsets: &HashMap<String, (i32, i32)>,
) -> GlobalSurfaceBounds {
    let (offset_x, offset_y) = target_output
        .and_then(|target_output| output_offsets.get(target_output).copied())
        .unwrap_or((0, 0));
    GlobalSurfaceBounds {
        x: f64::from(geometry.x.saturating_add(offset_x)),
        y: f64::from(geometry.y.saturating_add(offset_y)),
        width: f64::from(geometry.width.max(1)),
        height: f64::from(geometry.height.max(1)),
    }
}

fn current_output_timing(outputs: &Query<OutputRuntime>) -> Option<OutputTiming> {
    outputs.iter().min_by(|left, right| left.name().cmp(right.name())).map(|output| OutputTiming {
        output_name: output.name().to_owned(),
        width: output.properties.width.max(1),
        height: output.properties.height.max(1),
        refresh_millihz: output.properties.refresh_millihz,
        scale: output.properties.scale.max(1),
    })
}

fn current_output_presentation(
    outputs: &Query<OutputRuntime>,
    output_presentation: Option<&OutputPresentationState>,
) -> Option<PresentationFeedbackTiming> {
    let output_presentation = output_presentation?;
    let output_name = outputs
        .iter()
        .min_by(|left, right| left.name().cmp(right.name()))
        .map(|output| output.name().to_owned())?;
    let timeline =
        output_presentation.outputs.iter().find(|timeline| timeline.output_name == output_name)?;
    let frame_time = Time::<Monotonic>::from(Duration::from_nanos(timeline.present_time_nanos));
    let refresh = if timeline.refresh_interval_nanos == 0 {
        Refresh::Unknown
    } else {
        Refresh::fixed(Duration::from_nanos(timeline.refresh_interval_nanos))
    };

    Some(PresentationFeedbackTiming { frame_time, refresh, sequence: Some(timeline.sequence) })
}

fn map_xdg_resize_edge(
    edge: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
) -> ResizeEdges {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge as XdgResizeEdge;

    match edge {
        XdgResizeEdge::Top => ResizeEdges::Top,
        XdgResizeEdge::Bottom => ResizeEdges::Bottom,
        XdgResizeEdge::Left => ResizeEdges::Left,
        XdgResizeEdge::TopLeft => ResizeEdges::TopLeft,
        XdgResizeEdge::BottomLeft => ResizeEdges::BottomLeft,
        XdgResizeEdge::Right => ResizeEdges::Right,
        XdgResizeEdge::TopRight => ResizeEdges::TopRight,
        XdgResizeEdge::BottomRight => ResizeEdges::BottomRight,
        _ => ResizeEdges::BottomRight,
    }
}

fn map_x11_resize_edge(edge: ResizeEdge) -> ResizeEdges {
    match edge {
        ResizeEdge::Top => ResizeEdges::Top,
        ResizeEdge::Bottom => ResizeEdges::Bottom,
        ResizeEdge::Left => ResizeEdges::Left,
        ResizeEdge::TopLeft => ResizeEdges::TopLeft,
        ResizeEdge::BottomLeft => ResizeEdges::BottomLeft,
        ResizeEdge::Right => ResizeEdges::Right,
        ResizeEdge::TopRight => ResizeEdges::TopRight,
        ResizeEdge::BottomRight => ResizeEdges::BottomRight,
    }
}

fn refresh_from_output_timing(output_timing: OutputTiming) -> Refresh {
    if output_timing.refresh_millihz == 0 {
        return Refresh::Unknown;
    }

    let refresh_nanos = 1_000_000_000_000_u64 / u64::from(output_timing.refresh_millihz);
    Refresh::fixed(std::time::Duration::from_nanos(refresh_nanos.max(1)))
}

fn bind_wayland_socket() -> std::io::Result<(ListeningSocket, OsString)> {
    let _runtime_dir_guard = RuntimeDirGuard::install()?;

    match ListeningSocket::bind_auto("wayland", 0..33) {
        Ok(socket) => {
            let socket_name =
                OsString::from(socket_name_or_default(socket_name_or_none_ref(&socket), "wayland"));
            Ok((socket, socket_name))
        }
        Err(auto_error) => {
            let fallback_name = format!("nekoland-{}", std::process::id());
            match ListeningSocket::bind(&fallback_name) {
                Ok(socket) => Ok((socket, OsString::from(fallback_name))),
                Err(fallback_error) => Err(std::io::Error::other(format!(
                    "auto socket failed ({auto_error}); fallback socket `{fallback_name}` failed ({fallback_error})"
                ))),
            }
        }
    }
}

fn socket_name_or_none_ref(socket: &ListeningSocket) -> Option<&OsStr> {
    socket.socket_name()
}

fn socket_name_or_default(name: Option<&OsStr>, fallback: &str) -> String {
    name.unwrap_or_else(|| OsStr::new(fallback)).to_string_lossy().into_owned()
}

#[derive(Debug)]
struct RuntimeDirGuard {
    previous: Option<OsString>,
}

impl RuntimeDirGuard {
    fn install() -> std::io::Result<Option<Self>> {
        let Some(runtime_dir) = env::var_os("NEKOLAND_RUNTIME_DIR") else {
            return Ok(None);
        };

        fs::create_dir_all(&runtime_dir)?;
        let previous = env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            env::set_var("XDG_RUNTIME_DIR", &runtime_dir);
        }

        tracing::info!(
            runtime_dir = %display_runtime_dir(&runtime_dir),
            "using overridden Wayland runtime dir"
        );
        Ok(Some(Self { previous }))
    }
}

impl Drop for RuntimeDirGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(previous) => unsafe {
                env::set_var("XDG_RUNTIME_DIR", previous);
            },
            None => unsafe {
                env::remove_var("XDG_RUNTIME_DIR");
            },
        }
    }
}

fn display_runtime_dir(path: &OsStr) -> String {
    path.to_string_lossy().into_owned()
}

fn current_wayland_runtime_dir() -> Option<String> {
    env::var_os("NEKOLAND_RUNTIME_DIR")
        .or_else(|| env::var_os("XDG_RUNTIME_DIR"))
        .map(|path| path.to_string_lossy().into_owned())
}

fn seat_name(seat: &WlSeat) -> String {
    format!("wl_seat@{:?}", seat.id())
}

fn remember_protocol_error(
    slot: &mut Option<String>,
    error: impl std::fmt::Display,
    message: &str,
) {
    let error = error.to_string();
    if slot.as_deref() != Some(error.as_str()) {
        tracing::warn!(error = %error, "{message}");
    }
    *slot = Some(error);
}

fn register_calloop_sources(app: &mut App, server: &SmithayProtocolServer) {
    if app.world().get_non_send_resource::<CalloopSourceRegistry>().is_none() {
        app.insert_non_send_resource(CalloopSourceRegistry::default());
    }

    let Some(runtime) = server.runtime.as_ref() else {
        return;
    };

    let runtime = runtime.clone();
    let display_fd = runtime.borrow().display.as_fd().as_raw_fd();
    let socket_fd = runtime.borrow().socket.as_ref().map(AsRawFd::as_raw_fd);

    let mut registry = app
        .world_mut()
        .get_non_send_resource_mut::<CalloopSourceRegistry>()
        .expect("calloop registry inserted immediately before access");

    registry.push(move |handle| {
        let display_runtime = runtime.clone();
        handle
            .insert_source(
                Generic::new(
                    unsafe { FdWrapper::new(RegisteredRawFd(display_fd)) },
                    Interest::READ,
                    Mode::Level,
                ),
                move |_, _, _| {
                    display_runtime.borrow_mut().on_display_ready();
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;

        if let Some(socket_fd) = socket_fd {
            let socket_runtime = runtime.clone();
            handle
                .insert_source(
                    Generic::new(
                        unsafe { FdWrapper::new(RegisteredRawFd(socket_fd)) },
                        Interest::READ,
                        Mode::Level,
                    ),
                    move |_, _, _| {
                        socket_runtime.borrow_mut().on_socket_ready();
                        Ok(PostAction::Continue)
                    },
                )
                .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;
        }

        Ok(())
    });
}

impl AsRawFd for RegisteredRawFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

delegate_compositor!(ProtocolRuntimeState);
delegate_xdg_shell!(ProtocolRuntimeState);
delegate_xdg_decoration!(ProtocolRuntimeState);
delegate_layer_shell!(ProtocolRuntimeState);
delegate_xwayland_shell!(ProtocolRuntimeState);
delegate_viewporter!(ProtocolRuntimeState);
delegate_fractional_scale!(ProtocolRuntimeState);
delegate_data_device!(ProtocolRuntimeState);
delegate_primary_selection!(ProtocolRuntimeState);
delegate_dmabuf!(ProtocolRuntimeState);
delegate_shm!(ProtocolRuntimeState);
delegate_output!(ProtocolRuntimeState);
delegate_seat!(ProtocolRuntimeState);
delegate_presentation!(ProtocolRuntimeState);

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::io::ErrorKind;
    use std::os::unix::net::UnixListener;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::{Entity, Query, World};
    use bevy_ecs::query::With;
    use bevy_ecs::system::SystemState;
    use nekoland_ecs::bundles::OutputBundle;
    use nekoland_ecs::components::{
        DesiredOutputName, LayerOnOutput, LayerShellSurface, OutputBackgroundWindow, OutputDevice,
        OutputPlacement, OutputProperties, SurfaceGeometry, WindowViewportVisibility,
        WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::resources::{PrimaryOutputState, RenderElement, RenderList};
    use nekoland_ecs::views::SurfaceRuntime;
    use smithay::reexports::wayland_server::Display;
    use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
    use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

    use super::{
        DEFAULT_KEYBOARD_REPEAT_RATE, ProtocolClientState, ProtocolEvent, ProtocolRuntimeState,
        SmithayProtocolRuntime, XdgSurfaceRole, pointer_focus_target,
    };

    #[derive(Debug)]
    struct ClientSummary {
        globals: Vec<String>,
        configure_serial: u32,
    }

    #[derive(Debug, Default)]
    struct TestClientState {
        globals: Vec<String>,
        base_surface: Option<wl_surface::WlSurface>,
        wm_base: Option<xdg_wm_base::XdgWmBase>,
        xdg_surface: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
        configure_serial: Option<u32>,
    }

    #[test]
    fn roundtrip_exposes_globals_and_emits_toplevel_events() {
        let socket_path = temporary_socket_path();
        let listener = match UnixListener::bind(&socket_path) {
            Ok(listener) => listener,
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                eprintln!("skipping protocol round-trip test in restricted sandbox: {error}");
                return;
            }
            Err(error) => panic!("test UnixListener bind: {error}"),
        };

        let (result_tx, result_rx) = mpsc::channel();
        let client_socket_path = socket_path.clone();
        let client_thread = thread::spawn(move || {
            let result = run_test_client(client_socket_path);
            let _ = result_tx.send(result);
        });
        let (server_stream, _) = listener.accept().expect("test UnixListener accept");
        let _ = fs::remove_file(&socket_path);
        let mut runtime = test_runtime(server_stream);

        let Some(summary) = pump_server_until_client_finishes(&mut runtime, &result_rx) else {
            client_thread.join().expect("client thread should exit cleanly");
            return;
        };
        client_thread.join().expect("client thread should exit cleanly");

        for _ in 0..4 {
            runtime.dispatch_clients();
            thread::sleep(Duration::from_millis(1));
        }

        let events = runtime.drain_events();
        let surface_id = events
            .iter()
            .find_map(|event| match event {
                ProtocolEvent::ConfigureRequested {
                    surface_id,
                    role: XdgSurfaceRole::Toplevel,
                } => Some(*surface_id),
                _ => None,
            })
            .expect("server should emit a toplevel configure request");

        assert_globals_present(&summary.globals);
        assert!(
            events.iter().any(|event| matches!(
                event,
                ProtocolEvent::SurfaceCommitted {
                    surface_id: event_surface_id,
                    role: XdgSurfaceRole::Toplevel,
                    ..
                } if *event_surface_id == surface_id
            )),
            "server should record the toplevel commit: {events:#?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                ProtocolEvent::AckConfigure {
                    surface_id: event_surface_id,
                    role: XdgSurfaceRole::Toplevel,
                    serial,
                } if *event_surface_id == surface_id && *serial == summary.configure_serial
            )),
            "server should record the configure ack: {events:#?}"
        );
    }

    #[test]
    fn pointer_hit_test_prefers_layer_surfaces_above_windows() {
        let mut world = World::default();
        world.spawn((
            WlSurfaceHandle { id: 11 },
            SurfaceGeometry { x: 0, y: 0, width: 320, height: 64 },
            XdgWindow::default(),
        ));
        world.spawn((
            WlSurfaceHandle { id: 22 },
            SurfaceGeometry { x: 0, y: 0, width: 320, height: 64 },
            LayerShellSurface::default(),
        ));

        let render_list = RenderList {
            elements: vec![
                RenderElement { surface_id: 11, z_index: 0, opacity: 1.0 },
                RenderElement { surface_id: 22, z_index: 1, opacity: 1.0 },
            ],
        };
        let mut system_state: SystemState<(
            Query<(Entity, &OutputDevice, &OutputPlacement)>,
            Query<
                (
                    Entity,
                    SurfaceRuntime,
                    Option<&WindowViewportVisibility>,
                    Option<&OutputBackgroundWindow>,
                ),
                With<XdgWindow>,
            >,
            Query<(SurfaceRuntime, &ChildOf), With<nekoland_ecs::components::XdgPopup>>,
            Query<
                (SurfaceRuntime, Option<&LayerOnOutput>, Option<&DesiredOutputName>),
                With<LayerShellSurface>,
            >,
        )> = SystemState::new(&mut world);
        let (outputs, windows, popups, layers) = system_state.get(&world);

        let target = pointer_focus_target(
            16.0,
            16.0,
            None,
            (16.0, 16.0).into(),
            Some(&render_list),
            None,
            None,
            &outputs,
            &windows,
            &popups,
            &layers,
        )
        .expect("pointer focus target should exist");

        assert_eq!(target.surface_id, 22);
        assert_eq!(target.surface_origin, (0.0, 0.0).into());
    }

    #[test]
    fn pointer_hit_test_offsets_output_local_window_geometry_by_output_placement() {
        let mut world = World::default();
        world.spawn(OutputBundle {
            output: OutputDevice { name: "DP-1".to_owned(), ..Default::default() },
            properties: OutputProperties {
                width: 100,
                height: 100,
                refresh_millihz: 60_000,
                scale: 1,
            },
            placement: OutputPlacement { x: 0, y: 0 },
            ..Default::default()
        });
        world.spawn(OutputBundle {
            output: OutputDevice { name: "DP-2".to_owned(), ..Default::default() },
            properties: OutputProperties {
                width: 100,
                height: 100,
                refresh_millihz: 60_000,
                scale: 1,
            },
            placement: OutputPlacement { x: 100, y: 0 },
            ..Default::default()
        });
        world.spawn((
            WlSurfaceHandle { id: 42 },
            SurfaceGeometry { x: 0, y: 0, width: 80, height: 80 },
            WindowViewportVisibility { visible: true, output: Some("DP-2".to_owned()) },
            XdgWindow::default(),
        ));

        let render_list = RenderList {
            elements: vec![RenderElement { surface_id: 42, z_index: 0, opacity: 1.0 }],
        };
        let mut system_state: SystemState<(
            Query<(Entity, &OutputDevice, &OutputPlacement)>,
            Query<
                (
                    Entity,
                    SurfaceRuntime,
                    Option<&WindowViewportVisibility>,
                    Option<&OutputBackgroundWindow>,
                ),
                With<XdgWindow>,
            >,
            Query<(SurfaceRuntime, &ChildOf), With<nekoland_ecs::components::XdgPopup>>,
            Query<
                (SurfaceRuntime, Option<&LayerOnOutput>, Option<&DesiredOutputName>),
                With<LayerShellSurface>,
            >,
        )> = SystemState::new(&mut world);
        let (outputs, windows, popups, layers) = system_state.get(&world);

        let target = pointer_focus_target(
            110.0,
            10.0,
            None,
            (110.0, 10.0).into(),
            Some(&render_list),
            None,
            Some(&PrimaryOutputState { name: Some("DP-1".to_owned()) }),
            &outputs,
            &windows,
            &popups,
            &layers,
        )
        .expect("window on the second output should receive pointer focus");

        assert_eq!(target.surface_id, 42);
        assert_eq!(target.surface_origin, (100.0, 0.0).into());
        assert!(
            pointer_focus_target(
                10.0,
                10.0,
                None,
                (10.0, 10.0).into(),
                Some(&render_list),
                None,
                Some(&PrimaryOutputState { name: Some("DP-1".to_owned()) }),
                &outputs,
                &windows,
                &popups,
                &layers,
            )
            .is_none(),
            "output-local geometry should not be hit-tested at the wrong global origin",
        );
    }

    fn test_runtime(server_stream: UnixStream) -> SmithayProtocolRuntime {
        let display = Display::new().expect("server display");
        let mut display_handle = display.handle();
        let state = ProtocolRuntimeState::new(&display_handle, DEFAULT_KEYBOARD_REPEAT_RATE);
        let client = display_handle
            .insert_client(server_stream, std::sync::Arc::new(ProtocolClientState::default()))
            .expect("server client registration");

        SmithayProtocolRuntime {
            display,
            state,
            xwayland_event_loop: None,
            socket: None,
            clients: vec![client],
            last_accept_error: None,
            last_dispatch_error: None,
            last_xwayland_error: None,
        }
    }

    fn run_test_client(socket_path: std::path::PathBuf) -> Result<ClientSummary, String> {
        let stream = UnixStream::connect(&socket_path)
            .map_err(|error| format!("socket connect failed: {error}"))?;
        let conn = Connection::from_socket(stream)
            .map_err(|error| format!("from_socket failed: {error}"))?;
        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();
        conn.display().get_registry(&qh, ());

        let mut state = TestClientState::default();
        let deadline = Instant::now() + Duration::from_secs(2);

        while state.configure_serial.is_none() {
            client_dispatch_once(&mut event_queue, &mut state)
                .map_err(|error| format!("client dispatch failed: {error}"))?;
            if Instant::now() >= deadline {
                return Err("timed out waiting for xdg_surface.configure".to_owned());
            }
        }

        event_queue
            .flush()
            .map_err(|error| format!("final flush after configure failed: {error}"))?;

        Ok(ClientSummary {
            globals: state.globals,
            configure_serial: state
                .configure_serial
                .ok_or_else(|| "client never received xdg_surface.configure".to_owned())?,
        })
    }

    fn client_dispatch_once(
        event_queue: &mut EventQueue<TestClientState>,
        state: &mut TestClientState,
    ) -> Result<(), String> {
        event_queue
            .dispatch_pending(state)
            .map_err(|error| format!("dispatch_pending before read failed: {error}"))?;
        event_queue.flush().map_err(|error| format!("flush failed: {error}"))?;

        let Some(read_guard) = event_queue.prepare_read() else {
            return Ok(());
        };

        read_guard.read().map_err(|error| format!("socket read failed: {error}"))?;
        event_queue
            .dispatch_pending(state)
            .map_err(|error| format!("dispatch_pending after read failed: {error}"))?;
        Ok(())
    }

    fn pump_server_until_client_finishes(
        runtime: &mut SmithayProtocolRuntime,
        result_rx: &mpsc::Receiver<Result<ClientSummary, String>>,
    ) -> Option<ClientSummary> {
        let deadline = Instant::now() + Duration::from_secs(2);

        loop {
            runtime.dispatch_clients();

            match result_rx.try_recv() {
                Ok(Ok(summary)) => return Some(summary),
                Ok(Err(error)) if error.contains("Operation not permitted") => {
                    eprintln!("skipping protocol round-trip test in restricted sandbox: {error}");
                    return None;
                }
                Ok(Err(error)) => panic!("test client failed: {error}"),
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("test client exited without sending a result")
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }

            assert!(Instant::now() < deadline, "timed out waiting for the protocol round-trip");

            thread::sleep(Duration::from_millis(1));
        }
    }

    fn assert_globals_present(globals: &[String]) {
        for expected in [
            "wl_compositor",
            "wl_subcompositor",
            "xdg_wm_base",
            "zxdg_decoration_manager_v1",
            "zwlr_layer_shell_v1",
            "wl_data_device_manager",
            "zwp_linux_dmabuf_v1",
            "wp_viewporter",
            "wp_fractional_scale_manager_v1",
            "wl_shm",
            "wl_seat",
            "wl_output",
            "zxdg_output_manager_v1",
            "wp_presentation",
        ] {
            assert!(
                globals.iter().any(|global| global == expected),
                "missing advertised global `{expected}` in {globals:?}"
            );
        }
    }

    fn temporary_socket_path() -> std::path::PathBuf {
        let mut path = env::temp_dir();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        path.push(format!("nekoland-protocol-test-{}-{unique}.sock", std::process::id()));
        path
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for TestClientState {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _data: &(),
            _conn: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global { name, interface, .. } = event {
                state.globals.push(interface.clone());

                match interface.as_str() {
                    "wl_compositor" => {
                        let compositor =
                            registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ());
                        state.base_surface = Some(compositor.create_surface(qh, ()));
                        state.maybe_init_toplevel(qh);
                    }
                    "xdg_wm_base" => {
                        state.wm_base =
                            Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                        state.maybe_init_toplevel(qh);
                    }
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TestClientState {
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

    impl Dispatch<xdg_surface::XdgSurface, ()> for TestClientState {
        fn event(
            state: &mut Self,
            xdg_surface: &xdg_surface::XdgSurface,
            event: xdg_surface::Event,
            _data: &(),
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial, .. } = event {
                state.configure_serial = Some(serial);
                xdg_surface.ack_configure(serial);
                if let Some(surface) = state.base_surface.as_ref() {
                    surface.commit();
                }
            }
        }
    }

    delegate_noop!(TestClientState: ignore wl_compositor::WlCompositor);
    delegate_noop!(TestClientState: ignore wl_surface::WlSurface);
    delegate_noop!(TestClientState: ignore xdg_toplevel::XdgToplevel);

    impl TestClientState {
        fn maybe_init_toplevel(&mut self, qh: &QueueHandle<Self>) {
            if self.base_surface.is_none() || self.wm_base.is_none() || self.xdg_surface.is_some() {
                return;
            }

            let surface =
                self.base_surface.as_ref().expect("surface presence checked immediately above");
            let wm_base =
                self.wm_base.as_ref().expect("wm_base presence checked immediately above");

            let xdg_surface = wm_base.get_xdg_surface(surface, qh, ());
            let toplevel = xdg_surface.get_toplevel(qh, ());
            surface.commit();
            self.xdg_surface = Some((xdg_surface, toplevel));
        }
    }
}
