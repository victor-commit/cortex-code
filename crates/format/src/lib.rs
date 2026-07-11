//! Real post-mutation formatter selection and execution.
//!
//! The runner is intentionally small and side-effecting: once a mutating tool has written a file,
//! the surface asks this crate whether a real formatter applies to that path. Matching requires the
//! language extension, the formatter binary on `PATH`, and, for project-scoped formatters, a config
//! marker in the file's ancestor tree. Formatting is in-place and uses the real external binaries.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use toml::Value;

/// A formatter family Cortex knows how to run after a file mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FormatterKind {
    /// `gofmt -w`.
    Gofmt,
    /// `biome format --write`.
    Biome,
    /// `mix format`.
    Mix,
    /// `clang-format -i`.
    ClangFormat,
    /// `zig fmt`.
    ZigFmt,
    /// `ktlint -F`.
    Ktlint,
    /// `dart format`.
    DartFormat,
    /// `terraform fmt`.
    TerraformFmt,
    /// `shfmt -w`.
    Shfmt,
    /// `nixfmt`.
    Nixfmt,
    /// `ocamlformat -i`.
    Ocamlformat,
    /// `rubocop --autocorrect`.
    Rubocop,
    /// `standardrb --fix`.
    Standardrb,
    /// `htmlbeautifier`.
    Erb,
    /// `gleam format`.
    Gleam,
    /// `cljfmt fix --quiet`.
    Cljfmt,
    /// `ormolu -i`.
    Ormolu,
    /// `latexindent -w -s`.
    Latexindent,
    /// `air format`.
    Air,
    /// `rustfmt --edition 2024`.
    Rustfmt,
    /// `prettier --write`.
    Prettier,
    /// `ruff format`.
    Ruff,
    /// A user-configured command from `[formatter.custom]`.
    Custom,
}

impl FormatterKind {
    /// Stable formatter identifier used in config keys and status output.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Gofmt => "gofmt",
            Self::Biome => "biome",
            Self::Mix => "mix",
            Self::ClangFormat => "clang_format",
            Self::ZigFmt => "zig_fmt",
            Self::Ktlint => "ktlint",
            Self::DartFormat => "dart",
            Self::TerraformFmt => "terraform",
            Self::Shfmt => "shfmt",
            Self::Nixfmt => "nixfmt",
            Self::Ocamlformat => "ocamlformat",
            Self::Rubocop => "rubocop",
            Self::Standardrb => "standardrb",
            Self::Erb => "erb",
            Self::Gleam => "gleam",
            Self::Cljfmt => "cljfmt",
            Self::Ormolu => "ormolu",
            Self::Latexindent => "latexindent",
            Self::Air => "air",
            Self::Rustfmt => "rustfmt",
            Self::Prettier => "prettier",
            Self::Ruff => "ruff",
            Self::Custom => "custom",
        }
    }

    /// The executable name expected on `PATH`.
    #[must_use]
    pub fn binary(self) -> &'static str {
        match self {
            Self::Gofmt => "gofmt",
            Self::Biome => "biome",
            Self::Mix => "mix",
            Self::ClangFormat => "clang-format",
            Self::ZigFmt => "zig",
            Self::Ktlint => "ktlint",
            Self::DartFormat => "dart",
            Self::TerraformFmt => "terraform",
            Self::Shfmt => "shfmt",
            Self::Nixfmt => "nixfmt",
            Self::Ocamlformat => "ocamlformat",
            Self::Rubocop => "rubocop",
            Self::Standardrb => "standardrb",
            Self::Erb => "htmlbeautifier",
            Self::Gleam => "gleam",
            Self::Cljfmt => "cljfmt",
            Self::Ormolu => "ormolu",
            Self::Latexindent => "latexindent",
            Self::Air => "air",
            Self::Rustfmt => "rustfmt",
            Self::Prettier => "prettier",
            Self::Ruff => "ruff",
            Self::Custom => "custom",
        }
    }

    /// Stable human-readable formatter label for notes and diagnostics.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Gofmt => "gofmt",
            Self::Biome => "biome",
            Self::Mix => "mix",
            Self::ClangFormat => "clang-format",
            Self::ZigFmt => "zig fmt",
            Self::Ktlint => "ktlint",
            Self::DartFormat => "dart format",
            Self::TerraformFmt => "terraform fmt",
            Self::Shfmt => "shfmt",
            Self::Nixfmt => "nixfmt",
            Self::Ocamlformat => "ocamlformat",
            Self::Rubocop => "rubocop",
            Self::Standardrb => "standardrb",
            Self::Erb => "erb formatter",
            Self::Gleam => "gleam format",
            Self::Cljfmt => "cljfmt",
            Self::Ormolu => "ormolu",
            Self::Latexindent => "latexindent",
            Self::Air => "air",
            Self::Rustfmt => "rustfmt",
            Self::Prettier => "prettier",
            Self::Ruff => "ruff",
            Self::Custom => "custom formatter",
        }
    }
}

/// Why a file was not handed to a formatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// No supported extension matched the file.
    UnsupportedExtension,
    /// The matching formatter executable is absent or not runnable.
    MissingBinary { binary: &'static str },
    /// The language requires an explicit project marker and no marker was found.
    MissingConfigMarker { formatter: FormatterKind },
}

/// A formatter command failed after it was selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatFailure {
    /// The formatter that was attempted.
    pub formatter: FormatterKind,
    /// Process exit status code when the process exited normally.
    pub status: Option<i32>,
    /// Formatter stderr, lossy UTF-8 decoded and trimmed.
    pub stderr: String,
}

/// Result of a post-mutation formatter attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatOutcome {
    /// A real formatter was selected and exited successfully.
    Formatted(FormatterKind),
    /// No formatter was applicable or available, so the file was left untouched.
    Skipped(SkipReason),
    /// A real formatter was attempted but returned an error.
    Failed(FormatFailure),
}

/// Where a formatter's active command/config came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatterStatusSource {
    /// Using the built-in defaults.
    Default,
    /// Overridden by user config.
    Config,
}

impl FormatterStatusSource {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Config => "config",
        }
    }
}

/// Status category shown to users for each configured formatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatterStatusState {
    /// Formatter is runnable for files that match its extension + marker constraints.
    Enabled,
    /// Formatter is disabled by config.
    Disabled,
    /// Formatter command/binary is missing.
    Missing,
    /// Formatter needs a project marker and none was found.
    MissingConfigMarker,
}

impl FormatterStatusState {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Missing => "missing",
            Self::MissingConfigMarker => "missing_config_marker",
        }
    }
}

/// Formatter status row for CLI/server/TUI inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatterStatus {
    /// Stable formatter identifier.
    pub name: String,
    /// Formatter family.
    pub formatter: FormatterKind,
    /// Whether the row is defaulted or config-defined.
    pub source: FormatterStatusSource,
    /// Current state.
    pub state: FormatterStatusState,
    /// Effective command, if explicitly overridden.
    pub command: Option<Vec<String>>,
    /// Environment variable keys injected into the formatter process.
    pub env_keys: Vec<String>,
    /// Extensions this formatter is configured to match.
    pub extensions: Vec<String>,
    /// Whether this formatter requires a project marker.
    pub marker_required: bool,
    /// Whether the marker was found under the runner root (when required).
    pub marker_found: Option<bool>,
}

