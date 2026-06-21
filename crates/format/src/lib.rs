//! Real post-mutation formatter selection and execution.
//!
//! The runner is intentionally small and side-effecting: once a mutating tool has written a file,
//! the surface asks this crate whether a real formatter applies to that path. Matching requires the
//! language extension, the formatter binary on `PATH`, and, for project-scoped formatters, a config
//! marker in the file's ancestor tree. Formatting is in-place and uses the real external binaries.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use toml::Value;

/// A formatter family Cortex knows how to run after a file mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatterKind {
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
    Builtin(FormatterKind),
    Custom(CustomFormatter),
}

/// A configured formatter command that overrides built-in detection for matching extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFormatter {
    command: Vec<String>,
    extensions: Vec<String>,
}

impl CustomFormatter {
    /// Build a custom formatter from a command vector and extension list.
    ///
    /// The command may contain `$FILE`; if it does not, the target file path is appended as the last
    /// argument when the formatter runs. Extensions may be written with or without a leading dot.
    ///
    /// # Errors
    /// Returns a human-readable config error when the command or extension list is empty.
    pub fn new(command: Vec<String>, extensions: Vec<String>) -> Result<Self, String> {
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
    disabled: Vec<FormatterKind>,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            custom: Vec::new(),
            builtins_enabled: true,
            disabled: Vec::new(),
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
    /// `[formatter.prettier] disabled = true` (and likewise `rustfmt` / `ruff`).
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
                    disabled: parse_disabled_builtins(table),
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
        self.builtins_enabled && !self.disabled.contains(&kind)
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

        let Some(kind) = kind_for_path(path) else {
            return Err(SkipReason::UnsupportedExtension);
        };
        if !self.config.builtin_enabled(kind) {
            return Err(SkipReason::UnsupportedExtension);
        }
        if !binary_available(kind.binary()) {
            return Err(SkipReason::MissingBinary {
                binary: kind.binary(),
            });
        }
        let current_dir = match kind {
            FormatterKind::Rustfmt => self.root.clone(),
            FormatterKind::Prettier => marker_dir(path, &self.root, has_prettier_marker).ok_or(
                SkipReason::MissingConfigMarker {
                    formatter: FormatterKind::Prettier,
                },
            )?,
            FormatterKind::Ruff => marker_dir(path, &self.root, has_ruff_marker).ok_or(
                SkipReason::MissingConfigMarker {
                    formatter: FormatterKind::Ruff,
                },
            )?,
            FormatterKind::Custom => {
                unreachable!("custom formatters return before built-in selection")
            }
        };
        Ok(FormatterSelection {
            kind,
            current_dir,
            command: FormatterCommand::Builtin(kind),
        })
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
    Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run_formatter(path: &Path, selection: &FormatterSelection) -> FormatOutcome {
    let pre_format = fs::read(path).ok();
    let outcome = match &selection.command {
        FormatterCommand::Builtin(kind) => {
            run_builtin_formatter(path, *kind, &selection.current_dir)
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

fn run_builtin_formatter(path: &Path, kind: FormatterKind, current_dir: &Path) -> FormatOutcome {
    let mut command = Command::new(kind.binary());
    command
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    match kind {
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
    command
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

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

fn parse_disabled_builtins(table: &toml::value::Table) -> Vec<FormatterKind> {
    [
        ("rustfmt", FormatterKind::Rustfmt),
        ("prettier", FormatterKind::Prettier),
        ("ruff", FormatterKind::Ruff),
    ]
    .into_iter()
    .filter_map(|(key, kind)| {
        table
            .get(key)
            .and_then(Value::as_table)
            .and_then(|table| table.get("disabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then_some(kind)
    })
    .collect()
}

fn parse_custom_formatters(value: &Value) -> Result<Vec<CustomFormatter>, String> {
    match value {
        Value::Table(table)
            if table.contains_key("command") || table.contains_key("extensions") =>
        {
            Ok(vec![parse_custom_formatter(table)?])
        }
        Value::Table(table) => table
            .iter()
            .map(|(name, value)| {
                value
                    .as_table()
                    .ok_or_else(|| format!("formatter.custom.{name} must be a table"))
                    .and_then(parse_custom_formatter)
            })
            .collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value
                    .as_table()
                    .ok_or_else(|| format!("formatter.custom[{index}] must be a table"))
                    .and_then(parse_custom_formatter)
            })
            .collect(),
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
    CustomFormatter::new(command, extensions)
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

fn kind_for_path(path: &Path) -> Option<FormatterKind> {
    let ext = path.extension().and_then(OsStr::to_str)?;
    if ext.eq_ignore_ascii_case("rs") {
        return Some(FormatterKind::Rustfmt);
    }
    if matches!(
        ext.to_ascii_lowercase().as_str(),
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "json" | "css"
    ) {
        return Some(FormatterKind::Prettier);
    }
    if ext.eq_ignore_ascii_case("py") {
        return Some(FormatterKind::Ruff);
    }
    None
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
}
