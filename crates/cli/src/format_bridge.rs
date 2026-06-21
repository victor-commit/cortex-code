use std::path::{Path, PathBuf};
use std::sync::Arc;

use tools::{DiagnosticFeedback, MutationHooks};

/// Headless post-mutation hooks: run the real formatter first, then delegate diagnostics to the
/// LSP hook. The mutating tools own the call order (`format` before `diagnostic_feedback`), while
/// this bridge composes the two concrete M4 implementations into one `MutationHooks` object.
pub(crate) struct PostMutationHooks {
    formatter: format::FormatterRunner,
    diagnostics: Arc<dyn MutationHooks>,
}

impl PostMutationHooks {
    /// Build hooks rooted at the current run working directory.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(root: impl Into<PathBuf>, diagnostics: Arc<dyn MutationHooks>) -> Self {
        Self::with_config(root, format::FormatterConfig::default(), diagnostics)
    }

    /// Build hooks rooted at the current run working directory with config-driven formatter
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
            format::FormatOutcome::Failed(failure) => Some(format_failure_note(&failure)),
            format::FormatOutcome::Formatted(_) | format::FormatOutcome::Skipped(_) => None,
        }
    }

    fn diagnostic_feedback(&self, path: &Path) -> DiagnosticFeedback {
        self.diagnostics.diagnostic_feedback(path)
    }
}

fn format_failure_note(failure: &format::FormatFailure) -> String {
    let status = failure
        .status
        .map(|status| format!("exit {status}"))
        .unwrap_or_else(|| "spawn failure".to_owned());
    let stderr = if failure.stderr.is_empty() {
        "no stderr".to_owned()
    } else {
        failure.stderr.clone()
    };
    format!(
        "Formatter {} failed ({status}); left the written bytes intact. stderr: {stderr}",
        failure.formatter.label()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct NoteDiagnostics;

    impl MutationHooks for NoteDiagnostics {
        fn diagnostic_feedback(&self, _path: &Path) -> DiagnosticFeedback {
            DiagnosticFeedback {
                diagnostics: Vec::new(),
                note: Some("diagnostics delegated".to_owned()),
            }
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-cli-format-bridge-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn formats_rust_and_delegates_diagnostics() {
        if !format::binary_available("rustfmt") {
            eprintln!("skipping bridge test: rustfmt is not on PATH");
            return;
        }

        let dir = temp_dir("rust");
        let path = dir.join("main.rs");
        fs::write(&path, "fn  main( ){let x=1;println!(\"{}\",x);}\n").expect("write rust");
        let hooks = PostMutationHooks::new(&dir, Arc::new(NoteDiagnostics));

        hooks.format(&path);
        let formatted = fs::read_to_string(&path).expect("read rust");
        assert!(formatted.contains("fn main()"));
        assert!(formatted.contains("let x = 1;"));

        let feedback = hooks.diagnostic_feedback(&path);
        assert_eq!(feedback.note.as_deref(), Some("diagnostics delegated"));
    }
}
