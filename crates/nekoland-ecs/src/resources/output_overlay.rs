use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{CompositorSceneEntryId, QuadContent, RenderColor, RenderRect};

/// Stable user-facing identifier for one output overlay entry.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct OutputOverlayId(pub String);

impl OutputOverlayId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for OutputOverlayId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for OutputOverlayId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// User-facing overlay description staged or stored for one output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutputOverlaySpec {
    pub overlay_id: OutputOverlayId,
    pub rect: RenderRect,
    pub clip_rect: Option<RenderRect>,
    pub content: QuadContent,
    pub opacity: f32,
    pub z_index: i32,
}

impl OutputOverlaySpec {
    pub fn solid_color(
        overlay_id: impl Into<OutputOverlayId>,
        rect: RenderRect,
        clip_rect: Option<RenderRect>,
        color: RenderColor,
        opacity: f32,
        z_index: i32,
    ) -> Self {
        Self {
            overlay_id: overlay_id.into(),
            rect,
            clip_rect,
            content: QuadContent::SolidColor { color },
            opacity,
            z_index,
        }
    }

    pub fn normalized(mut self) -> Self {
        self.opacity = self.opacity.clamp(0.0, 1.0);
        self
    }
}

/// One user-facing overlay update before or after output selector resolution.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum OutputOverlayUpdate {
    Set(OutputOverlaySpec),
    Remove(OutputOverlayId),
}

impl OutputOverlayUpdate {
    pub fn overlay_id(&self) -> &OutputOverlayId {
        match self {
            Self::Set(spec) => &spec.overlay_id,
            Self::Remove(overlay_id) => overlay_id,
        }
    }
}

/// Output-id-resolved overlay control updates ready for runtime application.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PendingOutputOverlayControl {
    pub output_id: OutputId,
    pub clear_overlays: bool,
    pub overlay_updates: Vec<OutputOverlayUpdate>,
}

/// Mutable façade over one output-local pending overlay control.
pub struct OutputOverlayControlHandle<'a> {
    control: &'a mut PendingOutputOverlayControl,
}

/// Low-level bridge from high-level output control into output-local overlay runtime state.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PendingOutputOverlayControls {
    controls: BTreeMap<OutputId, PendingOutputOverlayControl>,
}

impl PendingOutputOverlayControls {
    pub fn output(&mut self, output_id: OutputId) -> OutputOverlayControlHandle<'_> {
        let control = self.controls.entry(output_id).or_insert_with(|| {
            PendingOutputOverlayControl { output_id, ..PendingOutputOverlayControl::default() }
        });

        OutputOverlayControlHandle { control }
    }

    pub fn take(&mut self) -> Vec<PendingOutputOverlayControl> {
        std::mem::take(&mut self.controls).into_values().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}

impl OutputOverlayControlHandle<'_> {
    pub fn set_overlay(&mut self, spec: OutputOverlaySpec) -> &mut Self {
        let overlay_id = spec.overlay_id.clone();
        self.control.overlay_updates.retain(|update| update.overlay_id() != &overlay_id);
        self.control.overlay_updates.push(OutputOverlayUpdate::Set(spec.normalized()));
        self
    }

    pub fn remove_overlay(&mut self, overlay_id: impl Into<OutputOverlayId>) -> &mut Self {
        let overlay_id = overlay_id.into();
        self.control.overlay_updates.retain(|update| update.overlay_id() != &overlay_id);
        self.control.overlay_updates.push(OutputOverlayUpdate::Remove(overlay_id));
        self
    }

    pub fn clear_overlays(&mut self) -> &mut Self {
        self.control.clear_overlays = true;
        self.control.overlay_updates.clear();
        self
    }
}

/// One persistent runtime overlay entry for one output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutputOverlayEntry {
    pub entry_id: CompositorSceneEntryId,
    pub rect: RenderRect,
    pub clip_rect: Option<RenderRect>,
    pub content: QuadContent,
    pub opacity: f32,
    pub z_index: i32,
}

/// Output-local runtime overlay collection keyed by stable user-facing ids.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputOverlayCollection {
    pub overlays: BTreeMap<OutputOverlayId, OutputOverlayEntry>,
}

impl OutputOverlayCollection {
    pub fn iter_sorted(&self) -> impl Iterator<Item = (&OutputOverlayId, &OutputOverlayEntry)> {
        let mut ordered = self.overlays.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|(overlay_id, entry)| (entry.z_index, (*overlay_id).clone()));
        ordered.into_iter()
    }
}

/// Persistent output-local compositor overlay state owned by the runtime.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputOverlayState {
    next_entry_id: u64,
    pub outputs: BTreeMap<OutputId, OutputOverlayCollection>,
}

impl OutputOverlayState {
    pub fn upsert(
        &mut self,
        output_id: OutputId,
        spec: OutputOverlaySpec,
    ) -> CompositorSceneEntryId {
        let spec = spec.normalized();
        let output = self.outputs.entry(output_id).or_default();

        if let Some(existing) = output.overlays.get_mut(&spec.overlay_id) {
            existing.rect = spec.rect;
            existing.clip_rect = spec.clip_rect;
            existing.content = spec.content;
            existing.opacity = spec.opacity;
            existing.z_index = spec.z_index;
            return existing.entry_id;
        }

        let entry_id = CompositorSceneEntryId(self.next_entry_id.max(1));
        self.next_entry_id = entry_id.0.saturating_add(1);
        output.overlays.insert(
            spec.overlay_id,
            OutputOverlayEntry {
                entry_id,
                rect: spec.rect,
                clip_rect: spec.clip_rect,
                content: spec.content,
                opacity: spec.opacity,
                z_index: spec.z_index,
            },
        );
        entry_id
    }

