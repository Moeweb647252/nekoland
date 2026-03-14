use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;
use std::time::Duration;

use bevy_app::App;
use bevy_ecs::prelude::Resource;
use calloop::timer::{TimeoutAction, Timer};
use nekoland_core::calloop::CalloopSourceRegistry;
use nekoland_core::error::NekolandError;
use nekoland_core::prelude::AppMetadata;
use nekoland_ecs::components::{OutputDevice, OutputKind, OutputProperties};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, CompositorConfig, DamageRect, OutputDamageRegions,
};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent,
};
use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self as smithay_winit, WinitEvent, WinitGraphicsBackend};
use smithay::reexports::winit::dpi::PhysicalSize;
use smithay::reexports::winit::window::{CursorGrabMode, Window as HostWindow};
use smithay::render_elements;
use smithay::utils::{Monotonic, Physical, Rectangle, Size, Transform};

use crate::common::cursor::{SoftwareCursorCache, cursor_position_on_output, cursor_render_source};
use crate::common::outputs::{
    BackendOutputBlueprint, BackendOutputChange, BackendOutputEventRecord,
    BackendOutputPropertyUpdate, parse_output_mode,
};
use crate::common::presentation::{OutputPresentationRuntime, emit_present_completion_events_at};
use crate::common::render_order::output_surfaces_in_presentation_order;
use crate::traits::{
    Backend, BackendApplyCtx, BackendCapabilities, BackendDescriptor, BackendExtractCtx, BackendId,
    BackendKind, BackendPresentCtx, BackendRole, OutputSnapshot,
};

type WinitRendererBackend = WinitGraphicsBackend<GlesRenderer>;

render_elements! {
    WinitRenderElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
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
struct WinitRenderState {
    damage_tracker: Option<OutputDamageTracker>,
    mode: Option<WinitOutputMode>,
    cursor: SoftwareCursorCache,
}

pub(crate) struct WinitRuntime {
    descriptor: BackendDescriptor,
    shared: Rc<RefCell<WinitPresentCompletionShared>>,
    render_state: WinitRenderState,
    presentation_runtime: OutputPresentationRuntime,
    seeded_output_name: Option<String>,
}

const INACTIVE_PRESENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const MIN_PRESENT_INTERVAL: Duration = Duration::from_micros(500);

impl WinitRuntime {
    pub fn install(app: &mut App, id: BackendId) -> Self {
        if app.world().get_non_send_resource::<CalloopSourceRegistry>().is_none() {
            app.insert_non_send_resource(CalloopSourceRegistry::default());
        }

        let initial_window_spec = desired_window_spec(
            app.world().get_resource::<AppMetadata>(),
            app.world().get_resource::<CompositorConfig>(),
            &[],
        );
        let shared = Rc::new(RefCell::new(WinitPresentCompletionShared::default()));
        shared.borrow_mut().desired_window_spec = initial_window_spec.clone();

        let mut registry = app
            .world_mut()
            .get_non_send_resource_mut::<CalloopSourceRegistry>()
            .expect("calloop registry inserted immediately before access");
        let source_shared = shared.clone();
        registry.push(move |handle| {
            match install_smithay_winit_source(handle, source_shared.clone()) {
                Ok(()) => Ok(()),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to initialize smithay winit event source; falling back to timer"
                    );
                    install_timer_source(handle, source_shared.clone())
                }
            }
        });
        drop(registry);

        app.insert_resource(WinitWindowState {
            title: initial_window_spec.title.clone(),
            requested_width: initial_window_spec.width,
            requested_height: initial_window_spec.height,
            ..WinitWindowState::default()
        });

        Self {
            descriptor: BackendDescriptor {
                id,
                kind: BackendKind::Winit,
                role: BackendRole::PrimaryDisplay,
                label: format!("winit-{}", id.0),
                description: "nested winit development backend".to_owned(),
            },
            shared,
            render_state: WinitRenderState::default(),
            presentation_runtime: OutputPresentationRuntime::default(),
            seeded_output_name: None,
        }
    }

    fn owned_outputs<'a>(
        &'a self,
        outputs: &'a [OutputSnapshot],
    ) -> impl Iterator<Item = &'a OutputSnapshot> {
        outputs.iter().filter(|output| output.backend_id == Some(self.id()))
    }

    fn desired_output_name(&self, config: Option<&CompositorConfig>) -> String {
        config
            .and_then(|config| config.outputs.iter().find(|output| output.enabled))
            .map(|output| output.name.clone())
            .unwrap_or_else(|| "Winit-1".to_owned())
    }
}

