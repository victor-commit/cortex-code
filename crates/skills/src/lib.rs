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
use std::path::{Component, Path, PathBuf};

use reqwest::blocking::Client;
use serde::Deserialize;
use settings::{ConfigInputs, EffectiveConfig};
use toml::Value;
use url::Url;

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
    let sources = skill_sources(inputs, &mut discovery.warnings);
    let mut dirs = sources.dirs;
    for url in sources.urls {
        for dir in remote_skill_dirs(inputs, &url, &mut discovery.warnings) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
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

#[derive(Debug, Default)]
struct SkillSources {
    dirs: Vec<PathBuf>,
    urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteIndex {
    skills: Vec<RemoteIndexSkill>,
}

#[derive(Debug, Deserialize)]
struct RemoteIndexSkill {
    name: String,
    files: Vec<String>,
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

fn skill_sources(inputs: &ConfigInputs, warnings: &mut Vec<String>) -> SkillSources {
    let mut sources = SkillSources::default();
    let mut seen_dirs = BTreeSet::new();
    let mut seen_urls = BTreeSet::new();

    let cwd = &inputs.cwd;
    for dir in conventional_project_dirs(inputs.home.as_deref(), cwd) {
        push_dir(&mut sources.dirs, &mut seen_dirs, dir);
    }
    for global in global_skill_dirs(inputs) {
        push_dir(&mut sources.dirs, &mut seen_dirs, global);
    }

    match settings::load(inputs.clone()) {
        Ok(effective) => {
            for dir in configured_skill_dirs(&effective, cwd) {
                push_dir(&mut sources.dirs, &mut seen_dirs, dir);
            }
            for url in configured_skill_urls(&effective) {
                push_url(&mut sources.urls, &mut seen_urls, url);
            }
        }
        Err(err) => warnings.push(format!(
            "skill discovery could not read configured skill sources: {err}"
        )),
    }

    sources
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
        dirs.push(dir.join(".claude").join("skills"));
        dirs.push(dir.join(".agents").join("skills"));
        dirs.push(dir.join(".cortex").join("skills"));
        dirs.push(dir.join(".cortex").join("skill"));
        dirs.push(dir.join("skills"));
    }
    dirs
}

fn global_skill_dirs(inputs: &ConfigInputs) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(config) = inputs
        .xdg_config_home
        .clone()
        .or_else(|| inputs.home.as_ref().map(|home| home.join(".config")))
    {
        dirs.push(config.join("cortex").join("skills"));
        dirs.push(config.join("cortex").join("skill"));
    }
    if let Some(home) = inputs.home.as_ref() {
        dirs.push(home.join(".claude").join("skills"));
        dirs.push(home.join(".agents").join("skills"));
    }
    dirs
}

fn configured_skill_dirs(effective: &EffectiveConfig, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for key in ["skill_dirs", "skills"] {
        if let Some(value) = effective.get(key) {
            append_dirs_from_value(&mut dirs, cwd, value);
        }
    }
    if let Some(skill) = effective.get("skill").and_then(Value::as_table) {
        for key in ["dirs", "directories", "path", "paths"] {
            if let Some(value) = skill.get(key) {
                append_dirs_from_value(&mut dirs, cwd, value);
            }
        }
    }
    dirs
}

fn configured_skill_urls(effective: &EffectiveConfig) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(value) = effective.get("skills") {
        append_urls_from_value(&mut urls, value);
    }
    if let Some(value) = effective.get("skill_urls") {
        append_urls_from_value(&mut urls, value);
    }
    if let Some(skills) = effective.get("skills").and_then(Value::as_table) {
        for key in ["urls", "url"] {
            if let Some(value) = skills.get(key) {
                append_urls_from_value(&mut urls, value);
            }
        }
    }
    if let Some(skill) = effective.get("skill").and_then(Value::as_table) {
        for key in ["urls", "url"] {
            if let Some(value) = skill.get(key) {
                append_urls_from_value(&mut urls, value);
            }
        }
    }
    urls
}

