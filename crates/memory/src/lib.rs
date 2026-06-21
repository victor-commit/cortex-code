//! Project memory discovery for AGENTS.md and configured instruction globs.
//!
//! Memory is loaded from the working-directory project chain and rendered as one system-prompt
//! block. The block lists sources in deterministic precedence order: global configured
//! instructions first, then for each ancestor directory from project root to the current working
//! directory, `AGENTS.md` followed by that directory's `instructions[]` config globs. Later entries
//! have higher precedence on conflicts.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use settings::{ConfigInputs, ConfigLayer};
use toml::Value;

const AGENTS_FILE: &str = "AGENTS.md";
const PROJECT_CONFIG_FILE: &str = "cortex.toml";
const PROJECT_CONFIG_DIR: &str = ".cortex";
const CONFIG_FILE: &str = "config.toml";

/// A discovered memory source that contributed non-empty instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySource {
    /// The source path, absolute when the run was invoked from an absolute working directory.
    pub path: PathBuf,
    /// Human-readable source kind, either `AGENTS.md` or `instructions[]`.
    pub kind: &'static str,
    /// The loaded markdown/text content.
    pub content: String,
}

/// All memory discovered for a project turn, plus non-fatal diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectMemory {
    sources: Vec<MemorySource>,
    warnings: Vec<String>,
}

impl ProjectMemory {
    /// The instruction sources in prompt order: lowest precedence first, highest precedence last.
    #[must_use]
    pub fn sources(&self) -> &[MemorySource] {
        &self.sources
    }

    /// Non-fatal diagnostics from memory discovery.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Whether no memory sources were loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Render the project memory block for insertion into the system prompt.
    #[must_use]
    pub fn render(&self) -> Option<String> {
        if self.sources.is_empty() {
            return None;
        }

        let mut out = String::from(
            "Project memory instructions are active. Sources are ordered from lowest to highest \
             precedence; when instructions conflict, later sources override earlier sources. \
             Follow all applicable project memory instructions.\n",
        );
        for source in &self.sources {
            out.push_str("\n--- project memory source: ");
            out.push_str(source.kind);
            out.push_str(" (");
            out.push_str(&source.path.display().to_string());
            out.push_str(") ---\n");
            out.push_str(source.content.trim());
            out.push('\n');
        }
        Some(out)
    }
}

/// Discover project memory from `AGENTS.md` files and config `instructions[]` globs.
#[must_use]
pub fn discover(inputs: &ConfigInputs) -> ProjectMemory {
    let mut memory = ProjectMemory::default();
    let layers = match settings::layer_tables(inputs) {
        Ok(layers) => layers,
        Err(err) => {
            memory.warnings.push(format!(
                "project memory could not read config layers: {err}"
            ));
            Vec::new()
        }
    };

    let dirs = project_dirs(inputs, &layers);
    let chain: BTreeSet<PathBuf> = dirs.iter().cloned().collect();
    let mut layer_sources = BTreeSet::new();
    let mut layer_by_dir: BTreeMap<PathBuf, Vec<ConfigLayer>> = BTreeMap::new();
    let mut global_layers = Vec::new();

    for layer in layers {
        let base = config_base_dir(&layer.source);
        if chain.contains(&base) {
            layer_by_dir.entry(base).or_default().push(layer);
        } else {
            global_layers.push(layer);
        }
    }

    for layer in &global_layers {
        append_instruction_sources(layer, &mut memory, &mut layer_sources);
    }

    for dir in dirs {
        append_agents_source(&dir, &mut memory, &mut layer_sources);
        if let Some(layers) = layer_by_dir.get(&dir) {
            for layer in layers {
                append_instruction_sources(layer, &mut memory, &mut layer_sources);
            }
        }
    }

    memory
}

