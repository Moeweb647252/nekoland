use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;
use std::time::Duration;

use bevy_app::App;
use bevy_ecs::prelude::{Local, NonSend, NonSendMut, Query, Res, ResMut, Resource};
use calloop::timer::{TimeoutAction, Timer};
use nekoland_core::calloop::CalloopSourceRegistry;
use nekoland_core::error::NekolandError;
use nekoland_core::prelude::AppMetadata;
use nekoland_ecs::components::{OutputDevice, OutputProperties, SurfaceGeometry, WlSurfaceHandle};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, CompositorConfig, PendingBackendInputEvents,
    PendingOutputPresentationEvents, PendingProtocolInputEvents, RenderList,
};
use nekoland_protocol::ProtocolSurfaceRegistry;
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent,
};
use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self as smithay_winit, WinitEvent, WinitGraphicsBackend};
use smithay::reexports::winit::dpi::PhysicalSize;
use smithay::reexports::winit::window::Window as HostWindow;
use smithay::utils::{Monotonic, Physical, Size, Transform};

use crate::plugin::{
    OutputPresentationRuntime, emit_present_completion_events_at, parse_output_mode,
};
use crate::traits::{Backend, BackendKind};

type WinitRendererBackend = WinitGraphicsBackend<GlesRenderer>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WinitBackend {
    pub title: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WinitPresentCompletionSource {
    shared: Rc<RefCell<WinitPresentCompletionShared>>,
}

#[derive(Debug, Clone, Resource, PartialEq, Eq)]
pub struct WinitWindowState {
    pub driver: String,
    pub title: String,
    pub requested_width: u32,
    pub requested_height: u32,
    pub actual_width: Option<u32>,
    pub actual_height: Option<u32>,
    pub actual_scale: Option<u32>,
    pub closed: bool,
}

impl Default for WinitWindowState {
    fn default() -> Self {
        Self {
            driver: "timer-fallback".to_owned(),
            title: "nekoland".to_owned(),
            requested_width: 1280,
            requested_height: 720,
            actual_width: None,
            actual_height: None,
            actual_scale: None,
            closed: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WinitPresentDriver {
    TimerFallback,
    SmithayEventLoop,
}

impl Default for WinitPresentDriver {
    fn default() -> Self {
        Self::TimerFallback
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingWinitWindowState {
    size: Size<i32, Physical>,
    scale_factor: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WinitWindowSpec {
    title: String,
    width: u32,
    height: u32,
}

#[derive(Debug)]
struct WinitPresentCompletionShared {
    active: bool,
    refresh_interval: Duration,
    pending_input_events: Vec<BackendInputEvent>,
    pending_timestamps_nanos: Vec<u64>,
    pending_window_state: Option<PendingWinitWindowState>,
    backend: Option<WinitRendererBackend>,
    closed: bool,
    driver: WinitPresentDriver,
    desired_window_spec: WinitWindowSpec,
}

impl Default for WinitPresentCompletionShared {
    fn default() -> Self {
        Self {
            active: false,
            refresh_interval: Duration::from_millis(16),
            pending_input_events: Vec::new(),
            pending_timestamps_nanos: Vec::new(),
            pending_window_state: None,
            backend: None,
            closed: false,
            driver: WinitPresentDriver::TimerFallback,
            desired_window_spec: WinitWindowSpec {
                title: "nekoland".to_owned(),
                width: 1280,
                height: 720,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WinitOutputMode {
    width: u32,
    height: u32,
    scale: u32,
}

#[derive(Debug, Default)]
pub(crate) struct WinitRenderState {
    damage_tracker: Option<OutputDamageTracker>,
    mode: Option<WinitOutputMode>,
}

const INACTIVE_PRESENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const MIN_PRESENT_INTERVAL: Duration = Duration::from_micros(500);

impl Backend for WinitBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Winit
    }

    fn label(&self) -> &str {
        "winit"
    }
}

pub(crate) fn install_winit_present_completion_source(app: &mut App) {
    if app.world().get_non_send_resource::<WinitPresentCompletionSource>().is_some() {
        return;
    }

    if app.world().get_non_send_resource::<CalloopSourceRegistry>().is_none() {
        app.insert_non_send_resource(CalloopSourceRegistry::default());
    }

    let initial_window_spec = desired_window_spec(
        app.world().get_resource::<AppMetadata>(),
        app.world().get_resource::<CompositorConfig>(),
        None,
    );
    let source = WinitPresentCompletionSource::default();
    {
        let mut shared = source.shared.borrow_mut();
        shared.desired_window_spec = initial_window_spec.clone();
    }
    let shared = source.shared.clone();
    let mut registry = app
        .world_mut()
        .get_non_send_resource_mut::<CalloopSourceRegistry>()
        .expect("calloop registry inserted immediately before access");

    registry.push(move |handle| {
        if requested_backend_kind() == BackendKind::Winit {
            match install_smithay_winit_source(handle, shared.clone()) {
                Ok(()) => Ok(()),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to initialize smithay winit event source; falling back to timer"
                    );
                    install_timer_source(handle, shared.clone())
                }
            }
        } else {
            install_timer_source(handle, shared.clone())
        }
    });
    drop(registry);

    app.insert_non_send_resource(source);
    app.insert_resource(WinitWindowState {
        title: initial_window_spec.title,
        requested_width: initial_window_spec.width,
        requested_height: initial_window_spec.height,
        ..WinitWindowState::default()
    });
}

pub(crate) fn winit_backend_system(
    mut selected_backend: ResMut<crate::traits::SelectedBackend>,
    outputs: Query<(&OutputDevice, &mut OutputProperties)>,
    mut pending_backend_inputs: ResMut<PendingBackendInputEvents>,
    mut pending_protocol_inputs: ResMut<PendingProtocolInputEvents>,
    mut window_state: ResMut<WinitWindowState>,
    completion_source: NonSendMut<WinitPresentCompletionSource>,
) {
    if selected_backend.kind == BackendKind::Winit {
        let mut shared = completion_source.shared.borrow_mut();
        if let Some(window_state) = shared.pending_window_state.take() {
            apply_window_state_to_outputs(outputs, window_state);
        }
        let pending_input_events = shared.pending_input_events.drain(..).collect::<Vec<_>>();
        pending_backend_inputs.items.extend(pending_input_events.clone());
        pending_protocol_inputs.items.extend(pending_input_events);

        window_state.driver = match shared.driver {
            WinitPresentDriver::SmithayEventLoop => "smithay-event-loop".to_owned(),
            WinitPresentDriver::TimerFallback => "timer-fallback".to_owned(),
        };
        window_state.closed = shared.closed;
        if let Some(backend) = shared.backend.as_ref() {
            let size = backend.window_size();
            window_state.actual_width = Some(u32::try_from(size.w.max(1)).unwrap_or(1));
            window_state.actual_height = Some(u32::try_from(size.h.max(1)).unwrap_or(1));
            window_state.actual_scale =
                Some(backend.window().scale_factor().round().clamp(1.0, u32::MAX as f64) as u32);
        } else {
            window_state.actual_width = None;
            window_state.actual_height = None;
            window_state.actual_scale = None;
        }

        selected_backend.description = match shared.driver {
            WinitPresentDriver::SmithayEventLoop => "nested winit development backend".to_owned(),
            WinitPresentDriver::TimerFallback => {
                "nested winit development backend (timer fallback)".to_owned()
            }
        };
    }
}

pub(crate) fn sync_winit_window_system(
    selected_backend: Res<crate::traits::SelectedBackend>,
    app_metadata: Res<AppMetadata>,
    config: Option<Res<CompositorConfig>>,
    outputs: Query<(&OutputDevice, &OutputProperties)>,
    mut window_state: ResMut<WinitWindowState>,
    completion_source: NonSendMut<WinitPresentCompletionSource>,
) {
    if selected_backend.kind != BackendKind::Winit {
        return;
    }

    let desired_spec = desired_window_spec(Some(&app_metadata), config.as_deref(), Some(&outputs));
    window_state.title = desired_spec.title.clone();
    window_state.requested_width = desired_spec.width;
    window_state.requested_height = desired_spec.height;

    let mut shared = completion_source.shared.borrow_mut();
    if shared.desired_window_spec == desired_spec {
        return;
    }

    shared.desired_window_spec = desired_spec.clone();
    if let Some(backend) = shared.backend.as_ref() {
        apply_window_spec(backend.window(), &desired_spec);
    }
}

pub(crate) fn winit_present_completion_system(
    selected_backend: Res<crate::traits::SelectedBackend>,
    outputs: Query<(&OutputDevice, &OutputProperties)>,
    mut pending_presentation_events: bevy_ecs::prelude::ResMut<PendingOutputPresentationEvents>,
    mut presentation_runtime: Local<OutputPresentationRuntime>,
    completion_source: NonSendMut<WinitPresentCompletionSource>,
) {
    let mut shared = completion_source.shared.borrow_mut();
    shared.active =
        selected_backend.kind == BackendKind::Winit && !outputs.is_empty() && !shared.closed;
    if let Some(refresh_interval) = current_refresh_interval(&outputs) {
        shared.refresh_interval = refresh_interval;
    }

    let pending_timestamps_nanos = shared.pending_timestamps_nanos.drain(..).collect::<Vec<_>>();
    drop(shared);

    for present_time_nanos in pending_timestamps_nanos {
        emit_present_completion_events_at(
            BackendKind::Winit,
            &selected_backend,
            &outputs,
            &mut pending_presentation_events,
            &mut presentation_runtime,
            present_time_nanos,
        );
    }
}

pub(crate) fn winit_render_system(
    selected_backend: Res<crate::traits::SelectedBackend>,
    config: Option<Res<CompositorConfig>>,
    outputs: Query<(&OutputDevice, &OutputProperties)>,
    surfaces: Query<(&WlSurfaceHandle, &SurfaceGeometry)>,
    render_list: Res<RenderList>,
    surface_registry: Option<NonSend<ProtocolSurfaceRegistry>>,
    completion_source: NonSendMut<WinitPresentCompletionSource>,
    mut render_state: Local<WinitRenderState>,
) {
    if selected_backend.kind != BackendKind::Winit {
        return;
    }

    let Some((_, output)) = outputs.iter().next() else {
        return;
    };
    let Some(surface_registry) = surface_registry else {
        return;
    };

    let mode = WinitOutputMode {
        width: output.width.max(1),
        height: output.height.max(1),
        scale: output.scale.max(1),
    };
    if render_state.mode != Some(mode) {
        render_state.damage_tracker = Some(OutputDamageTracker::new(
            (mode.width as i32, mode.height as i32),
            mode.scale as f64,
            winit_output_transform(),
        ));
        render_state.mode = Some(mode);
    }

    let geometry_by_surface = surfaces
        .iter()
        .map(|(surface, geometry)| (surface.id, geometry.clone()))
        .collect::<HashMap<_, _>>();

    let mut shared = completion_source.shared.borrow_mut();
    if shared.closed {
        return;
    }
    let Some(backend) = shared.backend.as_mut() else {
        return;
    };

    let damage = {
        let Some(damage_tracker) = render_state.damage_tracker.as_mut() else {
            return;
        };

        let (renderer, mut framebuffer) = match backend.bind() {
            Ok(bound) => bound,
            Err(error) => {
                tracing::warn!(error = %error, "failed to bind winit renderer framebuffer");
                return;
            }
        };
        // Smithay's winit backend documents buffer_age as meaningful only after a successful bind.
        // For now we trade damage-age optimization for stability and repaint the full output.
        let age = 0;

        let mut elements = Vec::<WaylandSurfaceRenderElement<GlesRenderer>>::new();
        for render_element in &render_list.elements {
            if render_element.surface_id == 0 {
                continue;
            }

            let Some(surface) = surface_registry.surface(render_element.surface_id) else {
                continue;
            };
            let Some(geometry) = geometry_by_surface.get(&render_element.surface_id) else {
                continue;
            };

            elements.extend(render_elements_from_surface_tree(
                renderer,
                surface,
                (geometry.x, geometry.y),
                mode.scale as f64,
                render_element.opacity,
                Kind::Unspecified,
            ));
        }

        match damage_tracker.render_output(
            renderer,
            &mut framebuffer,
            age,
            &elements,
            clear_color(config.as_deref()),
        ) {
            Ok(result) => result.damage.cloned(),
            Err(error) => {
                tracing::warn!(error = %error, "failed to render wayland surfaces into winit backend");
                None
            }
        }
    };

    let Some(damage) = damage else {
        return;
    };

    if let Err(error) = backend.submit(Some(damage.as_slice())) {
        tracing::warn!(error = %error, "failed to submit winit backbuffer");
    }
}

fn current_refresh_interval(
    outputs: &Query<(&OutputDevice, &OutputProperties)>,
) -> Option<Duration> {
    outputs
        .iter()
        .map(|(_, properties)| properties.refresh_millihz)
        .filter(|refresh_millihz| *refresh_millihz > 0)
        .max()
        .map(|refresh_millihz| {
            Duration::from_nanos((1_000_000_000_000_u64 / u64::from(refresh_millihz)).max(1))
        })
}

fn monotonic_now_nanos(monotonic_clock: &smithay::utils::Clock<Monotonic>) -> u64 {
    let now = std::time::Duration::from(monotonic_clock.now());
    now.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn apply_window_state_to_outputs(
    mut outputs: Query<(&OutputDevice, &mut OutputProperties)>,
    window_state: PendingWinitWindowState,
) {
    let width = u32::try_from(window_state.size.w.max(1)).unwrap_or(1);
    let height = u32::try_from(window_state.size.h.max(1)).unwrap_or(1);
    let scale = window_state.scale_factor.round().clamp(1.0, u32::MAX as f64) as u32;

    for (_, mut properties) in &mut outputs {
        properties.width = width;
        properties.height = height;
        properties.scale = scale;
    }
}

fn desired_window_spec(
    app_metadata: Option<&AppMetadata>,
    config: Option<&CompositorConfig>,
    outputs: Option<&Query<(&OutputDevice, &OutputProperties)>>,
) -> WinitWindowSpec {
    let app_name = app_metadata.map(|metadata| metadata.name.as_str()).unwrap_or("nekoland");
    if let Some(outputs) = outputs {
        if let Some((output, properties)) =
            outputs.iter().min_by(|(left, _), (right, _)| left.name.cmp(&right.name))
        {
            return WinitWindowSpec {
                title: format!(
                    "{app_name} [winit] - {} {}x{}",
                    output.name, properties.width, properties.height
                ),
                width: properties.width.max(1),
                height: properties.height.max(1),
            };
        }
    }

    let configured_output = config
        .and_then(|config| config.outputs.iter().find(|output| output.enabled))
        .and_then(|output| {
            parse_output_mode(&output.mode).map(|mode| {
                (output.name.as_str(), mode.width.max(1), mode.height.max(1), output.scale.max(1))
            })
        });

    let (output_name, width, height, scale) = configured_output.unwrap_or(("winit", 1280, 720, 1));
    let title_suffix = if config.is_some() { output_name } else { "bootstrap" };

    WinitWindowSpec {
        title: format!("{app_name} [winit] - {title_suffix} {width}x{height}@{scale}x"),
        width,
        height,
    }
}

fn apply_window_spec(window: &HostWindow, spec: &WinitWindowSpec) {
    window.set_title(&spec.title);
    let _ = window.request_inner_size(PhysicalSize::new(spec.width, spec.height));
    window.request_redraw();
}

fn clear_color(config: Option<&CompositorConfig>) -> Color32F {
    config
        .and_then(|config| parse_hex_color32f(&config.background_color))
        .unwrap_or(Color32F::BLACK)
}

fn winit_output_transform() -> Transform {
    // Smithay's nested winit path renders into an OpenGL-backed framebuffer whose native
    // orientation is flipped relative to Wayland's top-left surface coordinates.
    Transform::Flipped180
}

fn parse_hex_color32f(color: &str) -> Option<Color32F> {
    let hex = color.strip_prefix('#')?;
    let channels = match hex.len() {
        6 => {
            let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
            [red, green, blue, u8::MAX]
        }
        8 => {
            let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let alpha = u8::from_str_radix(&hex[6..8], 16).ok()?;
            [red, green, blue, alpha]
        }
        _ => return None,
    };

    Some(Color32F::new(
        f32::from(channels[0]) / 255.0,
        f32::from(channels[1]) / 255.0,
        f32::from(channels[2]) / 255.0,
        f32::from(channels[3]) / 255.0,
    ))
}

fn install_smithay_winit_source(
    handle: &calloop::LoopHandle<'_, ()>,
    shared: Rc<RefCell<WinitPresentCompletionShared>>,
) -> Result<(), NekolandError> {
    let desired_spec = shared.borrow().desired_window_spec.clone();
    let window_attributes = HostWindow::default_attributes()
        .with_inner_size(PhysicalSize::new(desired_spec.width, desired_spec.height))
        .with_title(desired_spec.title.clone())
        .with_visible(true);
    let (backend, event_loop) = match std::panic::catch_unwind(AssertUnwindSafe(|| {
        smithay_winit::init_from_attributes::<GlesRenderer>(window_attributes)
    })) {
        Ok(Ok(pair)) => pair,
        Ok(Err(error)) => return Err(NekolandError::Runtime(error.to_string())),
        Err(_) => {
            return Err(NekolandError::Runtime(
                "winit event loop initialization panicked".to_owned(),
            ));
        }
    };
    let monotonic_clock = smithay::utils::Clock::<Monotonic>::new();

    {
        let mut shared = shared.borrow_mut();
        shared.driver = WinitPresentDriver::SmithayEventLoop;
        shared.closed = false;
        shared.pending_window_state = Some(PendingWinitWindowState {
            size: backend.window_size(),
            scale_factor: backend.scale_factor(),
        });
        shared.backend = Some(backend);
    }
    {
        let shared = shared.borrow();
        if let Some(backend) = shared.backend.as_ref() {
            apply_window_spec(backend.window(), &shared.desired_window_spec);
        }
    }

    handle
        .insert_source(event_loop, move |event, _, _| match event {
            WinitEvent::Redraw => {
                let mut shared = shared.borrow_mut();
                if !shared.closed {
                    if shared.active {
                        shared.pending_timestamps_nanos.push(monotonic_now_nanos(&monotonic_clock));
                    }
                    if let Some(backend) = shared.backend.as_ref() {
                        backend.window().request_redraw();
                    }
                }
            }
            WinitEvent::Resized { size, scale_factor } => {
                let mut shared = shared.borrow_mut();
                shared.pending_window_state = Some(PendingWinitWindowState { size, scale_factor });
            }
            WinitEvent::Input(input_event) => {
                if let Some(event) = translate_winit_input_event(input_event) {
                    shared.borrow_mut().pending_input_events.push(event);
                }
            }
            WinitEvent::Focus(focused) => {
                shared.borrow_mut().pending_input_events.push(BackendInputEvent {
                    device: "winit".to_owned(),
                    action: BackendInputAction::FocusChanged { focused },
                });
            }
            WinitEvent::CloseRequested => {
                let mut shared = shared.borrow_mut();
                shared.closed = true;
            }
        })
        .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;

    Ok(())
}

fn install_timer_source(
    handle: &calloop::LoopHandle<'_, ()>,
    shared: Rc<RefCell<WinitPresentCompletionShared>>,
) -> Result<(), NekolandError> {
    let monotonic_clock = smithay::utils::Clock::<Monotonic>::new();
    {
        let mut shared = shared.borrow_mut();
        shared.driver = WinitPresentDriver::TimerFallback;
        shared.closed = false;
        shared.backend = None;
    }

    handle
        .insert_source(Timer::immediate(), move |_, _, _| {
            let mut shared = shared.borrow_mut();
            if shared.active {
                shared.pending_timestamps_nanos.push(monotonic_now_nanos(&monotonic_clock));
                TimeoutAction::ToDuration(shared.refresh_interval.max(MIN_PRESENT_INTERVAL))
            } else {
                TimeoutAction::ToDuration(INACTIVE_PRESENT_POLL_INTERVAL)
            }
        })
        .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;

    Ok(())
}

fn requested_backend_kind() -> BackendKind {
    match std::env::var("NEKOLAND_BACKEND").unwrap_or_else(|_| "winit".to_owned()).as_str() {
        "drm" => BackendKind::Drm,
        "virtual" | "headless" | "offscreen" => BackendKind::Virtual,
        "winit" | "x11" => BackendKind::Winit,
        _ => BackendKind::Winit,
    }
}

fn translate_winit_input_event(
    input_event: InputEvent<smithay_winit::WinitInput>,
) -> Option<BackendInputEvent> {
    let action = match input_event {
        InputEvent::Keyboard { event } => BackendInputAction::Key {
            keycode: event.key_code().into(),
            pressed: event.state() == KeyState::Pressed,
        },
        InputEvent::PointerMotionAbsolute { event } => {
            BackendInputAction::PointerMoved { x: event.x(), y: event.y() }
        }
        InputEvent::PointerButton { event } => BackendInputAction::PointerButton {
            button_code: event.button_code(),
            pressed: event.state() == ButtonState::Pressed,
        },
        InputEvent::PointerAxis { event } => BackendInputAction::PointerAxis {
            horizontal: event
                .amount(Axis::Horizontal)
                .or_else(|| event.amount_v120(Axis::Horizontal))
                .unwrap_or(0.0),
            vertical: event
                .amount(Axis::Vertical)
                .or_else(|| event.amount_v120(Axis::Vertical))
                .unwrap_or(0.0),
        },
        _ => return None,
    };

    Some(BackendInputEvent { device: "winit".to_owned(), action })
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::AppMetadata;
    use nekoland_ecs::resources::{CompositorConfig, ConfiguredOutput};
    use smithay::utils::Transform;

    use super::{desired_window_spec, parse_hex_color32f, winit_output_transform};

    #[test]
    fn parses_hex_clear_color() {
        let color = parse_hex_color32f("#f5f7ff").expect("hex color should parse");
        assert!((color.r() - 245.0 / 255.0).abs() < 0.0001);
        assert!((color.g() - 247.0 / 255.0).abs() < 0.0001);
        assert!((color.b() - 1.0).abs() < 0.0001);
        assert!((color.a() - 1.0).abs() < 0.0001);
    }

    #[test]
    fn desired_window_spec_uses_configured_output_and_app_name() {
        let metadata = AppMetadata { name: "nekoland".to_owned() };
        let mut config = CompositorConfig::default();
        config.outputs = vec![ConfiguredOutput {
            name: "HDMI-A-1".to_owned(),
            mode: "1600x900@75".to_owned(),
            scale: 2,
            enabled: true,
        }];

        let spec = desired_window_spec(Some(&metadata), Some(&config), None);

        assert_eq!(spec.width, 1600);
        assert_eq!(spec.height, 900);
        assert_eq!(spec.title, "nekoland [winit] - HDMI-A-1 1600x900@2x");
    }

    #[test]
    fn nested_winit_rendering_uses_flipped_transform() {
        assert_eq!(winit_output_transform(), Transform::Flipped180);
    }
}
