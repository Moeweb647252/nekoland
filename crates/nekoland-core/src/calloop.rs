use calloop::LoopHandle;

use crate::error::NekolandError;

type CalloopInstaller =
    dyn for<'a> FnMut(&LoopHandle<'a, ()>) -> Result<(), NekolandError> + 'static;

#[derive(Default)]
pub struct CalloopSourceRegistry {
    installers: Vec<Box<CalloopInstaller>>,
}

impl CalloopSourceRegistry {
    pub fn push<F>(&mut self, installer: F)
    where
        F: for<'a> FnMut(&LoopHandle<'a, ()>) -> Result<(), NekolandError> + 'static,
    {
        self.installers.push(Box::new(installer));
    }

    pub fn install_all<'a>(&mut self, handle: &LoopHandle<'a, ()>) -> Result<(), NekolandError> {
        for installer in &mut self.installers {
            installer(handle)?;
        }

        self.installers.clear();
        Ok(())
    }
}
