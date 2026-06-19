//! Session-scoped saved permission decisions ("always allow/deny this pattern").
//!
//! When an `ask` is answered with "always", the choice is recorded as a [`SavedDecision`] so later
//! matching actions in the same session reuse it without re-asking (architecture §3.4). A
//! [`SavedDecisions`] store is shared (via `Arc`) across the turns of a session and consulted by
//! [`crate::PermissionEngine`] **before** the configured ruleset: a matching saved allow/deny
//! resolves the action directly and emits **no** new `permission.asked` event (VAL-PERM-011).
//!
//! The store snapshots to and rebuilds from a plain list of decisions so a session can persist
//! saved decisions across process restarts (the durable cross-process reuse is a separate feature).

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::engine::Decision;
use crate::glob::glob_match;
use crate::rule::ANY_TOOL;

/// One saved "always allow/deny" decision for a tool/pattern.
///
/// Serializes field-for-field (`tool`, `pattern`, `decision`) so a snapshot taken via
/// [`SavedDecisions::snapshot`] persists durably (e.g. to JSON on disk) and rebuilds losslessly via
/// [`SavedDecisions::from_decisions`], enabling cross-process reuse of an "always" choice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedDecision {
    /// The gated tool the decision was saved for, or [`ANY_TOOL`] (`*`) for any tool.
    pub tool: String,
    /// The glob pattern (matched against the action subject) the decision applies to.
    pub pattern: String,
    /// Whether matching actions are always allowed or always denied.
    pub decision: Decision,
}

impl SavedDecision {
    /// Build a saved decision for `tool`/`pattern`.
    #[must_use]
    pub fn new(tool: impl Into<String>, pattern: impl Into<String>, decision: Decision) -> Self {
        Self {
            tool: tool.into(),
            pattern: pattern.into(),
            decision,
        }
    }

    /// Whether this saved decision matches a gated action against `tool` with subject `subject`.
    #[must_use]
    pub fn matches(&self, tool: &str, subject: &str) -> bool {
        (self.tool == ANY_TOOL || self.tool == tool) && glob_match(&self.pattern, subject)
    }
}

/// A session's store of saved "always" decisions, consulted before the configured ruleset.
///
/// The store is append-only within a session and uses interior mutability so it can be shared
/// (via `Arc`) across the engines built for successive turns and recorded from the engine's
/// `&self` evaluation path. Resolution is last-matching-wins, mirroring the ruleset, so a later
/// saved decision overrides an earlier one for the same subject.
#[derive(Debug, Default)]
pub struct SavedDecisions {
    decisions: Mutex<Vec<SavedDecision>>,
}

impl SavedDecisions {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild a store from previously snapshotted decisions (e.g. ones loaded from disk), in order.
    #[must_use]
    pub fn from_decisions(decisions: Vec<SavedDecision>) -> Self {
        Self {
            decisions: Mutex::new(decisions),
        }
    }

    /// Record a saved decision (append; it takes precedence over earlier ones for the same subject).
    pub fn record(&self, decision: SavedDecision) {
        self.decisions
            .lock()
            .expect("saved-decisions lock")
            .push(decision);
    }

    /// The last saved decision matching a gated action, if any.
    #[must_use]
    pub fn resolve(&self, tool: &str, subject: &str) -> Option<SavedDecision> {
        self.decisions
            .lock()
            .expect("saved-decisions lock")
            .iter()
            .rev()
            .find(|saved| saved.matches(tool, subject))
            .cloned()
    }

    /// A point-in-time copy of every saved decision in record order (for persistence).
    #[must_use]
    pub fn snapshot(&self) -> Vec<SavedDecision> {
        self.decisions.lock().expect("saved-decisions lock").clone()
    }

    /// Whether the store holds no saved decisions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decisions
            .lock()
            .expect("saved-decisions lock")
            .is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_decision_matches_tool_and_pattern() {
        let saved = SavedDecision::new("bash", "echo *", Decision::Allow);
        assert!(saved.matches("bash", "echo hi"));
        assert!(!saved.matches("bash", "rm x"));
        assert!(!saved.matches("write", "echo hi"));

        let any_tool = SavedDecision::new("*", "secrets/**", Decision::Deny);
        assert!(any_tool.matches("read", "secrets/k.txt"));
        assert!(any_tool.matches("write", "secrets/k.txt"));
    }

    #[test]
    fn resolve_returns_the_last_matching_decision() {
        let store = SavedDecisions::new();
        assert!(store.is_empty());
        store.record(SavedDecision::new("bash", "echo *", Decision::Deny));
        store.record(SavedDecision::new("bash", "echo *", Decision::Allow));
        assert_eq!(
            store.resolve("bash", "echo hi"),
            Some(SavedDecision::new("bash", "echo *", Decision::Allow))
        );
        assert_eq!(store.resolve("bash", "rm x"), None);
    }

    #[test]
    fn saved_decision_serializes_with_stable_lowercase_tokens() {
        let saved = SavedDecision::new("bash", "echo *", Decision::Allow);
        let value = serde_json::to_value(&saved).expect("serialize");
        assert_eq!(value["tool"], "bash");
        assert_eq!(value["pattern"], "echo *");
        assert_eq!(value["decision"], "allow");

        // The JSON form round-trips back to an identical decision (the durable on-disk contract).
        let decoded: SavedDecision = serde_json::from_value(value).expect("deserialize");
        assert_eq!(decoded, saved);
    }

    #[test]
    fn snapshot_round_trips_through_from_decisions() {
        let store = SavedDecisions::new();
        store.record(SavedDecision::new("bash", "echo *", Decision::Allow));
        store.record(SavedDecision::new("write", "src/**", Decision::Allow));
        let snapshot = store.snapshot();
        assert_eq!(snapshot.len(), 2);

        let rebuilt = SavedDecisions::from_decisions(snapshot.clone());
        assert_eq!(rebuilt.snapshot(), snapshot);
        assert_eq!(
            rebuilt.resolve("write", "src/a.txt"),
            Some(SavedDecision::new("write", "src/**", Decision::Allow))
        );
    }
}
