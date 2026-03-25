use std::cell::Cell;
use std::io::Error as IoError;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::{EventSource, Interest, PostAction, Readiness, Token};
use nekoland_core::error::NekolandError;
use nekoland_ecs::resources::{BackendInputAction, BackendInputEvent};
use smithay::backend::SwapBuffersError;
use smithay::backend::allocator::Format as DmabufFormat;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::egl::{
    EGLContext, EGLSurface, Error as EglError,
    context::{GlAttributes, PixelFormatRequirements},
    display::EGLDisplay,
    native,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{Bind, ImportDma, RendererSuper};
use smithay::reexports::winit::application::ApplicationHandler;
use smithay::reexports::winit::dpi::PhysicalPosition;
use smithay::reexports::winit::event::{
    DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent,
};
use smithay::reexports::winit::event_loop::{
    ActiveEventLoop, ControlFlow, DeviceEvents, EventLoop,
};
use smithay::reexports::winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use smithay::reexports::winit::platform::scancode::PhysicalKeyExtScancode;
use smithay::reexports::winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use smithay::reexports::winit::window::{
    CursorGrabMode, Window as HostWindow, WindowAttributes, WindowId,
};
use smithay::utils::{Physical, Rectangle, Size};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
    globals::{BindError, GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_seat, wl_surface},
};
use wayland_egl as wegl;
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::{self, ZwpKeyboardShortcutsInhibitManagerV1},
    zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
};

pub(crate) const HOST_WINIT_DEVICE: &str = "winit";

pub(crate) type HostCaptureModeState = Rc<Cell<Option<CursorGrabMode>>>;

/// Minimal host-side winit event stream used by the nested backend.
#[derive(Debug)]
pub(crate) enum HostWinitEvent {
    Resized { size: Size<i32, Physical>, scale_factor: f64 },
    Focus(bool),
    Input(BackendInputEvent),
    CloseRequested,
    Redraw,
}

/// Local copy of Smithay's winit graphics wrapper so we can pair it with a custom event source.
#[derive(Debug)]
pub(crate) struct HostWinitGraphicsBackend {
    renderer: GlesRenderer,
    _display: EGLDisplay,
    egl_surface: EGLSurface,
    window: Arc<HostWindow>,
    damage_tracking: bool,
    bind_size: Option<Size<i32, Physical>>,
    wayland_shortcuts_inhibit: Option<HostWaylandShortcutsInhibitState>,
}

impl HostWinitGraphicsBackend {
    pub(crate) fn window_size(&self) -> Size<i32, Physical> {
        let (w, h): (i32, i32) = self.window.inner_size().into();
        (w, h).into()
    }

    pub(crate) fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    pub(crate) fn window(&self) -> &HostWindow {
        &self.window
    }

    pub(crate) fn sync_wayland_shortcuts_inhibitor(&mut self) {
        let should_inhibit = should_inhibit_host_wayland_shortcuts(
            self.window.fullscreen().is_some(),
            self.window.has_focus(),
        );
        let Some(state) = self.wayland_shortcuts_inhibit.as_mut() else {
            return;
        };
        if let Err(error) = state.sync(should_inhibit) {
            tracing::warn!(
                error = %error,
                "failed to synchronize host Wayland keyboard shortcut inhibitor"
            );
            self.wayland_shortcuts_inhibit = None;
        }
    }

