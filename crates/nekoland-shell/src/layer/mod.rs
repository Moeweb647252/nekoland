//! Layer-shell lifecycle and arrangement systems.

use bevy_ecs::prelude::Resource;
use nekoland_ecs::resources::{LayerLifecycleRequest, PendingLayerRequests};

#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DeferredLayerRequests(PendingLayerRequests);

impl DeferredLayerRequests {
    pub(crate) fn take(&mut self) -> Vec<LayerLifecycleRequest> {
        self.0.take()
    }

    pub(crate) fn replace(&mut self, requests: Vec<LayerLifecycleRequest>) {
        self.0.replace(requests);
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, request: LayerLifecycleRequest) {
        self.0.push(request);
    }
}

pub mod arrange;
