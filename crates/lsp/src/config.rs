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
    /// Install a Rust binary through Cargo (`cargo install --locked <package>`).
    Cargo { package: String },
    /// Install a Go toolchain binary (`go install <module>@latest`).
    Go { module: String },
    /// Install a .NET global tool (`dotnet tool install --global <package>`).
    DotnetTool { package: String },
    /// Install a Ruby gem executable (`gem install <package>`).
    Gem { package: String },
}

impl InstallPlan {
    /// Stable source identifier used for status and install notes.
    #[must_use]
    pub const fn source(&self) -> &'static str {
        match self {
            Self::Npm { .. } => "npm",
            Self::UvTool { .. } => "uv",
            Self::Cargo { .. } => "cargo",
            Self::Go { .. } => "go",
            Self::DotnetTool { .. } => "dotnet",
            Self::Gem { .. } => "gem",
        }
    }

    /// Human-readable package/module identifier used for diagnostics/status.
    #[must_use]
    pub fn target(&self) -> &str {
        match self {
            Self::Npm { package }
            | Self::UvTool { package }
            | Self::Cargo { package }
            | Self::DotnetTool { package }
            | Self::Gem { package } => package,
            Self::Go { module } => module,
        }
    }

    /// Human-readable description for diagnostics/status.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::Npm { package } => format!("npm install -g {package}"),
            Self::UvTool { package } => format!("uv tool install {package}"),
            Self::Cargo { package } => format!("cargo install --locked {package}"),
            Self::Go { module } => format!("go install {module}@latest"),
            Self::DotnetTool { package } => {
                format!("dotnet tool install --global {package}")
            }
            Self::Gem { package } => format!("gem install {package}"),
        }
    }
}

