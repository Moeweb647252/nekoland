use std::collections::{BTreeMap, HashMap};

use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    CompletedScreenshotFrames, CompositorClock, CompositorConfig, CursorRenderSource,
    MaterialParamsId, OutputExecutionPlan, PendingScreenshotRequests, RenderColor,
    RenderMaterialFrameState, RenderPassKind, RenderPassPayload, RenderPlan, RenderPlanItem,
    RenderRect, RenderTargetId, ScreenshotFrame,
};
use nekoland_protocol::ProtocolSurfaceRegistry;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::utils::CropRenderElement;
use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType,
    UniformValue,
};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::utils::draw_render_elements;
use smithay::backend::renderer::{Bind, Color32F, ExportMem, Frame, Offscreen, Renderer, Texture};
use smithay::render_elements;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform};

use crate::common::cursor::SoftwareCursorCache;
use crate::traits::OutputSnapshot;

const BACKDROP_BLUR_SHADER: &str = r#"
precision mediump float;
//_DEFINES
varying vec2 v_coords;
uniform sampler2D tex;
uniform float alpha;
uniform vec2 tex_size;
uniform float radius;

void main() {
    vec2 inv_size = vec2(1.0) / max(tex_size, vec2(1.0));
    vec2 offset = inv_size * max(radius, 1.0);

    vec4 sum = vec4(0.0);
    sum += texture2D(tex, v_coords + vec2(-offset.x, -offset.y));
    sum += texture2D(tex, v_coords + vec2(0.0, -offset.y));
    sum += texture2D(tex, v_coords + vec2(offset.x, -offset.y));
    sum += texture2D(tex, v_coords + vec2(-offset.x, 0.0));
    sum += texture2D(tex, v_coords);
    sum += texture2D(tex, v_coords + vec2(offset.x, 0.0));
    sum += texture2D(tex, v_coords + vec2(-offset.x, offset.y));
    sum += texture2D(tex, v_coords + vec2(0.0, offset.y));
    sum += texture2D(tex, v_coords + vec2(offset.x, offset.y));
    sum /= 9.0;

    gl_FragColor = vec4(sum.rgb, sum.a * alpha);
}
"#;

render_elements! {
    pub(crate) CommonGlesRenderElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    ClippedSurface=CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Texture=TextureRenderElement<GlesTexture>,
}

#[derive(Debug, Clone)]
struct CachedExecutionTarget {
    texture: GlesTexture,
    size: Size<i32, Physical>,
    backdrop_regions: Vec<Rectangle<i32, Physical>>,
}

#[derive(Debug, Default)]
struct OutputExecutionCache {
    targets: BTreeMap<RenderTargetId, CachedExecutionTarget>,
}

#[derive(Debug, Default)]
struct MaterialPipelineCache {
    backdrop_blur: Option<GlesTexProgram>,
}

