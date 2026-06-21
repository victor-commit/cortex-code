//! The permission engine: resolve a gated action to allow/deny, surfacing asks observably and
//! failing closed when an ask cannot be answered.

use std::sync::Arc;

use events::{EventBus, PermissionDecision};
use serde::{Deserialize, Serialize};

use crate::action::Action;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopedDecision {
    /// Allow or deny the action.
    pub decision: Decision,
    /// Whether the decision is saved for later matching actions.
    pub scope: DecisionScope,
}

impl ScopedDecision {
    /// A one-shot decision applying only to the current action.
    #[must_use]
    pub const fn once(decision: Decision) -> Self {
        Self {
            decision,
            scope: DecisionScope::Once,
        }
    }

    /// A saved decision applied to and remembered for later matching actions.
    #[must_use]
    pub const fn always(decision: Decision) -> Self {
        Self {
            decision,
            scope: DecisionScope::Always,
        }
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
    bus: Option<Arc<EventBus>>,
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
            bus: None,
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

    /// Attach the event bus that `permission.asked`/`permission.replied` are published on
    /// (builder style); without one, the engine resolves silently.
    #[must_use]
    pub fn with_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.bus = Some(bus);
        self
    }

    /// The combined ruleset this engine evaluates.
    #[must_use]
    pub fn ruleset(&self) -> &Ruleset {
        &self.ruleset
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
        self.effective(tool, subject).0
    }

    /// Resolve a gated action to a final [`Outcome`], surfacing and answering an `ask` as needed.
    pub fn evaluate(&self, request: &PermissionRequest) -> Outcome {
        let (action, pattern) = self.effective(&request.tool, &request.subject);
        match action {
            Action::Allow => Outcome::Allowed,
            Action::Deny => Outcome::Denied,
            Action::Ask => {
                self.publish(events::Event::permission_asked(
                    &request.tool,
                    &pattern,
                    &request.subject,
                ));

                // No responder (headless) → fail closed: deny without waiting, and never save.
                let reply = match &self.responder {
                    Some(responder) => responder.respond_scoped(request),
                    None => ScopedDecision::once(Decision::Deny),
                };

                if reply.scope == DecisionScope::Always {
                    if let Some(saved) = &self.saved {
                        saved.record(SavedDecision::new(&request.tool, &pattern, reply.decision));
                    }
                }

                let outcome_decision = match reply.decision {
                    Decision::Allow => PermissionDecision::Allow,
                    Decision::Deny => PermissionDecision::Deny,
                };
                self.publish(events::Event::permission_replied(
                    &request.tool,
                    &pattern,
                    &request.subject,
                    outcome_decision,
                ));
                reply.decision.outcome()
            }
        }
    }

    /// The effective action and the pattern that classified it, combining hard-deny constraints,
    /// saved decisions, and the configured ruleset, then narrowing by the parent cap.
    fn effective(&self, tool: &str, subject: &str) -> (Action, String) {
        // Hard denies are absolute safety/mode constraints. They are checked before saved
        // decisions so a saved "always allow" cannot widen a read-only mode or similar hard block.
        if let Some(pattern) = self.hard_deny_pattern(tool, subject) {
            return (Action::Deny, pattern);
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
            if cap_action.is_more_restrictive_than(own_action) {
                let cap_pattern = cap_rule.map_or_else(|| "*".to_owned(), |r| r.pattern.clone());
                return (cap_action, cap_pattern);
            }
        }
        (own_action, own_pattern)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::Rule;

    /// A responder that always returns a fixed decision, recording the requests it saw.
    struct FixedResponder {
        decision: Decision,
    }

    impl Responder for FixedResponder {
        fn respond(&self, _request: &PermissionRequest) -> Decision {
            self.decision
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
        // Empty ruleset → default ask; no responder → fail closed (deny), no hang.
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
}