    pub(crate) fn bind(
        &mut self,
    ) -> Result<
        (&mut GlesRenderer, <GlesRenderer as RendererSuper>::Framebuffer<'_>),
        SwapBuffersError,
    > {
        let window_size = self.window_size();
        if Some(window_size) != self.bind_size {
            self.egl_surface.resize(window_size.w, window_size.h, 0, 0);
        }
        self.bind_size = Some(window_size);

        let framebuffer = self.renderer.bind(&mut self.egl_surface)?;
        Ok((&mut self.renderer, framebuffer))
    }

    pub(crate) fn submit(
        &mut self,
        damage: Option<&[Rectangle<i32, Physical>]>,
    ) -> Result<(), SwapBuffersError> {
        let mut damage = match damage {
            Some(damage) if self.damage_tracking && !damage.is_empty() => {
                let Some(bind_size) = self.bind_size else {
                    return Ok(());
                };
                Some(
                    damage
                        .iter()
                        .map(|rect| {
                            Rectangle::new(
                                (rect.loc.x, bind_size.h - rect.loc.y - rect.size.h).into(),
                                rect.size,
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            }
            _ => None,
        };

        self.window.pre_present_notify();
        self.egl_surface.swap_buffers(damage.as_deref_mut())?;
        Ok(())
    }

    pub(crate) fn dmabuf_formats(&self) -> Vec<DmabufFormat> {
        ImportDma::dmabuf_formats(&self.renderer).into_iter().collect::<Vec<_>>()
    }

    pub(crate) fn dmabuf_render_formats(&self) -> Vec<DmabufFormat> {
        Bind::<Dmabuf>::supported_formats(&self.renderer)
            .map(|formats| formats.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default()
    }

    pub(crate) fn dmabuf_main_device(&self) -> Option<u64> {
        smithay::backend::egl::EGLDevice::device_for_display(&self._display)
            .ok()
            .and_then(|device| device.try_get_render_node().ok().flatten())
            .map(|node| node.dev_id())
    }
}

#[derive(Debug, Default)]
struct HostWaylandShortcutDispatchState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for HostWaylandShortcutDispatchState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(HostWaylandShortcutDispatchState: ignore wl_seat::WlSeat);
delegate_noop!(HostWaylandShortcutDispatchState: ignore zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1);
delegate_noop!(HostWaylandShortcutDispatchState: ignore ZwpKeyboardShortcutsInhibitorV1);

#[derive(Debug)]
struct HostWaylandShortcutsInhibitState {
    event_queue: EventQueue<HostWaylandShortcutDispatchState>,
    dispatch_state: HostWaylandShortcutDispatchState,
    manager: ZwpKeyboardShortcutsInhibitManagerV1,
    seat: wl_seat::WlSeat,
    surface: wl_surface::WlSurface,
    inhibitor: Option<ZwpKeyboardShortcutsInhibitorV1>,
}

impl HostWaylandShortcutsInhibitState {
    fn try_new(window: &HostWindow) -> Option<Self> {
        let raw_display = match window.display_handle() {
            Ok(handle) => handle.as_raw(),
            Err(error) => {
                tracing::debug!(error = %error, "host window did not expose a display handle");
                return None;
            }
        };
        let raw_window = match window.window_handle() {
            Ok(handle) => handle.as_raw(),
            Err(error) => {
                tracing::debug!(error = %error, "host window did not expose a window handle");
                return None;
            }
        };
        let (display_ptr, surface_ptr) = match (raw_display, raw_window) {
            (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) => {
                (display.display.as_ptr().cast(), surface.surface.as_ptr().cast())
            }
            _ => return None,
        };

        let connection = Connection::from_backend(unsafe {
            wayland_client::backend::Backend::from_foreign_display(display_ptr)
        });
        let (globals, event_queue) = match registry_queue_init::<HostWaylandShortcutDispatchState>(
            &connection,
        ) {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to initialize host Wayland registry queue for keyboard shortcut inhibitor"
                );
                return None;
            }
        };
        let queue_handle = event_queue.handle();
        let manager = match globals.bind::<ZwpKeyboardShortcutsInhibitManagerV1, _, _>(
            &queue_handle,
            1..=1,
            (),
        ) {
            Ok(manager) => manager,
            Err(BindError::NotPresent) => {
                tracing::debug!(
                    "host Wayland compositor does not advertise keyboard-shortcuts-inhibit"
                );
                return None;
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to bind host Wayland keyboard shortcut inhibitor manager"
                );
                return None;
            }
        };
        let seat = match bind_first_global::<wl_seat::WlSeat>(&globals, &queue_handle, 1..=1) {
            Ok(seat) => seat,
            Err(BindError::NotPresent) => {
                tracing::debug!("host Wayland compositor did not advertise a wl_seat");
                return None;
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to bind host Wayland wl_seat");
                return None;
            }
        };
        let surface_id = unsafe {
            wayland_client::backend::ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                surface_ptr,
            )
        };
        let surface_id = match surface_id {
            Ok(surface_id) => surface_id,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to wrap host Wayland surface pointer for keyboard shortcut inhibitor"
                );
                return None;
            }
        };
        let surface = match wl_surface::WlSurface::from_id(&connection, surface_id) {
            Ok(surface) => surface,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to construct host Wayland surface proxy for keyboard shortcut inhibitor"
                );
                return None;
            }
        };

        Some(Self {
            event_queue,
            dispatch_state: HostWaylandShortcutDispatchState,
            manager,
            seat,
            surface,
            inhibitor: None,
        })
    }

    fn sync(&mut self, should_inhibit: bool) -> Result<(), String> {
        self.dispatch_pending()?;
        match (self.inhibitor.is_some(), should_inhibit) {
            (false, true) => self.create_inhibitor()?,
            (true, false) => self.destroy_inhibitor()?,
            _ => {}
        }
        Ok(())
    }

    fn create_inhibitor(&mut self) -> Result<(), String> {
        let queue_handle = self.event_queue.handle();
        let inhibitor =
            self.manager.inhibit_shortcuts(&self.surface, &self.seat, &queue_handle, ());
        self.inhibitor = Some(inhibitor);
        self.flush_requests()
    }

    fn destroy_inhibitor(&mut self) -> Result<(), String> {
        let Some(inhibitor) = self.inhibitor.take() else {
            return Ok(());
        };
        inhibitor.destroy();
        self.flush_requests()
    }

    fn dispatch_pending(&mut self) -> Result<(), String> {
        self.event_queue
            .dispatch_pending(&mut self.dispatch_state)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn flush_requests(&mut self) -> Result<(), String> {
        self.event_queue.flush().map_err(|error| error.to_string())
    }
}

