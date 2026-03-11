use std::cell::RefCell;
use std::rc::Rc;

use smithay::backend::allocator::gbm::{
    GbmAllocator as SmithayGbmAllocator, GbmBufferFlags, GbmDevice,
};
use smithay::backend::drm::DrmDeviceFd;

use super::device::DrmDeviceState;

/// GBM device and allocator state.
///
/// Holds the GBM device created from the DRM file descriptor and an allocator
/// suitable for creating scanout buffers.
#[derive(Debug)]
pub struct GbmState {
    pub device: GbmDevice<DrmDeviceFd>,
    pub allocator: SmithayGbmAllocator<DrmDeviceFd>,
}

/// Shared reference to GBM state used between systems.
pub type SharedGbmState = Rc<RefCell<Option<GbmState>>>;

/// Initialise GBM from an open DRM device.
///
/// Returns `Err` if the GBM device cannot be created (e.g. the driver does not
/// support GBM, which is rare on modern Linux).
pub fn init_gbm(drm_state: &DrmDeviceState) -> Result<GbmState, Box<dyn std::error::Error>> {
    let gbm_device = GbmDevice::new(drm_state.fd.clone())?;
    let allocator = SmithayGbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    tracing::info!(path = %drm_state.path.display(), "GBM device initialised");
    Ok(GbmState { device: gbm_device, allocator })
}

/// ECS system: initialise GBM once the DRM device is ready.
///
/// This system runs on every frame but is a fast no-op after the first
/// successful initialisation.
pub fn gbm_allocator_system(
    drm_state: bevy_ecs::prelude::NonSend<super::device::SharedDrmState>,
    gbm_state: bevy_ecs::prelude::NonSendMut<SharedGbmState>,
) {
    if gbm_state.borrow().is_some() {
        return; // Already initialised.
    }

    let drm = drm_state.borrow();
    let Some(drm) = drm.as_ref() else {
        return; // DRM not yet ready.
    };

    match init_gbm(drm) {
        Ok(state) => *gbm_state.borrow_mut() = Some(state),
        Err(error) => {
            tracing::warn!(error = %error, "failed to initialise GBM allocator");
        }
    }
}
