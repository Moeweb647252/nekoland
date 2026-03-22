use crate::kinds::FrameQueue;
use serde::{Deserialize, Serialize};

/// Low-level backend input actions before they are translated into higher-level ECS messages.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BackendInputAction {
    Key { keycode: u32, pressed: bool },
    PointerMoved { x: f64, y: f64 },
    PointerDelta { dx: f64, dy: f64 },
    PointerButton { button_code: u32, pressed: bool },
    PointerAxis { horizontal: f64, vertical: f64 },
    FocusChanged { focused: bool },
}

impl Default for BackendInputAction {
    fn default() -> Self {
        Self::FocusChanged { focused: true }
    }
}

/// One backend input record together with the device label that produced it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BackendInputEvent {
    pub device: String,
    pub action: BackendInputAction,
}

/// Backend input queue consumed by the input schedule.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingBackendInputEventsTag;

pub type PendingBackendInputEvents = FrameQueue<BackendInputEvent, PendingBackendInputEventsTag>;

/// Shell-facing alias used once backend input has crossed the platform boundary.
///
/// The underlying event shape is currently identical; the alias exists so main-app code can talk
/// in terms of platform input rather than backend implementation details.
pub type PlatformInputAction = BackendInputAction;

/// Shell-facing alias for one normalized platform input record.
pub type PlatformInputEvent = BackendInputEvent;

/// Shell-facing alias for the input mailbox exported through `WaylandIngress`.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingPlatformInputEventsTag;

pub type PendingPlatformInputEvents = FrameQueue<PlatformInputEvent, PendingPlatformInputEventsTag>;

/// Copy of backend input records forwarded to protocol-side consumers that need the same physical
/// input stream.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingProtocolInputEventsTag;

pub type PendingProtocolInputEvents = FrameQueue<BackendInputEvent, PendingProtocolInputEventsTag>;
