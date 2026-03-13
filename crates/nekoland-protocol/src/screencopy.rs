/// Protocol state for screencopy and image-copy capture support.
#[derive(Debug, Clone, Default)]
pub struct ScreencopyState;

impl ScreencopyState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwlr_screencopy_manager_v1", "ext_image_copy_capture_manager_v1"]
    }
}
