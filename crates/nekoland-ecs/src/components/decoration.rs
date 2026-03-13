use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Whether the compositor should draw server-side decorations for a surface.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerDecoration {
    /// `true` when borders/titlebars should be rendered by the compositor.
    pub enabled: bool,
}

/// Border styling associated with server-side decorations.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BorderTheme {
    /// Border width in logical pixels.
    pub width: u32,
    /// Border color encoded as a config hex string.
    pub color: String,
}
