use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt::{Display, Formatter};

use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    CompletedScreenshotFrames, CompositorClock, OutputExecutionPlan, OutputFinalTargetPlan,
    OutputPreparedGpuResources, OutputPreparedSceneResources, OutputProcessPlan,
    OutputTargetAllocationPlan, PendingScreenshotRequests, PreparedSceneItem, ProcessRect,
    ProcessShaderKey, ProcessUniformBlock, ProcessUniformValue, RenderColor,
    RenderMaterialFrameState, RenderPassKind, RenderPassPayload, RenderRect,
    RenderTargetAllocationSpec, RenderTargetId, ScreenshotFrame,
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
use smithay::backend::renderer::utils::import_surface_tree;
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
struct ProcessShaderCache {
    programs: BTreeMap<ProcessShaderKey, GlesTexProgram>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CachedSurfaceImport {
    content_version: u64,
    strategy: nekoland_ecs::resources::PreparedSurfaceImportStrategy,
}

#[derive(Debug, Default)]
pub(crate) struct GlesExecutionState {
    outputs: HashMap<OutputId, OutputExecutionCache>,
    process_shaders: ProcessShaderCache,
    surface_imports: BTreeMap<u64, CachedSurfaceImport>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutedOutputTexture {
    pub texture: GlesTexture,
}

#[derive(Debug)]
pub(crate) enum GlesExecutionError {
    Renderer(GlesError),
    MissingExecutionTarget { target_id: RenderTargetId },
    MissingProcessShaderProgram { shader_key: ProcessShaderKey },
}

impl Display for GlesExecutionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Renderer(error) => write!(f, "{error}"),
            Self::MissingExecutionTarget { target_id } => {
                write!(
                    f,
                    "render target {target_id:?} missing from execution cache after allocation"
                )
            }
            Self::MissingProcessShaderProgram { shader_key } => {
                write!(f, "process shader {shader_key:?} missing from cache after initialization")
            }
        }
    }
}

impl Error for GlesExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Renderer(error) => Some(error),
            Self::MissingExecutionTarget { .. } | Self::MissingProcessShaderProgram { .. } => None,
        }
    }
}

impl From<GlesError> for GlesExecutionError {
    fn from(value: GlesError) -> Self {
        Self::Renderer(value)
    }
}

