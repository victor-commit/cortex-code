//! Build the headless permission engine from `cortex run`'s repeatable rule flags.
//!
//! `cortex run` accepts `--permission-allow <TOOL:GLOB>` and `--permission-deny <TOOL:GLOB>` (each
//! repeatable). This module parses those specs into permission [`Rule`]s and assembles the
//! [`PermissionEngine`] the headless [`tools::ToolContext`] consults before every gated tool effect
//! (architecture §3.4 / §4). The tools already call that engine through `ctx.permits`, but the
//! default headless context attached none — so until this wiring landed, headless runs were
//! ungated. With the engine attached, a denied `write`/`edit`/`apply_patch`/`bash`/`read` produces
//! no side effect: the deny path returns before any byte is written or any command runs.
//!
//! ## Rule spec form
//! Each flag value is `TOOL:GLOB`, split on the first `:`:
//! - `TOOL` selects the gated tool (`read`/`write`/`edit`/`apply_patch`/`glob`/`grep`/`bash`) or
//!   `*` for any tool. An unknown tool is rejected so a mistyped rule never silently fails open.
//! - `GLOB` is matched against the action's subject — a normalized file path for the file tools, or
//!   the parsed command for `bash` (see [`permissions::request_for`]).
//!
//! ## Ordering / precedence
//! Allow rules are emitted before deny rules, so under the engine's last-matching-rule-wins
//! evaluation an explicit `--permission-deny` always overrides a broader `--permission-allow`
//! (deny-wins; fail-closed). With no flags supplied the engine is absent and tools run ungated,
//! preserving the default headless behavior the other M2 tool features rely on.

use std::sync::Arc;

use permissions::{ANY_TOOL, Action, PermissionEngine, Rule, Ruleset, scope_of};

/// Build the optional headless [`PermissionEngine`] from the repeatable allow/deny rule flags.
///
/// Allow specs become `allow` rules and deny specs become `deny` rules; the allows are ordered
/// before the denies so a deny overrides a conflicting allow under last-matching-rule-wins. Returns
/// `Ok(None)` when neither flag is supplied (tools run ungated, as before this wiring landed).
///
/// # Errors
/// Returns a human-readable message when a spec is not `TOOL:GLOB`, names an unknown tool, or has
/// an empty glob.
pub fn engine_from_flags(
    allow: &[String],
    deny: &[String],
) -> Result<Option<Arc<PermissionEngine>>, String> {
    if allow.is_empty() && deny.is_empty() {
        return Ok(None);
    }
    let mut rules = Vec::with_capacity(allow.len() + deny.len());
    for spec in allow {
        rules.push(parse_spec(spec, Action::Allow)?);
    }
    for spec in deny {
        rules.push(parse_spec(spec, Action::Deny)?);
    }
    Ok(Some(Arc::new(PermissionEngine::new(Ruleset::new(rules)))))
}

/// Parse one `TOOL:GLOB` spec into a [`Rule`] carrying `action`.
fn parse_spec(spec: &str, action: Action) -> Result<Rule, String> {
    let (tool, glob) = spec.split_once(':').ok_or_else(|| {
        format!(
            "invalid permission rule `{spec}`: expected TOOL:GLOB \
             (for example `write:src/**` or `bash:rm *`)"
        )
    })?;
    let glob = glob.trim();
    if glob.is_empty() {
        return Err(format!(
            "invalid permission rule `{spec}`: the glob after `:` is empty \
             (for example `write:src/**`)"
        ));
    }
    let tool = tool.trim();
    let tool = if tool.is_empty() { ANY_TOOL } else { tool };
    if tool != ANY_TOOL && scope_of(tool).is_none() {
        return Err(format!(
            "invalid permission rule `{spec}`: unknown tool `{tool}` \
             (expected read, write, edit, apply_patch, glob, grep, bash, or *)"
        ));
    }
    Ok(Rule::new(tool, glob, action))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_flags_yield_no_engine() {
        assert!(engine_from_flags(&[], &[]).expect("ok").is_none());
    }

    #[test]
    fn parses_allow_and_deny_specs() {
        assert_eq!(
            parse_spec("write:src/**", Action::Allow).expect("rule"),
            Rule::new("write", "src/**", Action::Allow)
        );
        assert_eq!(
            parse_spec("bash:rm *", Action::Deny).expect("rule"),
            Rule::new("bash", "rm *", Action::Deny)
        );
    }

    #[test]
    fn wildcard_tool_is_accepted() {
        assert_eq!(
            parse_spec("*:**", Action::Allow).expect("rule"),
            Rule::new("*", "**", Action::Allow)
        );
    }

    #[test]
    fn splits_on_the_first_colon_only() {
        // A `bash` glob may legitimately contain a colon; only the first colon separates TOOL.
        assert_eq!(
            parse_spec("bash:git log --format=%H:%s", Action::Deny).expect("rule"),
            Rule::new("bash", "git log --format=%H:%s", Action::Deny)
        );
    }

    #[test]
    fn empty_tool_defaults_to_any() {
        assert_eq!(
            parse_spec(":src/**", Action::Allow).expect("rule"),
            Rule::new("*", "src/**", Action::Allow)
        );
    }

    #[test]
    fn rejects_a_spec_without_a_colon() {
        let err = parse_spec("secrets/**", Action::Deny).unwrap_err();
        assert!(err.contains("TOOL:GLOB"), "{err}");
    }

    #[test]
    fn rejects_an_unknown_tool() {
        let err = parse_spec("wrte:src/**", Action::Allow).unwrap_err();
        assert!(err.contains("unknown tool"), "{err}");
    }

    #[test]
    fn rejects_an_empty_glob() {
        let err = parse_spec("write:", Action::Allow).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn allows_are_ordered_before_denies_so_deny_wins() {
        let engine = engine_from_flags(&["*:**".to_owned()], &["write:secrets/**".to_owned()])
            .expect("ok")
            .expect("engine");
        // The deny is evaluated last, so it wins for a matching subject; the broad allow governs
        // everything else (a non-secrets write, and read-class tools).
        assert_eq!(engine.classify("write", "secrets/k.txt"), Action::Deny);
        assert_eq!(engine.classify("write", "src/k.txt"), Action::Allow);
        assert_eq!(engine.classify("read", "secrets/k.txt"), Action::Allow);
    }

    #[test]
    fn a_malformed_spec_aborts_the_whole_build() {
        // `PermissionEngine` is not `Debug`, so match instead of `unwrap_err`.
        let err = match engine_from_flags(&["*:**".to_owned()], &["not-a-rule".to_owned()]) {
            Ok(_) => panic!("a malformed spec must error"),
            Err(message) => message,
        };
        assert!(err.contains("TOOL:GLOB"), "{err}");
    }
}
