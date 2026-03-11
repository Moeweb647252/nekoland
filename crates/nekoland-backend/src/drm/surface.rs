use std::collections::{HashMap, HashSet};

use bevy_ecs::prelude::{Local, NonSend, NonSendMut, Query, Res, ResMut};
use drm_fourcc::DrmFourcc;
use nekoland_ecs::components::{OutputDevice, OutputProperties, SurfaceGeometry, WlSurfaceHandle};
use nekoland_ecs::resources::{CompositorConfig, PendingOutputPresentationEvents, RenderList};
use nekoland_protocol::ProtocolSurfaceRegistry;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd};
use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::drm::control::{Device as ControlDevice, ModeTypeFlags, connector, crtc};
use smithay::utils::{Clock, Monotonic, Size, Transform};

use crate::plugin::{OutputPresentationRuntime, emit_present_completion_events};
use crate::traits::{BackendKind, SelectedBackend};

use super::device::{ConnectorInfo, SharedDrmState};
use super::gbm::{GbmState, SharedGbmState};

/// Concrete `DrmCompositor` type:
///   A = GbmAllocator<DrmDeviceFd>          — allocator
///   F = GbmFramebufferExporter<DrmDeviceFd> — framebuffer exporter
///   U = ()                                  — no per-frame user data
///   G = DrmDeviceFd                         — GBM device fd
pub(crate) type OurDrmCompositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

pub(crate) struct ConnectorState {
    pub compositor: OurDrmCompositor,
    #[allow(dead_code)]
    pub output: Output,
}

/// All DRM render state.
///
/// Stored as a `NonSend` resource because `GlesRenderer` and `DrmCompositor`
/// contain raw pointers and are `!Send`.
#[derive(Default)]
pub(crate) struct DrmRenderState {
    pub surfaces: HashMap<String, ConnectorState>,
    pub renderer: Option<GlesRenderer>,
}

/// Present-completion system for the DRM backend (runs in `ExtractSchedule`).
pub(crate) fn drm_present_completion_system(
    selected_backend: Res<SelectedBackend>,
    outputs: Query<(&OutputDevice, &OutputProperties)>,
    mut pending_presentation_events: ResMut<PendingOutputPresentationEvents>,
    mut presentation_runtime: Local<OutputPresentationRuntime>,
    mut monotonic_clock: Local<Option<Clock<Monotonic>>>,
) {
    emit_present_completion_events(
        BackendKind::Drm,
        &selected_backend,
        &outputs,
        &mut pending_presentation_events,
        &mut presentation_runtime,
        &mut monotonic_clock,
    );
}