impl Backend for WinitRuntime {
    fn id(&self) -> BackendId {
        self.descriptor.id
    }

    fn descriptor(&self) -> BackendDescriptor {
        self.descriptor.clone()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::INPUT
            | BackendCapabilities::OUTPUT_DISCOVERY
            | BackendCapabilities::OUTPUT_CONFIGURATION
            | BackendCapabilities::PRESENT
            | BackendCapabilities::PRESENT_TIMELINE
    }

    fn seed_output(&self, output_name: &str) -> Option<BackendOutputBlueprint> {
        Some(BackendOutputBlueprint {
            device: OutputDevice {
                name: output_name.to_owned(),
                kind: OutputKind::Nested,
                make: "Winit".to_owned(),
                model: self.descriptor.description.clone(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
        })
    }

    fn extract(&mut self, cx: &mut BackendExtractCtx<'_>) -> Result<(), NekolandError> {
        let desired_output_name = self.desired_output_name(cx.config);
        let owned_outputs = self.owned_outputs(cx.outputs).cloned().collect::<Vec<_>>();
        if owned_outputs.is_empty()
            && self.seeded_output_name.as_deref() != Some(desired_output_name.as_str())
        {
            if let Some(blueprint) = self.seed_output(&desired_output_name) {
                cx.output_events.push(BackendOutputEventRecord {
                    backend_id: self.id(),
                    output_name: desired_output_name.clone(),
                    change: BackendOutputChange::Connected(blueprint),
                });
                self.seeded_output_name = Some(desired_output_name);
            }
        }

        let mut shared = self.shared.borrow_mut();
        if let Some(window_state) = shared.pending_window_state.take() {
            let width = u32::try_from(window_state.size.w.max(1)).unwrap_or(1);
            let height = u32::try_from(window_state.size.h.max(1)).unwrap_or(1);
            let scale = window_state.scale_factor.round().clamp(1.0, u32::MAX as f64) as u32;
            for output in &owned_outputs {
                cx.output_updates.push(BackendOutputPropertyUpdate {
                    backend_id: self.id(),
                    output_name: output.device.name.clone(),
                    properties: OutputProperties {
                        width,
                        height,
                        refresh_millihz: output.properties.refresh_millihz,
                        scale,
                    },
                });
            }
        }

        let pending_input_events = shared.pending_input_events.drain(..).collect::<Vec<_>>();
        cx.backend_input_events.extend(pending_input_events.iter().cloned());
        cx.protocol_input_events.extend(pending_input_events);

        if let Some(window_state) = cx.winit_window_state.as_mut() {
            let window_state = &mut **window_state;
            window_state.driver = match shared.driver {
                WinitPresentDriver::SmithayEventLoop => "smithay-event-loop".to_owned(),
                WinitPresentDriver::TimerFallback => "timer-fallback".to_owned(),
            };
            window_state.closed = shared.closed;
            if let Some(backend) = shared.backend.as_ref() {
                let size = backend.window_size();
                window_state.actual_width = Some(u32::try_from(size.w.max(1)).unwrap_or(1));
                window_state.actual_height = Some(u32::try_from(size.h.max(1)).unwrap_or(1));
                window_state.actual_scale = Some(
                    backend.window().scale_factor().round().clamp(1.0, u32::MAX as f64) as u32,
                );
            } else {
                window_state.actual_width = None;
                window_state.actual_height = None;
                window_state.actual_scale = None;
            }
        }

        self.descriptor.description = match shared.driver {
            WinitPresentDriver::SmithayEventLoop => "nested winit development backend".to_owned(),
            WinitPresentDriver::TimerFallback => {
                "nested winit development backend (timer fallback)".to_owned()
            }
        };

        shared.active = !owned_outputs.is_empty() && !shared.closed;
        if let Some(refresh_interval) = current_refresh_interval(&owned_outputs) {
            shared.refresh_interval = refresh_interval;
        }
        let pending_timestamps_nanos =
            shared.pending_timestamps_nanos.drain(..).collect::<Vec<_>>();
        drop(shared);

        for present_time_nanos in pending_timestamps_nanos {
            emit_present_completion_events_at(
                owned_outputs
                    .iter()
                    .map(|output| (output.device.name.clone(), output.properties.clone())),
                cx.presentation_events,
                &mut self.presentation_runtime,
                present_time_nanos,
            );
        }

        Ok(())
    }

    fn apply(&mut self, cx: &mut BackendApplyCtx<'_>) -> Result<(), NekolandError> {
        let owned_outputs = self.owned_outputs(cx.outputs).cloned().collect::<Vec<_>>();
        let desired_spec = desired_window_spec(cx.app_metadata, cx.config, &owned_outputs);
        if let Some(window_state) = cx.winit_window_state.as_mut() {
            let window_state = &mut **window_state;
            window_state.title = desired_spec.title.clone();
            window_state.requested_width = desired_spec.width;
            window_state.requested_height = desired_spec.height;
        }

        let mut shared = self.shared.borrow_mut();
        if shared.desired_window_spec == desired_spec {
            return Ok(());
        }

        shared.desired_window_spec = desired_spec.clone();
        if let Some(backend) = shared.backend.as_ref() {
            apply_window_spec(backend.window(), &desired_spec);
        }
        Ok(())
    }

    fn present(&mut self, cx: &mut BackendPresentCtx<'_>) -> Result<(), NekolandError> {
        let owned_outputs = self.owned_outputs(cx.outputs).cloned().collect::<Vec<_>>();
        let Some(output) = owned_outputs.first() else {
            return Ok(());
        };
        let Some(surface_registry) = cx.surface_registry else {
            return Ok(());
        };

        let mode = WinitOutputMode {
            width: output.properties.width.max(1),
            height: output.properties.height.max(1),
            scale: output.properties.scale.max(1),
        };
        if self.render_state.mode != Some(mode) {
            self.render_state.damage_tracker = Some(OutputDamageTracker::new(
                (mode.width as i32, mode.height as i32),
                mode.scale as f64,
                winit_output_transform(),
            ));
            self.render_state.mode = Some(mode);
        }

        let mut shared = self.shared.borrow_mut();
        if shared.closed {
            return Ok(());
        }
        let Some(backend) = shared.backend.as_mut() else {
            return Ok(());
        };

        let damage = {
            let Some(damage_tracker) = self.render_state.damage_tracker.as_mut() else {
                return Ok(());
            };
            let cursor_cache = &mut self.render_state.cursor;

            let (renderer, mut framebuffer) = match backend.bind() {
                Ok(bound) => bound,
                Err(error) => {
                    tracing::warn!(error = %error, "failed to bind winit renderer framebuffer");
                    return Ok(());
                }
            };
            let age = 0;

            let mut cursor_elements = Vec::<WinitRenderElement>::new();

            if let Some((cursor_x, cursor_y)) =
                cursor_position_on_output(cx.cursor_render, &output.device.name)
            {
                match cursor_render_source(cx.cursor_image) {
                    crate::common::cursor::CursorRenderSource::Hidden => {}
                    crate::common::cursor::CursorRenderSource::Surface {
                        surface,
                        hotspot_x,
                        hotspot_y,
                    } => {
                        cursor_elements.extend(render_elements_from_surface_tree(
                            renderer,
                            surface,
                            (
                                cursor_x.round() as i32 - hotspot_x,
                                cursor_y.round() as i32 - hotspot_y,
                            ),
                            mode.scale as f64,
                            1.0,
                            Kind::Cursor,
                        ));
                    }
                    crate::common::cursor::CursorRenderSource::Named(icon) => {
                        let theme = cx
                            .config
                            .map(|config| config.cursor_theme.as_str())
                            .unwrap_or("default");
                        match cursor_cache.render_element(
                            renderer,
                            theme,
                            icon,
                            mode.scale.max(1),
                            cursor_x,
                            cursor_y,
                        ) {
                            Ok(element) => cursor_elements.push(element.into()),
                            Err(error) => {
                                tracing::warn!(error = %error, "failed to upload software cursor");
                            }
                        }
                    }
                }
            }

            let mut elements = cursor_elements;
            for (render_element, geometry) in output_surfaces_in_presentation_order(
                cx.render_list,
                cx.surfaces,
                &output.device.name,
            ) {
                let Some(surface) = surface_registry.surface(render_element.surface_id) else {
                    continue;
                };
                elements.extend(render_elements_from_surface_tree(
                    renderer,
                    surface,
                    (geometry.geometry.x, geometry.geometry.y),
                    mode.scale as f64,
                    render_element.opacity,
                    Kind::Unspecified,
                ));
            }

            let smithay_damage = match damage_tracker.render_output(
                renderer,
                &mut framebuffer,
                age,
                &elements,
                clear_color(cx.config),
            ) {
                Ok(result) => result.damage.cloned(),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to render wayland surfaces into winit backend"
                    );
                    None
                }
            };
            merge_submit_damage(
                smithay_damage,
                output_damage_regions_physical(
                    cx.output_damage_regions,
                    &output.device.name,
                    mode.scale,
                ),
            )
        };

        let Some(damage) = damage else {
            return Ok(());
        };

        if let Err(error) = backend.submit(Some(damage.as_slice())) {
            tracing::warn!(error = %error, "failed to submit winit backbuffer");
        }

        Ok(())
    }
}