    pub fn remove(
        &mut self,
        output_id: OutputId,
        overlay_id: &OutputOverlayId,
    ) -> Option<OutputOverlayEntry> {
        let output = self.outputs.get_mut(&output_id)?;
        let removed = output.overlays.remove(overlay_id);
        if output.overlays.is_empty() {
            self.outputs.remove(&output_id);
        }
        removed
    }

    pub fn clear_output(&mut self, output_id: OutputId) {
        self.outputs.remove(&output_id);
    }
}

#[cfg(test)]
mod tests {
    use crate::components::OutputId;
    use crate::resources::{
        OutputOverlayCollection, OutputOverlayId, OutputOverlaySpec, OutputOverlayState,
        OutputOverlayUpdate, PendingOutputOverlayControls, QuadContent, RenderColor, RenderRect,
    };

    #[test]
    fn pending_overlay_controls_keep_last_update_per_overlay_id() {
        let mut controls = PendingOutputOverlayControls::default();
        controls
            .output(OutputId(7))
            .set_overlay(OutputOverlaySpec::solid_color(
                "debug",
                RenderRect { x: 1, y: 2, width: 30, height: 40 },
                None,
                RenderColor { r: 1, g: 2, b: 3, a: 255 },
                0.5,
                3,
            ))
            .set_overlay(OutputOverlaySpec::solid_color(
                "debug",
                RenderRect { x: 5, y: 6, width: 70, height: 80 },
                None,
                RenderColor { r: 9, g: 8, b: 7, a: 255 },
                0.25,
                4,
            ));

        let taken = controls.take();
        assert_eq!(taken.len(), 1);
        assert_eq!(
            taken[0].overlay_updates,
            vec![OutputOverlayUpdate::Set(OutputOverlaySpec::solid_color(
                "debug",
                RenderRect { x: 5, y: 6, width: 70, height: 80 },
                None,
                RenderColor { r: 9, g: 8, b: 7, a: 255 },
                0.25,
                4,
            ))]
        );
    }

    #[test]
    fn clear_overlays_discards_previous_updates() {
        let mut controls = PendingOutputOverlayControls::default();
        controls
            .output(OutputId(9))
            .set_overlay(OutputOverlaySpec::solid_color(
                "debug",
                RenderRect { x: 1, y: 2, width: 30, height: 40 },
                None,
                RenderColor { r: 1, g: 2, b: 3, a: 255 },
                0.5,
                3,
            ))
            .clear_overlays();

        let taken = controls.take();
        assert!(taken[0].clear_overlays);
        assert!(taken[0].overlay_updates.is_empty());
    }

    #[test]
    fn output_overlay_state_reuses_entry_ids_for_same_overlay_id() {
        let mut state = OutputOverlayState::default();
        let first = state.upsert(
            OutputId(1),
            OutputOverlaySpec::solid_color(
                "debug",
                RenderRect { x: 0, y: 0, width: 100, height: 100 },
                None,
                RenderColor { r: 0, g: 0, b: 0, a: 255 },
                1.0,
                0,
            ),
        );
        let second = state.upsert(
            OutputId(1),
            OutputOverlaySpec::solid_color(
                "debug",
                RenderRect { x: 10, y: 10, width: 20, height: 20 },
                None,
                RenderColor { r: 255, g: 0, b: 0, a: 255 },
                0.5,
                8,
            ),
        );

        assert_eq!(first, second);
        let output = &state.outputs[&OutputId(1)];
        assert_eq!(output.overlays[&OutputOverlayId::from("debug")].rect.x, 10);
        assert_eq!(
            output.overlays[&OutputOverlayId::from("debug")].content,
            QuadContent::SolidColor { color: RenderColor { r: 255, g: 0, b: 0, a: 255 } }
        );
    }

    #[test]
    fn collection_iter_sorted_uses_z_index_then_overlay_id() {
        let mut collection = OutputOverlayCollection::default();
        collection.overlays.insert(
            OutputOverlayId::from("b"),
            crate::resources::OutputOverlayEntry {
                entry_id: crate::resources::CompositorSceneEntryId(2),
                rect: RenderRect { x: 0, y: 0, width: 1, height: 1 },
                clip_rect: None,
                content: QuadContent::SolidColor { color: RenderColor::default() },
                opacity: 1.0,
                z_index: 3,
            },
        );
        collection.overlays.insert(
            OutputOverlayId::from("a"),
            crate::resources::OutputOverlayEntry {
                entry_id: crate::resources::CompositorSceneEntryId(1),
                rect: RenderRect { x: 0, y: 0, width: 1, height: 1 },
                clip_rect: None,
                content: QuadContent::SolidColor { color: RenderColor::default() },
                opacity: 1.0,
                z_index: 3,
            },
        );

        let ordered = collection
            .iter_sorted()
            .map(|(overlay_id, _)| overlay_id.as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec!["a".to_owned(), "b".to_owned()]);
    }
}
