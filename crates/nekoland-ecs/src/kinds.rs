//! Semantic marker traits that classify the project's concrete event and request payloads.
//!
//! These traits intentionally do not replace the existing concrete names. They provide a
//! lightweight way to express architectural roles across crates without flattening the domain
//! vocabulary into abstract umbrella types.
//!
//! Intended usage:
//! - `ProtocolEvent`: callback-driven protocol facts entering the scheduled ECS world
//! - `BackendEvent`: backend/device/output facts entering the compositor
//! - `CompositorRequest`: requests asking the compositor to mutate state
//! - `CompositorEvent`: facts the compositor has already realized internally
//! - `SubscriptionEvent`: events emitted to external IPC subscribers

use std::marker::PhantomData;

use crate::events::{
    ExternalCommandFailed, ExternalCommandLaunched, GestureSwipe, KeyPress, OutputConnected,
    OutputDisconnected, PointerButton, PointerMotion, WindowClosed, WindowCreated, WindowMoved,
};
use crate::resources::{
    BackendInputEvent, ExternalCommandRequest, LayerLifecycleRequest, OutputEventRecord,
    OutputPresentationEventRecord, OutputServerRequest, PopupServerRequest, WindowLifecycleRequest,
    WindowServerRequest, X11LifecycleRequest,
};
use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Marker for protocol-driven facts entering ECS from Wayland callbacks.
pub trait ProtocolEvent: Send + Sync + 'static {}
/// Marker for backend/device facts entering ECS from input or output backends.
pub trait BackendEvent: Send + Sync + 'static {}
/// Marker for resources that request a compositor-side state mutation.
pub trait CompositorRequest: Send + Sync + 'static {}
/// Marker for facts the compositor has already realized internally.
pub trait CompositorEvent: Send + Sync + 'static {}
/// Marker for events emitted to external IPC subscribers.
pub trait SubscriptionEvent: Send + Sync + 'static {}

/// Small typed queue resource used to move a family of payloads across one frame boundary.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrameQueue<T, Tag = ()> {
    items: Vec<T>,
    marker: PhantomData<fn() -> Tag>,
}

impl<T, Tag> Default for FrameQueue<T, Tag> {
    fn default() -> Self {
        Self { items: Vec::new(), marker: PhantomData }
    }
}

impl<T, Tag> FrameQueue<T, Tag> {
    /// Build a queue from an already-collected payload vector.
    pub fn from_items(items: Vec<T>) -> Self {
        Self { items, marker: PhantomData }
    }

    /// Append one payload for consumption later in the frame.
    pub fn push(&mut self, item: T) {
        self.items.push(item);
    }

    /// Append multiple payloads while preserving their incoming order.
    pub fn extend<I>(&mut self, items: I)
    where
        I: IntoIterator<Item = T>,
    {
        self.items.extend(items);
    }

    /// Drain every queued payload, leaving the queue empty.
    pub fn drain(&mut self) -> std::vec::Drain<'_, T> {
        self.items.drain(..)
    }

    /// Take ownership of all queued payloads in one move.
    pub fn take(&mut self) -> Vec<T> {
        std::mem::take(&mut self.items)
    }

    /// Replace the current queue contents wholesale.
    pub fn replace(&mut self, items: Vec<T>) {
        self.items = items;
    }

    /// Drop every queued payload without iterating them.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Return whether the queue currently has no pending payloads.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Return the current number of queued payloads.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Borrow an iterator over queued payloads without consuming them.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    /// Borrow the underlying contiguous payload slice for read-only inspection.
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }
}

/// Tag that distinguishes protocol-event queues from other frame queues in type aliases.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtocolEventQueueTag;

/// Tag that distinguishes backend-event queues from other frame queues in type aliases.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendEventQueueTag;

/// Tag that distinguishes compositor-request queues from other frame queues in type aliases.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompositorRequestQueueTag;

/// Tag that distinguishes subscription-event queues from other frame queues in type aliases.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubscriptionEventQueueTag;

