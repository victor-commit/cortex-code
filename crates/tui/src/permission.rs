//! Interactive permission prompts for a TUI turn.
//!
//! A real turn executes tools through the same permission engine the headless `cortex run` uses,
//! but the TUI answers an `ask` interactively instead of failing closed. The turn runs on a worker
//! thread (it owns its own Tokio runtime), while the terminal is owned by the UI thread, so the
//! [`Responder`] cannot draw the prompt itself. Instead it forwards each ask to the UI thread as a
//! [`TurnMessage::Permission`] carrying a reply channel, then blocks until the UI sends a
//! [`Decision`]. Tool dispatch in the runner is sequential, so blocking the worker here only pauses
//! that one turn until the user answers — no stream is stalled and no other tool is in flight.
//!
//! Fail-closed by construction: if the UI is gone (channel send fails) or the reply channel is
//! dropped without an answer, the ask resolves to [`Decision::Deny`], so a tool never proceeds
//! without an explicit approval.
//!
//! The UI answers with a [`ScopedDecision`], so a user can pick "allow once" (apply to this action
//! only) or "allow always" (remember it). An "always" reply is recorded by the engine into the
//! shared saved-decisions store, which the turn persists to disk so a later run — including a later
//! headless `cortex run` on the same project — honors it without re-prompting (VAL-CROSS-028).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};

use permissions::{Decision, PermissionRequest, Responder, ScopedDecision};

use crate::turn::TurnMessage;

/// An interactive permission ask surfaced to the UI thread.
///
/// Carries the gated tool and its subject (a file path or a parsed command) for display, plus the
/// one-shot reply channel the UI answers on. The worker thread blocks on the matching receiver until
/// [`answer`](PermissionAsk::answer) is called (or the ask is dropped, which fails closed).
#[derive(Debug)]
pub struct PermissionAsk {
    /// The gated tool awaiting approval (e.g. `write`, `bash`).
    pub tool: String,
    /// The subject the prompt names (the file path for a file tool, the command for `bash`).
    pub subject: String,
    /// The one-shot channel the UI sends its [`ScopedDecision`] back on.
    reply: Sender<ScopedDecision>,
}

impl PermissionAsk {
    /// Build an ask for `tool`/`subject` answered over `reply`.
    #[must_use]
    pub(crate) fn new(
        tool: impl Into<String>,
        subject: impl Into<String>,
        reply: Sender<ScopedDecision>,
    ) -> Self {
        Self {
            tool: tool.into(),
            subject: subject.into(),
            reply,
        }
    }

    /// Answer the ask, unblocking the worker thread waiting on the reply channel. The decision's
    /// scope ([`once`](ScopedDecision::once) vs [`always`](ScopedDecision::always)) controls whether
    /// the engine records and the turn persists it. A send failure (the worker already gave up and
    /// failed closed) is ignored.
    pub fn answer(self, decision: ScopedDecision) {
        let _ = self.reply.send(decision);
    }
}

/// A [`Responder`] that surfaces each `ask` to the UI thread and blocks on the human's reply.
///
/// Identical `(tool, subject)` asks already answered this turn are reused from a small cache without
/// re-prompting, so a model that retries the same call does not re-open the dialog (and a denied
/// retry stays denied without nagging the user).
pub struct UiResponder {
    events: Sender<TurnMessage>,
    answered: Mutex<HashMap<AskKey, ScopedDecision>>,
    pending: Mutex<PendingAskMap>,
}

type AskKey = (String, String);
type PendingAskMap = HashMap<AskKey, Vec<Sender<ScopedDecision>>>;

