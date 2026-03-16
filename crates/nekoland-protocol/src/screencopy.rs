use crate::ProtocolGlobals;

/// Protocol state for screencopy and image-copy capture support.
#[derive(Debug, Clone, Default)]
pub struct ScreencopyState;

impl ProtocolGlobals for ScreencopyState {
    const GLOBALS: &'static [&'static str] =
        &["zwlr_screencopy_manager_v1", "ext_image_copy_capture_manager_v1"];
}