fn project_dirs(inputs: &ConfigInputs, layers: &[ConfigLayer]) -> Vec<PathBuf> {
    let mut ancestors = ancestor_chain(inputs.home.as_deref(), &inputs.cwd);
    if ancestors.is_empty() {
        ancestors.push(inputs.cwd.clone());
    }

    let mut marked = BTreeSet::new();
    for dir in &ancestors {
        if has_project_marker(dir) {
            marked.insert(dir.clone());
        }
    }
    for layer in layers {
        let base = config_base_dir(&layer.source);
        if is_ancestor_or_self(&base, &inputs.cwd) {
            marked.insert(base);
        }
    }

    let root = ancestors
        .iter()
        .rev()
        .find(|dir| marked.contains(*dir))
        .cloned()
        .unwrap_or_else(|| inputs.cwd.clone());

    ancestors.reverse();
    let start = ancestors
        .iter()
        .position(|dir| dir == &root)
        .unwrap_or_else(|| ancestors.len().saturating_sub(1));
    ancestors[start..].to_vec()
}

fn ancestor_chain(home: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut current = Some(cwd.to_path_buf());
    while let Some(dir) = current {
        if Some(dir.as_path()) == home {
            break;
        }
        let parent = dir.parent().map(Path::to_path_buf);
        dirs.push(dir);
        current = parent;
    }
    dirs
}

fn has_project_marker(dir: &Path) -> bool {
    dir.join(".git").exists()
        || dir.join(PROJECT_CONFIG_FILE).is_file()
        || dir.join(PROJECT_CONFIG_DIR).join(CONFIG_FILE).is_file()
        || dir.join(AGENTS_FILE).is_file()
}

fn is_ancestor_or_self(ancestor: &Path, path: &Path) -> bool {
    path == ancestor || path.starts_with(ancestor)
}

fn config_base_dir(path: &Path) -> PathBuf {
    let Some(parent) = path.parent() else {
        return PathBuf::from(".");
    };
    if path.file_name().and_then(|name| name.to_str()) == Some(CONFIG_FILE)
        && parent.file_name().and_then(|name| name.to_str()) == Some(PROJECT_CONFIG_DIR)
    {
        return parent.parent().unwrap_or(parent).to_path_buf();
    }
    parent.to_path_buf()
}

fn append_agents_source(dir: &Path, memory: &mut ProjectMemory, seen: &mut BTreeSet<PathBuf>) {
    let path = dir.join(AGENTS_FILE);
    if path.is_file() {
        append_file_source(path, "AGENTS.md", memory, seen);
    }
}

fn append_instruction_sources(
    layer: &ConfigLayer,
    memory: &mut ProjectMemory,
    seen: &mut BTreeSet<PathBuf>,
) {
    let base = config_base_dir(&layer.source);
    for pattern in instruction_patterns(&layer.table, &layer.source, &mut memory.warnings) {
        let mut matches = expand_instruction_glob(&base, &pattern, &mut memory.warnings);
        matches.sort();
        for path in matches {
            append_file_source(path, "instructions[]", memory, seen);
        }
    }
}

fn instruction_patterns(
    table: &toml::value::Table,
    source: &Path,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let Some(value) = table.get("instructions") else {
        return Vec::new();
    };
    match value {
        Value::String(pattern) => vec![pattern.to_owned()],
        Value::Array(items) => items
            .iter()
            .filter_map(|value| match value {
                Value::String(pattern) => Some(pattern.to_owned()),
                other => {
                    warnings.push(format!(
                        "{}: ignoring non-string instructions[] entry of type {}",
                        source.display(),
                        other.type_str()
                    ));
                    None
                }
            })
            .collect(),
        other => {
            warnings.push(format!(
                "{}: ignoring instructions value of type {}; expected a string or array of strings",
                source.display(),
                other.type_str()
            ));
            Vec::new()
        }
    }
}

fn expand_instruction_glob(base: &Path, pattern: &str, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let path = PathBuf::from(trimmed);
    let resolved = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    let pattern = resolved.to_string_lossy().replace('\\', "/");
    let entries = match glob::glob(&pattern) {
        Ok(entries) => entries,
        Err(err) => {
            warnings.push(format!("invalid instructions glob `{trimmed}`: {err}"));
            return Vec::new();
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(path) if path.is_file() => paths.push(path),
            Ok(_) => {}
            Err(err) => warnings.push(format!("instructions glob `{trimmed}`: {err}")),
        }
    }
    paths
}

