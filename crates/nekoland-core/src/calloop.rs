use bevy_app::App;
use bevy_ecs::error::Result as BevyResult;
use bevy_ecs::prelude::{NonSendMut, Res};
use calloop::LoopHandle;
use std::time::Duration;

use crate::app::{RunLoopSettings, WaylandSubApp};
use crate::error::NekolandError;

/// A deferred installer that only becomes usable once the top-level calloop loop exists.
///
/// Plugins register these closures during setup, and `NekolandApp::run` executes them when it
/// finally owns a live `LoopHandle`.
type CalloopInstaller =
    dyn for<'a> FnMut(&LoopHandle<'a, ()>) -> Result<(), NekolandError> + 'static;

/// Collects backend/protocol calloop sources while plugins are being assembled.
#[derive(Default)]
pub struct CalloopSourceRegistry {
    installers: Vec<Box<CalloopInstaller>>,
}

impl CalloopSourceRegistry {
    /// Defers source registration until the runtime event loop has been created.
    pub fn push<F>(&mut self, installer: F)
    where
        F: for<'a> FnMut(&LoopHandle<'a, ()>) -> Result<(), NekolandError> + 'static,
    {
        self.installers.push(Box::new(installer));
    }

    /// Installs every queued source exactly once, then clears the registry to avoid duplicate
    /// registrations if the app loop is restarted in tests.
    pub fn install_all<'a>(&mut self, handle: &LoopHandle<'a, ()>) -> Result<(), NekolandError> {
        for installer in &mut self.installers {
            installer(handle)?;
        }

        self.installers.clear();
        Ok(())
    }
}

/// The live calloop event loop used by runtime backends and protocol sources.
///
/// This is intentionally owned by the `wayland` subapp so extract-time polling stays inside the
/// platform runtime boundary rather than in the outer root runner.
pub struct WaylandCalloopRuntime {
    event_loop: calloop::EventLoop<'static, ()>,
}

impl WaylandCalloopRuntime {
    /// Wraps the live calloop event loop installed for the Wayland sub-app.
    pub fn new(event_loop: calloop::EventLoop<'static, ()>) -> Self {
        Self { event_loop }
    }

    /// Dispatches the event loop once with the provided timeout.
    pub fn dispatch(&mut self, timeout: Duration) -> Result<(), calloop::Error> {
        self.event_loop.dispatch(timeout, &mut ())
    }
}

/// Extract-phase system that polls the live Wayland calloop runtime.
pub fn dispatch_wayland_calloop_system(
    runtime: Option<NonSendMut<'_, WaylandCalloopRuntime>>,
    settings: Option<Res<'_, RunLoopSettings>>,
) -> BevyResult {
    let Some(mut runtime) = runtime else {
        return Ok(());
    };
    let timeout = settings
        .as_deref()
        .map(|settings| settings.frame_timeout)
        .unwrap_or_else(|| Duration::from_millis(16));
    runtime.dispatch(timeout).map_err(|error| NekolandError::Runtime(error.to_string()).into())
}

/// Ensures the appropriate world owns a [`CalloopSourceRegistry`] and passes it to the caller.
pub fn with_wayland_calloop_registry<R>(
    app: &mut App,
    f: impl FnOnce(&mut CalloopSourceRegistry) -> R,
) -> R {
    let wayland_world = if app.get_sub_app(WaylandSubApp).is_some() {
        app.sub_app_mut(WaylandSubApp).world_mut()
    } else {
        app.world_mut()
    };
    if wayland_world.get_non_send_resource::<CalloopSourceRegistry>().is_none() {
        wayland_world.insert_non_send_resource(CalloopSourceRegistry::default());
    }

    let mut registry = wayland_world
        .get_non_send_resource_mut::<CalloopSourceRegistry>()
        .expect("wayland calloop registry should exist after initialization");
    f(&mut registry)
}