impl Drop for HostWaylandShortcutsInhibitState {
    fn drop(&mut self) {
        let _ = self.destroy_inhibitor();
        self.manager.destroy();
        let _ = self.flush_requests();
        let _ = self.dispatch_pending();
    }
}

fn bind_first_global<I>(
    globals: &wayland_client::globals::GlobalList,
    queue_handle: &QueueHandle<HostWaylandShortcutDispatchState>,
    version: RangeInclusive<u32>,
) -> Result<I, BindError>
where
    I: Proxy + 'static,
    HostWaylandShortcutDispatchState: Dispatch<I, ()> + 'static,
{
    let version_start = *version.start();
    let version_end = *version.end();
    let global = globals
        .contents()
        .clone_list()
        .into_iter()
        .find(|global| global.interface == I::interface().name)
        .ok_or(BindError::NotPresent)?;

    if global.version < version_start {
        return Err(BindError::UnsupportedVersion);
    }

    Ok(globals.registry().bind::<I, _, _>(
        global.name,
        global.version.min(version_end),
        queue_handle,
        (),
    ))
}

fn should_inhibit_host_wayland_shortcuts(fullscreen: bool, focused: bool) -> bool {
    fullscreen && focused
}

#[derive(Debug)]
pub(crate) struct HostWinitEventLoop {
    inner: HostWinitEventLoopInner,
    fake_token: Option<Token>,
    pending_events: Vec<HostWinitEvent>,
    event_loop: Generic<EventLoop<()>>,
}

#[derive(Debug)]
struct HostWinitEventLoopInner {
    window: Arc<HostWindow>,
    is_x11: bool,
    scale_factor: f64,
    capture_mode: HostCaptureModeState,
}

impl HostWinitEventLoop {
    fn dispatch_new_events<F>(&mut self, callback: F) -> PumpStatus
    where
        F: FnMut(HostWinitEvent),
    {
        let event_loop = unsafe { self.event_loop.get_mut() };
        event_loop.pump_app_events(
            Some(Duration::ZERO),
            &mut HostWinitEventLoopApp { inner: &mut self.inner, callback },
        )
    }
}

