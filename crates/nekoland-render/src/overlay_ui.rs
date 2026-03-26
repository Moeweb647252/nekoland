//! Overlay-UI scene synchronization for compositor-owned panels, surfaces, and text.
//!
//! Text stays semantic through scene sync. Bounding boxes are resolved here so ordering, clipping,
//! and damage tracking can keep using stable output-local rectangles before the backend-specific
//! text preparation stage runs.
#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use bevy_ecs::prelude::{Query, Res, ResMut, Resource};
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::components::{OutputId, OutputProperties};
use nekoland_ecs::resources::{
    CompositorSceneEntry, CompositorSceneEntryId, CompositorSceneState, OverlayUiPrimitive,
    OverlayUiPrimitiveId, QuadContent, RenderItemInstance, RenderSceneRole, RenderTextContent,
    ShellRenderInput,
};

use crate::text::{DEFAULT_OVERLAY_FONT_FAMILY, TextRendererState};

const MIN_OVERLAY_TEXT_RASTER_SCALE: u32 = 4;

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct OverlayUiSceneSyncState {
    pub output_entries: BTreeMap<OutputId, BTreeSet<CompositorSceneEntryId>>,
}

/// Synchronizes shell-owned overlay UI primitives into compositor-scene entries.
pub fn sync_overlay_ui_scene_state_system(
    config: Option<Res<'_, CompositorConfig>>,
    shell_render_input: Res<'_, ShellRenderInput>,
    outputs: Query<'_, '_, (&'static OutputId, &'static OutputProperties)>,
    mut compositor_scene: ResMut<'_, CompositorSceneState>,
    mut sync_state: ResMut<'_, OverlayUiSceneSyncState>,
    mut text_renderer: ResMut<'_, TextRendererState>,
) {
    let mut current_entries = BTreeMap::<OutputId, BTreeSet<CompositorSceneEntryId>>::new();
    let overlay_ui = &shell_render_input.overlay_ui;
    let output_scales = outputs
        .iter()
        .map(|(output_id, properties)| (*output_id, properties.scale.max(1)))
        .collect::<BTreeMap<_, _>>();
    let overlay_font_family = config
        .as_deref()
        .map(|config| config.overlay_font_family.as_str())
        .unwrap_or(DEFAULT_OVERLAY_FONT_FAMILY);

    for (output_id, output_frame) in &overlay_ui.outputs {
        let output_scene = compositor_scene.outputs.entry(*output_id).or_default();
        let output_scale = output_scales.get(output_id).copied().unwrap_or(1);
        let mut primitives = output_frame.primitives.iter().collect::<Vec<_>>();
        primitives.sort_by_key(|primitive| {
            (primitive.layer(), primitive.z_index(), primitive.id().clone())
        });

        let mut touched = false;
        for primitive in primitives {
            let Some((entry_id, entry)) = overlay_ui_scene_entry(
                *output_id,
                primitive,
                output_scale,
                overlay_font_family,
                &mut text_renderer,
            ) else {
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
    overlay_font_family: &str,
    text_renderer: &mut TextRendererState,
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
            let content = RenderTextContent::new(
                text.text.clone(),
                overlay_font_family,
                text.color,
                text.font_size,
            );
            let raster_scale = output_scale.max(MIN_OVERLAY_TEXT_RASTER_SCALE);
            let (width, height) = text_renderer.logical_size(&content, raster_scale)?;
            let entry_id = overlay_ui_entry_id(output_id, &text.id);
            let entry = CompositorSceneEntry::text(
                content,
                RenderItemInstance {
                    rect: nekoland_ecs::resources::RenderRect {
                        x: text.x,
                        y: text.y,
                        width,
                        height,
                    },
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

#[cfg(test)]
mod tests {
    use nekoland_config::resources::CompositorConfig;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputPlacement, OutputProperties, OutputViewport, OutputWorkArea,
    };
    use nekoland_ecs::resources::{
        CompositorSceneState, OverlayUiLayer, RenderColor, RenderRect, ShellRenderInput,
    };

    use crate::text::TextRendererState;

    use super::{OverlayUiSceneSyncState, sync_overlay_ui_scene_state_system};

    #[test]
    fn panel_primitives_sync_into_overlay_scene_entries() {
        let mut app = NekolandApp::new("overlay-ui-panel-sync-test");
        app.inner_mut().world_mut().spawn((
            nekoland_ecs::components::OutputId(7),
            OutputDevice { name: "Virtual-1".to_owned(), ..OutputDevice::default() },
            OutputProperties { scale: 1, ..OutputProperties::default() },
            OutputViewport::default(),
            nekoland_ecs::components::OutputPlacement::default(),
            OutputWorkArea::default(),
        ));
        app.inner_mut()
            .init_resource::<ShellRenderInput>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<OverlayUiSceneSyncState>()
            .init_resource::<TextRendererState>()
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
            nekoland_ecs::resources::QuadContent::SolidColor {
                color: RenderColor { r: 1, g: 2, b: 3, a: 255 },
            }
        );
    }

    #[test]
    fn text_primitives_sync_into_overlay_text_entries() {
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
            .insert_resource(CompositorConfig {
                overlay_font_family: "Noto Sans".to_owned(),
                ..CompositorConfig::default()
            })
            .init_resource::<ShellRenderInput>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<OverlayUiSceneSyncState>()
            .init_resource::<TextRendererState>()
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
        let nekoland_ecs::resources::CompositorSceneItem::Text { content } = &entry.item else {
            panic!("expected text scene item");
        };
        assert_eq!(content.text, "猫land");
        assert_eq!(content.font_family, "Noto Sans");
        assert_eq!(entry.instance.rect.x, 10);
        assert!(entry.instance.rect.width > 0);
        assert!(entry.instance.rect.height > 0);
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
            .init_resource::<TextRendererState>()
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
}
