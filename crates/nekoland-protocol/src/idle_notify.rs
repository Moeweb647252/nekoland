/// Protocol state for idle-notify support.
#[derive(Debug, Clone, Default)]
pub struct IdleNotifyState;

impl IdleNotifyState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["ext_idle_notifier_v1"]
    }
}
