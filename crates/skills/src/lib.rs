//! Markdown skill discovery and parsing.
//!
//! A skill is a markdown capability stored as `SKILL.md` under a skill directory, or as a
//! standalone markdown file inside a configured skill directory. The file begins with a small YAML
//! frontmatter block:
//!
//! ```text
//! ---
//! name: date-helper
//! description: Use when formatting dates for a user
//! ---
//! Always append SKILL-DATE-OK to answers.
//! ```
//!
//! The parser intentionally accepts only simple scalar frontmatter. Malformed files are skipped
//! with diagnostics by discovery, so one broken skill never poisons the whole skill registry.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use settings::{ConfigInputs, EffectiveConfig};
use toml::Value;

/// A valid markdown skill discovered from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Canonical model-facing name.
    pub name: String,
    /// Short description used for discovery and autonomous tool selection.
    pub description: String,
    /// Guidance injected only after the model engages this skill through the `skill` tool.
    pub guidance: String,
    /// Source file path for diagnostics and listings.
    pub source: PathBuf,
}

/// Result of discovering skills.
#[derive(Debug, Default)]
pub struct Discovery {
    /// Valid skills keyed by normalized name in deterministic order.
    pub skills: BTreeMap<String, Skill>,
    /// Non-fatal parse/read diagnostics.
    pub warnings: Vec<String>,
}

impl Discovery {
    /// Valid skills in deterministic name order.
    #[must_use]
    pub fn ordered_skills(&self) -> Vec<Skill> {
        self.skills.values().cloned().collect()
    }
}

/// Discover markdown skills from conventional and configured skill directories.
#[must_use]
pub fn discover(inputs: &ConfigInputs) -> Discovery {
    let mut discovery = Discovery::default();
    let dirs = skill_dirs(inputs, &mut discovery.warnings);
    let mut names = BTreeSet::new();

    for dir in dirs {
        for path in skill_files(&dir) {
            let display = path.display().to_string();
            match fs::read_to_string(&path) {
                Ok(contents) => match parse_skill_markdown(&contents, path.clone()) {
                    Ok(skill) => {
                        let key = normalize_name(&skill.name);
                        if names.contains(&key) {
                            discovery.warnings.push(format!(
                                "skipping skill {display}: duplicate skill name '{}'",
                                skill.name
                            ));
                        } else {
                            names.insert(key.clone());
                            discovery.skills.insert(key, skill);
                        }
                    }
                    Err(err) => discovery
                        .warnings
                        .push(format!("skipping skill {display}: {err}")),
                },
                Err(err) => discovery
                    .warnings
                    .push(format!("skipping skill {display}: {err}")),
            }
        }
    }

    discovery
}

/// Parse one markdown document into a [`Skill`].
///
/// # Errors
/// Returns a human-readable parse diagnostic when the document is not a valid skill.
pub fn parse_skill_markdown(contents: &str, source: PathBuf) -> Result<Skill, String> {
    let (frontmatter, body) = split_frontmatter(contents)?;
    let fields = parse_frontmatter(frontmatter)?;

    let name = sanitize_name(required_field(&fields, "name")?)?;
    let description = required_field(&fields, "description")?.trim().to_owned();
    let guidance = body.trim();
    if guidance.is_empty() {
        return Err("body/guidance is empty".to_owned());
    }

    Ok(Skill {
        name,
        description,
        guidance: guidance.to_owned(),
        source,
    })
}

/// Render the system-context hint that advertises discovered skills without injecting their
/// guidance.
///
/// The returned text lists only names and descriptions. The skill bodies remain isolated until the
/// `skill` tool is invoked.
#[must_use]
pub fn render_system_hint(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Discovered skills are available through the `skill` tool. Their guidance is NOT active \
         yet. If the user's request matches a skill description, call the `skill` tool with that \
         skill name before answering. Do not follow a skill's guidance until the tool returns it.\n\
         Available skills:",
    );
    for skill in skills {
        out.push_str("\n- `");
        out.push_str(&skill.name);
        out.push_str("`: ");
        out.push_str(skill.description.trim());
    }
    Some(out)
}