fn append_dirs_from_value(dirs: &mut Vec<PathBuf>, cwd: &Path, value: &Value) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if looks_like_remote_url(trimmed) {
                return;
            }
            let parts = trimmed.split(':').flat_map(|part| part.split(','));
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
                    if !trimmed.is_empty() && !looks_like_remote_url(trimmed) {
                        dirs.push(resolve_dir(cwd, trimmed));
                    }
                }
            }
        }
        Value::Table(table) => {
            for key in ["dirs", "directories", "path", "paths"] {
                if let Some(value) = table.get(key) {
                    append_dirs_from_value(dirs, cwd, value);
                }
            }
        }
        _ => {}
    }
}

fn append_urls_from_value(urls: &mut Vec<String>, value: &Value) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if looks_like_remote_url(trimmed) {
                urls.push(trimmed.to_owned());
            }
        }
        Value::Array(values) => {
            for value in values {
                append_urls_from_value(urls, value);
            }
        }
        Value::Table(table) => {
            for key in ["urls", "url"] {
                if let Some(value) = table.get(key) {
                    append_urls_from_value(urls, value);
                }
            }
        }
        _ => {}
    }
}

fn looks_like_remote_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
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

fn push_url(urls: &mut Vec<String>, seen: &mut BTreeSet<String>, url: String) {
    if seen.insert(url.clone()) {
        urls.push(url);
    }
}

fn skill_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let direct = dir.join("SKILL.md");
    if direct.is_file() {
        files.push(direct);
    }

    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        let mut paths = entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        paths.sort();

        for path in paths {
            if is_directory_without_following_symlink(&path) {
                if path != current {
                    let nested = path.join("SKILL.md");
                    if nested.is_file() {
                        files.push(nested);
                    }
                }
                stack.push(path);
            }
        }
    }

    let Ok(entries) = fs::read_dir(dir) else {
        files.sort();
        files.dedup();
        return files;
    };
    let mut top_level = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    top_level.sort();
    for path in top_level {
        if path.is_file() && is_markdown_skill_file(&path) {
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

fn is_directory_without_following_symlink(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => false,
        Ok(metadata) => metadata.file_type().is_dir(),
        Err(_) => false,
    }
}

fn remote_skill_dirs(
    inputs: &ConfigInputs,
    source: &str,
    warnings: &mut Vec<String>,
) -> Vec<PathBuf> {
    let Some(cache_root) = remote_cache_root(inputs) else {
        warnings.push(format!(
            "skipping remote skills `{source}`: cannot resolve a cache directory"
        ));
        return Vec::new();
    };
    let Some(base_url) = normalize_remote_base_url(source, warnings) else {
        return Vec::new();
    };
    let cache_dir = cache_root.join(hex_encode(base_url.as_str().as_bytes()));
    if redaction::network_disabled() {
        warnings.push(format!(
            "skipping remote skills `{}`: network is disabled by CORTEX_NO_NET=1",
            base_url
        ));
        return cached_remote_skill_dirs(&cache_dir);
    }

    match fetch_remote_index(&base_url) {
        Ok(index) => {
            if let Err(err) = fs::create_dir_all(&cache_dir) {
                warnings.push(format!(
                    "skipping remote skills `{}`: cannot create cache dir {}: {err}",
                    base_url,
                    cache_dir.display()
                ));
                return Vec::new();
            }
            let mut discovered = Vec::new();
            for skill in index.skills {
                let Some(skill_name) = sanitize_remote_skill_name(&skill.name) else {
                    warnings.push(format!(
                        "skipping remote skill `{}` from `{}`: invalid name",
                        skill.name, base_url
                    ));
                    continue;
                };
                if !(skill.files.iter().any(|file| file == "SKILL.md")
                    || skill
                        .files
                        .iter()
                        .any(|file| file == &format!("{skill_name}.md")))
                {
                    warnings.push(format!(
                        "skipping remote skill `{skill_name}` from `{}`: missing SKILL.md",
                        base_url
                    ));
                    continue;
                }
                let Some(downloaded) =
                    cache_remote_skill(&cache_dir, &base_url, &skill_name, &skill.files, warnings)
                else {
                    continue;
                };
                discovered.push(downloaded);
            }
            discovered
        }
        Err(err) => {
            let cached = cached_remote_skill_dirs(&cache_dir);
            if cached.is_empty() {
                warnings.push(format!("skipping remote skills `{}`: {err}", base_url));
                Vec::new()
            } else {
                warnings.push(format!(
                    "using cached remote skills for `{}` after fetch error: {err}",
                    base_url
                ));
                cached
            }
        }
    }
}

fn normalize_remote_base_url(source: &str, warnings: &mut Vec<String>) -> Option<Url> {
    let trimmed = source.trim();
    let parsed = match Url::parse(trimmed) {
        Ok(url) => url,
        Err(err) => {
            warnings.push(format!(
                "skipping remote skills `{trimmed}`: invalid URL: {err}"
            ));
            return None;
        }
    };
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            warnings.push(format!(
                "skipping remote skills `{trimmed}`: unsupported URL scheme `{other}`"
            ));
            return None;
        }
    }
    if parsed.path().ends_with('/') {
        Some(parsed)
    } else {
        parsed
            .join("./")
            .ok()
            .or_else(|| Url::parse(&format!("{}/", parsed.as_str())).ok())
    }
}