struct HostWinitEventLoopApp<'a, F: FnMut(HostWinitEvent)> {
    inner: &'a mut HostWinitEventLoopInner,
    callback: F,
}

impl<F: FnMut(HostWinitEvent)> HostWinitEventLoopApp<'_, F> {
    fn emit_input(&mut self, action: BackendInputAction) {
        (self.callback)(HostWinitEvent::Input(BackendInputEvent {
            device: HOST_WINIT_DEVICE.to_owned(),
            action,
        }));
    }
}

impl<F: FnMut(HostWinitEvent)> ApplicationHandler for HostWinitEventLoopApp<'_, F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.listen_device_events(DeviceEvents::WhenFocused);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::Resized(size) => {
                let (w, h): (i32, i32) = size.into();
                (self.callback)(HostWinitEvent::Resized {
                    size: (w, h).into(),
                    scale_factor: self.inner.scale_factor,
                });
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.inner.scale_factor = scale_factor;
                let (w, h): (i32, i32) = self.inner.window.inner_size().into();
                (self.callback)(HostWinitEvent::Resized {
                    size: (w, h).into(),
                    scale_factor: self.inner.scale_factor,
                });
            }
            WindowEvent::RedrawRequested => {
                (self.callback)(HostWinitEvent::Redraw);
            }
            WindowEvent::CloseRequested => {
                (self.callback)(HostWinitEvent::CloseRequested);
            }
            WindowEvent::Focused(focused) => {
                (self.callback)(HostWinitEvent::Focus(focused));
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. }
                if !is_synthetic && !event.repeat =>
            {
                let scancode = event.physical_key.to_scancode().unwrap_or(0);
                self.emit_input(BackendInputAction::Key {
                    keycode: scancode.saturating_add(8),
                    pressed: event.state == ElementState::Pressed,
                });
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.emit_input(BackendInputAction::PointerMoved { x: position.x, y: position.y });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (horizontal, vertical) = translate_scroll_delta(delta);
                self.emit_input(BackendInputAction::PointerAxis { horizontal, vertical });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.emit_input(BackendInputAction::PointerButton {
                    button_code: translate_button_code(button, self.inner.is_x11),
                    pressed: state == ElementState::Pressed,
                });
            }
            WindowEvent::DroppedFile(_)
            | WindowEvent::Destroyed
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::AxisMotion { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::Ime(_)
            | WindowEvent::Moved(_)
            | WindowEvent::Occluded(_)
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::ThemeChanged(_)
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::ActivationTokenDone { .. }
            | WindowEvent::Touch(_) => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: smithay::reexports::winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let Some(BackendInputAction::PointerDelta { dx, dy }) =
            translate_device_mouse_motion(self.inner.capture_mode.get(), event)
        {
            self.emit_input(BackendInputAction::PointerDelta { dx, dy });
        }
    }
}

impl EventSource for HostWinitEventLoop {
    type Event = HostWinitEvent;
    type Metadata = ();
    type Ret = ();
    type Error = IoError;

    const NEEDS_EXTRA_LIFECYCLE_EVENTS: bool = true;

    fn before_sleep(&mut self) -> calloop::Result<Option<(Readiness, Token)>> {
        let mut pending_events = std::mem::take(&mut self.pending_events);
        self.dispatch_new_events(|event| pending_events.push(event));
        self.pending_events = pending_events;
        if self.pending_events.is_empty() {
            Ok(None)
        } else {
            let Some(fake_token) = self.fake_token else {
                return Err(calloop::Error::IoError(IoError::other(
                    "winit fake token missing before sleep",
                )));
            };
            Ok(Some((Readiness::EMPTY, fake_token)))
        }
    }

    fn process_events<F>(
        &mut self,
        _readiness: Readiness,
        _token: Token,
        mut callback: F,
    ) -> Result<PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        let mut callback = |event| callback(event, &mut ());
        for event in self.pending_events.drain(..) {
            callback(event);
        }
        Ok(match self.dispatch_new_events(callback) {
            PumpStatus::Continue => PostAction::Continue,
            PumpStatus::Exit(_) => PostAction::Remove,
        })
    }

    fn register(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.fake_token = Some(token_factory.token());
        self.event_loop.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.event_loop.register(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut calloop::Poll) -> calloop::Result<()> {
        self.event_loop.unregister(poll)
    }
}

