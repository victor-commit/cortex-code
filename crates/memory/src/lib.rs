//! Project memory discovery for instruction files and configured instruction sources.
//!
//! Memory is loaded from the working-directory project chain and rendered as one system-prompt
//! block. Sources are ordered in deterministic precedence, global sources first and nested project
//! sources later, so later entries override earlier entries on conflicts.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;
use settings::{ConfigInputs, ConfigLayer};
use toml::Value;

const AGENTS_FILE: &str = "AGENTS.md";
const CLAUDE_FILE: &str = "CLAUDE.md";
const CONTEXT_FILE: &str = "CONTEXT.md";
const PROJECT_CONFIG_FILE: &str = "cortex.toml";
const PROJECT_CONFIG_DIR: &str = ".cortex";
const CONFIG_FILE: &str = "config.toml";

const KIND_AGENTS: &str = "AGENTS.md";
const KIND_CLAUDE: &str = "CLAUDE.md";
const KIND_CONTEXT: &str = "CONTEXT.md";
const KIND_INSTRUCTIONS: &str = "instructions[]";
const KIND_INSTRUCTION_URL: &str = "instruction URL";

/// A discovered memory source that contributed non-empty instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySource {
    /// The source path (filesystem path or URL identifier).
    pub path: PathBuf,
    /// Human-readable source kind.
    pub kind: &'static str,
    /// The loaded markdown/text content.
    pub content: String,
}

/// All memory discovered for a project turn, plus non-fatal diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectMemory {
    sources: Vec<MemorySource>,
    warnings: Vec<String>,
    net_blocked: bool,
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

    /// Whether no-memory discovery failed closed because networking is disabled.
    #[must_use]
    pub fn net_blocked(&self) -> bool {
        self.net_blocked
    }

    /// Whether no memory sources were loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Render the project memory block for insertion into the system prompt.
    #[must_use]
    pub fn render(&self) -> Option<String> {
        render_sources(
            &self.sources,
            "Project memory instructions are active. Sources are ordered from lowest to highest \
             precedence, when instructions conflict, later sources override earlier sources. \
             Follow all applicable project memory instructions.",
        )
    }
}

/// The result of refreshing dynamic memory after new file reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DynamicMemoryUpdate {
    pub rendered: Option<String>,
    pub warnings: Vec<String>,
    pub net_blocked: bool,
    pub new_sources: Vec<MemorySource>,
}

/// Tracks dynamically activated memory instructions during a turn.
#[derive(Debug, Clone)]
pub struct DynamicMemoryTracker {
    inputs: ConfigInputs,
    discovered_dirs: BTreeSet<PathBuf>,
    seen_source_keys: BTreeSet<String>,
}

impl DynamicMemoryTracker {
    #[must_use]
    pub fn new(inputs: ConfigInputs, memory: &ProjectMemory) -> Self {
        Self {
            inputs,
            discovered_dirs: BTreeSet::new(),
            seen_source_keys: memory
                .sources
                .iter()
                .map(|source| source_key(source.kind, &source.path))
                .collect(),
        }
    }

    #[must_use]
    pub fn update_for_reads(&mut self, read_paths: &[PathBuf]) -> DynamicMemoryUpdate {
        let mut added_dir = false;
        for path in read_paths {
            let dir = read_parent_dir(path);
            if self.discovered_dirs.insert(dir) {
                added_dir = true;
            }
        }
        if !added_dir {
            return DynamicMemoryUpdate::default();
        }

        let discovered = discover_for_paths(
            &self.inputs,
            &self.discovered_dirs.iter().cloned().collect::<Vec<_>>(),
        );
        let mut new_sources = Vec::new();
        for source in discovered.sources {
            let key = source_key(source.kind, &source.path);
            if self.seen_source_keys.insert(key) {
                new_sources.push(source);
            }
        }
        let rendered = render_sources(
            &new_sources,
            "Additional project memory instructions became active after reading files in this turn. \
             Sources are ordered from lowest to highest precedence.",
        );

        DynamicMemoryUpdate {
            rendered,
            warnings: discovered.warnings,
            net_blocked: discovered.net_blocked,
            new_sources,
        }
    }
}

