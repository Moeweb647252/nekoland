use bevy_ecs::prelude::Resource;

/// Process-lifetime flags that let subsystems request orderly application shutdown.
#[derive(Debug, Clone, Default, Resource, PartialEq, Eq)]
pub struct AppLifecycleState {
    /// Set when some subsystem has requested an orderly shutdown.
    pub quit_requested: bool,
}
