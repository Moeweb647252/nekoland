#[derive(Debug, Clone, Default)]
pub struct IdleNotifyState;

impl IdleNotifyState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["ext_idle_notifier_v1"]
    }
}