/// Render a formatter failure as model-visible feedback, with stderr/status redacted.
#[must_use]
pub fn format_failure_note(failure: &FormatFailure) -> String {
    let status = failure
        .status
        .map(|status| format!("exit {status}"))
        .unwrap_or_else(|| "spawn failure".to_owned());
    let stderr = if failure.stderr.is_empty() {
        "no stderr".to_owned()
    } else {
        failure.stderr.clone()
    };
    redaction::redact(&format!(
        "Formatter {} failed ({status}); left the written bytes intact. stderr: {stderr}",
        failure.formatter.label()
    ))
}

/// Resolved formatter command for a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatterSelection {
    /// Which formatter to execute.
    pub kind: FormatterKind,
    /// Directory to execute the formatter from so project config discovery is stable.
    pub current_dir: PathBuf,
    search_root: PathBuf,
    command: FormatterCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FormatterCommand {
    Builtin {
        kind: FormatterKind,
        command: Option<Vec<String>>,
        env: BTreeMap<String, String>,
    },
    Custom(CustomFormatter),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct FormatterOverride {
    disabled: bool,
    command: Option<Vec<String>>,
    env: BTreeMap<String, String>,
    extensions: Option<Vec<String>>,
}

/// A configured formatter command that overrides built-in detection for matching extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFormatter {
    command: Vec<String>,
    extensions: Vec<String>,
    env: BTreeMap<String, String>,
}

impl CustomFormatter {
    /// Build a custom formatter from a command vector and extension list.
    ///
    /// The command may contain `$FILE`; if it does not, the target file path is appended as the last
    /// argument when the formatter runs. Extensions may be written with or without a leading dot.
    ///
    /// # Errors
    /// Returns a human-readable config error when the command or extension list is empty.
    pub fn new(
        command: Vec<String>,
        extensions: Vec<String>,
        env: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let command = command
            .into_iter()
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if command.is_empty() {
            return Err("custom formatter command must contain at least one string".to_owned());
        }

        let extensions = extensions
            .into_iter()
            .map(|extension| normalize_extension(&extension))
            .filter(|extension| !extension.is_empty())
            .collect::<Vec<_>>();
        if extensions.is_empty() {
            return Err(
                "custom formatter extensions must contain at least one extension".to_owned(),
            );
        }

        Ok(Self {
            command,
            extensions,
            env,
        })
    }

    fn matches_path(&self, path: &Path) -> bool {
        let Some(ext) = path.extension().and_then(OsStr::to_str) else {
            return false;
        };
        let file_ext = normalize_extension(ext);
        self.extensions
            .iter()
            .any(|candidate| candidate == "*" || candidate.eq_ignore_ascii_case(&file_ext))
    }
}

/// Formatter configuration loaded from the effective Cortex config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatterConfig {
    custom: Vec<CustomFormatter>,
    builtins_enabled: bool,
    builtin_overrides: BTreeMap<FormatterKind, FormatterOverride>,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            custom: Vec::new(),
            builtins_enabled: true,
            builtin_overrides: BTreeMap::new(),
        }
    }
}

impl FormatterConfig {
    /// Parse the top-level effective config table.
    ///
    /// Supported TOML shapes:
    ///
    /// ```toml
    /// [formatter.custom]
    /// command = ["/path/to/fmt", "$FILE"]
    /// extensions = [".foo"]
    /// ```
    ///
    /// `[[formatter.custom]]` and named subtables under `[formatter.custom.<name>]` are also
    /// accepted for multiple custom formatters. Built-ins can be disabled with
    /// `[formatter.<name>] disabled = true` (for example `prettier`, `biome`, `gofmt`).
    ///
    /// # Errors
    /// Returns a human-readable config error if a present formatter config has the wrong shape.
    pub fn from_table(table: &toml::value::Table) -> Result<Self, String> {
        let Some(value) = table.get("formatter") else {
            return Ok(Self::default());
        };
        Self::from_value(value)
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::Boolean(false) => Ok(Self {
                builtins_enabled: false,
                ..Self::default()
            }),
            Value::Boolean(true) => Ok(Self::default()),
            Value::Table(table) => {
                let mut config = Self {
                    builtin_overrides: parse_builtin_overrides(table)?,
                    ..Self::default()
                };
                if let Some(custom) = table.get("custom") {
                    config.custom = parse_custom_formatters(custom)?;
                }
                Ok(config)
            }
            other => Err(format!(
                "formatter must be a boolean or table, not {}",
                other.type_str()
            )),
        }
    }

    fn custom_for_path(&self, path: &Path) -> Vec<CustomFormatter> {
        self.custom
            .iter()
            .filter(|formatter| formatter.matches_path(path))
            .cloned()
            .collect()
    }

    fn builtin_kinds_for_path(&self, path: &Path) -> Vec<FormatterKind> {
        let default = kinds_for_path(path);
        builtin_kinds()
            .iter()
            .copied()
            .filter(|kind| {
                self.builtin_overrides
                    .get(kind)
                    .and_then(|override_| override_.extensions.as_ref())
                    .map_or_else(
                        || default.contains(kind),
                        |extensions| extension_list_matches_path(extensions, path),
                    )
            })
            .collect()
    }

    fn builtin_enabled(&self, kind: FormatterKind) -> bool {
        self.builtins_enabled
            && !self
                .builtin_overrides
                .get(&kind)
                .is_some_and(|override_| override_.disabled)
    }

    fn builtin_override(&self, kind: FormatterKind) -> FormatterOverride {
        self.builtin_overrides
            .get(&kind)
            .cloned()
            .unwrap_or_default()
    }
}

/// Real formatter runner rooted at one project/workspace directory.
#[derive(Debug, Clone)]
pub struct FormatterRunner {
    root: PathBuf,
    config: FormatterConfig,
}