pub(crate) fn execute_output_graph(
    renderer: &mut GlesRenderer,
    state: &mut GlesExecutionState,
    output: &OutputSnapshot,
    execution: &OutputExecutionPlan,
    final_output: Option<&OutputFinalTargetPlan>,
    allocation: Option<&OutputTargetAllocationPlan>,
    prepared_scene: &OutputPreparedSceneResources,
    prepared_gpu: Option<&OutputPreparedGpuResources>,
    process_plan: &OutputProcessPlan,
    _materials: &RenderMaterialFrameState,
    surface_registry: &ProtocolSurfaceRegistry,
    cursor_cache: &mut SoftwareCursorCache,
    config: Option<&CompositorConfig>,
    pending_screenshot_requests: &mut PendingScreenshotRequests,
    completed_screenshots: &mut CompletedScreenshotFrames,
    clock: Option<&CompositorClock>,
) -> Result<Option<ExecutedOutputTexture>, GlesExecutionError> {
    let output_size = Size::from((
        i32::try_from(output.properties.width.max(1)).unwrap_or(i32::MAX),
        i32::try_from(output.properties.height.max(1)).unwrap_or(i32::MAX),
    ));
    let output_rect = Rectangle::from_size(output_size);
    let output_scale = output.properties.scale.max(1);

    let output_cache = state
        .outputs
        .get_mut(&output.output_id)
        .ok_or(GlesExecutionError::MissingExecutionTarget {
            target_id: execution
                .ordered_passes
                .iter()
                .find_map(|pass_id| execution.passes.get(pass_id).map(|pass| pass.output_target))
                .unwrap_or(RenderTargetId(0)),
        })?;

    for pass_id in execution.reachable_passes_in_order() {
        let Some(pass) = execution.passes.get(&pass_id) else {
            continue;
        };

        match pass.kind {
            RenderPassKind::Scene => {
                let target = output_cache
                    .targets
                    .get_mut(&pass.output_target)
                    .ok_or(GlesExecutionError::MissingExecutionTarget {
                        target_id: pass.output_target,
                    })?;
                let built = build_scene_pass_elements(
                    renderer,
                    prepared_scene,
                    prepared_gpu,
                    surface_registry,
                    cursor_cache,
                    config,
                    output_scale,
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
                execute_process_units_for_pass(
                    renderer,
                    &mut state.process_shaders,
                    output_cache,
                    process_plan,
                    allocation,
                    pass_id,
                    output_size,
                    output_rect,
                    output.properties.scale.max(1),
                )?;
            }
            RenderPassKind::PostProcess => {
                execute_process_units_for_pass(
                    renderer,
                    &mut state.process_shaders,
                    output_cache,
                    process_plan,
                    allocation,
                    pass_id,
                    output_size,
                    output_rect,
                    output.properties.scale.max(1),
                )?;
            }
            RenderPassKind::Readback => {
                let RenderPassPayload::Readback(config_payload) = &pass.payload else {
                    continue;
                };
                if config_payload.request_ids.is_empty() {
                    continue;
                }
                let source_target = config_payload.source_target;
                let Some(target) = output_cache.targets.get_mut(&source_target) else {
                    continue;
                };
                let pixels = readback_texture_rgba(renderer, &mut target.texture, output_size)?;
                let frame = clock.map(|clock| clock.frame).unwrap_or_default();
                let uptime_millis = clock
                    .map(|clock| clock.uptime_millis.min(u128::from(u64::MAX)) as u64)
                    .unwrap_or_default();
                for request_id in &config_payload.request_ids {
                    completed_screenshots.push_frame(ScreenshotFrame {
                        request_id: *request_id,
                        output_id: output.output_id,
                        frame,
                        uptime_millis,
                        width: output.properties.width,
                        height: output.properties.height,
                        scale: output_scale,
                        pixels_rgba: pixels.clone(),
                    });
                }
                pending_screenshot_requests.finish_requests(&config_payload.request_ids);
            }
        }
    }

    let Some(final_target) = final_output.map(|plan| plan.present_target_id) else {
        return Ok(None);
    };
    let Some(target) = output_cache.targets.get(&final_target) else {
        return Ok(None);
    };

    Ok(Some(ExecutedOutputTexture { texture: target.texture.clone() }))
}

pub(crate) fn prepare_output_graph_process_shaders(
    renderer: &mut GlesRenderer,
    state: &mut GlesExecutionState,
    prepared_gpu: Option<&OutputPreparedGpuResources>,
    process_plan: &OutputProcessPlan,
) -> Result<(), GlesExecutionError> {
    prewarm_process_shader_programs(renderer, &mut state.process_shaders, prepared_gpu, process_plan)
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
    prepared_scene: &OutputPreparedSceneResources,
    prepared_gpu: Option<&OutputPreparedGpuResources>,
    surface_registry: &ProtocolSurfaceRegistry,
    cursor_cache: &mut SoftwareCursorCache,
    config: Option<&CompositorConfig>,
    output_scale: u32,
    item_ids: &[nekoland_ecs::resources::RenderItemId],
) -> ScenePassBuilt {
    let mut elements = Vec::new();
    let mut backdrop_regions = Vec::new();

    for item_id in scene_pass_item_ids_in_presentation_order(item_ids) {
        let Some(item) = prepared_scene.items.get(item_id) else {
            continue;
        };
        match item {
            PreparedSceneItem::Surface(item) => {
                let import_ready = prepared_gpu
                    .and_then(|prepared_gpu| prepared_gpu.surface_imports.get(&item.surface_id))
                    .map(|prepared_import| {
                        !matches!(
                            prepared_import.strategy,
                            nekoland_ecs::resources::PreparedSurfaceImportStrategy::Unsupported
                        )
                    })
                    .unwrap_or(item.import_ready);
                if !import_ready {
                    continue;
                }
                let Some(clip_rect) = render_rect_to_physical(&item.visible_rect, output_scale)
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
                        (item.x, item.y),
                        output_scale as f64,
                        item.opacity,
                        Kind::Unspecified,
                    )
                    .into_iter()
                    .filter_map(|element| {
                        CropRenderElement::from_element(element, output_scale as f64, clip_rect)
                    })
                    .map(CommonGlesRenderElement::from),
                );
            }
            PreparedSceneItem::SolidRect(item) => {
                let Some(visible_rect) = render_rect_to_physical(&item.visible_rect, output_scale)
                else {
                    continue;
                };
                elements.push(
                    SolidColorRenderElement::new(
                        Id::new(),
                        visible_rect,
                        CommitCounter::default(),
                        render_color_to_color32f(item.color, item.opacity),
                        Kind::Unspecified,
                    )
                    .into(),
                );
            }
            PreparedSceneItem::Backdrop(item) => {
                let Some(visible_rect) = render_rect_to_physical(&item.visible_rect, output_scale)
                else {
                    continue;
                };
                backdrop_regions.push(visible_rect);
            }
            PreparedSceneItem::CursorNamed(item) => {
                let theme = config.map(|config| config.cursor_theme.as_str()).unwrap_or("default");
                match cursor_cache.render_element(
                    renderer,
                    theme,
                    &item.icon_name,
                    item.scale.max(1),
                    item.x,
                    item.y,
                ) {
                    Ok(element) => elements.push(element.into()),
                    Err(error) => {
                        tracing::warn!(error = %error, "failed to upload software cursor");
                    }
                }
            }
            PreparedSceneItem::CursorSurface(item) => {
                let import_ready = prepared_gpu
                    .and_then(|prepared_gpu| prepared_gpu.surface_imports.get(&item.surface_id))
                    .map(|prepared_import| {
                        !matches!(
                            prepared_import.strategy,
                            nekoland_ecs::resources::PreparedSurfaceImportStrategy::Unsupported
                        )
                    })
                    .unwrap_or(item.import_ready);
                if !import_ready {
                    continue;
                }
                let Some(clip_rect) = render_rect_to_physical(&item.visible_rect, output_scale)
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
                        (item.x, item.y),
                        output_scale as f64,
                        item.opacity,
                        Kind::Cursor,
                    )
                    .into_iter()
                    .filter_map(|element| {
                        CropRenderElement::from_element(element, output_scale as f64, clip_rect)
                    })
                    .map(CommonGlesRenderElement::from),
                );
            }
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
    allocation: Option<&RenderTargetAllocationSpec>,
    output_size: Size<i32, Physical>,
) -> Result<&'a mut CachedExecutionTarget, GlesExecutionError> {
    let target_size = allocation
        .map(|allocation| {
            Size::from((
                i32::try_from(allocation.width.max(1)).unwrap_or(i32::MAX),
                i32::try_from(allocation.height.max(1)).unwrap_or(i32::MAX),
            ))
        })
        .unwrap_or(output_size);
    let recreate = cache.targets.get(&target_id).is_none_or(|target| target.size != target_size);
    if recreate {
        let texture = Offscreen::<GlesTexture>::create_buffer(
            renderer,
            Fourcc::Abgr8888,
            Size::<i32, Buffer>::from((target_size.w, target_size.h)),
        )?;
        cache.targets.insert(
            target_id,
            CachedExecutionTarget { texture, size: target_size, backdrop_regions: Vec::new() },
        );
    }

    cache
        .targets
        .get_mut(&target_id)
        .ok_or(GlesExecutionError::MissingExecutionTarget { target_id })
}

