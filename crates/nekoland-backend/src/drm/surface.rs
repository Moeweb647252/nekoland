use std::collections::{HashMap, HashSet};

use drm_fourcc::DrmFourcc;
use nekoland_ecs::resources::{
    CompletedScreenshotFrames, CompositorClock, CompositorConfig, DamageRect, OutputDamageRegions,
    PendingScreenshotRequests, RenderMaterialFrameState, RenderPassGraph, RenderPlan,
    RenderProcessPlan, RenderRect,
};
use nekoland_protocol::ProtocolSurfaceRegistry;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd};
use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::drm::control::{Device as ControlDevice, ModeTypeFlags, connector, crtc};
use smithay::utils::{Physical, Rectangle, Size, Transform};

use crate::common::cursor::SoftwareCursorCache;
use crate::common::gles_executor::{
    CommonGlesRenderElement, GlesExecutionState, execute_output_graph, final_output_texture_element,
};
use crate::traits::OutputSnapshot;

use super::device::{ConnectorInfo, SharedDrmState};
use super::gbm::{GbmState, SharedGbmState};
use super::session::SharedDrmSessionState;

pub(crate) type OurDrmCompositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

pub(crate) struct ConnectorState {
    pub compositor: OurDrmCompositor,
    #[allow(dead_code)]
    pub output: Output,
}

#[derive(Default)]
pub(crate) struct DrmRenderState {
    pub surfaces: HashMap<String, ConnectorState>,
    pub renderer: Option<GlesRenderer>,
    pub cursor: SoftwareCursorCache,
    pub execution: GlesExecutionState,
}

pub(crate) struct DrmPresentCtx<'a> {
    pub outputs: &'a [OutputSnapshot],
    pub config: Option<&'a CompositorConfig>,
    pub clock: Option<&'a CompositorClock>,
    pub output_damage_regions: &'a OutputDamageRegions,
    pub materials: &'a RenderMaterialFrameState,
    pub render_graph: &'a RenderPassGraph,
    pub render_plan: &'a RenderPlan,
    pub process_plan: &'a RenderProcessPlan,
    pub pending_screenshot_requests: &'a mut PendingScreenshotRequests,
    pub completed_screenshots: &'a mut CompletedScreenshotFrames,
    pub surface_registry: Option<&'a ProtocolSurfaceRegistry>,
    pub session_state: &'a SharedDrmSessionState,
    pub drm_shared: &'a SharedDrmState,
    pub gbm_shared: &'a SharedGbmState,
    pub render_state: &'a mut DrmRenderState,
}

pub(crate) fn render_drm_outputs(ctx: DrmPresentCtx<'_>) {
    let DrmPresentCtx {
        outputs,
        config,
        clock,
        output_damage_regions,
        materials,
        render_graph,
        render_plan,
        process_plan,
        pending_screenshot_requests,
        completed_screenshots,
        surface_registry,
        session_state,
        drm_shared,
        gbm_shared,
        render_state,
    } = ctx;
    if !session_state.borrow().active {
        return;
    }

    let Some(surface_registry) = surface_registry else { return };
    if outputs.is_empty() {
        return;
    }

    let mut drm_ref = drm_shared.borrow_mut();
    let Some(drm) = drm_ref.as_mut() else { return };

    let gbm = gbm_shared.borrow();
    let Some(gbm) = gbm.as_ref() else { return };

    if render_state.renderer.is_none() {
        match init_gles_renderer(&gbm.device) {
            Ok(renderer) => render_state.renderer = Some(renderer),
            Err(error) => {
                tracing::warn!(error = %error, "failed to initialise GlesRenderer for DRM");
                return;
            }
        }
    }

    let mut active_connectors = HashSet::new();
    let Some(renderer) = render_state.renderer.as_mut() else {
        tracing::warn!("gles renderer missing after drm renderer initialization");
        return;
    };
    let cursor_cache = &mut render_state.cursor;

    for output in outputs {
        let Some(connector_info) = drm
            .connectors
            .iter()
            .find(|connector| connector.name == output.device.name && connector.connected)
            .cloned()
        else {
            continue;
        };

        active_connectors.insert(connector_info.name.clone());

        if !render_state.surfaces.contains_key(&connector_info.name) {
            match create_connector_surface(
                &mut drm.device,
                &drm.fd,
                gbm,
                &connector_info,
                &output.properties,
            ) {
                Ok(state) => {
                    render_state.surfaces.insert(connector_info.name.clone(), state);
                }
                Err(error) => {
                    tracing::warn!(
                        connector = %connector_info.name,
                        error = %error,
                        "failed to create DRM surface"
                    );
                    continue;
                }
            }
        }

        let Some(surface) = render_state.surfaces.get_mut(&connector_info.name) else {
            continue;
        };

        let Some(execution) = render_graph.outputs.get(&output.output_id) else {
            continue;
        };
        let Some(executed) = execute_output_graph(
            renderer,
            &mut render_state.execution,
            output,
            execution,
            render_plan,
            process_plan,
            materials,
            surface_registry,
            cursor_cache,
            config,
            pending_screenshot_requests,
            completed_screenshots,
            clock,
        )
        .map_err(|error| {
            tracing::warn!(connector = %connector_info.name, error = %error, "failed to execute DRM render graph");
            error
        })
        .ok()
        .flatten()
        else {
            continue;
        };

        let mut elements = vec![final_output_texture_element(
            renderer,
            executed.texture,
            output.properties.scale.max(1),
        )];
        elements.extend(
            ecs_damage_render_elements(
                output_damage_regions,
                output.output_id,
                output.properties.scale.max(1),
            )
            .into_iter()
            .map(CommonGlesRenderElement::from),
        );

        if elements.is_empty() {
            tracing::trace!(connector = %connector_info.name, "DRM frame empty");
            continue;
        }

        let clear = clear_color(config);
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
                if let Err(error) = surface.compositor.queue_frame(()) {
                    tracing::warn!(connector = %connector_info.name, error = %error, "DRM queue_frame failed");
                    continue;
                }
                if let Err(error) = surface.compositor.frame_submitted() {
                    tracing::warn!(connector = %connector_info.name, error = %error, "DRM frame_submitted failed");
                    continue;
                }
                tracing::trace!(connector = %connector_info.name, "DRM frame queued");
            }
            Err(error) => {
                tracing::warn!(connector = %connector_info.name, error = %error, "DRM render_frame failed");
            }
        }
    }

    render_state.surfaces.retain(|name, _| active_connectors.contains(name));
}

