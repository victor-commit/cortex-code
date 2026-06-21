//! Custom markdown agent discovery and parsing.
//!
//! A custom agent is a markdown file with YAML frontmatter followed by the system prompt body:
//!
//! ```text
//! ---
//! name: pirate
//! description: Answers like a pirate
//! mode: primary
//! model: google/gemini-2.5-flash
//! permission:
//!   - tool: write
//!     pattern: "**"
//!     action: deny
//! ---
//! Always answer in pirate-speak.
//! ```
//!
//! The parser intentionally supports the small frontmatter subset Cortex Code needs (scalars and
//! a list of permission rule tables) without accepting arbitrary YAML constructs. Bad files are
//! skipped with diagnostics, so one malformed agent cannot poison the registry.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use engine::Agent;
use permissions::{Action, Rule};
use settings::{ConfigInputs, EffectiveConfig};
use toml::Value;

/// Where a custom agent may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAvailability {
    /// Selectable as the primary/top-level `--agent`, not advertised as a subagent target.
    Primary,
    /// Advertised only as a `task` subagent target.
    Subagent,
    /// Selectable as a primary agent and as a subagent target.
    All,
}

impl AgentAvailability {
    /// Whether this agent can be selected by `cortex run --agent`.
    #[must_use]
    pub const fn selectable(self) -> bool {
        matches!(self, Self::Primary | Self::All)
    }

    /// Whether this agent can be spawned through the `task` tool.
    #[must_use]
    pub const fn spawnable(self) -> bool {
        matches!(self, Self::Subagent | Self::All)
    }
}

/// A valid custom markdown agent discovered from disk.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomAgent {
    /// Canonical selectable name.
    pub name: String,
    /// Discovery-list description.
    pub description: String,
    /// Raw mode field from frontmatter.
    pub mode: String,
    /// Whether the mode makes this agent primary-selectable, subagent-spawnable, or both.
    pub availability: AgentAvailability,
    /// Built-in mode fragment this custom agent composes with.
    pub base_agent: Agent,
    /// Optional model id this agent selects when `--model` is absent.
    pub model: Option<String>,
    /// Optional sampling temperature for this agent's turns.
    pub temperature: Option<f32>,
    /// Frontmatter permission rules.
    pub permissions: Vec<Rule>,
    /// Markdown body, used as the custom system-prompt fragment.
    pub prompt: String,
    /// Source file path for diagnostics and listings.
    pub source: PathBuf,
}

/// Result of discovering custom agents.
#[derive(Debug, Default)]
pub struct Discovery {
    pub agents: BTreeMap<String, CustomAgent>,
    pub warnings: Vec<String>,
}

/// A resolved active agent, either one of the built-ins or a discovered custom definition.
#[derive(Debug, Clone, PartialEq)]
pub enum ActiveAgent {
    Builtin(Agent),
    Custom(CustomAgent),
}

impl ActiveAgent {
    /// Name reported in machine output and accepted by `--agent`.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Builtin(agent) => agent.name(),
            Self::Custom(agent) => &agent.name,
        }
    }

    /// Built-in mode fragment to compose into the system context.
    #[must_use]
    pub fn base_agent(&self) -> Agent {
        match self {
            Self::Builtin(agent) => *agent,
            Self::Custom(agent) => agent.base_agent,
        }
    }

    /// Custom prompt fragment, if this is a custom markdown agent.
    #[must_use]
    pub fn custom_prompt(&self) -> Option<&str> {
        match self {
            Self::Builtin(_) => None,
            Self::Custom(agent) => Some(agent.prompt.as_str()),
        }
    }

    /// Agent-specified model override.
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Builtin(_) => None,
            Self::Custom(agent) => agent.model.as_deref(),
        }
    }

    /// Agent-specified temperature override.
    #[must_use]
    pub fn temperature(&self) -> Option<f32> {
        match self {
            Self::Builtin(_) => None,
            Self::Custom(agent) => agent.temperature,
        }
    }

    /// Agent-specified permission rules.
    #[must_use]
    pub fn permissions(&self) -> &[Rule] {
        match self {
            Self::Builtin(_) => &[],
            Self::Custom(agent) => &agent.permissions,
        }
    }
}