impl FormatterRunner {
    /// Create a formatter runner. Relative roots are converted to an absolute path when possible.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_config(root, FormatterConfig::default())
    }

    /// Create a formatter runner with explicit formatter config.
    #[must_use]
    pub fn with_config(root: impl Into<PathBuf>, config: FormatterConfig) -> Self {
        let root = root.into();
        let root = absolutize_existing(root);
        Self { root, config }
    }

    /// Select the formatter that would be run for `path`, including binary/config gating.
    pub fn select(&self, path: &Path) -> Result<FormatterSelection, SkipReason> {
        self.select_chain(path)?
            .into_iter()
            .next()
            .ok_or(SkipReason::UnsupportedExtension)
    }

    /// Inspect formatter status for CLI/server/TUI surfaces.
    #[must_use]
    pub fn status(&self) -> Vec<FormatterStatus> {
        let mut rows = Vec::new();
        for kind in builtin_kinds().iter().copied() {
            let configured = self.config.builtin_overrides.contains_key(&kind);
            let override_ = self.config.builtin_override(kind);
            let marker_found = marker_presence_for_kind(kind, &self.root);
            let command_missing = override_
                .command
                .as_ref()
                .and_then(|command| command.first())
                .is_some_and(|program| !command_exists_for_root(program, &self.root));
            let binary_missing =
                override_.command.is_none() && !binary_available_in_root(kind.binary(), &self.root);
            let state = if !self.config.builtin_enabled(kind) {
                FormatterStatusState::Disabled
            } else if command_missing || binary_missing {
                FormatterStatusState::Missing
            } else if marker_found == Some(false) {
                FormatterStatusState::MissingConfigMarker
            } else {
                FormatterStatusState::Enabled
            };
            rows.push(FormatterStatus {
                name: kind.id().to_owned(),
                formatter: kind,
                source: if configured {
                    FormatterStatusSource::Config
                } else {
                    FormatterStatusSource::Default
                },
                state,
                command: override_.command.clone(),
                env_keys: override_.env.keys().cloned().collect(),
                extensions: override_
                    .extensions
                    .clone()
                    .unwrap_or_else(|| default_extensions_for_kind(kind)),
                marker_required: marker_found.is_some(),
                marker_found,
            });
        }

        let custom_len = self.config.custom.len();
        for (index, custom) in self.config.custom.iter().enumerate() {
            let name = if custom_len == 1 {
                "custom".to_owned()
            } else {
                format!("custom[{index}]")
            };
            let missing = custom
                .command
                .first()
                .is_none_or(|program| !command_exists_for_root(program, &self.root));
            rows.push(FormatterStatus {
                name,
                formatter: FormatterKind::Custom,
                source: FormatterStatusSource::Config,
                state: if missing {
                    FormatterStatusState::Missing
                } else {
                    FormatterStatusState::Enabled
                },
                command: Some(custom.command.clone()),
                env_keys: custom.env.keys().cloned().collect(),
                extensions: custom.extensions.clone(),
                marker_required: false,
                marker_found: None,
            });
        }

        rows
    }

    /// Run the applicable formatter in-place over `path`.
    ///
    /// `Skipped` outcomes never modify the file. `Formatted` means the real command exited 0.
    /// `Failed` reports the process failure; the caller decides how to surface it.
    #[must_use]
    pub fn format_path(&self, path: &Path) -> FormatOutcome {
        let selections = match self.select_chain(path) {
            Ok(selections) => selections,
            Err(reason) => return FormatOutcome::Skipped(reason),
        };
        let pre_format = fs::read(path).ok();
        let mut first_formatter = None;
        for selection in selections {
            first_formatter.get_or_insert(selection.kind);
            match run_formatter_once(path, &selection) {
                FormatOutcome::Formatted(_) => {}
                FormatOutcome::Failed(failure) => {
                    if let Some(bytes) = pre_format {
                        let _ = fs::write(path, bytes);
                    }
                    return FormatOutcome::Failed(failure);
                }
                FormatOutcome::Skipped(reason) => return FormatOutcome::Skipped(reason),
            }
        }
        FormatOutcome::Formatted(first_formatter.unwrap_or(FormatterKind::Custom))
    }

    fn select_chain(&self, path: &Path) -> Result<Vec<FormatterSelection>, SkipReason> {
        let custom = self.config.custom_for_path(path);
        if !custom.is_empty() {
            return Ok(custom
                .into_iter()
                .map(|custom| FormatterSelection {
                    kind: FormatterKind::Custom,
                    current_dir: self.root.clone(),
                    search_root: self.root.clone(),
                    command: FormatterCommand::Custom(custom),
                })
                .collect());
        }

        let kinds = self.config.builtin_kinds_for_path(path);
        if kinds.is_empty() {
            return Err(SkipReason::UnsupportedExtension);
        }

        let mut selections = Vec::new();
        let mut last_error = SkipReason::UnsupportedExtension;
        let mut record_error = |reason: SkipReason| {
            if matches!(last_error, SkipReason::UnsupportedExtension) {
                last_error = reason;
            }
        };
        for kind in kinds {
            if !self.config.builtin_enabled(kind) {
                continue;
            }
            let builtin_override = self.config.builtin_override(kind);
            if builtin_override.command.is_none()
                && !binary_available_in_root(kind.binary(), &self.root)
            {
                record_error(SkipReason::MissingBinary {
                    binary: kind.binary(),
                });
                continue;
            }
            match current_dir_for_kind(path, &self.root, kind) {
                Ok(current_dir) => selections.push(FormatterSelection {
                    kind,
                    current_dir,
                    search_root: self.root.clone(),
                    command: FormatterCommand::Builtin {
                        kind,
                        command: builtin_override.command,
                        env: builtin_override.env,
                    },
                }),
                Err(reason) => record_error(reason),
            }
        }
        if selections.is_empty() {
            Err(last_error)
        } else {
            Ok(selections)
        }
    }
}

/// Return whether `binary` resolves to a runnable executable.
///
/// Resolution checks `PATH` and known project-local formatter toolchain bins.
#[must_use]
pub fn binary_available(binary: &str) -> bool {
    let cwd = std::env::current_dir().ok();
    resolve_program_path(binary, cwd.as_deref()).is_some()
}

fn binary_available_in_root(binary: &str, root: &Path) -> bool {
    resolve_program_path(binary, Some(root)).is_some()
}

fn run_formatter_once(path: &Path, selection: &FormatterSelection) -> FormatOutcome {
    match &selection.command {
        FormatterCommand::Builtin { kind, command, env } => run_builtin_formatter(
            path,
            *kind,
            command.as_deref(),
            env,
            &selection.current_dir,
            &selection.search_root,
        ),
        FormatterCommand::Custom(custom) => {
            run_custom_formatter(path, custom, &selection.current_dir)
        }
    }
}

fn run_builtin_formatter(
    path: &Path,
    kind: FormatterKind,
    command_override: Option<&[String]>,
    env: &BTreeMap<String, String>,
    current_dir: &Path,
    search_root: &Path,
) -> FormatOutcome {
    if let Some(command_override) = command_override {
        return run_overridden_command(path, command_override, env, current_dir, kind);
    }

    let Some(program) = resolve_builtin_program(kind, search_root, current_dir) else {
        return FormatOutcome::Skipped(SkipReason::MissingBinary {
            binary: kind.binary(),
        });
    };
    let mut command = Command::new(program);
    configure_formatter_child(&mut command);
    command
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .envs(env);

    match kind {
        FormatterKind::Gofmt => {
            command.arg("-w").arg(path);
        }
        FormatterKind::Biome => {
            command.arg("format").arg("--write").arg(path);
        }
        FormatterKind::Mix => {
            command.arg("format").arg(path);
        }
        FormatterKind::ClangFormat => {
            command.arg("-i").arg(path);
        }
        FormatterKind::ZigFmt => {
            command.arg("fmt").arg(path);
        }
        FormatterKind::Ktlint => {
            command.arg("-F").arg(path);
        }
        FormatterKind::DartFormat => {
            command.arg("format").arg(path);
        }
        FormatterKind::TerraformFmt => {
            command.arg("fmt").arg(path);
        }
        FormatterKind::Shfmt => {
            command.arg("-w").arg(path);
        }
        FormatterKind::Nixfmt => {
            command.arg(path);
        }
        FormatterKind::Ocamlformat => {
            command.arg("-i").arg(path);
        }
        FormatterKind::Rubocop => {
            command.arg("--autocorrect").arg(path);
        }
        FormatterKind::Standardrb => {
            command.arg("--fix").arg(path);
        }
        FormatterKind::Erb => {
            command.arg(path);
        }
        FormatterKind::Gleam => {
            command.arg("format").arg(path);
        }
        FormatterKind::Cljfmt => {
            command.arg("fix").arg("--quiet").arg(path);
        }
        FormatterKind::Ormolu => {
            command.arg("-i").arg(path);
        }
        FormatterKind::Latexindent => {
            command.arg("-w").arg("-s").arg(path);
        }
        FormatterKind::Air => {
            command.arg("format").arg(path);
        }
        FormatterKind::Rustfmt => {
            command.arg("--edition").arg("2024").arg(path);
        }
        FormatterKind::Prettier => {
            command.arg("--write").arg(path);
        }
        FormatterKind::Ruff => {
            command.arg("format").arg(path);
        }
        FormatterKind::Custom => unreachable!("custom formatters are run separately"),
    }

    run_command(command, kind)
}