pub(crate) fn init_host_winit(
    attributes: WindowAttributes,
) -> Result<(HostWinitGraphicsBackend, HostWinitEventLoop, HostCaptureModeState), NekolandError> {
    let mut event_loop_builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        use smithay::reexports::winit::platform::wayland::EventLoopBuilderExtWayland;
        use smithay::reexports::winit::platform::x11::EventLoopBuilderExtX11;

        EventLoopBuilderExtWayland::with_any_thread(&mut event_loop_builder, true);
        EventLoopBuilderExtX11::with_any_thread(&mut event_loop_builder, true);
    }
    let event_loop =
        event_loop_builder.build().map_err(|error| NekolandError::Runtime(error.to_string()))?;
    event_loop.listen_device_events(DeviceEvents::WhenFocused);

    #[allow(deprecated)]
    let window = Arc::new(
        event_loop
            .create_window(attributes)
            .map_err(|error| NekolandError::Runtime(error.to_string()))?,
    );

    let (display, context, surface, is_x11) = create_host_surface(window.clone())?;
    let renderer = unsafe { GlesRenderer::new(context) }
        .map_err(|error| NekolandError::Runtime(error.to_string()))?;
    let damage_tracking = display.supports_damage();
    let capture_mode = Rc::new(Cell::new(None));
    let wayland_shortcuts_inhibit = HostWaylandShortcutsInhibitState::try_new(window.as_ref());

    event_loop.set_control_flow(ControlFlow::Poll);
    let event_loop = Generic::new(event_loop, Interest::READ, calloop::Mode::Level);

    Ok((
        HostWinitGraphicsBackend {
            renderer,
            _display: display,
            egl_surface: surface,
            window: window.clone(),
            damage_tracking,
            bind_size: None,
            wayland_shortcuts_inhibit,
        },
        HostWinitEventLoop {
            inner: HostWinitEventLoopInner {
                scale_factor: window.scale_factor(),
                window,
                is_x11,
                capture_mode: capture_mode.clone(),
            },
            fake_token: None,
            event_loop,
            pending_events: Vec::new(),
        },
        capture_mode,
    ))
}

fn create_host_surface(
    window: Arc<HostWindow>,
) -> Result<(EGLDisplay, EGLContext, EGLSurface, bool), NekolandError> {
    let display = unsafe { EGLDisplay::new(window.clone()) }
        .map_err(|error| NekolandError::Runtime(error.to_string()))?;
    let gl_attributes = GlAttributes {
        version: (3, 0),
        profile: None,
        debug: cfg!(debug_assertions),
        vsync: false,
    };
    let context =
        EGLContext::new_with_config(&display, gl_attributes, PixelFormatRequirements::_10_bit())
            .or_else(|_| {
                EGLContext::new_with_config(
                    &display,
                    gl_attributes,
                    PixelFormatRequirements::_8_bit(),
                )
            })
            .map_err(|error| NekolandError::Runtime(error.to_string()))?;

    let (surface, is_x11) = match window.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Wayland(handle)) => {
            let size = window.inner_size();
            let surface = unsafe {
                wegl::WlEglSurface::new_from_raw(
                    handle.surface.as_ptr() as *mut _,
                    size.width as i32,
                    size.height as i32,
                )
            }
            .map_err(|error| NekolandError::Runtime(error.to_string()))?;
            let surface = unsafe {
                EGLSurface::new(
                    &display,
                    context.pixel_format().ok_or_else(|| {
                        NekolandError::Runtime(
                            "nested winit EGL context did not expose a pixel format".to_owned(),
                        )
                    })?,
                    context.config_id(),
                    surface,
                )
            }
            .map_err(|error| map_egl_creation_error(EglError::CreationFailed(error)))?;
            (surface, false)
        }
        Ok(RawWindowHandle::Xlib(handle)) => {
            let surface = unsafe {
                EGLSurface::new(
                    &display,
                    context.pixel_format().ok_or_else(|| {
                        NekolandError::Runtime(
                            "nested winit EGL context did not expose a pixel format".to_owned(),
                        )
                    })?,
                    context.config_id(),
                    native::XlibWindow(handle.window),
                )
            }
            .map_err(|error| map_egl_creation_error(EglError::CreationFailed(error)))?;
            (surface, true)
        }
        _ => {
            return Err(NekolandError::Runtime(
                "nested winit backend only supports Wayland or X11 host windows".to_owned(),
            ));
        }
    };

    let _ = context.unbind();
    Ok((display, context, surface, is_x11))
}

