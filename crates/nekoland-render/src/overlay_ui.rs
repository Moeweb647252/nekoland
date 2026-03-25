//! Overlay-UI scene synchronization and text rasterization.
//!
//! This module bridges shell-owned overlay UI frames into compositor-owned scene entries. It is
//! intentionally state-heavy, so field-level rustdoc is kept minimal in favor of module and
//! function-level documentation.
#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use bevy_ecs::prelude::{Query, Res, ResMut, Resource};
use nekoland_ecs::components::{OutputId, OutputProperties};
use nekoland_ecs::resources::{
    CompositorSceneEntry, CompositorSceneEntryId, CompositorSceneState, OverlayUiPrimitive,
    OverlayUiPrimitiveId, QuadContent, QuadRasterImage, RenderColor, RenderItemInstance,
    RenderRect, RenderSceneRole, ShellRenderInput,
};

const OVERLAY_TEXT_CACHE_LIMIT: usize = 256;

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct OverlayUiSceneSyncState {
    pub output_entries: BTreeMap<OutputId, BTreeSet<CompositorSceneEntryId>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OverlayTextCacheKey {
    text: String,
    font_size_bits: u32,
    scale: u32,
    color: [u8; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum OverlayFontSource {
    Auto,
    Bytes(Vec<u8>),
}

impl Default for OverlayFontSource {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Resource, Default)]
pub struct OverlayTextRasterizerState {
    font_source: OverlayFontSource,
    font: Option<fontdue::Font>,
    font_path: Option<PathBuf>,
    attempted_load: bool,
    cache: BTreeMap<OverlayTextCacheKey, QuadRasterImage>,
    cache_order: VecDeque<OverlayTextCacheKey>,
    test_pattern: Option<QuadRasterImage>,
}

impl OverlayTextRasterizerState {
    #[cfg(test)]
    pub(crate) fn with_test_pattern(image: QuadRasterImage) -> Self {
        Self { test_pattern: Some(image), ..Self::default() }
    }

    fn rasterize_text(
        &mut self,
        text: &str,
        font_size: f32,
        scale: u32,
        color: RenderColor,
    ) -> Option<QuadRasterImage> {
        if text.is_empty() {
            return None;
        }

        if let Some(pattern) = &self.test_pattern {
            return Some(pattern.clone());
        }

        let key = OverlayTextCacheKey {
            text: text.to_owned(),
            font_size_bits: font_size.to_bits(),
            scale,
            color: [color.r, color.g, color.b, color.a],
        };
        if let Some(image) = self.cache.get(&key).cloned() {
            return Some(image);
        }

        let font = self.ensure_font_loaded()?;
        let image = rasterize_text_image(font, text, font_size, scale, color)?;
        self.insert_cached_image(key, image.clone());
        Some(image)
    }

    fn insert_cached_image(&mut self, key: OverlayTextCacheKey, image: QuadRasterImage) {
        if self.cache.contains_key(&key) {
            self.cache.insert(key, image);
            return;
        }

        self.cache.insert(key.clone(), image);
        self.cache_order.push_back(key);
        while self.cache_order.len() > OVERLAY_TEXT_CACHE_LIMIT {
            let Some(oldest) = self.cache_order.pop_front() else {
                break;
            };
            self.cache.remove(&oldest);
        }
    }

    fn ensure_font_loaded(&mut self) -> Option<&fontdue::Font> {
        if self.font.is_some() {
            return self.font.as_ref();
        }
        if self.attempted_load {
            return None;
        }
        self.attempted_load = true;

        let font_bytes = match &self.font_source {
            OverlayFontSource::Bytes(bytes) => Some(bytes.clone()),
            OverlayFontSource::Auto => discover_system_font().map(|(path, bytes)| {
                self.font_path = Some(path);
                bytes
            }),
        }?;
        match fontdue::Font::from_bytes(font_bytes, fontdue::FontSettings::default()) {
            Ok(font) => {
                self.font = Some(font);
                self.font.as_ref()
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to load overlay UI font");
                None
            }
        }
    }
}

/// Synchronizes shell-owned overlay UI primitives into compositor-scene entries.
pub fn sync_overlay_ui_scene_state_system(
    shell_render_input: Res<'_, ShellRenderInput>,
    outputs: Query<'_, '_, (&'static OutputId, &'static OutputProperties)>,
    mut compositor_scene: ResMut<'_, CompositorSceneState>,
    mut sync_state: ResMut<'_, OverlayUiSceneSyncState>,
    mut text_rasterizer: ResMut<'_, OverlayTextRasterizerState>,
) {
    let mut current_entries = BTreeMap::<OutputId, BTreeSet<CompositorSceneEntryId>>::new();
    let overlay_ui = &shell_render_input.overlay_ui;
    let output_scales = outputs
        .iter()
        .map(|(output_id, properties)| (*output_id, properties.scale.max(1)))
        .collect::<BTreeMap<_, _>>();

    for (output_id, output_frame) in &overlay_ui.outputs {
        let output_scene = compositor_scene.outputs.entry(*output_id).or_default();
        let output_scale = output_scales.get(output_id).copied().unwrap_or(1);
        let mut primitives = output_frame.primitives.iter().collect::<Vec<_>>();
        primitives.sort_by_key(|primitive| {
            (primitive.layer(), primitive.z_index(), primitive.id().clone())
        });

        let mut touched = false;
        for primitive in primitives {
            let Some((entry_id, entry)) =
                overlay_ui_scene_entry(*output_id, primitive, output_scale, &mut text_rasterizer)
            else {
                continue;
            };
            current_entries.entry(*output_id).or_default().insert(entry_id);
            output_scene.insert(entry_id, entry);
            touched = true;
        }

        if touched {
            output_scene.sort_by_z_index();
        }
    }

    for (output_id, previous_entry_ids) in &sync_state.output_entries {
        let retained = current_entries.get(output_id);
        let should_remove_output = {
            let Some(output_scene) = compositor_scene.outputs.get_mut(output_id) else {
                continue;
            };

            for entry_id in previous_entry_ids {
                if retained.is_some_and(|retained| retained.contains(entry_id)) {
                    continue;
                }
                output_scene.remove(*entry_id);
            }
            if output_scene.items.is_empty() {
                true
            } else {
                output_scene.sort_by_z_index();
                false
            }
        };

        if should_remove_output {
            compositor_scene.outputs.remove(output_id);
        }
    }

    sync_state.output_entries = current_entries;
}

fn overlay_ui_scene_entry(
    output_id: OutputId,
    primitive: &OverlayUiPrimitive,
    output_scale: u32,
    text_rasterizer: &mut OverlayTextRasterizerState,
) -> Option<(CompositorSceneEntryId, CompositorSceneEntry)> {
    match primitive {
        OverlayUiPrimitive::Surface(surface) => {
            let entry_id = overlay_ui_entry_id(output_id, &surface.id);
            let entry = CompositorSceneEntry::surface(
                surface.surface_id,
                RenderItemInstance {
                    rect: surface.rect,
                    opacity: surface.opacity,
                    clip_rect: surface.clip_rect,
                    z_index: surface.layer.z_index_bias().saturating_add(surface.z_index),
                    scene_role: RenderSceneRole::Overlay,
                },
            );
            Some((entry_id, entry))
        }
        OverlayUiPrimitive::Panel(panel) => {
            let entry_id = overlay_ui_entry_id(output_id, &panel.id);
            let entry = CompositorSceneEntry::quad(
                QuadContent::SolidColor { color: panel.color },
                RenderItemInstance {
                    rect: panel.rect,
                    opacity: panel.opacity,
                    clip_rect: panel.clip_rect,
                    z_index: panel.layer.z_index_bias().saturating_add(panel.z_index),
                    scene_role: RenderSceneRole::Overlay,
                },
            );
            Some((entry_id, entry))
        }
        OverlayUiPrimitive::Text(text) => {
            let image = text_rasterizer.rasterize_text(
                text.text.as_str(),
                text.font_size,
                output_scale.max(1),
                text.color,
            )?;
            let scale = image.scale.max(1);
            let rect = RenderRect {
                x: text.x,
                y: text.y,
                width: image.width.div_ceil(scale),
                height: image.height.div_ceil(scale),
            };
            let entry_id = overlay_ui_entry_id(output_id, &text.id);
            let entry = CompositorSceneEntry::quad(
                QuadContent::RasterImage { image },
                RenderItemInstance {
                    rect,
                    opacity: text.opacity,
                    clip_rect: text.clip_rect,
                    z_index: text.layer.z_index_bias().saturating_add(text.z_index),
                    scene_role: RenderSceneRole::Overlay,
                },
            );
            Some((entry_id, entry))
        }
    }
}

fn overlay_ui_entry_id(
    output_id: OutputId,
    primitive_id: &OverlayUiPrimitiveId,
) -> CompositorSceneEntryId {
    let mut hasher = DefaultHasher::new();
    "overlay_ui".hash(&mut hasher);
    output_id.hash(&mut hasher);
    primitive_id.hash(&mut hasher);
    CompositorSceneEntryId((1_u64 << 63) | (hasher.finish() & !(1_u64 << 63)))
}

fn rasterize_text_image(
    font: &fontdue::Font,
    text: &str,
    font_size: f32,
    scale: u32,
    color: RenderColor,
) -> Option<QuadRasterImage> {
    use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};

    let scaled_font_size = font_size.max(1.0) * scale.max(1) as f32;
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings::default());
    layout.append(&[font], &TextStyle::new(text, scaled_font_size, 0));

    let glyphs = layout.glyphs();
    if glyphs.is_empty() {
        return None;
    }

    let width = glyphs
        .iter()
        .map(|glyph| glyph.x.ceil().max(0.0) as u32 + glyph.width as u32)
        .max()
        .unwrap_or_default();
    let height = glyphs
        .iter()
        .map(|glyph| glyph.y.ceil().max(0.0) as u32 + glyph.height as u32)
        .max()
        .unwrap_or_default();
    if width == 0 || height == 0 {
        return None;
    }

    let mut pixels = vec![0_u8; usize::try_from(width.checked_mul(height)?.checked_mul(4)?).ok()?];
    for glyph in glyphs {
        let (metrics, bitmap) = font.rasterize_config(glyph.key);
        if metrics.width == 0 || metrics.height == 0 {
            continue;
        }

        let x = glyph.x.round().max(0.0) as usize;
        let y = glyph.y.round().max(0.0) as usize;
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let src_alpha = bitmap[row * metrics.width + col];
                if src_alpha == 0 {
                    continue;
                }
                let dst_x = x + col;
                let dst_y = y + row;
                if dst_x >= width as usize || dst_y >= height as usize {
                    continue;
                }
                let dst = (dst_y * width as usize + dst_x) * 4;
                pixels[dst] = color.r;
                pixels[dst + 1] = color.g;
                pixels[dst + 2] = color.b;
                pixels[dst + 3] = ((u16::from(src_alpha) * u16::from(color.a)) / 255) as u8;
            }
        }
    }

    Some(QuadRasterImage { width, height, scale: scale.max(1), pixels_rgba: pixels })
}

