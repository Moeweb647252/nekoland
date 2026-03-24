use super::*;
use bevy_ecs::change_detection::DetectChanges;
use nekoland_core::bridge::WaylandBridge;
use smithay::reexports::wayland_server::Display;

#[derive(Debug, Clone, Default, PartialEq, Eq, bevy_ecs::prelude::Resource)]
pub struct ProtocolDmabufSupport {
    pub formats: Vec<smithay::backend::allocator::Format>,
    pub renderable_formats: Vec<smithay::backend::allocator::Format>,
    pub importable: bool,
}

impl ProtocolDmabufSupport {
    pub fn merge_formats(
        &mut self,
        formats: impl IntoIterator<Item = smithay::backend::allocator::Format>,
        renderable_formats: impl IntoIterator<Item = smithay::backend::allocator::Format>,
        importable: bool,
    ) {
        for format in formats {
            if !self.formats.contains(&format) {
                self.formats.push(format);
            }
        }
        for format in renderable_formats {
            if !self.renderable_formats.contains(&format) {
                self.renderable_formats.push(format);
            }
        }
        self.importable |= importable;
    }

    pub fn importable_format(&self, format: smithay::backend::allocator::Format) -> bool {
        self.formats.contains(&format)
    }

    pub fn renderable_format(&self, format: smithay::backend::allocator::Format) -> bool {
        self.renderable_formats.contains(&format)
    }
}

#[derive(Debug, Clone)]
pub enum ProtocolCursorImage {
    Hidden,
    Named(smithay::input::pointer::CursorIcon),
    Surface {
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        hotspot_x: i32,
        hotspot_y: i32,
    },
}

#[derive(Debug, Clone)]
pub struct ProtocolCursorState {
    pub image: ProtocolCursorImage,
}

