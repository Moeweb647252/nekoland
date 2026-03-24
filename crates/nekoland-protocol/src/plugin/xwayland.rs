#[derive(Debug, Clone, Default)]
pub(crate) struct XWaylandRuntimeState {
    pub(crate) enabled: bool,
    pub(crate) ready: bool,
    pub(crate) display_number: Option<u32>,
    pub(crate) display_name: Option<String>,
    pub(crate) startup_error: Option<String>,
}

pub(crate) fn sync_xwayland_server_state_system(
    server: Option<bevy_ecs::prelude::NonSendMut<'_, super::server::SmithayProtocolServer>>,
    mut xwayland_state: bevy_ecs::prelude::ResMut<'_, nekoland_ecs::resources::XWaylandServerState>,
) {
    let Some(server) = server else {
        return;
    };
    server.sync_xwayland_state(&mut xwayland_state);
}

pub(crate) fn dispatch_xwayland_runtime_system(
    server: Option<bevy_ecs::prelude::NonSendMut<'_, super::server::SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    server.dispatch_xwayland();
}

pub(crate) fn dispatch_window_server_requests_system(
    mut pending_window_requests: bevy_ecs::prelude::ResMut<
        '_,
        nekoland_ecs::resources::PendingWindowServerRequests,
    >,
    server: Option<bevy_ecs::prelude::NonSendMut<'_, super::server::SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    let mut deferred = Vec::new();

    for request in pending_window_requests.drain() {
        let handled = match request.action {
            nekoland_ecs::resources::WindowServerAction::Close => {
                server.send_close(request.surface_id)
            }
            nekoland_ecs::resources::WindowServerAction::SyncXdgToplevelState {
                size,
                fullscreen,
                maximized,
                resizing,
            } => server
                .sync_xdg_toplevel_state(request.surface_id, size, fullscreen, maximized, resizing),
            nekoland_ecs::resources::WindowServerAction::SyncX11WindowPresentation {
                geometry,
                fullscreen,
                maximized,
            } => server.sync_x11_window_presentation(
                request.surface_id,
                geometry,
                fullscreen,
                maximized,
            ),
        };

        if !handled {
            deferred.push(request);
        }
    }

    pending_window_requests.replace(deferred);
}

pub(crate) fn dispatch_popup_server_requests_system(
    mut pending_popup_requests: bevy_ecs::prelude::ResMut<
        '_,
        nekoland_ecs::resources::PendingPopupServerRequests,
    >,
    server: Option<bevy_ecs::prelude::NonSendMut<'_, super::server::SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    let mut deferred = Vec::new();

    for request in pending_popup_requests.drain() {
        let handled = match request.action {
            crate::resources::PopupServerAction::Dismiss => {
                server.dismiss_popup(request.surface_id)
            }
        };

        if !handled {
            deferred.push(request);
        }
    }

    pending_popup_requests.replace(deferred);
}

pub(crate) fn map_x11_resize_edge(
    edge: smithay::xwayland::xwm::ResizeEdge,
) -> nekoland_ecs::resources::ResizeEdges {
    match edge {
        smithay::xwayland::xwm::ResizeEdge::Top => nekoland_ecs::resources::ResizeEdges::Top,
        smithay::xwayland::xwm::ResizeEdge::Bottom => nekoland_ecs::resources::ResizeEdges::Bottom,
        smithay::xwayland::xwm::ResizeEdge::Left => nekoland_ecs::resources::ResizeEdges::Left,
        smithay::xwayland::xwm::ResizeEdge::TopLeft => {
            nekoland_ecs::resources::ResizeEdges::TopLeft
        }
        smithay::xwayland::xwm::ResizeEdge::BottomLeft => {
            nekoland_ecs::resources::ResizeEdges::BottomLeft
        }
        smithay::xwayland::xwm::ResizeEdge::Right => nekoland_ecs::resources::ResizeEdges::Right,
        smithay::xwayland::xwm::ResizeEdge::TopRight => {
            nekoland_ecs::resources::ResizeEdges::TopRight
        }
        smithay::xwayland::xwm::ResizeEdge::BottomRight => {
            nekoland_ecs::resources::ResizeEdges::BottomRight
        }
    }
}

impl super::server::ProtocolRuntimeState {
    fn x11_app_id(window: &smithay::xwayland::xwm::X11Surface) -> String {
        let class = window.class();
        if class.is_empty() { window.instance() } else { class }
    }

    fn x11_geometry(
        window: &smithay::xwayland::xwm::X11Surface,
    ) -> nekoland_ecs::resources::X11WindowGeometry {
        let geometry = window.geometry();
        nekoland_ecs::resources::X11WindowGeometry {
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w.max(1) as u32,
            height: geometry.size.h.max(1) as u32,
        }
    }

