#[derive(Debug, Clone, Default)]
pub struct XdgShellState;

impl XdgShellState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["xdg_wm_base"]
    }
}