#[derive(Debug, Default)]
pub(crate) struct GlesExecutionState {
    outputs: HashMap<OutputId, OutputExecutionCache>,
    material_pipelines: MaterialPipelineCache,
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutedOutputTexture {
    pub texture: GlesTexture,
}

pub(crate) fn execute_output_graph(
    renderer: &mut GlesRenderer,
    state: &mut GlesExecutionState,
    output: &OutputSnapshot,
    execution: &OutputExecutionPlan,
    render_plan: &RenderPlan,
    materials: &RenderMaterialFrameState,
    surface_registry: &ProtocolSurfaceRegistry,
    cursor_cache: &mut SoftwareCursorCache,
    config: Option<&CompositorConfig>,
    pending_screenshot_requests: &mut PendingScreenshotRequests,
    completed_screenshots: &mut CompletedScreenshotFrames,
    clock: Option<&CompositorClock>,
) -> Result<Option<ExecutedOutputTexture>, GlesError> {
    let Some(output_plan) = render_plan.outputs.get(&output.output_id) else {
        return Ok(None);
    };

    let output_size = Size::from((
        i32::try_from(output.properties.width.max(1)).unwrap_or(i32::MAX),
        i32::try_from(output.properties.height.max(1)).unwrap_or(i32::MAX),
    ));
    let output_rect = Rectangle::from_size(output_size);
    let output_scale = output.properties.scale.max(1);

    let output_cache = state.outputs.entry(output.output_id).or_default();
    output_cache.targets.retain(|target_id, _| execution.targets.contains_key(target_id));
    for target in output_cache.targets.values_mut() {
        target.backdrop_regions.clear();
    }

    for pass_id in execution.reachable_passes_in_order() {
        let Some(pass) = execution.passes.get(&pass_id) else {
            continue;
        };

        match pass.kind {
            RenderPassKind::Scene => {
                let target =
                    ensure_target(renderer, output_cache, pass.output_target, output_size)?;
                let built = build_scene_pass_elements(
                    renderer,
                    surface_registry,
                    cursor_cache,
                    config,
                    output_scale,
                    output_plan,
                    pass.item_ids(),
                );
                target.backdrop_regions.extend(built.backdrop_regions);

                let mut framebuffer = renderer.bind(&mut target.texture)?;
                let mut frame =
                    renderer.render(&mut framebuffer, output_size, Transform::Normal)?;
                if pass.dependencies.is_empty() {
                    frame.clear(clear_color(config), &[output_rect])?;
                }
                let _ = draw_render_elements::<GlesRenderer, _, CommonGlesRenderElement>(
                    &mut frame,
                    Scale::from(output_scale as f64),
                    &built.elements,
                    &[output_rect],
                )?;
                let sync = frame.finish()?;
                renderer.wait(&sync).map_err(|_| GlesError::SyncInterrupted)?;
            }
            RenderPassKind::Composite => {
                let RenderPassPayload::Composite(config_payload) = &pass.payload else {
                    continue;
                };
                let source_texture = output_cache
                    .targets
                    .get(&config_payload.source_target)
                    .map(|target| (target.texture.clone(), target.backdrop_regions.clone()));
                let Some((source_texture, source_backdrops)) = source_texture else {
                    continue;
                };
                let target =
                    ensure_target(renderer, output_cache, pass.output_target, output_size)?;
                target.backdrop_regions = source_backdrops;
                composite_texture_to_target(
                    renderer,
                    &source_texture,
                    &mut target.texture,
                    output_size,
                    output_rect,
                )?;
            }
            RenderPassKind::PostProcess => {
                let RenderPassPayload::PostProcess(config_payload) = &pass.payload else {
                    continue;
                };
                let source_texture = output_cache
                    .targets
                    .get(&config_payload.source_target)
                    .map(|target| (target.texture.clone(), target.backdrop_regions.clone()));
                let Some((source_texture, source_backdrops)) = source_texture else {
                    continue;
                };
                let target =
                    ensure_target(renderer, output_cache, pass.output_target, output_size)?;
                target.backdrop_regions = source_backdrops.clone();
                execute_material_pass(
                    renderer,
                    &mut state.material_pipelines,
                    materials,
                    config_payload.material_id,
                    config_payload.params_id,
                    &source_texture,
                    &mut target.texture,
                    &source_backdrops,
                    output_size,
                    output_rect,
                )?;
            }
            RenderPassKind::Readback => {
                let RenderPassPayload::Readback(config_payload) = &pass.payload else {
                    continue;
                };
                let requests = pending_screenshot_requests.requests_for_output(output.output_id);
                if requests.is_empty() {
                    continue;
                }
                let Some(target) = output_cache.targets.get_mut(&config_payload.source_target)
                else {
                    continue;
                };
                let pixels = readback_texture_rgba(renderer, &mut target.texture, output_size)?;
                let frame = clock.map(|clock| clock.frame).unwrap_or_default();
                let uptime_millis = clock
                    .map(|clock| clock.uptime_millis.min(u128::from(u64::MAX)) as u64)
                    .unwrap_or_default();
                for request in &requests {
                    completed_screenshots.push_frame(ScreenshotFrame {
                        request_id: request.id,
                        output_id: output.output_id,
                        frame,
                        uptime_millis,
                        width: output.properties.width,
                        height: output.properties.height,
                        scale: output_scale,
                        pixels_rgba: pixels.clone(),
                    });
                }
                let completed_ids = requests.iter().map(|request| request.id).collect::<Vec<_>>();
                pending_screenshot_requests.finish_requests(&completed_ids);
            }
        }
    }

    let final_target = output_swapchain_target(execution, output.output_id).or_else(|| {
        execution
            .ordered_passes
            .last()
            .and_then(|pass_id| execution.passes.get(pass_id))
            .map(|pass| pass.output_target)
    });
    let Some(final_target) = final_target else {
        return Ok(None);
    };
    let Some(target) = output_cache.targets.get(&final_target) else {
        return Ok(None);
    };

    Ok(Some(ExecutedOutputTexture { texture: target.texture.clone() }))
}

pub(crate) fn final_output_texture_element(
    renderer: &GlesRenderer,
    texture: GlesTexture,
    output_scale: u32,
) -> CommonGlesRenderElement {
    TextureRenderElement::from_static_texture(
        Id::new(),
        renderer.context_id(),
        Point::from((0.0, 0.0)),
        texture,
        output_scale.max(1) as i32,
        Transform::Normal,
        Some(1.0),
        None,
        None,
        None,
        Kind::Unspecified,
    )
    .into()
}

pub(crate) fn render_rect_to_physical(
    rect: &RenderRect,
    scale: u32,
) -> Option<Rectangle<i32, Physical>> {
    let scale = i32::try_from(scale.max(1)).ok()?;
    let width = i32::try_from(rect.width).ok()?.checked_mul(scale)?;
    let height = i32::try_from(rect.height).ok()?.checked_mul(scale)?;
    let x = rect.x.checked_mul(scale)?;
    let y = rect.y.checked_mul(scale)?;
    Some(Rectangle::new((x, y).into(), (width, height).into()))
}

pub(crate) fn render_color_to_color32f(color: RenderColor, opacity: f32) -> Color32F {
    let alpha = (f32::from(color.a) / 255.0) * opacity.clamp(0.0, 1.0);
    Color32F::new(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        alpha,
    )
}

pub(crate) fn clear_color(config: Option<&CompositorConfig>) -> Color32F {
    let Some(config) = config else {
        return Color32F::new(0.0, 0.0, 0.0, 1.0);
    };

    parse_hex_color(&config.background_color).unwrap_or_else(|| Color32F::new(0.0, 0.0, 0.0, 1.0))
}

fn parse_hex_color(color: &str) -> Option<Color32F> {
    let trimmed = color.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color32F::new(
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color32F::new(
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                f32::from(a) / 255.0,
            ))
        }
        _ => None,
    }
}

