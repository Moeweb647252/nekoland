use std::collections::HashSet;
use std::fs;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::utils::Transform;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CursorCacheKey {
    theme: String,
    icon_name: String,
    scale: u32,
}

#[derive(Debug, Clone)]
struct LoadedSoftwareCursor {
    buffer: MemoryRenderBuffer,
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
        icon_name: &str,
        scale: u32,
        x: i32,
        y: i32,
    ) -> Result<MemoryRenderBufferRenderElement<R>, R::Error>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        let cursor = self.ensure_loaded(theme, icon_name, scale);
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (f64::from(x), f64::from(y)),
            &cursor.buffer,
            Some(1.0),
            None,
            None,
            Kind::Cursor,
        )
    }

    fn ensure_loaded(&mut self, theme: &str, icon_name: &str, scale: u32) -> &LoadedSoftwareCursor {
        let key = CursorCacheKey {
            theme: theme.to_owned(),
            icon_name: icon_name.to_owned(),
            scale: scale.max(1),
        };
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key.clone());
            self.cursor = None;
        }
        self.cursor
            .get_or_insert_with(|| load_software_cursor(&key.theme, &key.icon_name, key.scale))
    }
}

fn load_software_cursor(theme_name: &str, icon_name: &str, scale: u32) -> LoadedSoftwareCursor {
    load_theme_cursor(theme_name, icon_name, scale).unwrap_or_else(|| fallback_cursor(scale))
}

fn load_theme_cursor(
    theme_name: &str,
    icon_name: &str,
    scale: u32,
) -> Option<LoadedSoftwareCursor> {
    let nominal_size = 24_u32.saturating_mul(scale.max(1));
    for theme in theme_candidates(theme_name) {
        let theme = xcursor::CursorTheme::load(&theme);
        for cursor_name in cursor_name_candidates(icon_name) {
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

fn cursor_name_candidates(icon_name: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();

    for name in [icon_name, "default"] {
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
    }
}