impl Default for ProtocolCursorState {
    fn default() -> Self {
        Self { image: ProtocolCursorImage::Named(smithay::input::pointer::CursorIcon::Default) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignToplevelSnapshot {
    pub(crate) surface_id: u64,
    pub(crate) title: String,
    pub(crate) app_id: String,
}

#[derive(Debug, Clone, Default, bevy_ecs::prelude::Resource)]
pub(crate) struct ForeignToplevelSnapshotState {
    pub windows: Vec<ForeignToplevelSnapshot>,
}

#[derive(Debug, Default)]
pub(crate) struct ProtocolClientState {
    pub(crate) compositor_state: smithay::wayland::compositor::CompositorClientState,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SyntheticPointerGrab {
    pub(crate) serial: u32,
    pub(crate) surface_id: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct SmithayProtocolServer {
    pub(crate) runtime: Option<SharedProtocolRuntime>,
}

pub(crate) type SharedProtocolRuntime = std::rc::Rc<std::cell::RefCell<SmithayProtocolRuntime>>;

#[derive(Debug)]
pub(crate) struct SmithayProtocolRuntime {
    pub(crate) display: smithay::reexports::wayland_server::Display<ProtocolRuntimeState>,
    pub(crate) state: ProtocolRuntimeState,
    pub(crate) xwayland_event_loop: Option<calloop::EventLoop<'static, ProtocolRuntimeState>>,
    pub(crate) socket: Option<smithay::reexports::wayland_server::ListeningSocket>,
    pub(crate) clients: Vec<smithay::reexports::wayland_server::Client>,
    pub(crate) last_accept_error: Option<String>,
    pub(crate) last_dispatch_error: Option<String>,
    pub(crate) last_xwayland_error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ProtocolRuntimeState {
    pub(crate) compositor_state: smithay::wayland::compositor::CompositorState,
    pub(crate) xdg_shell_state: smithay::wayland::shell::xdg::XdgShellState,
    pub(crate) _xdg_decoration_state: smithay::wayland::shell::xdg::decoration::XdgDecorationState,
    pub(crate) _foreign_toplevel_list_state:
        smithay::wayland::foreign_toplevel_list::ForeignToplevelListState,
    pub(crate) _xdg_activation_state: smithay::wayland::xdg_activation::XdgActivationState,
    pub(crate) xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    pub(crate) layer_shell_state: smithay::wayland::shell::wlr_layer::WlrLayerShellState,
    pub(crate) data_device_state: smithay::wayland::selection::data_device::DataDeviceState,
    pub(crate) _primary_selection_state:
        smithay::wayland::selection::primary_selection::PrimarySelectionState,
    pub(crate) dmabuf_state: smithay::wayland::dmabuf::DmabufState,
    pub(crate) _dmabuf_global: smithay::wayland::dmabuf::DmabufGlobal,
    pub(crate) dmabuf_support: ProtocolDmabufSupport,
    pub(crate) _viewporter_state: smithay::wayland::viewporter::ViewporterState,
    pub(crate) _fractional_scale_state:
        smithay::wayland::fractional_scale::FractionalScaleManagerState,
    pub(crate) shm_state: smithay::wayland::shm::ShmState,
    pub(crate) _presentation_state: smithay::wayland::presentation::PresentationState,
    pub(crate) _output_manager_state: smithay::wayland::output::OutputManagerState,
    pub(crate) seat_state: smithay::input::SeatState<Self>,
    pub(crate) seat: smithay::input::Seat<Self>,
    pub(crate) primary_output: smithay::output::Output,
    pub(crate) popup_manager: smithay::desktop::PopupManager,
    pub(crate) foreign_toplevels: std::collections::HashMap<
        u64,
        smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle,
    >,
    pub(crate) toplevels:
        std::collections::HashMap<u64, smithay::wayland::shell::xdg::ToplevelSurface>,
    pub(crate) popups: std::collections::HashMap<u64, smithay::wayland::shell::xdg::PopupSurface>,
    pub(crate) layers:
        std::collections::HashMap<u64, smithay::wayland::shell::wlr_layer::LayerSurface>,
    pub(crate) xwms:
        std::collections::HashMap<smithay::xwayland::xwm::XwmId, smithay::xwayland::xwm::X11Wm>,
    pub(crate) x11_windows: std::collections::HashMap<u32, smithay::xwayland::xwm::X11Surface>,
    pub(crate) x11_surface_ids_by_window: std::collections::HashMap<u32, u64>,
    pub(crate) x11_window_ids_by_surface: std::collections::HashMap<u64, u32>,
    pub(crate) mapped_x11_windows: std::collections::BTreeSet<u32>,
    pub(crate) published_x11_windows: std::collections::BTreeSet<u32>,
    pub(crate) xwayland_client: Option<smithay::reexports::wayland_server::Client>,
    pub(crate) _xwm_connection: Option<std::os::unix::net::UnixStream>,
    pub(crate) mapped_primary_output_name: String,
    pub(crate) bound_output_names: std::collections::HashMap<String, String>,
    pub(crate) event_queue: std::collections::VecDeque<crate::ProtocolEvent>,
    pub(crate) next_surface_id: u64,
    pub(crate) presentation_sequence: u64,
    pub(crate) synthetic_pointer_grab: Option<SyntheticPointerGrab>,
    pub(crate) selection_persistence: super::selection::SelectionPersistenceState,
    pub(crate) xwayland_state: super::xwayland::XWaylandRuntimeState,
    pub(crate) cursor_state: ProtocolCursorState,
}

pub(crate) fn sync_protocol_server_state_system(
    server: Option<bevy_ecs::prelude::NonSendMut<'_, SmithayProtocolServer>>,
    mut server_state: bevy_ecs::prelude::ResMut<'_, nekoland_ecs::resources::ProtocolServerState>,
) {
    let Some(server) = server else {
        return;
    };
    let Some(runtime) = server.runtime.as_ref() else {
        return;
    };

    runtime.borrow().sync_server_state(&mut server_state);
}

pub(crate) fn sync_protocol_dmabuf_support_system(
    support: Option<bevy_ecs::prelude::Res<'_, ProtocolDmabufSupport>>,
    server: Option<bevy_ecs::prelude::NonSendMut<'_, SmithayProtocolServer>>,
) {
    let (Some(support), Some(mut server)) = (support, server) else {
        return;
    };
    if !support.is_changed() {
        return;
    }

    server.sync_dmabuf_support(&support);
}

pub(crate) fn sync_protocol_cursor_state_system(
    server: Option<bevy_ecs::prelude::NonSendMut<'_, SmithayProtocolServer>>,
    cursor_state: Option<bevy_ecs::prelude::NonSendMut<'_, ProtocolCursorState>>,
    mut cursor_image_snapshot: bevy_ecs::prelude::ResMut<
        '_,
        nekoland_ecs::resources::CursorImageSnapshot,
    >,
) {
    let (Some(mut server), Some(mut cursor_state)) = (server, cursor_state) else {
        return;
    };
    server.sync_cursor_state(&mut cursor_state);
    server.sync_cursor_image_snapshot(&cursor_state, &mut cursor_image_snapshot);
}

pub(crate) fn collect_smithay_callbacks_system(
    mut protocol_state: bevy_ecs::prelude::ResMut<'_, crate::ProtocolState>,
    server: Option<bevy_ecs::prelude::NonSendMut<'_, SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    for event in server.drain_events() {
        protocol_state.queue_event(event);
    }
}

pub(crate) fn sync_protocol_output_timing_system(
    output_snapshots: Option<
        bevy_ecs::prelude::Res<'_, nekoland_ecs::resources::OutputSnapshotState>,
    >,
    mut last_output_timing: bevy_ecs::prelude::Local<Option<super::feedback::OutputTiming>>,
    server: Option<bevy_ecs::prelude::NonSendMut<'_, SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    if let Some(output_timing) = super::feedback::current_output_timing(output_snapshots.as_deref())
        && last_output_timing.as_ref() != Some(&output_timing)
    {
        server.sync_output_timing(output_timing.clone());
        *last_output_timing = Some(output_timing);
    }
}

pub(crate) fn sync_keyboard_repeat_config_system(
    server: Option<bevy_ecs::prelude::NonSendMut<'_, SmithayProtocolServer>>,
    config: Option<bevy_ecs::prelude::Res<'_, nekoland_config::resources::CompositorConfig>>,
    mut last_repeat_rate: bevy_ecs::prelude::Local<Option<u16>>,
) {
    let (Some(server), Some(config)) = (server, config) else {
        return;
    };
    if *last_repeat_rate == Some(config.repeat_rate) {
        return;
    }

    let Some(runtime) = server.runtime.as_ref() else {
        return;
    };

    runtime.borrow_mut().sync_keyboard_repeat_info(config.repeat_rate);
    *last_repeat_rate = Some(config.repeat_rate);
}

pub(crate) fn sync_keyboard_layout_config_system(
    server: Option<bevy_ecs::prelude::NonSendMut<'_, SmithayProtocolServer>>,
    keyboard_layout_state: Option<
        bevy_ecs::prelude::Res<'_, nekoland_config::resources::KeyboardLayoutState>,
    >,
    mut last_layout: bevy_ecs::prelude::Local<
        Option<nekoland_config::resources::ConfiguredKeyboardLayout>,
    >,
) {
    let (Some(server), Some(keyboard_layout_state)) = (server, keyboard_layout_state) else {
        return;
    };
    let active_layout = keyboard_layout_state.active_layout().clone();
    if last_layout.as_ref() == Some(&active_layout) {
        return;
    }

    let Some(runtime) = server.runtime.as_ref() else {
        return;
    };

    if runtime.borrow_mut().sync_keyboard_layout(&active_layout) {
        *last_layout = Some(active_layout);
    }
}

pub(crate) fn sync_foreign_toplevel_list_system(
    snapshots: bevy_ecs::prelude::Res<'_, ForeignToplevelSnapshotState>,
    server: Option<bevy_ecs::prelude::NonSendMut<'_, SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };

    server.sync_foreign_toplevel_list(&snapshots.windows);
}

pub(crate) fn xkb_config_for_layout(
    keyboard_layout: &nekoland_config::resources::ConfiguredKeyboardLayout,
) -> smithay::input::keyboard::XkbConfig<'_> {
    smithay::input::keyboard::XkbConfig {
        rules: keyboard_layout.rules.as_str(),
        model: keyboard_layout.model.as_str(),
        layout: keyboard_layout.layout.as_str(),
        variant: keyboard_layout.variant.as_str(),
        options: (!keyboard_layout.options.is_empty()).then(|| keyboard_layout.options.clone()),
    }
}

impl SmithayProtocolServer {
    pub(crate) fn new(
        repeat_rate: u16,
        initial_keyboard_layout: nekoland_config::resources::ConfiguredKeyboardLayout,
        xwayland_enabled: bool,
    ) -> (Self, nekoland_ecs::resources::ProtocolServerState) {
        let mut server_state = nekoland_ecs::resources::ProtocolServerState::default();

        let runtime = match Display::new() {
            Ok(display) => {
                let display_handle = display.handle();
                let state = ProtocolRuntimeState::new(
                    &display_handle,
                    repeat_rate,
                    &initial_keyboard_layout,
                );
                let socket = match super::bootstrap::bind_wayland_socket() {
                    Ok((socket, socket_name)) => {
                        let socket_name = socket_name.to_string_lossy().into_owned();
                        tracing::info!(socket = %socket_name, "Wayland display socket ready");
                        server_state.socket_name = Some(socket_name);
                        server_state.runtime_dir = super::bootstrap::current_wayland_runtime_dir();
                        Some(socket)
                    }
                    Err(error) => {
                        let error = error.to_string();
                        tracing::warn!(error = %error, "failed to create Wayland display socket");
                        server_state.startup_error = Some(error);
                        None
                    }
                };

                let runtime = std::rc::Rc::new(std::cell::RefCell::new(SmithayProtocolRuntime {
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

    pub(crate) fn drain_events(&mut self) -> Vec<crate::ProtocolEvent> {
        self.runtime.as_ref().map(|runtime| runtime.borrow_mut().drain_events()).unwrap_or_default()
    }

    pub(crate) fn sync_surface_registry(&mut self, registry: &mut crate::ProtocolSurfaceRegistry) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().state.sync_surface_registry(registry);
        } else {
            registry.surfaces.clear();
        }
    }

    pub(crate) fn sync_cursor_state(&self, cursor_state: &mut ProtocolCursorState) {
        if let Some(runtime) = self.runtime.as_ref() {
            *cursor_state = runtime.borrow().state.cursor_state.clone();
        } else {
            *cursor_state = ProtocolCursorState::default();
        }
    }

    pub(crate) fn sync_cursor_image_snapshot(
        &mut self,
        cursor_state: &ProtocolCursorState,
        cursor_image_snapshot: &mut nekoland_ecs::resources::CursorImageSnapshot,
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            *cursor_image_snapshot = runtime.borrow_mut().state.cursor_image_snapshot(cursor_state);
        } else {
            *cursor_image_snapshot = nekoland_ecs::resources::CursorImageSnapshot::default();
        }
    }

    pub(crate) fn sync_dmabuf_support(&mut self, support: &ProtocolDmabufSupport) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };

        runtime.borrow_mut().sync_dmabuf_support(support);
    }

    pub(crate) fn sync_xwayland_state(
        &self,
        state: &mut nekoland_ecs::resources::XWaylandServerState,
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow().sync_xwayland_state(state);
        } else {
            *state = nekoland_ecs::resources::XWaylandServerState::default();
        }
    }

    pub(crate) fn process_selection_persistence(&mut self) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().process_selection_persistence();
        }
    }

    pub(crate) fn dispatch_xwayland(&mut self) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_xwayland();
        }
    }

