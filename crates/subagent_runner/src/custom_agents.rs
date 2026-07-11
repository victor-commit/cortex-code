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
use std::fs::OpenOptions;
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
pub fn default_project_agents_dir(cwd: &Path) -> PathBuf {
    cwd.join(".cortex").join("agents")
}

/// Check that a generated custom-agent identity and directory are safe to use.
///
/// A generation target must be one of the directories that the normal registry discovers, and its
/// name must not collide with a built-in or already discovered custom agent after normalization.
/// This check deliberately happens before creating the output directory or file.
pub fn preflight_generation(
    inputs: &ConfigInputs,
    target_dir: &Path,
    name: &str,
) -> Result<(), String> {
    let mut warnings = Vec::new();
    let discoverable_dirs = agent_dirs(inputs, &mut warnings);
    if !discoverable_dirs.iter().any(|dir| dir == target_dir) {
        return Err(format!(
            "generation target {} is not a discovered custom-agent directory",
            target_dir.display()
        ));
    }

    let normalized = normalize_name(name);
    if let Ok(builtin) = Agent::from_name(&normalized) {
        return Err(format!(
            "agent name '{}' collides with built-in agent '{}'",
            name.trim(),
            builtin.name()
        ));
    }

    let discovery = discover(inputs);
    if let Some(existing) = discovery.agents.get(&normalized) {
        return Err(format!(
            "agent name '{}' collides with custom agent '{}' at {}",
            name.trim(),
            existing.name,
            existing.source.display()
        ));
    }

    Ok(())
}

/// The Cortex-state directory used for generation leases.
///
/// Leases stay outside the project tree and use the canonical workspace key, so all configured
/// agent directories for a workspace contend for the same normalized-name lease without requiring
/// a writable ancestor or `.cortex/agents` directory.
#[must_use]
pub fn generation_lock_dir(inputs: &ConfigInputs) -> Option<PathBuf> {
    generation_lock_dir_with(inputs, &db::DbPathInputs::from_process())
}

fn generation_lock_dir_with(
    inputs: &ConfigInputs,
    state_inputs: &db::DbPathInputs,
) -> Option<PathBuf> {
    state_inputs.resolve().map(|db_path| {
        db_path
            .with_file_name("generated-agent-leases")
            .join(checkpoints::workspace_key(&inputs.cwd))
    })
}

/// An exclusive, per-name generation lease held while a new agent is persisted and re-discovered.
///
/// The lock is project-wide rather than target-directory-local. It prevents concurrent generators
/// using distinct configured agent directories from both passing preflight and reporting success
/// for the same identity. The lock file is never a markdown agent and is removed when the lease is
/// dropped.
pub struct GenerationLease {
    path: PathBuf,
}

impl GenerationLease {
    /// Acquire an exclusive lease for `name` in `lock_dir`.
    pub fn acquire(lock_dir: &Path, name: &str) -> Result<Self, String> {
        let normalized = normalize_name(name);
        let path = lock_dir.join(format!(".cortex-generate-{normalized}.lock"));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    format!(
                        "generation for agent '{}' is already in progress; try again after it finishes",
                        name.trim()
                    )
                } else {
                    format!(
                        "failed to reserve generated agent '{}' at {}: {err}",
                        name.trim(),
                        path.display()
                    )
                }
            })?;
        Ok(Self { path })
    }
}

impl Drop for GenerationLease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Return whether a freshly written path is the selected custom agent in a discovery result.
#[must_use]
pub fn generated_agent_is_discoverable(discovery: &Discovery, name: &str, path: &Path) -> bool {
    discovery
        .agents
        .get(&normalize_name(name))
        .is_some_and(|agent| agent.source == path && agent.availability.selectable())
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
    let Some(rest) = contents
        .strip_prefix("---\n")
        .or_else(|| contents.strip_prefix("---\r\n"))
    else {
        return Err("missing YAML frontmatter".to_owned());
    };

    let mut start = 0;
    for line_with_ending in rest.split_inclusive('\n') {
        let line = line_with_ending
            .strip_suffix('\n')
            .unwrap_or(line_with_ending)
            .strip_suffix('\r')
            .unwrap_or_else(|| {
                line_with_ending
                    .strip_suffix('\n')
                    .unwrap_or(line_with_ending)
            });
        if is_frontmatter_end_marker(line) {
            let body_start = start + line_with_ending.len();
            return Ok((&rest[..start], &rest[body_start..]));
        }
        start += line_with_ending.len();
    }

    if let Some(line) = rest.get(start..) {
        if is_frontmatter_end_marker(line) {
            return Ok((&rest[..start], ""));
        }
    }

    Err("unterminated YAML frontmatter".to_owned())
}