fn current_refresh_interval(outputs: &[OutputSnapshot]) -> Option<Duration> {
    outputs
        .iter()
        .map(|output| output.properties.refresh_millihz)
        .filter(|refresh_millihz| *refresh_millihz > 0)
        .max()
        .map(|refresh_millihz| {
            Duration::from_nanos((1_000_000_000_000_u64 / u64::from(refresh_millihz)).max(1))
        })
}

fn output_damage_regions_physical(
    output_damage_regions: &OutputDamageRegions,
    output_name: &str,
    scale: u32,
) -> Vec<Rectangle<i32, Physical>> {
    output_damage_regions
        .regions
        .get(output_name)
        .into_iter()
        .flatten()
        .filter_map(|rect| damage_rect_to_physical(rect, scale))
        .collect()
}

fn damage_rect_to_physical(rect: &DamageRect, scale: u32) -> Option<Rectangle<i32, Physical>> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }

    let scale = i64::from(scale.max(1));
    let x = (i64::from(rect.x) * scale).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let y = (i64::from(rect.y) * scale).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let width =
        (u64::from(rect.width) * u64::try_from(scale).ok()?).min(u64::from(i32::MAX as u32)) as i32;
    let height = (u64::from(rect.height) * u64::try_from(scale).ok()?)
        .min(u64::from(i32::MAX as u32)) as i32;

    Some(Rectangle::new((x, y).into(), (width, height).into()))
}

