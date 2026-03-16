//! Protocol-facing event types and the bridge that moves them into ECS-owned pending resources.
//!
//! Smithay callbacks enqueue `ProtocolEvent`s here first; later, `ProtocolState::flush_into_ecs`
//! translates them into the typed request queues consumed by shell/layout systems.

pub mod compositor;
pub mod data_device;
pub mod dmabuf;
pub mod foreign_toplevel_list;
pub mod fractional_scale;
pub mod idle_notify;
pub mod layer_shell;
pub mod output_management;
pub mod plugin;
pub mod presentation_time;
pub mod primary_selection;
pub mod screencopy;
pub mod session_lock;
pub mod viewporter;
pub mod xdg_activation;
pub mod xdg_decoration;
pub mod xdg_shell;

use bevy_ecs::prelude::Resource;
use nekoland_core::bridge::{EventBridge, WaylandBridge};
use nekoland_ecs::components::{LayerAnchor, LayerLevel, LayerMargins, X11WindowType};
use nekoland_ecs::kinds::ProtocolEvent as ProtocolEventKind;
use nekoland_ecs::resources::pending_events::{
    OutputEventRecord, PendingOutputEvents, PendingXdgRequests, PopupPlacement, ResizeEdges,
    SurfaceExtent, WindowLifecycleAction, WindowLifecycleRequest, XdgSurfaceRole,
};
use nekoland_ecs::resources::{
    ClipboardSelection, ClipboardSelectionState, DragAndDropDrop, DragAndDropSession,
    DragAndDropState, LayerLifecycleAction, LayerLifecycleRequest, LayerSurfaceCreateSpec,
    PendingLayerRequests, PendingWindowControls, PendingX11Requests, PrimarySelection,
    PrimarySelectionState, SelectionOwner, X11LifecycleAction, X11LifecycleRequest,
    X11WindowGeometry,
};
use nekoland_ecs::selectors::SurfaceId;
use serde::{Deserialize, Serialize};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

pub use plugin::{
    ProtocolCursorImage, ProtocolCursorState, ProtocolDmabufSupport, ProtocolPlugin,
    ProtocolSeatDispatchSet, ProtocolServerState, XWaylandServerState,
};

/// Trait implemented by protocol-state marker types that advertise one or more Wayland globals.
pub trait ProtocolGlobals {
    const GLOBALS: &'static [&'static str];

    fn globals(&self) -> &'static [&'static str] {
        Self::GLOBALS
    }
}