fn is_frontmatter_end_marker(line: &str) -> bool {
    line.strip_prefix("---")
        .is_some_and(|rest| rest.is_empty() || rest.trim_start().starts_with('#'))
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
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("line {line_no}: expected a scalar value"));
    }
    let bytes = value.as_bytes();
    if bytes[0] == b'"' {
        return parse_double_quoted_scalar(quoted_scalar(value, '"', line_no)?, line_no);
    }
    if bytes[0] == b'\'' {
        return parse_single_quoted_scalar(quoted_scalar(value, '\'', line_no)?, line_no);
    }
    let value = strip_comment(value).trim();
    if bytes[0] == b'[' || bytes[0] == b'{' {
        return Err(format!("line {line_no}: unsupported or malformed scalar"));
    }
    reject_raw_controls(value, line_no)?;
    Ok(value.to_owned())
}

fn strip_comment(value: &str) -> &str {
    value.split_once(" #").map_or(value, |(before, _)| before)
}

fn quoted_scalar(value: &str, quote: char, line_no: usize) -> Result<&str, String> {
    let mut chars = value.char_indices().peekable();
    let _ = chars.next();
    while let Some((index, ch)) = chars.next() {
        if quote == '"' && ch == '\\' {
            let _ = chars.next();
            continue;
        }
        if ch == '\'' && quote == '\'' && chars.next_if(|(_, next)| *next == '\'').is_some() {
            continue;
        }
        if ch == quote {
            let end = index + ch.len_utf8();
            let trailing = &value[end..];
            if trailing.trim().is_empty() || trailing.trim_start().starts_with('#') {
                return Ok(&value[..end]);
            }
            return Err(format!("line {line_no}: unsupported or malformed scalar"));
        }
    }
    Err(format!("line {line_no}: unsupported or malformed scalar"))
}

fn parse_double_quoted_scalar(value: &str, line_no: usize) -> Result<String, String> {
    if value.len() < 2 || !value.ends_with('"') {
        return Err(format!("line {line_no}: unsupported or malformed scalar"));
    }

    let mut out = String::new();
    let mut chars = value[1..value.len() - 1].chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let escaped = chars
                .next()
                .ok_or_else(|| format!("line {line_no}: incomplete quoted scalar escape"))?;
            let decoded = match escaped {
                '0' => '\0',
                'a' => '\u{7}',
                'b' => '\u{8}',
                't' | '\t' => '\t',
                'n' => '\n',
                'v' => '\u{b}',
                'f' => '\u{c}',
                'r' => '\r',
                'e' => '\u{1b}',
                ' ' => ' ',
                '"' => '"',
                '/' => '/',
                '\\' => '\\',
                'N' => '\u{85}',
                '_' => '\u{a0}',
                'L' => '\u{2028}',
                'P' => '\u{2029}',
                'x' => parse_unicode_escape(&mut chars, 2, line_no)?,
                'u' => parse_unicode_escape(&mut chars, 4, line_no)?,
                'U' => parse_unicode_escape(&mut chars, 8, line_no)?,
                other => {
                    return Err(format!(
                        "line {line_no}: unsupported quoted scalar escape `\\{other}`"
                    ));
                }
            };
            out.push(decoded);
        } else {
            reject_control(ch, line_no)?;
            out.push(ch);
        }
    }
    Ok(out)
}

fn parse_single_quoted_scalar(value: &str, line_no: usize) -> Result<String, String> {
    if value.len() < 2 || !value.ends_with('\'') {
        return Err(format!("line {line_no}: unsupported or malformed scalar"));
    }

    let content = &value[1..value.len() - 1];
    let mut out = String::new();
    let mut chars = content.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' && chars.next_if_eq(&'\'').is_some() {
            out.push('\'');
        } else {
            reject_control(ch, line_no)?;
            out.push(ch);
        }
    }
    Ok(out)
}