/// DRM render system (runs in `PresentSchedule`, gated by `SelectedBackend::Drm`).
pub(crate) fn drm_render_system(
    selected_backend: Res<SelectedBackend>,
    config: Option<Res<CompositorConfig>>,
    mut outputs: Query<(&OutputDevice, &mut OutputProperties)>,
    surfaces: Query<(&WlSurfaceHandle, &SurfaceGeometry)>,
    render_list: Res<RenderList>,
    surface_registry: Option<NonSend<ProtocolSurfaceRegistry>>,
    drm_shared: NonSend<SharedDrmState>,
    gbm_shared: NonSend<SharedGbmState>,
    mut render_state: NonSendMut<DrmRenderState>,
) {
    if selected_backend.kind != BackendKind::Drm {
        return;
    }

    let Some(surface_registry) = surface_registry else { return };
    let geometry_by_surface = surfaces
        .iter()
        .map(|(surface, geometry)| (surface.id, geometry.clone()))
        .collect::<HashMap<_, _>>();

    let mut drm_ref = drm_shared.borrow_mut();
    let Some(drm) = drm_ref.as_mut() else { return };

    let gbm = gbm_shared.borrow();
    let Some(gbm) = gbm.as_ref() else { return };

    // Ensure the GlesRenderer is initialised before iterating outputs.
    if render_state.renderer.is_none() {
        match init_gles_renderer(&gbm.device) {
            Ok(renderer) => render_state.renderer = Some(renderer),
            Err(e) => {
                tracing::warn!(error = %e, "failed to initialise GlesRenderer for DRM");
                return;
            }
        }
    }

    let mut active_connectors = HashSet::new();
    let DrmRenderState { ref mut renderer, ref mut surfaces } = *render_state;
    let renderer = renderer.as_mut().expect("initialised above");

    for (output_device, mut output_properties) in outputs.iter_mut() {
        let Some(connector_info) =
            drm.connectors.iter().find(|c| c.name == output_device.name && c.connected).cloned()
        else {
            continue;
        };

        active_connectors.insert(connector_info.name.clone());

        // Create surface on first encounter, then look it up.
        if !surfaces.contains_key(&connector_info.name) {
            match create_connector_surface(
                &mut drm.device,
                &drm.fd,
                gbm,
                &connector_info,
                &mut output_properties,
            ) {
                Ok(state) => {
                    surfaces.insert(connector_info.name.clone(), state);
                }
                Err(e) => {
                    tracing::warn!(
                        connector = %connector_info.name,
                        error = %e,
                        "failed to create DRM surface"
                    );
                    continue;
                }
            }
        }

        let Some(surface) = surfaces.get_mut(&connector_info.name) else {
            continue;
        };

        let scale = output_properties.scale.max(1) as f64;
        let mut elements = Vec::<WaylandSurfaceRenderElement<GlesRenderer>>::new();
        for render_element in &render_list.elements {
            if render_element.surface_id == 0 {
                continue;
            }

            let Some(wl_surface) = surface_registry.surface(render_element.surface_id) else {
                continue;
            };
            let Some(geometry) = geometry_by_surface.get(&render_element.surface_id) else {
                continue;
            };

            elements.extend(render_elements_from_surface_tree(
                renderer,
                wl_surface,
                (geometry.x, geometry.y),
                scale,
                render_element.opacity,
                Kind::Unspecified,
            ));
        }

        let clear = clear_color(config.as_deref());

        match surface.compositor.render_frame::<_, _>(
            renderer,
            &elements,
            clear,
            FrameFlags::DEFAULT,
        ) {
            Ok(result) => {
                if result.is_empty {
                    tracing::trace!(connector = %connector_info.name, "DRM frame empty");
                    continue;
                }
                if let Err(e) = surface.compositor.queue_frame(()) {
                    tracing::warn!(connector = %connector_info.name, error = %e, "DRM queue_frame failed");
                    continue;
                }
                if let Err(e) = surface.compositor.frame_submitted() {
                    tracing::warn!(connector = %connector_info.name, error = %e, "DRM frame_submitted failed");
                    continue;
                }
                tracing::trace!(connector = %connector_info.name, "DRM frame queued");
            }
            Err(e) => {
                tracing::warn!(connector = %connector_info.name, error = %e, "DRM render_frame failed");
            }
        }
    }

    surfaces.retain(|name, _| active_connectors.contains(name));
}

fn init_gles_renderer(
    gbm: &smithay::backend::allocator::gbm::GbmDevice<DrmDeviceFd>,
) -> Result<GlesRenderer, Box<dyn std::error::Error>> {
    let egl_display = unsafe { smithay::backend::egl::EGLDisplay::new(gbm.clone())? };
    let egl_context = smithay::backend::egl::EGLContext::new(&egl_display)?;
    let renderer = unsafe { GlesRenderer::new(egl_context)? };
    tracing::info!("GlesRenderer initialised for DRM");
    Ok(renderer)
}