/// Unknown-agent diagnostic with an availability hint spanning built-ins and custom agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAgent {
    requested: String,
    available: String,
}

impl UnknownAgent {
    fn new(requested: &str, available: String) -> Self {
        Self {
            requested: requested.to_owned(),
            available,
        }
    }
}

impl std::fmt::Display for UnknownAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown agent '{}': available agents are {}",
            self.requested, self.available
        )
    }
}

impl std::error::Error for UnknownAgent {}

/// Discover custom markdown agents from configured and conventional agent directories.
#[must_use]
pub fn discover(inputs: &ConfigInputs) -> Discovery {
    let mut discovery = Discovery::default();
    let dirs = agent_dirs(inputs, &mut discovery.warnings);
    let mut names = builtin_name_set();

    for dir in dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                continue;
            }
            let display = path.display().to_string();
            match fs::read_to_string(&path) {
                Ok(contents) => match parse_custom_agent(&contents, path.clone()) {
                    Ok(agent) => {
                        let key = normalize_name(&agent.name);
                        if names.contains(&key) {
                            discovery.warnings.push(format!(
                                "skipping custom agent {}: duplicate agent name '{}'",
                                display, agent.name
                            ));
                        } else {
                            names.insert(key.clone());
                            discovery.agents.insert(key, agent);
                        }
                    }
                    Err(err) => discovery
                        .warnings
                        .push(format!("skipping custom agent {display}: {err}")),
                },
                Err(err) => discovery
                    .warnings
                    .push(format!("skipping custom agent {display}: {err}")),
            }
        }
    }

    discovery
}

/// Resolve an optional agent name against built-ins and custom discoveries.
pub fn resolve(name: Option<&str>, discovery: &Discovery) -> Result<ActiveAgent, UnknownAgent> {
    match name {
        None => Ok(ActiveAgent::Builtin(Agent::default())),
        Some(name) => {
            let normalized = normalize_name(name);
            if let Ok(agent) = Agent::from_name(name) {
                return Ok(ActiveAgent::Builtin(agent));
            }
            if let Some(agent) = discovery
                .agents
                .get(&normalized)
                .filter(|agent| agent.availability.selectable())
            {
                return Ok(ActiveAgent::Custom(agent.clone()));
            }
            Err(UnknownAgent::new(name, available_names(discovery)))
        }
    }
}

/// Every available agent name in stable discovery order.
#[must_use]
pub fn available_names(discovery: &Discovery) -> String {
    let mut names = Agent::ALL
        .iter()
        .map(|agent| agent.name().to_owned())
        .collect::<Vec<_>>();
    names.extend(
        discovery
            .agents
            .values()
            .filter(|agent| agent.availability.selectable())
            .map(|agent| agent.name.clone()),
    );
    names.join(", ")
}

/// Default project-local directory used by `cortex generate`.
#[must_use]
pub fn default_project_agents_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".cortex")
        .join("agents")
}

/// Render a custom agent markdown file with quoted YAML scalars.
#[must_use]
pub fn render_agent_markdown(
    name: &str,
    description: &str,
    mode: &str,
    model: Option<&str>,
    prompt: &str,
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("name: ");
    out.push_str(&yaml_quote(name));
    out.push('\n');
    out.push_str("description: ");
    out.push_str(&yaml_quote(description));
    out.push('\n');
    out.push_str("mode: ");
    out.push_str(&yaml_quote(mode));
    out.push('\n');
    if let Some(model) = model {
        out.push_str("model: ");
        out.push_str(&yaml_quote(model));
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(prompt.trim());
    out.push('\n');
    out
}

/// Convert arbitrary text into a safe agent identifier.
#[must_use]
pub fn sanitize_identifier(text: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if (ch == '-' || ch == '_' || ch.is_whitespace()) && !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "generated-agent".to_owned()
    } else {
        out
    }
}