fn merge_submit_damage(
    smithay_damage: Option<Vec<Rectangle<i32, Physical>>>,
    ecs_damage: Vec<Rectangle<i32, Physical>>,
) -> Option<Vec<Rectangle<i32, Physical>>> {
    match (smithay_damage, ecs_damage.is_empty()) {
        (None, true) => None,
        (Some(damage), true) => Some(damage),
        (None, false) => Some(ecs_damage),
        (Some(mut damage), false) => {
            for rect in ecs_damage {
                if !damage.contains(&rect) {
                    damage.push(rect);
                }
            }
            Some(damage)
        }
    }
}

fn desired_window_spec(
    app_metadata: Option<&AppMetadata>,
    config: Option<&CompositorConfig>,
    outputs: &[OutputSnapshot],
) -> WinitWindowSpec {
    let app_name = app_metadata.map(|metadata| metadata.name.as_str()).unwrap_or("nekoland");
    if let Some((output, properties)) = outputs
        .iter()
        .min_by(|left, right| left.device.name.cmp(&right.device.name))
        .map(|output| (&output.device, &output.properties))
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

fn set_host_cursor_capture(window: &HostWindow, capture: bool) {
    if capture {
        let grab_result = preferred_cursor_grab_modes()
            .into_iter()
            .find_map(|mode| window.set_cursor_grab(mode).ok().map(|()| mode));
        match grab_result {
            Some(mode) => {
                window.set_cursor_visible(false);
                tracing::debug!(?mode, "captured winit host cursor");
            }
            None => {
                window.set_cursor_visible(true);
                tracing::warn!("failed to capture winit host cursor");
            }
        }
    } else {
        if let Err(error) = window.set_cursor_grab(CursorGrabMode::None) {
            tracing::debug!(error = %error, "failed to release winit host cursor grab");
        }
        window.set_cursor_visible(true);
    }
}

fn preferred_cursor_grab_modes() -> [CursorGrabMode; 2] {
    [CursorGrabMode::Confined, CursorGrabMode::Locked]
}

fn clear_color(config: Option<&CompositorConfig>) -> Color32F {
    config
        .and_then(|config| parse_hex_color32f(&config.background_color))
        .unwrap_or(Color32F::BLACK)
}

fn winit_output_transform() -> Transform {
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
            set_host_cursor_capture(backend.window(), true);
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
                shared.borrow_mut().pending_window_state =
                    Some(PendingWinitWindowState { size, scale_factor });
            }
            WinitEvent::Input(input_event) => {
                if let Some(event) = translate_winit_input_event(input_event) {
                    shared.borrow_mut().pending_input_events.push(event);
                }
            }
            WinitEvent::Focus(focused) => {
                let mut shared = shared.borrow_mut();
                if let Some(backend) = shared.backend.as_ref() {
                    set_host_cursor_capture(backend.window(), focused);
                }
                shared.pending_input_events.push(BackendInputEvent {
                    device: "winit".to_owned(),
                    action: BackendInputAction::FocusChanged { focused },
                });
            }
            WinitEvent::CloseRequested => {
                shared.borrow_mut().closed = true;
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

fn monotonic_now_nanos(monotonic_clock: &smithay::utils::Clock<Monotonic>) -> u64 {
    let now = std::time::Duration::from(monotonic_clock.now());
    now.as_nanos().min(u128::from(u64::MAX)) as u64
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
    use nekoland_ecs::components::{OutputDevice, OutputKind, OutputProperties};
    use nekoland_ecs::resources::{
        CompositorConfig, ConfiguredOutput, DamageRect, OutputDamageRegions,
    };
    use smithay::reexports::winit::window::CursorGrabMode;
    use smithay::utils::Transform;

    use crate::traits::{BackendId, OutputSnapshot};

    use super::{
        desired_window_spec, merge_submit_damage, output_damage_regions_physical,
        parse_hex_color32f, preferred_cursor_grab_modes, winit_output_transform,
    };

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

        let spec = desired_window_spec(Some(&metadata), Some(&config), &[]);

        assert_eq!(spec.width, 1600);
        assert_eq!(spec.height, 900);
        assert_eq!(spec.title, "nekoland [winit] - HDMI-A-1 1600x900@2x");
    }

    #[test]
    fn desired_window_spec_prefers_existing_owned_outputs() {
        let metadata = AppMetadata { name: "nekoland".to_owned() };
        let outputs = vec![OutputSnapshot {
            entity: bevy_ecs::entity::Entity::PLACEHOLDER,
            backend_id: Some(BackendId(1)),
            device: OutputDevice {
                name: "Winit-1".to_owned(),
                kind: OutputKind::Nested,
                make: "Winit".to_owned(),
                model: "test".to_owned(),
            },
            properties: OutputProperties {
                width: 800,
                height: 600,
                refresh_millihz: 60_000,
                scale: 1,
            },
        }];

        let spec = desired_window_spec(Some(&metadata), None, &outputs);
        assert_eq!(spec.width, 800);
        assert_eq!(spec.height, 600);
        assert_eq!(spec.title, "nekoland [winit] - Winit-1 800x600");
    }

    #[test]
    fn nested_winit_rendering_uses_flipped_transform() {
        assert_eq!(winit_output_transform(), Transform::Flipped180);
    }

    #[test]
    fn winit_cursor_grab_prefers_confined_before_locked() {
        assert_eq!(
            preferred_cursor_grab_modes(),
            [CursorGrabMode::Confined, CursorGrabMode::Locked]
        );
    }

    #[test]
    fn output_damage_regions_are_converted_to_physical_submit_damage() {
        let damage = OutputDamageRegions {
            regions: std::collections::BTreeMap::from([(
                "Virtual-1".to_owned(),
                vec![DamageRect { x: 10, y: -5, width: 30, height: 20 }],
            )]),
        };

        let physical = output_damage_regions_physical(&damage, "Virtual-1", 2);

        assert_eq!(physical.len(), 1);
        assert_eq!(physical[0].loc.x, 20);
        assert_eq!(physical[0].loc.y, -10);
        assert_eq!(physical[0].size.w, 60);
        assert_eq!(physical[0].size.h, 40);
    }

    #[test]
    fn merge_submit_damage_unions_ecs_and_smithay_damage() {
        let smithay_damage = vec![smithay::utils::Rectangle::new((0, 0).into(), (10, 10).into())];
        let ecs_damage = vec![
            smithay::utils::Rectangle::new((0, 0).into(), (10, 10).into()),
            smithay::utils::Rectangle::new((40, 20).into(), (5, 6).into()),
        ];

        let merged = merge_submit_damage(Some(smithay_damage), ecs_damage)
            .expect("merged damage should remain present");

        assert_eq!(merged.len(), 2);
        assert!(merged.contains(&smithay::utils::Rectangle::new((0, 0).into(), (10, 10).into())));
        assert!(merged.contains(&smithay::utils::Rectangle::new((40, 20).into(), (5, 6).into())));
    }
}
