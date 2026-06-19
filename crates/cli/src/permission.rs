//! Build the headless permission engine from `cortex run`'s config-file permission layers and its
//! repeatable rule flags.
//!
//! `cortex run` honors two sources of permission rules, combined into one ordered ruleset evaluated
//! last-matching-rule-wins by the [`PermissionEngine`] the headless [`tools::ToolContext`] consults
//! before every gated tool effect (architecture §3.4 / §4):
//!
//! 1. **Config-file `[[permission]]` layers** — loaded in the documented settings layering order
//!    (global → project → nested) as the BASE ruleset, so a project-level `cortex.toml` in the
//!    working dir overrides a conflicting global rule (VAL-PERM-017). When an `--agent` is active
//!    the base is that agent's effective ruleset ([`load_agent_rulesets`] →
//!    [`permissions::AgentRulesets::for_agent`]: the top-level `[[permission]]` rules followed by
//!    the agent's `[[agent.<name>.permission]]` overrides), so a per-agent rule gates the turn
//!    (VAL-PERM-009); with no `--agent` the base is the top-level layered ruleset
//!    ([`load_layered`]) exactly as before.
//! 2. **CLI `--permission-allow`/`--permission-deny` flags** — parsed here and appended ON TOP of
//!    the config base, so a CLI rule overrides a conflicting config rule.
//!
//! The tools already call that engine through `ctx.permits`, but the default headless context
//! attaches none — so a denied `write`/`edit`/`apply_patch`/`bash`/`read` produces no side effect:
//! the deny path returns before any byte is written or any command runs.
//!
//! ## CLI rule spec form
//! Each flag value is `TOOL:GLOB`, split on the first `:`:
//! - `TOOL` selects the gated tool (`read`/`write`/`edit`/`apply_patch`/`glob`/`grep`/`bash`) or
//!   `*` for any tool. An unknown tool is rejected so a mistyped rule never silently fails open.
//! - `GLOB` is matched against the action's subject — a normalized file path for the file tools, or
//!   the parsed command for `bash` (see [`permissions::request_for`]).
//!
//! ## Ordering / precedence
//! The combined ruleset is, in order: the config layers (global → project → nested), then the CLI
//! allow rules, then the CLI deny rules. Under last-matching-rule-wins this means a higher config
//! layer overrides a lower one, any CLI rule overrides a conflicting config rule, and a CLI deny
//! overrides a conflicting CLI allow (deny-wins; fail-closed). When the combined ruleset is empty
//! (no config rules AND no flags) the engine is absent and tools run ungated, preserving the
//! default headless behavior the other M2 tool features rely on.

use std::sync::Arc;

use events::EventBus;
use permissions::{
    ANY_TOOL, Action, Decision, PermissionEngine, PermissionRequest, Responder, Rule, RuleError,
    SavedDecision, SavedDecisions, ScopedDecision, load_agent_rulesets, load_layered, scope_of,
};
use settings::ConfigInputs;

/// Parse the repeatable allow/deny CLI rule flags into ordered permission [`Rule`]s.
///
/// Allow specs become `allow` rules and deny specs become `deny` rules; the allows are ordered
/// before the denies so a CLI deny overrides a conflicting CLI allow under last-matching-rule-wins.
/// Returns an empty vector when neither flag is supplied.
///
/// # Errors
/// Returns a human-readable message when a spec is not `TOOL:GLOB`, names an unknown tool, or has
/// an empty glob.
pub fn rules_from_flags(allow: &[String], deny: &[String]) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::with_capacity(allow.len() + deny.len());
    for spec in allow {
        rules.push(parse_spec(spec, Action::Allow)?);
    }
    for spec in deny {
        rules.push(parse_spec(spec, Action::Deny)?);
    }
    Ok(rules)
}