/// Queue used for protocol callback payloads entering the ECS world.
pub type ProtocolEventQueue<T> = FrameQueue<T, ProtocolEventQueueTag>;
/// Queue used for backend-originated payloads entering the ECS world.
pub type BackendEventQueue<T> = FrameQueue<T, BackendEventQueueTag>;
/// Queue used for pending server-side state mutations.
pub type CompositorRequestQueue<T> = FrameQueue<T, CompositorRequestQueueTag>;
/// Queue used for events exported to IPC subscription streams.
pub type SubscriptionEventQueue<T> = FrameQueue<T, SubscriptionEventQueueTag>;

impl ProtocolEvent for WindowLifecycleRequest {}
impl ProtocolEvent for LayerLifecycleRequest {}
impl ProtocolEvent for X11LifecycleRequest {}

impl BackendEvent for BackendInputEvent {}
impl BackendEvent for OutputEventRecord {}
impl BackendEvent for OutputPresentationEventRecord {}

impl CompositorRequest for WindowServerRequest {}
impl CompositorRequest for PopupServerRequest {}
impl CompositorRequest for OutputServerRequest {}
impl CompositorRequest for ExternalCommandRequest {}

impl CompositorEvent for WindowCreated {}
impl CompositorEvent for WindowClosed {}
impl CompositorEvent for WindowMoved {}
impl CompositorEvent for OutputConnected {}
impl CompositorEvent for OutputDisconnected {}
impl CompositorEvent for KeyPress {}
impl CompositorEvent for PointerMotion {}
impl CompositorEvent for PointerButton {}
impl CompositorEvent for GestureSwipe {}
impl CompositorEvent for ExternalCommandLaunched {}
impl CompositorEvent for ExternalCommandFailed {}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_protocol_event<T: ProtocolEvent>() {}
    fn assert_backend_event<T: BackendEvent>() {}
    fn assert_compositor_request<T: CompositorRequest>() {}
    fn assert_compositor_event<T: CompositorEvent>() {}

    #[test]
    fn local_protocol_types_have_expected_classification() {
        assert_protocol_event::<WindowLifecycleRequest>();
        assert_protocol_event::<LayerLifecycleRequest>();
        assert_protocol_event::<X11LifecycleRequest>();
    }

    #[test]
    fn local_backend_types_have_expected_classification() {
        assert_backend_event::<BackendInputEvent>();
        assert_backend_event::<OutputEventRecord>();
        assert_backend_event::<OutputPresentationEventRecord>();
    }

    #[test]
    fn local_request_types_have_expected_classification() {
        assert_compositor_request::<WindowServerRequest>();
        assert_compositor_request::<PopupServerRequest>();
        assert_compositor_request::<OutputServerRequest>();
        assert_compositor_request::<ExternalCommandRequest>();
    }

    #[test]
    fn local_message_types_have_expected_classification() {
        assert_compositor_event::<WindowCreated>();
        assert_compositor_event::<WindowClosed>();
        assert_compositor_event::<WindowMoved>();
        assert_compositor_event::<OutputConnected>();
        assert_compositor_event::<OutputDisconnected>();
        assert_compositor_event::<KeyPress>();
        assert_compositor_event::<PointerMotion>();
        assert_compositor_event::<PointerButton>();
        assert_compositor_event::<GestureSwipe>();
        assert_compositor_event::<ExternalCommandLaunched>();
        assert_compositor_event::<ExternalCommandFailed>();
    }

    #[test]
    fn generic_queue_api_operates_without_payload_default() {
        let mut queue = CompositorRequestQueue::<WindowServerRequest>::default();
        assert!(queue.is_empty());

        queue.push(WindowServerRequest {
            surface_id: 42,
            action: crate::resources::WindowServerAction::Close,
        });
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.as_slice()[0].surface_id, 42);

        let drained = queue.drain().collect::<Vec<_>>();
        assert_eq!(drained.len(), 1);
        assert!(queue.is_empty());
    }
}