/// Discover project memory from instruction files and config instruction sources.
#[must_use]
pub fn discover(inputs: &ConfigInputs) -> ProjectMemory {
    discover_for_paths(inputs, &[])
}

/// Discover project memory with extra path scopes (used for dynamic, read-driven memory loading).
#[must_use]
pub fn discover_for_paths(inputs: &ConfigInputs, extra_paths: &[PathBuf]) -> ProjectMemory {
    let mut memory = ProjectMemory::default();
    let network_disabled = no_net_enabled(&inputs.env);
    let layers = match settings::layer_tables(inputs) {
        Ok(layers) => layers,
        Err(err) => {
            memory.warnings.push(format!(
                "project memory could not read config layers: {err}"
            ));
            Vec::new()
        }
    };

    let mut dirs = project_dirs(inputs, &layers);
    for path in extra_paths {
        let dir = read_parent_dir(path);
        let mut scoped = inputs.clone();
        scoped.cwd = dir;
        for extra_dir in project_dirs(&scoped, &layers) {
            if !dirs.contains(&extra_dir) {
                dirs.push(extra_dir);
            }
        }
    }

    let chain: BTreeSet<PathBuf> = dirs.iter().cloned().collect();
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

    let mut seen_source_keys = BTreeSet::new();

    append_global_file_sources(inputs, &mut memory, &mut seen_source_keys);
    for layer in &global_layers {
        append_instruction_sources(layer, network_disabled, &mut memory, &mut seen_source_keys);
    }

    for dir in dirs {
        append_directory_file_sources(&dir, &mut memory, &mut seen_source_keys);
        if let Some(layers) = layer_by_dir.get(&dir) {
            for layer in layers {
                append_instruction_sources(
                    layer,
                    network_disabled,
                    &mut memory,
                    &mut seen_source_keys,
                );
            }
        }
    }

    memory
}

/// Render a formatted memory block for a set of sources.
#[must_use]
pub fn render_sources(sources: &[MemorySource], heading: &str) -> Option<String> {
    if sources.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(heading);
    out.push('\n');
    for source in sources {
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

fn read_parent_dir(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    }
}

fn append_global_file_sources(
    inputs: &ConfigInputs,
    memory: &mut ProjectMemory,
    seen: &mut BTreeSet<String>,
) {
    if let Some(home) = inputs.home.as_ref() {
        append_file_source(
            home.join(".claude").join(CLAUDE_FILE),
            KIND_CLAUDE,
            memory,
            seen,
        );
        append_file_source(
            home.join(".claude").join(CONTEXT_FILE),
            KIND_CONTEXT,
            memory,
            seen,
        );
        append_file_source(
            home.join(".cortex").join(AGENTS_FILE),
            KIND_AGENTS,
            memory,
            seen,
        );
    }

    for path in global_agents_candidates(inputs) {
        append_file_source(path, KIND_AGENTS, memory, seen);
    }
}

fn global_agents_candidates(inputs: &ConfigInputs) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let config_home = inputs
        .xdg_config_home
        .clone()
        .or_else(|| inputs.home.as_ref().map(|home| home.join(".config")));
    if let Some(config_home) = config_home {
        paths.push(config_home.join("cortex").join(AGENTS_FILE));
        paths.push(config_home.join(AGENTS_FILE));
    }
    paths
}

