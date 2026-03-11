use bevy_app::App;

pub trait NekolandPlugin: Send + Sync + 'static {
    fn build(&self, app: &mut App);

    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

pub struct BevyPlugin<T: bevy_app::Plugin>(T);

impl<T: bevy_app::Plugin> BevyPlugin<T> {
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
