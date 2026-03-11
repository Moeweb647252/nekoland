use std::collections::VecDeque;

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

    pub fn drain(&mut self) -> impl Iterator<Item = E> + '_ {
        self.queue.drain(..)
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

pub trait WaylandBridge {
    type Event;

    fn queue_event(&mut self, event: Self::Event);
}