/// Parse the repeatable `--permission-allow-save` flags into ordered "ask-and-save-always" specs.
///
/// Each spec is a `TOOL:GLOB` (parsed exactly like an allow/deny rule) meaning: when a matching
/// gated action is asked, answer "always allow" AND persist that choice so a LATER run reuses it
/// without re-asking (the headless analog of the TUI's "approve and remember"). The carried
/// [`Action`] is irrelevant (only the tool + pattern matter), so [`Action::Allow`] is used as a
/// neutral placeholder.
///
/// # Errors
/// Returns a human-readable message when a spec is not `TOOL:GLOB`, names an unknown tool, or has
/// an empty glob.
pub fn rules_from_save_flags(specs: &[String]) -> Result<Vec<Rule>, String> {
    specs
        .iter()
        .map(|spec| parse_spec(spec, Action::Allow))
        .collect()
}

/// The headless permission setup for a `cortex run`: the optional engine the tool context consults,
/// the shared saved-decisions store, and whether saved decisions should be persisted after the run.
pub struct HeadlessPermissions {
    /// The engine gating each tool effect, or `None` when the run is ungated (no config rules, no
    /// flags, and no previously saved decisions).
    pub engine: Option<Arc<PermissionEngine>>,
    /// The shared store of saved "always" decisions (seeded from disk, recorded into during the
    /// run); snapshot it to persist new decisions.
    pub saved: Arc<SavedDecisions>,
    /// Whether this run may add saved decisions (a `--permission-allow-save` spec was present), so
    /// the caller should persist [`saved`](HeadlessPermissions::saved) after the turn.
    pub persist: bool,
}

/// Assemble the headless permission engine from every source a `cortex run` honors: the config-file
/// `[[permission]]` layers (scoped to the active agent when one is selected), the CLI allow/deny
/// flags, the previously saved cross-process decisions, and the `--permission-allow-save`
/// "ask-and-remember" specs.
///
/// `agent` is the active `--agent` name, or `None` when no agent flag is present. When `Some(name)`,
/// the BASE ruleset is that agent's effective ruleset ([`load_agent_rulesets`] →
/// [`permissions::AgentRulesets::for_agent`]: the top-level `[[permission]]` base followed by the
/// agent's `[[agent.<name>.permission]]` overrides), so a per-agent rule gates the turn and two
/// agents whose rules differ decide the same action differently (VAL-PERM-009). An agent with no
/// overrides falls back to the top-level base. With `None` the base is the top-level layered ruleset
/// ([`load_layered`]) exactly as before.
///
/// The combined ruleset is, in last-matching-rule-wins order: the (agent-scoped) config base, then
/// the CLI allow rules, then the CLI deny rules, then each `save_specs` pattern as an `ask` rule (so
/// a matching action is surfaced as an ask — recording a `permission.asked` event — and the
/// [`SaveResponder`] answers "always allow", saving the SPEC's precise pattern). The engine also
/// carries:
/// - the `saved` store (consulted BEFORE the ruleset, so a previously saved decision is honored
///   with no new ask — VAL-CROSS-027), seeded from `loaded`;
/// - the event `bus`, so `permission.asked`/`permission.replied` are observable;
/// - the [`SaveResponder`] (only when `save_specs` is non-empty) that turns a matching ask into a
///   saved "always allow".
///
/// Returns an engine whenever any source is non-empty (rules, prior saved decisions, or save
/// specs); with none of them the run stays ungated (`engine: None`), preserving the default the
/// other M2 tool features rely on.
///
/// # Errors
/// Returns [`RuleError`] when a config layer cannot be read/parsed or a config-file rule is
/// malformed.
pub fn prepare_headless_permissions(
    inputs: &ConfigInputs,
    agent: Option<&str>,
    cli_rules: Vec<Rule>,
    save_specs: Vec<Rule>,
    loaded: Vec<SavedDecision>,
    bus: Arc<EventBus>,
) -> Result<HeadlessPermissions, RuleError> {
    // When an agent is active its effective ruleset (base ++ that agent's overrides) is the base, so
    // a per-agent rule gates the turn (VAL-PERM-009); otherwise the top-level layered ruleset is the
    // base exactly as before.
    let mut ruleset = match agent {
        Some(name) => load_agent_rulesets(inputs)?.for_agent(name),
        None => load_layered(inputs)?,
    };
    for rule in cli_rules {
        ruleset.push(rule);
    }
    // Each save spec becomes an `ask` rule so a matching action classifies as `ask` against the
    // SPEC's pattern (not the broad default `*`), making the recorded saved decision precise.
    for spec in &save_specs {
        ruleset.push(Rule::new(
            spec.tool.clone(),
            spec.pattern.clone(),
            Action::Ask,
        ));
    }

    let saved = Arc::new(SavedDecisions::from_decisions(loaded));
    let persist = !save_specs.is_empty();

    // Attach an engine when anything would gate or honor an action: a configured/CLI rule, a
    // previously saved decision to honor, or a save spec to record this run.
    if ruleset.is_empty() && saved.is_empty() {
        return Ok(HeadlessPermissions {
            engine: None,
            saved,
            persist,
        });
    }

    let mut engine = PermissionEngine::new(ruleset)
        .with_saved_decisions(saved.clone())
        .with_bus(bus);
    if !save_specs.is_empty() {
        engine = engine.with_responder(Arc::new(SaveResponder::new(save_specs)));
    }
    Ok(HeadlessPermissions {
        engine: Some(Arc::new(engine)),
        saved,
        persist,
    })
}