struct ScenePassBuilt {
    elements: Vec<CommonGlesRenderElement>,
    backdrop_regions: Vec<Rectangle<i32, Physical>>,
}

fn build_scene_pass_elements(
    renderer: &mut GlesRenderer,
    surface_registry: &ProtocolSurfaceRegistry,
    cursor_cache: &mut SoftwareCursorCache,
    config: Option<&CompositorConfig>,
    output_scale: u32,
    output_plan: &nekoland_ecs::resources::OutputRenderPlan,
    item_ids: &[nekoland_ecs::resources::RenderItemId],
) -> ScenePassBuilt {
    let mut elements = Vec::new();
    let mut backdrop_regions = Vec::new();

    for item_id in scene_pass_item_ids_in_presentation_order(item_ids) {
        let Some(item) = output_plan.item(*item_id) else {
            continue;
        };
        match item {
            RenderPlanItem::Surface(item) => {
                let Some(clip_rect) = item
                    .instance
                    .visible_rect()
                    .and_then(|rect| render_rect_to_physical(&rect, output_scale))
                else {
                    continue;
                };
                let Some(surface) = surface_registry.surface(item.surface_id) else {
                    continue;
                };
                elements.extend(
                    render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<_>>(
                        renderer,
                        surface,
                        (item.instance.rect.x, item.instance.rect.y),
                        output_scale as f64,
                        item.instance.opacity,
                        Kind::Unspecified,
                    )
                    .into_iter()
                    .filter_map(|element| {
                        CropRenderElement::from_element(element, output_scale as f64, clip_rect)
                    })
                    .map(CommonGlesRenderElement::from),
                );
            }
            RenderPlanItem::SolidRect(item) => {
                let Some(visible_rect) = item
                    .instance
                    .visible_rect()
                    .and_then(|rect| render_rect_to_physical(&rect, output_scale))
                else {
                    continue;
                };
                elements.push(
                    SolidColorRenderElement::new(
                        Id::new(),
                        visible_rect,
                        CommitCounter::default(),
                        render_color_to_color32f(item.color, item.instance.opacity),
                        Kind::Unspecified,
                    )
                    .into(),
                );
            }
            RenderPlanItem::Backdrop(item) => {
                let Some(visible_rect) = item
                    .instance
                    .visible_rect()
                    .and_then(|rect| render_rect_to_physical(&rect, output_scale))
                else {
                    continue;
                };
                backdrop_regions.push(visible_rect);
            }
            RenderPlanItem::Cursor(item) => match &item.source {
                CursorRenderSource::Named { icon_name } => {
                    let theme =
                        config.map(|config| config.cursor_theme.as_str()).unwrap_or("default");
                    match cursor_cache.render_element(
                        renderer,
                        theme,
                        icon_name,
                        output_scale.max(1),
                        item.instance.rect.x,
                        item.instance.rect.y,
                    ) {
                        Ok(element) => elements.push(element.into()),
                        Err(error) => {
                            tracing::warn!(error = %error, "failed to upload software cursor");
                        }
                    }
                }
                CursorRenderSource::Surface { surface_id } => {
                    let Some(clip_rect) = item
                        .instance
                        .visible_rect()
                        .and_then(|rect| render_rect_to_physical(&rect, output_scale))
                    else {
                        continue;
                    };
                    let Some(surface) = surface_registry.surface(*surface_id) else {
                        continue;
                    };
                    elements.extend(
                        render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<_>>(
                            renderer,
                            surface,
                            (item.instance.rect.x, item.instance.rect.y),
                            output_scale as f64,
                            item.instance.opacity,
                            Kind::Cursor,
                        )
                        .into_iter()
                        .filter_map(|element| {
                            CropRenderElement::from_element(element, output_scale as f64, clip_rect)
                        })
                        .map(CommonGlesRenderElement::from),
                    );
                }
            },
        }
    }

    ScenePassBuilt { elements, backdrop_regions }
}

