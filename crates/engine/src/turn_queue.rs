//! Session-scoped turn admission and cooperative abort signaling shared by runtime surfaces.
//!
//! The engine runner itself is a per-turn executor. Surfaces that need "one active turn per
//! session" semantics (and mid-turn prompt queueing) use this queue to serialize turn admission
//! while still supporting cooperative cancellation of the active turn.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, watch};

/// Session-scoped turn admission queue with cooperative abort signaling.
///
/// At most one active turn per session is admitted at a time. Additional turn requests for that
/// session wait in-memory until the active turn releases its lease.
#[derive(Clone, Default)]
pub struct TurnQueue {
    active: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    waiters: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
}

impl TurnQueue {
    /// Acquire the active-turn lease for `session_id`.
    ///
    /// If the session already has an active turn, this waits until that turn completes and returns
    /// a lease marked as queued.
    pub async fn acquire(&self, session_id: impl Into<String>) -> Result<TurnLease, String> {
        self.acquire_inner(session_id.into(), || {}).await
    }

    /// Request cancellation for the active turn in `session_id`.
    ///
    /// Returns `true` if an active turn existed and received the abort signal.
    pub fn abort(&self, session_id: &str) -> Result<bool, String> {
        let sender = self
            .active
            .lock()
            .map_err(|_| "active turn registry is unavailable".to_owned())?
            .get(session_id)
            .cloned();
        let Some(sender) = sender else {
            return Ok(false);
        };
        let _ = sender.send(true);
        Ok(true)
    }

    /// Whether `session_id` currently has an active turn.
    pub fn is_active(&self, session_id: &str) -> Result<bool, String> {
        Ok(self
            .active
            .lock()
            .map_err(|_| "active turn registry is unavailable".to_owned())?
            .contains_key(session_id))
    }

    fn try_acquire(&self, session_id: &str, queued: bool) -> Result<Option<TurnLease>, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "active turn registry is unavailable".to_owned())?;
        if active.contains_key(session_id) {
            return Ok(None);
        }
        let (abort_tx, abort_rx) = watch::channel(false);
        active.insert(session_id.to_owned(), abort_tx);
        Ok(Some(TurnLease {
            session_id: session_id.to_owned(),
            queue: self.clone(),
            abort_rx,
            queued,
        }))
    }

    fn waiter_for(&self, session_id: &str) -> Result<Arc<Notify>, String> {
        let mut waiters = self
            .waiters
            .lock()
            .map_err(|_| "active turn queue is unavailable".to_owned())?;
        Ok(waiters
            .entry(session_id.to_owned())
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone())
    }

    fn release(&self, session_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(session_id);
        }
        if let Ok(mut waiters) = self.waiters.lock() {
            let remove_entry = if let Some(waiter) = waiters.get(session_id) {
                waiter.notify_waiters();
                Arc::strong_count(waiter) == 1
            } else {
                false
            };
            if remove_entry {
                waiters.remove(session_id);
            }
        }
    }

    async fn acquire_inner<F>(
        &self,
        session_id: String,
        mut on_wait: F,
    ) -> Result<TurnLease, String>
    where
        F: FnMut(),
    {
        let mut queued = false;
        loop {
            let waiter = self.waiter_for(&session_id)?;
            let notified = waiter.notified();
            if let Some(lease) = self.try_acquire(&session_id, queued)? {
                return Ok(lease);
            }
            queued = true;
            on_wait();
            notified.await;
        }
    }
}

/// The active-turn lease for one session.
///
/// Dropping this lease releases the session so the next queued prompt can start.
pub struct TurnLease {
    session_id: String,
    queue: TurnQueue,
    abort_rx: watch::Receiver<bool>,
    queued: bool,
}

impl TurnLease {
    /// Whether this turn request waited behind another in-flight turn.
    #[must_use]
    pub fn queued(&self) -> bool {
        self.queued
    }

    /// The cooperative abort receiver for this active turn.
    #[must_use]
    pub fn abort_receiver(&self) -> watch::Receiver<bool> {
        self.abort_rx.clone()
    }
}

