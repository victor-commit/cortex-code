//! Custom markdown mode discovery and parsing.
//!
//! Modes are composable overlays for `cortex run --mode`: they can change the base persona
//! (`build`/`plan`/`general`/`explore`), add permission rules, and append prompt instructions while
//! preserving the selected agent identity.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use engine::Agent;
use permissions::{Action, Rule};
use settings::{ConfigInputs, EffectiveConfig};
use toml::Value;

/// A valid custom mode discovered from disk.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomMode {
    /// Canonical selectable mode name.
    pub name: String,
    /// Discovery-list description.
    pub description: String,
    /// Optional base agent override.
    pub base_agent: Option<Agent>,
    /// Mode-scoped permission rules merged with agent rules.
    pub permissions: Vec<Rule>,
    /// Optional markdown body instructions.
    pub prompt: String,
    /// Source file path for diagnostics and listings.
    pub source: PathBuf,
}

/// Result of discovering custom modes.
#[derive(Debug, Default)]
pub struct Discovery {
    pub modes: BTreeMap<String, CustomMode>,
    pub warnings: Vec<String>,
}

/// One active mode selected for a run.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveMode {
    name: String,
    base_agent: Option<Agent>,
    permissions: Vec<Rule>,
    prompt: String,
}

impl ActiveMode {
    /// The mode name reported in machine output.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Base-agent override from this mode, if any.
    #[must_use]
    pub const fn base_agent(&self) -> Option<Agent> {
        self.base_agent
    }

    /// Prompt fragment carried by this mode.
    #[must_use]
    pub fn prompt(&self) -> Option<&str> {
        (!self.prompt.trim().is_empty()).then_some(self.prompt.as_str())
    }

    /// Mode permission rules.
    #[must_use]
    pub fn permissions(&self) -> &[Rule] {
        &self.permissions
    }
}

/// Unknown-mode diagnostic with an availability hint spanning built-ins and custom modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownMode {
    requested: String,
    available: String,
}

impl UnknownMode {
    fn new(requested: &str, available: String) -> Self {
        Self {
            requested: requested.to_owned(),
            available,
        }
    }
}

impl std::fmt::Display for UnknownMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown mode '{}': available modes are {}",
            self.requested, self.available
        )
    }
}

impl std::error::Error for UnknownMode {}

/// Discover custom markdown modes from configured and conventional mode directories.
#[must_use]
pub fn discover(inputs: &ConfigInputs) -> Discovery {
    let mut discovery = Discovery::default();
    let dirs = mode_dirs(inputs, &mut discovery.warnings);
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
                Ok(contents) => match parse_custom_mode(&contents, path.clone()) {
                    Ok(mode) => {
                        let key = normalize_name(&mode.name);
                        if names.contains(&key) {
                            discovery.warnings.push(format!(
                                "skipping custom mode {display}: duplicate mode name '{}'",
                                mode.name
                            ));
                        } else {
                            names.insert(key.clone());
                            discovery.modes.insert(key, mode);
                        }
                    }
                    Err(err) => discovery
                        .warnings
                        .push(format!("skipping custom mode {display}: {err}")),
                },
                Err(err) => discovery
                    .warnings
                    .push(format!("skipping custom mode {display}: {err}")),
            }
        }
    }

    discovery
}

/// Resolve an optional mode name against built-ins and custom discoveries.
pub fn resolve(
    name: Option<&str>,
    discovery: &Discovery,
) -> Result<Option<ActiveMode>, UnknownMode> {
    let Some(name) = name else {
        return Ok(None);
    };
    if let Ok(agent) = Agent::from_name(name) {
        return Ok(Some(ActiveMode {
            name: agent.name().to_owned(),
            base_agent: Some(agent),
            permissions: Vec::new(),
            prompt: String::new(),
        }));
    }
    let normalized = normalize_name(name);
    if let Some(mode) = discovery.modes.get(&normalized) {
        return Ok(Some(ActiveMode {
            name: mode.name.clone(),
            base_agent: mode.base_agent,
            permissions: mode.permissions.clone(),
            prompt: mode.prompt.clone(),
        }));
    }
    Err(UnknownMode::new(name, available_names(discovery)))
}

