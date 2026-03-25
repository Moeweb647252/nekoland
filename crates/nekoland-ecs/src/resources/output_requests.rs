//! Backend-facing output request queue used after high-level control resolution.

#![allow(missing_docs)]

use crate::kinds::CompositorRequestQueue;
use serde::{Deserialize, Serialize};

/// Internal backend bridge actions for outputs.
///
/// New user-facing control flows should go through `PendingOutputControls` and `OutputOps`.
/// This queue remains as the backend-facing contract after high-level output control updates have
/// been folded into backend-specific request application.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputServerAction {
    Configure {
        output: String,
        mode: String,
        #[serde(default)]
        scale: Option<u32>,
    },
    Enable {
        output: String,
    },
    Disable {
        output: String,
    },
}

/// One output-management request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputServerRequest {
    pub action: OutputServerAction,
}

/// Queue of pending backend-bridge output requests.
pub type PendingOutputServerRequests = CompositorRequestQueue<OutputServerRequest>;
