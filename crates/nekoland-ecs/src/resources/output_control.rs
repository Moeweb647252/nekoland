use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::WorkspaceCoord;
use crate::resources::{
    OutputOverlayId, OutputOverlaySpec, OutputOverlayUpdate, RenderColor, RenderRect,
};
use crate::selectors::{OutputName, OutputSelector, SurfaceId};

/// High-level output control updates staged by IPC, keybindings, or tests.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PendingOutputControls {
    controls: Vec<PendingOutputControl>,
}

/// One staged control update for a single output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PendingOutputControl {
    pub selector: OutputSelector,
    pub enabled: Option<bool>,
    pub configuration: Option<OutputControlConfiguration>,
    pub viewport_origin: Option<OutputViewportOrigin>,
    pub viewport_pan: Option<OutputViewportPan>,
    pub center_viewport_on: Option<SurfaceId>,
    pub clear_overlays: bool,
    pub overlay_updates: Vec<OutputOverlayUpdate>,
}

/// Desired output configuration staged for backend reconciliation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputControlConfiguration {
    pub mode: String,
    pub scale: Option<u32>,
}

/// Absolute viewport origin staged for one output.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputViewportOrigin {
    pub x: WorkspaceCoord,
    pub y: WorkspaceCoord,
}

/// Relative viewport motion staged for one output.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputViewportPan {
    pub delta_x: WorkspaceCoord,
    pub delta_y: WorkspaceCoord,
}

/// Mutable façade over one staged output control entry.
pub struct OutputControlHandle<'a> {
    control: &'a mut PendingOutputControl,
}

impl Default for PendingOutputControl {
    fn default() -> Self {
        Self {
            selector: OutputSelector::Primary,
            enabled: None,
            configuration: None,
            viewport_origin: None,
            viewport_pan: None,
            center_viewport_on: None,
            clear_overlays: false,
            overlay_updates: Vec::new(),
        }
    }
}

impl PendingOutputControls {
    pub fn select(&mut self, selector: OutputSelector) -> OutputControlHandle<'_> {
        let index = self.controls.iter().position(|control| control.selector == selector);
        let control = if let Some(index) = index {
            &mut self.controls[index]
        } else {
            self.controls
                .push(PendingOutputControl { selector, ..PendingOutputControl::default() });
            let last_index = self.controls.len() - 1;
            &mut self.controls[last_index]
        };

        OutputControlHandle { control }
    }

    pub fn named(&mut self, output: OutputName) -> OutputControlHandle<'_> {
        self.select(OutputSelector::Name(output))
    }

    pub fn primary(&mut self) -> OutputControlHandle<'_> {
        self.select(OutputSelector::Primary)
    }

    pub fn take(&mut self) -> Vec<PendingOutputControl> {
        std::mem::take(&mut self.controls)
    }

    pub fn replace(&mut self, controls: Vec<PendingOutputControl>) {
        self.controls = controls;
    }

    pub fn as_slice(&self) -> &[PendingOutputControl] {
        &self.controls
    }

    pub fn clear(&mut self) {
        self.controls.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}