/// Every available mode name in stable discovery order.
#[must_use]
pub fn available_names(discovery: &Discovery) -> String {
    let mut names = Agent::ALL
        .iter()
        .map(|agent| agent.name().to_owned())
        .collect::<Vec<_>>();
    names.extend(discovery.modes.values().map(|mode| mode.name.clone()));
    names.join(", ")
}

/// Parse one markdown document into a [`CustomMode`].
pub fn parse_custom_mode(contents: &str, source: PathBuf) -> Result<CustomMode, String> {
    let (frontmatter, body) = split_frontmatter(contents)?;
    let parsed = parse_frontmatter(frontmatter)?;

    let name = required_field(&parsed.fields, "name")?;
    let name = sanitize_name(name)?;
    let description = parsed
        .fields
        .get("description")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("mode {name}"));
    let base_agent = parsed
        .fields
        .get("agent")
        .or_else(|| parsed.fields.get("base_agent"))
        .or_else(|| parsed.fields.get("mode"))
        .map(|value| parse_agent_name(value))
        .transpose()?;
    Ok(CustomMode {
        name,
        description,
        base_agent,
        permissions: parsed.permissions,
        prompt: body.trim().to_owned(),
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

fn parse_agent_name(raw: &str) -> Result<Agent, String> {
    Agent::from_name(raw).map_err(|_| {
        format!("unsupported mode agent `{raw}` (expected build, plan, general, or explore)")
    })
}

fn mode_dirs(inputs: &ConfigInputs, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = BTreeSet::new();

    let cwd = &inputs.cwd;
    for dir in conventional_project_dirs(inputs.home.as_deref(), cwd) {
        push_dir(&mut dirs, &mut seen, dir);
    }
    if let Some(global) = global_modes_dir(inputs) {
        push_dir(&mut dirs, &mut seen, global);
    }

    match settings::load(inputs.clone()) {
        Ok(effective) => {
            for dir in configured_mode_dirs(&effective, cwd) {
                push_dir(&mut dirs, &mut seen, dir);
            }
        }
        Err(err) => warnings.push(format!(
            "custom mode discovery could not read configured mode directories: {err}"
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
        dirs.push(dir.join(".cortex").join("modes"));
        dirs.push(dir.join("modes"));
    }
    dirs
}

fn global_modes_dir(inputs: &ConfigInputs) -> Option<PathBuf> {
    inputs
        .xdg_config_home
        .clone()
        .or_else(|| inputs.home.as_ref().map(|home| home.join(".config")))
        .map(|config| config.join("cortex").join("modes"))
}

fn configured_mode_dirs(effective: &EffectiveConfig, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for key in ["mode_dirs", "modes"] {
        if let Some(value) = effective.get(key) {
            append_dirs_from_value(&mut dirs, cwd, value);
        }
    }
    if let Some(mode) = effective.get("mode").and_then(Value::as_table) {
        for key in ["dirs", "directories", "path"] {
            if let Some(value) = mode.get(key) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_mode_frontmatter_and_body() {
        let mode = parse_custom_mode(
            r#"---
name: review
description: Review mode
agent: plan
permission:
  - tool: write
    pattern: "**"
    action: deny
---
Plan carefully and avoid writes.
"#,
            PathBuf::from("review.md"),
        )
        .expect("valid mode");

        assert_eq!(mode.name, "review");
        assert_eq!(mode.description, "Review mode");
        assert_eq!(mode.base_agent, Some(Agent::Plan));
        assert_eq!(mode.permissions.len(), 1);
        assert_eq!(mode.permissions[0], Rule::new("write", "**", Action::Deny));
        assert!(mode.prompt.contains("avoid writes"));
    }

    #[test]
    fn resolve_accepts_builtin_modes() {
        let discovery = Discovery::default();
        let mode = resolve(Some("plan"), &discovery)
            .expect("resolve")
            .expect("mode");
        assert_eq!(mode.name(), "plan");
        assert_eq!(mode.base_agent(), Some(Agent::Plan));
    }
}
