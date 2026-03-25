//! High-level tiling control queues used by IPC, keybindings, and shell systems.

#![allow(missing_docs)]

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use super::{HorizontalDirection, TilingPanDirection, VerticalDirection};

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingTilingControls {
    controls: Vec<PendingTilingControl>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PendingTilingControl {
    FocusColumn { direction: HorizontalDirection },
    FocusWindow { direction: VerticalDirection },
    MoveColumn { direction: HorizontalDirection },
    MoveWindow { direction: VerticalDirection },
    ConsumeIntoColumn { direction: HorizontalDirection },
    ExpelFromColumn { direction: HorizontalDirection },
    PanViewport { direction: TilingPanDirection },
}

pub struct TilingControlHandle<'a> {
    pending: &'a mut PendingTilingControls,
}

impl PendingTilingControls {
    pub fn api(&mut self) -> TilingControlHandle<'_> {
        TilingControlHandle { pending: self }
    }

    pub fn push(&mut self, control: PendingTilingControl) {
        self.controls.push(control);
    }

    pub fn take(&mut self) -> Vec<PendingTilingControl> {
        std::mem::take(&mut self.controls)
    }

    pub fn replace(&mut self, controls: Vec<PendingTilingControl>) {
        self.controls = controls;
    }

    pub fn as_slice(&self) -> &[PendingTilingControl] {
        &self.controls
    }

    pub fn clear(&mut self) {
        self.controls.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}

impl TilingControlHandle<'_> {
    pub fn focus_column(&mut self, direction: HorizontalDirection) -> &mut Self {
        self.pending.push(PendingTilingControl::FocusColumn { direction });
        self
    }

    pub fn focus_window(&mut self, direction: VerticalDirection) -> &mut Self {
        self.pending.push(PendingTilingControl::FocusWindow { direction });
        self
    }

    pub fn move_column(&mut self, direction: HorizontalDirection) -> &mut Self {
        self.pending.push(PendingTilingControl::MoveColumn { direction });
        self
    }

    pub fn move_window(&mut self, direction: VerticalDirection) -> &mut Self {
        self.pending.push(PendingTilingControl::MoveWindow { direction });
        self
    }

    pub fn consume_into_column(&mut self, direction: HorizontalDirection) -> &mut Self {
        self.pending.push(PendingTilingControl::ConsumeIntoColumn { direction });
        self
    }

    pub fn expel_from_column(&mut self, direction: HorizontalDirection) -> &mut Self {
        self.pending.push(PendingTilingControl::ExpelFromColumn { direction });
        self
    }

    pub fn pan_viewport(&mut self, direction: TilingPanDirection) -> &mut Self {
        self.pending.push(PendingTilingControl::PanViewport { direction });
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::resources::{
        HorizontalDirection, PendingTilingControl, PendingTilingControls, TilingPanDirection,
        VerticalDirection,
    };

    #[test]
    fn tiling_controls_preserve_sequence() {
        let mut controls = PendingTilingControls::default();
        controls
            .api()
            .focus_column(HorizontalDirection::Right)
            .move_window(VerticalDirection::Down)
            .consume_into_column(HorizontalDirection::Left)
            .pan_viewport(TilingPanDirection::Right);

        assert_eq!(
            controls.as_slice(),
            &[
                PendingTilingControl::FocusColumn { direction: HorizontalDirection::Right },
                PendingTilingControl::MoveWindow { direction: VerticalDirection::Down },
                PendingTilingControl::ConsumeIntoColumn { direction: HorizontalDirection::Left },
                PendingTilingControl::PanViewport { direction: TilingPanDirection::Right },
            ]
        );
    }
}
