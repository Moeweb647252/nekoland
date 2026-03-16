use crate::ProtocolGlobals;

/// Protocol state for idle-notify support.
#[derive(Debug, Clone, Default)]
pub struct IdleNotifyState;

impl ProtocolGlobals for IdleNotifyState {
    const GLOBALS: &'static [&'static str] = &["ext_idle_notifier_v1"];
}