/// Resolve a built-in formatter executable, substituting a concrete toolchain `rustfmt` binary
/// when PATH resolves to rustup's launcher proxy. Formatter subprocesses intentionally run in a
/// secret-stripped environment, and a surface may also provide an isolated `HOME`; a rustup proxy
/// then cannot discover the toolchain that was available to the parent process. Running the
/// concrete binary preserves the real formatter behavior without exposing the parent's secrets.
fn resolve_builtin_program(
    kind: FormatterKind,
    search_root: &Path,
    current_dir: &Path,
) -> Option<PathBuf> {
    let program = resolve_program_path(kind.binary(), Some(search_root))?;
    if kind == FormatterKind::Rustfmt {
        return match concrete_rustfmt_for_rustup_proxy(&program, current_dir) {
            RustfmtProxyResolution::Concrete(program) => Some(program),
            RustfmtProxyResolution::Blocked => None,
            RustfmtProxyResolution::NotProxy => Some(program),
        };
    }
    Some(program)
}

/// The outcome of attempting to replace rustup's `rustfmt` launcher proxy.
enum RustfmtProxyResolution {
    /// A verified concrete formatter to execute instead of the proxy.
    Concrete(PathBuf),
    /// A project path toolchain was requested but failed validation.
    Blocked,
    /// The executable was not a rustup proxy, or rustup resolution did not apply.
    NotProxy,
}

/// A rustup toolchain selection with project-path failures kept explicit.
enum RustupToolchainSelection {
    /// A named toolchain to resolve beneath rustup's `toolchains` directory.
    Named(String),
    /// A verified concrete formatter from a project-local path toolchain.
    ProjectPath(PathBuf),
    /// A TOML path selection that was unsafe or not runnable.
    InvalidProjectPath,
}

/// Return a concrete `rustfmt` path when `program` is a rustup launcher proxy.
///
/// The formatter process intentionally receives a secret-stripped environment and may have an
/// isolated `HOME`. Resolve the proxy's toolchain before constructing that process so the concrete
/// binary follows rustup's selection rules from the formatter's project working directory.
fn concrete_rustfmt_for_rustup_proxy(program: &Path, project_cwd: &Path) -> RustfmtProxyResolution {
    let Ok(canonical_program) = program.canonicalize() else {
        return RustfmtProxyResolution::NotProxy;
    };
    if canonical_program.file_name() != Some(std::ffi::OsStr::new("rustup")) {
        return RustfmtProxyResolution::NotProxy;
    }

    for ancestor in program.ancestors() {
        let rustup_home = ancestor.join(".rustup");
        let toolchains = rustup_home.join("toolchains");
        if !toolchains.is_dir() {
            continue;
        }

        return match selected_rustup_toolchain(project_cwd, &rustup_home) {
            Some(RustupToolchainSelection::Named(toolchain)) => {
                concrete_rustfmt_in_toolchain(&toolchains, &toolchain).map_or(
                    RustfmtProxyResolution::NotProxy,
                    RustfmtProxyResolution::Concrete,
                )
            }
            Some(RustupToolchainSelection::ProjectPath(rustfmt)) => {
                RustfmtProxyResolution::Concrete(rustfmt)
            }
            Some(RustupToolchainSelection::InvalidProjectPath) => RustfmtProxyResolution::Blocked,
            None => {
                let mut candidates = fs::read_dir(&toolchains)
                    .ok()
                    .into_iter()
                    .flat_map(|entries| entries.filter_map(Result::ok))
                    .map(|entry| entry.path().join("bin").join("rustfmt"))
                    .filter(|path| is_executable_file(path))
                    .collect::<Vec<_>>();
                candidates.sort();
                candidates.into_iter().next().map_or(
                    RustfmtProxyResolution::NotProxy,
                    RustfmtProxyResolution::Concrete,
                )
            }
        };
    }

    RustfmtProxyResolution::NotProxy
}

fn concrete_rustfmt_in_toolchain(toolchains: &Path, toolchain: &str) -> Option<PathBuf> {
    let mut components = Path::new(toolchain).components();
    let std::path::Component::Normal(toolchain) = components.next()? else {
        return None;
    };
    if components.next().is_some() {
        return None;
    }

    let rustfmt = toolchains.join(toolchain).join("bin").join("rustfmt");
    is_executable_file(&rustfmt).then_some(rustfmt)
}

/// Select rustup's active toolchain before falling back to the configured default.
fn selected_rustup_toolchain(
    project_cwd: &Path,
    rustup_home: &Path,
) -> Option<RustupToolchainSelection> {
    std::env::var("RUSTUP_TOOLCHAIN")
        .ok()
        .filter(|toolchain| !toolchain.trim().is_empty())
        .map(RustupToolchainSelection::Named)
        .or_else(|| project_rustup_toolchain(project_cwd))
        .or_else(|| configured_rustup_toolchain(rustup_home).map(RustupToolchainSelection::Named))
}

/// Read the nearest project toolchain selection using rustup's TOML-before-legacy precedence.
fn project_rustup_toolchain(project_cwd: &Path) -> Option<RustupToolchainSelection> {
    project_cwd.ancestors().find_map(|directory| {
        read_toml_rust_toolchain(&directory.join("rust-toolchain.toml"))
            .or_else(|| read_legacy_rust_toolchain(&directory.join("rust-toolchain")))
    })
}

fn read_toml_rust_toolchain(path: &Path) -> Option<RustupToolchainSelection> {
    let parsed = fs::read_to_string(path).ok()?.parse::<toml::Value>().ok()?;
    let toolchain = parsed.get("toolchain")?.as_table()?;

    if let Some(configured_path) = toolchain.get("path") {
        return Some(
            configured_path
                .as_str()
                .and_then(|configured_path| concrete_rustfmt_in_project_path(path, configured_path))
                .map_or(
                    RustupToolchainSelection::InvalidProjectPath,
                    RustupToolchainSelection::ProjectPath,
                ),
        );
    }

    toolchain
        .get("channel")?
        .as_str()
        .map(|toolchain| RustupToolchainSelection::Named(toolchain.to_owned()))
}

/// Resolve a TOML `[toolchain].path` only when it remains below the TOML directory.
///
/// Rustup accepts project-local path toolchains. The formatter child has an isolated environment,
/// so resolve that path before spawning it, but never allow a path component or symlink to escape
/// the configuration directory. Canonicalizing both the toolchain and `bin/rustfmt` closes
/// symlink escapes at either level.
fn concrete_rustfmt_in_project_path(toml_path: &Path, configured_path: &str) -> Option<PathBuf> {
    let configured_path = Path::new(configured_path);
    if configured_path.as_os_str().is_empty()
        || configured_path.is_absolute()
        || configured_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return None;
    }

    let toml_directory = toml_path.parent()?.canonicalize().ok()?;
    let toolchain = toml_directory.join(configured_path).canonicalize().ok()?;
    if !toolchain.is_dir() || !toolchain.starts_with(&toml_directory) {
        return None;
    }

    let rustfmt = toolchain.join("bin").join("rustfmt").canonicalize().ok()?;
    (rustfmt.starts_with(&toml_directory) && is_executable_file(&rustfmt)).then_some(rustfmt)
}