fn discover_system_font() -> Option<(PathBuf, Vec<u8>)> {
    let mut candidates = collect_font_candidates();
    candidates.sort_by_key(|path| font_priority(path));

    for path in candidates {
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        return Some((path, bytes));
    }

    None
}

fn collect_font_candidates() -> Vec<PathBuf> {
    let mut roots =
        vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(&home).join(".fonts"));
        roots.push(PathBuf::from(home).join(".local/share/fonts"));
    }

    let mut candidates = Vec::new();
    for root in roots {
        collect_font_candidates_from_dir(&root, &mut candidates);
    }
    candidates
}

fn collect_font_candidates_from_dir(path: &Path, candidates: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_candidates_from_dir(&path, candidates);
            continue;
        }
        let Some(ext) = path.extension().and_then(OsStr::to_str) else {
            continue;
        };
        if matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
            candidates.push(path);
        }
    }
}

fn font_priority(path: &Path) -> (u8, String) {
    let filename =
        path.file_name().and_then(OsStr::to_str).unwrap_or_default().to_ascii_lowercase();
    let preferred = [
        "notosanscjk",
        "notosans",
        "sourcehansans",
        "wenquanyi",
        "droidsansfallback",
        "dejavusans",
        "liberationsans",
    ];
    let priority = preferred
        .iter()
        .position(|needle| filename.contains(needle))
        .map(|index| index as u8)
        .unwrap_or(u8::MAX);
    (priority, filename)
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputPlacement, OutputProperties, OutputViewport, OutputWorkArea,
    };
    use nekoland_ecs::resources::{
        CompositorSceneState, OverlayUiLayer, QuadContent, QuadRasterImage, RenderColor,
        RenderRect, ShellRenderInput,
    };

    use super::{
        OverlayTextRasterizerState, OverlayUiSceneSyncState, sync_overlay_ui_scene_state_system,
    };

    #[test]
    fn panel_primitives_sync_into_overlay_scene_entries() {
        let mut app = NekolandApp::new("overlay-ui-panel-sync-test");
        let output = app
            .inner_mut()
            .world_mut()
            .spawn((
                nekoland_ecs::components::OutputId(7),
                OutputDevice { name: "Virtual-1".to_owned(), ..OutputDevice::default() },
                OutputProperties { scale: 1, ..OutputProperties::default() },
                OutputViewport::default(),
                nekoland_ecs::components::OutputPlacement::default(),
                OutputWorkArea::default(),
            ))
            .id();
        let _ = output;
        app.inner_mut()
            .init_resource::<ShellRenderInput>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<OverlayUiSceneSyncState>()
            .init_resource::<OverlayTextRasterizerState>()
            .add_systems(RenderSchedule, sync_overlay_ui_scene_state_system);

        app.inner_mut()
            .world_mut()
            .resource_mut::<ShellRenderInput>()
            .overlay_ui
            .output(OutputId(7))
            .panel(
                "panel",
                OverlayUiLayer::Foreground,
                RenderRect { x: 5, y: 6, width: 20, height: 30 },
                None,
                RenderColor { r: 1, g: 2, b: 3, a: 255 },
                0.5,
                7,
            );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let scene = &app.inner().world().resource::<CompositorSceneState>().outputs[&OutputId(7)];
        let entry = scene
            .iter_ordered()
            .next()
            .map(|(_, entry)| entry)
            .expect("expected one panel overlay entry");
        let nekoland_ecs::resources::CompositorSceneItem::Quad { content } = &entry.item else {
            panic!("expected quad scene item");
        };
        assert_eq!(
            *content,
            QuadContent::SolidColor { color: RenderColor { r: 1, g: 2, b: 3, a: 255 } }
        );
    }

    #[test]
    fn text_primitives_rasterize_into_overlay_quads() {
        let mut app = NekolandApp::new("overlay-ui-text-sync-test");
        app.inner_mut().world_mut().spawn((
            nekoland_ecs::components::OutputId(3),
            OutputDevice { name: "Virtual-1".to_owned(), ..OutputDevice::default() },
            OutputProperties { scale: 2, ..OutputProperties::default() },
            OutputViewport::default(),
            OutputPlacement::default(),
            OutputWorkArea::default(),
        ));
        app.inner_mut()
            .init_resource::<ShellRenderInput>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<OverlayUiSceneSyncState>()
            .insert_resource(OverlayTextRasterizerState::with_test_pattern(QuadRasterImage {
                width: 12,
                height: 8,
                scale: 2,
                pixels_rgba: vec![255; 12 * 8 * 4],
            }))
            .add_systems(RenderSchedule, sync_overlay_ui_scene_state_system);

        app.inner_mut()
            .world_mut()
            .resource_mut::<ShellRenderInput>()
            .overlay_ui
            .output(OutputId(3))
            .text(
                "label",
                OverlayUiLayer::Main,
                10,
                12,
                None,
                "猫land",
                14.0,
                RenderColor { r: 240, g: 241, b: 242, a: 255 },
                1.0,
                0,
            );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let scene = &app.inner().world().resource::<CompositorSceneState>().outputs[&OutputId(3)];
        let entry = scene
            .iter_ordered()
            .next()
            .map(|(_, entry)| entry)
            .expect("expected one text overlay entry");
        let nekoland_ecs::resources::CompositorSceneItem::Quad { content } = &entry.item else {
            panic!("expected quad scene item");
        };
        let QuadContent::RasterImage { image } = content else {
            panic!("expected raster image content");
        };
        assert_eq!(image.scale, 2);
        assert_eq!(entry.instance.rect.x, 10);
    }

    #[test]
    fn surface_primitives_sync_into_overlay_scene_entries() {
        let mut app = NekolandApp::new("overlay-ui-surface-sync-test");
        app.inner_mut().world_mut().spawn((
            nekoland_ecs::components::OutputId(4),
            OutputDevice { name: "Virtual-1".to_owned(), ..OutputDevice::default() },
            OutputProperties { scale: 1, ..OutputProperties::default() },
            OutputViewport::default(),
            OutputPlacement::default(),
            OutputWorkArea::default(),
        ));
        app.inner_mut()
            .init_resource::<ShellRenderInput>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<OverlayUiSceneSyncState>()
            .init_resource::<OverlayTextRasterizerState>()
            .add_systems(RenderSchedule, sync_overlay_ui_scene_state_system);

        app.inner_mut()
            .world_mut()
            .resource_mut::<ShellRenderInput>()
            .overlay_ui
            .output(OutputId(4))
            .surface(
                "thumbnail",
                OverlayUiLayer::Foreground,
                91,
                RenderRect { x: 30, y: 40, width: 120, height: 80 },
                None,
                0.9,
                3,
            );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let scene = &app.inner().world().resource::<CompositorSceneState>().outputs[&OutputId(4)];
        let entry = scene
            .iter_ordered()
            .next()
            .map(|(_, entry)| entry)
            .expect("expected one surface overlay entry");
        let nekoland_ecs::resources::CompositorSceneItem::Surface { surface_id } = &entry.item
        else {
            panic!("expected surface scene item");
        };
        assert_eq!(*surface_id, 91);
        assert_eq!(entry.instance.rect.height, 80);
    }

    #[test]
    fn rasterized_text_cache_is_fifo_bounded() {
        let mut state = OverlayTextRasterizerState::default();
        let image = QuadRasterImage { width: 1, height: 1, scale: 1, pixels_rgba: vec![255; 4] };

        for index in 0..=super::OVERLAY_TEXT_CACHE_LIMIT {
            state.insert_cached_image(
                super::OverlayTextCacheKey {
                    text: format!("text-{index}"),
                    font_size_bits: 14.0_f32.to_bits(),
                    scale: 1,
                    color: [255, 255, 255, 255],
                },
                image.clone(),
            );
        }

        assert_eq!(state.cache.len(), super::OVERLAY_TEXT_CACHE_LIMIT);
        assert_eq!(state.cache_order.len(), super::OVERLAY_TEXT_CACHE_LIMIT);
        assert!(!state.cache.contains_key(&super::OverlayTextCacheKey {
            text: "text-0".to_owned(),
            font_size_bits: 14.0_f32.to_bits(),
            scale: 1,
            color: [255, 255, 255, 255],
        }));
        assert!(state.cache.contains_key(&super::OverlayTextCacheKey {
            text: format!("text-{}", super::OVERLAY_TEXT_CACHE_LIMIT),
            font_size_bits: 14.0_f32.to_bits(),
            scale: 1,
            color: [255, 255, 255, 255],
        }));
    }
}
