#[derive(Debug, Clone, Default)]
pub struct ScreencopyState;

impl ScreencopyState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwlr_screencopy_manager_v1", "ext_image_copy_capture_manager_v1"]
    }
}