/// High-level protocol notifications that need to cross from callback-driven Smithay code into
/// the compositor's scheduled ECS world.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProtocolEvent {
    SurfaceCommitted {
        surface_id: u64,
        role: XdgSurfaceRole,
        size: Option<SurfaceExtent>,
    },
    ConfigureRequested {
        surface_id: u64,
        role: XdgSurfaceRole,
    },
    AckConfigure {
        surface_id: u64,
        role: XdgSurfaceRole,
        serial: u32,
    },
    ToplevelMetadataChanged {
        surface_id: u64,
        title: Option<String>,
        app_id: Option<String>,
    },
    MoveRequested {
        surface_id: u64,
        seat_name: String,
        serial: u32,
    },
    ResizeRequested {
        surface_id: u64,
        seat_name: String,
        serial: u32,
        edges: ResizeEdges,
    },
    MaximizeRequested {
        surface_id: u64,
    },
    UnMaximizeRequested {
        surface_id: u64,
    },
    FullscreenRequested {
        surface_id: u64,
        output_name: Option<String>,
    },
    UnFullscreenRequested {
        surface_id: u64,
    },
    MinimizeRequested {
        surface_id: u64,
    },
    ActivationRequested {
        surface_id: u64,
    },
    PopupCreated {
        surface_id: u64,
        parent_surface_id: Option<u64>,
        placement: PopupPlacement,
    },
    PopupRepositionRequested {
        surface_id: u64,
        placement: PopupPlacement,
    },
    PopupGrabRequested {
        surface_id: u64,
        seat_name: String,
        serial: u32,
    },
    SurfaceDestroyed {
        surface_id: u64,
        role: XdgSurfaceRole,
    },
    LayerSurfaceCreated {
        surface_id: u64,
        namespace: String,
        output_name: Option<String>,
        layer: LayerLevel,
        anchor: LayerAnchor,
        desired_width: u32,
        desired_height: u32,
        exclusive_zone: i32,
        margins: LayerMargins,
    },
    LayerSurfaceCommitted {
        surface_id: u64,
        size: Option<SurfaceExtent>,
        anchor: LayerAnchor,
        desired_width: u32,
        desired_height: u32,
        exclusive_zone: i32,
        margins: LayerMargins,
    },
    LayerSurfaceDestroyed {
        surface_id: u64,
    },
    X11WindowMapped {
        surface_id: u64,
        window_id: u32,
        override_redirect: bool,
        popup: bool,
        transient_for: Option<u32>,
        window_type: Option<X11WindowType>,
        title: String,
        app_id: String,
        geometry: X11WindowGeometry,
    },
    X11WindowReconfigured {
        surface_id: u64,
        title: String,
        app_id: String,
        popup: bool,
        transient_for: Option<u32>,
        window_type: Option<X11WindowType>,
        geometry: X11WindowGeometry,
    },
    X11WindowMaximizeRequested {
        surface_id: u64,
    },
    X11WindowUnMaximizeRequested {
        surface_id: u64,
    },
    X11WindowFullscreenRequested {
        surface_id: u64,
    },
    X11WindowUnFullscreenRequested {
        surface_id: u64,
    },
    X11WindowMinimizeRequested {
        surface_id: u64,
    },
    X11WindowUnMinimizeRequested {
        surface_id: u64,
    },
    X11WindowMoveRequested {
        surface_id: u64,
        button: u32,
    },
    X11WindowResizeRequested {
        surface_id: u64,
        button: u32,
        edges: ResizeEdges,
    },
    X11WindowUnmapped {
        surface_id: u64,
    },
    X11WindowDestroyed {
        surface_id: u64,
    },
    OutputAnnounced {
        output_name: String,
    },
    ClipboardSelectionChanged {
        seat_name: String,
        mime_types: Vec<String>,
    },
    DragStarted {
        seat_name: String,
        source_surface_id: Option<u64>,
        icon_surface_id: Option<u64>,
        mime_types: Vec<String>,
    },
    DragDropped {
        seat_name: String,
        target_surface_id: Option<u64>,
        validated: bool,
    },
    DragAccepted {
        seat_name: String,
        mime_type: Option<String>,
    },
    DragActionSelected {
        seat_name: String,
        action: String,
    },
    ClipboardSelectionPersisted {
        persisted_mime_types: Vec<String>,
    },
    PrimarySelectionChanged {
        seat_name: String,
        mime_types: Vec<String>,
    },
    PrimarySelectionPersisted {
        persisted_mime_types: Vec<String>,
    },
}

impl ProtocolEventKind for ProtocolEvent {}

#[cfg(test)]
mod kind_tests {
    use super::ProtocolEvent;
    use nekoland_ecs::kinds::ProtocolEvent as ProtocolEventKind;

    fn assert_protocol_event<T: ProtocolEventKind>() {}

    #[test]
    fn protocol_event_implements_protocol_event_trait() {
        assert_protocol_event::<ProtocolEvent>();
    }
}

#[cfg(test)]
mod tests {
    use nekoland_core::bridge::WaylandBridge;
    use nekoland_ecs::resources::{
        ClipboardSelectionState, DragAndDropState, PendingLayerRequests, PendingOutputEvents,
        PendingWindowControls, PendingX11Requests, PendingXdgRequests, PrimarySelectionState,
    };

    use super::{ProtocolEvent, ProtocolFlushTargets, ProtocolState, supported_protocols};