/// A headless [`Responder`] that answers asks for `--permission-allow-save` patterns with a saved
/// "always allow", and fails closed (a one-shot deny) for any other ask.
///
/// This is the non-interactive analog of the TUI's "approve and remember": there is no human to
/// prompt, so a matching ask is auto-approved AND remembered (the engine records it into the saved
/// store, which the caller persists), while a non-matching ask still denies in bounded time (no
/// hang, no silent allow), preserving the headless fail-closed guarantee.
struct SaveResponder {
    specs: Vec<Rule>,
}

impl SaveResponder {
    fn new(specs: Vec<Rule>) -> Self {
        Self { specs }
    }

    /// Whether `request` matches one of the save specs (so it should be allowed-and-remembered).
    fn matches(&self, request: &PermissionRequest) -> bool {
        self.specs
            .iter()
            .any(|spec| spec.matches(&request.tool, &request.subject))
    }
}

impl Responder for SaveResponder {
    fn respond(&self, request: &PermissionRequest) -> Decision {
        self.respond_scoped(request).decision
    }

    fn respond_scoped(&self, request: &PermissionRequest) -> ScopedDecision {
        if self.matches(request) {
            ScopedDecision::always(Decision::Allow)
        } else {
            ScopedDecision::once(Decision::Deny)
        }
    }
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
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A self-cleaning scratch tree with an isolated HOME (the global-config root and the project
    /// walk boundary) and a project directory inside it that doubles as the run's working dir.
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = format!(
                "cortex-cli-perm-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let root = std::env::temp_dir().join(unique);
            fs::create_dir_all(root.join("home/proj")).expect("create project tree");
            Self { root }
        }

        fn home(&self) -> PathBuf {
            self.root.join("home")
        }

        fn proj(&self) -> PathBuf {
            self.root.join("home/proj")
        }

        fn write(&self, rel: &str, contents: &str) {
            let path = self.root.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent dir");
            }
            fs::write(path, contents).expect("write config file");
        }

        fn write_global(&self, contents: &str) {
            self.write("home/.config/cortex/config.toml", contents);
        }

        fn write_project(&self, contents: &str) {
            self.write("home/proj/cortex.toml", contents);
        }