impl Drop for TurnLease {
    fn drop(&mut self) {
        self.queue.release(&self.session_id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use super::TurnQueue;

    #[tokio::test]
    async fn second_turn_waits_and_reports_queued_state() {
        let queue = TurnQueue::default();
        let first = queue.acquire("ses").await.expect("first lease");
        let queue_for_second = queue.clone();
        let mut second = tokio::spawn(async move { queue_for_second.acquire("ses").await });

        let waiting = tokio::time::timeout(Duration::from_millis(40), &mut second);
        assert!(
            waiting.await.is_err(),
            "second acquire should wait while first lease is active"
        );

        drop(first);
        let second = second
            .await
            .expect("join second acquire")
            .expect("queued second lease");
        assert!(second.queued(), "waiting acquire should report queued");
        drop(second);

        let second = queue.acquire("ses").await.expect("fresh second lease");
        assert!(
            !second.queued(),
            "a fresh acquire after release should not report queued"
        );
    }

    #[tokio::test]
    async fn waiting_acquire_becomes_queued_after_release() {
        let queue = TurnQueue::default();
        let first = queue.acquire("ses").await.expect("first lease");
        let queue_for_second = queue.clone();
        let second = tokio::spawn(async move { queue_for_second.acquire("ses").await });

        tokio::time::sleep(Duration::from_millis(40)).await;
        drop(first);

        let second = second
            .await
            .expect("join second acquire")
            .expect("queued lease");
        assert!(second.queued(), "waiting acquire should report queued");
    }

    #[tokio::test]
    async fn acquire_does_not_miss_release_between_check_and_wait() {
        let queue = TurnQueue::default();
        let first = queue.acquire("ses").await.expect("first lease");
        let first_lease = Arc::new(StdMutex::new(Some(first)));

        let queue_for_second = queue.clone();
        let first_for_hook = Arc::clone(&first_lease);
        let second = tokio::spawn(async move {
            queue_for_second
                .acquire_inner("ses".to_owned(), || {
                    if let Some(lease) = first_for_hook.lock().expect("lock first lease").take() {
                        drop(lease);
                    }
                })
                .await
        });

        let second = tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .expect("second acquire should finish after release")
            .expect("join second acquire")
            .expect("queued lease");
        assert!(second.queued(), "waiting acquire should report queued");
    }

    #[tokio::test]
    async fn abort_signals_only_active_turn() {
        let queue = TurnQueue::default();
        let lease = queue.acquire("ses").await.expect("lease");
        let mut abort_rx = lease.abort_receiver();
        assert!(queue.abort("ses").expect("abort result"));
        abort_rx.changed().await.expect("abort signal");
        assert!(*abort_rx.borrow(), "active lease should observe abort");
    }

    #[tokio::test]
    async fn uncontended_releases_do_not_leave_stale_waiters() {
        let queue = TurnQueue::default();
        for index in 0..64 {
            let lease = queue
                .acquire(format!("session-{index}"))
                .await
                .expect("lease");
            drop(lease);
        }
        let waiters = queue.waiters.lock().expect("lock waiters");
        assert!(
            waiters.is_empty(),
            "waiter map should be empty after uncontended release cycles"
        );
    }

    #[tokio::test]
    async fn contended_release_eventually_cleans_waiter_entry() {
        let queue = TurnQueue::default();
        let first = queue.acquire("ses").await.expect("first lease");
        let queue_for_second = queue.clone();
        let second = tokio::spawn(async move { queue_for_second.acquire("ses").await });

        tokio::time::sleep(Duration::from_millis(40)).await;
        drop(first);
        let second = second
            .await
            .expect("join second acquire")
            .expect("queued second lease");
        drop(second);

        let waiters = queue.waiters.lock().expect("lock waiters");
        assert!(
            !waiters.contains_key("ses"),
            "waiter entry should be removed once no waiter still needs it"
        );
    }
}