    #[test]
    fn supported_protocol_lists_include_xdg_activation() {
        let state = ProtocolState::default();
        assert!(state.supported_globals().contains(&"ext_foreign_toplevel_list_v1"));
        assert!(state.supported_globals().contains(&"xdg_activation_v1"));
        assert!(supported_protocols().contains(&"ext_foreign_toplevel_list_v1"));
        assert!(supported_protocols().contains(&"xdg_activation_v1"));
    }

    #[test]
    fn activation_request_flushes_into_window_focus_control() {
        let mut protocol_state = ProtocolState::default();
        protocol_state.queue_event(ProtocolEvent::ActivationRequested { surface_id: 77 });

        let mut pending_xdg_requests = PendingXdgRequests::default();
        let mut pending_layer_requests = PendingLayerRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_x11_requests = PendingX11Requests::default();
        let mut pending_output_events = PendingOutputEvents::default();
        let mut clipboard_selection = ClipboardSelectionState::default();
        let mut drag_and_drop = DragAndDropState::default();
        let mut primary_selection = PrimarySelectionState::default();

        protocol_state.flush_into_ecs(&mut ProtocolFlushTargets {
            pending_xdg_requests: &mut pending_xdg_requests,
            pending_layer_requests: &mut pending_layer_requests,
            pending_window_controls: &mut pending_window_controls,
            pending_x11_requests: &mut pending_x11_requests,
            pending_output_events: &mut pending_output_events,
            clipboard_selection: &mut clipboard_selection,
            drag_and_drop: &mut drag_and_drop,
            primary_selection: &mut primary_selection,
        });

        assert!(pending_xdg_requests.is_empty());
        assert_eq!(pending_window_controls.as_slice().len(), 1);
        assert_eq!(pending_window_controls.as_slice()[0].surface_id.0, 77);
        assert!(pending_window_controls.as_slice()[0].focus);
    }
}

/// Aggregates per-protocol Smithay state together with the bridge that buffers protocol events
/// until the protocol schedule flushes them into ECS resources.
#[derive(Debug, Default, Resource)]
pub struct ProtocolState {
    pub compositor: compositor::CompositorProtocolState,
    pub xdg_shell: xdg_shell::XdgShellState,
    pub foreign_toplevel_list: foreign_toplevel_list::ForeignToplevelListProtocolState,
    pub xdg_activation: xdg_activation::XdgActivationState,
    pub layer_shell: layer_shell::LayerShellState,
    pub data_device: data_device::DataDeviceState,
    pub primary_selection: primary_selection::PrimarySelectionProtocolState,
    pub dmabuf: dmabuf::DmabufState,
    pub viewporter: viewporter::ViewporterState,
    pub fractional_scale: fractional_scale::FractionalScaleState,
    pub xdg_decoration: xdg_decoration::XdgDecorationState,
    pub presentation_time: presentation_time::PresentationTimeState,
    pub screencopy: screencopy::ScreencopyState,
    pub output_management: output_management::OutputManagementState,
    pub session_lock: session_lock::SessionLockState,
    pub idle_notify: idle_notify::IdleNotifyState,
    bridge: EventBridge<ProtocolEvent>,
}

