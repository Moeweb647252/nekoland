use std::collections::VecDeque;

/// A small FIFO bridge used when protocol/backend callbacks cannot mutate ECS world state
/// directly and must hand work over to the next scheduled frame phase.
#[derive(Debug, Clone)]
pub struct EventBridge<E> {
    queue: VecDeque<E>,
}

impl<E> Default for EventBridge<E> {
    fn default() -> Self {
        Self { queue: VecDeque::new() }
    }
}

impl<E> EventBridge<E> {
    pub fn push(&mut self, event: E) {
        self.queue.push_back(event);
    }

    /// Drains events in arrival order so the receiving system can flush the bridge exactly once
    /// per frame without cloning buffered items.
    pub fn drain(&mut self) -> impl Iterator<Item = E> + '_ {
        self.queue.drain(..)
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Implemented by protocol adapters that enqueue events into the shared bridge instead of
/// touching ECS resources from Smithay callbacks directly.
pub trait WaylandBridge {
    type Event;

    fn queue_event(&mut self, event: Self::Event);
}
