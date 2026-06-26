//! TUI bridge from the tool-layer mutation hook seam to the real post-mutation formatter.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tools::{DiagnosticFeedback, MutationHooks};

/// TUI post-mutation hooks: run the real formatter first, then delegate diagnostics to the LSP hook.
pub(crate) struct PostMutationHooks {
    formatter: format::FormatterRunner,
    diagnostics: Arc<dyn MutationHooks>,
}

impl PostMutationHooks {
    /// Build hooks rooted at the current TUI working directory with config-driven formatter
    /// overrides.
    #[must_use]
    pub(crate) fn with_config(
        root: impl Into<PathBuf>,
        config: format::FormatterConfig,
        diagnostics: Arc<dyn MutationHooks>,
    ) -> Self {
        Self {
            formatter: format::FormatterRunner::with_config(root, config),
            diagnostics,
        }
    }
}

impl MutationHooks for PostMutationHooks {
    fn format(&self, path: &Path) {
        let _ = self.formatter.format_path(path);
    }

    fn format_feedback(&self, path: &Path) -> Option<String> {
        match self.formatter.format_path(path) {
            format::FormatOutcome::Failed(failure) => Some(format::format_failure_note(&failure)),
            format::FormatOutcome::Formatted(_) | format::FormatOutcome::Skipped(_) => None,
        }
    }

    fn diagnostic_feedback(&self, path: &Path) -> DiagnosticFeedback {
        self.diagnostics.diagnostic_feedback(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct ContentDiagnostics;

    impl MutationHooks for ContentDiagnostics {
        fn diagnostic_feedback(&self, path: &Path) -> DiagnosticFeedback {
            DiagnosticFeedback {
                diagnostics: Vec::new(),
                note: Some(fs::read_to_string(path).expect("diagnostics read formatted file")),
            }
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-tui-format-bridge-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[cfg(unix)]
    #[test]
    fn tui_bridge_formats_before_delegating_diagnostics() {
        let dir = temp_dir("order");
        let path = dir.join("demo.foo");
        fs::write(&path, "raw\n").expect("write raw input");
        let mut custom = toml::value::Table::new();
        custom.insert(
            "command".to_owned(),
            toml::Value::Array(vec![
                toml::Value::String("/bin/sh".to_owned()),
                toml::Value::String("-ceu".to_owned()),
                toml::Value::String(
                    "file=\"$1\"; printf 'formatted for diagnostics\\n' > \"$file\"".to_owned(),
                ),
                toml::Value::String("format-bridge".to_owned()),
                toml::Value::String("$FILE".to_owned()),
            ]),
        );
        custom.insert(
            "extensions".to_owned(),
            toml::Value::Array(vec![toml::Value::String(".foo".to_owned())]),
        );
        let mut formatter = toml::value::Table::new();
        formatter.insert("custom".to_owned(), toml::Value::Table(custom));
        let mut root = toml::value::Table::new();
        root.insert("formatter".to_owned(), toml::Value::Table(formatter));
        let config = format::FormatterConfig::from_table(&root).expect("parse formatter config");
        let hooks = PostMutationHooks::with_config(&dir, config, Arc::new(ContentDiagnostics));

        assert!(
            hooks.format_feedback(&path).is_none(),
            "successful formatter should not add a failure note"
        );
        let feedback = hooks.diagnostic_feedback(&path);
        assert_eq!(
            feedback.note.as_deref(),
            Some("formatted for diagnostics\n"),
            "TUI-compatible hooks must delegate diagnostics after formatter output is on disk"
        );
    }
}
