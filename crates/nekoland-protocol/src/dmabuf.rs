use crate::ProtocolGlobals;

/// Protocol state for linux-dmabuf buffer import support.
#[derive(Debug, Clone, Default)]
pub struct DmabufState;

impl ProtocolGlobals for DmabufState {
    const GLOBALS: &'static [&'static str] = &["zwp_linux_dmabuf_v1"];
}