    fn x11_window_type(
        window: &smithay::xwayland::xwm::X11Surface,
    ) -> Option<nekoland_ecs::components::X11WindowType> {
        match window.window_type() {
            Some(smithay::xwayland::xwm::WmWindowType::DropdownMenu) => {
                Some(nekoland_ecs::components::X11WindowType::DropdownMenu)
            }
            Some(smithay::xwayland::xwm::WmWindowType::Dialog) => {
                Some(nekoland_ecs::components::X11WindowType::Dialog)
            }
            Some(smithay::xwayland::xwm::WmWindowType::Menu) => {
                Some(nekoland_ecs::components::X11WindowType::Menu)
            }
            Some(smithay::xwayland::xwm::WmWindowType::Notification) => {
                Some(nekoland_ecs::components::X11WindowType::Notification)
            }
            Some(smithay::xwayland::xwm::WmWindowType::Normal) => {
                Some(nekoland_ecs::components::X11WindowType::Normal)
            }
            Some(smithay::xwayland::xwm::WmWindowType::PopupMenu) => {
                Some(nekoland_ecs::components::X11WindowType::PopupMenu)
            }
            Some(smithay::xwayland::xwm::WmWindowType::Splash) => {
                Some(nekoland_ecs::components::X11WindowType::Splash)
            }
            Some(smithay::xwayland::xwm::WmWindowType::Toolbar) => {
                Some(nekoland_ecs::components::X11WindowType::Toolbar)
            }
            Some(smithay::xwayland::xwm::WmWindowType::Tooltip) => {
                Some(nekoland_ecs::components::X11WindowType::Tooltip)
            }
            Some(smithay::xwayland::xwm::WmWindowType::Utility) => {
                Some(nekoland_ecs::components::X11WindowType::Utility)
            }
            None => None,
        }
    }

    fn should_publish_managed_x11_window(
        title: &str,
        app_id: &str,
        geometry: nekoland_ecs::resources::X11WindowGeometry,
        popup: bool,
        window_type: Option<nekoland_ecs::components::X11WindowType>,
    ) -> bool {
        if title.is_empty() && app_id.is_empty() && geometry.width <= 1 && geometry.height <= 1 {
            return false;
        }

        if popup {
            return false;
        }

        !matches!(
            window_type,
            Some(
                nekoland_ecs::components::X11WindowType::DropdownMenu
                    | nekoland_ecs::components::X11WindowType::Menu
                    | nekoland_ecs::components::X11WindowType::Notification
                    | nekoland_ecs::components::X11WindowType::PopupMenu
                    | nekoland_ecs::components::X11WindowType::Tooltip
            )
        )
    }

