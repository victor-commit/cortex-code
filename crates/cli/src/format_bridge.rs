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

    fn synthetic_openrouter_key() -> String {
        let body = "deadbeef".repeat(8);
        format!("sk-{}-{}", "or-v1", body)
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, body).expect("write executable script");
        let mut permissions = fs::metadata(path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod executable script");
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

    #[cfg(unix)]
    #[test]
    fn format_feedback_redacts_secret_stderr_and_preserves_bytes() {
        let dir = temp_dir("failure-redaction");
        let secret = synthetic_openrouter_key();
        let script = dir.join("formatter.sh");
        write_executable(
            &script,
            &format!(
                "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'CORRUPTED\\n' > \"$file\"\nprintf 'formatter stderr {}\\n' >&2\nexit 7\n",
                secret
            ),
        );
        let path = dir.join("demo.foo");
        let raw = "raw bytes that must survive\n";
        fs::write(&path, raw).expect("write custom input");
        let config = format::FormatterConfig::from_table(
            toml::from_str::<toml::Value>(&format!(
                "[formatter.custom]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n",
                script.display()
            ))
            .unwrap()
            .as_table()
            .unwrap(),
        )
        .expect("parse formatter config");
        let hooks = PostMutationHooks::with_config(&dir, config, Arc::new(NoteDiagnostics));

        let note = hooks
            .format_feedback(&path)
            .expect("formatter failure produces model-visible feedback");

        assert!(
            note.contains(redaction::PLACEHOLDER),
            "formatter feedback must use canonical redaction: {note}"
        );
        assert!(
            !note.contains(&secret),
            "raw formatter stderr secret must not reach model-visible output: {note}"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read restored file"),
            raw,
            "formatter failure must preserve the written bytes"
        );
    }
}