/// Parse one markdown document into a [`CustomAgent`].
pub fn parse_custom_agent(contents: &str, source: PathBuf) -> Result<CustomAgent, String> {
    let (frontmatter, body) = split_frontmatter(contents)?;
    let parsed = parse_frontmatter(frontmatter)?;

    let name = required_field(&parsed.fields, "name")?;
    let name = sanitize_name(name)?;
    let description = required_field(&parsed.fields, "description")?.to_owned();
    let mode = required_field(&parsed.fields, "mode")?.to_owned();
    let parsed_mode = parse_mode(&mode)?;
    let prompt = body.trim();
    if prompt.is_empty() {
        return Err("body/system prompt is empty".to_owned());
    }

    let model = parsed
        .fields
        .get("model")
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty());
    let temperature = parsed
        .fields
        .get("temperature")
        .map(|raw| {
            raw.trim()
                .parse::<f32>()
                .map_err(|_| "temperature must be a number".to_owned())
        })
        .transpose()?;

    Ok(CustomAgent {
        name,
        description,
        availability: parsed_mode.availability,
        base_agent: parsed_mode.base_agent,
        mode,
        model,
        temperature,
        permissions: parsed.permissions,
        prompt: prompt.to_owned(),
        source,
    })
}

fn split_frontmatter(contents: &str) -> Result<(&str, &str), String> {
    let Some(rest) = contents.strip_prefix("---") else {
        return Err("missing YAML frontmatter".to_owned());
    };
    let rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"));
    let Some(rest) = rest else {
        return Err("missing YAML frontmatter body".to_owned());
    };
    let marker = "\n---";
    let Some(end) = rest.find(marker) else {
        return Err("unterminated YAML frontmatter".to_owned());
    };
    let frontmatter = &rest[..end];
    let after_marker = &rest[end + marker.len()..];
    let body = after_marker
        .strip_prefix('\n')
        .or_else(|| after_marker.strip_prefix("\r\n"))
        .unwrap_or(after_marker);
    Ok((frontmatter, body))
}

#[derive(Debug, Default)]
struct ParsedFrontmatter {
    fields: BTreeMap<String, String>,
    permissions: Vec<Rule>,
}

#[derive(Debug, Default)]
struct RawPermission {
    tool: Option<String>,
    pattern: Option<String>,
    action: Option<String>,
}

fn parse_frontmatter(frontmatter: &str) -> Result<ParsedFrontmatter, String> {
    let mut parsed = ParsedFrontmatter::default();
    let mut in_permissions = false;
    let mut current_permission: Option<RawPermission> = None;

    for (idx, raw) in frontmatter.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }

        if in_permissions {
            if let Some(rest) = line
                .strip_prefix("  - ")
                .or_else(|| line.strip_prefix("- "))
            {
                finish_permission(&mut parsed.permissions, current_permission.take(), line_no)?;
                let mut next = RawPermission::default();
                if !rest.trim().is_empty() {
                    parse_permission_field(rest.trim(), &mut next, line_no)?;
                }
                current_permission = Some(next);
                continue;
            }
            if line.starts_with("    ") || line.starts_with("  ") {
                let current = current_permission.as_mut().ok_or_else(|| {
                    format!("line {line_no}: permission field appears before a list item")
                })?;
                parse_permission_field(line.trim(), current, line_no)?;
                continue;
            }
            finish_permission(&mut parsed.permissions, current_permission.take(), line_no)?;
            in_permissions = false;
        }

        let (key, value) = parse_key_value(line, line_no)?;
        match key.as_str() {
            "permission" | "permissions" => {
                if value.trim().is_empty() {
                    in_permissions = true;
                } else if value.trim() == "[]" {
                    // Explicit empty list.
                } else {
                    return Err(format!(
                        "line {line_no}: permission must be a list of rule tables"
                    ));
                }
            }
            _ => {
                let scalar = parse_scalar(&value, line_no)?;
                parsed.fields.insert(key, scalar);
            }
        }
    }

    if in_permissions {
        finish_permission(&mut parsed.permissions, current_permission.take(), 0)?;
    }
    Ok(parsed)
}