fn read_legacy_rust_toolchain(path: &Path) -> Option<RustupToolchainSelection> {
    let toolchain = fs::read_to_string(path).ok()?;
    let toolchain = toolchain.trim();
    (!toolchain.is_empty()).then(|| RustupToolchainSelection::Named(toolchain.to_owned()))
}

/// Read rustup's selected default toolchain from its on-disk settings, if one exists.
fn configured_rustup_toolchain(rustup_home: &Path) -> Option<String> {
    fs::read_to_string(rustup_home.join("settings.toml"))
        .ok()?
        .parse::<toml::Value>()
        .ok()?
        .get("default_toolchain")?
        .as_str()
        .map(str::to_owned)
}

fn run_custom_formatter(
    path: &Path,
    custom: &CustomFormatter,
    current_dir: &Path,
) -> FormatOutcome {
    let mut command = Command::new(&custom.command[0]);
    configure_formatter_child(&mut command);
    command
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .envs(&custom.env);

    let file_arg = path.to_string_lossy().to_string();
    let mut saw_file_placeholder = false;
    for arg in custom.command.iter().skip(1) {
        if arg.contains("$FILE") {
            command.arg(arg.replace("$FILE", &file_arg));
            saw_file_placeholder = true;
        } else {
            command.arg(arg);
        }
    }
    if !saw_file_placeholder {
        command.arg(&file_arg);
    }

    run_command(command, FormatterKind::Custom)
}

fn run_overridden_command(
    path: &Path,
    command_parts: &[String],
    env: &BTreeMap<String, String>,
    current_dir: &Path,
    kind: FormatterKind,
) -> FormatOutcome {
    let Some(program) = command_parts.first() else {
        return FormatOutcome::Failed(FormatFailure {
            formatter: kind,
            status: None,
            stderr: "formatter command override is empty".to_owned(),
        });
    };
    let mut command = Command::new(program);
    configure_formatter_child(&mut command);
    command
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .envs(env);

    let file_arg = path.to_string_lossy().to_string();
    let mut saw_file_placeholder = false;
    for arg in command_parts.iter().skip(1) {
        if arg.contains("$FILE") {
            command.arg(arg.replace("$FILE", &file_arg));
            saw_file_placeholder = true;
        } else {
            command.arg(arg);
        }
    }
    if !saw_file_placeholder {
        command.arg(&file_arg);
    }
    run_command(command, kind)
}

fn run_command(mut command: Command, kind: FormatterKind) -> FormatOutcome {
    match command.output() {
        Ok(output) if output.status.success() => FormatOutcome::Formatted(kind),
        Ok(output) => FormatOutcome::Failed(FormatFailure {
            formatter: kind,
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }),
        Err(err) => FormatOutcome::Failed(FormatFailure {
            formatter: kind,
            status: None,
            stderr: err.to_string(),
        }),
    }
}

fn parse_builtin_overrides(
    table: &toml::value::Table,
) -> Result<BTreeMap<FormatterKind, FormatterOverride>, String> {
    let mut overrides = BTreeMap::new();
    for kind in builtin_kinds().iter().copied() {
        let name = kind.id();
        let Some(value) = table.get(name) else {
            continue;
        };
        let builtin = value
            .as_table()
            .ok_or_else(|| format!("formatter.{name} must be a table"))?;
        let disabled = match builtin.get("disabled") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| format!("formatter.{name}.disabled must be a boolean"))?,
            None => false,
        };
        let command = builtin
            .get("command")
            .map(parse_string_or_array)
            .transpose()?
            .map(|parts| normalize_command(parts, &format!("formatter.{name}.command")))
            .transpose()?;
        let env = builtin
            .get("env")
            .map(parse_string_table)
            .transpose()?
            .unwrap_or_default();
        let extensions = builtin
            .get("extensions")
            .map(parse_extensions)
            .transpose()?;
        if disabled || command.is_some() || !env.is_empty() || extensions.is_some() {
            overrides.insert(
                kind,
                FormatterOverride {
                    disabled,
                    command,
                    env,
                    extensions,
                },
            );
        }
    }
    Ok(overrides)
}

fn parse_extensions(value: &Value) -> Result<Vec<String>, String> {
    let normalized = parse_string_or_array(value)?
        .into_iter()
        .map(|extension| normalize_extension(&extension))
        .filter(|extension| !extension.is_empty())
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        return Err("formatter extensions must contain at least one extension".to_owned());
    }
    Ok(normalized)
}

fn parse_custom_formatters(value: &Value) -> Result<Vec<CustomFormatter>, String> {
    match value {
        Value::Table(table)
            if table.contains_key("command") || table.contains_key("extensions") =>
        {
            if table
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                Ok(Vec::new())
            } else {
                Ok(vec![parse_custom_formatter(table)?])
            }
        }
        Value::Table(table) => table
            .iter()
            .map(|(name, value)| {
                let entry = value
                    .as_table()
                    .ok_or_else(|| format!("formatter.custom.{name} must be a table"))?;
                if entry
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return Ok(None);
                }
                parse_custom_formatter(entry).map(Some)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|items| items.into_iter().flatten().collect()),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let entry = value
                    .as_table()
                    .ok_or_else(|| format!("formatter.custom[{index}] must be a table"))?;
                if entry
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return Ok(None);
                }
                parse_custom_formatter(entry).map(Some)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|items| items.into_iter().flatten().collect()),
        other => Err(format!(
            "formatter.custom must be a table or array of tables, not {}",
            other.type_str()
        )),
    }
}

fn parse_custom_formatter(table: &toml::value::Table) -> Result<CustomFormatter, String> {
    let command = table
        .get("command")
        .ok_or_else(|| "formatter.custom.command is required".to_owned())
        .and_then(parse_string_or_array)?;
    let extensions = table
        .get("extensions")
        .ok_or_else(|| "formatter.custom.extensions is required".to_owned())
        .and_then(parse_extensions)?;
    let env = table
        .get("env")
        .map(parse_string_table)
        .transpose()?
        .unwrap_or_default();
    CustomFormatter::new(
        normalize_command(command, "formatter.custom.command")?,
        extensions,
        env,
    )
}

fn parse_string_or_array(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(text) => Ok(vec![text.clone()]),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "expected an array of strings".to_owned())
            })
            .collect(),
        other => Err(format!(
            "expected a string or array of strings, not {}",
            other.type_str()
        )),
    }
}

fn parse_string_table(value: &Value) -> Result<BTreeMap<String, String>, String> {
    let table = value
        .as_table()
        .ok_or_else(|| format!("expected a table of strings, not {}", value.type_str()))?;
    let mut out = BTreeMap::new();
    for (key, value) in table {
        let text = value
            .as_str()
            .ok_or_else(|| format!("expected string for env.{key}, not {}", value.type_str()))?;
        out.insert(key.clone(), text.to_owned());
    }
    Ok(out)
}

fn normalize_command(parts: Vec<String>, key: &str) -> Result<Vec<String>, String> {
    let command = parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if command.is_empty() {
        return Err(format!("{key} must contain at least one non-empty string"));
    }
    Ok(command)
}

fn normalize_extension(extension: &str) -> String {
    let trimmed = extension.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed == "*" {
        return "*".to_owned();
    }
    let body = trimmed.trim_start_matches('.').to_ascii_lowercase();
    format!(".{body}")
}