fn fetch_remote_index(base_url: &Url) -> Result<RemoteIndex, String> {
    let index_url = base_url
        .join("index.json")
        .map_err(|err| format!("cannot build index URL: {err}"))?;
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|err| format!("cannot build HTTP client: {err}"))?;
    let response = client
        .get(index_url.clone())
        .send()
        .map_err(|err| format!("cannot fetch {index_url}: {err}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "cannot fetch {index_url}: http status {}",
            response.status()
        ));
    }
    let body = response
        .bytes()
        .map_err(|err| format!("cannot read {index_url}: {err}"))?;
    serde_json::from_slice::<RemoteIndex>(&body)
        .map_err(|err| format!("cannot parse {index_url}: {err}"))
}

fn cache_remote_skill(
    cache_dir: &Path,
    base_url: &Url,
    skill_name: &str,
    files: &[String],
    warnings: &mut Vec<String>,
) -> Option<PathBuf> {
    let Some(skill_root) = safe_join_remote_path(cache_dir, skill_name) else {
        warnings.push(format!(
            "skipping remote skill `{skill_name}` from `{}`: invalid cache destination",
            base_url
        ));
        return None;
    };
    let Some(skill_base_url) = base_url.join(&format!("{skill_name}/")).ok() else {
        warnings.push(format!(
            "skipping remote skill `{skill_name}` from `{}`: invalid skill URL",
            base_url
        ));
        return None;
    };
    for file in files {
        let Some(relative) = sanitize_relative_remote_file(file) else {
            warnings.push(format!(
                "skipping remote skill `{skill_name}` from `{}`: unsafe file path `{file}`",
                base_url
            ));
            return None;
        };
        let Ok(file_url) = skill_base_url.join(&relative) else {
            warnings.push(format!(
                "skipping remote skill `{skill_name}` from `{}`: invalid file URL `{file}`",
                base_url
            ));
            return None;
        };
        if file_url.origin() != base_url.origin() {
            warnings.push(format!(
                "skipping remote skill `{skill_name}` from `{}`: cross-origin file `{}`",
                base_url, file_url
            ));
            return None;
        }
        let Some(destination) = safe_join_remote_path(&skill_root, &relative) else {
            warnings.push(format!(
                "skipping remote skill `{skill_name}` from `{}`: escaped destination `{}`",
                base_url, relative
            ));
            return None;
        };
        if destination.exists() {
            continue;
        }
        let Some(parent) = destination.parent() else {
            warnings.push(format!(
                "skipping remote skill `{skill_name}` from `{}`: invalid destination path",
                base_url
            ));
            return None;
        };
        if let Err(err) = fs::create_dir_all(parent) {
            warnings.push(format!(
                "skipping remote skill `{skill_name}` from `{}`: cannot create {}: {err}",
                base_url,
                parent.display()
            ));
            return None;
        }
        if let Err(err) = download_remote_file(&file_url, &destination) {
            warnings.push(format!(
                "skipping remote skill `{skill_name}` from `{}`: {err}",
                base_url
            ));
            return None;
        }
    }

    let default_skill = skill_root.join("SKILL.md");
    let named_skill = skill_root.join(format!("{skill_name}.md"));
    if default_skill.is_file() || named_skill.is_file() {
        Some(skill_root)
    } else {
        warnings.push(format!(
            "skipping remote skill `{skill_name}` from `{}`: downloaded files did not include SKILL.md",
            base_url
        ));
        None
    }
}

