use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    CompositorSceneEntry, CompositorSceneEntryId, CompositorSceneState, RenderItemInstance,
    RenderSceneRole, ShellRenderInput,
};

/// Render-local bookkeeping for overlay-owned compositor-scene entries from the previous frame.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct OutputOverlaySceneSyncState {
    pub output_entries: BTreeMap<OutputId, BTreeSet<CompositorSceneEntryId>>,
}

/// Synchronizes user-controlled output overlays into the formal compositor scene provider state.
pub fn sync_output_overlay_scene_state_system(
    shell_render_input: Res<'_, ShellRenderInput>,
    mut compositor_scene: ResMut<'_, CompositorSceneState>,
    mut sync_state: ResMut<'_, OutputOverlaySceneSyncState>,
) {
    let mut current_entries = BTreeMap::<OutputId, BTreeSet<CompositorSceneEntryId>>::new();
    let overlay_state = &shell_render_input.output_overlays;

    for (output_id, overlays) in &overlay_state.outputs {
        let output_scene = compositor_scene.outputs.entry(*output_id).or_default();
        let mut touched = false;
        for (_, overlay) in overlays.iter_sorted() {
            current_entries.entry(*output_id).or_default().insert(overlay.entry_id);
            output_scene.insert(
                overlay.entry_id,
                CompositorSceneEntry::solid_rect(
                    overlay.color,
                    RenderItemInstance {
                        rect: overlay.rect,
                        opacity: overlay.opacity,
                        clip_rect: overlay.clip_rect,
                        z_index: overlay.z_index,
                        scene_role: RenderSceneRole::Overlay,
                    },
                ),
            );
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

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        CompositorSceneState, OutputOverlayId, OutputOverlaySpec, RenderColor, RenderRect,
        ShellRenderInput,
    };

    use super::{OutputOverlaySceneSyncState, sync_output_overlay_scene_state_system};

    #[test]
    fn overlay_state_syncs_into_compositor_scene_and_preserves_entry_ids() {
        let mut app = NekolandApp::new("overlay-sync-test");
        app.inner_mut()
            .init_resource::<ShellRenderInput>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<OutputOverlaySceneSyncState>()
            .add_systems(RenderSchedule, sync_output_overlay_scene_state_system);

        let first_entry_id = {
            let mut shell_render_input =
                app.inner_mut().world_mut().resource_mut::<ShellRenderInput>();
            shell_render_input.output_overlays.upsert(
                OutputId(7),
                OutputOverlaySpec {
                    overlay_id: OutputOverlayId::from("debug"),
                    rect: RenderRect { x: 1, y: 2, width: 30, height: 40 },
                    clip_rect: None,
                    color: RenderColor { r: 10, g: 20, b: 30, a: 255 },
                    opacity: 0.5,
                    z_index: 9,
                },
            )
        };
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let scene = &app.inner().world().resource::<CompositorSceneState>().outputs[&OutputId(7)];
        assert_eq!(scene.ordered_items, vec![first_entry_id]);

        {
            let mut shell_render_input =
                app.inner_mut().world_mut().resource_mut::<ShellRenderInput>();
            shell_render_input.output_overlays.upsert(
                OutputId(7),
                OutputOverlaySpec {
                    overlay_id: OutputOverlayId::from("debug"),
                    rect: RenderRect { x: 5, y: 6, width: 70, height: 80 },
                    clip_rect: None,
                    color: RenderColor { r: 90, g: 91, b: 92, a: 255 },
                    opacity: 1.0,
                    z_index: 2,
                },
            );
        }
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let scene = &app.inner().world().resource::<CompositorSceneState>().outputs[&OutputId(7)];
        assert_eq!(scene.ordered_items, vec![first_entry_id]);
        assert_eq!(scene.items[&first_entry_id].instance.rect.x, 5);
    }

    #[test]
    fn removing_overlay_state_removes_compositor_scene_entry() {
        let mut app = NekolandApp::new("overlay-sync-remove-test");
        app.inner_mut()
            .init_resource::<ShellRenderInput>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<OutputOverlaySceneSyncState>()
            .add_systems(RenderSchedule, sync_output_overlay_scene_state_system);

        app.inner_mut().world_mut().resource_mut::<ShellRenderInput>().output_overlays.upsert(
            OutputId(3),
            OutputOverlaySpec {
                overlay_id: OutputOverlayId::from("debug"),
                rect: RenderRect { x: 1, y: 2, width: 30, height: 40 },
                clip_rect: None,
                color: RenderColor { r: 10, g: 20, b: 30, a: 255 },
                opacity: 0.5,
                z_index: 9,
            },
        );
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        app.inner_mut()
            .world_mut()
            .resource_mut::<ShellRenderInput>()
            .output_overlays
            .remove(OutputId(3), &OutputOverlayId::from("debug"));
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        assert!(
            app.inner()
                .world()
                .resource::<CompositorSceneState>()
                .outputs
                .get(&OutputId(3))
                .is_none()
        );
    }
}