        fn inputs(&self) -> ConfigInputs {
            ConfigInputs {
                home: Some(self.home()),
                xdg_config_home: None,
                cwd: self.proj(),
                env: Vec::new(),
                cli_overrides: BTreeMap::new(),
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const ALLOW_RM: &str =
        "[[permission]]\ntool = \"bash\"\npattern = \"rm *\"\naction = \"allow\"\n";
    const DENY_RM: &str =
        "[[permission]]\ntool = \"bash\"\npattern = \"rm *\"\naction = \"deny\"\n";

    fn flags(specs: &[&str]) -> Vec<String> {
        specs.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Build the headless engine option from config layers + CLI rules alone (no save specs, no
    /// prior saved decisions), with no active agent — as the layered-precedence tests exercise it.
    fn engine_for(inputs: &ConfigInputs, cli_rules: Vec<Rule>) -> Option<Arc<PermissionEngine>> {
        engine_for_agent(inputs, None, cli_rules)
    }

    /// Build the headless engine option scoped to `agent` (the active `--agent` name, or `None` for
    /// the top-level layered base) from config layers + CLI rules alone.
    fn engine_for_agent(
        inputs: &ConfigInputs,
        agent: Option<&str>,
        cli_rules: Vec<Rule>,
    ) -> Option<Arc<PermissionEngine>> {
        prepare_headless_permissions(
            inputs,
            agent,
            cli_rules,
            Vec::new(),
            Vec::new(),
            Arc::new(EventBus::new()),
        )
        .expect("prepare permissions")
        .engine
    }

    // With no config files AND no flags the combined ruleset is empty, so no engine is attached and
    // tools run ungated (the default the other M2 tool features rely on).
    #[test]
    fn no_config_and_no_flags_yield_no_engine() {
        let scratch = Scratch::new("none");
        assert!(engine_for(&scratch.inputs(), Vec::new()).is_none());
    }

    // VAL-PERM-017: a project-layer rule deterministically overrides a conflicting global-layer one.
    // Global allows `bash rm *`; the project (working-dir) config denies it, so the combined engine
    // denies `rm x` (project layer wins).
    #[test]
    fn project_layer_overrides_global_layer() {
        let scratch = Scratch::new("layers");
        scratch.write_global(ALLOW_RM);
        scratch.write_project(DENY_RM);

        let engine = engine_for(&scratch.inputs(), Vec::new())
            .expect("rules present so an engine is attached");
        assert_eq!(engine.classify("bash", "rm x"), Action::Deny);
    }

    // VAL-PERM-017: removing the project `deny` (leaving only the global `allow`) restores the
    // allow, proving the cross-layer combination is deterministic and no layer is silently dropped.
    #[test]
    fn removing_the_project_layer_restores_the_global_outcome() {
        let scratch = Scratch::new("restore");
        scratch.write_global(ALLOW_RM);

        let engine =
            engine_for(&scratch.inputs(), Vec::new()).expect("the global rule attaches an engine");
        assert_eq!(engine.classify("bash", "rm x"), Action::Allow);
    }

    // A CLI rule is appended ON TOP of the config layers, so a `--permission-deny` overrides a
    // config-file `allow` for the same action (the CLI layer wins).
    #[test]
    fn cli_rule_overrides_a_config_allow() {
        let scratch = Scratch::new("cli-over-config");
        scratch.write_global(ALLOW_RM);

        let cli_rules = rules_from_flags(&[], &flags(&["bash:rm *"])).expect("parse cli rules");
        let engine = engine_for(&scratch.inputs(), cli_rules).expect("rules present");
        assert_eq!(
            engine.classify("bash", "rm x"),
            Action::Deny,
            "the appended CLI deny overrides the config-file allow"
        );
    }

    // With no config layers, the CLI flags alone build the engine and a CLI deny overrides a CLI
    // allow (allows are ordered before denies).
    #[test]
    fn cli_flags_alone_build_the_engine_with_deny_winning() {
        let scratch = Scratch::new("cli-only");
        let cli_rules =
            rules_from_flags(&flags(&["*:**"]), &flags(&["write:secrets/**"])).expect("parse");
        let engine = engine_for(&scratch.inputs(), cli_rules)
            .expect("flags present so an engine is attached");
        assert_eq!(engine.classify("write", "secrets/k.txt"), Action::Deny);
        assert_eq!(engine.classify("write", "src/k.txt"), Action::Allow);
        assert_eq!(engine.classify("read", "secrets/k.txt"), Action::Allow);
    }

    /// A project config declaring per-agent `[[agent.<name>.permission]]` overrides: agent `plan`
    /// denies `write **` while agent `build` allows it.
    const AGENT_WRITE_RULES: &str = "[[agent.plan.permission]]\ntool = \"write\"\npattern = \"**\"\naction = \"deny\"\n\n\
         [[agent.build.permission]]\ntool = \"write\"\npattern = \"**\"\naction = \"allow\"\n";

    // VAL-PERM-009: with agent `plan` denying `write **` and agent `build` allowing it in the SAME
    // config, the active agent's ruleset governs the turn — the identical write is denied under
    // `plan` and allowed under `build`.
    #[test]
    fn agent_ruleset_decides_the_same_write_differently_per_agent() {
        let scratch = Scratch::new("agent-perm009");
        scratch.write_project(AGENT_WRITE_RULES);

        let plan = engine_for_agent(&scratch.inputs(), Some("plan"), Vec::new())
            .expect("plan's deny attaches an engine");
        assert_eq!(
            plan.classify("write", "src/a.txt"),
            Action::Deny,
            "agent `plan` denies the write"
        );

        let build = engine_for_agent(&scratch.inputs(), Some("build"), Vec::new())
            .expect("build's allow attaches an engine");
        assert_eq!(
            build.classify("write", "src/a.txt"),
            Action::Allow,
            "agent `build` allows the same write on the same config"
        );
    }

    // An agent with no `[[agent.<name>.permission]]` override falls back to the top-level
    // `[[permission]]` base, while an agent that DOES override wins under last-matching-rule-wins.
    #[test]
    fn agent_without_overrides_falls_back_to_the_top_level_base() {
        let scratch = Scratch::new("agent-fallback");
        // Top-level base allows `write **`; only agent `plan` overrides it to deny.
        scratch.write_project(
            "[[permission]]\ntool = \"write\"\npattern = \"**\"\naction = \"allow\"\n\n\
             [[agent.plan.permission]]\ntool = \"write\"\npattern = \"**\"\naction = \"deny\"\n",
        );

        let general = engine_for_agent(&scratch.inputs(), Some("general"), Vec::new())
            .expect("the base rule attaches an engine");
        assert_eq!(
            general.classify("write", "src/a.txt"),
            Action::Allow,
            "an agent with no override is governed by the top-level base"
        );

        let plan = engine_for_agent(&scratch.inputs(), Some("plan"), Vec::new())
            .expect("the agent override attaches an engine");
        assert_eq!(
            plan.classify("write", "src/a.txt"),
            Action::Deny,
            "agent `plan`'s override wins over the base"
        );
    }

    // A CLI `--permission-deny` is appended ON TOP of the agent ruleset, so a CLI deny overrides an
    // agent's allow for the same action (the CLI layer wins last).
    #[test]
    fn cli_deny_overrides_an_agent_allow() {
        let scratch = Scratch::new("agent-cli-over");
        scratch.write_project(AGENT_WRITE_RULES);

        let cli_rules = rules_from_flags(&[], &flags(&["write:**"])).expect("parse cli rules");
        let build = engine_for_agent(&scratch.inputs(), Some("build"), cli_rules)
            .expect("rules present so an engine is attached");
        assert_eq!(
            build.classify("write", "src/a.txt"),
            Action::Deny,
            "the appended CLI deny overrides agent `build`'s allow"
        );
    }

    // With no `--agent` active the base is the top-level layered ruleset and the per-agent
    // `[[agent.<name>.permission]]` rules are ignored, so behavior is unchanged: a config with ONLY
    // agent overrides (no top-level `[[permission]]`) yields no rules and stays ungated.
    #[test]
    fn no_agent_ignores_agent_config_and_uses_the_top_level_base() {
        let scratch = Scratch::new("agent-none");
        scratch.write_project(AGENT_WRITE_RULES);
        assert!(
            engine_for(&scratch.inputs(), Vec::new()).is_none(),
            "without --agent the agent-only config contributes no top-level rules, so the run stays \
             ungated exactly as before"
        );

        // And when a top-level base IS present, the no-agent run is governed by it alone (the agent
        // deny is not applied).
        let scratch = Scratch::new("agent-none-base");
        scratch.write_project(
            "[[permission]]\ntool = \"write\"\npattern = \"**\"\naction = \"allow\"\n\n\
             [[agent.plan.permission]]\ntool = \"write\"\npattern = \"**\"\naction = \"deny\"\n",
        );
        let engine = engine_for(&scratch.inputs(), Vec::new())
            .expect("the top-level base attaches an engine");
        assert_eq!(
            engine.classify("write", "src/a.txt"),
            Action::Allow,
            "with no --agent the top-level base governs and the agent override is ignored"
        );
    }

    // The combination is deterministic: loading the same fixtures twice yields the same decision.
    #[test]
    fn combination_is_stable_across_repeated_loads() {
        let scratch = Scratch::new("stable");
        scratch.write_global(ALLOW_RM);
        scratch.write_project(DENY_RM);

        let first = engine_for(&scratch.inputs(), Vec::new()).expect("engine");
        let second = engine_for(&scratch.inputs(), Vec::new()).expect("engine");
        assert_eq!(first.classify("bash", "rm x"), Action::Deny);
        assert_eq!(second.classify("bash", "rm x"), Action::Deny);
    }

    // A malformed config-file permission rule surfaces a `RuleError` (mapped to the config exit
    // code by the caller), naming the offending config file.
    #[test]
    fn a_malformed_config_rule_is_a_rule_error() {
        let scratch = Scratch::new("bad-config");
        scratch.write_project("[[permission]]\ntool = \"bash\"\n");
        let err = match prepare_headless_permissions(
            &scratch.inputs(),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Arc::new(EventBus::new()),
        ) {
            Ok(_) => panic!("a malformed config rule must error"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("action"), "{err}");
    }

    #[test]
    fn no_flags_yield_no_rules() {
        assert!(rules_from_flags(&[], &[]).expect("ok").is_empty());
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
    fn allows_are_ordered_before_denies() {
        let rules = rules_from_flags(&["*:**".to_owned()], &["write:secrets/**".to_owned()])
            .expect("parse");
        assert_eq!(
            rules,
            vec![
                Rule::new("*", "**", Action::Allow),
                Rule::new("write", "secrets/**", Action::Deny),
            ],
            "allow rules precede deny rules so a CLI deny is evaluated last and wins"
        );
    }

    #[test]
    fn a_malformed_spec_aborts_the_parse() {
        let err = match rules_from_flags(&["*:**".to_owned()], &["not-a-rule".to_owned()]) {
            Ok(_) => panic!("a malformed spec must error"),
            Err(message) => message,
        };
        assert!(err.contains("TOOL:GLOB"), "{err}");
    }

    #[test]
    fn save_flags_parse_like_allow_rules() {
        let specs = rules_from_save_flags(&flags(&["bash:echo *", "write:src/**"])).expect("parse");
        assert_eq!(
            specs,
            vec![
                Rule::new("bash", "echo *", Action::Allow),
                Rule::new("write", "src/**", Action::Allow),
            ]
        );
        // A malformed save spec is rejected, just like an allow/deny spec.
        assert!(rules_from_save_flags(&flags(&["not-a-rule"])).is_err());
    }

    // The save responder remembers (always-allows) a matching ask and fails closed (a one-shot
    // deny, never a silent allow) for any non-matching ask.
    #[test]
    fn save_responder_remembers_matches_and_fails_closed_otherwise() {
        let responder =
            SaveResponder::new(rules_from_save_flags(&flags(&["bash:echo *"])).expect("specs"));

        let matching = responder.respond_scoped(&PermissionRequest::new("bash", "echo hi"));
        assert_eq!(matching, ScopedDecision::always(Decision::Allow));

        let other = responder.respond_scoped(&PermissionRequest::new("bash", "rm victim"));
        assert_eq!(other, ScopedDecision::once(Decision::Deny));
    }

    /// An event-bus subscriber that records the kind of every event it observes, so a test can
    /// assert that a `permission.asked` was (or was not) emitted.
    #[derive(Default)]
    struct EventCollector {
        kinds: std::sync::Mutex<Vec<String>>,
    }

    impl EventCollector {
        fn saw(&self, kind: &str) -> bool {
            self.kinds
                .lock()
                .expect("collector lock")
                .iter()
                .any(|seen| seen == kind)
        }
    }

    impl events::Subscriber for EventCollector {
        fn on_event(&self, event: &events::Event) {
            self.kinds
                .lock()
                .expect("collector lock")
                .push(event.kind().to_owned());
        }
    }

    // VAL-CROSS-027 (the durable, cross-process core): run 1 surfaces an ask for `bash echo *`,
    // answers it "always allow", and that decision — once snapshotted to disk and reloaded — is
    // honored by a fully separate run 2 with NO new `permission.asked`, and the action is allowed.
    #[test]
    fn saved_always_allow_is_honored_across_a_persist_reload_without_reasking() {
        let scratch = Scratch::new("cross");
        let store = scratch.root.join("permissions.json");

        // --- Run 1: ask + save always-allow, then persist the snapshot to disk. ---
        let bus1 = Arc::new(EventBus::new());
        let collector1 = Arc::new(EventCollector::default());
        bus1.subscribe(collector1.clone());
        let prepared1 = prepare_headless_permissions(
            &scratch.inputs(),
            None,
            Vec::new(),
            rules_from_save_flags(&flags(&["bash:echo *"])).expect("save specs"),
            Vec::new(),
            bus1,
        )
        .expect("prepare run 1");
        assert!(
            prepared1.persist,
            "a save spec marks the run for persistence"
        );
        let engine1 = prepared1
            .engine
            .clone()
            .expect("engine attached for save specs");

        let outcome1 = engine1.evaluate(&permissions::request_for("bash", "echo hi"));
        assert_eq!(outcome1, permissions::Outcome::Allowed);
        assert!(collector1.saw("permission.asked"), "run 1 surfaces the ask");

        let snapshot = prepared1.saved.snapshot();
        assert_eq!(
            snapshot,
            vec![SavedDecision::new("bash", "echo *", Decision::Allow)],
            "the saved decision carries the SPEC's precise pattern, not the broad default"
        );
        crate::permission_store::write(&store, &snapshot).expect("persist run 1");

        // --- Run 2: a fully separate process loads the decision and honors it with no re-ask. ---
        let loaded = crate::permission_store::read(&store);
        assert_eq!(loaded, snapshot, "the decision survives the persist/reload");

        let bus2 = Arc::new(EventBus::new());
        let collector2 = Arc::new(EventCollector::default());
        bus2.subscribe(collector2.clone());
        let prepared2 = prepare_headless_permissions(
            &scratch.inputs(),
            None,
            Vec::new(),
            Vec::new(),
            loaded,
            bus2,
        )
        .expect("prepare run 2");
        assert!(
            !prepared2.persist,
            "run 2 records nothing new, so it does not rewrite the store"
        );
        let engine2 = prepared2
            .engine
            .expect("engine attached to honor the saved decision");

        let outcome2 = engine2.evaluate(&permissions::request_for("bash", "echo persisted-ok"));
        assert_eq!(
            outcome2,
            permissions::Outcome::Allowed,
            "the saved always-allow lets the matching action proceed"
        );
        assert!(
            !collector2.saw("permission.asked"),
            "run 2 must honor the saved decision WITHOUT re-asking (VAL-CROSS-027)"
        );
    }

    // Without a saved decision, the same `bash echo` action would NOT be auto-allowed in a gated
    // run: it defaults to ask and fails closed (denied) in headless, proving the saved decision is
    // what makes run 2 succeed (the difference the persistence buys).
    #[test]
    fn the_same_action_fails_closed_without_the_saved_decision() {
        let scratch = Scratch::new("no-saved");
        // An engine is present (a benign config rule), but there is no saved decision for `echo`.
        scratch.write_project(ALLOW_RM);
        let engine =
            engine_for(&scratch.inputs(), Vec::new()).expect("a config rule attaches an engine");
        let outcome = engine.evaluate(&permissions::request_for("bash", "echo hi"));
        assert_eq!(
            outcome,
            permissions::Outcome::Denied,
            "with no saved decision the default ask fails closed in headless"
        );
    }
}
