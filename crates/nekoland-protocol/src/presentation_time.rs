#[derive(Debug, Clone, Default)]
pub struct PresentationTimeState;

impl PresentationTimeState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wp_presentation"]
    }
}
