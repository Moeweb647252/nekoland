use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{QuadContent, RenderColor, RenderItemInstance, RenderSceneRole};

/// Stable identity for one compositor-owned scene entry.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct CompositorSceneEntryId(pub u64);

/// One compositor-owned item emitted into the frame scene.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CompositorSceneItem {
    Quad { content: QuadContent },
    Backdrop,
}

/// One stable compositor-owned scene entry for a specific output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CompositorSceneEntry {
    pub item: CompositorSceneItem,
    pub instance: RenderItemInstance,
}

impl CompositorSceneEntry {
    pub fn quad(content: QuadContent, instance: RenderItemInstance) -> Self {
        Self { item: CompositorSceneItem::Quad { content }, instance }
    }

    pub fn solid_color(color: RenderColor, instance: RenderItemInstance) -> Self {
        Self::quad(QuadContent::SolidColor { color }, instance)
    }

    pub fn backdrop(instance: RenderItemInstance) -> Self {
        Self { item: CompositorSceneItem::Backdrop, instance }
    }
}

/// Stable ordered compositor-owned scene entries for one output.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputCompositorScene {
    pub items: BTreeMap<CompositorSceneEntryId, CompositorSceneEntry>,
    pub ordered_items: Vec<CompositorSceneEntryId>,
}

impl OutputCompositorScene {
    pub fn from_entries(
        entries: impl IntoIterator<Item = (CompositorSceneEntryId, CompositorSceneEntry)>,
    ) -> Self {
        let mut scene = Self::default();
        for (entry_id, entry) in entries {
            scene.insert(entry_id, entry);
        }
        scene.sort_by_z_index();
        scene
    }

    pub fn insert(&mut self, entry_id: CompositorSceneEntryId, entry: CompositorSceneEntry) {
        debug_assert!(
            matches!(
                entry.instance.scene_role,
                RenderSceneRole::Compositor | RenderSceneRole::Overlay
            ),
            "compositor scene entries may only use compositor or overlay roles"
        );

        let is_new = self.items.insert(entry_id, entry).is_none();
        if is_new {
            self.ordered_items.push(entry_id);
        }
    }

    pub fn remove(&mut self, entry_id: CompositorSceneEntryId) -> Option<CompositorSceneEntry> {
        self.ordered_items.retain(|ordered| *ordered != entry_id);
        self.items.remove(&entry_id)
    }

    pub fn sort_by_z_index(&mut self) {
        self.ordered_items.sort_by_key(|entry_id| {
            self.items.get(entry_id).map(|entry| entry.instance.z_index).unwrap_or(i32::MAX)
        });
    }

    pub fn entry(&self, entry_id: CompositorSceneEntryId) -> Option<&CompositorSceneEntry> {
        self.items.get(&entry_id)
    }

    pub fn iter_ordered(
        &self,
    ) -> impl Iterator<Item = (CompositorSceneEntryId, &CompositorSceneEntry)> {
        self.ordered_items
            .iter()
            .filter_map(|entry_id| self.items.get(entry_id).map(|entry| (*entry_id, entry)))
    }
}

/// Compositor-owned per-output scene truth that feeds the render-side provider path.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CompositorSceneState {
    pub outputs: BTreeMap<OutputId, OutputCompositorScene>,
}

#[cfg(test)]
mod tests {
    use crate::components::OutputId;
    use crate::resources::{QuadContent, RenderColor, RenderItemInstance, RenderRect, RenderSceneRole};

    use super::{
        CompositorSceneEntry, CompositorSceneEntryId, CompositorSceneItem, CompositorSceneState,
        OutputCompositorScene,
    };

    fn overlay_instance(z_index: i32) -> RenderItemInstance {
        RenderItemInstance {
            rect: RenderRect { x: 0, y: 0, width: 10, height: 10 },
            opacity: 1.0,
            clip_rect: None,
            z_index,
            scene_role: RenderSceneRole::Overlay,
        }
    }

    #[test]
    fn output_scene_sorts_by_z_index_but_preserves_entry_ids() {
        let scene = OutputCompositorScene::from_entries([
            (
                CompositorSceneEntryId(2),
                CompositorSceneEntry::quad(
                    QuadContent::SolidColor { color: RenderColor { r: 1, g: 2, b: 3, a: 4 } },
                    overlay_instance(5),
                ),
            ),
            (CompositorSceneEntryId(1), CompositorSceneEntry::backdrop(overlay_instance(1))),
        ]);

        assert_eq!(scene.ordered_items, vec![CompositorSceneEntryId(1), CompositorSceneEntryId(2)]);
        assert!(matches!(
            scene.entry(CompositorSceneEntryId(1)).map(|entry| &entry.item),
            Some(CompositorSceneItem::Backdrop)
        ));
    }

    #[test]
    fn output_scene_remove_updates_ordering() {
        let mut scene = OutputCompositorScene::from_entries([(
            CompositorSceneEntryId(7),
            CompositorSceneEntry::backdrop(overlay_instance(0)),
        )]);

        let removed = scene.remove(CompositorSceneEntryId(7));
        assert!(removed.is_some());
        assert!(scene.ordered_items.is_empty());
        assert!(scene.items.is_empty());
    }

    #[test]
    fn compositor_scene_state_defaults_to_empty_outputs() {
        let state = CompositorSceneState::default();
        assert!(state.outputs.is_empty());
        assert_eq!(state.outputs.get(&OutputId(1)), None);
    }
}