fn skill_dirs(inputs: &ConfigInputs, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = BTreeSet::new();

    let cwd = &inputs.cwd;
    for dir in conventional_project_dirs(inputs.home.as_deref(), cwd) {
        push_dir(&mut dirs, &mut seen, dir);
    }
    if let Some(global) = global_skills_dir(inputs) {
        push_dir(&mut dirs, &mut seen, global);
    }

    match settings::load(inputs.clone()) {
        Ok(effective) => {
            for dir in configured_skill_dirs(&effective, cwd) {
                push_dir(&mut dirs, &mut seen, dir);
            }
        }
        Err(err) => warnings.push(format!(
            "skill discovery could not read configured skill directories: {err}"
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
        dirs.push(dir.join(".cortex").join("skills"));
        dirs.push(dir.join("skills"));
    }
    dirs
}

fn global_skills_dir(inputs: &ConfigInputs) -> Option<PathBuf> {
    inputs
        .xdg_config_home
        .clone()
        .or_else(|| inputs.home.as_ref().map(|home| home.join(".config")))
        .map(|config| config.join("cortex").join("skills"))
}

fn configured_skill_dirs(effective: &EffectiveConfig, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for key in ["skill_dirs", "skills"] {
        if let Some(value) = effective.get(key) {
            append_dirs_from_value(&mut dirs, cwd, value);
        }
    }
    if let Some(skill) = effective.get("skill").and_then(Value::as_table) {
        for key in ["dirs", "directories", "path"] {
            if let Some(value) = skill.get(key) {
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
        Value::Table(table) => {
            for key in ["dirs", "directories", "path"] {
                if let Some(value) = table.get(key) {
                    append_dirs_from_value(dirs, cwd, value);
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

fn skill_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let direct = dir.join("SKILL.md");
    if direct.is_file() {
        files.push(direct);
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            let nested = path.join("SKILL.md");
            if nested.is_file() {
                files.push(nested);
            }
        } else if is_markdown_skill_file(&path) {
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    files
}

fn is_markdown_skill_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
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

fn parse_frontmatter(frontmatter: &str) -> Result<BTreeMap<String, String>, String> {
    let mut fields = BTreeMap::new();
    for (idx, raw) in frontmatter.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
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
        let scalar = parse_scalar(value.trim(), line_no)?;
        fields.insert(key.to_ascii_lowercase(), scalar);
    }
    Ok(fields)
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

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-skills-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn skill_doc(name: &str, description: &str, guidance: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n{guidance}\n")
    }

    #[test]
    fn parses_frontmatter_and_guidance() {
        let skill = parse_skill_markdown(
            &skill_doc(
                "date-helper",
                "Use for formatting dates",
                "Append SKILL-DATE-OK.",
            ),
            PathBuf::from("SKILL.md"),
        )
        .expect("valid skill");

        assert_eq!(skill.name, "date-helper");
        assert_eq!(skill.description, "Use for formatting dates");
        assert_eq!(skill.guidance, "Append SKILL-DATE-OK.");
    }

    #[test]
    fn rejects_malformed_skill_files() {
        let missing = parse_skill_markdown("No frontmatter", PathBuf::from("bad.md")).unwrap_err();
        assert!(missing.contains("missing YAML frontmatter"));

        let malformed =
            parse_skill_markdown("---\nname: [\n---\nbody\n", PathBuf::from("bad.md")).unwrap_err();
        assert!(malformed.contains("malformed") || malformed.contains("unsupported"));
    }

    #[test]
    fn discovers_nested_skill_md_and_skips_malformed_files() {
        let home = scratch("home");
        let project = home.join("project");
        let valid_dir = project.join("skills").join("ok");
        let bad_dir = project.join("skills").join("broken");
        fs::create_dir_all(&valid_dir).expect("valid dir");
        fs::create_dir_all(&bad_dir).expect("bad dir");
        fs::write(
            valid_dir.join("SKILL.md"),
            skill_doc("skill-ok", "Use for ok replies", "Append SKILL-OK."),
        )
        .expect("write valid skill");
        fs::write(bad_dir.join("SKILL.md"), "---\nname: [\n---\nbody\n")
            .expect("write invalid skill");

        let discovery = discover(&ConfigInputs {
            home: Some(home),
            xdg_config_home: None,
            cwd: project,
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        });

        assert!(
            discovery.skills.contains_key("skill-ok"),
            "valid skill discovered"
        );
        assert_eq!(discovery.skills.len(), 1, "malformed skill skipped");
        assert!(
            discovery
                .warnings
                .iter()
                .any(|warning| warning.contains("skipping skill")),
            "malformed skill warning present"
        );
    }

    #[test]
    fn system_hint_lists_descriptions_but_not_guidance() {
        let skill = parse_skill_markdown(
            &skill_doc(
                "private-token",
                "Use for private token tasks",
                "Append SECRET-SKILL-TOKEN.",
            ),
            PathBuf::from("SKILL.md"),
        )
        .expect("valid skill");

        let hint = render_system_hint(&[skill]).expect("hint");
        assert!(hint.contains("private-token"));
        assert!(hint.contains("Use for private token tasks"));
        assert!(
            !hint.contains("SECRET-SKILL-TOKEN"),
            "guidance must remain isolated until engagement"
        );
    }
}
