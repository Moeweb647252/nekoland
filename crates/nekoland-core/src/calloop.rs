use calloop::LoopHandle;

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