    pub(crate) fn send_close(&mut self, surface_id: u64) -> bool {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.borrow_mut().send_close(surface_id))
            .unwrap_or(false)
    }

    pub(crate) fn pointer_focus_candidate_accepts(
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

    pub(crate) fn sync_xdg_toplevel_state(
        &mut self,
        surface_id: u64,
        size: Option<nekoland_ecs::resources::SurfaceExtent>,
        fullscreen: bool,
        maximized: bool,
        resizing: bool,
    ) -> bool {
        self.runtime
            .as_ref()
            .map(|runtime| {
                runtime
                    .borrow_mut()
                    .sync_xdg_toplevel_state(surface_id, size, fullscreen, maximized, resizing)
            })
            .unwrap_or(false)
    }

    pub(crate) fn sync_x11_window_presentation(
        &mut self,
        surface_id: u64,
        geometry: nekoland_ecs::resources::X11WindowGeometry,
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

    pub(crate) fn dismiss_popup(&mut self, surface_id: u64) -> bool {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.borrow_mut().dismiss_popup(surface_id))
            .unwrap_or(false)
    }

    pub(crate) fn sync_keyboard_focus(&mut self, surface_id: Option<u64>) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().sync_keyboard_focus(surface_id);
        }
    }

    pub(crate) fn sync_foreign_toplevel_list(&mut self, windows: &[ForeignToplevelSnapshot]) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().sync_foreign_toplevel_list(windows);
        }
    }

    pub(crate) fn dispatch_keyboard_input(&mut self, keycode: u32, pressed: bool, time: u32) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_keyboard_input(keycode, pressed, time);
        }
    }

    pub(crate) fn dispatch_pointer_motion(
        &mut self,
        focus: Option<super::seat::PointerSurfaceFocus>,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_pointer_motion(focus, location, time);
        }
    }

    pub(crate) fn dispatch_pointer_button(
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

    pub(crate) fn dispatch_pointer_axis(&mut self, horizontal: f64, vertical: f64, time: u32) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().dispatch_pointer_axis(horizontal, vertical, time);
        }
    }

    pub(crate) fn sync_workspace_visibility(
        &mut self,
        activated_toplevels: &[u64],
        dismissed_popups: &[u64],
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().sync_workspace_visibility(activated_toplevels, dismissed_popups);
        }
    }

    pub(crate) fn send_frame_callbacks(
        &mut self,
        surface_ids: &[u64],
        frame_time: Time<Monotonic>,
    ) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().send_frame_callbacks(surface_ids, frame_time);
        }
    }

    pub(crate) fn send_presentation_feedback(
        &mut self,
        surface_ids: &[u64],
        frame_time: Time<Monotonic>,
        refresh: smithay::wayland::presentation::Refresh,
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

    pub(crate) fn sync_output_timing(&mut self, output_timing: super::feedback::OutputTiming) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.borrow_mut().sync_output_timing(output_timing);
        }
    }
}

