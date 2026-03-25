//! Layer-shell components and output-binding markers.
#![allow(missing_docs)]

use bevy_ecs::component::Component;
use bevy_ecs::prelude::Entity;
use serde::{Deserialize, Serialize};

/// Stored protocol state for a layer-shell surface after it has been materialized as an entity.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[require(
    crate::components::SurfaceGeometry,
    crate::components::BufferState,
    crate::components::SurfaceContentVersion,
    LayerAnchor,
    crate::components::WindowAnimation
)]
pub struct LayerShellSurface {
    pub namespace: String,
    pub layer: LayerLevel,
    pub desired_width: u32,
    pub desired_height: u32,
    pub exclusive_zone: i32,
    pub margins: LayerMargins,
}

/// Edge-facing output selector mirrored from protocol requests.
/// Runtime systems should prefer `LayerOnOutput` once it has been resolved. If this names an
/// output that does not currently exist, the layer stays detached instead of silently falling back
/// to the primary output.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesiredOutputName(pub Option<String>);

/// Relationship pointing from a layer surface to the output entity it targets.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[relationship(relationship_target = OutputLayers)]
pub struct LayerOnOutput(#[relationship] pub Entity);

/// Reverse relationship target stored on outputs so their attached layers can be enumerated.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
#[relationship_target(relationship = LayerOnOutput)]
pub struct OutputLayers(#[relationship] Vec<Entity>);

/// Edge anchors requested by the layer-shell client.
#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerAnchor {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl LayerAnchor {
    pub const fn centered() -> Self {
        Self { top: false, bottom: false, left: false, right: false }
    }
}

/// Layer-shell margins around the anchored rectangle.
#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerMargins {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

/// Layer-shell stacking levels from back to front.
#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerLevel {
    #[default]
    Background,
    Bottom,
    Top,
    Overlay,
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;

    use super::{DesiredOutputName, LayerAnchor, LayerOnOutput, LayerShellSurface, OutputLayers};
    use crate::components::{BufferState, SurfaceGeometry, WindowAnimation};

    #[test]
    fn layer_surface_requires_surface_runtime_components() {
        let mut world = World::new();
        let entity = world.spawn(LayerShellSurface::default()).id();

        assert!(world.get::<SurfaceGeometry>(entity).is_some());
        assert!(world.get::<BufferState>(entity).is_some());
        assert!(world.get::<LayerAnchor>(entity).is_some());
        assert!(world.get::<WindowAnimation>(entity).is_some());
    }

    #[test]
    fn layer_output_relationship_populates_output_target() {
        let mut world = World::new();
        let output = world.spawn_empty().id();
        let layer = world
            .spawn((LayerShellSurface::default(), DesiredOutputName(None), LayerOnOutput(output)))
            .id();

        let Some(output_layers) = world.get::<OutputLayers>(output) else {
            panic!("output should track related layers through the relationship target");
        };
        assert!(output_layers.0.contains(&layer), "output target should include the layer entity");
    }
}
