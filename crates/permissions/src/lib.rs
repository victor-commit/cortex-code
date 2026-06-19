//! The Cortex Code permission engine: resolve every gated tool action to allow/ask/deny.
//!
//! This crate is the M2 capability that decides whether a gated tool action may run (architecture
//! §3.4). It is deliberately small and subject-agnostic — it answers "given this tool and this
//! subject, what should happen?" and surfaces asks observably — leaving *what the subject is* (a
//! file path for file tools, the parsed command for `bash`), per-agent ruleset selection, subagent
//! narrowing, and saved decisions to the scoping layer that builds on it.
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
mod config;
mod engine;
mod glob;
mod rule;
mod ruleset;

pub use action::Action;
pub use config::{RuleError, load_layered, rules_from_table};
pub use engine::{Decision, Outcome, PermissionEngine, PermissionRequest, Responder};
pub use glob::glob_match;
pub use rule::{ANY_TOOL, Rule};
pub use ruleset::Ruleset;