impl UiResponder {
    /// Build a responder that forwards asks over `events` (the turn worker's channel to the UI).
    #[must_use]
    pub fn new(events: Sender<TurnMessage>) -> Self {
        Self {
            events,
            answered: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// A decision already given for `key` this turn, if any.
    fn cached(&self, key: &(String, String)) -> Option<ScopedDecision> {
        self.answered
            .lock()
            .ok()
            .and_then(|cache| cache.get(key).cloned())
    }

    /// Remember `decision` for `key` so an identical later ask this turn reuses it.
    fn remember(&self, key: (String, String), decision: ScopedDecision) {
        if let Ok(mut cache) = self.answered.lock() {
            cache.insert(key, decision);
        }
    }

    /// Register this request as pending. Returns `None` for the leader ask that should be
    /// forwarded to the UI, or a receiver for a correlated follower that should wait for the
    /// leader's answer.
    fn register_pending(
        &self,
        key: &(String, String),
    ) -> Option<std::sync::mpsc::Receiver<ScopedDecision>> {
        let mut pending = self.pending.lock().ok()?;
        if let Some(waiters) = pending.get_mut(key) {
            let (tx, rx) = mpsc::channel();
            waiters.push(tx);
            Some(rx)
        } else {
            pending.insert(key.clone(), Vec::new());
            None
        }
    }

    /// Resolve a pending request and fan the answer out to correlated followers.
    fn resolve_pending(&self, key: (String, String), decision: ScopedDecision) {
        let waiters = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&key));
        if let Some(waiters) = waiters {
            for waiter in waiters {
                let _ = waiter.send(decision.clone());
            }
        }
    }
}

impl Responder for UiResponder {
    fn respond(&self, request: &PermissionRequest) -> Decision {
        self.respond_scoped(request).decision
    }

