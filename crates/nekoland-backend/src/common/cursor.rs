use std::collections::HashSet;
use std::fs;

use nekoland_ecs::resources::CursorRenderState;
use nekoland_protocol::{ProtocolCursorImage, ProtocolCursorState};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::input::pointer::CursorIcon;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Transform;

#[derive(Debug, Clone, Copy)]
pub(crate) enum CursorRenderSource<'a> {
    Hidden,
    Named(CursorIcon),
    Surface { surface: &'a WlSurface, hotspot_x: i32, hotspot_y: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureCursorGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CursorCacheKey {
    theme: String,
    icon: CursorIcon,
    scale: u32,
}

#[derive(Debug, Clone)]
struct LoadedSoftwareCursor {
    buffer: MemoryRenderBuffer,
    hotspot_x: i32,
    hotspot_y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Default)]
pub(crate) struct SoftwareCursorCache {
    key: Option<CursorCacheKey>,
    cursor: Option<LoadedSoftwareCursor>,
}

impl SoftwareCursorCache {
    pub(crate) fn render_element<R>(
        &mut self,
        renderer: &mut R,
        theme: &str,
        icon: CursorIcon,
        scale: u32,
        x: f64,
        y: f64,
    ) -> Result<MemoryRenderBufferRenderElement<R>, R::Error>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        let cursor = self.ensure_loaded(theme, icon, scale);
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (
                f64::from(x.round() as i32 - cursor.hotspot_x),
                f64::from(y.round() as i32 - cursor.hotspot_y),
            ),
            &cursor.buffer,
            Some(1.0),
            None,
            None,
            Kind::Cursor,
        )
    }

    pub(crate) fn capture_geometry(
        &mut self,
        theme: &str,
        icon: CursorIcon,
        scale: u32,
        x: f64,
        y: f64,
    ) -> CaptureCursorGeometry {
        let cursor = self.ensure_loaded(theme, icon, scale);
        CaptureCursorGeometry {
            x: x.round() as i32 - cursor.hotspot_x,
            y: y.round() as i32 - cursor.hotspot_y,
            width: cursor.width,
            height: cursor.height,
        }
    }

    fn ensure_loaded(
        &mut self,
        theme: &str,
        icon: CursorIcon,
        scale: u32,
    ) -> &LoadedSoftwareCursor {
        let key = CursorCacheKey { theme: theme.to_owned(), icon, scale: scale.max(1) };
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key.clone());
            self.cursor = None;
        }
        self.cursor.get_or_insert_with(|| load_software_cursor(&key.theme, key.icon, key.scale))
    }
}

pub(crate) fn cursor_position_on_output(
    cursor_render: Option<&CursorRenderState>,
    output_name: &str,
) -> Option<(f64, f64)> {
    let cursor_render = cursor_render?;
    if !cursor_render.visible || cursor_render.output_name.as_deref() != Some(output_name) {
        return None;
    }
    Some((cursor_render.x, cursor_render.y))
}

pub(crate) fn cursor_render_source(
    cursor_state: Option<&ProtocolCursorState>,
) -> CursorRenderSource<'_> {
    match cursor_state.map(|state| &state.image) {
        Some(ProtocolCursorImage::Hidden) => CursorRenderSource::Hidden,
        Some(ProtocolCursorImage::Named(icon)) => CursorRenderSource::Named(*icon),
        Some(ProtocolCursorImage::Surface { surface, hotspot_x, hotspot_y }) => {
            CursorRenderSource::Surface { surface, hotspot_x: *hotspot_x, hotspot_y: *hotspot_y }
        }
        None => CursorRenderSource::Named(CursorIcon::Default),
    }
}

fn load_software_cursor(theme_name: &str, icon: CursorIcon, scale: u32) -> LoadedSoftwareCursor {
    load_theme_cursor(theme_name, icon, scale).unwrap_or_else(|| fallback_cursor(scale))
}

