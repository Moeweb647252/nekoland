use std::cell::RefCell;
use std::rc::Rc;

use smithay::backend::allocator::gbm::{
    GbmAllocator as SmithayGbmAllocator, GbmBufferFlags, GbmDevice,
};
use smithay::backend::drm::DrmDeviceFd;

use super::device::DrmDeviceState;
use super::session::SharedDrmSessionState;

#[derive(Debug)]
pub struct GbmState {
    pub device: GbmDevice<DrmDeviceFd>,
    pub allocator: SmithayGbmAllocator<DrmDeviceFd>,
}

pub type SharedGbmState = Rc<RefCell<Option<GbmState>>>;

pub fn init_gbm(drm_state: &DrmDeviceState) -> Result<GbmState, Box<dyn std::error::Error>> {
    let gbm_device = GbmDevice::new(drm_state.fd.clone())?;
    let allocator = SmithayGbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    tracing::info!(path = %drm_state.path.display(), "GBM device initialised");
    Ok(GbmState { device: gbm_device, allocator })
}

pub fn ensure_gbm_allocator(
    session_state: &SharedDrmSessionState,
    drm_state: &super::device::SharedDrmState,
    gbm_state: &SharedGbmState,
) {
    if !session_state.borrow().active {
        return;
    }

    if gbm_state.borrow().is_some() {
        return;
    }

    let drm = drm_state.borrow();
    let Some(drm) = drm.as_ref() else {
        return;
    };

    match init_gbm(drm) {
        Ok(state) => *gbm_state.borrow_mut() = Some(state),
        Err(error) => {
            tracing::warn!(error = %error, "failed to initialise GBM allocator");
        }
    }
}