/// Configuration for one language server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LspServerConfig {
    language_id: String,
    enabled: bool,
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
            enabled: true,
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

    /// Enable or disable this server definition.
    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
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

    /// Whether the server definition is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
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
        self.enabled
            && path
                .as_ref()
                .extension()
                .and_then(|os| os.to_str())
                .is_some_and(|ext| self.extensions.contains(&normalize_extension(ext)))
    }

    /// Convert to a spawnable command.
    #[must_use]
    pub fn to_command(&self) -> LspServerCommand {
        self.to_command_with_program(self.program.clone())
    }

    /// Convert to a spawnable command while overriding the launch program path.
    #[must_use]
    pub fn to_command_with_program(&self, program: impl Into<String>) -> LspServerCommand {
        let mut command = LspServerCommand::new(program)
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
            .with_server(
                LspServerConfig::new("go", "gopls")
                    .with_extension("go")
                    .with_root_marker("go.work")
                    .with_root_marker("go.mod")
                    .with_install(InstallPlan::Go {
                        module: "golang.org/x/tools/gopls".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("cpp", "clangd")
                    .with_extensions(["c", "cc", "cpp", "cxx", "h", "hh", "hpp", "hxx"]),
            )
            .with_server(
                LspServerConfig::new("java", "jdtls")
                    .with_extension("java")
                    .with_root_marker("pom.xml")
                    .with_root_marker("build.gradle")
                    .with_root_marker("build.gradle.kts"),
            )
            .with_server(
                LspServerConfig::new("zig", "zls")
                    .with_extension("zig")
                    .with_root_marker("build.zig")
                    .with_root_marker("build.zig.zon"),
            )
            .with_server(
                LspServerConfig::new("csharp", "roslyn-language-server")
                    .with_args(["--stdio", "--autoLoadProjects"])
                    .with_extensions(["cs", "csx"])
                    .with_root_marker(".sln")
                    .with_root_marker(".slnx")
                    .with_root_marker(".csproj")
                    .with_root_marker("global.json")
                    .with_install(InstallPlan::DotnetTool {
                        package: "roslyn-language-server".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("razor", "roslyn-language-server")
                    .with_args(["--stdio", "--autoLoadProjects"])
                    .with_extensions(["razor", "cshtml"])
                    .with_root_marker(".sln")
                    .with_root_marker(".slnx")
                    .with_root_marker(".csproj")
                    .with_root_marker("global.json")
                    .with_install(InstallPlan::DotnetTool {
                        package: "roslyn-language-server".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("ruby", "ruby-lsp")
                    .with_extensions(["rb", "rake", "gemspec", "ru"])
                    .with_root_marker("Gemfile")
                    .with_install(InstallPlan::Gem {
                        package: "ruby-lsp".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("nix", "nixd")
                    .with_extension("nix")
                    .with_root_marker("flake.nix"),
            )
            .with_server(
                LspServerConfig::new("terraform", "terraform-ls")
                    .with_arg("serve")
                    .with_extensions(["tf", "tfvars", "hcl"])
                    .with_root_marker(".terraform")
                    .with_install(InstallPlan::Go {
                        module: "github.com/hashicorp/terraform-ls".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("elixir", "elixir-ls")
                    .with_extensions(["ex", "exs"])
                    .with_root_marker("mix.exs"),
            )
            .with_server(
                LspServerConfig::new("dart", "dart")
                    .with_args(["language-server", "--protocol=lsp"])
                    .with_extension("dart")
                    .with_root_marker("pubspec.yaml"),
            )
            .with_server(
                LspServerConfig::new("kotlin", "kotlin-lsp")
                    .with_extensions(["kt", "kts"])
                    .with_root_marker("settings.gradle")
                    .with_root_marker("settings.gradle.kts")
                    .with_root_marker("pom.xml"),
            )
            .with_server(
                LspServerConfig::new("tinymist", "tinymist")
                    .with_extensions(["typ", "typc"])
                    .with_root_marker("typst.toml")
                    .with_install(InstallPlan::Cargo {
                        package: "tinymist".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("vue", "vue-language-server")
                    .with_arg("--stdio")
                    .with_extension("vue")
                    .with_root_marker("package.json")
                    .with_install(InstallPlan::Npm {
                        package: "@vue/language-server".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("svelte", "svelteserver")
                    .with_arg("--stdio")
                    .with_extension("svelte")
                    .with_root_marker("package.json")
                    .with_install(InstallPlan::Npm {
                        package: "svelte-language-server".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("astro", "astro-ls")
                    .with_arg("--stdio")
                    .with_extension("astro")
                    .with_root_marker("package.json")
                    .with_install(InstallPlan::Npm {
                        package: "@astrojs/language-server".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("yaml", "yaml-language-server")
                    .with_arg("--stdio")
                    .with_extensions(["yaml", "yml"])
                    .with_install(InstallPlan::Npm {
                        package: "yaml-language-server".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("bash", "bash-language-server")
                    .with_arg("start")
                    .with_extensions(["sh", "bash", "zsh"])
                    .with_install(InstallPlan::Npm {
                        package: "bash-language-server".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("dockerfile", "docker-langserver")
                    .with_arg("--stdio")
                    .with_extension("dockerfile")
                    .with_install(InstallPlan::Npm {
                        package: "dockerfile-language-server-nodejs".to_owned(),
                    }),
            )
            .with_server(
                LspServerConfig::new("prisma", "prisma-language-server")
                    .with_arg("--stdio")
                    .with_extension("prisma")
                    .with_root_marker("schema.prisma")
                    .with_install(InstallPlan::Npm {
                        package: "@prisma/language-server".to_owned(),
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
    fn resolves_broad_language_matrix_by_extension() {
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
        assert_eq!(
            set.resolve_for_path("pkg/main.go")
                .map(LspServerConfig::language_id),
            Some("go")
        );
        assert_eq!(
            set.resolve_for_path("native/main.cpp")
                .map(LspServerConfig::language_id),
            Some("cpp")
        );
        assert_eq!(
            set.resolve_for_path("app/Main.java")
                .map(LspServerConfig::language_id),
            Some("java")
        );
        assert_eq!(
            set.resolve_for_path("pkg/main.zig")
                .map(LspServerConfig::language_id),
            Some("zig")
        );
        assert_eq!(
            set.resolve_for_path("infra/main.tf")
                .map(LspServerConfig::language_id),
            Some("terraform")
        );
        assert_eq!(
            set.resolve_for_path("nix/flake.nix")
                .map(LspServerConfig::language_id),
            Some("nix")
        );
        assert_eq!(
            set.resolve_for_path("typesetting/paper.typ")
                .map(LspServerConfig::language_id),
            Some("tinymist")
        );
        assert_eq!(
            set.resolve_for_path("frontend/App.vue")
                .map(LspServerConfig::language_id),
            Some("vue")
        );
        assert!(set.resolve_for_path("README.md").is_none());
    }

    #[test]
    fn install_plans_cover_uv_npm_go_cargo_dotnet_and_gem() {
        let set = LspConfigSet::defaults();
        assert!(matches!(
            set.get("python").and_then(LspServerConfig::install),
            Some(InstallPlan::UvTool { .. })
        ));
        assert!(matches!(
            set.get("typescript").and_then(LspServerConfig::install),
            Some(InstallPlan::Npm { .. })
        ));
        assert!(matches!(
            set.get("go").and_then(LspServerConfig::install),
            Some(InstallPlan::Go { .. })
        ));
        assert!(matches!(
            set.get("tinymist").and_then(LspServerConfig::install),
            Some(InstallPlan::Cargo { .. })
        ));
        assert!(matches!(
            set.get("csharp").and_then(LspServerConfig::install),
            Some(InstallPlan::DotnetTool { .. })
        ));
        assert!(matches!(
            set.get("razor").and_then(LspServerConfig::install),
            Some(InstallPlan::DotnetTool { .. })
        ));
        assert!(matches!(
            set.get("ruby").and_then(LspServerConfig::install),
            Some(InstallPlan::Gem { .. })
        ));
        assert!(set.get("rust").and_then(LspServerConfig::install).is_none());
    }

    #[test]
    fn install_plan_descriptions_include_source_and_target() {
        let plans = [
            InstallPlan::Npm {
                package: "typescript-language-server".to_owned(),
            },
            InstallPlan::UvTool {
                package: "ruff".to_owned(),
            },
            InstallPlan::Go {
                module: "golang.org/x/tools/gopls".to_owned(),
            },
            InstallPlan::Cargo {
                package: "tinymist".to_owned(),
            },
            InstallPlan::DotnetTool {
                package: "roslyn-language-server".to_owned(),
            },
            InstallPlan::Gem {
                package: "ruby-lsp".to_owned(),
            },
        ];

        for plan in plans {
            assert!(
                !plan.source().is_empty(),
                "install plan source should be stable"
            );
            assert!(
                !plan.target().is_empty(),
                "install plan target should be stable"
            );
            assert!(
                plan.description().contains(plan.target()),
                "description should contain target: {}",
                plan.description()
            );
        }
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

    #[test]
    fn disabled_servers_do_not_resolve_by_extension() {
        let set = LspConfigSet::from_servers(vec![
            LspServerConfig::new("python", "ruff")
                .with_extension("py")
                .with_enabled(false),
        ]);
        assert!(set.resolve_for_path("main.py").is_none());
        assert!(set.get("python").is_some());
    }
}
