use crate::kinds::CompositorRequestQueue;
use serde::{Deserialize, Serialize};

/// External command request waiting to be launched by the shell command subsystem.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalCommandRequest {
    pub origin: String,
    pub candidates: Vec<Vec<String>>,
}

/// Queue of external command launch requests collected during the current frame.
pub type PendingExternalCommandRequests = CompositorRequestQueue<ExternalCommandRequest>;
