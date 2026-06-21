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
    fn write_executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, body).expect("write script");
        let mut permissions = fs::metadata(path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod script");
    }

    #[cfg(unix)]
    #[test]
    fn tui_bridge_formats_before_delegating_diagnostics() {
        let dir = temp_dir("order");
        let script = dir.join("formatter.sh");
        write_executable(
            &script,
            "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'formatted for diagnostics\\n' > \"$file\"\n",
        );
        let path = dir.join("demo.foo");
        fs::write(&path, "raw\n").expect("write raw input");
        let config = format::FormatterConfig::from_table(
            toml::from_str::<toml::Value>(&format!(
                "[formatter.custom]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n",
                script.display()
            ))
            .expect("parse toml")
            .as_table()
            .expect("formatter table"),
        )
        .expect("parse formatter config");
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
