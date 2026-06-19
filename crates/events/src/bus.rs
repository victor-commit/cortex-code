//! The in-process typed publish/subscribe event bus.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::Event;

/// A subscriber that observes every event published on the bus after it is attached.
///
/// Implemented for any `Fn(&Event) + Send + Sync`, so closures can be subscribed directly, and
/// for explicit subscriber structs that need to retain state across deliveries.
pub trait Subscriber: Send + Sync {
    /// Handle a single delivered event.
    fn on_event(&self, event: &Event);
}

impl<F> Subscriber for F
where
    F: Fn(&Event) + Send + Sync,
{
    fn on_event(&self, event: &Event) {
        self(event);
    }
}

/// A typed, in-process publish/subscribe bus.
///
/// Publishing delivers the event to each attached subscriber exactly once, in subscription
/// order. Publishing with no subscribers attached is a clean no-op (it never errors or blocks).
/// The bus backs in-process subscribers today and is the foundation for the server's SSE stream
/// in a later milestone.
#[derive(Default)]
pub struct EventBus {
    subscribers: Mutex<Vec<Arc<dyn Subscriber>>>,
}

impl EventBus {
    /// Create an empty bus with no subscribers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a subscriber. It observes every event published after this call.
    pub fn subscribe(&self, subscriber: Arc<dyn Subscriber>) {
        self.lock().push(subscriber);
    }

    /// Publish an event to every attached subscriber, exactly once each.
    ///
    /// The subscriber list is snapshotted under the lock and the lock is released before any
    /// subscriber runs, so a subscriber may itself publish or subscribe without deadlocking, and
    /// a single publish is never delivered twice to the same subscriber.
    pub fn publish(&self, event: &Event) {
        let subscribers: Vec<Arc<dyn Subscriber>> = self.lock().clone();
        for subscriber in &subscribers {
            subscriber.on_event(event);
        }
    }

    /// The number of currently attached subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<dyn Subscriber>>> {
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, LifecyclePhase, Surface};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn sample() -> Event {
        Event::lifecycle(LifecyclePhase::Startup, Surface::Run, "0.1.0")
    }

    #[derive(Default)]
    struct Collector {
        events: Mutex<Vec<Event>>,
    }

    impl Collector {
        fn received(&self) -> Vec<Event> {
            self.events.lock().expect("collector lock").clone()
        }
    }

    impl Subscriber for Collector {
        fn on_event(&self, event: &Event) {
            self.events
                .lock()
                .expect("collector lock")
                .push(event.clone());
        }
    }

    #[test]
    fn publish_without_subscribers_is_a_clean_noop() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);
        // Must neither panic nor block.
        bus.publish(&sample());
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn single_subscriber_receives_typed_event() {
        let bus = EventBus::new();
        let collector = Arc::new(Collector::default());
        bus.subscribe(collector.clone());

        bus.publish(&sample());

        let received = collector.received();
        assert_eq!(received.len(), 1);
        match &received[0] {
            Event::Lifecycle(lifecycle) => {
                assert_eq!(lifecycle.phase, LifecyclePhase::Startup);
                assert_eq!(lifecycle.surface, Surface::Run);
                assert_eq!(lifecycle.version, "0.1.0");
            }
        }
    }

    #[test]
    fn each_of_multiple_subscribers_receives_exactly_once() {
        let bus = EventBus::new();
        let first = Arc::new(Collector::default());
        let second = Arc::new(Collector::default());
        bus.subscribe(first.clone());
        bus.subscribe(second.clone());

        bus.publish(&sample());

        assert_eq!(first.received().len(), 1, "no loss/dup to subscriber A");
        assert_eq!(second.received().len(), 1, "no loss/dup to subscriber B");
        assert_eq!(
            first.received(),
            second.received(),
            "both saw the same event"
        );
    }

    #[test]
    fn closures_can_subscribe_and_see_every_publish() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        bus.subscribe(Arc::new(move |_event: &Event| {
            seen.fetch_add(1, Ordering::SeqCst);
        }));

        bus.publish(&sample());
        bus.publish(&sample());

        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "each publish delivered once"
        );
    }
}