impl ProtocolRuntimeState {
    pub(crate) fn new(
        display_handle: &smithay::reexports::wayland_server::DisplayHandle,
        repeat_rate: u16,
        initial_keyboard_layout: &nekoland_config::resources::ConfiguredKeyboardLayout,
    ) -> Self {
        let compositor_state = super::SmithayCompositorState::new::<Self>(display_handle);
        let xdg_shell_state = super::SmithayXdgShellState::new_with_capabilities::<Self>(
            display_handle,
            super::SUPPORTED_XDG_WM_CAPABILITIES,
        );
        let xdg_decoration_state = super::SmithayXdgDecorationState::new::<Self>(display_handle);
        let foreign_toplevel_list_state =
            super::SmithayForeignToplevelListState::new::<Self>(display_handle);
        let xdg_activation_state = super::SmithayXdgActivationState::new::<Self>(display_handle);
        let xwayland_shell_state = super::SmithayXWaylandShellState::new::<Self>(display_handle);
        let layer_shell_state = super::WlrLayerShellState::new::<Self>(display_handle);
        let data_device_state = super::SmithayDataDeviceState::new::<Self>(display_handle);
        let primary_selection_state =
            super::SmithayPrimarySelectionState::new::<Self>(display_handle);
        let mut dmabuf_state = super::SmithayDmabufState::new();
        let dmabuf_global =
            dmabuf_state.create_global::<Self>(display_handle, Vec::<super::DmabufFormat>::new());
        let viewporter_state = super::SmithayViewporterState::new::<Self>(display_handle);
        let fractional_scale_state =
            super::FractionalScaleManagerState::new::<Self>(display_handle);
        let shm_state = super::SmithayShmState::new::<Self>(
            display_handle,
            vec![
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Argb8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Xrgb8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Rgb565,
            ],
        );
        let presentation_state =
            super::SmithayPresentationState::new::<Self>(display_handle, super::MONOTONIC_CLOCK_ID);
        let output_manager_state =
            super::SmithayOutputManagerState::new_with_xdg_output::<Self>(display_handle);
        let mut seat_state = super::SeatState::new();
        let mut seat = seat_state.new_wl_seat(display_handle, "seat-0");
        seat.add_pointer();
        let _ = seat.add_keyboard(
            xkb_config_for_layout(initial_keyboard_layout),
            super::DEFAULT_KEYBOARD_REPEAT_DELAY_MS,
            i32::from(repeat_rate),
        );

        let primary_output = smithay::output::Output::new(
            "Nekoland-1".into(),
            smithay::output::PhysicalProperties {
                size: (344, 194).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "Nekoland".into(),
                model: "Virtual Output".into(),
            },
        );
        primary_output.create_global::<Self>(display_handle);
        let mode = smithay::output::Mode { size: (1280, 720).into(), refresh: 60_000 };
        primary_output.change_current_state(
            Some(mode),
            Some(smithay::utils::Transform::Normal),
            Some(smithay::output::Scale::Integer(1)),
            Some((0, 0).into()),
        );
        primary_output.set_preferred(mode);

        let mut state = Self {
            compositor_state,
            xdg_shell_state,
            _xdg_decoration_state: xdg_decoration_state,
            _foreign_toplevel_list_state: foreign_toplevel_list_state,
            _xdg_activation_state: xdg_activation_state,
            xwayland_shell_state,
            layer_shell_state,
            data_device_state,
            _primary_selection_state: primary_selection_state,
            dmabuf_state,
            _dmabuf_global: dmabuf_global,
            dmabuf_support: ProtocolDmabufSupport::default(),
            _viewporter_state: viewporter_state,
            _fractional_scale_state: fractional_scale_state,
            shm_state,
            _presentation_state: presentation_state,
            _output_manager_state: output_manager_state,
            seat_state,
            seat,
            primary_output: primary_output.clone(),
            popup_manager: smithay::desktop::PopupManager::default(),
            foreign_toplevels: std::collections::HashMap::new(),
            toplevels: std::collections::HashMap::new(),
            popups: std::collections::HashMap::new(),
            layers: std::collections::HashMap::new(),
            xwms: std::collections::HashMap::new(),
            x11_windows: std::collections::HashMap::new(),
            x11_surface_ids_by_window: std::collections::HashMap::new(),
            x11_window_ids_by_surface: std::collections::HashMap::new(),
            mapped_x11_windows: std::collections::BTreeSet::new(),
            published_x11_windows: std::collections::BTreeSet::new(),
            xwayland_client: None,
            _xwm_connection: None,
            mapped_primary_output_name: primary_output.name(),
            bound_output_names: std::collections::HashMap::new(),
            event_queue: std::collections::VecDeque::new(),
            next_surface_id: 1,
            presentation_sequence: 0,
            synthetic_pointer_grab: None,
            selection_persistence: super::selection::SelectionPersistenceState::default(),
            xwayland_state: super::xwayland::XWaylandRuntimeState::default(),
            cursor_state: ProtocolCursorState::default(),
        };

        state.queue_event(crate::ProtocolEvent::OutputAnnounced {
            output_name: primary_output.name(),
        });

        state
    }