fn append_file_source(
    path: PathBuf,
    kind: &'static str,
    memory: &mut ProjectMemory,
    seen: &mut BTreeSet<PathBuf>,
) {
    let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if !seen.insert(key) {
        return;
    }
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                memory.sources.push(MemorySource {
                    path,
                    kind,
                    content: trimmed.to_owned(),
                });
            }
        }
        Err(err) => memory.warnings.push(format!(
            "failed to read project memory {}: {err}",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("cortex-memory-{tag}-{}-{id}", std::process::id()));
        let home = root.join("home");
        let proj = root.join("project");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&proj).expect("project");
        (root, home, proj)
    }

    fn inputs(home: &Path, cwd: &Path) -> ConfigInputs {
        ConfigInputs {
            home: Some(home.to_path_buf()),
            xdg_config_home: None,
            cwd: cwd.to_path_buf(),
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        }
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, contents).expect("write");
    }

    fn source_names(memory: &ProjectMemory) -> Vec<String> {
        memory
            .sources()
            .iter()
            .map(|source| {
                source
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn no_memory_sources_is_a_clean_no_op() {
        let (_root, home, proj) = scratch("empty");
        let memory = discover(&inputs(&home, &proj));
        assert!(memory.is_empty());
        assert!(memory.render().is_none());
        assert!(memory.warnings().is_empty());
    }

    #[test]
    fn loads_root_and_nested_agents_in_directory_order() {
        let (_root, home, proj) = scratch("agents");
        let sub = proj.join("sub");
        fs::create_dir_all(&sub).expect("sub");
        write(&proj.join(AGENTS_FILE), "ROOT-MEM");
        write(&sub.join(AGENTS_FILE), "SUB-MEM");

        let memory = discover(&inputs(&home, &sub));
        assert_eq!(source_names(&memory), vec!["AGENTS.md", "AGENTS.md"]);
        let rendered = memory.render().expect("rendered");
        let root_pos = rendered.find("ROOT-MEM").expect("root");
        let sub_pos = rendered.find("SUB-MEM").expect("sub");
        assert!(root_pos < sub_pos, "root should precede nested source");
    }

    #[test]
    fn config_instruction_globs_are_sorted_and_concatenated_after_agents() {
        let (_root, home, proj) = scratch("instructions");
        write(&proj.join(AGENTS_FILE), "AGENTS-FIRST");
        write(
            &proj.join(PROJECT_CONFIG_FILE),
            r#"instructions = ["rules/*.md"]"#,
        );
        write(&proj.join("rules/b.md"), "B-RULE");
        write(&proj.join("rules/a.md"), "A-RULE");

        let memory = discover(&inputs(&home, &proj));
        assert_eq!(source_names(&memory), vec!["AGENTS.md", "a.md", "b.md"]);
        let rendered = memory.render().expect("rendered");
        let agents_pos = rendered.find("AGENTS-FIRST").expect("agents");
        let a_pos = rendered.find("A-RULE").expect("a");
        let b_pos = rendered.find("B-RULE").expect("b");
        assert!(agents_pos < a_pos && a_pos < b_pos, "rendered: {rendered}");
    }

    #[test]
    fn nested_config_instructions_follow_root_config_instructions() {
        let (_root, home, proj) = scratch("nested-config");
        let sub = proj.join("sub");
        fs::create_dir_all(&sub).expect("sub");
        write(
            &proj.join(PROJECT_CONFIG_FILE),
            r#"instructions = ["rules/root.md"]"#,
        );
        write(&proj.join("rules/root.md"), "ROOT-RULE");
        write(
            &sub.join(PROJECT_CONFIG_FILE),
            r#"instructions = ["rules/sub.md"]"#,
        );
        write(&sub.join("rules/sub.md"), "SUB-RULE");

        let rendered = discover(&inputs(&home, &sub)).render().expect("rendered");
        let root_pos = rendered.find("ROOT-RULE").expect("root");
        let sub_pos = rendered.find("SUB-RULE").expect("sub");
        assert!(root_pos < sub_pos, "root config must precede nested config");
    }

    #[test]
    fn missing_instruction_globs_are_absent_without_warnings() {
        let (_root, home, proj) = scratch("missing-glob");
        write(
            &proj.join(PROJECT_CONFIG_FILE),
            r#"instructions = ["missing/*.md"]"#,
        );

        let memory = discover(&inputs(&home, &proj));
        assert!(memory.is_empty());
        assert!(memory.warnings().is_empty());
    }
}