fn parse_key_value(line: &str, line_no: usize) -> Result<(String, String), String> {
    if line.chars().next().is_some_and(char::is_whitespace) {
        return Err(format!("line {line_no}: unexpected indentation"));
    }
    let (key, value) = line
        .split_once(':')
        .ok_or_else(|| format!("line {line_no}: expected `key: value`"))?;
    let key = key.trim();
    if key.is_empty() {
        return Err(format!("line {line_no}: empty key"));
    }
    Ok((key.to_ascii_lowercase(), value.trim().to_owned()))
}

fn parse_permission_field(
    line: &str,
    permission: &mut RawPermission,
    line_no: usize,
) -> Result<(), String> {
    let (key, value) = line
        .split_once(':')
        .ok_or_else(|| format!("line {line_no}: expected permission `key: value`"))?;
    let value = parse_scalar(value.trim(), line_no)?;
    match key.trim().to_ascii_lowercase().as_str() {
        "tool" => permission.tool = Some(value),
        "pattern" => permission.pattern = Some(value),
        "action" => permission.action = Some(value),
        other => {
            return Err(format!(
                "line {line_no}: unknown permission field `{other}`"
            ));
        }
    }
    Ok(())
}

fn finish_permission(
    rules: &mut Vec<Rule>,
    raw: Option<RawPermission>,
    line_no: usize,
) -> Result<(), String> {
    let Some(raw) = raw else {
        return Ok(());
    };
    let action_raw = raw.action.ok_or_else(|| {
        if line_no == 0 {
            "permission rule is missing action".to_owned()
        } else {
            format!("line {line_no}: permission rule is missing action")
        }
    })?;
    let action = Action::parse(&action_raw).ok_or_else(|| {
        format!("invalid permission action `{action_raw}` (expected allow, ask, or deny)")
    })?;
    rules.push(Rule::new(
        raw.tool.unwrap_or_else(|| "*".to_owned()),
        raw.pattern.unwrap_or_else(|| "*".to_owned()),
        action,
    ));
    Ok(())
}

fn parse_scalar(value: &str, line_no: usize) -> Result<String, String> {
    let value = strip_comment(value).trim();
    if value.is_empty() {
        return Err(format!("line {line_no}: expected a scalar value"));
    }
    let bytes = value.as_bytes();
    if (bytes[0] == b'"' || bytes[0] == b'\'') && bytes.last() == Some(&bytes[0]) {
        return Ok(value[1..value.len() - 1].to_owned());
    }
    if bytes[0] == b'"' || bytes[0] == b'\'' || bytes[0] == b'[' || bytes[0] == b'{' {
        return Err(format!("line {line_no}: unsupported or malformed scalar"));
    }
    Ok(value.to_owned())
}

fn strip_comment(value: &str) -> &str {
    value.split_once(" #").map_or(value, |(before, _)| before)
}

fn required_field<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    fields
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing required frontmatter field `{key}`"))
}

fn sanitize_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("frontmatter field `name` is empty".to_owned());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(format!(
            "frontmatter field `name` must contain only letters, numbers, '-' or '_' (got `{name}`)"
        ));
    }
    Ok(name.to_ascii_lowercase())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedMode {
    base_agent: Agent,
    availability: AgentAvailability,
}