    fn remember_x11_window(&mut self, window: &smithay::xwayland::xwm::X11Surface) {
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
        let popup = window.is_popup();
        let transient_for = window.is_transient_for();
        let window_type = Self::x11_window_type(&window);

        if !Self::should_publish_managed_x11_window(&title, &app_id, geometry, popup, window_type) {
            tracing::trace!(
                window_id,
                surface_id,
                window_type = ?window_type,
                popup,
                transient_for = ?transient_for,
                override_redirect = window.is_override_redirect(),
                "ignoring XWayland helper surface"
            );
            return;
        }

        let event = if self.published_x11_windows.insert(window_id) {
            crate::ProtocolEvent::X11WindowMapped {
                surface_id,
                window_id,
                override_redirect: window.is_override_redirect(),
                popup,
                transient_for,
                window_type,
                title,
                app_id,
                geometry,
            }
        } else {
            crate::ProtocolEvent::X11WindowReconfigured {
                surface_id,
                title,
                app_id,
                popup,
                transient_for,
                window_type,
                geometry,
            }
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
        let popup = window.is_popup();
        let transient_for = window.is_transient_for();
        let window_type = Self::x11_window_type(&window);

        self.queue_event(crate::ProtocolEvent::X11WindowReconfigured {
            surface_id,
            title: window.title(),
            app_id: Self::x11_app_id(&window),
            popup,
            transient_for,
            window_type,
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

    pub(crate) fn handle_xwayland_event(
        &mut self,
        handle: calloop::LoopHandle<'static, super::server::ProtocolRuntimeState>,
        event: smithay::xwayland::XWaylandEvent,
    ) {
        match event {
            smithay::xwayland::XWaylandEvent::Ready { x11_socket, display_number } => {
                let Some(client) = self.xwayland_client.clone() else {
                    self.xwayland_state.startup_error =
                        Some("XWayland client handle disappeared before startup".to_owned());
                    self.xwayland_state.ready = false;
                    return;
                };

                let xwm_socket = x11_socket.try_clone().ok();
                match smithay::xwayland::xwm::X11Wm::start_wm(handle, x11_socket, client) {
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
            smithay::xwayland::XWaylandEvent::Error => {
                self.xwayland_state.ready = false;
                self.xwayland_state.display_number = None;
                self.xwayland_state.display_name = None;
                tracing::warn!("XWayland failed during startup");
            }
        }
    }
}

impl smithay::wayland::xwayland_shell::XWaylandShellHandler
    for super::server::ProtocolRuntimeState
{
    fn xwayland_shell_state(
        &mut self,
    ) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(
        &mut self,
        xwm: smithay::xwayland::xwm::XwmId,
        wl_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        surface: smithay::xwayland::xwm::X11Surface,
    ) {
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

impl smithay::xwayland::xwm::XwmHandler for super::server::ProtocolRuntimeState {
    #[allow(clippy::expect_used)]
    fn xwm_state(
        &mut self,
        xwm: smithay::xwayland::xwm::XwmId,
    ) -> &mut smithay::xwayland::xwm::X11Wm {
        self.xwms.get_mut(&xwm).expect("XWayland WM callback referenced an unknown XWM")
    }

    fn new_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        self.remember_x11_window(&window);
    }

    fn new_override_redirect_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        self.remember_x11_window(&window);
    }

    fn map_window_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        let window_id = window.window_id();
        self.remember_x11_window(&window);
        if let Err(error) = window.set_mapped(true) {
            tracing::warn!(window_id, error = %error, "failed to map XWayland window");
            return;
        }
        self.mapped_x11_windows.insert(window_id);
        self.publish_x11_window_if_ready(window_id);
    }

    fn map_window_notify(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        let window_id = window.window_id();
        self.remember_x11_window(&window);
        self.mapped_x11_windows.insert(window_id);
        self.publish_x11_window_if_ready(window_id);
    }

    fn mapped_override_redirect_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        let window_id = window.window_id();
        self.remember_x11_window(&window);
        self.mapped_x11_windows.insert(window_id);
        self.publish_x11_window_if_ready(window_id);
    }

    fn unmapped_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        let window_id = window.window_id();
        if let Some(surface_id) = self.unpublish_x11_window(window_id) {
            self.queue_event(crate::ProtocolEvent::X11WindowUnmapped { surface_id });
        }
        self.x11_windows.insert(window_id, window);
    }

    fn destroyed_window(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        let window_id = window.window_id();
        if let Some(surface_id) = self.unpublish_x11_window(window_id) {
            self.queue_event(crate::ProtocolEvent::X11WindowDestroyed { surface_id });
        }
        self.x11_windows.remove(&window_id);
    }

    fn configure_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<smithay::xwayland::xwm::Reorder>,
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
            tracing::warn!(window_id = window.window_id(), error = %error, "failed to configure XWayland window");
            return;
        }

        self.remember_x11_window(&window);
        self.queue_x11_reconfigured(window.window_id());
    }

    fn configure_notify(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
        _geometry: smithay::utils::Rectangle<i32, smithay::utils::Logical>,
        _above: Option<smithay::xwayland::xwm::X11Window>,
    ) {
        self.remember_x11_window(&window);
        self.queue_x11_reconfigured(window.window_id());
    }

    fn property_notify(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
        _property: smithay::xwayland::xwm::WmWindowProperty,
    ) {
        self.remember_x11_window(&window);
        self.queue_x11_reconfigured(window.window_id());
    }

    fn maximize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(crate::ProtocolEvent::X11WindowMaximizeRequested { surface_id });
        }
    }

    fn unmaximize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(crate::ProtocolEvent::X11WindowUnMaximizeRequested { surface_id });
        }
    }

    fn fullscreen_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(crate::ProtocolEvent::X11WindowFullscreenRequested { surface_id });
        }
    }

    fn unfullscreen_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(crate::ProtocolEvent::X11WindowUnFullscreenRequested { surface_id });
        }
    }

    fn minimize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(crate::ProtocolEvent::X11WindowMinimizeRequested { surface_id });
        }
    }

    fn unminimize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(crate::ProtocolEvent::X11WindowUnMinimizeRequested { surface_id });
        }
    }

    fn resize_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
        button: u32,
        resize_edge: smithay::xwayland::xwm::ResizeEdge,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(crate::ProtocolEvent::X11WindowResizeRequested {
                surface_id,
                button,
                edges: map_x11_resize_edge(resize_edge),
            });
        }
    }

    fn move_request(
        &mut self,
        _xwm: smithay::xwayland::xwm::XwmId,
        window: smithay::xwayland::xwm::X11Surface,
        button: u32,
    ) {
        self.remember_x11_window(&window);
        if let Some(surface_id) = self.x11_surface_ids_by_window.get(&window.window_id()).copied() {
            self.queue_event(crate::ProtocolEvent::X11WindowMoveRequested { surface_id, button });
        }
    }

    fn disconnected(&mut self, xwm: smithay::xwayland::xwm::XwmId) {
        self.xwms.remove(&xwm);
    }
}