impl ProtocolState {
    /// Moves buffered protocol events into the typed ECS request/resources that downstream systems
    /// consume during layout, focus, selection, and output handling.
    pub fn flush_into_ecs(&mut self, targets: &mut ProtocolFlushTargets<'_>) {
        let pending_xdg_requests = &mut *targets.pending_xdg_requests;
        let pending_layer_requests = &mut *targets.pending_layer_requests;
        let pending_x11_requests = &mut *targets.pending_x11_requests;
        let pending_window_controls = &mut *targets.pending_window_controls;
        let pending_output_events = &mut *targets.pending_output_events;
        let clipboard_selection = &mut *targets.clipboard_selection;
        let drag_and_drop = &mut *targets.drag_and_drop;
        let primary_selection = &mut *targets.primary_selection;
        for event in self.bridge.drain() {
            match event {
                ProtocolEvent::SurfaceCommitted { surface_id, role, size } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::Committed { role, size },
                    });
                }
                ProtocolEvent::ConfigureRequested { surface_id, role } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::ConfigureRequested { role },
                    });
                }
                ProtocolEvent::AckConfigure { surface_id, role, serial } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::AckConfigure { role, serial },
                    });
                }
                ProtocolEvent::ToplevelMetadataChanged { surface_id, title, app_id } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::MetadataChanged { title, app_id },
                    });
                }
                ProtocolEvent::MoveRequested { surface_id, seat_name, serial } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::InteractiveMove { seat_name, serial },
                    });
                }
                ProtocolEvent::ResizeRequested { surface_id, seat_name, serial, edges } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::InteractiveResize {
                            seat_name,
                            serial,
                            edges,
                        },
                    });
                }
                ProtocolEvent::MaximizeRequested { surface_id } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::Maximize,
                    });
                }
                ProtocolEvent::UnMaximizeRequested { surface_id } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::UnMaximize,
                    });
                }
                ProtocolEvent::FullscreenRequested { surface_id, output_name } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::Fullscreen { output_name },
                    });
                }
                ProtocolEvent::UnFullscreenRequested { surface_id } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::UnFullscreen,
                    });
                }
                ProtocolEvent::MinimizeRequested { surface_id } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::Minimize,
                    });
                }
                ProtocolEvent::ActivationRequested { surface_id } => {
                    pending_window_controls.surface(SurfaceId(surface_id)).focus();
                }
                ProtocolEvent::PopupCreated { surface_id, parent_surface_id, placement } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::PopupCreated {
                            parent_surface_id,
                            placement,
                        },
                    });
                }
                ProtocolEvent::PopupRepositionRequested { surface_id, placement } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::PopupRepositioned { placement },
                    });
                }
                ProtocolEvent::PopupGrabRequested { surface_id, seat_name, serial } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::PopupGrab { seat_name, serial },
                    });
                }
                ProtocolEvent::SurfaceDestroyed { surface_id, role } => {
                    pending_xdg_requests.push(WindowLifecycleRequest {
                        surface_id,
                        action: WindowLifecycleAction::Destroyed { role },
                    });
                }
                ProtocolEvent::LayerSurfaceCreated {
                    surface_id,
                    namespace,
                    output_name,
                    layer,
                    anchor,
                    desired_width,
                    desired_height,
                    exclusive_zone,
                    margins,
                } => {
                    pending_layer_requests.push(LayerLifecycleRequest {
                        surface_id,
                        action: LayerLifecycleAction::Created {
                            spec: LayerSurfaceCreateSpec {
                                namespace,
                                output_name,
                                layer,
                                anchor,
                                desired_width,
                                desired_height,
                                exclusive_zone,
                                margins,
                            },
                        },
                    });
                }
                ProtocolEvent::LayerSurfaceCommitted {
                    surface_id,
                    size,
                    anchor,
                    desired_width,
                    desired_height,
                    exclusive_zone,
                    margins,
                } => {
                    pending_layer_requests.push(LayerLifecycleRequest {
                        surface_id,
                        action: LayerLifecycleAction::Committed {
                            size,
                            anchor,
                            desired_width,
                            desired_height,
                            exclusive_zone,
                            margins,
                        },
                    });
                }
                ProtocolEvent::LayerSurfaceDestroyed { surface_id } => {
                    pending_layer_requests.push(LayerLifecycleRequest {
                        surface_id,
                        action: LayerLifecycleAction::Destroyed,
                    });
                }
                ProtocolEvent::X11WindowMapped {
                    surface_id,
                    window_id,
                    override_redirect,
                    popup,
                    transient_for,
                    window_type,
                    title,
                    app_id,
                    geometry,
                } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::Mapped {
                            window_id,
                            override_redirect,
                            popup,
                            transient_for,
                            window_type,
                            title,
                            app_id,
                            geometry,
                        },
                    });
                }
                ProtocolEvent::X11WindowReconfigured {
                    surface_id,
                    title,
                    app_id,
                    popup,
                    transient_for,
                    window_type,
                    geometry,
                } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::Reconfigured {
                            title,
                            app_id,
                            popup,
                            transient_for,
                            window_type,
                            geometry,
                        },
                    });
                }
                ProtocolEvent::X11WindowMaximizeRequested { surface_id } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::Maximize,
                    });
                }
                ProtocolEvent::X11WindowUnMaximizeRequested { surface_id } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::UnMaximize,
                    });
                }
                ProtocolEvent::X11WindowFullscreenRequested { surface_id } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::Fullscreen,
                    });
                }
                ProtocolEvent::X11WindowUnFullscreenRequested { surface_id } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::UnFullscreen,
                    });
                }
                ProtocolEvent::X11WindowMinimizeRequested { surface_id } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::Minimize,
                    });
                }
                ProtocolEvent::X11WindowUnMinimizeRequested { surface_id } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::UnMinimize,
                    });
                }
                ProtocolEvent::X11WindowMoveRequested { surface_id, button } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::InteractiveMove { button },
                    });
                }
                ProtocolEvent::X11WindowResizeRequested { surface_id, button, edges } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::InteractiveResize { button, edges },
                    });
                }
                ProtocolEvent::X11WindowUnmapped { surface_id } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::Unmapped,
                    });
                }
                ProtocolEvent::X11WindowDestroyed { surface_id } => {
                    pending_x11_requests.push(X11LifecycleRequest {
                        surface_id,
                        action: X11LifecycleAction::Destroyed,
                    });
                }
                ProtocolEvent::OutputAnnounced { output_name } => {
                    pending_output_events
                        .push(OutputEventRecord { output_name, change: "announced".to_owned() });
                }
                ProtocolEvent::ClipboardSelectionChanged { seat_name, mime_types } => {
                    clipboard_selection.selection = if mime_types.is_empty() {
                        None
                    } else {
                        Some(ClipboardSelection {
                            seat_name,
                            mime_types,
                            owner: SelectionOwner::Client,
                            persisted_mime_types: Vec::new(),
                        })
                    };
                }
                ProtocolEvent::DragStarted {
                    seat_name,
                    source_surface_id,
                    icon_surface_id,
                    mime_types,
                } => {
                    drag_and_drop.active_session = Some(DragAndDropSession {
                        seat_name,
                        source_surface_id,
                        icon_surface_id,
                        mime_types,
                        accepted_mime_type: None,
                        chosen_action: None,
                    });
                    drag_and_drop.last_drop = None;
                }
                ProtocolEvent::DragDropped { seat_name, target_surface_id, validated } => {
                    let source_surface_id = drag_and_drop
                        .active_session
                        .as_ref()
                        .and_then(|session| session.source_surface_id);
                    let mime_types = drag_and_drop
                        .active_session
                        .as_ref()
                        .map(|session| session.mime_types.clone())
                        .unwrap_or_default();
                    drag_and_drop.last_drop = Some(DragAndDropDrop {
                        seat_name,
                        source_surface_id,
                        target_surface_id,
                        validated,
                        mime_types,
                    });
                    drag_and_drop.active_session = None;
                }
                ProtocolEvent::DragAccepted { seat_name, mime_type } => {
                    if let Some(session) = drag_and_drop.active_session.as_mut()
                        && session.seat_name == seat_name
                    {
                        session.accepted_mime_type = mime_type;
                    }
                }
                ProtocolEvent::DragActionSelected { seat_name, action } => {
                    if let Some(session) = drag_and_drop.active_session.as_mut()
                        && session.seat_name == seat_name
                    {
                        session.chosen_action = Some(action);
                    }
                }
                ProtocolEvent::ClipboardSelectionPersisted { persisted_mime_types } => {
                    if let Some(selection) = clipboard_selection.selection.as_mut() {
                        selection.owner = SelectionOwner::Compositor;
                        selection.persisted_mime_types = persisted_mime_types;
                    }
                }
                ProtocolEvent::PrimarySelectionChanged { seat_name, mime_types } => {
                    primary_selection.selection = if mime_types.is_empty() {
                        None
                    } else {
                        Some(PrimarySelection {
                            seat_name,
                            mime_types,
                            owner: SelectionOwner::Client,
                            persisted_mime_types: Vec::new(),
                        })
                    };
                }
                ProtocolEvent::PrimarySelectionPersisted { persisted_mime_types } => {
                    if let Some(selection) = primary_selection.selection.as_mut() {
                        selection.owner = SelectionOwner::Compositor;
                        selection.persisted_mime_types = persisted_mime_types;
                    }
                }
            }
        }
    }

    pub fn supported_globals(&self) -> Vec<&'static str> {
        let mut globals = Vec::new();
        globals.extend_from_slice(self.compositor.globals());
        globals.extend_from_slice(self.xdg_shell.globals());
        globals.extend_from_slice(self.foreign_toplevel_list.globals());
        globals.extend_from_slice(self.xdg_activation.globals());
        globals.extend_from_slice(self.xdg_decoration.globals());
        globals.extend_from_slice(self.layer_shell.globals());
        globals.extend_from_slice(self.data_device.globals());
        globals.extend_from_slice(self.primary_selection.globals());
        globals.extend_from_slice(self.dmabuf.globals());
        globals.extend_from_slice(self.viewporter.globals());
        globals.extend_from_slice(self.fractional_scale.globals());
        globals.extend_from_slice(self.presentation_time.globals());
        globals.extend(["wl_shm", "wl_seat", "wl_output", "zxdg_output_manager_v1"]);
        globals
    }
}