fn builtin_kinds() -> &'static [FormatterKind] {
    &[
        FormatterKind::Gofmt,
        FormatterKind::Biome,
        FormatterKind::Mix,
        FormatterKind::ClangFormat,
        FormatterKind::ZigFmt,
        FormatterKind::Ktlint,
        FormatterKind::DartFormat,
        FormatterKind::TerraformFmt,
        FormatterKind::Shfmt,
        FormatterKind::Nixfmt,
        FormatterKind::Ocamlformat,
        FormatterKind::Rubocop,
        FormatterKind::Standardrb,
        FormatterKind::Erb,
        FormatterKind::Gleam,
        FormatterKind::Cljfmt,
        FormatterKind::Ormolu,
        FormatterKind::Latexindent,
        FormatterKind::Air,
        FormatterKind::Rustfmt,
        FormatterKind::Prettier,
        FormatterKind::Ruff,
    ]
}

fn extension_list_matches_path(extensions: &[String], path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };
    let file_ext = normalize_extension(ext);
    extensions
        .iter()
        .any(|candidate| candidate == "*" || candidate.eq_ignore_ascii_case(&file_ext))
}

fn default_extensions_for_kind(kind: FormatterKind) -> Vec<String> {
    match kind {
        FormatterKind::Gofmt => vec![".go".to_owned()],
        FormatterKind::Biome => vec![
            ".js".to_owned(),
            ".jsx".to_owned(),
            ".mjs".to_owned(),
            ".cjs".to_owned(),
            ".ts".to_owned(),
            ".tsx".to_owned(),
            ".mts".to_owned(),
            ".cts".to_owned(),
            ".html".to_owned(),
            ".htm".to_owned(),
            ".css".to_owned(),
            ".scss".to_owned(),
            ".sass".to_owned(),
            ".less".to_owned(),
            ".vue".to_owned(),
            ".svelte".to_owned(),
            ".json".to_owned(),
            ".jsonc".to_owned(),
            ".yaml".to_owned(),
            ".yml".to_owned(),
            ".toml".to_owned(),
            ".xml".to_owned(),
            ".md".to_owned(),
            ".mdx".to_owned(),
            ".graphql".to_owned(),
            ".gql".to_owned(),
        ],
        FormatterKind::Mix => vec![
            ".ex".to_owned(),
            ".exs".to_owned(),
            ".eex".to_owned(),
            ".heex".to_owned(),
            ".leex".to_owned(),
            ".neex".to_owned(),
            ".sface".to_owned(),
        ],
        FormatterKind::ClangFormat => vec![
            ".c".to_owned(),
            ".cc".to_owned(),
            ".cpp".to_owned(),
            ".cxx".to_owned(),
            ".h".to_owned(),
            ".hh".to_owned(),
            ".hpp".to_owned(),
            ".hxx".to_owned(),
            ".ino".to_owned(),
        ],
        FormatterKind::ZigFmt => vec![".zig".to_owned(), ".zon".to_owned()],
        FormatterKind::Ktlint => vec![".kt".to_owned(), ".kts".to_owned()],
        FormatterKind::DartFormat => vec![".dart".to_owned()],
        FormatterKind::TerraformFmt => vec![".tf".to_owned(), ".tfvars".to_owned()],
        FormatterKind::Shfmt => vec![".sh".to_owned(), ".bash".to_owned()],
        FormatterKind::Nixfmt => vec![".nix".to_owned()],
        FormatterKind::Ocamlformat => vec![".ml".to_owned(), ".mli".to_owned()],
        FormatterKind::Rubocop | FormatterKind::Standardrb => {
            vec![".rb".to_owned(), ".ru".to_owned()]
        }
        FormatterKind::Erb => vec![".erb".to_owned()],
        FormatterKind::Gleam => vec![".gleam".to_owned()],
        FormatterKind::Cljfmt => vec![
            ".clj".to_owned(),
            ".cljs".to_owned(),
            ".cljc".to_owned(),
            ".edn".to_owned(),
        ],
        FormatterKind::Ormolu => vec![".hs".to_owned()],
        FormatterKind::Latexindent => vec![".tex".to_owned()],
        FormatterKind::Air => vec![".r".to_owned()],
        FormatterKind::Rustfmt => vec![".rs".to_owned()],
        FormatterKind::Prettier => vec![
            ".js".to_owned(),
            ".jsx".to_owned(),
            ".mjs".to_owned(),
            ".cjs".to_owned(),
            ".ts".to_owned(),
            ".tsx".to_owned(),
            ".mts".to_owned(),
            ".cts".to_owned(),
            ".html".to_owned(),
            ".htm".to_owned(),
            ".css".to_owned(),
            ".scss".to_owned(),
            ".sass".to_owned(),
            ".less".to_owned(),
            ".vue".to_owned(),
            ".svelte".to_owned(),
            ".json".to_owned(),
            ".jsonc".to_owned(),
            ".yaml".to_owned(),
            ".yml".to_owned(),
            ".toml".to_owned(),
            ".xml".to_owned(),
            ".md".to_owned(),
            ".mdx".to_owned(),
            ".graphql".to_owned(),
            ".gql".to_owned(),
        ],
        FormatterKind::Ruff => vec![".py".to_owned(), ".pyi".to_owned()],
        FormatterKind::Custom => vec!["*".to_owned()],
    }
}

fn kinds_for_path(path: &Path) -> Vec<FormatterKind> {
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .map(|name| name.to_ascii_lowercase());
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.to_ascii_lowercase());
    let mut kinds = Vec::new();

    match extension.as_deref() {
        Some("go") => kinds.push(FormatterKind::Gofmt),
        Some("ex" | "exs" | "eex" | "heex" | "leex" | "neex" | "sface") => {
            kinds.push(FormatterKind::Mix);
        }
        Some(
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "html" | "htm" | "css"
            | "scss" | "sass" | "less" | "vue" | "svelte" | "json" | "jsonc" | "yaml" | "yml"
            | "toml" | "xml" | "md" | "mdx" | "graphql" | "gql",
        ) => {
            kinds.push(FormatterKind::Biome);
            kinds.push(FormatterKind::Prettier);
        }
        Some("zig" | "zon") => kinds.push(FormatterKind::ZigFmt),
        Some("c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx" | "ino") => {
            kinds.push(FormatterKind::ClangFormat);
        }
        Some("kt" | "kts") => kinds.push(FormatterKind::Ktlint),
        Some("py" | "pyi") => kinds.push(FormatterKind::Ruff),
        Some("r") => kinds.push(FormatterKind::Air),
        Some("rb" | "ru") => {
            kinds.push(FormatterKind::Rubocop);
            kinds.push(FormatterKind::Standardrb);
        }
        Some("erb") => kinds.push(FormatterKind::Erb),
        Some("dart") => kinds.push(FormatterKind::DartFormat),
        Some("ml" | "mli") => kinds.push(FormatterKind::Ocamlformat),
        Some("tf" | "tfvars") => kinds.push(FormatterKind::TerraformFmt),
        Some("tex") => kinds.push(FormatterKind::Latexindent),
        Some("gleam") => kinds.push(FormatterKind::Gleam),
        Some("sh" | "bash") => kinds.push(FormatterKind::Shfmt),
        Some("nix") => kinds.push(FormatterKind::Nixfmt),
        Some("rs") => kinds.push(FormatterKind::Rustfmt),
        Some("hs") => kinds.push(FormatterKind::Ormolu),
        Some("clj" | "cljs" | "cljc" | "edn") => kinds.push(FormatterKind::Cljfmt),
        _ => {}
    }

    if matches!(file_name.as_deref(), Some("rakefile")) {
        kinds.push(FormatterKind::Rubocop);
        kinds.push(FormatterKind::Standardrb);
    }
    if matches!(file_name.as_deref(), Some(name) if name.ends_with(".rake") || name.ends_with(".gemspec"))
    {
        kinds.push(FormatterKind::Rubocop);
        kinds.push(FormatterKind::Standardrb);
    }
    if matches!(file_name.as_deref(), Some(name) if name.ends_with(".html.erb")) {
        kinds.push(FormatterKind::Erb);
    }

    let mut deduped = Vec::new();
    for kind in kinds {
        if !deduped.contains(&kind) {
            deduped.push(kind);
        }
    }
    deduped
}

