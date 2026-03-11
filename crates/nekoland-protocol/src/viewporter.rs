#[derive(Debug, Clone, Default)]
pub struct ViewporterState;

impl ViewporterState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wp_viewporter"]
    }
}
