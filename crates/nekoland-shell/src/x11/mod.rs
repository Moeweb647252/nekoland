//! XWayland/X11 shell integration.

use bevy_ecs::prelude::Resource;
use nekoland_ecs::resources::{PendingX11Requests, X11LifecycleRequest};

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DeferredX11Requests(PendingX11Requests);

impl DeferredX11Requests {
    pub(crate) fn take(&mut self) -> Vec<X11LifecycleRequest> {
        self.0.take()
    }

    pub(crate) fn replace(&mut self, requests: Vec<X11LifecycleRequest>) {
        self.0.replace(requests);
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, request: X11LifecycleRequest) {
        self.0.push(request);
    }
}

pub mod xwayland;
