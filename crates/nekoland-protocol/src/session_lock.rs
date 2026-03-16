use crate::ProtocolGlobals;

/// Protocol state for session-lock support.
#[derive(Debug, Clone, Default)]
pub struct SessionLockState;

impl ProtocolGlobals for SessionLockState {
    const GLOBALS: &'static [&'static str] = &["ext_session_lock_manager_v1"];
}
