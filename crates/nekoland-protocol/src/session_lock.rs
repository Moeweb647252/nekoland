#[derive(Debug, Clone, Default)]
pub struct SessionLockState;

impl SessionLockState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["ext_session_lock_manager_v1"]
    }
}