    pub(crate) fn sync_dmabuf_support(
        &mut self,
        display_handle: &smithay::reexports::wayland_server::DisplayHandle,
        support: &ProtocolDmabufSupport,
    ) {
        if &self.dmabuf_support == support {
            return;
        }

        self.dmabuf_state.disable_global::<Self>(display_handle, &self._dmabuf_global);
        self.dmabuf_state.destroy_global::<Self>(display_handle, self._dmabuf_global);
        self._dmabuf_global =
            self.dmabuf_state.create_global::<Self>(display_handle, support.formats.clone());
        self.dmabuf_support = support.clone();
    }

    pub(crate) fn surface_id(&mut self, surface: &super::WlSurface) -> u64 {
        super::surface::surface_identity(surface, &mut self.next_surface_id)
    }

    pub(crate) fn known_surface_id(&self, surface: &super::WlSurface) -> Option<u64> {
        super::compositor::with_states(surface, |states| {
            states.data_map.get::<super::surface::SurfaceIdentity>().map(|identity| identity.0)
        })
    }

    pub(crate) fn cursor_image_snapshot(
        &mut self,
        cursor_state: &ProtocolCursorState,
    ) -> nekoland_ecs::resources::CursorImageSnapshot {
        match &cursor_state.image {
            ProtocolCursorImage::Hidden => nekoland_ecs::resources::CursorImageSnapshot::Hidden,
            ProtocolCursorImage::Named(icon) => {
                nekoland_ecs::resources::CursorImageSnapshot::Named {
                    icon_name: icon.name().to_owned(),
                }
            }
            ProtocolCursorImage::Surface { surface, hotspot_x, hotspot_y } => {
                let surface_id = self.surface_id(surface);
                let size = super::surface::committed_surface_extent(surface)
                    .unwrap_or(nekoland_ecs::resources::SurfaceExtent { width: 1, height: 1 });
                nekoland_ecs::resources::CursorImageSnapshot::Surface {
                    surface_id,
                    hotspot_x: *hotspot_x,
                    hotspot_y: *hotspot_y,
                    width: size.width,
                    height: size.height,
                }
            }
        }
    }

    pub(crate) fn validate_interactive_request(
        &mut self,
        wl_seat: &super::WlSeat,
        serial: smithay::utils::Serial,
        expected_focus_surface_id: u64,
        kind: super::surface::InteractiveRequestKind,
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

        let raw_serial = u32::from(serial);

        if !pointer.has_grab(serial) {
            if synthetic_pointer_grab_matches(
                self.synthetic_pointer_grab,
                raw_serial,
                expected_focus_surface_id,
                kind,
            ) {
                return true;
            }
            tracing::warn!(
                request = kind.as_str(),
                seat_name = seat.name(),
                seat_resource = %seat_name(wl_seat),
                expected_focus_surface_id,
                serial = raw_serial,
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
                serial = raw_serial,
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
                serial = raw_serial,
                "rejecting interactive xdg request because the implicit grab did not start on a surface"
            );
            return false;
        };

        let focused_surface_id = self.surface_id(&focused_surface);
        if focused_surface_id != expected_focus_surface_id {
            if synthetic_pointer_grab_matches(
                self.synthetic_pointer_grab,
                raw_serial,
                expected_focus_surface_id,
                kind,
            ) {
                return true;
            }
            tracing::warn!(
                request = kind.as_str(),
                seat_name = seat.name(),
                seat_resource = %seat_name(wl_seat),
                expected_focus_surface_id,
                focused_surface_id,
                serial = raw_serial,
                "rejecting interactive xdg request because the implicit grab belongs to a different surface"
            );
            return false;
        }

        true
    }

    pub(crate) fn queue_event(&mut self, event: crate::ProtocolEvent) {
        self.event_queue.push_back(event);
    }

    pub(crate) fn sync_x11_surface_mapping(
        &mut self,
        window: &smithay::xwayland::xwm::X11Surface,
    ) -> Option<u64> {
        let surface = window.wl_surface()?;
        let surface_id = self.surface_id(&surface);
        let window_id = window.window_id();
        self.x11_surface_ids_by_window.insert(window_id, surface_id);
        self.x11_window_ids_by_surface.insert(surface_id, window_id);
        self.update_surface_fractional_scale(&surface);
        Some(surface_id)
    }

    pub(crate) fn queue_toplevel_metadata_changed(
        &mut self,
        surface: &smithay::wayland::shell::xdg::ToplevelSurface,
    ) {
        let surface_id = self.surface_id(surface.wl_surface());
        let (title, app_id) = super::compositor::with_states(surface.wl_surface(), |states| {
            let Some(attributes) =
                states.data_map.get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
            else {
                return (None, None);
            };
            let Ok(attributes) = attributes.lock() else {
                tracing::warn!("failed to lock XDG toplevel attributes");
                return (None, None);
            };
            (attributes.title.clone(), attributes.app_id.clone())
        });

        self.queue_event(crate::ProtocolEvent::ToplevelMetadataChanged {
            surface_id,
            title,
            app_id,
        });
    }
}