fn marker_presence_for_kind(kind: FormatterKind, root: &Path) -> Option<bool> {
    match kind {
        FormatterKind::Biome => Some(has_biome_marker(root)),
        FormatterKind::ClangFormat => Some(has_clang_format_marker(root)),
        FormatterKind::Ocamlformat => Some(has_ocamlformat_marker(root)),
        FormatterKind::Prettier => Some(has_prettier_marker(root)),
        FormatterKind::Ruff => Some(has_ruff_marker(root)),
        _ => None,
    }
}

fn command_exists_for_root(program: &str, root: &Path) -> bool {
    resolve_program_path(program, Some(root)).is_some()
}

fn resolve_program_path(program: &str, root: Option<&Path>) -> Option<PathBuf> {
    if program.trim().is_empty() {
        return None;
    }

    if program.contains(std::path::MAIN_SEPARATOR)
        || (cfg!(windows) && program.contains('/'))
        || (cfg!(windows) && program.contains('\\'))
    {
        let candidate = PathBuf::from(program);
        return is_executable_file(&candidate).then_some(candidate);
    }

    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if let Some(found) = resolve_program_in_dir(program, &dir) {
                return Some(found);
            }
        }
    }

    for dir in formatter_toolchain_bin_dirs(root) {
        if let Some(found) = resolve_program_in_dir(program, &dir) {
            return Some(found);
        }
    }
    None
}

fn resolve_program_in_dir(program: &str, dir: &Path) -> Option<PathBuf> {
    let candidate = dir.join(program);
    if is_executable_file(&candidate) {
        return Some(candidate);
    }
    if cfg!(windows) {
        for extension in ["exe", "cmd", "bat"] {
            let with_extension = candidate.with_extension(extension);
            if is_executable_file(&with_extension) {
                return Some(with_extension);
            }
        }
    }
    None
}

fn formatter_toolchain_bin_dirs(root: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(root) = root {
        push_toolchain_bin_dirs(root, &mut dirs);
    }
    if let Some(repo_root) = repo_root_from_current_exe() {
        push_toolchain_bin_dirs(&repo_root, &mut dirs);
    }
    dirs
}

fn push_toolchain_bin_dirs(root: &Path, dirs: &mut Vec<PathBuf>) {
    for candidate in [
        root.join(".cortex").join("formatter-toolchain").join("bin"),
        root.join(".cortex").join("toolchain").join("bin"),
        root.join(".cortex").join("toolchains").join("bin"),
    ] {
        if !dirs.contains(&candidate) {
            dirs.push(candidate);
        }
    }
}

fn repo_root_from_current_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|dir| dir.join("Cargo.toml").is_file() && dir.join("crates").is_dir())
        .map(Path::to_path_buf)
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if let Ok(metadata) = fs::metadata(path) {
            return metadata.permissions().mode() & 0o111 != 0;
        }
        false
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn current_dir_for_kind(
    path: &Path,
    root: &Path,
    kind: FormatterKind,
) -> Result<PathBuf, SkipReason> {
    match kind {
        FormatterKind::Gofmt
        | FormatterKind::Mix
        | FormatterKind::ZigFmt
        | FormatterKind::Ktlint
        | FormatterKind::DartFormat
        | FormatterKind::TerraformFmt
        | FormatterKind::Shfmt
        | FormatterKind::Nixfmt
        | FormatterKind::Rubocop
        | FormatterKind::Standardrb
        | FormatterKind::Erb
        | FormatterKind::Gleam
        | FormatterKind::Cljfmt
        | FormatterKind::Ormolu
        | FormatterKind::Latexindent
        | FormatterKind::Air
        | FormatterKind::Rustfmt => Ok(root.to_path_buf()),
        FormatterKind::Biome => {
            marker_dir(path, root, has_biome_marker).ok_or(SkipReason::MissingConfigMarker {
                formatter: FormatterKind::Biome,
            })
        }
        FormatterKind::ClangFormat => {
            marker_dir(path, root, has_clang_format_marker).ok_or(SkipReason::MissingConfigMarker {
                formatter: FormatterKind::ClangFormat,
            })
        }
        FormatterKind::Ocamlformat => {
            marker_dir(path, root, has_ocamlformat_marker).ok_or(SkipReason::MissingConfigMarker {
                formatter: FormatterKind::Ocamlformat,
            })
        }
        FormatterKind::Prettier => {
            marker_dir(path, root, has_prettier_marker).ok_or(SkipReason::MissingConfigMarker {
                formatter: FormatterKind::Prettier,
            })
        }
        FormatterKind::Ruff => {
            marker_dir(path, root, has_ruff_marker).ok_or(SkipReason::MissingConfigMarker {
                formatter: FormatterKind::Ruff,
            })
        }
        FormatterKind::Custom => {
            unreachable!("custom formatters return before built-in selection")
        }
    }
}

fn marker_dir(path: &Path, root: &Path, has_marker: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut current = path.parent();
    while let Some(dir) = current {
        if has_marker(dir) {
            return Some(dir.to_path_buf());
        }
        if same_path(dir, root) {
            break;
        }
        current = dir.parent();
    }
    None
}

fn has_prettier_marker(dir: &Path) -> bool {
    const FILE_MARKERS: &[&str] = &[
        ".prettierrc",
        ".prettierrc.json",
        ".prettierrc.yml",
        ".prettierrc.yaml",
        ".prettierrc.toml",
        ".prettierrc.js",
        ".prettierrc.cjs",
        ".prettierrc.mjs",
        "prettier.config.js",
        "prettier.config.cjs",
        "prettier.config.mjs",
    ];
    FILE_MARKERS.iter().any(|name| dir.join(name).is_file()) || package_json_has_prettier(dir)
}

fn has_biome_marker(dir: &Path) -> bool {
    dir.join("biome.json").is_file() || dir.join("biome.jsonc").is_file()
}

fn has_clang_format_marker(dir: &Path) -> bool {
    dir.join(".clang-format").is_file()
}

fn has_ocamlformat_marker(dir: &Path) -> bool {
    dir.join(".ocamlformat").is_file()
}

fn package_json_has_prettier(dir: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(dir.join("package.json")) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&contents)
        .ok()
        .and_then(|value| {
            value
                .as_object()
                .map(|object| object.contains_key("prettier"))
        })
        .unwrap_or(false)
}

fn has_ruff_marker(dir: &Path) -> bool {
    dir.join("ruff.toml").is_file() || dir.join(".ruff.toml").is_file() || pyproject_has_ruff(dir)
}

fn pyproject_has_ruff(dir: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(dir.join("pyproject.toml")) else {
        return false;
    };
    contents
        .parse::<toml::Value>()
        .ok()
        .and_then(|value| value.get("tool").cloned())
        .and_then(|tool| tool.get("ruff").cloned())
        .is_some()
}