fn ecs_damage_render_elements(
    output_damage_regions: &OutputDamageRegions,
    output_id: nekoland_ecs::components::OutputId,
    scale: u32,
) -> Vec<SolidColorRenderElement> {
    output_damage_regions
        .regions
        .get(&output_id)
        .into_iter()
        .flatten()
        .filter_map(|rect| damage_rect_to_physical(rect, scale))
        .map(|geometry| {
            SolidColorRenderElement::new(
                Id::new(),
                geometry,
                CommitCounter::default(),
                Color32F::TRANSPARENT,
                Kind::Unspecified,
            )
        })
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

    Some(Rectangle::new((x, y).into(), (width, height).into()))
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
    output_properties: &nekoland_ecs::components::OutputProperties,
) -> Result<ConnectorState, Box<dyn std::error::Error>> {
    let resources = fd.resource_handles()?;
    let crtc_handle = pick_crtc(fd, connector_info.handle, &resources)?;
    let connector_state = fd.get_connector(connector_info.handle, true)?;
    let mode = pick_mode(&connector_state, output_properties)?;

    let drm_surface = device.create_surface(crtc_handle, mode, &[connector_info.handle])?;

    let (width, height) = mode.size();
    let refresh_millihz = mode_refresh_millihz(&mode);

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

    if let Some(crtc) = connector_state
        .current_encoder()
        .and_then(|enc| fd.get_encoder(enc).ok())
        .and_then(|enc| enc.crtc())
    {
        return Ok(crtc);
    }

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
    output_properties: &nekoland_ecs::components::OutputProperties,
) -> Result<smithay::reexports::drm::control::Mode, Box<dyn std::error::Error>> {
    let modes = connector_state.modes();
    if modes.is_empty() {
        return Err("connector has no modes".into());
    }
    if let Some(mode) =
        modes.iter().find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
    {
        return Ok(*mode);
    }
    if let Some(mode) = modes.iter().find(|mode| {
        let (width, height) = mode.size();
        width as u32 == output_properties.width && height as u32 == output_properties.height
    }) {
        return Ok(*mode);
    }
    Ok(modes[0])
}

fn mode_refresh_millihz(mode: &smithay::reexports::drm::control::Mode) -> u32 {
    let refresh = mode.vrefresh();
    if refresh > 0 { refresh * 1000 } else { 60_000 }
}

fn clear_color(config: Option<&CompositorConfig>) -> Color32F {
    config
        .map(|config| config.background_color.as_str())
        .and_then(|color| color.strip_prefix('#'))
        .and_then(|hex| {
            if hex.len() != 6 && hex.len() != 8 {
                return None;
            }
            let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let alpha =
                if hex.len() == 8 { u8::from_str_radix(&hex[6..8], 16).ok()? } else { u8::MAX };
            Some(Color32F::new(
                f32::from(red) / 255.0,
                f32::from(green) / 255.0,
                f32::from(blue) / 255.0,
                f32::from(alpha) / 255.0,
            ))
        })
        .unwrap_or(Color32F::BLACK)
}

#[cfg(test)]
mod tests {
    use nekoland_ecs::resources::{DamageRect, OutputDamageRegions};
    use smithay::backend::renderer::element::Element;
    use smithay::utils::Scale;

    use super::{damage_rect_to_physical, ecs_damage_render_elements};

    #[test]
    fn damage_rects_convert_to_physical_coordinates() {
        let Some(rect) =
            damage_rect_to_physical(&DamageRect { x: 10, y: -4, width: 30, height: 20 }, 2)
        else {
            panic!("damage rect should convert to physical coordinates");
        };
        assert_eq!(rect.loc.x, 20);
        assert_eq!(rect.loc.y, -8);
        assert_eq!(rect.size.w, 60);
        assert_eq!(rect.size.h, 40);
    }

    #[test]
    fn ecs_damage_render_elements_follow_output_routing() {
        let output_damage_regions = OutputDamageRegions {
            regions: std::collections::BTreeMap::from([
                (
                    nekoland_ecs::components::OutputId(1),
                    vec![DamageRect { x: 5, y: 6, width: 20, height: 10 }],
                ),
                (
                    nekoland_ecs::components::OutputId(2),
                    vec![DamageRect { x: 100, y: 200, width: 40, height: 30 }],
                ),
            ]),
        };

        let elements = ecs_damage_render_elements(
            &output_damage_regions,
            nekoland_ecs::components::OutputId(2),
            1,
        );

        assert_eq!(elements.len(), 1);
        let geometry = elements[0].geometry(Scale::from(1.0));
        assert_eq!(geometry.loc.x, 100);
        assert_eq!(geometry.loc.y, 200);
        assert_eq!(geometry.size.w, 40);
        assert_eq!(geometry.size.h, 30);
    }
}
