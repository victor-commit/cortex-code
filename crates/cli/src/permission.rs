//! Build the headless permission engine from `cortex run`'s config-file permission layers and its
//! repeatable rule flags.
//!
//! `cortex run` honors two sources of permission rules, combined into one ordered ruleset evaluated
//! last-matching-rule-wins by the [`PermissionEngine`] the headless [`tools::ToolContext`] consults
//! before every gated tool effect (architecture §3.4 / §4):
//!
//! 1. **Config-file `[[permission]]` layers** — loaded via [`load_layered`] in the documented
//!    settings layering order (global → project → nested) as the BASE ruleset, so a project-level
//!    `cortex.toml` in the working dir overrides a conflicting global rule (VAL-PERM-017).
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

use permissions::{ANY_TOOL, Action, PermissionEngine, Rule, RuleError, load_layered, scope_of};
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

/// Build the optional headless [`PermissionEngine`] from the config-file permission layers plus the
/// already-parsed CLI rules.
///
/// The config-file `[[permission]]` rules are loaded via [`load_layered`] in ascending precedence
/// order (global → project → nested) as the BASE ruleset, then `cli_rules` are appended ON TOP so a
/// CLI rule overrides a conflicting config rule under last-matching-rule-wins. Returns `Ok(None)`
/// when the combined ruleset is empty (no config rules AND no flags), preserving the ungated default
/// headless behavior the other M2 tool features rely on.
///
/// # Errors
/// Returns [`RuleError`] when a config layer cannot be read/parsed or a config-file rule is
/// malformed.
pub fn engine_from_layers_and_flags(
    inputs: &ConfigInputs,
    cli_rules: Vec<Rule>,
) -> Result<Option<Arc<PermissionEngine>>, RuleError> {
    let mut ruleset = load_layered(inputs)?;
    for rule in cli_rules {
        ruleset.push(rule);
    }
    if ruleset.is_empty() {
        return Ok(None);
    }
    Ok(Some(Arc::new(PermissionEngine::new(ruleset))))
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

    // With no config files AND no flags the combined ruleset is empty, so no engine is attached and
    // tools run ungated (the default the other M2 tool features rely on).
    #[test]
    fn no_config_and_no_flags_yield_no_engine() {
        let scratch = Scratch::new("none");
        let engine = engine_from_layers_and_flags(&scratch.inputs(), Vec::new()).expect("ok");
        assert!(engine.is_none());
    }

    // VAL-PERM-017: a project-layer rule deterministically overrides a conflicting global-layer one.
    // Global allows `bash rm *`; the project (working-dir) config denies it, so the combined engine
    // denies `rm x` (project layer wins).
    #[test]
    fn project_layer_overrides_global_layer() {
        let scratch = Scratch::new("layers");
        scratch.write_global(ALLOW_RM);
        scratch.write_project(DENY_RM);

        let engine = engine_from_layers_and_flags(&scratch.inputs(), Vec::new())
            .expect("ok")
            .expect("rules present so an engine is attached");
        assert_eq!(engine.classify("bash", "rm x"), Action::Deny);
    }

    // VAL-PERM-017: removing the project `deny` (leaving only the global `allow`) restores the
    // allow, proving the cross-layer combination is deterministic and no layer is silently dropped.
    #[test]
    fn removing_the_project_layer_restores_the_global_outcome() {
        let scratch = Scratch::new("restore");
        scratch.write_global(ALLOW_RM);

        let engine = engine_from_layers_and_flags(&scratch.inputs(), Vec::new())
            .expect("ok")
            .expect("the global rule attaches an engine");
        assert_eq!(engine.classify("bash", "rm x"), Action::Allow);
    }

    // A CLI rule is appended ON TOP of the config layers, so a `--permission-deny` overrides a
    // config-file `allow` for the same action (the CLI layer wins).
    #[test]
    fn cli_rule_overrides_a_config_allow() {
        let scratch = Scratch::new("cli-over-config");
        scratch.write_global(ALLOW_RM);

        let cli_rules = rules_from_flags(&[], &flags(&["bash:rm *"])).expect("parse cli rules");
        let engine = engine_from_layers_and_flags(&scratch.inputs(), cli_rules)
            .expect("ok")
            .expect("rules present");
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
        let engine = engine_from_layers_and_flags(&scratch.inputs(), cli_rules)
            .expect("ok")
            .expect("flags present so an engine is attached");
        assert_eq!(engine.classify("write", "secrets/k.txt"), Action::Deny);
        assert_eq!(engine.classify("write", "src/k.txt"), Action::Allow);
        assert_eq!(engine.classify("read", "secrets/k.txt"), Action::Allow);
    }

    // The combination is deterministic: loading the same fixtures twice yields the same decision.
    #[test]
    fn combination_is_stable_across_repeated_loads() {
        let scratch = Scratch::new("stable");
        scratch.write_global(ALLOW_RM);
        scratch.write_project(DENY_RM);

        let first = engine_from_layers_and_flags(&scratch.inputs(), Vec::new())
            .expect("ok")
            .expect("engine");
        let second = engine_from_layers_and_flags(&scratch.inputs(), Vec::new())
            .expect("ok")
            .expect("engine");
        assert_eq!(first.classify("bash", "rm x"), Action::Deny);
        assert_eq!(second.classify("bash", "rm x"), Action::Deny);
    }

    // A malformed config-file permission rule surfaces a `RuleError` (mapped to the config exit
    // code by the caller), naming the offending config file.
    #[test]
    fn a_malformed_config_rule_is_a_rule_error() {
        let scratch = Scratch::new("bad-config");
        scratch.write_project("[[permission]]\ntool = \"bash\"\n");
        let err = match engine_from_layers_and_flags(&scratch.inputs(), Vec::new()) {
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
}