fn scene_pass_item_ids_in_presentation_order(
    item_ids: &[nekoland_ecs::resources::RenderItemId],
) -> impl Iterator<Item = &nekoland_ecs::resources::RenderItemId> {
    item_ids.iter().rev()
}

fn ensure_target<'a>(
    renderer: &mut GlesRenderer,
    cache: &'a mut OutputExecutionCache,
    target_id: RenderTargetId,
    output_size: Size<i32, Physical>,
) -> Result<&'a mut CachedExecutionTarget, GlesError> {
    let recreate = cache.targets.get(&target_id).is_none_or(|target| target.size != output_size);
    if recreate {
        let texture = Offscreen::<GlesTexture>::create_buffer(
            renderer,
            Fourcc::Abgr8888,
            Size::<i32, Buffer>::from((output_size.w, output_size.h)),
        )?;
        cache.targets.insert(
            target_id,
            CachedExecutionTarget { texture, size: output_size, backdrop_regions: Vec::new() },
        );
    }

    Ok(cache.targets.get_mut(&target_id).expect("target inserted above"))
}

fn composite_texture_to_target(
    renderer: &mut GlesRenderer,
    source_texture: &GlesTexture,
    dest_texture: &mut GlesTexture,
    output_size: Size<i32, Physical>,
    output_rect: Rectangle<i32, Physical>,
) -> Result<(), GlesError> {
    let mut framebuffer = renderer.bind(dest_texture)?;
    let mut frame = renderer.render(&mut framebuffer, output_size, Transform::Normal)?;
    frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[output_rect])?;
    frame.render_texture_from_to(
        source_texture,
        Rectangle::<f64, Buffer>::from_size(source_texture.size().to_f64()),
        output_rect,
        &[output_rect],
        &[],
        Transform::Normal,
        1.0,
        None,
        &[],
    )?;
    let sync = frame.finish()?;
    renderer.wait(&sync).map_err(|_| GlesError::SyncInterrupted)?;
    Ok(())
}

fn execute_material_pass(
    renderer: &mut GlesRenderer,
    pipelines: &mut MaterialPipelineCache,
    materials: &RenderMaterialFrameState,
    material_id: nekoland_ecs::resources::RenderMaterialId,
    params_id: Option<MaterialParamsId>,
    source_texture: &GlesTexture,
    dest_texture: &mut GlesTexture,
    backdrop_regions: &[Rectangle<i32, Physical>],
    output_size: Size<i32, Physical>,
    output_rect: Rectangle<i32, Physical>,
) -> Result<(), GlesError> {
    let pipeline_key = materials
        .descriptor(material_id)
        .map(|descriptor| descriptor.pipeline_key.0.as_str())
        .unwrap_or("passthrough");

    match pipeline_key {
        "backdrop_blur" => execute_backdrop_blur_pass(
            renderer,
            pipelines,
            materials,
            params_id,
            source_texture,
            dest_texture,
            backdrop_regions,
            output_size,
            output_rect,
        ),
        _ => composite_texture_to_target(
            renderer,
            source_texture,
            dest_texture,
            output_size,
            output_rect,
        ),
    }
}

