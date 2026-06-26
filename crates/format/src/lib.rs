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

    fn custom_for_path(&self, path: &Path) -> Option<CustomFormatter> {
        self.custom
            .iter()
            .find(|formatter| formatter.matches_path(path))
            .cloned()
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
        if let Some(custom) = self.config.custom_for_path(path) {
            return Ok(FormatterSelection {
                kind: FormatterKind::Custom,
                current_dir: self.root.clone(),
                command: FormatterCommand::Custom(custom),
            });
        }

        let kinds = kinds_for_path(path);
        if kinds.is_empty() {
            return Err(SkipReason::UnsupportedExtension);
        }

        let mut last_error = SkipReason::UnsupportedExtension;
        for kind in kinds {
            if !self.config.builtin_enabled(kind) {
                continue;
            }
            let builtin_override = self.config.builtin_override(kind);
            if builtin_override.command.is_none() && !binary_available(kind.binary()) {
                last_error = SkipReason::MissingBinary {
                    binary: kind.binary(),
                };
                continue;
            }
            match current_dir_for_kind(path, &self.root, kind) {
                Ok(current_dir) => {
                    return Ok(FormatterSelection {
                        kind,
                        current_dir,
                        command: FormatterCommand::Builtin {
                            kind,
                            command: builtin_override.command,
                            env: builtin_override.env,
                        },
                    });
                }
                Err(reason) => {
                    last_error = reason;
                }
            }
        }

        Err(last_error)
    }

    /// Run the applicable formatter in-place over `path`.
    ///
    /// `Skipped` outcomes never modify the file. `Formatted` means the real command exited 0.
    /// `Failed` reports the process failure; the caller decides how to surface it.
    #[must_use]
    pub fn format_path(&self, path: &Path) -> FormatOutcome {
        let selection = match self.select(path) {
            Ok(selection) => selection,
            Err(reason) => return FormatOutcome::Skipped(reason),
        };
        run_formatter(path, &selection)
    }
}

/// Return whether `binary` is runnable from the current `PATH`.
#[must_use]
pub fn binary_available(binary: &str) -> bool {
    let mut command = Command::new(binary);
    configure_formatter_child(&mut command);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.status().is_ok_and(|status| status.success())
}

fn run_formatter(path: &Path, selection: &FormatterSelection) -> FormatOutcome {
    let pre_format = fs::read(path).ok();
    let outcome = match &selection.command {
        FormatterCommand::Builtin { kind, command, env } => {
            run_builtin_formatter(path, *kind, command.as_deref(), env, &selection.current_dir)
        }
        FormatterCommand::Custom(custom) => {
            run_custom_formatter(path, custom, &selection.current_dir)
        }
    };

    if matches!(outcome, FormatOutcome::Failed(_)) {
        if let Some(bytes) = pre_format {
            let _ = fs::write(path, bytes);
        }
    }

    outcome
}

fn run_builtin_formatter(
    path: &Path,
    kind: FormatterKind,
    command_override: Option<&[String]>,
    env: &BTreeMap<String, String>,
    current_dir: &Path,
) -> FormatOutcome {
    if let Some(command_override) = command_override {
        return run_overridden_command(path, command_override, env, current_dir, kind);
    }

    let mut command = Command::new(kind.binary());
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
    for (name, kind) in [
        ("gofmt", FormatterKind::Gofmt),
        ("biome", FormatterKind::Biome),
        ("mix", FormatterKind::Mix),
        ("clang_format", FormatterKind::ClangFormat),
        ("zig_fmt", FormatterKind::ZigFmt),
        ("ktlint", FormatterKind::Ktlint),
        ("dart", FormatterKind::DartFormat),
        ("terraform", FormatterKind::TerraformFmt),
        ("shfmt", FormatterKind::Shfmt),
        ("nixfmt", FormatterKind::Nixfmt),
        ("ocamlformat", FormatterKind::Ocamlformat),
        ("rubocop", FormatterKind::Rubocop),
        ("standardrb", FormatterKind::Standardrb),
        ("erb", FormatterKind::Erb),
        ("gleam", FormatterKind::Gleam),
        ("cljfmt", FormatterKind::Cljfmt),
        ("ormolu", FormatterKind::Ormolu),
        ("latexindent", FormatterKind::Latexindent),
        ("air", FormatterKind::Air),
        ("rustfmt", FormatterKind::Rustfmt),
        ("prettier", FormatterKind::Prettier),
        ("ruff", FormatterKind::Ruff),
    ] {
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
        if disabled || command.is_some() || !env.is_empty() {
            overrides.insert(
                kind,
                FormatterOverride {
                    disabled,
                    command,
                    env,
                },
            );
        }
    }
    Ok(overrides)
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
        .and_then(parse_string_or_array)?;
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
        if binary_available("prettier") {
            assert_eq!(
                selected,
                Err(SkipReason::MissingConfigMarker {
                    formatter: FormatterKind::Prettier
                })
            );
        } else {
            assert_eq!(
                selected,
                Err(SkipReason::MissingBinary { binary: "prettier" })
            );
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
