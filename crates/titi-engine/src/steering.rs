//! Mid-flight steering: a bounded queue drained at the next step boundary.
//!
//! The user types while the agent works; the message is injected before the
//! next provider attempt instead of aborting and restarting the turn.
//!
//! Spec: `docs/research/empryo-port/README.md` (E4).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use smol_str::SmolStr;

/// How many steering messages may wait before the oldest is evicted.
pub const STEERING_CAPACITY: usize = 5;

#[derive(Default)]
struct Inner {
    queue: VecDeque<SmolStr>,
}

/// A shared, bounded steering queue. Cheap to clone.
#[derive(Clone)]
pub struct Steering {
    inner: Arc<Mutex<Inner>>,
    capacity: usize,
}

impl Default for Steering {
    fn default() -> Self {
        Self::new(STEERING_CAPACITY)
    }
}

impl Steering {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            capacity: capacity.max(1),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Queues a steering message. Returns the evicted message when the queue
    /// was already full, so the caller can report what the user lost.
    pub fn push(&self, text: impl Into<SmolStr>) -> Option<SmolStr> {
        let text = text.into();
        if text.trim().is_empty() {
            return None;
        }
        let mut inner = self.lock();
        inner.queue.push_back(text);
        // At most one message can overflow, since each push adds exactly one.
        if inner.queue.len() > self.capacity {
            return inner.queue.pop_front();
        }
        None
    }

    /// Takes everything queued, oldest first. Called at a step boundary.
    pub fn drain(&self) -> Vec<SmolStr> {
        let mut inner = self.lock();
        inner.queue.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.lock().queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_returns_messages_oldest_first_and_empties() {
        let steering = Steering::new(4);
        steering.push("first");
        steering.push("second");
        assert_eq!(steering.len(), 2);

        let drained: Vec<String> = steering.drain().into_iter().map(String::from).collect();
        assert_eq!(drained, vec!["first".to_owned(), "second".to_owned()]);
        assert!(steering.is_empty());
        assert!(steering.drain().is_empty());
    }

    #[test]
    fn the_queue_is_bounded_and_reports_the_eviction() {
        let steering = Steering::new(2);
        assert_eq!(steering.push("one"), None);
        assert_eq!(steering.push("two"), None);
        let evicted = steering.push("three");
        assert_eq!(evicted.as_deref(), Some("one"));
        assert_eq!(steering.len(), 2);
        let drained: Vec<String> = steering.drain().into_iter().map(String::from).collect();
        assert_eq!(drained, vec!["two".to_owned(), "three".to_owned()]);
    }

    #[test]
    fn blank_messages_are_ignored() {
        let steering = Steering::new(2);
        assert_eq!(steering.push("   "), None);
        assert!(steering.is_empty());
    }
}