fn map_egl_creation_error(error: EglError) -> NekolandError {
    NekolandError::Runtime(error.to_string())
}

fn translate_scroll_delta(delta: MouseScrollDelta) -> (f64, f64) {
    match delta {
        MouseScrollDelta::PixelDelta(PhysicalPosition { x, y }) => (-x, -y),
        MouseScrollDelta::LineDelta(x, y) => (-(x as f64) * 120.0, -(y as f64) * 120.0),
    }
}

fn translate_button_code(button: MouseButton, is_x11: bool) -> u32 {
    match button {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
        MouseButton::Forward => 0x115,
        MouseButton::Back => 0x116,
        MouseButton::Other(button) => {
            if is_x11 {
                xorg_mouse_to_libinput(button.into())
            } else {
                button.into()
            }
        }
    }
}

fn translate_device_mouse_motion(
    capture_mode: Option<CursorGrabMode>,
    event: DeviceEvent,
) -> Option<BackendInputAction> {
    if capture_mode != Some(CursorGrabMode::Locked) {
        return None;
    }

    match event {
        DeviceEvent::MouseMotion { delta: (dx, dy) } => {
            Some(BackendInputAction::PointerDelta { dx, dy })
        }
        _ => None,
    }
}

fn xorg_mouse_to_libinput(button: u32) -> u32 {
    match button {
        0 => 0,
        1 => 0x110,
        2 => 0x112,
        3 => 0x111,
        _ => button - 8 + 0x113,
    }
}

#[cfg(test)]
mod tests {
    use smithay::reexports::winit::event::{DeviceEvent, MouseButton, MouseScrollDelta};
    use smithay::reexports::winit::window::CursorGrabMode;

    use crate::winit::host::{
        should_inhibit_host_wayland_shortcuts, translate_device_mouse_motion,
    };

    use super::{translate_button_code, translate_scroll_delta, xorg_mouse_to_libinput};

    #[test]
    fn line_scroll_delta_uses_v120_units() {
        assert_eq!(translate_scroll_delta(MouseScrollDelta::LineDelta(1.0, -2.0)), (-120.0, 240.0));
    }

    #[test]
    fn x11_other_buttons_follow_libinput_mapping() {
        assert_eq!(translate_button_code(MouseButton::Other(1), true), 0x110);
        assert_eq!(translate_button_code(MouseButton::Other(9), true), xorg_mouse_to_libinput(9));
    }

    #[test]
    fn device_mouse_motion_only_translates_when_locked() {
        assert_eq!(
            translate_device_mouse_motion(
                Some(CursorGrabMode::Locked),
                DeviceEvent::MouseMotion { delta: (3.0, -4.0) }
            ),
            Some(nekoland_ecs::resources::BackendInputAction::PointerDelta { dx: 3.0, dy: -4.0 })
        );
        assert_eq!(
            translate_device_mouse_motion(
                Some(CursorGrabMode::Confined),
                DeviceEvent::MouseMotion { delta: (3.0, -4.0) }
            ),
            None
        );
    }

    #[test]
    fn host_wayland_shortcuts_are_only_inhibited_for_focused_fullscreen_windows() {
        assert!(should_inhibit_host_wayland_shortcuts(true, true));
        assert!(!should_inhibit_host_wayland_shortcuts(true, false));
        assert!(!should_inhibit_host_wayland_shortcuts(false, true));
    }
}
