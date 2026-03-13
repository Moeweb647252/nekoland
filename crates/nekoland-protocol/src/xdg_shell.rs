/// Protocol state for XDG shell support.
#[derive(Debug, Clone, Default)]
pub struct XdgShellState;

impl XdgShellState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["xdg_wm_base"]
    }
}
