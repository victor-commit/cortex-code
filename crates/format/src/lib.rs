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

/// A formatter family Cortex knows how to run after a file mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatterKind {
    /// `rustfmt --edition 2024`.
    Rustfmt,
    /// `prettier --write`.
    Prettier,
    /// `ruff format`.
    Ruff,
}

impl FormatterKind {
    /// The executable name expected on `PATH`.
    #[must_use]
    pub fn binary(self) -> &'static str {
        match self {
            Self::Rustfmt => "rustfmt",
            Self::Prettier => "prettier",
            Self::Ruff => "ruff",
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
}

/// Real formatter runner rooted at one project/workspace directory.
#[derive(Debug, Clone)]
pub struct FormatterRunner {
    root: PathBuf,
}

impl FormatterRunner {
    /// Create a formatter runner. Relative roots are converted to an absolute path when possible.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = absolutize_existing(root);
        Self { root }
    }

    /// Select the formatter that would be run for `path`, including binary/config gating.
    pub fn select(&self, path: &Path) -> Result<FormatterSelection, SkipReason> {
        let Some(kind) = kind_for_path(path) else {
            return Err(SkipReason::UnsupportedExtension);
        };
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
        };
        Ok(FormatterSelection { kind, current_dir })
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
    let mut command = Command::new(selection.kind.binary());
    command
        .current_dir(&selection.current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    match selection.kind {
        FormatterKind::Rustfmt => {
            command.arg("--edition").arg("2024").arg(path);
        }
        FormatterKind::Prettier => {
            command.arg("--write").arg(path);
        }
        FormatterKind::Ruff => {
            command.arg("format").arg(path);
        }
    }

    match command.output() {
        Ok(output) if output.status.success() => FormatOutcome::Formatted(selection.kind),
        Ok(output) => FormatOutcome::Failed(FormatFailure {
            formatter: selection.kind,
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }),
        Err(err) => FormatOutcome::Failed(FormatFailure {
            formatter: selection.kind,
            status: None,
            stderr: err.to_string(),
        }),
    }
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