fn parse_unicode_escape(
    chars: &mut std::str::Chars<'_>,
    digits: usize,
    line_no: usize,
) -> Result<char, String> {
    let mut hex = String::with_capacity(digits);
    for _ in 0..digits {
        let Some(ch) = chars.next() else {
            return Err(format!(
                "line {line_no}: incomplete quoted scalar unicode escape"
            ));
        };
        if !ch.is_ascii_hexdigit() {
            return Err(format!(
                "line {line_no}: invalid quoted scalar unicode escape `\\{hex}{ch}`"
            ));
        }
        hex.push(ch);
    }
    let codepoint = u32::from_str_radix(&hex, 16).expect("validated hexadecimal escape");
    char::from_u32(codepoint).ok_or_else(|| {
        format!("line {line_no}: invalid quoted scalar Unicode code point U+{codepoint:04X}")
    })
}

fn reject_raw_controls(value: &str, line_no: usize) -> Result<(), String> {
    for ch in value.chars() {
        reject_control(ch, line_no)?;
    }
    Ok(())
}

fn reject_control(ch: char, line_no: usize) -> Result<(), String> {
    if ch.is_control() {
        Err(format!(
            "line {line_no}: raw control character U+{:04X} is not allowed in a scalar",
            ch as u32
        ))
    } else {
        Ok(())
    }
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
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\0' => out.push_str("\\0"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '\u{1b}' => out.push_str("\\e"),
            '"' => out.push_str("\\\""),
            '/' => out.push_str("\\/"),
            '\\' => out.push_str("\\\\"),
            control if control.is_control() => {
                use std::fmt::Write as _;
                write!(out, "\\u{:04X}", control as u32).expect("writing into String cannot fail");
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
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
    fn quoted_scalars_round_trip_quotes_slashes_and_controls() {
        let description = "quotes: \" / slash \\\\ line\ncrlf\r\n tab\t nul\0 bell\u{7} \
backspace\u{8} vertical\u{b} form\u{c} escape\u{1b} unit\u{1f}";
        let markdown = render_agent_markdown(
            "scalar-tester",
            description,
            "primary",
            Some("provider/model"),
            "Keep scalar values exact.",
        );

        assert!(
            !markdown.contains("line\ncrlf"),
            "frontmatter must escape newlines rather than create a new YAML key: {markdown}"
        );
        assert!(
            !markdown.contains("\r"),
            "renderer must not emit raw CR control bytes into frontmatter: {markdown:?}"
        );
        assert!(
            markdown.contains("\\/") && markdown.contains("\\\"") && markdown.contains("\\\\"),
            "renderer must use YAML double-quoted escapes: {markdown}"
        );

        let parsed = parse_custom_agent(&markdown, PathBuf::from("scalar-tester.md"))
            .expect("rendered YAML must parse");
        assert_eq!(parsed.description, description);
    }

    #[test]
    fn parses_existing_crlf_double_and_single_quoted_scalars() {
        let agent = parse_custom_agent(
            "---\r\n\
name: 'legacy-agent'\r\n\
description: 'legacy''s agent # still text'\r\n\
mode: \"primary\"\r\n\
---\r\n\
Use the legacy format.\r\n",
            PathBuf::from("legacy.md"),
        )
        .expect("existing valid CRLF frontmatter must remain compatible");

        assert_eq!(agent.name, "legacy-agent");
        assert_eq!(agent.description, "legacy's agent # still text");
        assert_eq!(agent.prompt, "Use the legacy format.");
    }

    #[test]
    fn parses_existing_commented_frontmatter_end_markers() {
        for contents in [
            "---\nname: commented\ndescription: Commented closing marker\nmode: primary\n--- # end agent\nPrompt.\n",
            "---\r\nname: commented\ndescription: Commented closing marker\nmode: primary\r\n--- # end agent\r\nPrompt.\r\n",
        ] {
            let agent = parse_custom_agent(contents, PathBuf::from("commented.md"))
                .expect("a valid commented YAML closing marker must remain compatible");
            assert_eq!(agent.name, "commented");
            assert_eq!(agent.prompt, "Prompt.");
        }
    }

    #[test]
    fn rejects_malformed_or_raw_control_quoted_scalars() {
        for (label, contents) in [
            (
                "unknown escape",
                "---\nname: safe\ndescription: \"bad\\q\"\nmode: primary\n---\nprompt\n",
            ),
            (
                "truncated unicode",
                "---\nname: safe\ndescription: \"bad\\u12\"\nmode: primary\n---\nprompt\n",
            ),
            (
                "surrogate",
                "---\nname: safe\ndescription: \"bad\\uD800\"\nmode: primary\n---\nprompt\n",
            ),
            (
                "raw tab",
                "---\nname: safe\ndescription: \"bad\tcontrol\"\nmode: primary\n---\nprompt\n",
            ),
        ] {
            let err = parse_custom_agent(contents, PathBuf::from("bad.md"))
                .expect_err("{label} must not be accepted");
            assert!(
                err.contains("line 2"),
                "{label} should identify the scalar line, got: {err}"
            );
        }
    }

    #[test]
    fn generation_preflight_rejects_builtin_and_normalized_custom_collisions() {
        let root = test_dir("generation-preflight");
        let cwd = root.join("project");
        fs::create_dir_all(&cwd).expect("create project");
        let inputs = test_inputs(&root, &cwd);
        let dir = default_project_agents_dir(&inputs.cwd);

        let builtin = preflight_generation(&inputs, &dir, " PLAN ")
            .expect_err("built-in identity must be reserved");
        assert!(
            builtin.to_string().contains("built-in agent 'plan'"),
            "built-in collision must be clear: {builtin}"
        );
        assert!(
            !dir.exists(),
            "a rejected built-in collision must not create an agent directory"
        );

        fs::create_dir_all(&dir).expect("create discovery directory");
        fs::write(
            dir.join("existing.md"),
            "---\nname: Existing-Agent\ndescription: Existing\nmode: primary\n---\nExisting prompt.\n",
        )
        .expect("write existing custom agent");
        let custom = preflight_generation(&inputs, &dir, " existing-agent ")
            .expect_err("normalized custom identity must be reserved");
        assert!(
            custom.to_string().contains("custom agent 'existing-agent'"),
            "custom collision must be clear: {custom}"
        );

        let undiscoverable = root.join("not-an-agent-directory");
        let target = preflight_generation(&inputs, &undiscoverable, "new-agent")
            .expect_err("generation must never report an undiscoverable target");
        assert!(
            target
                .to_string()
                .contains("not a discovered custom-agent directory"),
            "undiscoverable target must be clear: {target}"
        );
        assert!(
            !undiscoverable.exists(),
            "preflight must not create an undiscoverable target"
        );

        let _ = fs::remove_dir_all(root);
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

    #[test]
    fn generation_leases_use_the_state_resolver_and_workspace_key() {
        let root = test_dir("generation-lease-state");
        let first_workspace = root.join("first-project");
        let second_workspace = root.join("second-project");
        fs::create_dir_all(&first_workspace).expect("create first workspace");
        fs::create_dir_all(&second_workspace).expect("create second workspace");
        let state_db = root.join("state").join("cortex.db");
        let state_inputs = db::DbPathInputs {
            db_path: Some(state_db.clone()),
            ..Default::default()
        };

        let first = generation_lock_dir_with(
            &test_inputs(&root.join("home"), &first_workspace),
            &state_inputs,
        )
        .expect("an explicit state path resolves a generation lease directory");
        let second = generation_lock_dir_with(
            &test_inputs(&root.join("home"), &second_workspace),
            &state_inputs,
        )
        .expect("an explicit state path resolves a generation lease directory");

        assert_eq!(
            first,
            state_db
                .with_file_name("generated-agent-leases")
                .join(checkpoints::workspace_key(&first_workspace))
        );
        assert_eq!(
            second,
            state_db
                .with_file_name("generated-agent-leases")
                .join(checkpoints::workspace_key(&second_workspace))
        );
        assert_ne!(
            first, second,
            "workspaces sharing a Cortex state directory need distinct lease directories"
        );
        assert!(
            !first.starts_with(&first_workspace) && !second.starts_with(&second_workspace),
            "generation leases must not be derived from workspace ancestors"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn generation_leases_require_a_resolvable_cortex_state_location() {
        let root = test_dir("generation-lease-no-state");
        let workspace = root.join("project");
        fs::create_dir_all(&workspace).expect("create workspace");

        assert_eq!(
            generation_lock_dir_with(
                &test_inputs(&root.join("home"), &workspace),
                &db::DbPathInputs::default(),
            ),
            None,
            "generation must fail clearly instead of deriving a lock directory from a workspace ancestor"
        );

        let _ = fs::remove_dir_all(root);
    }

    fn test_inputs(home: &Path, cwd: &Path) -> ConfigInputs {
        ConfigInputs {
            home: Some(home.to_path_buf()),
            xdg_config_home: None,
            cwd: cwd.to_path_buf(),
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        }
    }

    fn test_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cortex-custom-agents-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create test directory");
        path
    }
}
