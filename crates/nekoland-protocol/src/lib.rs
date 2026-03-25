#![warn(missing_docs)]

//! Protocol-facing event types and the bridge that moves them into ECS-owned pending resources.
//!
//! Smithay callbacks enqueue `ProtocolEvent`s here first; later, `ProtocolState::flush_into_ecs`
//! translates them into the typed request queues consumed by shell/layout systems.

use std::collections::BTreeSet;

/// `wl_compositor` surface registration and commit tracking.
pub mod compositor;
/// Clipboard, drag-and-drop, and `wl_data_device` integration.
pub mod data_device;
/// Linux dma-buf protocol support and capability bookkeeping.
pub mod dmabuf;
/// Export of compositor toplevel metadata to external protocol clients.
pub mod foreign_toplevel_list;
/// Fractional-scale negotiation for surfaces and outputs.
pub mod fractional_scale;
/// Idle-notify protocol state and request handling.
pub mod idle_notify;
/// Layer-shell surface lifecycle and request translation.
pub mod layer_shell;
/// Output-management protocol integration and request forwarding.
pub mod output_management;
pub mod plugin;
/// Presentation-time feedback protocol support.
pub mod presentation_time;
/// Primary-selection protocol integration.
pub mod primary_selection;
/// Protocol-owned resources and re-exports consumed by the rest of the compositor.
pub mod resources;
/// Screencopy protocol requests and capture bookkeeping.
pub mod screencopy;
/// Session-lock protocol surface handling.
pub mod session_lock;
pub mod subapp;
/// `wp_viewporter` source/destination crop support.
pub mod viewporter;
/// Activation-token handling for focus/raise requests.
pub mod xdg_activation;
/// XDG decoration negotiation and mode tracking.
pub mod xdg_decoration;
/// XDG shell toplevel/popup lifecycle handling.
pub mod xdg_shell;

use bevy_ecs::prelude::Resource;
use nekoland_core::bridge::{EventBridge, WaylandBridge};
use nekoland_ecs::components::{
    LayerAnchor, LayerLevel, LayerMargins, SeatId, WindowManagementHints, WindowSceneGeometry,
    X11WindowType,
};
use nekoland_ecs::kinds::ProtocolEvent as ProtocolEventKind;
use nekoland_ecs::resources::{
    ClipboardSelection, ClipboardSelectionState, DragAndDropDrop, DragAndDropSession,
    DragAndDropState, LayerLifecycleAction, LayerLifecycleRequest, LayerSurfaceCreateSpec,
    OutputEventRecord, PendingLayerRequests, PendingOutputEvents, PendingPopupEvents,
    PendingWindowControls, PendingWindowEvents, PendingXdgRequests, PopupEvent, PopupEventRequest,
    PopupPlacement, PrimarySelection, PrimarySelectionState, ResizeEdges, SeatRegistry,
    SelectionOwner, SurfaceExtent, WindowEvent, WindowEventRequest, WindowManagerRequest,
    X11WindowGeometry, XdgSurfaceRole,
};
use nekoland_ecs::selectors::SurfaceId;
use serde::{Deserialize, Serialize};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

/// Protocol server and XWayland runtime snapshots mirrored out of the Wayland sub-app.
pub use nekoland_ecs::resources::{ProtocolServerState, XWaylandServerState};
/// Cursor, dma-buf, and protocol plugin entrypoints exposed to the rest of the workspace.
pub use plugin::server::{ProtocolCursorImage, ProtocolCursorState, ProtocolDmabufSupport};
pub use plugin::{ProtocolPlugin, ProtocolSeatDispatchSystems};
/// Wayland sub-app entrypoints used by the root compositor runner.
pub use subapp::{
    WaylandSubAppPlugin, configure_wayland_subapp, extract_wayland_subapp_inputs,
    sync_wayland_subapp_back,
};

