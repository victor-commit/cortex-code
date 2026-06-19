//! Integration coverage for saved permission decisions (VAL-PERM-011).
//!
//! After an `ask` is answered with "always", the choice is recorded in a shared
//! [`SavedDecisions`] store; a later matching action in the same session reuses it and emits **no**
//! new `permission.asked` event. Asserted through the real [`PermissionEngine`] with an event
//! collector on the `events` bus and a responder that answers allow-and-save.

use std::sync::{Arc, Mutex};

use events::{Event, EventBus, Subscriber};
use permissions::{
    Action, Decision, Outcome, PermissionEngine, PermissionRequest, Responder, Rule, Ruleset,
    SavedDecision, SavedDecisions, ScopedDecision, request_for,
};

/// Collects every event published on the bus so a test can count `permission.asked` events.
#[derive(Default)]
struct Collector {
    events: Mutex<Vec<Event>>,
}

impl Collector {
    fn asked_count(&self) -> usize {
        self.events
            .lock()
            .expect("collector lock")
            .iter()
            .filter(|event| event.kind() == "permission.asked")
            .count()
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

/// A responder that allows and saves ("always allow this pattern").
struct AllowAndSave;

impl Responder for AllowAndSave {
    fn respond(&self, _request: &PermissionRequest) -> Decision {
        Decision::Allow
    }

    fn respond_scoped(&self, _request: &PermissionRequest) -> ScopedDecision {
        ScopedDecision::always(Decision::Allow)
    }
}

/// VAL-PERM-011: after an allow-and-save for a pattern, a subsequent matching action runs honoring
/// the saved decision and emits no new `permission.asked` event.
#[test]
fn saved_allow_is_reused_without_re_asking() {
    let bus = Arc::new(EventBus::new());
    let collector = Arc::new(Collector::default());
    bus.subscribe(collector.clone());

    // The store is shared across the session's turns (one store, fresh engine per action).
    let saved = Arc::new(SavedDecisions::new());

    // Config asks before any echo runs.
    let ruleset = || Ruleset::new(vec![Rule::new("bash", "echo *", Action::Ask)]);

    // First action: the ask is surfaced and the responder allows-and-saves.
    let first = PermissionEngine::new(ruleset())
        .with_responder(Arc::new(AllowAndSave))
        .with_saved_decisions(saved.clone())
        .with_bus(bus.clone());
    assert_eq!(
        first.evaluate(&request_for("bash", "echo hi")),
        Outcome::Allowed
    );
    assert_eq!(
        collector.asked_count(),
        1,
        "the first action surfaces one ask"
    );
    assert_eq!(
        saved.resolve("bash", "echo bye"),
        Some(SavedDecision::new("bash", "echo *", Decision::Allow)),
        "the allow was saved for the matching pattern"
    );

    // Second matching action on the same store: honored directly, no new ask. A fresh engine (a
    // later turn) sees the saved decision because the store is shared.
    let second = PermissionEngine::new(ruleset())
        .with_saved_decisions(saved.clone())
        .with_bus(bus.clone());
    assert_eq!(
        second.evaluate(&request_for("bash", "echo there")),
        Outcome::Allowed,
        "the matching action runs via the saved decision"
    );
    assert_eq!(
        collector.asked_count(),
        1,
        "no new permission.asked event was emitted for the saved pattern"
    );
}

/// A saved deny is likewise reused without re-asking (the save mechanism is decision-agnostic).
#[test]
fn saved_deny_is_reused_without_re_asking() {
    let bus = Arc::new(EventBus::new());
    let collector = Arc::new(Collector::default());
    bus.subscribe(collector.clone());

    let saved = Arc::new(SavedDecisions::from_decisions(vec![SavedDecision::new(
        "bash",
        "rm *",
        Decision::Deny,
    )]));

    // With the deny already saved, a matching action is denied with no ask surfaced.
    let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "rm *", Action::Ask)]))
        .with_saved_decisions(saved)
        .with_bus(bus);
    assert_eq!(
        engine.evaluate(&request_for("bash", "rm victim.txt")),
        Outcome::Denied
    );
    assert_eq!(
        collector.asked_count(),
        0,
        "a saved deny short-circuits before the ask"
    );
}

/// A non-matching action is unaffected by a saved decision and still surfaces its own ask.
#[test]
fn saved_decision_does_not_leak_to_non_matching_actions() {
    let bus = Arc::new(EventBus::new());
    let collector = Arc::new(Collector::default());
    bus.subscribe(collector.clone());

    let saved = Arc::new(SavedDecisions::from_decisions(vec![SavedDecision::new(
        "bash",
        "echo *",
        Decision::Allow,
    )]));
    let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "rm *", Action::Ask)]))
        .with_saved_decisions(saved)
        .with_bus(bus);

    // `rm x` does not match the saved `echo *`, so it still asks (and fails closed headless).
    assert_eq!(
        engine.evaluate(&request_for("bash", "rm x")),
        Outcome::Denied
    );
    assert_eq!(
        collector.asked_count(),
        1,
        "a non-matching action still surfaces its own ask"
    );
}
