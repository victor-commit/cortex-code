//! Integration coverage for the permission engine's resolution flow and its observable events.
//!
//! Mirrors the M2 permission contract (VAL-PERM-001..005, 008): default-ask emits an observable
//! `permission.asked` event, `allow` runs with no ask event, `deny` blocks, an `ask` is withheld
//! pending a decision, headless (no responder) fails closed, and last-matching-rule-wins.

use std::sync::{Arc, Mutex};

use events::{Event, EventBus, PermissionDecision, Subscriber};
use permissions::{
    Action, Decision, Outcome, PermissionEngine, PermissionRequest, Responder, Rule, Ruleset,
    SavedDecision, SavedDecisions,
};

/// Collects every event published on the bus, so a test can assert on what was surfaced.
#[derive(Default)]
struct Collector {
    events: Mutex<Vec<Event>>,
}

impl Collector {
    fn events(&self) -> Vec<Event> {
        self.events.lock().expect("collector lock").clone()
    }

    fn asked(&self) -> Vec<Event> {
        self.events()
            .into_iter()
            .filter(|event| event.kind() == "permission.asked")
            .collect()
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

/// A responder returning a fixed decision and recording the requests it was asked about.
struct RecordingResponder {
    decision: Decision,
    seen: Mutex<Vec<PermissionRequest>>,
}

impl RecordingResponder {
    fn new(decision: Decision) -> Self {
        Self {
            decision,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<PermissionRequest> {
        self.seen.lock().expect("responder lock").clone()
    }
}

impl Responder for RecordingResponder {
    fn respond(&self, request: &PermissionRequest) -> Decision {
        self.seen
            .lock()
            .expect("responder lock")
            .push(request.clone());
        self.decision
    }
}

fn bus_with_collector() -> (Arc<EventBus>, Arc<Collector>) {
    let bus = Arc::new(EventBus::new());
    let collector = Arc::new(Collector::default());
    bus.subscribe(collector.clone());
    (bus, collector)
}

#[test]
fn default_ask_emits_an_observable_permission_asked_event() {
    // VAL-PERM-001: with an empty permission config, a gated bash action is classified ask and
    // surfaces a permission.asked event instead of running.
    let (bus, collector) = bus_with_collector();
    let engine = PermissionEngine::new(Ruleset::empty()).with_bus(bus);

    let outcome = engine.evaluate(&PermissionRequest::new("bash", "echo hi"));

    assert_eq!(outcome, Outcome::Denied, "headless ask fails closed");
    let asked = collector.asked();
    assert_eq!(asked.len(), 1, "exactly one ask was surfaced");
    match &asked[0] {
        Event::PermissionAsked(ask) => {
            assert_eq!(ask.tool, "bash", "event names the tool");
            assert_eq!(ask.request, "echo hi", "event names the request subject");
        }
        other => panic!("expected a permission.asked event, got {other:?}"),
    }
}

#[test]
fn allow_rule_runs_with_no_ask_event() {
    // VAL-PERM-002: an allow-covered action is allowed and emits no permission.asked event.
    let (bus, collector) = bus_with_collector();
    let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
        "bash",
        "echo *",
        Action::Allow,
    )]))
    .with_bus(bus);

    let outcome = engine.evaluate(&PermissionRequest::new("bash", "echo hi"));

    assert_eq!(outcome, Outcome::Allowed);
    assert!(
        collector.asked().is_empty(),
        "an allow rule must not surface an ask"
    );
}

#[test]
fn deny_rule_blocks_with_no_ask_event() {
    // VAL-PERM-003: a deny-covered action is denied (the effect is the caller's to withhold).
    let (bus, collector) = bus_with_collector();
    let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "rm *", Action::Deny)]))
        .with_bus(bus);

    let outcome = engine.evaluate(&PermissionRequest::new("bash", "rm target.txt"));

    assert_eq!(outcome, Outcome::Denied);
    assert!(
        collector.asked().is_empty(),
        "a deny is a policy block, not an ask"
    );
}