impl OutputControlHandle<'_> {
    pub fn enable(&mut self) -> &mut Self {
        self.control.enabled = Some(true);
        self
    }

    pub fn disable(&mut self) -> &mut Self {
        self.control.enabled = Some(false);
        self
    }

    pub fn configure(&mut self, mode: impl Into<String>, scale: Option<u32>) -> &mut Self {
        self.control.configuration = Some(OutputControlConfiguration { mode: mode.into(), scale });
        self
    }

    pub fn move_viewport_to(&mut self, x: WorkspaceCoord, y: WorkspaceCoord) -> &mut Self {
        self.control.viewport_origin = Some(OutputViewportOrigin { x, y });
        self
    }

    pub fn pan_viewport_by(
        &mut self,
        delta_x: WorkspaceCoord,
        delta_y: WorkspaceCoord,
    ) -> &mut Self {
        let viewport_pan = self.control.viewport_pan.get_or_insert_default();
        viewport_pan.delta_x = viewport_pan.delta_x.saturating_add(delta_x);
        viewport_pan.delta_y = viewport_pan.delta_y.saturating_add(delta_y);
        self
    }

    pub fn center_viewport_on_window(&mut self, surface_id: SurfaceId) -> &mut Self {
        self.control.center_viewport_on = Some(surface_id);
        self
    }

    pub fn set_overlay_rect(
        &mut self,
        overlay_id: impl Into<OutputOverlayId>,
        rect: RenderRect,
        color: RenderColor,
        opacity: Option<f32>,
        z_index: Option<i32>,
        clip_rect: Option<RenderRect>,
    ) -> &mut Self {
        let spec = OutputOverlaySpec {
            overlay_id: overlay_id.into(),
            rect,
            clip_rect,
            color,
            opacity: opacity.unwrap_or(1.0).clamp(0.0, 1.0),
            z_index: z_index.unwrap_or_default(),
        };
        let overlay_id = spec.overlay_id.clone();
        self.control.overlay_updates.retain(|update| update.overlay_id() != &overlay_id);
        self.control.overlay_updates.push(OutputOverlayUpdate::Set(spec));
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

#[cfg(test)]
mod tests {
    use crate::resources::{OutputOverlayId, OutputOverlayUpdate, RenderColor, RenderRect};
    use crate::selectors::{OutputName, OutputSelector, SurfaceId};

    use super::{
        OutputControlConfiguration, OutputViewportOrigin, OutputViewportPan, PendingOutputControl,
        PendingOutputControls,
    };

    #[test]
    fn named_controls_merge_enable_disable_and_configure() {
        let mut controls = PendingOutputControls::default();
        controls
            .named(OutputName::from("Virtual-1"))
            .enable()
            .configure("1600x900@75", Some(2))
            .move_viewport_to(120, 240)
            .pan_viewport_by(-20, 15)
            .center_viewport_on_window(SurfaceId(7));

        assert_eq!(
            controls.as_slice(),
            &[PendingOutputControl {
                selector: OutputSelector::Name(OutputName::from("Virtual-1")),
                enabled: Some(true),
                configuration: Some(OutputControlConfiguration {
                    mode: "1600x900@75".to_owned(),
                    scale: Some(2),
                }),
                viewport_origin: Some(OutputViewportOrigin { x: 120, y: 240 }),
                viewport_pan: Some(OutputViewportPan { delta_x: -20, delta_y: 15 }),
                center_viewport_on: Some(SurfaceId(7)),
                clear_overlays: false,
                overlay_updates: Vec::new(),
            }]
        );
    }

    #[test]
    fn overlay_controls_keep_last_update_per_overlay_id() {
        let mut controls = PendingOutputControls::default();
        controls
            .named(OutputName::from("Virtual-1"))
            .set_overlay_rect(
                "debug",
                RenderRect { x: 1, y: 2, width: 100, height: 50 },
                RenderColor { r: 10, g: 20, b: 30, a: 255 },
                Some(0.25),
                Some(3),
                None,
            )
            .set_overlay_rect(
                "debug",
                RenderRect { x: 5, y: 6, width: 20, height: 30 },
                RenderColor { r: 40, g: 50, b: 60, a: 200 },
                Some(0.75),
                Some(9),
                Some(RenderRect { x: 6, y: 7, width: 10, height: 11 }),
            );

        assert_eq!(controls.as_slice().len(), 1);
        assert_eq!(
            controls.as_slice()[0].overlay_updates,
            vec![OutputOverlayUpdate::Set(crate::resources::OutputOverlaySpec {
                overlay_id: OutputOverlayId::from("debug"),
                rect: RenderRect { x: 5, y: 6, width: 20, height: 30 },
                clip_rect: Some(RenderRect { x: 6, y: 7, width: 10, height: 11 }),
                color: RenderColor { r: 40, g: 50, b: 60, a: 200 },
                opacity: 0.75,
                z_index: 9,
            })]
        );
    }

    #[test]
    fn clear_overlays_discards_pending_overlay_updates() {
        let mut controls = PendingOutputControls::default();
        controls
            .named(OutputName::from("Virtual-1"))
            .set_overlay_rect(
                "debug",
                RenderRect { x: 1, y: 2, width: 100, height: 50 },
                RenderColor { r: 10, g: 20, b: 30, a: 255 },
                Some(0.25),
                Some(3),
                None,
            )
            .clear_overlays();

        assert!(controls.as_slice()[0].clear_overlays);
        assert!(controls.as_slice()[0].overlay_updates.is_empty());
    }
}
