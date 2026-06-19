//! The permission engine: resolve a gated action to allow/deny, surfacing asks observably and
//! failing closed when an ask cannot be answered.

use std::sync::Arc;

use events::{EventBus, PermissionDecision};

use crate::action::Action;
use crate::ruleset::Ruleset;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Allow the action to proceed.
    Allow,
    /// Block the action.
    Deny,
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
    responder: Option<Arc<dyn Responder>>,
    bus: Option<Arc<EventBus>>,
}

impl PermissionEngine {
    /// Build a headless engine from a ruleset: it has no responder, so every `ask` fails closed.
    #[must_use]
    pub fn new(ruleset: Ruleset) -> Self {
        Self {
            ruleset,
            responder: None,
            bus: None,
        }
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
    #[must_use]
    pub fn classify(&self, tool: &str, subject: &str) -> Action {
        self.ruleset.classify(tool, subject)
    }

    /// Resolve a gated action to a final [`Outcome`], surfacing and answering an `ask` as needed.
    pub fn evaluate(&self, request: &PermissionRequest) -> Outcome {
        let (action, rule) = self.ruleset.resolve(&request.tool, &request.subject);
        match action {
            Action::Allow => Outcome::Allowed,
            Action::Deny => Outcome::Denied,
            Action::Ask => {
                // The pattern that classified the action: the matching rule's pattern, or the
                // implicit `*` of the default-ask case when no rule matched.
                let pattern = rule.map_or("*", |rule| rule.pattern.as_str());
                self.publish(events::Event::permission_asked(
                    &request.tool,
                    pattern,
                    &request.subject,
                ));

                // No responder (headless) → fail closed: deny without waiting.
                let decision = match &self.responder {
                    Some(responder) => responder.respond(request),
                    None => Decision::Deny,
                };

                let (outcome, reply) = match decision {
                    Decision::Allow => (Outcome::Allowed, PermissionDecision::Allow),
                    Decision::Deny => (Outcome::Denied, PermissionDecision::Deny),
                };
                self.publish(events::Event::permission_replied(
                    &request.tool,
                    pattern,
                    &request.subject,
                    reply,
                ));
                outcome
            }
        }
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
}