    fn respond_scoped(&self, request: &PermissionRequest) -> ScopedDecision {
        let key = (request.tool.clone(), request.subject.clone());
        if let Some(decision) = self.cached(&key) {
            return decision;
        }

        if let Some(wait) = self.register_pending(&key) {
            return wait
                .recv()
                .unwrap_or_else(|_| ScopedDecision::once(Decision::Deny));
        }

        let (reply, wait) = mpsc::channel();
        let ask = PermissionAsk::new(request.tool.clone(), request.subject.clone(), reply);
        // The UI is gone (closed during the turn): fail closed rather than block forever.
        if self
            .events
            .send(TurnMessage::Permission(Box::new(ask)))
            .is_err()
        {
            self.resolve_pending(key.clone(), ScopedDecision::once(Decision::Deny));
            return ScopedDecision::once(Decision::Deny);
        }

        // Block until the UI answers; a dropped reply channel (UI vanished) fails closed.
        let decision = wait
            .recv()
            .unwrap_or_else(|_| ScopedDecision::once(Decision::Deny));
        self.remember(key.clone(), decision.clone());
        self.resolve_pending(key, decision.clone());
        decision
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc::channel;
    use std::thread;

    use super::*;

    fn request(tool: &str, subject: &str) -> PermissionRequest {
        PermissionRequest::new(tool, subject)
    }

    // An ask is forwarded to the UI as a `Permission` message; once the UI answers, the responder
    // returns that decision.
    #[test]
    fn forwards_ask_and_returns_the_uis_decision() {
        let (tx, rx) = channel();
        let responder = Arc::new(UiResponder::new(tx));

        let worker = {
            let responder = responder.clone();
            thread::spawn(move || responder.respond_scoped(&request("write", "marker.txt")))
        };

        let message = rx.recv().expect("the ask is forwarded to the UI");
        let TurnMessage::Permission(ask) = message else {
            panic!("expected a permission ask message");
        };
        assert_eq!(ask.tool, "write");
        assert_eq!(ask.subject, "marker.txt");
        ask.answer(ScopedDecision::once(Decision::Allow));

        assert_eq!(
            worker.join().unwrap(),
            ScopedDecision::once(Decision::Allow)
        );
    }

    // An "always allow" reply carries the Always scope back so the engine records and persists it.
    #[test]
    fn relays_an_always_allow() {
        let (tx, rx) = channel();
        let responder = Arc::new(UiResponder::new(tx));
        let worker = {
            let responder = responder.clone();
            thread::spawn(move || responder.respond_scoped(&request("bash", "echo hi")))
        };
        let TurnMessage::Permission(ask) = rx.recv().unwrap() else {
            panic!("expected a permission ask");
        };
        ask.answer(ScopedDecision::always(Decision::Allow));
        assert_eq!(
            worker.join().unwrap(),
            ScopedDecision::always(Decision::Allow)
        );
    }

    // A denial flows back unchanged, blocking the tool.
    #[test]
    fn relays_a_denial() {
        let (tx, rx) = channel();
        let responder = Arc::new(UiResponder::new(tx));
        let worker = {
            let responder = responder.clone();
            thread::spawn(move || responder.respond(&request("bash", "rm -rf /")))
        };
        let TurnMessage::Permission(ask) = rx.recv().unwrap() else {
            panic!("expected a permission ask");
        };
        ask.answer(ScopedDecision::once(Decision::Deny));
        assert_eq!(worker.join().unwrap(), Decision::Deny);
    }

    // With the UI channel already closed, an ask fails closed (deny) instead of blocking forever.
    #[test]
    fn fails_closed_when_the_ui_is_gone() {
        let (tx, rx) = channel();
        drop(rx);
        let responder = UiResponder::new(tx);
        assert_eq!(
            responder.respond(&request("write", "x.txt")),
            Decision::Deny
        );
    }

    // An identical ask already answered this turn is reused without forwarding a second prompt.
    #[test]
    fn caches_an_identical_ask_within_the_turn() {
        let (tx, rx) = channel();
        let responder = Arc::new(UiResponder::new(tx));

        let first = {
            let responder = responder.clone();
            thread::spawn(move || responder.respond(&request("write", "a.txt")))
        };
        let TurnMessage::Permission(ask) = rx.recv().unwrap() else {
            panic!("expected a permission ask");
        };
        ask.answer(ScopedDecision::once(Decision::Allow));
        assert_eq!(first.join().unwrap(), Decision::Allow);

        // The second identical ask resolves from the cache: no new message is forwarded.
        assert_eq!(
            responder.respond(&request("write", "a.txt")),
            Decision::Allow
        );
        assert!(
            rx.try_recv().is_err(),
            "a cached ask must not forward a second prompt"
        );
    }

    // VAL-CORTEX-CHAT-003: deny is the fail-closed gate — a drop or explicit Deny both yield
    // Decision::Deny so a mutating tool cannot proceed without an explicit Allow.
    #[test]
    fn permission_gate_deny_blocks_effect_and_allow_unblocks() {
        let (tx, rx) = channel();
        let responder = Arc::new(UiResponder::new(tx));

        // Explicit deny path.
        let denied = {
            let responder = responder.clone();
            thread::spawn(move || responder.respond(&request("write", "secret.txt")))
        };
        let TurnMessage::Permission(ask) = rx.recv().expect("ask forwarded") else {
            panic!("expected permission ask");
        };
        assert_eq!(ask.tool, "write");
        assert_eq!(ask.subject, "secret.txt");
        ask.answer(ScopedDecision::once(Decision::Deny));
        assert_eq!(
            denied.join().unwrap(),
            Decision::Deny,
            "deny must block the gated tool"
        );

        // Explicit allow path (different subject so the in-turn cache does not short-circuit).
        let allowed = {
            let responder = responder.clone();
            thread::spawn(move || responder.respond(&request("write", "ok.txt")))
        };
        let TurnMessage::Permission(ask) = rx.recv().expect("second ask") else {
            panic!("expected permission ask");
        };
        ask.answer(ScopedDecision::once(Decision::Allow));
        assert_eq!(
            allowed.join().unwrap(),
            Decision::Allow,
            "allow must unblock the gated tool"
        );
    }

    // Concurrent identical asks are correlated: one UI prompt is shown and the answered decision
    // (including deny feedback) fans out to all waiting callers.
    #[test]
    fn concurrent_identical_asks_share_one_prompt_and_feedback() {
        let (tx, rx) = channel();
        let responder = Arc::new(UiResponder::new(tx));
        let gate = Arc::new(std::sync::Barrier::new(3));

        let worker = |responder: Arc<UiResponder>, gate: Arc<std::sync::Barrier>| {
            thread::spawn(move || {
                gate.wait();
                responder.respond_scoped(&request("bash", "rm target.txt"))
            })
        };
        let first = worker(responder.clone(), gate.clone());
        let second = worker(responder.clone(), gate.clone());
        gate.wait();

        let TurnMessage::Permission(ask) = rx.recv().expect("single forwarded ask") else {
            panic!("expected permission ask");
        };
        ask.answer(ScopedDecision::once(Decision::Deny).with_feedback("Use ls first."));

        let decision_one = first.join().expect("first decision");
        let decision_two = second.join().expect("second decision");
        let expected = ScopedDecision::once(Decision::Deny).with_feedback("Use ls first.");
        assert_eq!(decision_one, expected);
        assert_eq!(decision_two, expected);
        assert!(
            rx.try_recv().is_err(),
            "only one prompt should be forwarded for correlated asks"
        );
    }
}