fn composite_texture_to_target(
    renderer: &mut GlesRenderer,
    source_texture: &GlesTexture,
    dest_texture: &mut GlesTexture,
    output_size: Size<i32, Physical>,
    sample_rect: Rectangle<i32, Physical>,
    output_rect: Rectangle<i32, Physical>,
) -> Result<(), GlesError> {
    let mut framebuffer = renderer.bind(dest_texture)?;
    let mut frame = renderer.render(&mut framebuffer, output_size, Transform::Normal)?;
    frame.clear(Color32F::new(0.0, 0.0, 0.0, 0.0), &[output_rect])?;
    frame.render_texture_from_to(
        source_texture,
        physical_rect_to_buffer_rect(sample_rect),
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

pub(crate) fn prepare_output_graph_targets(
    renderer: &mut GlesRenderer,
    state: &mut GlesExecutionState,
    output: &OutputSnapshot,
    execution: &OutputExecutionPlan,
    allocation: Option<&OutputTargetAllocationPlan>,
) -> Result<(), GlesExecutionError> {
    let output_size = Size::from((
        i32::try_from(output.properties.width.max(1)).unwrap_or(i32::MAX),
        i32::try_from(output.properties.height.max(1)).unwrap_or(i32::MAX),
    ));
    let output_cache = state.outputs.entry(output.output_id).or_default();
    output_cache.targets.retain(|target_id, _| {
        allocation.is_some_and(|allocation| allocation.targets.contains_key(target_id))
            || execution.targets.contains_key(target_id)
    });
    for target in output_cache.targets.values_mut() {
        target.backdrop_regions.clear();
    }
    for target_id in execution.targets.keys().copied() {
        let _ = ensure_target(
            renderer,
            output_cache,
            target_id,
            allocation.and_then(|allocation| allocation.targets.get(&target_id)),
            output_size,
        )?;
    }
    Ok(())
}

pub(crate) fn prepare_output_surface_imports(
    renderer: &mut GlesRenderer,
    state: &mut GlesExecutionState,
    prepared_scene: &OutputPreparedSceneResources,
    prepared_gpu: Option<&OutputPreparedGpuResources>,
    surface_registry: &ProtocolSurfaceRegistry,
) -> Result<(), GlesExecutionError> {
    let importable = importable_surface_imports(prepared_scene, prepared_gpu);
    let needed_surface_ids = importable.iter().map(|prepared_import| prepared_import.surface_id).collect::<BTreeSet<_>>();
    state.surface_imports.retain(|surface_id, _| needed_surface_ids.contains(surface_id));

    for prepared_import in importable {
        if !needs_surface_reimport(state.surface_imports.get(&prepared_import.surface_id), &prepared_import)
        {
            continue;
        }
        let surface_id = prepared_import.surface_id;
        let Some(surface) = surface_registry.surface(surface_id) else {
            continue;
        };
        import_surface_tree(renderer, surface)?;
        state.surface_imports.insert(
            surface_id,
            CachedSurfaceImport {
                content_version: prepared_import.descriptor.content_version,
                strategy: prepared_import.strategy,
            },
        );
    }

    Ok(())
}

fn prewarm_process_shader_programs(
    renderer: &mut GlesRenderer,
    shaders: &mut ProcessShaderCache,
    prepared_gpu: Option<&OutputPreparedGpuResources>,
    output_process: &OutputProcessPlan,
) -> Result<(), GlesExecutionError> {
    let mut required = prepared_gpu
        .map(|prepared_gpu| prepared_gpu.process_shaders.clone())
        .unwrap_or_else(|| {
            output_process
                .units
                .values()
                .map(|unit| unit.shader_key.clone())
                .collect::<BTreeSet<_>>()
        });
    required.remove(&ProcessShaderKey::Passthrough);
    required.remove(&ProcessShaderKey::BuiltinComposite);

    for shader_key in required {
        let _ = process_shader_program(renderer, shaders, &shader_key)?;
    }

    Ok(())
}

fn execute_process_units_for_pass(
    renderer: &mut GlesRenderer,
    shaders: &mut ProcessShaderCache,
    output_cache: &mut OutputExecutionCache,
    output_process: &OutputProcessPlan,
    _allocation: Option<&OutputTargetAllocationPlan>,
    pass_id: nekoland_ecs::resources::RenderPassId,
    output_size: Size<i32, Physical>,
    output_rect: Rectangle<i32, Physical>,
    output_scale: u32,
) -> Result<(), GlesExecutionError> {
    for unit in output_process.units_for_pass(pass_id) {
        let source_texture =
            output_cache.targets.get(&unit.input.target_id).map(|target| target.texture.clone());
        let Some(source_texture) = source_texture else {
            continue;
        };
        let target = output_cache
            .targets
            .get_mut(&unit.output.target_id)
            .ok_or(GlesExecutionError::MissingExecutionTarget {
                target_id: unit.output.target_id,
            })?;
        execute_process_unit(
            renderer,
            shaders,
            unit,
            &source_texture,
            &mut target.texture,
            output_size,
            output_rect,
            output_scale,
        )?;
    }

    Ok(())
}

fn execute_process_unit(
    renderer: &mut GlesRenderer,
    shaders: &mut ProcessShaderCache,
    unit: &nekoland_ecs::resources::ProcessUnit,
    source_texture: &GlesTexture,
    dest_texture: &mut GlesTexture,
    output_size: Size<i32, Physical>,
    default_output_rect: Rectangle<i32, Physical>,
    output_scale: u32,
) -> Result<(), GlesExecutionError> {
    let sample_rect = unit
        .input
        .sample_rect
        .and_then(|rect| process_rect_to_physical(&rect, output_scale))
        .unwrap_or(default_output_rect);
    let output_rect = unit
        .output
        .output_rect
        .and_then(|rect| process_rect_to_physical(&rect, output_scale))
        .unwrap_or(default_output_rect);

    match &unit.shader_key {
        ProcessShaderKey::Material(key)
            if key.material == nekoland_ecs::resources::RenderMaterialKind::BackdropBlur =>
        {
            let backdrop_regions = unit
                .process_regions
                .iter()
                .filter_map(|rect| process_rect_to_physical(rect, output_scale))
                .collect::<Vec<_>>();
            execute_backdrop_blur_pass(
                renderer,
                shaders,
                &unit.uniforms,
                source_texture,
                dest_texture,
                &backdrop_regions,
                output_size,
                sample_rect,
                output_rect,
            )
        }
        _ => {
            composite_texture_to_target(
                renderer,
                source_texture,
                dest_texture,
                output_size,
                sample_rect,
                output_rect,
            )?;
            Ok(())
        }
    }
}

fn importable_surface_imports(
    prepared_scene: &OutputPreparedSceneResources,
    prepared_gpu: Option<&OutputPreparedGpuResources>,
) -> Vec<nekoland_ecs::resources::PreparedSurfaceImport> {
    prepared_scene
        .ordered_items
        .iter()
        .filter_map(|item_id| prepared_scene.items.get(item_id))
        .filter_map(|item| match item {
            PreparedSceneItem::Surface(item) if item.import_ready => Some(item.surface_id),
            PreparedSceneItem::CursorSurface(item) if item.import_ready => Some(item.surface_id),
            _ => None,
        })
        .filter_map(|surface_id| {
            prepared_gpu
                .and_then(|prepared_gpu| prepared_gpu.surface_imports.get(&surface_id))
                .cloned()
                .filter(|prepared_import| {
                    !matches!(
                        prepared_import.strategy,
                        nekoland_ecs::resources::PreparedSurfaceImportStrategy::Unsupported
                    )
                })
        })
        .collect()
}

fn needs_surface_reimport(
    cached: Option<&CachedSurfaceImport>,
    prepared_import: &nekoland_ecs::resources::PreparedSurfaceImport,
) -> bool {
    cached.is_none_or(|cached| {
        cached.content_version != prepared_import.descriptor.content_version
            || cached.strategy != prepared_import.strategy
    })
}

fn execute_backdrop_blur_pass(
    renderer: &mut GlesRenderer,
    shaders: &mut ProcessShaderCache,
    uniforms: &ProcessUniformBlock,
    source_texture: &GlesTexture,
    dest_texture: &mut GlesTexture,
    backdrop_regions: &[Rectangle<i32, Physical>],
    output_size: Size<i32, Physical>,
    sample_rect: Rectangle<i32, Physical>,
    output_rect: Rectangle<i32, Physical>,
) -> Result<(), GlesExecutionError> {
    composite_texture_to_target(
        renderer,
        source_texture,
        dest_texture,
        output_size,
        sample_rect,
        output_rect,
    )?;

    if backdrop_regions.is_empty() {
        return Ok(());
    }

    let radius = uniforms
        .values
        .get("radius")
        .and_then(|value| match value {
            ProcessUniformValue::Float(value) => Some(*value),
            _ => None,
        })
        .unwrap_or(12.0);
    let program = process_shader_program(
        renderer,
        shaders,
        &ProcessShaderKey::Material(
            nekoland_ecs::resources::RenderMaterialPipelineKey::post_process(
                nekoland_ecs::resources::RenderMaterialKind::BackdropBlur,
            ),
        ),
    )?
    .clone();
    let shader_uniforms = vec![
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
        &shader_uniforms,
    )?;
    let sync = frame.finish()?;
    renderer.wait(&sync).map_err(|_| GlesError::SyncInterrupted)?;
    Ok(())
}

fn process_shader_program<'a>(
    renderer: &mut GlesRenderer,
    shaders: &'a mut ProcessShaderCache,
    shader_key: &ProcessShaderKey,
) -> Result<&'a GlesTexProgram, GlesExecutionError> {
    if !shaders.programs.contains_key(shader_key) {
        let program = match shader_key {
            ProcessShaderKey::Material(key)
                if key.material == nekoland_ecs::resources::RenderMaterialKind::BackdropBlur =>
            {
                renderer.compile_custom_texture_shader(
                    BACKDROP_BLUR_SHADER,
                    &[
                        UniformName::new("tex_size", UniformType::_2f),
                        UniformName::new("radius", UniformType::_1f),
                    ],
                )?
            }
            _ => return Err(GlesExecutionError::Renderer(GlesError::ShaderCompileError)),
        };
        shaders.programs.insert(shader_key.clone(), program);
    }

    shaders.programs.get(shader_key).ok_or_else(|| {
        GlesExecutionError::MissingProcessShaderProgram { shader_key: shader_key.clone() }
    })
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

fn physical_rect_to_buffer_rect(rect: Rectangle<i32, Physical>) -> Rectangle<f64, Buffer> {
    Rectangle::new(
        (f64::from(rect.loc.x), f64::from(rect.loc.y)).into(),
        (f64::from(rect.size.w), f64::from(rect.size.h)).into(),
    )
}

fn process_rect_to_physical(rect: &ProcessRect, scale: u32) -> Option<Rectangle<i32, Physical>> {
    let scale = i32::try_from(scale.max(1)).ok()?;
    let width = i32::try_from(rect.width).ok()?.checked_mul(scale)?;
    let height = i32::try_from(rect.height).ok()?.checked_mul(scale)?;
    let x = rect.x.checked_mul(scale)?;
    let y = rect.y.checked_mul(scale)?;
    Some(Rectangle::new((x, y).into(), (width, height).into()))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nekoland_ecs::resources::{
        OutputPreparedGpuResources, OutputPreparedSceneResources, PreparedSceneItem,
        PreparedSurfaceCursorSceneItem, PreparedSurfaceImport, PreparedSurfaceImportStrategy,
        PreparedSurfaceSceneItem, RenderItemId, RenderRect, SurfaceTextureImportDescriptor,
    };

    use super::{
        CachedSurfaceImport, importable_surface_imports, needs_surface_reimport,
        scene_pass_item_ids_in_presentation_order,
    };

    #[test]
    fn scene_pass_draw_order_is_front_to_back() {
        let item_ids = [RenderItemId(11), RenderItemId(22), RenderItemId(33)];
        assert_eq!(
            scene_pass_item_ids_in_presentation_order(&item_ids).copied().collect::<Vec<_>>(),
            vec![RenderItemId(33), RenderItemId(22), RenderItemId(11)]
        );
    }

    #[test]
    fn importable_surface_imports_skip_unsupported_or_not_ready_surfaces() {
        let prepared_scene = OutputPreparedSceneResources {
            items: BTreeMap::from([
                (
                    RenderItemId(1),
                    PreparedSceneItem::Surface(PreparedSurfaceSceneItem {
                        surface_id: 11,
                        surface_kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                        x: 0,
                        y: 0,
                        visible_rect: RenderRect { x: 0, y: 0, width: 10, height: 10 },
                        opacity: 1.0,
                        import_ready: true,
                    }),
                ),
                (
                    RenderItemId(2),
                    PreparedSceneItem::Surface(PreparedSurfaceSceneItem {
                        surface_id: 22,
                        surface_kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                        x: 0,
                        y: 0,
                        visible_rect: RenderRect { x: 0, y: 0, width: 10, height: 10 },
                        opacity: 1.0,
                        import_ready: false,
                    }),
                ),
                (
                    RenderItemId(3),
                    PreparedSceneItem::CursorSurface(PreparedSurfaceCursorSceneItem {
                        surface_id: 33,
                        x: 0,
                        y: 0,
                        visible_rect: RenderRect { x: 0, y: 0, width: 5, height: 5 },
                        opacity: 1.0,
                        import_ready: true,
                    }),
                ),
            ]),
            ordered_items: vec![RenderItemId(1), RenderItemId(2), RenderItemId(3)],
        };
        let prepared_gpu = OutputPreparedGpuResources {
            surface_imports: BTreeMap::from([
                (
                    11,
                    PreparedSurfaceImport {
                        surface_id: 11,
                        descriptor: SurfaceTextureImportDescriptor {
                            surface_id: 11,
                            surface_kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                            buffer_source:
                                nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                            dmabuf_format: None,
                            import_strategy:
                                nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                            target_outputs: Default::default(),
                            content_version: 1,
                            attached: true,
                            scale: 1,
                        },
                        strategy: PreparedSurfaceImportStrategy::ShmUpload,
                    },
                ),
                (
                    22,
                    PreparedSurfaceImport {
                        surface_id: 22,
                        descriptor: SurfaceTextureImportDescriptor {
                            surface_id: 22,
                            surface_kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                            buffer_source:
                                nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                            dmabuf_format: None,
                            import_strategy:
                                nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                            target_outputs: Default::default(),
                            content_version: 1,
                            attached: true,
                            scale: 1,
                        },
                        strategy: PreparedSurfaceImportStrategy::ShmUpload,
                    },
                ),
                (
                    33,
                    PreparedSurfaceImport {
                        surface_id: 33,
                        descriptor: SurfaceTextureImportDescriptor {
                            surface_id: 33,
                            surface_kind: nekoland_ecs::resources::PlatformSurfaceKind::Cursor,
                            buffer_source:
                                nekoland_ecs::resources::PlatformSurfaceBufferSource::Unknown,
                            dmabuf_format: None,
                            import_strategy:
                                nekoland_ecs::resources::PlatformSurfaceImportStrategy::Unsupported,
                            target_outputs: Default::default(),
                            content_version: 1,
                            attached: true,
                            scale: 1,
                        },
                        strategy: PreparedSurfaceImportStrategy::Unsupported,
                    },
                ),
            ]),
            ..Default::default()
        };

        let imports = importable_surface_imports(&prepared_scene, Some(&prepared_gpu));
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].surface_id, 11);
    }

    #[test]
    fn importable_surface_imports_include_external_texture_strategy() {
        let prepared_scene = OutputPreparedSceneResources {
            items: BTreeMap::from([(
                RenderItemId(1),
                PreparedSceneItem::Surface(PreparedSurfaceSceneItem {
                    surface_id: 44,
                    surface_kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                    x: 0,
                    y: 0,
                    visible_rect: RenderRect { x: 0, y: 0, width: 10, height: 10 },
                    opacity: 1.0,
                    import_ready: true,
                }),
            )]),
            ordered_items: vec![RenderItemId(1)],
        };
        let prepared_gpu = OutputPreparedGpuResources {
            surface_imports: BTreeMap::from([(
                44,
                PreparedSurfaceImport {
                    surface_id: 44,
                    descriptor: SurfaceTextureImportDescriptor {
                        surface_id: 44,
                        surface_kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                        buffer_source:
                            nekoland_ecs::resources::PlatformSurfaceBufferSource::DmaBuf,
                        dmabuf_format: Some(nekoland_ecs::resources::PlatformDmabufFormat {
                            code: 875713112,
                            modifier: 0,
                        }),
                        import_strategy: nekoland_ecs::resources::PlatformSurfaceImportStrategy::ExternalTextureImport,
                        target_outputs: Default::default(),
                        content_version: 1,
                        attached: true,
                        scale: 1,
                    },
                    strategy: PreparedSurfaceImportStrategy::ExternalTextureImport,
                },
            )]),
            ..Default::default()
        };

        let imports = importable_surface_imports(&prepared_scene, Some(&prepared_gpu));
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].surface_id, 44);
        assert_eq!(imports[0].strategy, PreparedSurfaceImportStrategy::ExternalTextureImport);
    }

    #[test]
    fn needs_surface_reimport_only_when_strategy_or_content_changes() {
        let prepared_import = PreparedSurfaceImport {
            surface_id: 11,
            descriptor: SurfaceTextureImportDescriptor {
                surface_id: 11,
                surface_kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                dmabuf_format: None,
                import_strategy: nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                target_outputs: Default::default(),
                content_version: 7,
                attached: true,
                scale: 1,
            },
            strategy: PreparedSurfaceImportStrategy::ShmUpload,
        };
        assert!(needs_surface_reimport(None, &prepared_import));
        assert!(!needs_surface_reimport(
            Some(&CachedSurfaceImport {
                content_version: 7,
                strategy: PreparedSurfaceImportStrategy::ShmUpload,
            }),
            &prepared_import,
        ));
        assert!(needs_surface_reimport(
            Some(&CachedSurfaceImport {
                content_version: 6,
                strategy: PreparedSurfaceImportStrategy::ShmUpload,
            }),
            &prepared_import,
        ));
        assert!(needs_surface_reimport(
            Some(&CachedSurfaceImport {
                content_version: 7,
                strategy: PreparedSurfaceImportStrategy::Unsupported,
            }),
            &prepared_import,
        ));
    }

    #[test]
    fn surface_import_cache_can_drop_surfaces_that_are_no_longer_needed() {
        let needed = BTreeSet::from([11_u64, 33_u64]);
        let mut cached = BTreeMap::from([
            (
                11_u64,
                CachedSurfaceImport {
                    content_version: 1,
                    strategy: PreparedSurfaceImportStrategy::ShmUpload,
                },
            ),
            (
                22_u64,
                CachedSurfaceImport {
                    content_version: 1,
                    strategy: PreparedSurfaceImportStrategy::ShmUpload,
                },
            ),
            (
                33_u64,
                CachedSurfaceImport {
                    content_version: 2,
                    strategy: PreparedSurfaceImportStrategy::DmaBufImport,
                },
            ),
        ]);

        cached.retain(|surface_id, _| needed.contains(surface_id));

        assert_eq!(cached.len(), 2);
        assert!(cached.contains_key(&11));
        assert!(!cached.contains_key(&22));
        assert!(cached.contains_key(&33));
    }
}
