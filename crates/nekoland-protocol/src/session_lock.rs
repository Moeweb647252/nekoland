/// Protocol state for session-lock support.
#[derive(Debug, Clone, Default)]
pub struct SessionLockState;

impl SessionLockState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["ext_session_lock_manager_v1"]
    }
}
