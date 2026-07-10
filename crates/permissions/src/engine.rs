//! The permission engine: resolve a gated action to allow/deny, surfacing asks observably and
//! failing closed when an ask cannot be answered.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use events::{EventBus, PermissionDecision};
use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::rule::Rule;
use crate::ruleset::Ruleset;
use crate::saved::{SavedDecision, SavedDecisions};

/// A gated tool action awaiting a permission verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequest {
    /// The gated tool (e.g. `bash`, `write`).
    pub tool: String,
    /// The subject the rule patterns are matched against (a file path, a parsed command, ...).
    pub subject: String,
}

impl PermissionRequest {
    /// Build a request from a tool name and its subject.
    #[must_use]
    pub fn new(tool: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            subject: subject.into(),
        }
    }
}

/// The concrete answer a [`Responder`] gives to an `ask`: run the action or block it.
///
/// Serializes to the stable lowercase tokens `allow`/`deny` so a [`SavedDecision`](crate::SavedDecision)
/// round-trips through the durable on-disk saved-decisions store unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// Allow the action to proceed.
    Allow,
    /// Block the action.
    Deny,
}

impl Decision {
    /// The configured [`Action`] this decision is equivalent to (`Allow`/`Deny`).
    #[must_use]
    pub const fn action(self) -> Action {
        match self {
            Decision::Allow => Action::Allow,
            Decision::Deny => Action::Deny,
        }
    }

    /// The [`Outcome`] this decision resolves to.
    #[must_use]
    pub const fn outcome(self) -> Outcome {
        match self {
            Decision::Allow => Outcome::Allowed,
            Decision::Deny => Outcome::Denied,
        }
    }
}

/// Whether a [`Decision`] applies only to the current action or is saved for matching ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionScope {
    /// Apply to this action only; do not save it.
    Once,
    /// Save the decision so later matching actions reuse it without re-asking (VAL-PERM-011).
    Always,
}

/// A responder's answer to an `ask`: the decision plus whether to save it for matching actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedDecision {
    /// Allow or deny the action.
    pub decision: Decision,
    /// Whether the decision is saved for later matching actions.
    pub scope: DecisionScope,
    /// Optional corrective feedback carried with a denial, surfaced back to the running turn.
    pub feedback: Option<String>,
}

impl ScopedDecision {
    /// A one-shot decision applying only to the current action.
    #[must_use]
    pub const fn once(decision: Decision) -> Self {
        Self {
            decision,
            scope: DecisionScope::Once,
            feedback: None,
        }
    }

    /// A saved decision applied to and remembered for later matching actions.
    #[must_use]
    pub const fn always(decision: Decision) -> Self {
        Self {
            decision,
            scope: DecisionScope::Always,
            feedback: None,
        }
    }

    /// Attach corrective feedback text to this decision.
    ///
    /// Empty or whitespace-only feedback is ignored.
    #[must_use]
    pub fn with_feedback(mut self, feedback: impl Into<String>) -> Self {
        self.feedback = normalize_feedback(Some(feedback.into()));
        self
    }
}

/// The final verdict the engine returns for a gated action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The action may proceed and produce its effect.
    Allowed,
    /// The action is blocked; its effect must not occur.
    Denied,
}

impl Outcome {
    /// Whether the action may proceed.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Outcome::Allowed)
    }

    /// The stable lowercase audit label for this final outcome.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Outcome::Allowed => "allowed",
            Outcome::Denied => "denied",
        }
    }
}

/// The configured permission action, matched pattern, and final resolved outcome for one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionEvaluation {
    /// The final constrained action before any responder answer, preserving terminal provenance.
    pub action: Action,
    /// The matched rule/saved/default pattern that classified the request.
    pub pattern: String,
    /// The final verdict after responder and override hooks.
    pub outcome: Outcome,
    /// Optional corrective feedback carried with a denied ask.
    pub feedback: Option<String>,
}

