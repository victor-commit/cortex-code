//! Built-in language-server definitions and file-extension routing.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::client::LspServerCommand;

/// How a missing language server can be installed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallPlan {
    /// Install a npm package globally (`npm install -g <package>`).
    Npm { package: String },
    /// Install a Python tool through uv (`uv tool install <package>`).
    UvTool { package: String },
}

impl InstallPlan {
    /// Human-readable description for diagnostics/status.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::Npm { package } => format!("npm install -g {package}"),
            Self::UvTool { package } => format!("uv tool install {package}"),
        }
    }
}

/// Configuration for one language server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LspServerConfig {
    language_id: String,
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    extensions: Vec<String>,
    root_markers: Vec<String>,
    request_timeout: Duration,
    install: Option<InstallPlan>,
}

impl LspServerConfig {
    /// Create a server config for `language_id` launched via `program`.
    pub fn new(language_id: impl Into<String>, program: impl Into<String>) -> Self {
        Self {
            language_id: language_id.into(),
            program: program.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            extensions: Vec::new(),
            root_markers: Vec::new(),
            request_timeout: Duration::from_secs(10),
            install: None,
        }
    }

    /// Append one launch argument.
    #[must_use]
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Append several launch arguments.
    #[must_use]
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Replace the launch program.
    #[must_use]
    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = program.into();
        self
    }

    /// Replace launch arguments.
    #[must_use]
    pub fn with_exact_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Set one launch-time environment variable.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Replace launch-time environment variables.
    #[must_use]
    pub fn with_envs<I, K, V>(mut self, env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.env = env
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self
    }

    /// Register a handled file extension.
    #[must_use]
    pub fn with_extension(mut self, ext: impl AsRef<str>) -> Self {
        let normalized = normalize_extension(ext.as_ref());
        if !normalized.is_empty() && !self.extensions.contains(&normalized) {
            self.extensions.push(normalized);
        }
        self
    }

    /// Register several file extensions.
    #[must_use]
    pub fn with_extensions<I, S>(mut self, exts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for ext in exts {
            self = self.with_extension(ext);
        }
        self
    }

    /// Replace handled file extensions.
    #[must_use]
    pub fn with_exact_extensions<I, S>(mut self, exts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.extensions.clear();
        self.with_extensions(exts)
    }

    /// Register a project root marker.
    #[must_use]
    pub fn with_root_marker(mut self, marker: impl Into<String>) -> Self {
        let marker = marker.into();
        if !marker.is_empty() && !self.root_markers.contains(&marker) {
            self.root_markers.push(marker);
        }
        self
    }

    /// Attach an install plan used only when the binary is missing.
    #[must_use]
    pub fn with_install(mut self, install: InstallPlan) -> Self {
        self.install = Some(install);
        self
    }

    /// Set the per-request timeout.
    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// LSP `languageId`.
    #[must_use]
    pub fn language_id(&self) -> &str {
        &self.language_id
    }

    /// Program to spawn.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Launch args.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Launch-time environment overrides.
    #[must_use]
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// Normalized handled extensions.
    #[must_use]
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    /// Root markers.
    #[must_use]
    pub fn root_markers(&self) -> &[String] {
        &self.root_markers
    }

    /// Install plan, if any.
    #[must_use]
    pub const fn install(&self) -> Option<&InstallPlan> {
        self.install.as_ref()
    }

    /// Whether this server handles `path` by extension.
    #[must_use]
    pub fn matches_path(&self, path: impl AsRef<Path>) -> bool {
        path.as_ref()
            .extension()
            .and_then(|os| os.to_str())
            .is_some_and(|ext| self.extensions.contains(&normalize_extension(ext)))
    }

    /// Convert to a spawnable command.
    #[must_use]
    pub fn to_command(&self) -> LspServerCommand {
        let mut command = LspServerCommand::new(self.program.clone())
            .with_args(self.args.iter().cloned())
            .with_request_timeout(self.request_timeout);
        for (key, value) in &self.env {
            command = command.with_env(key.clone(), value.clone());
        }
        command
    }
}

/// A registry of known language servers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LspConfigSet {
    servers: Vec<LspServerConfig>,
}

impl Default for LspConfigSet {
    fn default() -> Self {
        Self::defaults()
    }
}

impl LspConfigSet {
    /// Empty registry.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            servers: Vec::new(),
        }
    }

    /// Built-in server definitions used by Cortex Code.
    #[must_use]
    pub fn defaults() -> Self {
        Self::empty()
            .with_server(
                LspServerConfig::new("rust", "rust-analyzer")
                    .with_extension("rs")
                    .with_root_marker("Cargo.toml"),
            )
            .with_server(
                LspServerConfig::new("python", "ruff")
                    .with_arg("server")
                    .with_extensions(["py", "pyi"])
                    .with_root_marker("pyproject.toml")
                    .with_install(InstallPlan::UvTool {
                        package: "ruff".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("typescript", "typescript-language-server")
                    .with_arg("--stdio")
                    .with_extensions(["ts", "tsx", "js", "jsx"])
                    .with_root_marker("package.json")
                    .with_install(InstallPlan::Npm {
                        package: "typescript-language-server".to_owned(),
                    }),
            )
    }

    /// Build a config set from an explicit server list.
    #[must_use]
    pub fn from_servers(servers: Vec<LspServerConfig>) -> Self {
        Self { servers }
    }

    /// Add a server definition.
    #[must_use]
    pub fn with_server(mut self, server: LspServerConfig) -> Self {
        self.servers.push(server);
        self
    }

    /// All configured servers.
    #[must_use]
    pub fn servers(&self) -> &[LspServerConfig] {
        &self.servers
    }

    /// Resolve by file extension.
    #[must_use]
    pub fn resolve_for_path(&self, path: impl AsRef<Path>) -> Option<&LspServerConfig> {
        self.servers
            .iter()
            .find(|server| server.matches_path(&path))
    }

    /// Lookup by language id.
    #[must_use]
    pub fn get(&self, language_id: &str) -> Option<&LspServerConfig> {
        self.servers
            .iter()
            .find(|server| server.language_id == language_id)
    }
}

fn normalize_extension(ext: &str) -> String {
    ext.trim().trim_start_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_rust_python_and_typescript_by_extension() {
        let set = LspConfigSet::defaults();
        assert_eq!(
            set.resolve_for_path("src/main.rs")
                .map(LspServerConfig::language_id),
            Some("rust")
        );
        assert_eq!(
            set.resolve_for_path("pkg/app.py")
                .map(LspServerConfig::language_id),
            Some("python")
        );
        assert_eq!(
            set.resolve_for_path("web/app.ts")
                .map(LspServerConfig::language_id),
            Some("typescript")
        );
        assert!(set.resolve_for_path("README.md").is_none());
    }

    #[test]
    fn python_and_typescript_have_guarded_install_plans() {
        let set = LspConfigSet::defaults();
        assert!(matches!(
            set.get("python").and_then(LspServerConfig::install),
            Some(InstallPlan::UvTool { .. })
        ));
        assert!(matches!(
            set.get("typescript").and_then(LspServerConfig::install),
            Some(InstallPlan::Npm { .. })
        ));
        assert!(set.get("rust").and_then(LspServerConfig::install).is_none());
    }

    #[test]
    fn command_conversion_carries_configured_env() {
        let command = LspServerConfig::new("python", "ruff")
            .with_env("RUFF_CACHE_DIR", "/tmp/cache")
            .to_command();
        assert_eq!(command.program(), "ruff");
        assert_eq!(
            command.env(),
            &[("RUFF_CACHE_DIR".to_owned(), "/tmp/cache".to_owned())]
        );
    }
}
