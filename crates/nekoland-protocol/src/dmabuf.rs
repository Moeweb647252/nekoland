/// Protocol state for linux-dmabuf buffer import support.
#[derive(Debug, Clone, Default)]
pub struct DmabufState;

impl DmabufState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwp_linux_dmabuf_v1"]
    }
}
