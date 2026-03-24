//! Thin protocol plugin facade.
//!
//! The heavy Smithay runtime glue lives in the sibling `plugin/*` modules; this root keeps the
//! plugin entrypoint, schedule wiring, and delegate macros that still need a shared
//! `ProtocolRuntimeState` path.

/// Bootstrap of the Smithay server runtime and calloop sources.
pub mod bootstrap;
/// Feedback helpers for frame callbacks, presentation timing, and workspace visibility.
pub mod feedback;
/// Flushes queued protocol events into typed ECS resources.
pub mod queue;
/// Seat-input dispatch and pointer/keyboard focus synchronization.
pub mod seat;
/// Clipboard and primary-selection persistence helpers.
pub mod selection;
/// Smithay protocol server state, cursor state, and callback collection.
pub mod server;
/// Surface registry and platform-surface snapshot extraction.
pub mod surface;
/// XWayland runtime dispatch and protocol-side request handling.
pub mod xwayland;

pub use server::{ProtocolCursorImage, ProtocolCursorState, ProtocolDmabufSupport};

use bevy_app::App;
use bevy_ecs::schedule::SystemSet;
use nekoland_config::resources::{CompositorConfig, KeyboardLayoutState};
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::prelude::WaylandSubApp;
use nekoland_core::schedules::PresentSchedule;
use nekoland_ecs::resources::CompositorClock;
use smithay::backend::allocator::{Buffer, Format as DmabufFormat};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::backend::renderer::{BufferType, buffer_type};
use smithay::delegate_data_device;
use smithay::delegate_dmabuf;
use smithay::delegate_foreign_toplevel_list;
use smithay::delegate_fractional_scale;
use smithay::delegate_output;
use smithay::delegate_presentation;
use smithay::delegate_primary_selection;
use smithay::delegate_seat;
use smithay::delegate_shm;
use smithay::delegate_viewporter;
use smithay::delegate_xdg_activation;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource as WaylandResource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, ClockSource, Logical, Monotonic, Point, Time};
use smithay::wayland::compositor::{
    self, BufferAssignment, CompositorState as SmithayCompositorState, SurfaceAttributes,
};
use smithay::wayland::dmabuf::{DmabufState as SmithayDmabufState, get_dmabuf};
use smithay::wayland::foreign_toplevel_list::ForeignToplevelListState as SmithayForeignToplevelListState;
use smithay::wayland::fractional_scale::{
    FractionalScaleHandler, FractionalScaleManagerState, with_fractional_scale,
};
use smithay::wayland::output::OutputManagerState as SmithayOutputManagerState;
use smithay::wayland::presentation::{PresentationState as SmithayPresentationState, Refresh};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState as SmithayDataDeviceState,
    ServerDndGrabHandler, clear_data_device_selection, request_data_device_client_selection,
    set_data_device_selection, with_source_metadata,
};
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState as SmithayPrimarySelectionState,
    clear_primary_selection, request_primary_client_selection, set_primary_selection,
};
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::XdgShellState as SmithayXdgShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState as SmithayXdgDecorationState;
use smithay::wayland::shm::ShmState as SmithayShmState;
use smithay::wayland::viewporter::ViewporterState as SmithayViewporterState;
use smithay::wayland::xdg_activation::XdgActivationState as SmithayXdgActivationState;
use smithay::wayland::xwayland_shell::XWaylandShellState as SmithayXWaylandShellState;
use smithay::xwayland::xwm::X11Surface;
use smithay::{
    delegate_compositor, delegate_layer_shell, delegate_xdg_decoration, delegate_xdg_shell,
    delegate_xwayland_shell,
};

use crate::{ProtocolEvent, ProtocolRegistry, ProtocolState};
use server::ProtocolRuntimeState;

type PresentationKind = wp_presentation_feedback::Kind;

const MONOTONIC_CLOCK_ID: u32 = Monotonic::ID as u32;
const DEFAULT_KEYBOARD_REPEAT_DELAY_MS: i32 = 200;
const DEFAULT_KEYBOARD_REPEAT_RATE: u16 = 25;
const MAX_PERSISTED_SELECTION_BYTES: usize = 1024 * 1024;
const SUPPORTED_XDG_WM_CAPABILITIES: [xdg_toplevel::WmCapabilities; 4] = [
    xdg_toplevel::WmCapabilities::Fullscreen,
    xdg_toplevel::WmCapabilities::Maximize,
    xdg_toplevel::WmCapabilities::Minimize,
    xdg_toplevel::WmCapabilities::WindowMenu,
];

/// Installs the Smithay runtime and bridges its callback-driven world into the compositor's ECS
/// schedules.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProtocolPlugin;

/// Present-phase system set that updates Smithay seat focus/hit-test state from the current frame.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtocolSeatDispatchSystems;

impl NekolandPlugin for ProtocolPlugin {
    /// Seeds protocol bootstrap config and public protocol registry state.
    fn build(&self, app: &mut App) {
        let state = ProtocolState::default();
        let registry = ProtocolRegistry { globals: state.supported_globals() };
        let repeat_rate = app
            .world()
            .get_resource::<CompositorConfig>()
            .map(|config| config.repeat_rate)
            .unwrap_or(DEFAULT_KEYBOARD_REPEAT_RATE);
        let initial_keyboard_layout = app
            .world()
            .get_resource::<KeyboardLayoutState>()
            .map(|state| state.active_layout().clone())
            .unwrap_or_default();
        let xwayland_enabled = app
            .world()
            .get_resource::<CompositorConfig>()
            .map(|config| config.xwayland.enabled)
            .unwrap_or(true);
        app.sub_app_mut(WaylandSubApp).world_mut().insert_resource(
            bootstrap::ProtocolBootstrapConfig {
                repeat_rate,
                initial_keyboard_layout,
                xwayland_enabled,
            },
        );

        app.insert_resource(registry)
            .init_resource::<CompositorClock>()
            .configure_sets(PresentSchedule, ProtocolSeatDispatchSystems);
    }
}

delegate_compositor!(ProtocolRuntimeState);
delegate_xdg_shell!(ProtocolRuntimeState);
delegate_xdg_decoration!(ProtocolRuntimeState);
delegate_foreign_toplevel_list!(ProtocolRuntimeState);
delegate_xdg_activation!(ProtocolRuntimeState);
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
mod tests;
