use bevy_app::App;

/// Small plugin abstraction used by nekoland's internal plugin registry.
///
/// It mirrors Bevy's `Plugin` trait but keeps the surface area minimal and
/// object-safe so plugins can be stored behind trait objects.
pub trait NekolandPlugin: Send + Sync + 'static {
    /// Register systems, resources, and messages on the target app.
    fn build(&self, app: &mut App);

    /// Human-readable plugin name used for tracing and diagnostics.
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Adapter that lets a regular Bevy plugin participate in the internal
/// `NekolandPlugin` registry.
pub struct BevyPlugin<T: bevy_app::Plugin>(T);

impl<T: bevy_app::Plugin> BevyPlugin<T> {
    /// Wrap a Bevy plugin so it can be stored as a `NekolandPlugin`.
    pub fn new(plugin: T) -> Self {
        Self(plugin)
    }
}

impl<T: bevy_app::Plugin> NekolandPlugin for BevyPlugin<T> {
    fn build(&self, app: &mut App) {
        self.0.build(app);
    }

    fn name(&self) -> &'static str {
        std::any::type_name::<T>()
    }
}

/// Adapter that lets an internal `NekolandPlugin` be installed into a Bevy `SubApp`.
pub struct NekolandAppPlugin<T: NekolandPlugin>(T);

impl<T: NekolandPlugin> NekolandAppPlugin<T> {
    pub fn new(plugin: T) -> Self {
        Self(plugin)
    }
}

impl<T: NekolandPlugin> bevy_app::Plugin for NekolandAppPlugin<T> {
    fn build(&self, app: &mut App) {
        self.0.build(app);
    }

    fn name(&self) -> &str {
        self.0.name()
    }
}