fn absolutize_existing(path: PathBuf) -> PathBuf {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    };
    absolute.canonicalize().unwrap_or(absolute)
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right || left.canonicalize().ok().as_deref() == right.canonicalize().ok().as_deref()
}

fn configure_formatter_child(command: &mut Command) {
    command.env_clear().envs(formatter_child_env());
}

fn formatter_child_env() -> Vec<(OsString, OsString)> {
    filter_secret_env(std::env::vars_os())
}

fn filter_secret_env<I>(vars: I) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    vars.into_iter()
        .filter(|(name, value)| {
            if redaction::is_secret_env_var(&name.to_string_lossy()) {
                return false;
            }
            !redaction::value_is_secret(&value.to_string_lossy())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-format-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn toml_string(path: &Path) -> String {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[test]
    fn unsupported_extensions_are_left_unselected() {
        let dir = temp_dir("unknown");
        let runner = FormatterRunner::new(&dir);

        assert_eq!(
            runner.select(&dir.join("note.unknownext")),
            Err(SkipReason::UnsupportedExtension)
        );
    }

    #[test]
    fn prettier_requires_a_marker() {
        let dir = temp_dir("prettier-marker");
        let runner = FormatterRunner::new(&dir);

        let selected = runner.select(&dir.join("demo.ts"));
        if binary_available("biome") {
            assert_eq!(
                selected,
                Err(SkipReason::MissingConfigMarker {
                    formatter: FormatterKind::Biome
                })
            );
        } else {
            assert_eq!(selected, Err(SkipReason::MissingBinary { binary: "biome" }));
        }

        fs::write(dir.join(".prettierrc"), "{}").expect("write prettier marker");
        if binary_available("prettier") {
            assert_eq!(
                runner.select(&dir.join("demo.ts")).expect("selection").kind,
                FormatterKind::Prettier
            );
        }
    }

    #[test]
    fn ruff_requires_a_project_marker() {
        let dir = temp_dir("ruff-marker");
        let runner = FormatterRunner::new(&dir);

        let selected = runner.select(&dir.join("demo.py"));
        if binary_available("ruff") {
            assert_eq!(
                selected,
                Err(SkipReason::MissingConfigMarker {
                    formatter: FormatterKind::Ruff
                })
            );
        } else {
            assert_eq!(selected, Err(SkipReason::MissingBinary { binary: "ruff" }));
        }

        fs::write(dir.join("pyproject.toml"), "[tool.ruff]\n").expect("write ruff marker");
        if binary_available("ruff") {
            assert_eq!(
                runner.select(&dir.join("demo.py")).expect("selection").kind,
                FormatterKind::Ruff
            );
        }
    }

    #[test]
    fn biome_requires_a_project_marker() {
        let dir = temp_dir("biome-marker");
        let runner = FormatterRunner::new(&dir);

        let selected = runner.select(&dir.join("demo.ts"));
        if binary_available("biome") {
            assert_eq!(
                selected,
                Err(SkipReason::MissingConfigMarker {
                    formatter: FormatterKind::Biome
                })
            );
        } else {
            assert!(
                matches!(
                    selected,
                    Err(SkipReason::MissingBinary { binary: "biome" })
                        | Err(SkipReason::MissingConfigMarker {
                            formatter: FormatterKind::Prettier
                        })
                ),
                "expected a clean skip when biome is unavailable, got {selected:?}"
            );
        }

        fs::write(dir.join("biome.json"), "{}").expect("write biome marker");
        if binary_available("biome") {
            assert_eq!(
                runner.select(&dir.join("demo.ts")).expect("selection").kind,
                FormatterKind::Biome
            );
        }
    }

    #[test]
    fn clang_format_requires_a_project_marker() {
        let dir = temp_dir("clang-format-marker");
        let runner = FormatterRunner::new(&dir);

        let selected = runner.select(&dir.join("demo.c"));
        if binary_available("clang-format") {
            assert_eq!(
                selected,
                Err(SkipReason::MissingConfigMarker {
                    formatter: FormatterKind::ClangFormat
                })
            );
        } else {
            assert_eq!(
                selected,
                Err(SkipReason::MissingBinary {
                    binary: "clang-format"
                })
            );
        }

        fs::write(dir.join(".clang-format"), "BasedOnStyle: LLVM\n")
            .expect("write clang-format marker");
        if binary_available("clang-format") {
            assert_eq!(
                runner.select(&dir.join("demo.c")).expect("selection").kind,
                FormatterKind::ClangFormat
            );
        }
    }

    #[test]
    fn ocamlformat_requires_a_project_marker() {
        let dir = temp_dir("ocamlformat-marker");
        let runner = FormatterRunner::new(&dir);

        let selected = runner.select(&dir.join("demo.ml"));
        if binary_available("ocamlformat") {
            assert_eq!(
                selected,
                Err(SkipReason::MissingConfigMarker {
                    formatter: FormatterKind::Ocamlformat
                })
            );
        } else {
            assert_eq!(
                selected,
                Err(SkipReason::MissingBinary {
                    binary: "ocamlformat"
                })
            );
        }

        fs::write(dir.join(".ocamlformat"), "version = 0.26.1\n")
            .expect("write ocamlformat marker");
        if binary_available("ocamlformat") {
            assert_eq!(
                runner.select(&dir.join("demo.ml")).expect("selection").kind,
                FormatterKind::Ocamlformat
            );
        }
    }

    #[test]
    fn builtins_can_be_disabled_per_formatter() {
        let dir = temp_dir("disabled-rustfmt");
        let table: toml::value::Table = toml::from_str(
            r#"
[formatter.rustfmt]
disabled = true
"#,
        )
        .expect("toml");
        let config = FormatterConfig::from_table(&table).expect("config");
        let runner = FormatterRunner::with_config(&dir, config);
        assert_eq!(
            runner.select(&dir.join("demo.rs")),
            Err(SkipReason::UnsupportedExtension),
            "disabled built-in formatter should not be selected"
        );
    }

    #[cfg(unix)]
    #[test]
    fn builtin_command_override_runs_with_configured_env() {
        let dir = temp_dir("override-env");
        let marker = dir.join("formatter-env.txt");
        let script = dir.join("fake-rustfmt.sh");
        fs::write(
            &script,
            "#!/bin/sh\nprintf '%s' \"${CORTEX_FORMAT_TEST:-missing}\" > \"${CORTEX_FORMAT_MARKER:-/tmp/missing}\"\nexit 0\n",
        )
        .expect("script");
        make_executable(&script);

        let table: toml::value::Table = toml::from_str(&format!(
            r#"
[formatter.rustfmt]
command = ["{}", "$FILE"]
[formatter.rustfmt.env]
CORTEX_FORMAT_TEST = "env-ok"
CORTEX_FORMAT_MARKER = "{}"
"#,
            toml_string(&script),
            toml_string(&marker)
        ))
        .expect("toml");
        let config = FormatterConfig::from_table(&table).expect("config");
        let runner = FormatterRunner::with_config(&dir, config);
        let file = dir.join("demo.rs");
        fs::write(&file, "fn main() {}\n").expect("fixture file");

        let outcome = runner.format_path(&file);
        assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Rustfmt));
        assert_eq!(
            fs::read_to_string(&marker).expect("marker"),
            "env-ok",
            "override command should receive configured environment"
        );
    }
}
