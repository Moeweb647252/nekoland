use std::collections::{BTreeMap, HashSet};
use std::fs;

use bevy_ecs::prelude::{Query, Res, ResMut, Resource};
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::resources::{
    CursorImageSnapshot, CursorSceneSnapshot, GlobalPointerPosition, RenderItemInstance,
    RenderRect, RenderSceneRole,
};
use nekoland_ecs::views::OutputRuntime;

use crate::scene_source::RenderSceneContribution;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CursorRenderer;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CursorThemeCacheKey {
    theme: String,
    icon_name: String,
    scale: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CursorImageMetadata {
    hotspot_x: i32,
    hotspot_y: i32,
    width: u32,
    height: u32,
}

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CursorThemeGeometryCache {
    metadata: BTreeMap<CursorThemeCacheKey, CursorImageMetadata>,
}

impl CursorThemeGeometryCache {
    fn geometry(&mut self, theme: &str, icon_name: &str, scale: u32) -> CursorImageMetadata {
        let key = CursorThemeCacheKey {
            theme: theme.to_owned(),
            icon_name: icon_name.to_owned(),
            scale: scale.max(1),
        };
        if let Some(metadata) = self.metadata.get(&key).copied() {
            return metadata;
        }

        let metadata = load_theme_cursor_metadata(&key.theme, &key.icon_name, key.scale)
            .unwrap_or_else(|| fallback_cursor_metadata(key.scale));
        self.metadata.insert(key, metadata);
        metadata
    }
}

/// Tracks the pointer against the current output layout and produces an output-local cursor scene
/// snapshot for the scene provider path.
pub fn cursor_scene_snapshot_system(
    pointer: Res<'_, GlobalPointerPosition>,
    outputs: Query<'_, '_, OutputRuntime>,
    mut cursor_scene: ResMut<'_, CursorSceneSnapshot>,
) {
    let next_output = outputs.iter().find(|output| {
        let left = f64::from(output.placement.x);
        let top = f64::from(output.placement.y);
        let right = left + f64::from(output.properties.width.max(1));
        let bottom = top + f64::from(output.properties.height.max(1));
        pointer.x >= left && pointer.x < right && pointer.y >= top && pointer.y < bottom
    });

    if let Some(output) = next_output {
        cursor_scene.visible = true;
        cursor_scene.output_id = Some(output.id());
        cursor_scene.x = pointer.x - f64::from(output.placement.x);
        cursor_scene.y = pointer.y - f64::from(output.placement.y);
    } else {
        cursor_scene.visible = false;
        cursor_scene.output_id = None;
        cursor_scene.x = 0.0;
        cursor_scene.y = 0.0;
    }

    tracing::trace!(
        visible = cursor_scene.visible,
        output_id = ?cursor_scene.output_id,
        x = cursor_scene.x,
        y = cursor_scene.y,
        "cursor scene snapshot tick"
    );
}

/// Emits one output-local cursor contribution so cursor rendering goes through the normal scene
/// assembly path.
pub fn emit_cursor_scene_contributions_system(
    config: Option<Res<'_, CompositorConfig>>,
    cursor_scene: Res<'_, CursorSceneSnapshot>,
    cursor_image: Res<'_, CursorImageSnapshot>,
    outputs: Query<'_, '_, OutputRuntime>,
    mut geometry_cache: ResMut<'_, CursorThemeGeometryCache>,
    mut contributions: ResMut<'_, crate::scene_source::RenderSceneContributionQueue>,
) {
    if !cursor_scene.visible {
        return;
    }
    let Some(output_id) = cursor_scene.output_id else {
        return;
    };
    let Some(output) = outputs.iter().find(|output| output.id() == output_id) else {
        return;
    };

    let contribution = match &*cursor_image {
        CursorImageSnapshot::Hidden => return,
        CursorImageSnapshot::Named { icon_name } => {
            let theme =
                config.as_deref().map(|config| config.cursor_theme.as_str()).unwrap_or("default");
            let metadata =
                geometry_cache.geometry(theme, icon_name, output.properties.scale.max(1));
            RenderSceneContribution::cursor(
                output_id,
                nekoland_ecs::resources::CursorRenderSource::Named { icon_name: icon_name.clone() },
                RenderItemInstance {
                    rect: RenderRect {
                        x: cursor_scene.x.round() as i32 - metadata.hotspot_x,
                        y: cursor_scene.y.round() as i32 - metadata.hotspot_y,
                        width: metadata.width,
                        height: metadata.height,
                    },
                    opacity: 1.0,
                    clip_rect: None,
                    z_index: i32::MAX,
                    scene_role: RenderSceneRole::Cursor,
                },
            )
        }
        CursorImageSnapshot::Surface { surface_id, hotspot_x, hotspot_y, width, height } => {
            RenderSceneContribution::cursor(
                output_id,
                nekoland_ecs::resources::CursorRenderSource::Surface { surface_id: *surface_id },
                RenderItemInstance {
                    rect: RenderRect {
                        x: cursor_scene.x.round() as i32 - *hotspot_x,
                        y: cursor_scene.y.round() as i32 - *hotspot_y,
                        width: (*width).max(1),
                        height: (*height).max(1),
                    },
                    opacity: 1.0,
                    clip_rect: None,
                    z_index: i32::MAX,
                    scene_role: RenderSceneRole::Cursor,
                },
            )
        }
    };

    contributions.outputs.entry(output_id).or_default().push(contribution);
}

fn load_theme_cursor_metadata(
    theme_name: &str,
    icon_name: &str,
    scale: u32,
) -> Option<CursorImageMetadata> {
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
            return Some(CursorImageMetadata {
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

fn fallback_cursor_metadata(scale: u32) -> CursorImageMetadata {
    let scale = scale.max(1);
    CursorImageMetadata {
        hotspot_x: 0,
        hotspot_y: 0,
        width: 16_u32.saturating_mul(scale),
        height: 24_u32.saturating_mul(scale),
    }
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::bundles::OutputBundle;
    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputKind, OutputPlacement, OutputProperties,
    };
    use nekoland_ecs::resources::{
        CursorImageSnapshot, CursorSceneSnapshot, GlobalPointerPosition,
    };

    use crate::scene_source::RenderSceneContributionQueue;

    use super::{
        CursorThemeGeometryCache, cursor_scene_snapshot_system,
        emit_cursor_scene_contributions_system,
    };

    #[test]
    fn cursor_scene_snapshot_tracks_output_local_position() {
        let mut app = NekolandApp::new("cursor-scene-snapshot-test");
        app.inner_mut()
            .init_resource::<GlobalPointerPosition>()
            .init_resource::<CursorSceneSnapshot>()
            .add_systems(RenderSchedule, cursor_scene_snapshot_system);

        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "DP-1".to_owned(),
                    kind: OutputKind::Nested,
                    make: "Nekoland".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1920,
                    height: 1080,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                placement: OutputPlacement { x: 640, y: 360 },
                ..Default::default()
            })
            .id();

        {
            let mut pointer = app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>();
            pointer.x = 700.0;
            pointer.y = 400.0;
        }

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let cursor = app.inner().world().resource::<CursorSceneSnapshot>();
        assert!(cursor.visible);
        assert_eq!(cursor.output_id, app.inner().world().get::<OutputId>(output).copied());
        assert_eq!(cursor.x, 60.0);
        assert_eq!(cursor.y, 40.0);
    }

    #[test]
    fn cursor_scene_snapshot_hides_cursor_outside_outputs() {
        let mut app = NekolandApp::new("cursor-scene-hidden-test");
        app.inner_mut()
            .init_resource::<GlobalPointerPosition>()
            .init_resource::<CursorSceneSnapshot>()
            .add_systems(RenderSchedule, cursor_scene_snapshot_system);

        {
            let mut pointer = app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>();
            pointer.x = 4000.0;
            pointer.y = 3000.0;
        }

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let cursor = app.inner().world().resource::<CursorSceneSnapshot>();
        assert!(!cursor.visible);
        assert_eq!(cursor.output_id, None);
    }

    #[test]
    fn emit_cursor_scene_contribution_builds_named_cursor_item() {
        let mut app = NekolandApp::new("cursor-scene-contribution-test");
        app.inner_mut()
            .init_resource::<CursorSceneSnapshot>()
            .init_resource::<CursorImageSnapshot>()
            .init_resource::<CursorThemeGeometryCache>()
            .init_resource::<RenderSceneContributionQueue>()
            .add_systems(RenderSchedule, emit_cursor_scene_contributions_system);
        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "Nekoland".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let output_id =
            *app.inner().world().get::<OutputId>(output).expect("output id should exist");
        *app.inner_mut().world_mut().resource_mut::<CursorSceneSnapshot>() =
            CursorSceneSnapshot { visible: true, output_id: Some(output_id), x: 30.0, y: 40.0 };
        *app.inner_mut().world_mut().resource_mut::<CursorImageSnapshot>() =
            CursorImageSnapshot::Named { icon_name: "default".to_owned() };

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let contributions = app.inner().world().resource::<RenderSceneContributionQueue>();
        let output_contributions = &contributions.outputs[&output_id];
        assert_eq!(output_contributions.len(), 1);
        match &output_contributions[0].payload {
            crate::scene_source::RenderSceneContributionPayload::Cursor { source } => {
                assert_eq!(
                    source,
                    &nekoland_ecs::resources::CursorRenderSource::Named {
                        icon_name: "default".to_owned(),
                    }
                );
            }
            payload => panic!("expected cursor payload, got {payload:?}"),
        }
    }
}
