#[derive(Debug, Clone, Default)]
pub struct XdgDecorationState;

impl XdgDecorationState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zxdg_decoration_manager_v1"]
    }
}