#[test]
fn ask_is_withheld_until_a_decision_and_names_tool_and_pattern() {
    // VAL-PERM-004: an ask-classified write is surfaced (naming the tool + pattern + path) and is
    // only allowed once the responder approves.
    let (bus, collector) = bus_with_collector();
    let responder = Arc::new(RecordingResponder::new(Decision::Allow));
    let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
        "write",
        "src/**",
        Action::Ask,
    )]))
    .with_responder(responder.clone())
    .with_bus(bus);

    let outcome = engine.evaluate(&PermissionRequest::new("write", "src/new.txt"));

    assert_eq!(outcome, Outcome::Allowed, "approved ask proceeds");
    assert_eq!(responder.seen().len(), 1, "the responder was consulted");

    let asked = collector.asked();
    assert_eq!(asked.len(), 1);
    match &asked[0] {
        Event::PermissionAsked(ask) => {
            assert_eq!(ask.tool, "write");
            assert_eq!(ask.pattern, "src/**", "event names the matching pattern");
            assert_eq!(ask.request, "src/new.txt", "event names the target path");
        }
        other => panic!("expected a permission.asked event, got {other:?}"),
    }
}

#[test]
fn headless_ask_fails_closed_and_is_still_observable() {
    // VAL-PERM-005: with no responder, an ask resolves to Denied in bounded time (no hang) and
    // both the asked and the auto-deny replied events are observable.
    let (bus, collector) = bus_with_collector();
    let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new("write", "**", Action::Ask)]))
        .with_bus(bus);

    let outcome = engine.evaluate(&PermissionRequest::new("write", "out.txt"));

    assert_eq!(outcome, Outcome::Denied, "headless ask fails closed");
    let replied: Vec<_> = collector
        .events()
        .into_iter()
        .filter_map(|event| match event {
            Event::PermissionReplied(reply) => Some(reply.decision),
            _ => None,
        })
        .collect();
    assert_eq!(
        replied,
        vec![PermissionDecision::Deny],
        "the fail-closed deny is recorded"
    );
}

#[test]
fn last_matching_rule_wins_and_reordering_flips_the_decision() {
    // VAL-PERM-008: with conflicting rules, the last matching rule governs; swapping the order
    // flips the outcome.
    let allow_then_deny = PermissionEngine::new(Ruleset::new(vec![
        Rule::new("bash", "*", Action::Allow),
        Rule::new("bash", "rm *", Action::Deny),
    ]));
    assert_eq!(
        allow_then_deny.evaluate(&PermissionRequest::new("bash", "rm x")),
        Outcome::Denied,
        "deny matches last → blocked"
    );

    let deny_then_allow = PermissionEngine::new(Ruleset::new(vec![
        Rule::new("bash", "rm *", Action::Deny),
        Rule::new("bash", "*", Action::Allow),
    ]));
    assert_eq!(
        deny_then_allow.evaluate(&PermissionRequest::new("bash", "rm x")),
        Outcome::Allowed,
        "broad allow matches last → permitted"
    );
}

#[test]
fn saved_decision_outranks_the_ordinary_ruleset() {
    let saved = Arc::new(SavedDecisions::from_decisions(vec![SavedDecision::new(
        "bash",
        "*",
        Decision::Allow,
    )]));
    let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "*", Action::Deny)]))
        .with_saved_decisions(saved);

    assert_eq!(engine.classify("bash", "echo hi"), Action::Allow);
    assert_eq!(
        engine.evaluate(&PermissionRequest::new("bash", "echo hi")),
        Outcome::Allowed
    );
}

#[test]
fn hard_deny_outranks_saved_decisions_and_ruleset() {
    let saved = Arc::new(SavedDecisions::from_decisions(vec![SavedDecision::new(
        "bash",
        "*",
        Decision::Allow,
    )]));
    let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "*", Action::Allow)]))
        .with_saved_decisions(saved)
        .with_hard_denies(Ruleset::new(vec![Rule::new("bash", "*", Action::Deny)]));

    assert_eq!(engine.classify("bash", "echo hi"), Action::Deny);
    assert_eq!(
        engine.evaluate(&PermissionRequest::new("bash", "echo hi")),
        Outcome::Denied
    );
}
