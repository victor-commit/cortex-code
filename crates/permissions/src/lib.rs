//! The Cortex Code permission engine: resolve every gated tool action to allow/ask/deny.
//!
//! This crate is the M2 capability that decides whether a gated tool action may run (architecture
//! §3.4). The core engine is deliberately small and subject-agnostic — it answers "given this tool
//! and this subject, what should happen?" and surfaces asks observably — and the scoping layer on
//! top supplies the rest:
//!
//! - **Subject derivation** ([`scope`]): file tools scope by their normalized target path; `bash`
//!   scopes by its command (with compound-command splitting). [`request_for`] builds the
//!   right-subject [`PermissionRequest`] for a tool (VAL-PERM-006/007).
//! - **Per-agent rulesets** ([`AgentRulesets`]): each agent carries its own ruleset layered on a
//!   shared base, so two agents decide the same action differently (VAL-PERM-009).
//! - **Subagent narrowing** ([`narrow_action`] / [`PermissionEngine::with_parent_cap`]): a spawned
//!   subagent's effective permission is the most restrictive of its own and its parent's, so it can
//!   never exceed the parent (VAL-PERM-010).
//! - **Saved decisions** ([`SavedDecisions`]): an "always allow/deny this pattern" choice is reused
//!   for later matching actions in the session with no new ask (VAL-PERM-011), except that
//!   explicit hard-deny constraints still win.
//!
//! ## Model (blueprint: the durable runner's permission layer, architecture §4)
//!
//! - A [`Rule`] is `{ tool, pattern, action }`. Its `tool` selector matches a gated tool (exactly,
//!   or `*` for any), and its glob `pattern` ([`glob_match`]) is matched against the action's
//!   subject.
//! - A [`Ruleset`] is an ordered list of rules evaluated **last-matching-rule-wins**: the last
//!   rule that matches decides the outcome (VAL-PERM-008). With no matching rule the action
//!   defaults to [`Action::Ask`] — never a silent allow or deny (VAL-PERM-001).
//! - Rules from multiple config layers combine into one ordered ruleset with
//!   [`Ruleset::from_layers`] / [`load_layered`], concatenated in ascending precedence order
//!   (global → project → nested) so a higher layer overrides a conflicting lower one
//!   deterministically (VAL-PERM-017), consistent with the `settings` layering order.
//! - The [`PermissionEngine`] resolves a [`PermissionRequest`] to an [`Outcome`]: `allow` runs
//!   immediately with no event; `deny` blocks the effect; `ask` publishes a `permission.asked`
//!   event naming the tool and pattern, then consults a [`Responder`]. In headless `cortex run`
//!   with **no responder**, `ask` **fails closed** (denies in bounded time, no hang;
//!   VAL-PERM-005).
//!
//! Events (`permission.asked` / `permission.replied`) are published on the `events` bus so the
//! pending request is observable to any subscriber (the headless event stream, the server SSE
//! feed, an audit sink).

mod action;
mod agent;
mod config;
mod engine;
mod glob;
mod rule;
mod ruleset;
mod saved;
mod scope;
pub mod store;

pub use action::Action;
pub use agent::{AgentRulesets, narrow_action};
pub use config::{RuleError, load_agent_rulesets, load_layered, rules_from_table};
pub use engine::{
    Decision, DecisionScope, Outcome, PermissionEngine, PermissionRequest, Responder,
    ScopedDecision,
};
pub use glob::glob_match;
pub use rule::{ANY_TOOL, Rule};
pub use ruleset::Ruleset;
pub use saved::{SavedDecision, SavedDecisions};
pub use scope::{
    ToolScope, classify_command, command_subject, file_subject, is_command_tool, is_file_tool,
    normalize_path, request_for, scope_of, split_bash_commands,
};