fn download_remote_file(url: &Url, destination: &Path) -> Result<(), String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|err| format!("cannot build HTTP client: {err}"))?;
    let response = client
        .get(url.clone())
        .send()
        .map_err(|err| format!("cannot fetch {url}: {err}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "cannot fetch {url}: http status {}",
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .map_err(|err| format!("cannot read {url}: {err}"))?;
    fs::write(destination, &bytes).map_err(|err| {
        format!(
            "cannot write remote skill file {}: {err}",
            destination.display()
        )
    })
}

fn remote_cache_root(inputs: &ConfigInputs) -> Option<PathBuf> {
    let root = inputs
        .xdg_config_home
        .clone()
        .or_else(|| inputs.home.as_ref().map(|home| home.join(".config")))?;
    Some(root.join("cortex").join("cache").join("skills"))
}

fn cached_remote_skill_dirs(cache_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return Vec::new();
    };
    let mut dirs = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return false;
            };
            path.join("SKILL.md").is_file() || path.join(format!("{name}.md")).is_file()
        })
        .collect::<Vec<_>>();
    dirs.sort();
    dirs
}

fn sanitize_remote_skill_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('\0')
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.chars().all(|ch| ch == '.')
        || trimmed.contains("..")
    {
        return None;
    }
    if trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        is_single_normal_component(trimmed).then(|| trimmed.to_owned())
    } else {
        None
    }
}

fn sanitize_relative_remote_file(file: &str) -> Option<String> {
    let trimmed = file.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || looks_like_remote_url(trimmed)
        || Url::parse(trimmed).is_ok()
    {
        return None;
    }
    let candidate = Path::new(trimmed);
    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return None;
    }
    Some(trimmed.to_owned())
}