fn parse_mode(mode: &str) -> Result<ParsedMode, String> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "primary" => Ok(ParsedMode {
            base_agent: Agent::Build,
            availability: AgentAvailability::Primary,
        }),
        "subagent" => Ok(ParsedMode {
            base_agent: Agent::Build,
            availability: AgentAvailability::Subagent,
        }),
        "all" => Ok(ParsedMode {
            base_agent: Agent::Build,
            availability: AgentAvailability::All,
        }),
        "build" => Ok(ParsedMode {
            base_agent: Agent::Build,
            availability: AgentAvailability::Primary,
        }),
        "plan" => Ok(ParsedMode {
            base_agent: Agent::Plan,
            availability: AgentAvailability::Primary,
        }),
        "general" => Ok(ParsedMode {
            base_agent: Agent::General,
            availability: AgentAvailability::Primary,
        }),
        "explore" => Ok(ParsedMode {
            base_agent: Agent::Explore,
            availability: AgentAvailability::Primary,
        }),
        other => Err(format!(
            "unsupported mode `{other}` (expected primary, all, subagent, build, plan, general, or explore)"
        )),
    }
}

fn agent_dirs(inputs: &ConfigInputs, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = BTreeSet::new();

    let cwd = &inputs.cwd;
    for dir in conventional_project_dirs(inputs.home.as_deref(), cwd) {
        push_dir(&mut dirs, &mut seen, dir);
    }
    if let Some(global) = global_agents_dir(inputs) {
        push_dir(&mut dirs, &mut seen, global);
    }

    match settings::load(inputs.clone()) {
        Ok(effective) => {
            for dir in configured_agent_dirs(&effective, cwd) {
                push_dir(&mut dirs, &mut seen, dir);
            }
        }
        Err(err) => warnings.push(format!(
            "custom agent discovery could not read configured agent directories: {err}"
        )),
    }

    dirs
}

fn conventional_project_dirs(home: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = Some(cwd.to_path_buf());
    while let Some(dir) = current {
        if Some(dir.as_path()) == home {
            break;
        }
        let parent = dir.parent().map(Path::to_path_buf);
        chain.push(dir);
        current = parent;
    }
    chain.reverse();

    let mut dirs = Vec::new();
    for dir in chain {
        dirs.push(dir.join(".cortex").join("agents"));
        dirs.push(dir.join("agents"));
    }
    dirs
}

fn global_agents_dir(inputs: &ConfigInputs) -> Option<PathBuf> {
    inputs
        .xdg_config_home
        .clone()
        .or_else(|| inputs.home.as_ref().map(|home| home.join(".config")))
        .map(|config| config.join("cortex").join("agents"))
}

fn configured_agent_dirs(effective: &EffectiveConfig, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for key in ["agent_dirs", "agents"] {
        if let Some(value) = effective.get(key) {
            append_dirs_from_value(&mut dirs, cwd, value);
        }
    }
    if let Some(agent) = effective.get("agent").and_then(Value::as_table) {
        for key in ["dirs", "directories", "path"] {
            if let Some(value) = agent.get(key) {
                append_dirs_from_value(&mut dirs, cwd, value);
            }
        }
    }
    dirs
}

fn append_dirs_from_value(dirs: &mut Vec<PathBuf>, cwd: &Path, value: &Value) {
    match value {
        Value::String(text) => {
            let parts = text.split(':').flat_map(|part| part.split(','));
            for part in parts {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    dirs.push(resolve_dir(cwd, trimmed));
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                if let Some(text) = value.as_str() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        dirs.push(resolve_dir(cwd, trimmed));
                    }
                }
            }
        }
        _ => {}
    }
}