pub struct ProtocolFlushTargets<'a> {
    pub pending_xdg_requests: &'a mut PendingXdgRequests,
    pub pending_layer_requests: &'a mut PendingLayerRequests,
    pub pending_window_controls: &'a mut PendingWindowControls,
    pub pending_x11_requests: &'a mut PendingX11Requests,
    pub pending_output_events: &'a mut PendingOutputEvents,
    pub clipboard_selection: &'a mut ClipboardSelectionState,
    pub drag_and_drop: &'a mut DragAndDropState,
    pub primary_selection: &'a mut PrimarySelectionState,
}

impl WaylandBridge for ProtocolState {
    type Event = ProtocolEvent;

    fn queue_event(&mut self, event: Self::Event) {
        self.bridge.push(event);
    }
}

#[derive(Debug, Clone, Default, Resource)]
pub struct ProtocolRegistry {
    pub globals: Vec<&'static str>,
}

/// One compositor-managed surface tracked by the protocol runtime.
#[derive(Debug, Clone)]
pub struct ProtocolSurfaceEntry {
    pub kind: ProtocolSurfaceKind,
    pub surface: WlSurface,
}

/// Surface classes the protocol runtime distinguishes when registering surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolSurfaceKind {
    Toplevel,
    Popup,
    Layer,
}

/// Lookup table from compositor surface id to live Smithay surface handle.
#[derive(Debug, Default)]
pub struct ProtocolSurfaceRegistry {
    pub surfaces: std::collections::HashMap<u64, ProtocolSurfaceEntry>,
}

impl ProtocolSurfaceRegistry {
    /// Returns the live Smithay surface handle associated with a compositor surface id.
    pub fn surface(&self, surface_id: u64) -> Option<&WlSurface> {
        self.surfaces.get(&surface_id).map(|entry| &entry.surface)
    }
}

/// Static list of protocol globals the compositor currently intends to expose.
pub fn supported_protocols() -> &'static [&'static str] {
    &[
        "wl_compositor",
        "wl_subcompositor",
        "xdg_wm_base",
        "ext_foreign_toplevel_list_v1",
        "xdg_activation_v1",
        "zxdg_decoration_manager_v1",
        "zwlr_layer_shell_v1",
        "wl_data_device_manager",
        "zwp_primary_selection_device_manager_v1",
        "zwp_linux_dmabuf_v1",
        "wp_viewporter",
        "wp_fractional_scale_manager_v1",
        "wl_shm",
        "wl_seat",
        "wl_output",
        "zxdg_output_manager_v1",
        "wp_presentation",
    ]
}
