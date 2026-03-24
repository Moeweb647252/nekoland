//! XDG toplevel, popup, and configure-handling systems.

use bevy_ecs::prelude::Resource;
use nekoland_ecs::resources::{PendingXdgRequests, WindowLifecycleRequest};

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DeferredXdgRequests(PendingXdgRequests);

impl DeferredXdgRequests {
    pub(crate) fn take(&mut self) -> Vec<WindowLifecycleRequest> {
        self.0.take()
    }

    pub(crate) fn replace(&mut self, requests: Vec<WindowLifecycleRequest>) {
        self.0.replace(requests);
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, request: WindowLifecycleRequest) {
        self.0.push(request);
    }
}

pub mod configure;
pub mod popup;
#[cfg(test)]
pub mod toplevel;
