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

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};

use permissions::{Decision, PermissionRequest, Responder};

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
    /// The one-shot channel the UI sends its [`Decision`] back on.
    reply: Sender<Decision>,
}

impl PermissionAsk {
    /// Build an ask for `tool`/`subject` answered over `reply`.
    #[must_use]
    pub(crate) fn new(
        tool: impl Into<String>,
        subject: impl Into<String>,
        reply: Sender<Decision>,
    ) -> Self {
        Self {
            tool: tool.into(),
            subject: subject.into(),
            reply,
        }
    }

    /// Answer the ask, unblocking the worker thread waiting on the reply channel. A send failure
    /// (the worker already gave up and failed closed) is ignored.
    pub fn answer(self, decision: Decision) {
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
    answered: Mutex<HashMap<(String, String), Decision>>,
}

impl UiResponder {
    /// Build a responder that forwards asks over `events` (the turn worker's channel to the UI).
    #[must_use]
    pub fn new(events: Sender<TurnMessage>) -> Self {
        Self {
            events,
            answered: Mutex::new(HashMap::new()),
        }
    }

    /// A decision already given for `key` this turn, if any.
    fn cached(&self, key: &(String, String)) -> Option<Decision> {
        self.answered
            .lock()
            .ok()
            .and_then(|cache| cache.get(key).copied())
    }

    /// Remember `decision` for `key` so an identical later ask this turn reuses it.
    fn remember(&self, key: (String, String), decision: Decision) {
        if let Ok(mut cache) = self.answered.lock() {
            cache.insert(key, decision);
        }
    }
}

impl Responder for UiResponder {
    fn respond(&self, request: &PermissionRequest) -> Decision {
        let key = (request.tool.clone(), request.subject.clone());
        if let Some(decision) = self.cached(&key) {
            return decision;
        }

        let (reply, wait) = mpsc::channel();
        let ask = PermissionAsk::new(request.tool.clone(), request.subject.clone(), reply);
        // The UI is gone (closed during the turn): fail closed rather than block forever.
        if self
            .events
            .send(TurnMessage::Permission(Box::new(ask)))
            .is_err()
        {
            return Decision::Deny;
        }

        // Block until the UI answers; a dropped reply channel (UI vanished) fails closed.
        let decision = wait.recv().unwrap_or(Decision::Deny);
        self.remember(key, decision);
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
            thread::spawn(move || responder.respond(&request("write", "marker.txt")))
        };

        let message = rx.recv().expect("the ask is forwarded to the UI");
        let TurnMessage::Permission(ask) = message else {
            panic!("expected a permission ask message");
        };
        assert_eq!(ask.tool, "write");
        assert_eq!(ask.subject, "marker.txt");
        ask.answer(Decision::Allow);

        assert_eq!(worker.join().unwrap(), Decision::Allow);
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
        ask.answer(Decision::Deny);
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
        ask.answer(Decision::Allow);
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
}