/// Trait implemented by protocol-state marker types that advertise one or more Wayland globals.
pub trait ProtocolGlobals {
    /// Names of Wayland globals contributed by this protocol-state marker.
    const GLOBALS: &'static [&'static str];

    /// Returns the globals advertised by this protocol-state marker.
    fn globals(&self) -> &'static [&'static str] {
        Self::GLOBALS
    }
}

/// High-level protocol notifications that need to cross from callback-driven Smithay code into
/// the compositor's scheduled ECS world.
#[allow(missing_docs)]
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
        transient_parent_surface_id: Option<u64>,
        popup_placement: Option<PopupPlacement>,
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
        transient_parent_surface_id: Option<u64>,
        popup_placement: Option<PopupPlacement>,
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
    use nekoland_ecs::components::X11WindowType;
    use nekoland_ecs::resources::PendingWindowControls;

    use nekoland_ecs::resources::{
        ClipboardSelectionState, DragAndDropState, PendingLayerRequests, PendingOutputEvents,
        PendingPopupEvents, PendingWindowEvents, PendingXdgRequests, PopupEvent,
        PrimarySelectionState, ResizeEdges, SeatRegistry, SurfaceExtent, WindowEvent,
        X11WindowGeometry, XdgSurfaceRole,
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
        let mut pending_window_events = PendingWindowEvents::default();
        let mut pending_popup_events = PendingPopupEvents::default();
        let mut pending_layer_requests = PendingLayerRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_output_events = PendingOutputEvents::default();
        let mut seat_registry = SeatRegistry::default();
        let mut clipboard_selection = ClipboardSelectionState::default();
        let mut drag_and_drop = DragAndDropState::default();
        let mut primary_selection = PrimarySelectionState::default();

        protocol_state.flush_into_ecs(&mut ProtocolFlushTargets {
            pending_xdg_requests: &mut pending_xdg_requests,
            pending_window_events: &mut pending_window_events,
            pending_popup_events: &mut pending_popup_events,
            pending_layer_requests: &mut pending_layer_requests,
            pending_window_controls: &mut pending_window_controls,
            pending_output_events: &mut pending_output_events,
            seat_registry: &mut seat_registry,
            clipboard_selection: &mut clipboard_selection,
            drag_and_drop: &mut drag_and_drop,
            primary_selection: &mut primary_selection,
        });

        assert!(pending_xdg_requests.is_empty());
        assert!(pending_window_events.is_empty());
        assert!(pending_popup_events.is_empty());
        assert_eq!(pending_window_controls.as_slice().len(), 1);
        assert_eq!(pending_window_controls.as_slice()[0].surface_id.0, 77);
        assert!(pending_window_controls.as_slice()[0].focus);
    }

    #[test]
    fn toplevel_commit_flushes_into_unified_window_events() {
        let mut protocol_state = ProtocolState::default();
        protocol_state.queue_event(ProtocolEvent::SurfaceCommitted {
            surface_id: 42,
            role: XdgSurfaceRole::Toplevel,
            size: Some(SurfaceExtent { width: 800, height: 600 }),
        });

        let mut pending_xdg_requests = PendingXdgRequests::default();
        let mut pending_window_events = PendingWindowEvents::default();
        let mut pending_popup_events = PendingPopupEvents::default();
        let mut pending_layer_requests = PendingLayerRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_output_events = PendingOutputEvents::default();
        let mut seat_registry = SeatRegistry::default();
        let mut clipboard_selection = ClipboardSelectionState::default();
        let mut drag_and_drop = DragAndDropState::default();
        let mut primary_selection = PrimarySelectionState::default();

        protocol_state.flush_into_ecs(&mut ProtocolFlushTargets {
            pending_xdg_requests: &mut pending_xdg_requests,
            pending_window_events: &mut pending_window_events,
            pending_popup_events: &mut pending_popup_events,
            pending_layer_requests: &mut pending_layer_requests,
            pending_window_controls: &mut pending_window_controls,
            pending_output_events: &mut pending_output_events,
            seat_registry: &mut seat_registry,
            clipboard_selection: &mut clipboard_selection,
            drag_and_drop: &mut drag_and_drop,
            primary_selection: &mut primary_selection,
        });

        assert!(pending_xdg_requests.is_empty());
        assert!(pending_popup_events.is_empty());
        let events = pending_window_events.as_slice();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].action, WindowEvent::Upsert { .. }));
        assert!(matches!(
            events[1].action,
            WindowEvent::Committed {
                size: Some(SurfaceExtent { width: 800, height: 600 }),
                attached: true
            }
        ));
    }

    #[test]
    fn x11_helper_map_flushes_into_unified_popup_events() {
        let mut protocol_state = ProtocolState::default();
        protocol_state.queue_event(ProtocolEvent::X11WindowMapped {
            surface_id: 64,
            window_id: 9,
            override_redirect: true,
            popup: true,
            transient_parent_surface_id: Some(7),
            popup_placement: Some(nekoland_ecs::resources::PopupPlacement {
                x: 3,
                y: 4,
                width: 333,
                height: 444,
                reposition_token: None,
            }),
            window_type: Some(X11WindowType::Tooltip),
            title: "Tip".to_owned(),
            app_id: "x11.test".to_owned(),
            geometry: X11WindowGeometry { x: 11, y: 22, width: 333, height: 444 },
        });
        protocol_state.queue_event(ProtocolEvent::X11WindowResizeRequested {
            surface_id: 64,
            button: 1,
            edges: ResizeEdges::TopLeft,
        });

        let mut pending_xdg_requests = PendingXdgRequests::default();
        let mut pending_window_events = PendingWindowEvents::default();
        let mut pending_popup_events = PendingPopupEvents::default();
        let mut pending_layer_requests = PendingLayerRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_output_events = PendingOutputEvents::default();
        let mut seat_registry = SeatRegistry::default();
        let mut clipboard_selection = ClipboardSelectionState::default();
        let mut drag_and_drop = DragAndDropState::default();
        let mut primary_selection = PrimarySelectionState::default();

        protocol_state.flush_into_ecs(&mut ProtocolFlushTargets {
            pending_xdg_requests: &mut pending_xdg_requests,
            pending_window_events: &mut pending_window_events,
            pending_popup_events: &mut pending_popup_events,
            pending_layer_requests: &mut pending_layer_requests,
            pending_window_controls: &mut pending_window_controls,
            pending_output_events: &mut pending_output_events,
            seat_registry: &mut seat_registry,
            clipboard_selection: &mut clipboard_selection,
            drag_and_drop: &mut drag_and_drop,
            primary_selection: &mut primary_selection,
        });

        assert!(pending_window_events.is_empty());
        let events = pending_popup_events.as_slice();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].action,
            PopupEvent::Created {
                parent_surface_id: 7,
                placement: nekoland_ecs::resources::PopupPlacement {
                    x: 3,
                    y: 4,
                    width: 333,
                    height: 444,
                    reposition_token: None,
                }
            }
        ));
        assert!(matches!(
            events[1].action,
            PopupEvent::Committed {
                size: Some(SurfaceExtent { width: 333, height: 444 }),
                attached: true
            }
        ));
    }
}