/// The seam through which an `ask` is answered.
///
/// The interactive surfaces (TUI; the server's remote control) implement this to prompt a human.
/// A responder MUST return in bounded time — the engine treats its answer as authoritative and
/// does not impose its own wait. The headless `cortex run` path has no responder, so an `ask`
/// fails closed (see [`PermissionEngine::evaluate`]).
pub trait Responder: Send + Sync {
    /// Answer the ask for `request`, returning whether to allow or deny it.
    fn respond(&self, request: &PermissionRequest) -> Decision;

    /// Answer the ask for `request`, optionally saving the decision for later matching actions.
    ///
    /// The default applies [`respond`](Responder::respond) once (not saved); a surface that lets a
    /// user pick "always allow/deny this pattern" overrides this to return [`DecisionScope::Always`]
    /// so the engine records the choice (VAL-PERM-011).
    fn respond_scoped(&self, request: &PermissionRequest) -> ScopedDecision {
        ScopedDecision::once(self.respond(request))
    }
}

/// A final decision override hook for plugin and policy extensions.
///
/// The core permission engine first resolves the action that the configured rules, saved
/// decisions, hard-deny layer, and parent cap would take. A registered override receives that
/// proposed action and may narrow ordinary decisions or widen an ordinary deny. Hard denies and
/// parent caps remain terminal ceilings, so an override may never make the final result more
/// permissive than either constraint. Returning `None` preserves the engine's proposal. This stays
/// subject-agnostic so the plugin host can live outside the permissions crate.
pub trait DecisionOverride: Send + Sync {
    /// Return a final decision for `request`, or `None` to keep `proposed`.
    fn override_decision(&self, request: &PermissionRequest, proposed: Action) -> Option<Decision>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TerminalConstraint {
    None,
    HardDeny { pattern: String },
    ParentCap { action: Action, pattern: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EffectiveDecision {
    action: Action,
    pattern: String,
    constraint: TerminalConstraint,
}

/// Resolves every gated tool action to [`Outcome::Allowed`] or [`Outcome::Denied`].
///
/// The engine owns the combined [`Ruleset`], an optional [`Responder`] for answering asks, and an
/// optional event bus on which it publishes `permission.asked`/`permission.replied`. It is the one
/// place a gated action is classified (architecture §3.4):
///
/// - `allow` → [`Outcome::Allowed`] immediately, with no event.
/// - `deny` → [`Outcome::Denied`]; the effect never runs.
/// - `ask` → a `permission.asked` event is published naming the tool and pattern, then the
///   responder decides. With **no responder** (headless `cortex run`), the ask **fails closed**:
///   it resolves to [`Outcome::Denied`] in bounded time (no hang, no silent allow; VAL-PERM-005).
pub struct PermissionEngine {
    ruleset: Ruleset,
    hard_denies: Ruleset,
    parent_cap: Option<Ruleset>,
    saved: Option<Arc<SavedDecisions>>,
    responder: Option<Arc<dyn Responder>>,
    override_hook: Option<Arc<dyn DecisionOverride>>,
    bus: Option<Arc<EventBus>>,
    event_context: Option<PermissionEventContext>,
    in_flight_asks: InFlightAsks,
}

#[derive(Clone, Default)]
struct InFlightAsks {
    state: Arc<Mutex<InFlightAskMap>>,
}

type AskKey = (String, String);
type InFlightAskMap = HashMap<AskKey, Arc<InFlightAsk>>;

struct InFlightAsk {
    reply: Mutex<Option<ScopedDecision>>,
    ready: Condvar,
}

impl InFlightAsk {
    fn new() -> Self {
        Self {
            reply: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn wait(&self) -> ScopedDecision {
        let mut guard = self.reply.lock().expect("in-flight ask lock");
        loop {
            if let Some(reply) = guard.clone() {
                return reply;
            }
            guard = self.ready.wait(guard).expect("in-flight ask wait");
        }
    }

    fn resolve(&self, reply: ScopedDecision) {
        if let Ok(mut guard) = self.reply.lock()
            && guard.is_none()
        {
            *guard = Some(reply);
        }
        self.ready.notify_all();
    }
}

#[derive(Clone)]
struct PermissionEventContext {
    session_id: String,
    message_id: String,
}

impl PermissionEngine {
    /// Build a headless engine from a ruleset: it has no responder, so every `ask` fails closed.
    #[must_use]
    pub fn new(ruleset: Ruleset) -> Self {
        Self {
            ruleset,
            hard_denies: Ruleset::empty(),
            parent_cap: None,
            saved: None,
            responder: None,
            override_hook: None,
            bus: None,
            event_context: None,
            in_flight_asks: InFlightAsks::default(),
        }
    }

    /// Attach a highest-precedence hard-deny layer (builder style).
    ///
    /// Matching `deny` rules in this layer are safety/mode constraints, not ordinary permission
    /// rules: they are checked before saved "always allow" decisions, so a remembered approval can
    /// never bypass them. Non-deny rules in this layer are ignored; callers should pass only deny
    /// rules to make the intent explicit.
    #[must_use]
    pub fn with_hard_denies(mut self, hard_denies: Ruleset) -> Self {
        self.hard_denies = hard_denies;
        self
    }

    /// Cap this engine's decisions with a parent agent's ruleset (builder style).
    ///
    /// When set, the effective action is the most restrictive of this engine's own decision and the
    /// parent's, so a spawned subagent can never exceed its parent: an action the parent denies
    /// stays denied here regardless of this engine's ruleset (VAL-PERM-010).
    #[must_use]
    pub fn with_parent_cap(mut self, parent: Ruleset) -> Self {
        self.parent_cap = Some(parent);
        self
    }

    /// Attach a shared store of saved "always" decisions (builder style).
    ///
    /// A saved decision matching the action is honored before the ordinary configured ruleset, so a
    /// matching later action resolves directly and emits no new `permission.asked` event
    /// (VAL-PERM-011). Hard-deny constraints still run first. The store is shared across a
    /// session's turns, and an `Always` reply is recorded into it.
    #[must_use]
    pub fn with_saved_decisions(mut self, saved: Arc<SavedDecisions>) -> Self {
        self.saved = Some(saved);
        self
    }

    /// Attach a responder that answers asks (builder style); without one, asks fail closed.
    #[must_use]
    pub fn with_responder(mut self, responder: Arc<dyn Responder>) -> Self {
        self.responder = Some(responder);
        self
    }

    /// Attach a final decision override hook (builder style).
    ///
    /// The hook receives the engine's proposed action before an `ask` is surfaced. It may force
    /// an ordinary allow or deny, but hard denies and parent caps are terminal ceilings: an
    /// override is constrained before the final outcome, event, and audit evaluation are emitted.
    #[must_use]
    pub fn with_override(mut self, override_hook: Arc<dyn DecisionOverride>) -> Self {
        self.override_hook = Some(override_hook);
        self
    }

    /// Attach the event bus that `permission.asked`/`permission.replied` are published on
    /// (builder style); without one, the engine resolves silently.
    #[must_use]
    pub fn with_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Attach session/turn ids to emitted `permission.asked` and `permission.replied` events.
    ///
    /// Surfaces that stream a concrete turn (for example `cortex serve` SSE) use this so remote
    /// clients can correlate a permission prompt with the originating prompt request. Surfaces that
    /// do not need turn correlation can omit it and retain the legacy uncorrelated event shape.
    #[must_use]
    pub fn with_event_context(
        mut self,
        session_id: impl Into<String>,
        message_id: impl Into<String>,
    ) -> Self {
        self.event_context = Some(PermissionEventContext {
            session_id: session_id.into(),
            message_id: message_id.into(),
        });
        self
    }

    /// The combined ruleset this engine evaluates.
    #[must_use]
    pub fn ruleset(&self) -> &Ruleset {
        &self.ruleset
    }

    /// Snapshot every rule-based constraint that can limit a spawned subagent.
    ///
    /// The snapshot preserves the parent engine's normal rules, saved decisions, and absolute hard
    /// denies in their effective precedence order. A child installs it with
    /// [`with_parent_cap`](Self::with_parent_cap), making the child at least as restrictive as the
    /// parent's actual policy instead of only the parent's ordinary config rules.
    #[must_use]
    pub fn parent_cap_ruleset(&self) -> Ruleset {
        let mut rules = self.ruleset.rules().to_vec();
        if let Some(saved) = &self.saved {
            rules.extend(saved.snapshot().into_iter().map(|decision| {
                Rule::new(decision.tool, decision.pattern, decision.decision.action())
            }));
        }
        rules.extend(self.hard_denies.rules().iter().cloned());
        Ruleset::new(rules)
    }

    /// Whether a responder is attached (so asks can be answered rather than failing closed).
    #[must_use]
    pub fn has_responder(&self) -> bool {
        self.responder.is_some()
    }

    /// The action this engine would take for a gated action, without surfacing or resolving an ask.
    ///
    /// Honors saved "always" decisions and the parent cap, so it reflects the same effective
    /// decision `evaluate` would reach (minus surfacing/answering the ask).
    #[must_use]
    pub fn classify(&self, tool: &str, subject: &str) -> Action {
        self.effective(tool, subject).action
    }

    /// Resolve a gated action to a final [`Outcome`], surfacing and answering an `ask` as needed.
    pub fn evaluate(&self, request: &PermissionRequest) -> Outcome {
        self.evaluate_detail(request).outcome
    }

    /// Resolve a gated action and return its full audit-relevant decision details.
    pub fn evaluate_detail(&self, request: &PermissionRequest) -> PermissionEvaluation {
        let effective = self.effective(&request.tool, &request.subject);
        let mut action = effective.action;
        let mut pattern = effective.pattern.clone();
        let mut final_action = action;
        let mut override_applied = false;
        if let Some(override_hook) = &self.override_hook
            && let Some(decision) = override_hook.override_decision(request, action)
        {
            override_applied = true;
            let overridden_action = decision.action();
            match &effective.constraint {
                TerminalConstraint::HardDeny {
                    pattern: constraint_pattern,
                } if overridden_action != Action::Deny => {
                    action = Action::Deny;
                    pattern = constraint_pattern.clone();
                    final_action = Action::Deny;
                }
                TerminalConstraint::ParentCap {
                    action: cap_action,
                    pattern: cap_pattern,
                } if cap_action.is_more_restrictive_than(overridden_action) => {
                    action = *cap_action;
                    pattern = cap_pattern.clone();
                    final_action = *cap_action;
                }
                _ => {
                    final_action = overridden_action;
                }
            }
        }
        let mut feedback = None;
        let outcome = match final_action {
            Action::Allow => {
                if override_applied {
                    self.publish_permission_replied(
                        request,
                        &pattern,
                        PermissionDecision::Allow,
                        None,
                    );
                }
                Outcome::Allowed
            }
            Action::Deny => {
                if override_applied {
                    self.publish_permission_replied(
                        request,
                        &pattern,
                        PermissionDecision::Deny,
                        None,
                    );
                }
                Outcome::Denied
            }
            Action::Ask => {
                self.publish_permission_asked(request, &pattern);
                let reply = self.resolve_ask(request);

                if reply.scope == DecisionScope::Always {
                    if let Some(saved) = &self.saved {
                        saved.record(SavedDecision::new(&request.tool, &pattern, reply.decision));
                    }
                }

                let outcome_decision = match reply.decision {
                    Decision::Allow => PermissionDecision::Allow,
                    Decision::Deny => PermissionDecision::Deny,
                };
                self.publish_permission_replied(
                    request,
                    &pattern,
                    outcome_decision,
                    reply.feedback.as_deref(),
                );
                feedback = normalize_feedback(reply.feedback.clone());
                reply.decision.outcome()
            }
        };
        PermissionEvaluation {
            action,
            pattern,
            outcome,
            feedback,
        }
    }

    fn resolve_ask(&self, request: &PermissionRequest) -> ScopedDecision {
        let key = (request.tool.clone(), request.subject.clone());
        let (pending, leader) = {
            let mut state = self
                .in_flight_asks
                .state
                .lock()
                .expect("in-flight asks map lock");
            if let Some(existing) = state.get(&key) {
                (existing.clone(), false)
            } else {
                let pending = Arc::new(InFlightAsk::new());
                state.insert(key.clone(), pending.clone());
                (pending, true)
            }
        };

        if !leader {
            return pending.wait();
        }

        // No responder (headless) -> fail closed: deny without waiting, and never save.
        let reply = match &self.responder {
            Some(responder) => responder.respond_scoped(request),
            None => ScopedDecision::once(Decision::Deny),
        };
        let reply = ScopedDecision {
            feedback: normalize_feedback(reply.feedback),
            ..reply
        };
        pending.resolve(reply.clone());
        if let Ok(mut state) = self.in_flight_asks.state.lock() {
            state.remove(&key);
        }
        reply
    }

    /// The effective action and the pattern that classified it, combining hard-deny constraints,
    /// saved decisions, and the configured ruleset, then narrowing by the parent cap.
    fn effective(&self, tool: &str, subject: &str) -> EffectiveDecision {
        // Hard denies are absolute safety/mode constraints. They are checked before saved
        // decisions so a saved "always allow" cannot widen a read-only mode or similar hard block.
        if let Some(pattern) = self.hard_deny_pattern(tool, subject) {
            return EffectiveDecision {
                action: Action::Deny,
                pattern: pattern.clone(),
                constraint: TerminalConstraint::HardDeny { pattern },
            };
        }

        // A saved "always" decision takes precedence over the ordinary configured ruleset;
        // otherwise the ruleset decides. Either way we keep the pattern that classified the action
        // for the event.
        let (own_action, own_pattern) = self
            .saved
            .as_ref()
            .and_then(|saved| saved.resolve(tool, subject))
            .map(|saved| (saved.decision.action(), saved.pattern))
            .unwrap_or_else(|| {
                let (action, rule) = self.ruleset.resolve(tool, subject);
                (
                    action,
                    rule.map_or_else(|| "*".to_owned(), |r| r.pattern.clone()),
                )
            });

        // Narrow by the parent cap (subagent): the most restrictive of own and parent governs.
        if let Some(cap) = &self.parent_cap {
            let (cap_action, cap_rule) = cap.resolve(tool, subject);
            let cap_pattern = cap_rule.map_or_else(|| "*".to_owned(), |r| r.pattern.clone());
            return if cap_action.is_more_restrictive_than(own_action) {
                EffectiveDecision {
                    action: cap_action,
                    pattern: cap_pattern.clone(),
                    constraint: TerminalConstraint::ParentCap {
                        action: cap_action,
                        pattern: cap_pattern,
                    },
                }
            } else {
                EffectiveDecision {
                    action: own_action,
                    pattern: own_pattern,
                    constraint: TerminalConstraint::ParentCap {
                        action: cap_action,
                        pattern: cap_pattern,
                    },
                }
            };
        }
        EffectiveDecision {
            action: own_action,
            pattern: own_pattern,
            constraint: TerminalConstraint::None,
        }
    }

    fn hard_deny_pattern(&self, tool: &str, subject: &str) -> Option<String> {
        let (action, rule) = self.hard_denies.resolve(tool, subject);
        (action == Action::Deny).then(|| rule.map_or_else(|| "*".to_owned(), |r| r.pattern.clone()))
    }

    fn publish(&self, event: events::Event) {
        if let Some(bus) = &self.bus {
            bus.publish(&event);
        }
    }

    fn publish_permission_asked(&self, request: &PermissionRequest, pattern: &str) {
        let event = match &self.event_context {
            Some(context) => events::Event::permission_asked_for_turn(
                &context.session_id,
                &context.message_id,
                &request.tool,
                pattern,
                &request.subject,
            ),
            None => events::Event::permission_asked(&request.tool, pattern, &request.subject),
        };
        self.publish(event);
    }

    fn publish_permission_replied(
        &self,
        request: &PermissionRequest,
        pattern: &str,
        decision: PermissionDecision,
        feedback: Option<&str>,
    ) {
        let event = match &self.event_context {
            Some(context) => events::Event::permission_replied_for_turn(
                &context.session_id,
                &context.message_id,
                &request.tool,
                pattern,
                &request.subject,
                decision,
                feedback,
            ),
            None => events::Event::permission_replied(
                &request.tool,
                pattern,
                &request.subject,
                decision,
                feedback,
            ),
        };
        self.publish(event);
    }
}

fn normalize_feedback(feedback: Option<String>) -> Option<String> {
    feedback
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use events::Event;

    use super::*;
    use crate::rule::Rule;

    /// A responder that always returns a fixed decision.
    struct FixedResponder {
        decision: Decision,
    }

    impl Responder for FixedResponder {
        fn respond(&self, _request: &PermissionRequest) -> Decision {
            self.decision
        }
    }

    struct ScopedResponder {
        reply: ScopedDecision,
        calls: AtomicUsize,
    }

    impl ScopedResponder {
        fn new(reply: ScopedDecision) -> Self {
            Self {
                reply,
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl Responder for ScopedResponder {
        fn respond(&self, _request: &PermissionRequest) -> Decision {
            self.reply.decision
        }

        fn respond_scoped(&self, _request: &PermissionRequest) -> ScopedDecision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.reply.clone()
        }
    }

    struct FixedOverride {
        decision: Decision,
    }

    impl DecisionOverride for FixedOverride {
        fn override_decision(
            &self,
            _request: &PermissionRequest,
            _proposed: Action,
        ) -> Option<Decision> {
            Some(self.decision)
        }
    }

    #[test]
    fn allow_rule_is_allowed_without_consulting_a_responder() {
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "bash",
            "echo *",
            Action::Allow,
        )]));
        let outcome = engine.evaluate(&PermissionRequest::new("bash", "echo hi"));
        assert_eq!(outcome, Outcome::Allowed);
    }

    #[test]
    fn deny_rule_is_denied() {
        let engine =
            PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "rm *", Action::Deny)]));
        let outcome = engine.evaluate(&PermissionRequest::new("bash", "rm target.txt"));
        assert_eq!(outcome, Outcome::Denied);
    }

    #[test]
    fn default_ask_with_no_responder_fails_closed() {
        let engine = PermissionEngine::new(Ruleset::empty());
        let outcome = engine.evaluate(&PermissionRequest::new("bash", "rm target.txt"));
        assert_eq!(outcome, Outcome::Denied);
    }

    #[test]
    fn ask_with_allowing_responder_is_allowed() {
        let engine =
            PermissionEngine::new(Ruleset::empty()).with_responder(Arc::new(FixedResponder {
                decision: Decision::Allow,
            }));
        let outcome = engine.evaluate(&PermissionRequest::new("write", "src/a.txt"));
        assert_eq!(outcome, Outcome::Allowed);
    }

    #[test]
    fn ask_with_denying_responder_is_denied() {
        let engine =
            PermissionEngine::new(Ruleset::empty()).with_responder(Arc::new(FixedResponder {
                decision: Decision::Deny,
            }));
        let outcome = engine.evaluate(&PermissionRequest::new("write", "src/a.txt"));
        assert_eq!(outcome, Outcome::Denied);
    }

    #[test]
    fn deny_feedback_is_returned_with_denied_evaluation() {
        let responder = Arc::new(ScopedResponder::new(
            ScopedDecision::once(Decision::Deny).with_feedback("Use ls first."),
        ));
        let engine = PermissionEngine::new(Ruleset::empty()).with_responder(responder);

        let evaluation = engine.evaluate_detail(&PermissionRequest::new("bash", "rm target.txt"));
        assert_eq!(evaluation.outcome, Outcome::Denied);
        assert_eq!(evaluation.feedback.as_deref(), Some("Use ls first."));
    }

    #[test]
    fn concurrent_identical_asks_share_one_reply_with_feedback() {
        struct BlockingResponder {
            inner: Arc<ScopedResponder>,
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }

        impl Responder for BlockingResponder {
            fn respond(&self, request: &PermissionRequest) -> Decision {
                self.inner.respond(request)
            }

            fn respond_scoped(&self, request: &PermissionRequest) -> ScopedDecision {
                let _ = self.entered.send(());
                let _ = self.release.lock().expect("release lock").recv();
                self.inner.respond_scoped(request)
            }
        }

        let responder = Arc::new(ScopedResponder::new(
            ScopedDecision::once(Decision::Deny).with_feedback("Inspect before deleting."),
        ));
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let blocking = Arc::new(BlockingResponder {
            inner: responder.clone(),
            entered: entered_tx,
            release: Mutex::new(release_rx),
        });
        let engine = Arc::new(PermissionEngine::new(Ruleset::empty()).with_responder(blocking));
        let request = PermissionRequest::new("bash", "rm target.txt");

        let first = {
            let engine = engine.clone();
            let request = request.clone();
            thread::spawn(move || engine.evaluate_detail(&request))
        };
        entered_rx.recv().expect("first ask reached responder");
        let second = {
            let engine = engine.clone();
            thread::spawn(move || engine.evaluate_detail(&request))
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = release_tx.send(());

        let first_eval = first.join().expect("first evaluation");
        let second_eval = second.join().expect("second evaluation");
        assert_eq!(
            first_eval.feedback.as_deref(),
            Some("Inspect before deleting.")
        );
        assert_eq!(
            second_eval.feedback.as_deref(),
            Some("Inspect before deleting.")
        );
        assert_eq!(responder.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn hard_denies_outrank_saved_allows() {
        let saved = Arc::new(SavedDecisions::from_decisions(vec![SavedDecision::new(
            "bash",
            "echo *",
            Decision::Allow,
        )]));
        let engine = PermissionEngine::new(Ruleset::empty())
            .with_saved_decisions(saved)
            .with_hard_denies(Ruleset::new(vec![Rule::new("bash", "*", Action::Deny)]));

        assert_eq!(engine.classify("bash", "echo hi"), Action::Deny);
        assert_eq!(
            engine.evaluate(&PermissionRequest::new("bash", "echo hi")),
            Outcome::Denied
        );
    }

    #[test]
    fn parent_cap_snapshot_preserves_saved_and_hard_denies() {
        let parent =
            PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "*", Action::Allow)]))
                .with_saved_decisions(Arc::new(SavedDecisions::from_decisions(vec![
                    SavedDecision::new("bash", "rm *", Decision::Deny),
                ])))
                .with_hard_denies(Ruleset::new(vec![Rule::new("write", "*", Action::Deny)]));
        let child = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("bash", "*", Action::Allow),
            Rule::new("write", "*", Action::Allow),
        ]))
        .with_parent_cap(parent.parent_cap_ruleset());

        assert_eq!(
            child.classify("bash", "rm victim.txt"),
            Action::Deny,
            "a parent's saved deny must narrow its child"
        );
        assert_eq!(
            child.classify("write", "new.txt"),
            Action::Deny,
            "a parent's hard deny must narrow its child"
        );
    }

    #[test]
    fn override_allow_cannot_lift_a_hard_deny_and_keeps_its_provenance() {
        let bus = Arc::new(EventBus::new());
        let replies = Arc::new(Mutex::new(Vec::new()));
        let observed_replies = replies.clone();
        bus.subscribe(Arc::new(move |event: &Event| {
            if let Event::PermissionReplied(reply) = event {
                observed_replies
                    .lock()
                    .expect("replies lock")
                    .push(reply.decision);
            }
        }));
        let engine =
            PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "*", Action::Allow)]))
                .with_hard_denies(Ruleset::new(vec![Rule::new(
                    "bash",
                    "printf blocked",
                    Action::Deny,
                )]))
                .with_override(Arc::new(FixedOverride {
                    decision: Decision::Allow,
                }))
                .with_bus(bus);

        let evaluation = engine.evaluate_detail(&PermissionRequest::new("bash", "printf blocked"));
        assert_eq!(evaluation.action, Action::Deny);
        assert_eq!(evaluation.pattern, "printf blocked");
        assert_eq!(evaluation.outcome, Outcome::Denied);
        assert_eq!(
            *replies.lock().expect("replies lock"),
            vec![PermissionDecision::Deny],
            "the event must report the constrained denial, not the plugin's allow"
        );
    }

    #[test]
    fn override_allow_cannot_widen_a_parent_deny() {
        let parent = Ruleset::new(vec![Rule::new("write", "restricted.txt", Action::Deny)]);
        let child = Ruleset::new(vec![Rule::new("write", "*", Action::Allow)]);
        let engine = PermissionEngine::new(child)
            .with_parent_cap(parent)
            .with_override(Arc::new(FixedOverride {
                decision: Decision::Allow,
            }));

        let evaluation = engine.evaluate_detail(&PermissionRequest::new("write", "restricted.txt"));
        assert_eq!(evaluation.action, Action::Deny);
        assert_eq!(evaluation.pattern, "restricted.txt");
        assert_eq!(evaluation.outcome, Outcome::Denied);
    }

    #[test]
    fn override_allow_cannot_widen_a_parent_ask() {
        let parent = Ruleset::new(vec![Rule::new("write", "review.txt", Action::Ask)]);
        let child = Ruleset::new(vec![Rule::new("write", "*", Action::Allow)]);
        let engine = PermissionEngine::new(child)
            .with_parent_cap(parent)
            .with_override(Arc::new(FixedOverride {
                decision: Decision::Allow,
            }));

        let evaluation = engine.evaluate_detail(&PermissionRequest::new("write", "review.txt"));
        assert_eq!(evaluation.action, Action::Ask);
        assert_eq!(evaluation.pattern, "review.txt");
        assert_eq!(
            evaluation.outcome,
            Outcome::Denied,
            "the headless parent ask must still fail closed"
        );
    }

    #[test]
    fn override_still_widens_ordinary_denies_and_narrows_ordinary_allows() {
        let allow_override = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "write",
            "ordinary.txt",
            Action::Deny,
        )]))
        .with_override(Arc::new(FixedOverride {
            decision: Decision::Allow,
        }));
        let deny_override = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "write",
            "ordinary.txt",
            Action::Allow,
        )]))
        .with_override(Arc::new(FixedOverride {
            decision: Decision::Deny,
        }));
        let ask_deny_override = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "write",
            "ordinary-ask.txt",
            Action::Ask,
        )]))
        .with_responder(Arc::new(FixedResponder {
            decision: Decision::Allow,
        }))
        .with_override(Arc::new(FixedOverride {
            decision: Decision::Deny,
        }));

        assert_eq!(
            allow_override.evaluate(&PermissionRequest::new("write", "ordinary.txt")),
            Outcome::Allowed,
            "plugins may still widen ordinary non-terminal decisions"
        );
        assert_eq!(
            deny_override.evaluate(&PermissionRequest::new("write", "ordinary.txt")),
            Outcome::Denied,
            "plugins may narrow ordinary decisions"
        );
        assert_eq!(
            ask_deny_override.evaluate(&PermissionRequest::new("write", "ordinary-ask.txt")),
            Outcome::Denied,
            "plugins may narrow ordinary asks without consulting a responder"
        );
    }
}