impl SmithayProtocolRuntime {
    pub(crate) fn initialize_xwayland(&mut self, enabled: bool) {
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

        let (xwayland, client) = match smithay::xwayland::XWayland::spawn(
            &self.display.handle(),
            None,
            std::iter::empty::<(&str, &str)>(),
            true,
            std::process::Stdio::null(),
            std::process::Stdio::null(),
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

    pub(crate) fn on_socket_ready(&mut self) {
        self.accept_pending_clients();
        self.dispatch_clients();
    }

    pub(crate) fn on_display_ready(&mut self) {
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
                    match handle
                        .insert_client(stream, std::sync::Arc::new(ProtocolClientState::default()))
                    {
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

    pub(crate) fn dispatch_clients(&mut self) {
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

    pub(crate) fn dispatch_xwayland(&mut self) {
        let Some(event_loop) = self.xwayland_event_loop.as_mut() else {
            return;
        };

        match event_loop.dispatch(std::time::Duration::ZERO, &mut self.state) {
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

    pub(crate) fn drain_events(&mut self) -> Vec<crate::ProtocolEvent> {
        self.state.event_queue.drain(..).collect()
    }

    pub(crate) fn send_close(&mut self, surface_id: u64) -> bool {
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

        if handled && let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after sending close",
            );
        }

        handled
    }

    pub(crate) fn sync_xdg_toplevel_state(
        &mut self,
        surface_id: u64,
        size: Option<nekoland_ecs::resources::SurfaceExtent>,
        fullscreen: bool,
        maximized: bool,
        resizing: bool,
    ) -> bool {
        let handled = if let Some(toplevel) = self.state.toplevels.get(&surface_id).cloned() {
            toplevel.with_pending_state(|state| {
                state.size = size.map(|size| {
                    smithay::utils::Size::<i32, smithay::utils::Logical>::from((
                        size.width.max(1) as i32,
                        size.height.max(1) as i32,
                    ))
                });
                if fullscreen {
                    state.states.set(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen,
                    );
                } else {
                    state.states.unset(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen,
                    );
                }
                if maximized {
                    state.states.set(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                    );
                } else {
                    state.states.unset(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized,
                    );
                }
                if resizing {
                    state.states.set(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Resizing,
                    );
                } else {
                    state.states.unset(
                        smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Resizing,
                    );
                }
            });
            toplevel.send_configure();
            true
        } else {
            false
        };

        if handled && let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after syncing XDG toplevel state",
            );
        }

        handled
    }

    pub(crate) fn sync_x11_window_presentation(
        &mut self,
        surface_id: u64,
        geometry: nekoland_ecs::resources::X11WindowGeometry,
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
                let rect = smithay::utils::Rectangle::new(
                    smithay::utils::Point::from((geometry.x, geometry.y)),
                    smithay::utils::Size::from((
                        geometry.width.max(1) as i32,
                        geometry.height.max(1) as i32,
                    )),
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

        if handled && let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after syncing X11 window presentation",
            );
        }

        handled
    }

    pub(crate) fn dismiss_popup(&mut self, surface_id: u64) -> bool {
        let Some(popup) = self.state.popups.get(&surface_id).cloned() else {
            return false;
        };

        let dismissed = self.dismiss_popup_surface(surface_id, &popup);
        if dismissed {
            self.state.queue_event(crate::ProtocolEvent::SurfaceDestroyed {
                surface_id,
                role: nekoland_ecs::resources::XdgSurfaceRole::Popup,
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

    pub(crate) fn sync_keyboard_focus(&mut self, surface_id: Option<u64>) {
        let focus = surface_id.and_then(|surface_id| self.surface_for_id(surface_id));
        let seat = self.state.seat.clone();
        let client = focus.as_ref().and_then(smithay::reexports::wayland_server::Resource::client);
        smithay::wayland::selection::data_device::set_data_device_focus::<ProtocolRuntimeState>(
            &self.display.handle(),
            &seat,
            client.clone(),
        );
        smithay::wayland::selection::primary_selection::set_primary_focus::<ProtocolRuntimeState>(
            &self.display.handle(),
            &seat,
            client,
        );
        let Some(keyboard) = seat.get_keyboard() else {
            return;
        };

        keyboard.set_focus(&mut self.state, focus, smithay::utils::SERIAL_COUNTER.next_serial());
        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after syncing keyboard focus",
            );
        }
    }

    pub(crate) fn dispatch_keyboard_input(&mut self, keycode: u32, pressed: bool, time: u32) {
        let seat = self.state.seat.clone();
        let Some(keyboard) = seat.get_keyboard() else {
            return;
        };

        keyboard.input::<(), _>(
            &mut self.state,
            keycode.into(),
            if pressed {
                smithay::backend::input::KeyState::Pressed
            } else {
                smithay::backend::input::KeyState::Released
            },
            smithay::utils::SERIAL_COUNTER.next_serial(),
            time,
            |_, _, _| smithay::input::keyboard::FilterResult::Forward,
        );

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after dispatching keyboard input",
            );
        }
    }

    pub(crate) fn dispatch_pointer_motion(
        &mut self,
        focus: Option<super::seat::PointerSurfaceFocus>,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        let seat = self.state.seat.clone();
        let Some(pointer) = seat.get_pointer() else {
            return;
        };

        let focus = focus.and_then(|focus| {
            let root_surface = self.surface_for_id(focus.surface_id)?;
            smithay::desktop::utils::under_from_surface_tree(
                &root_surface,
                location,
                focus.surface_origin.to_i32_round(),
                smithay::desktop::WindowSurfaceType::ALL,
            )
            .map(|(surface, origin)| (surface, origin.to_f64()))
        });
        pointer.motion(
            &mut self.state,
            focus,
            &smithay::input::pointer::MotionEvent {
                location,
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time,
            },
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

    pub(crate) fn dispatch_pointer_button(
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
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if pressed {
            self.state.synthetic_pointer_grab = focus_surface_id
                .map(|surface_id| SyntheticPointerGrab { serial: u32::from(serial), surface_id });
        }

        pointer.button(
            &mut self.state,
            &smithay::input::pointer::ButtonEvent {
                serial,
                time,
                button: button_code,
                state: if pressed {
                    smithay::backend::input::ButtonState::Pressed
                } else {
                    smithay::backend::input::ButtonState::Released
                },
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

    pub(crate) fn dispatch_pointer_axis(&mut self, horizontal: f64, vertical: f64, time: u32) {
        let seat = self.state.seat.clone();
        let Some(pointer) = seat.get_pointer() else {
            return;
        };

        let mut axis = smithay::input::pointer::AxisFrame::new(time)
            .source(smithay::backend::input::AxisSource::Continuous);
        if horizontal != 0.0 {
            axis = axis.value(smithay::backend::input::Axis::Horizontal, horizontal);
        }
        if vertical != 0.0 {
            axis = axis.value(smithay::backend::input::Axis::Vertical, vertical);
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

    pub(crate) fn sync_workspace_visibility(
        &mut self,
        activated_toplevels: &[u64],
        dismissed_popups: &[u64],
    ) {
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

        if sent_protocol_update && let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after syncing workspace visibility",
            );
        }
    }

    pub(crate) fn sync_keyboard_layout(
        &mut self,
        keyboard_layout: &nekoland_config::resources::ConfiguredKeyboardLayout,
    ) -> bool {
        let seat = self.state.seat.clone();
        let Some(keyboard) = seat.get_keyboard() else {
            return false;
        };

        if let Err(error) =
            keyboard.set_xkb_config(&mut self.state, xkb_config_for_layout(keyboard_layout))
        {
            tracing::warn!(
                layout = %keyboard_layout.name,
                error = %error,
                "failed to apply keyboard layout to Smithay seat"
            );
            return false;
        }

        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after updating keyboard layout",
            );
        }

        true
    }

    pub(crate) fn sync_foreign_toplevel_list(&mut self, windows: &[ForeignToplevelSnapshot]) {
        let current_ids = windows
            .iter()
            .map(|window| window.surface_id)
            .collect::<std::collections::HashSet<_>>();
        let mut changed = false;

        for window in windows {
            let handle = self
                .state
                .foreign_toplevels
                .entry(window.surface_id)
                .or_insert_with(|| {
                    changed = true;
                    self.state._foreign_toplevel_list_state.new_toplevel::<ProtocolRuntimeState>(
                        window.title.clone(),
                        window.app_id.clone(),
                    )
                })
                .clone();

            let mut updated = false;
            if handle.title() != window.title {
                handle.send_title(&window.title);
                updated = true;
            }
            if handle.app_id() != window.app_id {
                handle.send_app_id(&window.app_id);
                updated = true;
            }
            if updated {
                handle.send_done();
                changed = true;
            }
        }

        let removed_ids = self
            .state
            .foreign_toplevels
            .keys()
            .copied()
            .filter(|surface_id| !current_ids.contains(surface_id))
            .collect::<Vec<_>>();
        for surface_id in removed_ids {
            if let Some(handle) = self.state.foreign_toplevels.remove(&surface_id) {
                self.state._foreign_toplevel_list_state.remove_toplevel(&handle);
                changed = true;
            }
        }
        self.state._foreign_toplevel_list_state.cleanup_closed_handles();

        if changed && let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after syncing foreign toplevel list",
            );
        }
    }

    pub(crate) fn sync_keyboard_repeat_info(&mut self, repeat_rate: u16) {
        let seat = self.state.seat.clone();
        let Some(keyboard) = seat.get_keyboard() else {
            return;
        };

        keyboard.change_repeat_info(repeat_rate as i32, super::DEFAULT_KEYBOARD_REPEAT_DELAY_MS);
        if let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after updating keyboard repeat info",
            );
        }
    }

    pub(crate) fn send_frame_callbacks(
        &mut self,
        surface_ids: &[u64],
        frame_time: Time<Monotonic>,
    ) {
        let mut sent_callbacks = false;

        for surface_id in surface_ids {
            let Some(surface) = self.surface_for_id(*surface_id) else {
                continue;
            };

            let output = self.state.primary_output.clone();
            smithay::desktop::utils::send_frames_surface_tree(
                &surface,
                &output,
                frame_time,
                None,
                |_, _| Some(output.clone()),
            );
            sent_callbacks = true;
        }

        if sent_callbacks && let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after sending frame callbacks",
            );
        }
    }

    pub(crate) fn send_presentation_feedback(
        &mut self,
        surface_ids: &[u64],
        frame_time: Time<Monotonic>,
        refresh: smithay::wayland::presentation::Refresh,
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
                super::MONOTONIC_CLOCK_ID,
                frame_time,
                refresh,
                sequence,
                super::PresentationKind::Vsync,
            );
            sent_feedback = true;
        }

        if sent_feedback && let Err(error) = self.display.flush_clients() {
            remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after sending presentation feedback",
            );
        }
    }

    pub(crate) fn sync_output_timing(&mut self, output_timing: super::feedback::OutputTiming) {
        let mode = smithay::output::Mode {
            size: (output_timing.width as i32, output_timing.height as i32).into(),
            refresh: output_timing.refresh_millihz as i32,
        };
        self.state.mapped_primary_output_name = output_timing.output_name.clone();

        self.state.primary_output.change_current_state(
            Some(mode),
            Some(smithay::utils::Transform::Normal),
            Some(smithay::output::Scale::Integer(output_timing.scale.max(1) as i32)),
            Some((0, 0).into()),
        );
        self.state.primary_output.set_preferred(mode);
        self.state.update_all_fractional_scales();
    }

    fn surface_for_id(
        &self,
        surface_id: u64,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
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
                    .and_then(smithay::xwayland::xwm::X11Surface::wl_surface)
            })
    }

    fn presentation_feedback_for_id(
        &self,
        surface_id: u64,
    ) -> Option<smithay::desktop::utils::SurfacePresentationFeedback> {
        let surface = self.surface_for_id(surface_id)?;
        smithay::wayland::compositor::with_states(&surface, |states| {
            smithay::desktop::utils::SurfacePresentationFeedback::from_states(
                states,
                super::PresentationKind::empty(),
            )
        })
    }

    pub(crate) fn pointer_focus_candidate_accepts(
        &self,
        surface_id: u64,
        location: Point<f64, Logical>,
        surface_origin: Point<f64, Logical>,
    ) -> bool {
        let Some(root_surface) = self.surface_for_id(surface_id) else {
            return false;
        };

        smithay::desktop::utils::under_from_surface_tree(
            &root_surface,
            location,
            surface_origin.to_i32_round(),
            smithay::desktop::WindowSurfaceType::ALL,
        )
        .is_some()
    }

    pub(crate) fn sync_server_state(
        &self,
        server_state: &mut nekoland_ecs::resources::ProtocolServerState,
    ) {
        server_state.last_accept_error = self.last_accept_error.clone();
        server_state.last_dispatch_error = self.last_dispatch_error.clone();
    }

    pub(crate) fn sync_dmabuf_support(&mut self, support: &ProtocolDmabufSupport) {
        self.state.sync_dmabuf_support(&self.display.handle(), support);
    }

    pub(crate) fn sync_xwayland_state(
        &self,
        state: &mut nekoland_ecs::resources::XWaylandServerState,
    ) {
        *state = nekoland_ecs::resources::XWaylandServerState {
            enabled: self.state.xwayland_state.enabled,
            ready: self.state.xwayland_state.ready,
            display_number: self.state.xwayland_state.display_number,
            display_name: self.state.xwayland_state.display_name.clone(),
            startup_error: self.state.xwayland_state.startup_error.clone(),
            last_error: self.last_xwayland_error.clone(),
        };
    }

    fn dismiss_popup_surface(
        &mut self,
        surface_id: u64,
        popup: &smithay::wayland::shell::xdg::PopupSurface,
    ) -> bool {
        let popup_kind = smithay::desktop::PopupKind::from(popup.clone());
        match smithay::desktop::find_popup_root_surface(&popup_kind) {
            Ok(root_surface) => {
                if let Err(error) =
                    smithay::desktop::PopupManager::dismiss_popup(&root_surface, &popup_kind)
                {
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

impl smithay::wayland::foreign_toplevel_list::ForeignToplevelListHandler for ProtocolRuntimeState {
    fn foreign_toplevel_list_state(
        &mut self,
    ) -> &mut smithay::wayland::foreign_toplevel_list::ForeignToplevelListState {
        &mut self._foreign_toplevel_list_state
    }
}

impl smithay::wayland::buffer::BufferHandler for ProtocolRuntimeState {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
    }
}

impl smithay::wayland::dmabuf::DmabufHandler for ProtocolRuntimeState {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &smithay::wayland::dmabuf::DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: smithay::wayland::dmabuf::ImportNotifier,
    ) {
        if self.dmabuf_support.importable && self.dmabuf_support.formats.contains(&dmabuf.format())
        {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}

impl smithay::wayland::shm::ShmHandler for ProtocolRuntimeState {
    fn shm_state(&self) -> &smithay::wayland::shm::ShmState {
        &self.shm_state
    }
}

impl smithay::wayland::output::OutputHandler for ProtocolRuntimeState {
    fn output_bound(
        &mut self,
        output: smithay::output::Output,
        wl_output: smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
    ) {
        self.bound_output_names.insert(wl_output_resource_key(&wl_output), output.name());
    }
}

impl smithay::reexports::wayland_server::backend::ClientData for ProtocolClientState {
    fn initialized(&self, client_id: smithay::reexports::wayland_server::backend::ClientId) {
        tracing::debug!(?client_id, "Wayland client initialized");
    }

    fn disconnected(
        &self,
        client_id: smithay::reexports::wayland_server::backend::ClientId,
        reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {
        tracing::debug!(?client_id, ?reason, "Wayland client disconnected");
    }
}

pub(crate) fn seat_name(
    seat: &smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
) -> String {
    format!("wl_seat@{:?}", seat.id())
}

pub(crate) fn wl_output_resource_key(
    output: &smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
) -> String {
    format!("wl_output@{:?}", output.id())
}

fn synthetic_pointer_grab_matches(
    synthetic_pointer_grab: Option<SyntheticPointerGrab>,
    serial: u32,
    expected_focus_surface_id: u64,
    kind: super::surface::InteractiveRequestKind,
) -> bool {
    synthetic_pointer_grab.is_some_and(|grab| {
        grab.serial == serial
            && (grab.surface_id == expected_focus_surface_id
                || matches!(kind, super::surface::InteractiveRequestKind::PopupGrab))
    })
}

pub(crate) fn remember_protocol_error(
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

#[cfg(test)]
mod tests {
    use super::synthetic_pointer_grab_matches;
    use super::SyntheticPointerGrab;
    use crate::plugin::surface::InteractiveRequestKind;

    #[test]
    fn popup_grab_accepts_matching_press_serial_even_after_pointer_grab_ends() {
        assert!(synthetic_pointer_grab_matches(
            Some(SyntheticPointerGrab { serial: 35, surface_id: 3 }),
            35,
            1,
            InteractiveRequestKind::PopupGrab,
        ));
    }

    #[test]
    fn non_popup_interactive_request_still_requires_exact_surface_match() {
        assert!(!synthetic_pointer_grab_matches(
            Some(SyntheticPointerGrab { serial: 35, surface_id: 3 }),
            35,
            1,
            InteractiveRequestKind::Move,
        ));
    }
}