fn load_theme_cursor(
    theme_name: &str,
    icon: CursorIcon,
    scale: u32,
) -> Option<LoadedSoftwareCursor> {
    let nominal_size = 24_u32.saturating_mul(scale.max(1));
    for theme in theme_candidates(theme_name) {
        let theme = xcursor::CursorTheme::load(&theme);
        for cursor_name in cursor_name_candidates(icon) {
            let Some(path) = theme.load_icon(&cursor_name) else {
                continue;
            };
            let Ok(bytes) = fs::read(path) else {
                continue;
            };
            let Some(images) = xcursor::parser::parse_xcursor(&bytes) else {
                continue;
            };
            let Some(image) = images.into_iter().min_by_key(|image| {
                (
                    image.size.abs_diff(nominal_size),
                    image.width.abs_diff(nominal_size) + image.height.abs_diff(nominal_size),
                )
            }) else {
                continue;
            };
            return Some(LoadedSoftwareCursor {
                buffer: MemoryRenderBuffer::from_slice(
                    &image.pixels_rgba,
                    Fourcc::Abgr8888,
                    (image.width as i32, image.height as i32),
                    scale.max(1) as i32,
                    Transform::Normal,
                    None,
                ),
                hotspot_x: image.xhot as i32,
                hotspot_y: image.yhot as i32,
                width: image.width,
                height: image.height,
            });
        }
    }

    None
}

fn theme_candidates(theme_name: &str) -> Vec<String> {
    let mut themes = vec![theme_name.to_owned()];
    if theme_name != "default" {
        themes.push("default".to_owned());
    }
    themes
}

fn cursor_name_candidates(icon: CursorIcon) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();

    for name in std::iter::once(icon.name())
        .chain(icon.alt_names().iter().copied())
        .chain(std::iter::once(CursorIcon::Default.name()))
        .chain(CursorIcon::Default.alt_names().iter().copied())
    {
        if seen.insert(name) {
            names.push(name.to_owned());
        }
    }

    names
}

fn fallback_cursor(scale: u32) -> LoadedSoftwareCursor {
    const CURSOR_ART: &[&str] = &[
        "X...............",
        "XX..............",
        "XoX.............",
        "XooX............",
        "XoooX...........",
        "XooooX..........",
        "XoooooX.........",
        "XooooooX........",
        "XoooooooX.......",
        "XooooooooX......",
        "XoooooooooX.....",
        "XooooooooooX....",
        "XoooooooooooX...",
        "XooooooooXXXXX..",
        "XooooXXooX......",
        "XooXX.XooX......",
        "XXX...XooX......",
        "......XooX......",
        "......XooX......",
        "......XooX......",
        "......XooX......",
        "......XooX......",
        "......XXXX......",
        "................",
    ];

    let scale = scale.max(1) as usize;
    let width = CURSOR_ART[0].len() * scale;
    let height = CURSOR_ART.len() * scale;
    let mut pixels = vec![0_u8; width * height * 4];

    for (base_y, row) in CURSOR_ART.iter().enumerate() {
        for (base_x, pixel) in row.as_bytes().iter().enumerate() {
            let color = match pixel {
                b'X' => [0, 0, 0, 255],
                b'o' => [255, 255, 255, 255],
                _ => [0, 0, 0, 0],
            };
            for dy in 0..scale {
                for dx in 0..scale {
                    let x = base_x * scale + dx;
                    let y = base_y * scale + dy;
                    let offset = (y * width + x) * 4;
                    pixels[offset..offset + 4].copy_from_slice(&color);
                }
            }
        }
    }

    LoadedSoftwareCursor {
        buffer: MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Abgr8888,
            (width as i32, height as i32),
            scale as i32,
            Transform::Normal,
            None,
        ),
        hotspot_x: 0,
        hotspot_y: 0,
        width: width as u32,
        height: height as u32,
    }
}