fn create_connector_surface(
    device: &mut DrmDevice,
    fd: &DrmDeviceFd,
    gbm: &GbmState,
    connector_info: &ConnectorInfo,
    output_properties: &mut OutputProperties,
) -> Result<ConnectorState, Box<dyn std::error::Error>> {
    let resources = fd.resource_handles()?;
    let crtc_handle = pick_crtc(fd, connector_info.handle, &resources)?;
    let connector_state = fd.get_connector(connector_info.handle, true)?;
    let mode = pick_mode(&connector_state, output_properties)?;

    let drm_surface = device.create_surface(crtc_handle, mode, &[connector_info.handle])?;

    let (width, height) = mode.size();
    let refresh_millihz = mode_refresh_millihz(&mode);
    output_properties.width = width.max(1) as u32;
    output_properties.height = height.max(1) as u32;
    output_properties.refresh_millihz = refresh_millihz;

    let output = Output::new(
        connector_info.name.clone(),
        PhysicalProperties {
            size: Size::from((0, 0)),
            subpixel: Subpixel::Unknown,
            make: "Unknown".to_owned(),
            model: "Unknown".to_owned(),
        },
    );
    let smithay_mode = OutputMode {
        size: Size::from((width as i32, height as i32)),
        refresh: refresh_millihz as i32,
    };
    output.change_current_state(
        Some(smithay_mode),
        Some(Transform::Normal),
        Some(Scale::Integer(output_properties.scale.max(1) as i32)),
        None,
    );
    output.set_preferred(smithay_mode);

    let gbm_allocator =
        GbmAllocator::new(gbm.device.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let fb_exporter = GbmFramebufferExporter::new(gbm.device.clone(), None);
    let cursor_size = device.cursor_size();

    let compositor = OurDrmCompositor::new(
        &output,
        drm_surface,
        None,
        gbm_allocator,
        fb_exporter,
        [DrmFourcc::Argb8888, DrmFourcc::Xrgb8888],
        std::iter::empty(),
        cursor_size,
        Some(gbm.device.clone()),
    )?;

    tracing::info!(connector = %connector_info.name, width, height, refresh_millihz, "DRM connector surface created");
    Ok(ConnectorState { compositor, output })
}

fn pick_crtc(
    fd: &DrmDeviceFd,
    connector_handle: connector::Handle,
    resources: &smithay::reexports::drm::control::ResourceHandles,
) -> Result<crtc::Handle, Box<dyn std::error::Error>> {
    let connector_state = fd.get_connector(connector_handle, false)?;

    // Prefer the currently active CRTC.
    if let Some(crtc) = connector_state
        .current_encoder()
        .and_then(|enc| fd.get_encoder(enc).ok())
        .and_then(|enc| enc.crtc())
    {
        return Ok(crtc);
    }

    // Find first compatible CRTC via encoder bitmask.
    for &encoder_handle in connector_state.encoders() {
        let encoder = fd.get_encoder(encoder_handle)?;
        let compatible_crtcs = resources.filter_crtcs(encoder.possible_crtcs());
        if let Some(&crtc) = compatible_crtcs.first() {
            return Ok(crtc);
        }
    }

    Err(format!("no compatible CRTC for connector {:?}", connector_handle).into())
}

fn pick_mode(
    connector_state: &connector::Info,
    output_properties: &OutputProperties,
) -> Result<smithay::reexports::drm::control::Mode, Box<dyn std::error::Error>> {
    let modes = connector_state.modes();
    if modes.is_empty() {
        return Err("connector has no modes".into());
    }
    if let Some(mode) = modes.iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)) {
        return Ok(*mode);
    }
    if let Some(mode) = modes.iter().find(|m| {
        let (w, h) = m.size();
        w as u32 == output_properties.width && h as u32 == output_properties.height
    }) {
        return Ok(*mode);
    }
    Ok(modes[0])
}

fn mode_refresh_millihz(mode: &smithay::reexports::drm::control::Mode) -> u32 {
    // Approximate refresh from pixel clock and active resolution.
    // The DRM compositor will use the precise vblank timing from the hardware.
    let (width, height) = mode.size();
    let clock = mode.clock() as u64 * 1_000; // Convert kHz → Hz
    let htotal = width as u64;
    let vtotal = height as u64;
    if htotal == 0 || vtotal == 0 {
        return 60_000;
    }
    ((clock * 1_000) / (htotal * vtotal)).min(u64::from(u32::MAX)) as u32
}

fn clear_color(config: Option<&CompositorConfig>) -> Color32F {
    config.and_then(|c| parse_hex_color32f(&c.background_color)).unwrap_or(Color32F::BLACK)
}

fn parse_hex_color32f(color: &str) -> Option<Color32F> {
    let hex = color.strip_prefix('#')?;
    let (r, g, b, a) = match hex.len() {
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::MAX,
        ),
        8 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::from_str_radix(&hex[6..8], 16).ok()?,
        ),
        _ => return None,
    };
    Some(Color32F::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ))
}