/// Aggregates per-protocol Smithay state together with the bridge that buffers protocol events
/// until the protocol schedule flushes them into ECS resources.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Resource)]
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
    x11_helper_surfaces: BTreeSet<u64>,
    x11_popup_surfaces: BTreeSet<u64>,
    bridge: EventBridge<ProtocolEvent>,
}

impl ProtocolState {
    /// Moves buffered protocol events into the typed ECS request/resources that downstream systems
    /// consume during layout, focus, selection, and output handling.
    pub fn flush_into_ecs(&mut self, targets: &mut ProtocolFlushTargets<'_>) {
        let _pending_xdg_requests = &mut *targets.pending_xdg_requests;
        let pending_window_events = &mut *targets.pending_window_events;
        let pending_popup_events = &mut *targets.pending_popup_events;
        let pending_layer_requests = &mut *targets.pending_layer_requests;
        let pending_window_controls = &mut *targets.pending_window_controls;
        let pending_output_events = &mut *targets.pending_output_events;
        let clipboard_selection = &mut *targets.clipboard_selection;
        let drag_and_drop = &mut *targets.drag_and_drop;
        let primary_selection = &mut *targets.primary_selection;
        let seat_registry = &mut *targets.seat_registry;
        for event in self.bridge.drain() {
            match event {
                ProtocolEvent::SurfaceCommitted {
                    surface_id,
                    role: XdgSurfaceRole::Toplevel,
                    size,
                } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::Upsert {
                            title: None,
                            app_id: None,
                            hints: WindowManagementHints::native_wayland(),
                            scene_geometry: size.map(|size| WindowSceneGeometry {
                                x: 0,
                                y: 0,
                                width: size.width.max(1),
                                height: size.height.max(1),
                            }),
                            attached: size.is_some(),
                        },
                    });
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::Committed { size, attached: size.is_some() },
                    });
                }
                ProtocolEvent::SurfaceCommitted {
                    surface_id,
                    role: XdgSurfaceRole::Popup,
                    size,
                } => {
                    pending_popup_events.push(PopupEventRequest {
                        surface_id,
                        action: PopupEvent::Committed { size, attached: size.is_some() },
                    });
                }
                ProtocolEvent::ConfigureRequested { role: XdgSurfaceRole::Popup, .. } => {}
                ProtocolEvent::ConfigureRequested { role: XdgSurfaceRole::Toplevel, .. }
                | ProtocolEvent::AckConfigure { role: XdgSurfaceRole::Toplevel, .. } => {}
                ProtocolEvent::AckConfigure { role: XdgSurfaceRole::Popup, .. } => {}
                ProtocolEvent::ToplevelMetadataChanged { surface_id, title, app_id } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::Upsert {
                            title,
                            app_id,
                            hints: WindowManagementHints::native_wayland(),
                            scene_geometry: None,
                            attached: false,
                        },
                    });
                }
                ProtocolEvent::MoveRequested { surface_id, .. } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::BeginMove),
                    });
                }
                ProtocolEvent::ResizeRequested { surface_id, edges, .. } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::BeginResize {
                            edges,
                        }),
                    });
                }
                ProtocolEvent::MaximizeRequested { surface_id } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::Maximize),
                    });
                }
                ProtocolEvent::UnMaximizeRequested { surface_id } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::UnMaximize),
                    });
                }
                ProtocolEvent::FullscreenRequested { surface_id, output_name } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::Fullscreen {
                            output_name,
                        }),
                    });
                }
                ProtocolEvent::UnFullscreenRequested { surface_id } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::UnFullscreen),
                    });
                }
                ProtocolEvent::MinimizeRequested { surface_id } => {
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::Minimize),
                    });
                }
                ProtocolEvent::ActivationRequested { surface_id } => {
                    pending_window_controls.surface(SurfaceId(surface_id)).focus();
                }
                ProtocolEvent::PopupCreated { surface_id, parent_surface_id, placement } => {
                    let Some(parent_surface_id) = parent_surface_id else {
                        continue;
                    };
                    pending_popup_events.push(PopupEventRequest {
                        surface_id,
                        action: PopupEvent::Created { parent_surface_id, placement },
                    });
                }
                ProtocolEvent::PopupRepositionRequested { surface_id, placement } => {
                    pending_popup_events.push(PopupEventRequest {
                        surface_id,
                        action: PopupEvent::Repositioned { placement },
                    });
                }
                ProtocolEvent::PopupGrabRequested { surface_id, seat_name, serial } => {
                    let seat_id = seat_id_for_wayland_name(seat_registry, seat_name);
                    pending_popup_events.push(PopupEventRequest {
                        surface_id,
                        action: PopupEvent::Grab { seat_id, serial },
                    });
                }
                ProtocolEvent::SurfaceDestroyed { surface_id, role: XdgSurfaceRole::Toplevel } => {
                    pending_window_events
                        .push(WindowEventRequest { surface_id, action: WindowEvent::Closed });
                }
                ProtocolEvent::SurfaceDestroyed { surface_id, role: XdgSurfaceRole::Popup } => {
                    pending_popup_events
                        .push(PopupEventRequest { surface_id, action: PopupEvent::Closed });
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
                    window_id: _,
                    override_redirect,
                    popup,
                    transient_parent_surface_id,
                    popup_placement,
                    window_type,
                    title,
                    app_id,
                    geometry,
                } => {
                    let helper_surface = x11_helper_surface(popup, window_type);
                    if helper_surface {
                        self.x11_helper_surfaces.insert(surface_id);
                        if let (Some(parent_surface_id), Some(placement)) =
                            (transient_parent_surface_id, popup_placement)
                        {
                            self.x11_popup_surfaces.insert(surface_id);
                            pending_popup_events.push(PopupEventRequest {
                                surface_id,
                                action: PopupEvent::Created { parent_surface_id, placement },
                            });
                            pending_popup_events.push(PopupEventRequest {
                                surface_id,
                                action: PopupEvent::Committed {
                                    size: Some(SurfaceExtent {
                                        width: geometry.width.max(1),
                                        height: geometry.height.max(1),
                                    }),
                                    attached: true,
                                },
                            });
                        } else {
                            self.x11_popup_surfaces.remove(&surface_id);
                        }
                        continue;
                    }
                    self.x11_helper_surfaces.remove(&surface_id);
                    self.x11_popup_surfaces.remove(&surface_id);
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::Upsert {
                            title: Some(title),
                            app_id: Some(app_id),
                            hints: WindowManagementHints::x11(
                                helper_surface,
                                override_redirect,
                                override_redirect || helper_surface,
                                transient_parent_surface_id,
                            ),
                            scene_geometry: Some(WindowSceneGeometry {
                                x: geometry.x as isize,
                                y: geometry.y as isize,
                                width: geometry.width.max(1),
                                height: geometry.height.max(1),
                            }),
                            attached: true,
                        },
                    });
                }
                ProtocolEvent::X11WindowReconfigured {
                    surface_id,
                    title,
                    app_id,
                    popup,
                    transient_parent_surface_id,
                    popup_placement,
                    window_type,
                    geometry,
                } => {
                    let helper_surface = x11_helper_surface(popup, window_type);
                    if helper_surface {
                        self.x11_helper_surfaces.insert(surface_id);
                        if let (Some(parent_surface_id), Some(placement)) =
                            (transient_parent_surface_id, popup_placement)
                        {
                            let event = if self.x11_popup_surfaces.insert(surface_id) {
                                PopupEvent::Created { parent_surface_id, placement }
                            } else {
                                PopupEvent::Repositioned { placement }
                            };
                            pending_popup_events
                                .push(PopupEventRequest { surface_id, action: event });
                            pending_popup_events.push(PopupEventRequest {
                                surface_id,
                                action: PopupEvent::Committed {
                                    size: Some(SurfaceExtent {
                                        width: geometry.width.max(1),
                                        height: geometry.height.max(1),
                                    }),
                                    attached: true,
                                },
                            });
                        } else if self.x11_popup_surfaces.remove(&surface_id) {
                            pending_popup_events
                                .push(PopupEventRequest { surface_id, action: PopupEvent::Closed });
                        }
                        continue;
                    }
                    self.x11_helper_surfaces.remove(&surface_id);
                    if self.x11_popup_surfaces.remove(&surface_id) {
                        pending_popup_events
                            .push(PopupEventRequest { surface_id, action: PopupEvent::Closed });
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::Upsert {
                            title: Some(title),
                            app_id: Some(app_id),
                            hints: WindowManagementHints::x11(
                                helper_surface,
                                false,
                                helper_surface,
                                transient_parent_surface_id,
                            ),
                            scene_geometry: Some(WindowSceneGeometry {
                                x: geometry.x as isize,
                                y: geometry.y as isize,
                                width: geometry.width.max(1),
                                height: geometry.height.max(1),
                            }),
                            attached: true,
                        },
                    });
                }
                ProtocolEvent::X11WindowMaximizeRequested { surface_id } => {
                    if self.x11_helper_surfaces.contains(&surface_id) {
                        continue;
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::Maximize),
                    });
                }
                ProtocolEvent::X11WindowUnMaximizeRequested { surface_id } => {
                    if self.x11_helper_surfaces.contains(&surface_id) {
                        continue;
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::UnMaximize),
                    });
                }
                ProtocolEvent::X11WindowFullscreenRequested { surface_id } => {
                    if self.x11_helper_surfaces.contains(&surface_id) {
                        continue;
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::Fullscreen {
                            output_name: None,
                        }),
                    });
                }
                ProtocolEvent::X11WindowUnFullscreenRequested { surface_id } => {
                    if self.x11_helper_surfaces.contains(&surface_id) {
                        continue;
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::UnFullscreen),
                    });
                }
                ProtocolEvent::X11WindowMinimizeRequested { surface_id } => {
                    if self.x11_helper_surfaces.contains(&surface_id) {
                        continue;
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::Minimize),
                    });
                }
                ProtocolEvent::X11WindowUnMinimizeRequested { surface_id } => {
                    if self.x11_helper_surfaces.contains(&surface_id) {
                        continue;
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::UnMinimize),
                    });
                }
                ProtocolEvent::X11WindowMoveRequested { surface_id, .. } => {
                    if self.x11_helper_surfaces.contains(&surface_id) {
                        continue;
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::BeginMove),
                    });
                }
                ProtocolEvent::X11WindowResizeRequested { surface_id, edges, .. } => {
                    if self.x11_helper_surfaces.contains(&surface_id) {
                        continue;
                    }
                    pending_window_events.push(WindowEventRequest {
                        surface_id,
                        action: WindowEvent::ManagerRequest(WindowManagerRequest::BeginResize {
                            edges,
                        }),
                    });
                }
                ProtocolEvent::X11WindowUnmapped { surface_id }
                | ProtocolEvent::X11WindowDestroyed { surface_id } => {
                    self.x11_helper_surfaces.remove(&surface_id);
                    if self.x11_popup_surfaces.remove(&surface_id) {
                        pending_popup_events
                            .push(PopupEventRequest { surface_id, action: PopupEvent::Closed });
                        continue;
                    }
                    pending_window_events
                        .push(WindowEventRequest { surface_id, action: WindowEvent::Closed });
                }
                ProtocolEvent::OutputAnnounced { output_name } => {
                    pending_output_events
                        .push(OutputEventRecord { output_name, change: "announced".to_owned() });
                }
                ProtocolEvent::ClipboardSelectionChanged { seat_name, mime_types } => {
                    let seat_id = seat_id_for_wayland_name(seat_registry, seat_name);
                    clipboard_selection.selection = if mime_types.is_empty() {
                        None
                    } else {
                        Some(ClipboardSelection {
                            seat_id,
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
                    let seat_id = seat_id_for_wayland_name(seat_registry, seat_name);
                    drag_and_drop.active_session = Some(DragAndDropSession {
                        seat_id,
                        source_surface_id,
                        icon_surface_id,
                        mime_types,
                        accepted_mime_type: None,
                        chosen_action: None,
                    });
                    drag_and_drop.last_drop = None;
                }
                ProtocolEvent::DragDropped { seat_name, target_surface_id, validated } => {
                    let seat_id = seat_id_for_wayland_name(seat_registry, seat_name);
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
                        seat_id,
                        source_surface_id,
                        target_surface_id,
                        validated,
                        mime_types,
                    });
                    drag_and_drop.active_session = None;
                }
                ProtocolEvent::DragAccepted { seat_name, mime_type } => {
                    let seat_id = seat_id_for_wayland_name(seat_registry, seat_name);
                    if let Some(session) = drag_and_drop.active_session.as_mut()
                        && session.seat_id == seat_id
                    {
                        session.accepted_mime_type = mime_type;
                    }
                }
                ProtocolEvent::DragActionSelected { seat_name, action } => {
                    let seat_id = seat_id_for_wayland_name(seat_registry, seat_name);
                    if let Some(session) = drag_and_drop.active_session.as_mut()
                        && session.seat_id == seat_id
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
                    let seat_id = seat_id_for_wayland_name(seat_registry, seat_name);
                    primary_selection.selection = if mime_types.is_empty() {
                        None
                    } else {
                        Some(PrimarySelection {
                            seat_id,
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

    /// Returns the list of Wayland globals enabled by the current protocol-state set.
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

fn x11_helper_surface(popup: bool, window_type: Option<X11WindowType>) -> bool {
    popup
        || matches!(
            window_type,
            Some(
                X11WindowType::DropdownMenu
                    | X11WindowType::Menu
                    | X11WindowType::Notification
                    | X11WindowType::PopupMenu
                    | X11WindowType::Tooltip
            )
        )
}

/// Mutable ECS sinks that receive normalized protocol output during `flush_into_ecs`.
///
/// Grouping these references keeps the protocol bridge explicit without forcing an extremely long
/// function parameter list at every call site.
#[allow(missing_docs)]
pub struct ProtocolFlushTargets<'a> {
    pub pending_xdg_requests: &'a mut PendingXdgRequests,
    pub pending_window_events: &'a mut PendingWindowEvents,
    pub pending_popup_events: &'a mut PendingPopupEvents,
    pub pending_layer_requests: &'a mut PendingLayerRequests,
    pub pending_window_controls: &'a mut PendingWindowControls,
    pub pending_output_events: &'a mut PendingOutputEvents,
    pub seat_registry: &'a mut SeatRegistry,
    pub clipboard_selection: &'a mut ClipboardSelectionState,
    pub drag_and_drop: &'a mut DragAndDropState,
    pub primary_selection: &'a mut PrimarySelectionState,
}

fn seat_id_for_wayland_name(seat_registry: &mut SeatRegistry, seat_name: String) -> SeatId {
    seat_registry.ensure_wayland_name(seat_name)
}

impl WaylandBridge for ProtocolState {
    type Event = ProtocolEvent;

    fn queue_event(&mut self, event: Self::Event) {
        self.bridge.push(event);
    }
}

/// Snapshot of the protocol globals the compositor intends to advertise.
#[derive(Debug, Clone, Default, Resource)]
pub struct ProtocolRegistry {
    /// Wayland global interface names currently enabled.
    pub globals: Vec<&'static str>,
}

/// One compositor-managed surface tracked by the protocol runtime.
#[derive(Debug, Clone)]
pub struct ProtocolSurfaceEntry {
    /// Surface role classification used by protocol/runtime code.
    pub kind: ProtocolSurfaceKind,
    /// Live Smithay handle for the surface.
    pub surface: WlSurface,
}

/// Surface classes the protocol runtime distinguishes when registering surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolSurfaceKind {
    /// A regular XDG toplevel window.
    Toplevel,
    /// An XDG popup surface.
    Popup,
    /// A layer-shell surface.
    Layer,
    /// A cursor image surface.
    Cursor,
}

/// Lookup table from compositor surface id to live Smithay surface handle.
#[derive(Debug, Clone, Default)]
pub struct ProtocolSurfaceRegistry {
    /// Entries keyed by compositor surface id.
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