fn execute_backdrop_blur_pass(
    renderer: &mut GlesRenderer,
    pipelines: &mut MaterialPipelineCache,
    materials: &RenderMaterialFrameState,
    params_id: Option<MaterialParamsId>,
    source_texture: &GlesTexture,
    dest_texture: &mut GlesTexture,
    backdrop_regions: &[Rectangle<i32, Physical>],
    output_size: Size<i32, Physical>,
    output_rect: Rectangle<i32, Physical>,
) -> Result<(), GlesError> {
    composite_texture_to_target(renderer, source_texture, dest_texture, output_size, output_rect)?;

    if backdrop_regions.is_empty() {
        return Ok(());
    }

    let radius = params_id
        .and_then(|params_id| materials.params(params_id))
        .and_then(|params| params.float("radius"))
        .unwrap_or(12.0);
    let program = backdrop_blur_program(renderer, pipelines)?.clone();
    let uniforms = vec![
        Uniform::new("tex_size", UniformValue::_2f(output_size.w as f32, output_size.h as f32)),
        Uniform::new("radius", UniformValue::_1f(radius)),
    ];

    let mut framebuffer = renderer.bind(dest_texture)?;
    let mut frame = renderer.render(&mut framebuffer, output_size, Transform::Normal)?;
    frame.render_texture_from_to(
        source_texture,
        Rectangle::<f64, Buffer>::from_size(source_texture.size().to_f64()),
        output_rect,
        backdrop_regions,
        &[],
        Transform::Normal,
        1.0,
        Some(&program),
        &uniforms,
    )?;
    let sync = frame.finish()?;
    renderer.wait(&sync).map_err(|_| GlesError::SyncInterrupted)?;
    Ok(())
}

fn backdrop_blur_program<'a>(
    renderer: &mut GlesRenderer,
    pipelines: &'a mut MaterialPipelineCache,
) -> Result<&'a GlesTexProgram, GlesError> {
    if pipelines.backdrop_blur.is_none() {
        pipelines.backdrop_blur = Some(renderer.compile_custom_texture_shader(
            BACKDROP_BLUR_SHADER,
            &[
                UniformName::new("tex_size", UniformType::_2f),
                UniformName::new("radius", UniformType::_1f),
            ],
        )?);
    }

    Ok(pipelines.backdrop_blur.as_ref().expect("initialized above"))
}

fn readback_texture_rgba(
    renderer: &mut GlesRenderer,
    texture: &mut GlesTexture,
    output_size: Size<i32, Physical>,
) -> Result<Vec<u8>, GlesError> {
    let region = Rectangle::<i32, Buffer>::from_size((output_size.w, output_size.h).into());
    let framebuffer = renderer.bind(texture)?;
    let mapping = renderer.copy_framebuffer(&framebuffer, region, Fourcc::Abgr8888)?;
    let bytes = renderer.map_texture(&mapping)?;
    Ok(bytes.to_vec())
}

fn output_swapchain_target(
    execution: &OutputExecutionPlan,
    output_id: OutputId,
) -> Option<RenderTargetId> {
    execution.targets.iter().find_map(|(target_id, target_kind)| match target_kind {
        nekoland_ecs::resources::RenderTargetKind::OutputSwapchain(target_output_id)
            if *target_output_id == output_id =>
        {
            Some(*target_id)
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use nekoland_ecs::resources::RenderItemId;

    use super::scene_pass_item_ids_in_presentation_order;

    #[test]
    fn scene_pass_draw_order_is_front_to_back() {
        let item_ids = [RenderItemId(11), RenderItemId(22), RenderItemId(33)];
        assert_eq!(
            scene_pass_item_ids_in_presentation_order(&item_ids).copied().collect::<Vec<_>>(),
            vec![RenderItemId(33), RenderItemId(22), RenderItemId(11)]
        );
    }
}
