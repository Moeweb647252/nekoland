/// Protocol state for XDG decoration negotiation.
#[derive(Debug, Clone, Default)]
pub struct XdgDecorationState;

impl XdgDecorationState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zxdg_decoration_manager_v1"]
    }
}
