//! Nested `winit` backend runtime used for the default development path.
//!
//! This backend owns host-window integration, event-loop bridging, and final submission of the
//! compiled output graph into a nested desktop window.

use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;
use std::time::Duration;

use bevy_app::App;
use bevy_ecs::prelude::Resource;
use calloop::timer::{TimeoutAction, Timer};
use nekoland_config::resources::CompositorConfig;
use nekoland_core::calloop::with_wayland_calloop_registry;
use nekoland_core::error::NekolandError;
use nekoland_core::prelude::AppMetadata;
use nekoland_ecs::components::{OutputDevice, OutputKind, OutputProperties};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, DamageRect, OutputDamageRegions, RenderRect,
};
use nekoland_protocol::ProtocolDmabufSupport;
use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::utils::draw_render_elements;
use smithay::backend::renderer::{Frame, Renderer};
use smithay::reexports::winit::dpi::PhysicalSize;
use smithay::reexports::winit::window::{CursorGrabMode, Window as HostWindow};
use smithay::utils::{Monotonic, Physical, Rectangle, Size, Transform};

use crate::common::cursor::SoftwareCursorCache;
use crate::common::gles_executor::{
    CommonGlesRenderElement, GlesExecutionState, execute_output_graph,
    final_output_texture_element, prepare_output_graph_process_shaders,
    prepare_output_graph_targets, prepare_output_material_bindings, prepare_output_surface_imports,
};
use crate::common::outputs::{
    BackendOutputBlueprint, BackendOutputChange, BackendOutputEventRecord,
    BackendOutputPropertyUpdate, parse_output_mode,
};
use crate::common::presentation::{OutputPresentationRuntime, emit_present_completion_events_at};
use crate::traits::{
    Backend, BackendApplyCtx, BackendCapabilities, BackendDescriptor, BackendExtractCtx, BackendId,
    BackendKind, BackendPresentCtx, BackendRole, OutputSnapshot,
};
use crate::winit::host::{
    HOST_WINIT_DEVICE, HostCaptureModeState, HostWinitEvent, HostWinitGraphicsBackend,
    init_host_winit,
};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum WinitPresentDriver {
    #[default]
    TimerFallback,
    HostEventLoop,
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
    backend: Option<HostWinitGraphicsBackend>,
    capture_mode: HostCaptureModeState,
    closed: bool,
    driver: WinitPresentDriver,
    desired_window_spec: WinitWindowSpec,
    window_ready_for_present: bool,
    waiting_for_window_ready_logged: bool,
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
            capture_mode: Rc::new(std::cell::Cell::new(None)),
            closed: false,
            driver: WinitPresentDriver::TimerFallback,
            desired_window_spec: WinitWindowSpec {
                title: "nekoland".to_owned(),
                width: 1280,
                height: 720,
            },
            window_ready_for_present: false,
            waiting_for_window_ready_logged: false,
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
    mode: Option<WinitOutputMode>,
    cursor: SoftwareCursorCache,
    execution: GlesExecutionState,
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
const WINIT_PRIMARY_OUTPUT_LOCAL_ID: &str = "primary";

impl WinitRuntime {
    pub fn install(app: &mut App, id: BackendId) -> Self {
        let initial_window_spec = desired_window_spec(
            app.world().get_resource::<AppMetadata>(),
            app.world().get_resource::<CompositorConfig>(),
            &[],
        );
        let shared = Rc::new(RefCell::new(WinitPresentCompletionShared::default()));
        shared.borrow_mut().desired_window_spec = initial_window_spec.clone();

        with_wayland_calloop_registry(app, |registry| {
            let source_shared = shared.clone();
            registry.push(move |handle| {
                match install_host_winit_source(handle, source_shared.clone()) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "failed to initialize nested winit event source; falling back to timer"
                        );
                        install_timer_source(handle, source_shared.clone())
                    }
                }
            });
        });

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

    fn seed_output_blueprint(
        &self,
        output_name: &str,
        pending_window_state: Option<&PendingWinitWindowState>,
        config: Option<&CompositorConfig>,
    ) -> BackendOutputBlueprint {
        let (width, height, scale) = pending_window_state
            .map(|window_state| {
                let width = u32::try_from(window_state.size.w.max(1)).unwrap_or(1);
                let height = u32::try_from(window_state.size.h.max(1)).unwrap_or(1);
                let scale = window_state.scale_factor.round().clamp(1.0, u32::MAX as f64) as u32;
                (width, height, scale)
            })
            .or_else(|| {
                config
                    .and_then(|config| {
                        config
                            .outputs
                            .iter()
                            .find(|configured| configured.enabled && configured.name == output_name)
                    })
                    .and_then(|configured| {
                        parse_output_mode(&configured.mode).map(|mode| {
                            (mode.width.max(1), mode.height.max(1), configured.scale.max(1))
                        })
                    })
            })
            .unwrap_or((1280, 720, 1));

        BackendOutputBlueprint {
            local_id: WINIT_PRIMARY_OUTPUT_LOCAL_ID.to_owned(),
            device: OutputDevice {
                name: output_name.to_owned(),
                kind: OutputKind::Nested,
                make: "Winit".to_owned(),
                model: self.descriptor.description.clone(),
            },
            properties: OutputProperties { width, height, refresh_millihz: 60_000, scale },
        }
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
        Some(self.seed_output_blueprint(output_name, None, None))
    }

    fn extract(&mut self, cx: &mut BackendExtractCtx<'_>) -> Result<(), NekolandError> {
        let desired_output_name = self.desired_output_name(cx.config);
        let owned_outputs = self.owned_outputs(cx.outputs).cloned().collect::<Vec<_>>();
        let mut shared = self.shared.borrow_mut();
        if owned_outputs.is_empty()
            && self.seeded_output_name.as_deref() != Some(desired_output_name.as_str())
        {
            let blueprint = self.seed_output_blueprint(
                &desired_output_name,
                shared.pending_window_state.as_ref(),
                cx.config,
            );
            cx.output_events.push(BackendOutputEventRecord {
                backend_id: self.id(),
                output_name: desired_output_name.clone(),
                local_id: blueprint.local_id.clone(),
                change: BackendOutputChange::Connected(blueprint),
            });
            self.seeded_output_name = Some(desired_output_name);
        }

        if !owned_outputs.is_empty()
            && let Some(window_state) = shared.pending_window_state.take()
        {
            let width = u32::try_from(window_state.size.w.max(1)).unwrap_or(1);
            let height = u32::try_from(window_state.size.h.max(1)).unwrap_or(1);
            let scale = window_state.scale_factor.round().clamp(1.0, u32::MAX as f64) as u32;
            for output in &owned_outputs {
                cx.output_updates.push(BackendOutputPropertyUpdate {
                    backend_id: self.id(),
                    output_name: output.device.name.clone(),
                    local_id: output
                        .backend_output_id
                        .as_ref()
                        .map(|output_id| output_id.local_id.clone())
                        .unwrap_or_else(|| WINIT_PRIMARY_OUTPUT_LOCAL_ID.to_owned()),
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
                WinitPresentDriver::HostEventLoop => "host-winit-event-loop".to_owned(),
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
            WinitPresentDriver::HostEventLoop => "nested winit development backend".to_owned(),
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
                owned_outputs.iter().map(|output| (output.output_id, output.properties.clone())),
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
            self.render_state.mode = Some(mode);
        }

        let mut shared = self.shared.borrow_mut();
        if shared.closed {
            return Ok(());
        }
        if !shared.window_ready_for_present {
            let Some((window_size, scale_factor)) = shared.backend.as_ref().map(|backend| {
                let window_size = backend.window_size();
                let scale_factor = backend.scale_factor();
                backend.window().request_redraw();
                (window_size, scale_factor)
            }) else {
                return Ok(());
            };
            if !shared.waiting_for_window_ready_logged {
                tracing::info!(
                    window_size = ?window_size,
                    scale_factor,
                    "waiting for nested winit host window readiness before first present"
                );
                shared.waiting_for_window_ready_logged = true;
            }
            return Ok(());
        }
        let Some(backend) = shared.backend.as_mut() else {
            return Ok(());
        };

        let damage = {
            let cursor_cache = &mut self.render_state.cursor;

            let (renderer, mut framebuffer) = match backend.bind() {
                Ok(bound) => bound,
                Err(error) => {
                    tracing::warn!(error = %error, "failed to bind winit renderer framebuffer");
                    return Ok(());
                }
            };

            let Some(compiled_output) = cx.compiled_frames.output(output.output_id) else {
                return Ok(());
            };
            if let Err(error) = prepare_output_graph_process_shaders(
                renderer,
                &mut self.render_state.execution,
                compiled_output.gpu_prep.as_ref(),
                &compiled_output.process_plan,
            ) {
                tracing::warn!(
                    error = %error,
                    "failed to prewarm process shaders for winit output"
                );
                return Ok(());
            }
            if let Err(error) = prepare_output_graph_targets(
                renderer,
                &mut self.render_state.execution,
                &output,
                &compiled_output.execution_plan,
                compiled_output.target_allocation.as_ref(),
                compiled_output.gpu_prep.as_ref(),
            ) {
                tracing::warn!(
                    error = %error,
                    "failed to prepare output targets for winit output"
                );
                return Ok(());
            }
            prepare_output_material_bindings(
                &mut self.render_state.execution,
                output.output_id,
                &cx.compiled_frames.prepared_gpu,
                compiled_output.gpu_prep.as_ref(),
            );
            if let Err(error) = prepare_output_surface_imports(
                renderer,
                &mut self.render_state.execution,
                &compiled_output.prepared_scene,
                compiled_output.gpu_prep.as_ref(),
                surface_registry,
            ) {
                tracing::warn!(
                    error = %error,
                    "failed to import wayland surfaces for winit output"
                );
                if let Some(diagnostics) = cx.import_diagnostics.as_deref_mut() {
                    match &error {
                        crate::common::gles_executor::GlesExecutionError::SurfaceImport {
                            surface_id,
                            strategy,
                            ..
                        } => diagnostics.push_surface_import_failure(
                            output.device.name.clone(),
                            *surface_id,
                            *strategy,
                            error.to_string(),
                        ),
                        _ => diagnostics
                            .push_present_failure(output.device.name.clone(), error.to_string()),
                    }
                }
                return Ok(());
            }
            let executed = match execute_output_graph(
                renderer,
                &mut self.render_state.execution,
                &output,
                &compiled_output.execution_plan,
                compiled_output.final_output.as_ref(),
                compiled_output.target_allocation.as_ref(),
                &compiled_output.prepared_scene,
                compiled_output.gpu_prep.as_ref(),
                &compiled_output.process_plan,
                &cx.compiled_frames.materials,
                surface_registry,
                cursor_cache,
                cx.config,
                cx.pending_screenshot_requests,
                cx.completed_screenshots,
                cx.clock,
            ) {
                Ok(Some(executed)) => executed,
                Ok(None) => return Ok(()),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to execute render graph for winit output"
                    );
                    if let Some(diagnostics) = cx.import_diagnostics.as_deref_mut() {
                        diagnostics
                            .push_present_failure(output.device.name.clone(), error.to_string());
                    }
                    return Ok(());
                }
            };

            let full_damage =
                vec![Rectangle::from_size((mode.width as i32, mode.height as i32).into())];
            let final_element =
                final_output_texture_element(renderer, executed.texture, mode.scale);
            let elements = vec![final_element];
            let mut frame = match renderer.render(
                &mut framebuffer,
                (mode.width as i32, mode.height as i32).into(),
                winit_output_transform(),
            ) {
                Ok(frame) => frame,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to start final winit composite frame"
                    );
                    return Ok(());
                }
            };
            if let Err(error) = frame.clear(clear_color(cx.config), &full_damage) {
                tracing::warn!(error = %error, "failed to clear winit framebuffer");
                return Ok(());
            }
            if let Err(error) = draw_render_elements::<_, _, CommonGlesRenderElement>(
                &mut frame,
                mode.scale as f64,
                &elements,
                &full_damage,
            ) {
                tracing::warn!(error = %error, "failed to draw final winit composite texture");
                return Ok(());
            }
            // Smithay's nested-winit path submits after `finish()` without an extra renderer wait.
            if let Err(error) = frame.finish().map(|_| ()) {
                tracing::warn!(error = %error, "failed to finish final winit composite frame");
                return Ok(());
            }

            Some(full_damage)
        };

        let Some(damage) = damage else {
            return Ok(());
        };

        if let Err(error) = backend.submit(Some(damage.as_slice())) {
            tracing::warn!(error = %error, "failed to submit winit backbuffer");
        }

        Ok(())
    }

    fn collect_protocol_dmabuf_support(
        &mut self,
        support: &mut ProtocolDmabufSupport,
    ) -> Result<(), NekolandError> {
        let formats = self
            .shared
            .borrow()
            .backend
            .as_ref()
            .map(|backend| backend.dmabuf_formats())
            .unwrap_or_default();
        let renderable_formats = self
            .shared
            .borrow()
            .backend
            .as_ref()
            .map(|backend| backend.dmabuf_render_formats())
            .unwrap_or_default();
        if !formats.is_empty() {
            support.merge_formats(formats, renderable_formats, true);
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

#[allow(dead_code)]
fn output_damage_regions_physical(
    output_damage_regions: &OutputDamageRegions,
    output_id: nekoland_ecs::components::OutputId,
    scale: u32,
) -> Vec<Rectangle<i32, Physical>> {
    output_damage_regions
        .regions
        .get(&output_id)
        .into_iter()
        .flatten()
        .filter_map(|rect| damage_rect_to_physical(rect, scale))
        .collect()
}

#[allow(dead_code)]
fn render_rect_to_physical(rect: &RenderRect, scale: u32) -> Option<Rectangle<i32, Physical>> {
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
    Some(Rectangle::new((x, y).into(), (width.max(1), height.max(1)).into()))
}

#[allow(dead_code)]
fn render_color_to_color32f(color: nekoland_ecs::resources::RenderColor, opacity: f32) -> Color32F {
    let alpha = (f32::from(color.a) / 255.0) * opacity.clamp(0.0, 1.0);
    Color32F::new(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        alpha,
    )
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
    let configured_output = config
        .and_then(|config| config.outputs.iter().find(|output| output.enabled))
        .and_then(|output| {
            parse_output_mode(&output.mode).map(|mode| {
                (output.name.as_str(), mode.width.max(1), mode.height.max(1), output.scale.max(1))
            })
        });
    if let Some((output_name, width, height, scale)) = configured_output {
        return WinitWindowSpec {
            title: format!("{app_name} [winit] - {output_name} {width}x{height}@{scale}x"),
            width,
            height,
        };
    }

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

    let (output_name, width, height, scale) = ("winit", 1280, 720, 1);
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

fn set_host_cursor_capture(
    window: &HostWindow,
    capture: bool,
    capture_mode: &std::cell::Cell<Option<CursorGrabMode>>,
) {
    if capture {
        let grab_result = preferred_cursor_grab_modes()
            .into_iter()
            .find_map(|mode| window.set_cursor_grab(mode).ok().map(|()| mode));
        match grab_result {
            Some(mode) => {
                window.set_cursor_visible(false);
                capture_mode.set(Some(mode));
                tracing::debug!(?mode, "captured winit host cursor");
            }
            None => {
                window.set_cursor_visible(true);
                capture_mode.set(None);
                tracing::warn!("failed to capture winit host cursor");
            }
        }
    } else {
        if let Err(error) = window.set_cursor_grab(CursorGrabMode::None) {
            tracing::debug!(error = %error, "failed to release winit host cursor grab");
        }
        window.set_cursor_visible(true);
        capture_mode.set(None);
    }
}

fn host_window_center(window: &HostWindow) -> (f64, f64) {
    let size = window.inner_size();
    (f64::from(size.width.max(1)) / 2.0, f64::from(size.height.max(1)) / 2.0)
}

fn locked_pointer_seed_event(
    window: &HostWindow,
    capture_mode: Option<CursorGrabMode>,
) -> Option<BackendInputEvent> {
    if capture_mode != Some(CursorGrabMode::Locked) {
        return None;
    }

    let (x, y) = host_window_center(window);
    Some(BackendInputEvent {
        device: HOST_WINIT_DEVICE.to_owned(),
        action: BackendInputAction::PointerMoved { x, y },
    })
}

fn preferred_cursor_grab_modes() -> [CursorGrabMode; 2] {
    [CursorGrabMode::Locked, CursorGrabMode::Confined]
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

fn install_host_winit_source(
    handle: &calloop::LoopHandle<'_, ()>,
    shared: Rc<RefCell<WinitPresentCompletionShared>>,
) -> Result<(), NekolandError> {
    let desired_spec = shared.borrow().desired_window_spec.clone();
    let window_attributes = HostWindow::default_attributes()
        .with_inner_size(PhysicalSize::new(desired_spec.width, desired_spec.height))
        .with_title(desired_spec.title.clone())
        .with_visible(true);
    let (backend, event_loop, capture_mode) =
        match std::panic::catch_unwind(AssertUnwindSafe(|| init_host_winit(window_attributes))) {
            Ok(Ok(triple)) => triple,
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(NekolandError::Runtime(
                    "winit event loop initialization panicked".to_owned(),
                ));
            }
        };
    let monotonic_clock = smithay::utils::Clock::<Monotonic>::new();

    {
        let mut shared = shared.borrow_mut();
        shared.driver = WinitPresentDriver::HostEventLoop;
        shared.closed = false;
        shared.pending_window_state = Some(PendingWinitWindowState {
            size: backend.window_size(),
            scale_factor: backend.scale_factor(),
        });
        shared.backend = Some(backend);
        shared.capture_mode = capture_mode;
        shared.window_ready_for_present = false;
        shared.waiting_for_window_ready_logged = false;
    }
    {
        let mut shared = shared.borrow_mut();
        let desired_window_spec = shared.desired_window_spec.clone();
        let capture_mode = shared.capture_mode.clone();
        if let Some(backend) = shared.backend.as_mut() {
            apply_window_spec(backend.window(), &desired_window_spec);
            set_host_cursor_capture(backend.window(), true, capture_mode.as_ref());
            backend.sync_wayland_shortcuts_inhibitor();
            let seed_event = locked_pointer_seed_event(backend.window(), capture_mode.get());
            if let Some(seed_event) = seed_event {
                shared.pending_input_events.push(seed_event);
            }
        }
    }

    handle
        .insert_source(event_loop, move |event, _, _| match event {
            HostWinitEvent::Redraw => {
                let mut shared = shared.borrow_mut();
                if !shared.closed {
                    if !shared.window_ready_for_present {
                        shared.window_ready_for_present = true;
                    }
                    if shared.active {
                        shared.pending_timestamps_nanos.push(monotonic_now_nanos(&monotonic_clock));
                    }
                    if let Some(backend) = shared.backend.as_mut() {
                        backend.sync_wayland_shortcuts_inhibitor();
                        backend.window().request_redraw();
                    }
                }
            }
            HostWinitEvent::Resized { size, scale_factor } => {
                let mut shared = shared.borrow_mut();
                shared.pending_window_state = Some(PendingWinitWindowState { size, scale_factor });
                if let Some(backend) = shared.backend.as_mut() {
                    backend.sync_wayland_shortcuts_inhibitor();
                }
            }
            HostWinitEvent::Input(input_event) => {
                let mut shared = shared.borrow_mut();
                if should_forward_host_winit_input(shared.capture_mode.get(), &input_event.action) {
                    shared.pending_input_events.push(input_event);
                }
            }
            HostWinitEvent::Focus(focused) => {
                let mut shared = shared.borrow_mut();
                let capture_mode = shared.capture_mode.clone();
                if let Some(backend) = shared.backend.as_mut() {
                    set_host_cursor_capture(backend.window(), focused, capture_mode.as_ref());
                    backend.sync_wayland_shortcuts_inhibitor();
                    let seed_event = locked_pointer_seed_event(backend.window(), capture_mode.get());
                    if let Some(seed_event) = seed_event {
                        shared.pending_input_events.push(seed_event);
                    }
                }
                shared.pending_input_events.push(BackendInputEvent {
                    device: HOST_WINIT_DEVICE.to_owned(),
                    action: BackendInputAction::FocusChanged { focused },
                });
            }
            HostWinitEvent::CloseRequested => {
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
        shared.capture_mode.set(None);
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

fn should_forward_host_winit_input(
    capture_mode: Option<CursorGrabMode>,
    action: &BackendInputAction,
) -> bool {
    match action {
        BackendInputAction::PointerMoved { .. } => capture_mode != Some(CursorGrabMode::Locked),
        BackendInputAction::PointerDelta { .. } => capture_mode == Some(CursorGrabMode::Locked),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use nekoland_config::resources::{CompositorConfig, ConfiguredOutput};
    use nekoland_core::prelude::AppMetadata;
    use nekoland_ecs::components::{OutputDevice, OutputKind, OutputProperties};
    use nekoland_ecs::resources::{BackendInputAction, DamageRect, OutputDamageRegions};
    use smithay::reexports::winit::window::CursorGrabMode;
    use smithay::utils::Transform;

    use crate::traits::{BackendId, OutputSnapshot};

    use super::{
        desired_window_spec, merge_submit_damage, output_damage_regions_physical,
        parse_hex_color32f, preferred_cursor_grab_modes, should_forward_host_winit_input,
        winit_output_transform,
    };

    #[test]
    fn parses_hex_clear_color() {
        let Some(color) = parse_hex_color32f("#f5f7ff") else {
            panic!("hex color should parse");
        };
        assert!((color.r() - 245.0 / 255.0).abs() < 0.0001);
        assert!((color.g() - 247.0 / 255.0).abs() < 0.0001);
        assert!((color.b() - 1.0).abs() < 0.0001);
        assert!((color.a() - 1.0).abs() < 0.0001);
    }

    #[test]
    fn desired_window_spec_uses_configured_output_and_app_name() {
        let metadata = AppMetadata { name: "nekoland".to_owned() };
        let config = CompositorConfig {
            outputs: vec![ConfiguredOutput {
                name: "HDMI-A-1".to_owned(),
                mode: "1600x900@75".to_owned(),
                scale: 2,
                enabled: true,
            }],
            ..CompositorConfig::default()
        };

        let spec = desired_window_spec(Some(&metadata), Some(&config), &[]);

        assert_eq!(spec.width, 1600);
        assert_eq!(spec.height, 900);
        assert_eq!(spec.title, "nekoland [winit] - HDMI-A-1 1600x900@2x");
    }

    #[test]
    fn desired_window_spec_prefers_existing_owned_outputs() {
        let metadata = AppMetadata { name: "nekoland".to_owned() };
        let outputs = vec![OutputSnapshot {
            output_id: nekoland_ecs::components::OutputId(1),
            backend_id: Some(BackendId(1)),
            backend_output_id: Some(crate::traits::BackendOutputId {
                backend_id: BackendId(1),
                local_id: "Winit-1".to_owned(),
            }),
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
    fn winit_cursor_grab_prefers_locked_before_confined() {
        assert_eq!(
            preferred_cursor_grab_modes(),
            [CursorGrabMode::Locked, CursorGrabMode::Confined]
        );
    }

    #[test]
    fn output_damage_regions_are_converted_to_physical_submit_damage() {
        let damage = OutputDamageRegions {
            regions: std::collections::BTreeMap::from([(
                nekoland_ecs::components::OutputId(1),
                vec![DamageRect { x: 10, y: -5, width: 30, height: 20 }],
            )]),
        };

        let physical =
            output_damage_regions_physical(&damage, nekoland_ecs::components::OutputId(1), 2);

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

        let Some(merged) = merge_submit_damage(Some(smithay_damage), ecs_damage) else {
            panic!("merged damage should remain present");
        };

        assert_eq!(merged.len(), 2);
        assert!(merged.contains(&smithay::utils::Rectangle::new((0, 0).into(), (10, 10).into())));
        assert!(merged.contains(&smithay::utils::Rectangle::new((40, 20).into(), (5, 6).into())));
    }

    #[test]
    fn locked_grab_prefers_relative_pointer_delta() {
        assert!(!should_forward_host_winit_input(
            Some(CursorGrabMode::Locked),
            &BackendInputAction::PointerMoved { x: 10.0, y: 20.0 },
        ));
        assert!(should_forward_host_winit_input(
            Some(CursorGrabMode::Locked),
            &BackendInputAction::PointerDelta { dx: 3.0, dy: -4.0 },
        ));
    }

    #[test]
    fn confined_grab_uses_absolute_pointer_motion() {
        assert!(should_forward_host_winit_input(
            Some(CursorGrabMode::Confined),
            &BackendInputAction::PointerMoved { x: 10.0, y: 20.0 },
        ));
        assert!(!should_forward_host_winit_input(
            Some(CursorGrabMode::Confined),
            &BackendInputAction::PointerDelta { dx: 3.0, dy: -4.0 },
        ));
    }
}