fn resolve_dir(cwd: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn push_dir(dirs: &mut Vec<PathBuf>, seen: &mut BTreeSet<PathBuf>, dir: PathBuf) {
    if seen.insert(dir.clone()) {
        dirs.push(dir);
    }
}

fn builtin_name_set() -> BTreeSet<String> {
    Agent::ALL
        .iter()
        .map(|agent| normalize_name(agent.name()))
        .collect()
}

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn yaml_quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_agent_frontmatter_and_body() {
        let agent = parse_custom_agent(
            r#"---
name: pirate
description: Pirate responder
mode: primary
model: anthropic/claude-sonnet-4
temperature: 0.2
permission:
  - tool: write
    pattern: "**"
    action: deny
---
Answer like a pirate.
"#,
            PathBuf::from("pirate.md"),
        )
        .expect("valid agent");

        assert_eq!(agent.name, "pirate");
        assert_eq!(agent.description, "Pirate responder");
        assert_eq!(agent.availability, AgentAvailability::Primary);
        assert_eq!(agent.base_agent, Agent::Build);
        assert_eq!(agent.model.as_deref(), Some("anthropic/claude-sonnet-4"));
        assert_eq!(agent.temperature, Some(0.2));
        assert_eq!(agent.permissions.len(), 1);
        assert_eq!(agent.permissions[0], Rule::new("write", "**", Action::Deny));
        assert_eq!(agent.prompt, "Answer like a pirate.");
    }

    #[test]
    fn rejects_missing_or_malformed_frontmatter() {
        let missing = parse_custom_agent("No frontmatter", PathBuf::from("bad.md")).unwrap_err();
        assert!(missing.contains("missing YAML frontmatter"));

        let malformed =
            parse_custom_agent("---\nname: [\n---\nbody\n", PathBuf::from("bad.md")).unwrap_err();
        assert!(malformed.contains("malformed") || malformed.contains("unsupported"));
    }

    #[test]
    fn render_round_trips_through_the_parser() {
        let markdown = render_agent_markdown(
            "unit-tester",
            "Writes unit tests",
            "primary",
            Some("google/gemini-2.5-flash"),
            "Focus on useful tests.",
        );
        let parsed = parse_custom_agent(&markdown, PathBuf::from("unit-tester.md")).unwrap();
        assert_eq!(parsed.name, "unit-tester");
        assert_eq!(parsed.description, "Writes unit tests");
        assert_eq!(parsed.availability, AgentAvailability::Primary);
        assert_eq!(parsed.model.as_deref(), Some("google/gemini-2.5-flash"));
        assert_eq!(parsed.prompt, "Focus on useful tests.");
    }

    #[test]
    fn subagent_mode_is_spawnable_but_not_primary_selectable() {
        let agent = parse_custom_agent(
            r#"---
name: scout
description: Subagent scout
mode: subagent
---
Investigate and report.
"#,
            PathBuf::from("scout.md"),
        )
        .expect("valid subagent");
        assert_eq!(agent.availability, AgentAvailability::Subagent);
        assert!(!agent.availability.selectable());
        assert!(agent.availability.spawnable());

        let discovery = Discovery {
            agents: BTreeMap::from([("scout".to_owned(), agent)]),
            warnings: Vec::new(),
        };
        assert!(
            !available_names(&discovery).contains("scout"),
            "subagent-only agents should not be suggested for --agent"
        );
        let err = resolve(Some("scout"), &discovery).expect_err("not primary-selectable");
        assert!(
            err.to_string().contains("unknown agent 'scout'"),
            "the diagnostic still names the requested agent: {err}"
        );
    }

    #[test]
    fn all_mode_is_primary_selectable_and_spawnable() {
        let agent = parse_custom_agent(
            r#"---
name: helper
description: Helper
mode: all
---
Help with delegated or primary work.
"#,
            PathBuf::from("helper.md"),
        )
        .expect("valid all-mode agent");
        assert_eq!(agent.availability, AgentAvailability::All);
        assert!(agent.availability.selectable());
        assert!(agent.availability.spawnable());
    }

    #[test]
    fn sanitizes_generated_identifiers() {
        assert_eq!(
            sanitize_identifier("An Agent That Writes Tests!"),
            "an-agent-that-writes-tests"
        );
        assert_eq!(sanitize_identifier("!!!"), "generated-agent");
    }
}
