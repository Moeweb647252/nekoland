#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GbmAllocator {
    pub format: String,
}

pub fn gbm_allocator_system() {
    tracing::trace!("gbm allocator system tick");
}
