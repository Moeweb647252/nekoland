//! Shared compositor-owned text shaping, glyph caching, and atlas packing.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use bevy_ecs::prelude::Resource;
use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent,
    SwashImage, Wrap,
    fontdb::{Query, Source, Stretch, Style, Weight},
};
use nekoland_ecs::resources::{
    PreparedTextAtlasPage, PreparedTextAtlasPageCacheKey, PreparedTextGlyph, RenderRect,
    RenderTextContent, TextAtlasContentKind, TextAtlasPageId, TextAtlasRect,
};

const LAYOUT_CACHE_LIMIT: usize = 256;
const TEXT_ATLAS_PAGE_WIDTH: u32 = 1024;
const TEXT_ATLAS_PAGE_HEIGHT: u32 = 1024;
const TEXT_ATLAS_PADDING: u32 = 1;

/// Default family used when no compositor config is available to provide an explicit overlay font.
pub const DEFAULT_OVERLAY_FONT_FAMILY: &str = "Noto Sans";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct TextLayoutCacheKey {
    text: String,
    font_family: String,
    font_size_bits: u32,
    raster_scale: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextAtlasGlyphKey {
    cache_key: CacheKey,
    content_kind: TextAtlasContentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CachedTextGlyph {
    cache_key: CacheKey,
    physical_left: i32,
    physical_top: i32,
    physical_right: i32,
    physical_bottom: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedTextLayout {
    logical_width: u32,
    logical_height: u32,
    glyphs: Vec<CachedTextGlyph>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextAtlasGlyphEntry {
    page_id: TextAtlasPageId,
    atlas_rect: TextAtlasRect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextAtlasPageState {
    id: TextAtlasPageId,
    content_kind: TextAtlasContentKind,
    width: u32,
    height: u32,
    pixels_rgba: Vec<u8>,
    next_x: u32,
    next_y: u32,
    row_height: u32,
    version: u64,
}

impl TextAtlasPageState {
    fn new(id: TextAtlasPageId, content_kind: TextAtlasContentKind) -> Self {
        let width = TEXT_ATLAS_PAGE_WIDTH;
        let height = TEXT_ATLAS_PAGE_HEIGHT;
        Self {
            id,
            content_kind,
            width,
            height,
            pixels_rgba: vec![0; width as usize * height as usize * 4],
            next_x: 0,
            next_y: 0,
            row_height: 0,
            version: 1,
        }
    }

    fn try_insert(&mut self, image: &SwashImage) -> Option<TextAtlasRect> {
        let required_width = image.placement.width.checked_add(TEXT_ATLAS_PADDING * 2)?;
        let required_height = image.placement.height.checked_add(TEXT_ATLAS_PADDING * 2)?;
        if required_width > self.width || required_height > self.height {
            return None;
        }

        if self.next_x.checked_add(required_width)? > self.width {
            self.next_x = 0;
            self.next_y = self.next_y.checked_add(self.row_height)?;
            self.row_height = 0;
        }
        if self.next_y.checked_add(required_height)? > self.height {
            return None;
        }

        let inner_x = self.next_x.checked_add(TEXT_ATLAS_PADDING)?;
        let inner_y = self.next_y.checked_add(TEXT_ATLAS_PADDING)?;
        let rgba = swash_image_rgba(image)?;
        copy_rgba_into_page(
            &mut self.pixels_rgba,
            self.width,
            inner_x,
            inner_y,
            image.placement.width,
            image.placement.height,
            &rgba,
        )?;

        self.next_x = self.next_x.checked_add(required_width)?;
        self.row_height = self.row_height.max(required_height);
        self.version = self.version.saturating_add(1);

        Some(TextAtlasRect {
            x: inner_x,
            y: inner_y,
            width: image.placement.width,
            height: image.placement.height,
        })
    }

    fn prepared_page(&self) -> PreparedTextAtlasPage {
        PreparedTextAtlasPage {
            id: self.id,
            width: self.width,
            height: self.height,
            content_kind: self.content_kind,
            pixels_rgba: self.pixels_rgba.clone(),
            cache_key: PreparedTextAtlasPageCacheKey {
                page_id: self.id,
                version: self.version,
                width: self.width,
                height: self.height,
                content_kind: self.content_kind,
            },
        }
    }
}

/// Render-local text shaping state shared by overlay scene sync and prepared-scene generation.
#[derive(Resource, Debug)]
pub struct TextRendererState {
    font_system: FontSystem,
    swash_cache: SwashCache,
    layout_cache: BTreeMap<TextLayoutCacheKey, CachedTextLayout>,
    layout_order: VecDeque<TextLayoutCacheKey>,
    glyph_atlas: HashMap<TextAtlasGlyphKey, TextAtlasGlyphEntry>,
    atlas_pages: BTreeMap<TextAtlasPageId, TextAtlasPageState>,
    logged_requested_families: BTreeSet<String>,
    logged_face_ids: BTreeSet<cosmic_text::fontdb::ID>,
    next_page_id: u64,
}

impl Default for TextRendererState {
    fn default() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            layout_cache: BTreeMap::new(),
            layout_order: VecDeque::new(),
            glyph_atlas: HashMap::new(),
            atlas_pages: BTreeMap::new(),
            logged_requested_families: BTreeSet::new(),
            logged_face_ids: BTreeSet::new(),
            next_page_id: 1,
        }
    }
}

impl TextRendererState {
    /// Returns the logical bounding size of one text payload at the requested raster scale.
    pub fn logical_size(
        &mut self,
        content: &RenderTextContent,
        raster_scale: u32,
    ) -> Option<(u32, u32)> {
        let layout = self.layout(content, raster_scale)?;
        Some((layout.logical_width, layout.logical_height))
    }

    /// Shapes one text payload and resolves every glyph into atlas-backed prepared glyph records.
    pub fn prepare_glyphs(
        &mut self,
        content: &RenderTextContent,
        raster_scale: u32,
        origin_x: i32,
        origin_y: i32,
    ) -> Option<(Vec<PreparedTextGlyph>, BTreeSet<TextAtlasPageId>)> {
        let layout = self.layout(content, raster_scale)?.clone();
        let mut glyphs = Vec::with_capacity(layout.glyphs.len());
        let mut used_pages = BTreeSet::new();
        let scale_i32 = i32::try_from(raster_scale.max(1)).ok()?;

        for glyph in layout.glyphs {
            let image =
                self.swash_cache.get_image(&mut self.font_system, glyph.cache_key).clone()?;
            let entry = self.atlas_entry(glyph.cache_key, &image)?;
            used_pages.insert(entry.page_id);

            let left = origin_x.checked_add(div_floor_i32(glyph.physical_left, scale_i32))?;
            let top = origin_y.checked_add(div_floor_i32(glyph.physical_top, scale_i32))?;
            let right = origin_x.checked_add(div_ceil_i32(glyph.physical_right, scale_i32))?;
            let bottom = origin_y.checked_add(div_ceil_i32(glyph.physical_bottom, scale_i32))?;
            if right <= left || bottom <= top {
                continue;
            }

            glyphs.push(PreparedTextGlyph {
                atlas_page_id: entry.page_id,
                atlas_rect: entry.atlas_rect,
                target_rect: RenderRect {
                    x: left,
                    y: top,
                    width: (right - left) as u32,
                    height: (bottom - top) as u32,
                },
            });
        }

        Some((glyphs, used_pages))
    }

    /// Returns one prepared atlas page snapshot when the page is currently populated.
    pub fn prepared_page(&self, page_id: TextAtlasPageId) -> Option<PreparedTextAtlasPage> {
        self.atlas_pages.get(&page_id).map(TextAtlasPageState::prepared_page)
    }

    fn layout(
        &mut self,
        content: &RenderTextContent,
        raster_scale: u32,
    ) -> Option<&CachedTextLayout> {
        let key = TextLayoutCacheKey {
            text: content.text.clone(),
            font_family: content.font_family.clone(),
            font_size_bits: content.font_size_bits,
            raster_scale: raster_scale.max(1),
        };
        if !self.layout_cache.contains_key(&key) {
            let layout = self.compute_layout(content, raster_scale.max(1))?;
            self.layout_cache.insert(key.clone(), layout);
            self.layout_order.push_back(key.clone());
            while self.layout_order.len() > LAYOUT_CACHE_LIMIT {
                let Some(oldest) = self.layout_order.pop_front() else {
                    break;
                };
                self.layout_cache.remove(&oldest);
            }
        }
        self.layout_cache.get(&key)
    }

    fn compute_layout(
        &mut self,
        content: &RenderTextContent,
        raster_scale: u32,
    ) -> Option<CachedTextLayout> {
        if content.text.is_empty() {
            return None;
        }

        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(content.font_size(), content.font_size()),
        );
        buffer.set_wrap(&mut self.font_system, Wrap::None);
        buffer.set_size(&mut self.font_system, None, None);
        let attrs = Attrs::new().family(Family::Name(content.font_family.as_str()));
        buffer.set_text(
            &mut self.font_system,
            content.text.as_str(),
            &attrs,
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut raw_glyphs = Vec::new();
        let mut min_left = i32::MAX;
        let mut min_top = i32::MAX;
        let mut max_right = i32::MIN;
        let mut max_bottom = i32::MIN;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((0.0, run.line_y), raster_scale as f32);
                let Some(image) =
                    self.swash_cache.get_image(&mut self.font_system, physical.cache_key).as_ref()
                else {
                    continue;
                };
                if image.placement.width == 0 || image.placement.height == 0 {
                    continue;
                }

                let left = physical.x.checked_add(image.placement.left)?;
                let top = physical.y.checked_sub(image.placement.top)?;
                let right = left.checked_add(i32::try_from(image.placement.width).ok()?)?;
                let bottom = top.checked_add(i32::try_from(image.placement.height).ok()?)?;
                min_left = min_left.min(left);
                min_top = min_top.min(top);
                max_right = max_right.max(right);
                max_bottom = max_bottom.max(bottom);
                raw_glyphs.push(CachedTextGlyph {
                    cache_key: physical.cache_key,
                    physical_left: left,
                    physical_top: top,
                    physical_right: right,
                    physical_bottom: bottom,
                });
            }
        }

        if raw_glyphs.is_empty() || min_left == i32::MAX || min_top == i32::MAX {
            return None;
        }

        self.log_requested_family_match(content.font_family.as_str());
        self.log_used_faces(
            content.font_family.as_str(),
            raw_glyphs.iter().map(|glyph| glyph.cache_key.font_id),
        );

        let scale_i32 = i32::try_from(raster_scale).ok()?;
        let logical_width = div_ceil_i32(max_right.checked_sub(min_left)?, scale_i32);
        let logical_height = div_ceil_i32(max_bottom.checked_sub(min_top)?, scale_i32);
        if logical_width <= 0 || logical_height <= 0 {
            return None;
        }

        for glyph in &mut raw_glyphs {
            glyph.physical_left -= min_left;
            glyph.physical_top -= min_top;
            glyph.physical_right -= min_left;
            glyph.physical_bottom -= min_top;
        }

        Some(CachedTextLayout {
            logical_width: logical_width as u32,
            logical_height: logical_height as u32,
            glyphs: raw_glyphs,
        })
    }

    fn family_match_face_id(&self, requested_family: &str) -> Option<cosmic_text::fontdb::ID> {
        let families = [Family::Name(requested_family)];
        let query = Query {
            families: &families,
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        self.font_system.db().query(&query)
    }

    fn log_requested_family_match(&mut self, requested_family: &str) {
        if !self.logged_requested_families.insert(requested_family.to_owned()) {
            return;
        }

        let Some(face_id) = self.family_match_face_id(requested_family) else {
            tracing::warn!(
                requested_family,
                "overlay text font family did not match any fontdb face"
            );
            return;
        };
        let Some(face) = self.font_system.db().face(face_id) else {
            tracing::warn!(
                requested_family,
                face_id = %face_id,
                "overlay text fontdb query returned missing face"
            );
            return;
        };

        tracing::info!(
            requested_family,
            face_id = %face_id,
            family = primary_family_name(face),
            post_script = %face.post_script_name,
            source = %face_source_label(face),
            "overlay text primary font match"
        );
    }

    fn log_used_faces(
        &mut self,
        requested_family: &str,
        font_ids: impl IntoIterator<Item = cosmic_text::fontdb::ID>,
    ) {
        let unique_ids = font_ids.into_iter().collect::<BTreeSet<_>>();
        for font_id in unique_ids {
            if !self.logged_face_ids.insert(font_id) {
                continue;
            }
            let Some(face) = self.font_system.db().face(font_id) else {
                tracing::warn!(
                    requested_family,
                    face_id = %font_id,
                    "overlay text used missing fontdb face"
                );
                continue;
            };
            let fallback =
                !face.families.iter().any(|(family_name, _)| family_name == requested_family);
            tracing::info!(
                requested_family,
                face_id = %font_id,
                family = primary_family_name(face),
                post_script = %face.post_script_name,
                source = %face_source_label(face),
                fallback,
                "overlay text shaping used font face"
            );
        }
    }

    fn atlas_entry(
        &mut self,
        cache_key: CacheKey,
        image: &SwashImage,
    ) -> Option<TextAtlasGlyphEntry> {
        let content_kind = atlas_content_kind(image.content);
        let key = TextAtlasGlyphKey { cache_key, content_kind };
        if let Some(entry) = self.glyph_atlas.get(&key).copied() {
            return Some(entry);
        }

        for page in self.atlas_pages.values_mut() {
            if page.content_kind != content_kind {
                continue;
            }
            if let Some(atlas_rect) = page.try_insert(image) {
                let entry = TextAtlasGlyphEntry { page_id: page.id, atlas_rect };
                self.glyph_atlas.insert(key, entry);
                return Some(entry);
            }
        }

        let page_id = TextAtlasPageId(self.next_page_id.max(1));
        self.next_page_id = page_id.0.saturating_add(1);
        let mut page = TextAtlasPageState::new(page_id, content_kind);
        let atlas_rect = page.try_insert(image)?;
        let entry = TextAtlasGlyphEntry { page_id, atlas_rect };
        self.glyph_atlas.insert(key, entry);
        self.atlas_pages.insert(page_id, page);
        Some(entry)
    }
}

fn atlas_content_kind(content: SwashContent) -> TextAtlasContentKind {
    match content {
        SwashContent::Mask | SwashContent::SubpixelMask => TextAtlasContentKind::Mask,
        SwashContent::Color => TextAtlasContentKind::Color,
    }
}

fn swash_image_rgba(image: &SwashImage) -> Option<Vec<u8>> {
    let width = usize::try_from(image.placement.width).ok()?;
    let height = usize::try_from(image.placement.height).ok()?;
    let len = width.checked_mul(height)?.checked_mul(4)?;
    let mut rgba = vec![0_u8; len];

    match image.content {
        SwashContent::Mask => {
            if image.data.len() != width.checked_mul(height)? {
                return None;
            }
            for (index, alpha) in image.data.iter().copied().enumerate() {
                let offset = index.checked_mul(4)?;
                rgba[offset] = alpha;
                rgba[offset + 1] = alpha;
                rgba[offset + 2] = alpha;
                rgba[offset + 3] = alpha;
            }
        }
        SwashContent::Color => {
            if image.data.len() != len {
                return None;
            }
            rgba.copy_from_slice(&image.data);
        }
        SwashContent::SubpixelMask => {
            if image.data.len() != len {
                return None;
            }
            for index in 0..width.checked_mul(height)? {
                let offset = index.checked_mul(4)?;
                let alpha =
                    image.data[offset].max(image.data[offset + 1]).max(image.data[offset + 2]);
                rgba[offset] = alpha;
                rgba[offset + 1] = alpha;
                rgba[offset + 2] = alpha;
                rgba[offset + 3] = alpha;
            }
        }
    }

    Some(rgba)
}

fn copy_rgba_into_page(
    pixels: &mut [u8],
    page_width: u32,
    dst_x: u32,
    dst_y: u32,
    width: u32,
    height: u32,
    src: &[u8],
) -> Option<()> {
    let page_width = usize::try_from(page_width).ok()?;
    let dst_x = usize::try_from(dst_x).ok()?;
    let dst_y = usize::try_from(dst_y).ok()?;
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    let row_len = width.checked_mul(4)?;
    if src.len() != height.checked_mul(row_len)? {
        return None;
    }

    for row in 0..height {
        let src_start = row.checked_mul(row_len)?;
        let src_end = src_start.checked_add(row_len)?;
        let dst_start = ((dst_y + row) * page_width + dst_x).checked_mul(4)?;
        let dst_end = dst_start.checked_add(row_len)?;
        pixels.get_mut(dst_start..dst_end)?.copy_from_slice(src.get(src_start..src_end)?);
    }

    Some(())
}

fn div_floor_i32(value: i32, divisor: i32) -> i32 {
    value.div_euclid(divisor.max(1))
}

fn div_ceil_i32(value: i32, divisor: i32) -> i32 {
    let divisor = divisor.max(1);
    if value >= 0 { (value + divisor - 1) / divisor } else { value / divisor }
}

fn primary_family_name(face: &cosmic_text::fontdb::FaceInfo) -> &str {
    face.families
        .first()
        .map(|(family_name, _)| family_name.as_str())
        .unwrap_or(face.post_script_name.as_str())
}

fn face_source_label(face: &cosmic_text::fontdb::FaceInfo) -> String {
    match &face.source {
        Source::Binary(_) => "<binary>".to_owned(),
        Source::File(path) | Source::SharedFile(path, _) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use cosmic_text::{Placement, SwashContent, SwashImage};
    use nekoland_ecs::resources::RenderColor;

    use super::{RenderTextContent, TextRendererState, swash_image_rgba};

    #[test]
    fn logical_size_is_positive_for_overlay_text() {
        let mut state = TextRendererState::default();
        let content = RenderTextContent::new(
            "猫land",
            "Noto Sans",
            RenderColor { r: 255, g: 255, b: 255, a: 255 },
            14.0,
        );

        let size = state.logical_size(&content, 4).expect("text should shape");
        assert!(size.0 > 0);
        assert!(size.1 > 0);
    }

    #[test]
    fn preparing_glyphs_populates_atlas_pages() {
        let mut state = TextRendererState::default();
        let content = RenderTextContent::new(
            "Switch Windows",
            "Noto Sans",
            RenderColor { r: 244, g: 245, b: 246, a: 255 },
            18.0,
        );

        let (glyphs, pages) =
            state.prepare_glyphs(&content, 4, 12, 18).expect("text glyphs should prepare");
        assert!(!glyphs.is_empty());
        assert!(!pages.is_empty());
        for page_id in pages {
            let page = state.prepared_page(page_id).expect("atlas page should exist");
            assert!(page.width > 0);
            assert!(page.height > 0);
            assert!(!page.pixels_rgba.is_empty());
        }
    }

    #[test]
    fn mask_images_expand_coverage_into_every_channel() {
        let image = SwashImage {
            content: SwashContent::Mask,
            placement: Placement { left: 0, top: 0, width: 2, height: 1 },
            data: vec![0, 127],
            ..SwashImage::new()
        };

        let rgba = swash_image_rgba(&image).expect("mask image should convert");
        assert_eq!(rgba, vec![0, 0, 0, 0, 127, 127, 127, 127]);
    }
}