fn is_single_normal_component(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

fn safe_join_remote_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut destination = root.to_path_buf();
    let mut saw_component = false;
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => {
                destination.push(part);
                saw_component = true;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    saw_component.then_some(destination)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
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
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

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

    fn write_skill(path: &Path, name: &str, description: &str, guidance: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create skill parent");
        }
        fs::write(path, skill_doc(name, description, guidance)).expect("write skill");
    }

    struct HttpFixture {
        base_url: String,
        hits: Arc<AtomicUsize>,
        handle: std::thread::JoinHandle<()>,
    }

    impl HttpFixture {
        fn start(index: String, files: BTreeMap<String, String>) -> Self {
            let listener = bind_loopback_listener();
            listener
                .set_nonblocking(true)
                .expect("set listener nonblocking");
            let port = listener.local_addr().expect("local addr").port();
            let hits = Arc::new(AtomicUsize::new(0));
            let hits_thread = Arc::clone(&hits);
            let handle = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while Instant::now() < deadline {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            hits_thread.fetch_add(1, Ordering::Relaxed);
                            let request_path = read_request_path(&mut stream);
                            let body = if request_path == "/catalog/index.json" {
                                Some(index.as_str())
                            } else {
                                files.get(request_path.as_str()).map(String::as_str)
                            };
                            if let Some(body) = body {
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                                    body.len(),
                                    body
                                );
                                let _ = stream.write_all(response.as_bytes());
                                let _ = stream.flush();
                            } else {
                                let body = "not found";
                                let response = format!(
                                    "HTTP/1.1 404 Not Found\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                                    body.len(),
                                    body
                                );
                                let _ = stream.write_all(response.as_bytes());
                                let _ = stream.flush();
                            }
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                base_url: format!("http://127.0.0.1:{port}/catalog/"),
                hits,
                handle,
            }
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::Relaxed)
        }

        fn join(self) {
            let _ = self.handle.join();
        }
    }

    fn bind_loopback_listener() -> TcpListener {
        TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener")
    }

    fn read_request_path(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("set timeout");
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 1024];
        while bytes.windows(4).all(|window| window != b"\r\n\r\n") {
            let read = stream.read(&mut chunk).unwrap_or(0);
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&bytes);
        let mut parts = request.lines().next().unwrap_or("").split_whitespace();
        let _method = parts.next();
        parts.next().unwrap_or("/").to_owned()
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

    #[test]
    fn discovers_skill_md_recursively_beyond_one_directory_level() {
        let home = scratch("home-recursive");
        let project = home.join("project");
        let nested_skill = project
            .join("skills")
            .join("level-one")
            .join("level-two")
            .join("deep-skill")
            .join("SKILL.md");
        write_skill(
            &nested_skill,
            "deep-skill",
            "Use for deep recursion checks",
            "Append SKILL-DEEP.",
        );

        let discovery = discover(&ConfigInputs {
            home: Some(home),
            xdg_config_home: None,
            cwd: project,
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        });

        assert!(
            discovery.skills.contains_key("deep-skill"),
            "recursive SKILL.md should be discovered at arbitrary depth"
        );
    }

    #[test]
    fn discovers_project_and_global_claude_and_agents_skill_directories() {
        let home = scratch("home-claude-agents");
        let project = home.join("workspace").join("project");
        fs::create_dir_all(&project).expect("create project");

        write_skill(
            &project
                .join(".claude")
                .join("skills")
                .join("project-claude")
                .join("SKILL.md"),
            "project-claude",
            "Project claude skill",
            "Append SKILL-PROJECT-CLAUDE.",
        );
        write_skill(
            &project
                .join(".agents")
                .join("skills")
                .join("project-agent")
                .join("SKILL.md"),
            "project-agent",
            "Project agent skill",
            "Append SKILL-PROJECT-AGENT.",
        );
        write_skill(
            &home
                .join(".claude")
                .join("skills")
                .join("global-claude")
                .join("SKILL.md"),
            "global-claude",
            "Global claude skill",
            "Append SKILL-GLOBAL-CLAUDE.",
        );
        write_skill(
            &home
                .join(".agents")
                .join("skills")
                .join("global-agent")
                .join("SKILL.md"),
            "global-agent",
            "Global agent skill",
            "Append SKILL-GLOBAL-AGENT.",
        );

        let discovery = discover(&ConfigInputs {
            home: Some(home),
            xdg_config_home: None,
            cwd: project,
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        });

        for name in [
            "project-claude",
            "project-agent",
            "global-claude",
            "global-agent",
        ] {
            assert!(
                discovery.skills.contains_key(name),
                "missing discovered skill {name}"
            );
        }
    }

    #[test]
    fn remote_skill_name_validation_rejects_unsafe_inputs() {
        assert_eq!(
            sanitize_remote_skill_name("remote-skill"),
            Some("remote-skill".to_owned())
        );
        assert_eq!(
            sanitize_remote_skill_name("remote.skill"),
            Some("remote.skill".to_owned())
        );

        for invalid in [
            "",
            " ",
            ".",
            "..",
            "...",
            "../skill",
            "./skill",
            "/skill",
            "skill/name",
            "skill\\name",
            "skill..name",
            "bad\0name",
        ] {
            assert!(
                sanitize_remote_skill_name(invalid).is_none(),
                "expected `{invalid}` to be rejected"
            );
        }
    }

    #[test]
    fn discovers_remote_skill_urls_and_reuses_cached_copy_when_source_is_offline() {
        let home = scratch("home-remote-cache");
        let project = home.join("project");
        fs::create_dir_all(&project).expect("create project");
        let index = serde_json::json!({
            "skills": [{
                "name": "remote-skill",
                "files": ["SKILL.md", "references/guide.md"]
            }]
        })
        .to_string();
        let files = BTreeMap::from([
            (
                "/catalog/remote-skill/SKILL.md".to_owned(),
                skill_doc(
                    "remote-skill",
                    "Remote catalog skill",
                    "Append SKILL-REMOTE-CACHED.",
                ),
            ),
            (
                "/catalog/remote-skill/references/guide.md".to_owned(),
                "# remote guide\n".to_owned(),
            ),
        ]);
        let fixture = HttpFixture::start(index, files);
        fs::write(
            project.join("cortex.toml"),
            format!(
                r#"
[skills]
urls = ["{}"]
"#,
                fixture.base_url
            ),
        )
        .expect("write config");

        let first = discover(&ConfigInputs {
            home: Some(home.clone()),
            xdg_config_home: None,
            cwd: project.clone(),
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        });
        assert!(
            first.skills.contains_key("remote-skill"),
            "remote skill should be fetched from configured URL"
        );
        assert!(
            fixture.hits() > 0,
            "fixture should observe at least one network request"
        );
        fixture.join();

        let second = discover(&ConfigInputs {
            home: Some(home),
            xdg_config_home: None,
            cwd: project,
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        });
        assert!(
            second.skills.contains_key("remote-skill"),
            "cached remote skill should remain available while source is offline"
        );
    }

    #[test]
    fn skips_traversal_like_remote_skill_names_without_cache_escape() {
        let home = scratch("home-remote-traversal");
        let project = home.join("project");
        fs::create_dir_all(&project).expect("create project");
        let index = serde_json::json!({
            "skills": [
                {
                    "name": "..",
                    "files": ["SKILL.md"]
                },
                {
                    "name": "remote.skill",
                    "files": ["SKILL.md"]
                }
            ]
        })
        .to_string();
        let files = BTreeMap::from([
            (
                "/SKILL.md".to_owned(),
                skill_doc(
                    "escaped-skill",
                    "Traversal should never be cached",
                    "Append SKILL-ESCAPED.",
                ),
            ),
            (
                "/catalog/remote.skill/SKILL.md".to_owned(),
                skill_doc(
                    "remote-skill",
                    "Safe dotted remote skill",
                    "Append SKILL-REMOTE-SAFE.",
                ),
            ),
        ]);
        let fixture = HttpFixture::start(index, files);
        fs::write(
            project.join("cortex.toml"),
            format!(
                r#"
[skills]
urls = ["{}"]
"#,
                fixture.base_url
            ),
        )
        .expect("write config");

        let discovery = discover(&ConfigInputs {
            home: Some(home.clone()),
            xdg_config_home: None,
            cwd: project,
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        });

        let cache_root = home
            .join(".config")
            .join("cortex")
            .join("cache")
            .join("skills");
        let cache_dir = cache_root.join(hex_encode(fixture.base_url.as_bytes()));
        fixture.join();

        assert!(
            !cache_root.join("SKILL.md").is_file(),
            "remote traversal-like names must never write outside the source cache subtree"
        );
        assert!(
            !discovery.skills.contains_key("escaped-skill"),
            "traversal-like remote skills must be skipped"
        );
        assert!(
            discovery.skills.contains_key("remote-skill"),
            "safe dotted remote skill names should still be accepted"
        );
        assert!(
            cache_dir.join("remote.skill").join("SKILL.md").is_file(),
            "safe remote skill cache path should be written under the source cache subtree"
        );
    }
}