fn append_directory_file_sources(
    dir: &Path,
    memory: &mut ProjectMemory,
    seen: &mut BTreeSet<String>,
) {
    append_file_source(dir.join(CLAUDE_FILE), KIND_CLAUDE, memory, seen);
    append_file_source(dir.join(AGENTS_FILE), KIND_AGENTS, memory, seen);
    append_file_source(dir.join(CONTEXT_FILE), KIND_CONTEXT, memory, seen);
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
        || dir.join(CLAUDE_FILE).is_file()
        || dir.join(CONTEXT_FILE).is_file()
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

#[derive(Debug, Clone)]
enum InstructionEntry {
    Glob(String),
    Url(String),
}

fn append_instruction_sources(
    layer: &ConfigLayer,
    network_disabled: bool,
    memory: &mut ProjectMemory,
    seen: &mut BTreeSet<String>,
) {
    let base = config_base_dir(&layer.source);
    for entry in instruction_entries(&layer.table, &layer.source, &mut memory.warnings) {
        match entry {
            InstructionEntry::Glob(pattern) => {
                let mut matches = expand_instruction_glob(&base, &pattern, &mut memory.warnings);
                matches.sort();
                for path in matches {
                    append_file_source(path, KIND_INSTRUCTIONS, memory, seen);
                }
            }
            InstructionEntry::Url(url) => {
                append_remote_instruction_source(&url, network_disabled, memory, seen)
            }
        }
    }
}

fn instruction_entries(
    table: &toml::value::Table,
    source: &Path,
    warnings: &mut Vec<String>,
) -> Vec<InstructionEntry> {
    let mut entries = Vec::new();
    if let Some(value) = table.get("instructions") {
        collect_instruction_entries(value, source, warnings, &mut entries, EntryMode::Mixed);
    }
    if let Some(value) = table.get("instruction_urls") {
        collect_instruction_entries(value, source, warnings, &mut entries, EntryMode::UrlOnly);
    }
    if let Some(value) = table.get("instruction_url") {
        collect_instruction_entries(value, source, warnings, &mut entries, EntryMode::UrlOnly);
    }
    entries
}

#[derive(Debug, Clone, Copy)]
enum EntryMode {
    Mixed,
    GlobOnly,
    UrlOnly,
}

fn collect_instruction_entries(
    value: &Value,
    source: &Path,
    warnings: &mut Vec<String>,
    entries: &mut Vec<InstructionEntry>,
    mode: EntryMode,
) {
    match value {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return;
            }
            match mode {
                EntryMode::UrlOnly => {
                    if looks_like_remote_url(trimmed) {
                        entries.push(InstructionEntry::Url(trimmed.to_owned()));
                    } else {
                        warnings.push(format!(
                            "{}: ignoring non-URL instruction entry `{trimmed}`",
                            source.display()
                        ));
                    }
                }
                EntryMode::GlobOnly => entries.push(InstructionEntry::Glob(trimmed.to_owned())),
                EntryMode::Mixed => {
                    if looks_like_remote_url(trimmed) {
                        entries.push(InstructionEntry::Url(trimmed.to_owned()));
                    } else {
                        entries.push(InstructionEntry::Glob(trimmed.to_owned()));
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_instruction_entries(item, source, warnings, entries, mode);
            }
        }
        Value::Table(table) => {
            for key in ["url", "urls"] {
                if let Some(urls) = table.get(key) {
                    collect_instruction_entries(
                        urls,
                        source,
                        warnings,
                        entries,
                        EntryMode::UrlOnly,
                    );
                }
            }
            for key in [
                "glob", "globs", "pattern", "patterns", "path", "paths", "file", "files",
            ] {
                if let Some(globs) = table.get(key) {
                    collect_instruction_entries(
                        globs,
                        source,
                        warnings,
                        entries,
                        EntryMode::GlobOnly,
                    );
                }
            }
        }
        other => warnings.push(format!(
            "{}: ignoring instruction value of type {}; expected string, array, or table",
            source.display(),
            other.type_str()
        )),
    }
}

fn looks_like_remote_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn append_remote_instruction_source(
    url: &str,
    network_disabled: bool,
    memory: &mut ProjectMemory,
    seen: &mut BTreeSet<String>,
) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return;
    }
    let key = source_key(KIND_INSTRUCTION_URL, Path::new(trimmed));
    if !seen.insert(key) {
        return;
    }

    if network_disabled {
        memory.net_blocked = true;
        memory.warnings.push(format!(
            "network is disabled by CORTEX_NO_NET=1, cannot fetch instruction URL `{trimmed}`"
        ));
        return;
    }

    let client = match Client::builder().timeout(Duration::from_secs(10)).build() {
        Ok(client) => client,
        Err(err) => {
            memory.warnings.push(format!(
                "failed to initialize HTTP client for instruction URL `{trimmed}`: {err}"
            ));
            return;
        }
    };

    let response = match client.get(trimmed).send() {
        Ok(response) => response,
        Err(err) => {
            memory.warnings.push(format!(
                "failed to fetch instruction URL `{trimmed}`: {err}"
            ));
            return;
        }
    };

    if !response.status().is_success() {
        memory.warnings.push(format!(
            "failed to fetch instruction URL `{trimmed}`: HTTP {}",
            response.status()
        ));
        return;
    }

    match response.text() {
        Ok(contents) => {
            let trimmed_contents = contents.trim();
            if !trimmed_contents.is_empty() {
                memory.sources.push(MemorySource {
                    path: PathBuf::from(trimmed),
                    kind: KIND_INSTRUCTION_URL,
                    content: trimmed_contents.to_owned(),
                });
            }
        }
        Err(err) => memory.warnings.push(format!(
            "failed to read instruction URL `{trimmed}` response body: {err}"
        )),
    }
}

