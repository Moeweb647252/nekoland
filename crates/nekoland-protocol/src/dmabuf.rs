#[derive(Debug, Clone, Default)]
pub struct DmabufState;

impl DmabufState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwp_linux_dmabuf_v1"]
    }
}