fn no_net_enabled(env: &[(String, String)]) -> bool {
    env.iter().any(|(key, value)| {
        key == "CORTEX_NO_NET"
            && matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
    })
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
    seen: &mut BTreeSet<String>,
) {
    if !path.is_file() {
        return;
    }

    let key = source_key(kind, &path);
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

fn source_key(kind: &str, path: &Path) -> String {
    let normalized = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("{kind}:{}", normalized.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

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
    fn loads_claude_and_context_files() {
        let (_root, home, proj) = scratch("claude-context");
        write(&proj.join(CLAUDE_FILE), "CLAUDE-RULES");
        write(&proj.join(CONTEXT_FILE), "CONTEXT-RULES");

        let memory = discover(&inputs(&home, &proj));
        assert_eq!(source_names(&memory), vec!["CLAUDE.md", "CONTEXT.md"]);
    }

    #[test]
    fn global_claude_is_loaded_from_home() {
        let (_root, home, proj) = scratch("global-claude");
        write(&home.join(".claude").join(CLAUDE_FILE), "GLOBAL-CLAUDE");

        let memory = discover(&inputs(&home, &proj));
        let rendered = memory.render().expect("rendered");
        assert!(rendered.contains("GLOBAL-CLAUDE"));
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
    fn remote_instruction_url_is_loaded() {
        let (_root, home, proj) = scratch("remote-url");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\ncontent-type: text/plain\r\n\r\nREMOTE-RULE",
                )
                .expect("write");
        });
        write(
            &proj.join(PROJECT_CONFIG_FILE),
            &format!("instructions = [\"http://{addr}/memory.md\"]"),
        );

        let memory = discover(&inputs(&home, &proj));
        handle.join().expect("server");
        let rendered = memory.render().expect("rendered");
        assert!(rendered.contains("REMOTE-RULE"));
    }

    #[test]
    fn remote_instruction_url_fails_closed_when_network_disabled() {
        let (_root, home, proj) = scratch("remote-url-no-net");
        write(
            &proj.join(PROJECT_CONFIG_FILE),
            "instructions = [\"https://example.com/memory.md\"]",
        );
        let mut scoped_inputs = inputs(&home, &proj);
        scoped_inputs
            .env
            .push(("CORTEX_NO_NET".to_owned(), "1".to_owned()));

        let memory = discover(&scoped_inputs);
        assert!(memory.net_blocked());
        assert!(
            memory
                .warnings()
                .iter()
                .any(|warning| warning.contains("network is disabled"))
        );
    }

    #[test]
    fn dynamic_tracker_adds_new_nested_sources_after_reads() {
        let (_root, home, proj) = scratch("dynamic-read");
        let sub = proj.join("nested");
        fs::create_dir_all(&sub).expect("sub");
        write(&proj.join(AGENTS_FILE), "ROOT");
        write(&sub.join(AGENTS_FILE), "NESTED");

        let base = discover(&inputs(&home, &proj));
        assert_eq!(source_names(&base), vec!["AGENTS.md"]);

        let mut tracker = DynamicMemoryTracker::new(inputs(&home, &proj), &base);
        let update = tracker.update_for_reads(&[sub.join("file.rs")]);
        assert!(update.rendered.is_some());
        assert_eq!(update.new_sources.len(), 1);
        assert_eq!(update.new_sources[0].content, "NESTED");
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
