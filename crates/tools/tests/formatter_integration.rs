//! Real formatter hook coverage for every mutating edit path.
//!
//! The tool layer owns the post-mutation hook seam. These tests attach the real formatter runner to
//! that seam and prove `write`, `edit`, and GPT-family `apply_patch` all leave persisted files equal
//! to canonical formatter output. They skip only when the external formatter binary needed for the
//! proof is absent.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use permissions::{Action, PermissionEngine, Rule, Ruleset};
use tools::{Diagnostic, MutationHooks, ToolContext, ToolRegistry};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct FormatterHooks {
    runner: format::FormatterRunner,
}

impl FormatterHooks {
    fn new(root: &Path) -> Self {
        Self {
            runner: format::FormatterRunner::new(root),
        }
    }

    fn with_config(root: &Path, config: format::FormatterConfig) -> Self {
        Self {
            runner: format::FormatterRunner::with_config(root, config),
        }
    }
}

impl MutationHooks for FormatterHooks {
    fn format(&self, path: &Path) {
        let _ = self.runner.format_path(path);
    }
}

struct UppercaseHooks {
    calls: AtomicUsize,
}

impl UppercaseHooks {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
        })
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl MutationHooks for UppercaseHooks {
    fn format(&self, path: &Path) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let content = fs::read_to_string(path).expect("read file for uppercase formatter");
        fs::write(path, content.to_uppercase()).expect("write uppercase formatter output");
    }
}

struct FailingNoteHooks;

impl MutationHooks for FailingNoteHooks {
    fn format_feedback(&self, _path: &Path) -> Option<String> {
        Some("formatter rustfmt failed with exit 1: parse error".to_owned())
    }
}

struct LineShiftAndDiagnoseHooks;

impl MutationHooks for LineShiftAndDiagnoseHooks {
    fn format(&self, path: &Path) {
        let content = fs::read_to_string(path).expect("read before line-shift formatter");
        fs::write(path, content.replace("BAD_TOKEN", "\nBAD_TOKEN"))
            .expect("write line-shift formatter output");
    }

    fn diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        let content = fs::read_to_string(path).unwrap_or_default();
        content
            .lines()
            .position(|line| line.contains("BAD_TOKEN"))
            .map(|index| {
                vec![Diagnostic::new(
                    "error",
                    index + 1,
                    "post-format diagnostic",
                )]
            })
            .unwrap_or_default()
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-tools-format-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn context_with_formatters(dir: &Path) -> ToolContext {
    ToolContext::new(dir.to_path_buf()).with_hooks(Arc::new(FormatterHooks::new(dir)))
}

fn context_with_formatters_config(dir: &Path, config: format::FormatterConfig) -> ToolContext {
    ToolContext::new(dir.to_path_buf())
        .with_hooks(Arc::new(FormatterHooks::with_config(dir, config)))
}

fn context_with_fake_formatter(
    dir: &Path,
    hooks: Arc<dyn MutationHooks>,
    format_action: Action,
) -> ToolContext {
    let engine = PermissionEngine::new(Ruleset::new(vec![
        Rule::new("write", "**", Action::Allow),
        Rule::new("edit", "**", Action::Allow),
        Rule::new("apply_patch", "**", Action::Allow),
        Rule::new("format", "**", format_action),
    ]));
    ToolContext::new(dir.to_path_buf())
        .with_permissions(Arc::new(engine))
        .with_hooks(hooks)
}

fn rustfmt_expected(raw: &str) -> String {
    let dir = temp_dir("expected");
    let path = dir.join("expected.rs");
    fs::write(&path, raw).expect("write rustfmt input");
    let output = Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rustfmt");
    assert!(
        output.status.success(),
        "rustfmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read_to_string(path).expect("read rustfmt output")
}

fn formatter_config_from_toml(toml: &str) -> format::FormatterConfig {
    let value = toml::from_str::<toml::Value>(toml).expect("valid formatter config");
    format::FormatterConfig::from_table(value.as_table().expect("formatter table"))
        .expect("parse formatter config")
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).expect("write executable script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable script");
}

#[tokio::test]
async fn write_runs_real_rustfmt_after_persisting_bytes() {
    if !format::binary_available("rustfmt") {
        eprintln!("skipping formatter hook test: rustfmt is not on PATH");
        return;
    }

    let dir = temp_dir("write");
    let ctx = context_with_formatters(&dir);
    let raw = "fn  main( ){let x=1;println!(\"{}\",x);}\n";
    let args = serde_json::json!({ "path": "main.rs", "content": raw }).to_string();

    let output = ToolRegistry::builtin().execute("write", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("main.rs")).expect("read formatted rust"),
        rustfmt_expected(raw)
    );
}

#[tokio::test]
async fn edit_runs_real_rustfmt_after_the_replacement() {
    if !format::binary_available("rustfmt") {
        eprintln!("skipping formatter hook test: rustfmt is not on PATH");
        return;
    }

    let dir = temp_dir("edit");
    let path = dir.join("lib.rs");
    fs::write(&path, "fn demo() {}\n").expect("write original rust");
    let ctx = context_with_formatters(&dir);
    ctx.record_read(&path);
    let replacement = "fn  demo( ){let x=1;println!(\"{}\",x);}\n";
    let args = serde_json::json!({
        "path": "lib.rs",
        "oldString": "fn demo() {}\n",
        "newString": replacement
    })
    .to_string();

    let output = ToolRegistry::builtin().execute("edit", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(&path).expect("read formatted rust"),
        rustfmt_expected(replacement)
    );
}

#[tokio::test]
async fn apply_patch_formats_written_rust_and_leaves_unknown_extensions_verbatim() {
    if !format::binary_available("rustfmt") {
        eprintln!("skipping formatter hook test: rustfmt is not on PATH");
        return;
    }

    let dir = temp_dir("apply-patch");
    let ctx = context_with_formatters(&dir);
    let raw_rust = "fn  patched( ){let x=1;println!(\"{}\",x);}";
    let unknown = "verbatim   spacing\n";
    let patch = format!(
        "*** Begin Patch\n\
*** Add File: patched.rs\n\
+{raw_rust}\n\
*** Add File: note.unknownext\n\
+{unknown}\
*** End Patch"
    );
    let args = serde_json::json!({ "patch": patch }).to_string();

    let output = ToolRegistry::gpt()
        .execute("apply_patch", &args, &ctx)
        .await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("patched.rs")).expect("read formatted rust"),
        rustfmt_expected(&(raw_rust.to_owned() + "\n"))
    );
    assert_eq!(
        fs::read_to_string(dir.join("note.unknownext")).expect("read unknown"),
        unknown
    );
}

#[cfg(unix)]
#[tokio::test]
async fn write_runs_gofmt_formatter_for_go_files() {
    let dir = temp_dir("write-gofmt");
    let script = dir.join("fake-gofmt.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'package main\\n\\nfunc main() {}\\n' > \"$file\"\n",
    );
    let config = formatter_config_from_toml(&format!(
        "[formatter.gofmt]\ncommand = [\"{}\", \"$FILE\"]\n",
        script.display()
    ));
    let ctx = context_with_formatters_config(&dir, config);
    let args = serde_json::json!({
        "path": "main.go",
        "content": "package main\nfunc main(){println(\"hi\")}\n"
    })
    .to_string();

    let output = ToolRegistry::builtin().execute("write", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("main.go")).expect("read formatted go"),
        "package main\n\nfunc main() {}\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn edit_runs_gofmt_formatter_for_go_files() {
    let dir = temp_dir("edit-gofmt");
    let script = dir.join("fake-gofmt.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'package main\\n\\nfunc main() {}\\n' > \"$file\"\n",
    );
    let config = formatter_config_from_toml(&format!(
        "[formatter.gofmt]\ncommand = [\"{}\", \"$FILE\"]\n",
        script.display()
    ));
    let ctx = context_with_formatters_config(&dir, config);
    let path = dir.join("main.go");
    fs::write(&path, "package main\nfunc main() {}\n").expect("seed go file");
    ctx.record_read(&path);
    let args = serde_json::json!({
        "path": "main.go",
        "oldString": "func main() {}\n",
        "newString": "func main(){println(\"hi\")}\n"
    })
    .to_string();

    let output = ToolRegistry::builtin().execute("edit", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(&path).expect("read formatted go"),
        "package main\n\nfunc main() {}\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn apply_patch_runs_gofmt_formatter_for_go_files() {
    let dir = temp_dir("apply-gofmt");
    let script = dir.join("fake-gofmt.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'package main\\n\\nfunc main() {}\\n' > \"$file\"\n",
    );
    let config = formatter_config_from_toml(&format!(
        "[formatter.gofmt]\ncommand = [\"{}\", \"$FILE\"]\n",
        script.display()
    ));
    let ctx = context_with_formatters_config(&dir, config);
    let stars = "*".repeat(3);
    let patch = format!(
        "{stars} Begin Patch\n\
{stars} Add File: main.go\n\
+package main\n\
+func main(){{println(\"hi\")}}\n\
{stars} End Patch"
    );
    let args = serde_json::json!({ "patch": patch }).to_string();

    let output = ToolRegistry::gpt()
        .execute("apply_patch", &args, &ctx)
        .await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("main.go")).expect("read formatted go"),
        "package main\n\nfunc main() {}\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn biome_formats_ts_files_when_marker_exists() {
    let dir = temp_dir("write-biome");
    let script = dir.join("fake-biome.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'const value = { answer: 42 };\\n' > \"$file\"\n",
    );
    fs::write(dir.join("biome.json"), "{}\n").expect("write biome marker");
    let config = formatter_config_from_toml(&format!(
        "[formatter.biome]\ncommand = [\"{}\", \"$FILE\"]\n",
        script.display()
    ));
    let ctx = context_with_formatters_config(&dir, config);
    let args = serde_json::json!({
        "path": "demo.ts",
        "content": "const value={answer:42};\n"
    })
    .to_string();

    let output = ToolRegistry::builtin().execute("write", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("demo.ts")).expect("read formatted ts"),
        "const value = { answer: 42 };\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn biome_skips_ts_files_without_marker_and_keeps_raw_bytes() {
    let dir = temp_dir("write-biome-no-marker");
    let script = dir.join("fake-biome.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'const value = { answer: 42 };\\n' > \"$file\"\n",
    );
    let config = formatter_config_from_toml(&format!(
        "[formatter.biome]\ncommand = [\"{}\", \"$FILE\"]\n",
        script.display()
    ));
    let ctx = context_with_formatters_config(&dir, config);
    let raw = "const value={answer:42};\n";
    let args = serde_json::json!({
        "path": "demo.ts",
        "content": raw
    })
    .to_string();

    let output = ToolRegistry::builtin().execute("write", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("demo.ts")).expect("read unformatted ts"),
        raw
    );
}

#[tokio::test]
async fn formatter_permission_deny_skips_formatter_but_keeps_write_successful() {
    let dir = temp_dir("format-deny");
    let hooks = UppercaseHooks::new();
    let ctx = context_with_fake_formatter(&dir, hooks.clone(), Action::Deny);
    let raw = "lowercase content\n";
    let args = serde_json::json!({ "path": "note.txt", "content": raw }).to_string();

    let output = ToolRegistry::builtin().execute("write", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("note.txt")).expect("read raw write"),
        raw,
        "denying format keeps the raw written bytes"
    );
    assert_eq!(hooks.call_count(), 0, "formatter must not run when denied");
}

#[tokio::test]
async fn formatter_permission_allow_runs_formatter_after_write() {
    let dir = temp_dir("format-allow");
    let hooks = UppercaseHooks::new();
    let ctx = context_with_fake_formatter(&dir, hooks.clone(), Action::Allow);
    let args =
        serde_json::json!({ "path": "note.txt", "content": "lowercase content\n" }).to_string();

    let output = ToolRegistry::builtin().execute("write", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("note.txt")).expect("read formatted write"),
        "LOWERCASE CONTENT\n"
    );
    assert_eq!(hooks.call_count(), 1, "formatter ran exactly once");
}

#[tokio::test]
async fn formatter_failure_note_is_surfaced_without_failing_the_write() {
    let dir = temp_dir("format-failure-note");
    let ctx = context_with_fake_formatter(&dir, Arc::new(FailingNoteHooks), Action::Allow);
    let raw = "fn  main( {\n";
    let args = serde_json::json!({ "path": "broken.rs", "content": raw }).to_string();

    let output = ToolRegistry::builtin().execute("write", &args, &ctx).await;

    assert!(
        !output.is_error,
        "formatter failures must not turn the write into a failed mutation: {}",
        output.model_content()
    );
    assert_eq!(
        fs::read_to_string(dir.join("broken.rs")).expect("read failed format write"),
        raw,
        "the raw write remains intact after formatter failure"
    );
    assert!(
        output.model_content().contains("formatter rustfmt failed"),
        "formatter failure should be visible to the model: {}",
        output.model_content()
    );
}

#[tokio::test]
async fn diagnostics_are_collected_after_formatter_rewrites_the_file() {
    let dir = temp_dir("format-before-diagnostics");
    let ctx = context_with_fake_formatter(&dir, Arc::new(LineShiftAndDiagnoseHooks), Action::Allow);
    let raw = "first line\nBAD_TOKEN\n";
    let args = serde_json::json!({ "path": "diag.rs", "content": raw }).to_string();

    let output = ToolRegistry::builtin().execute("write", &args, &ctx).await;

    assert!(!output.is_error, "{}", output.model_content());
    assert_eq!(
        fs::read_to_string(dir.join("diag.rs")).expect("read post-format file"),
        "first line\n\nBAD_TOKEN\n",
        "formatter shifted the diagnostic token down one line"
    );
    let text = output.model_content();
    assert!(
        text.contains("line 3"),
        "diagnostics should point at the post-format line, got: {text}"
    );
    assert!(
        !text.contains("line 2"),
        "diagnostics must not use stale pre-format positions, got: {text}"
    );

    let clean_args =
        serde_json::json!({ "path": "clean.rs", "content": "fn main() {}\n" }).to_string();
    let clean_output = ToolRegistry::builtin()
        .execute("write", &clean_args, &ctx)
        .await;
    assert!(!clean_output.is_error, "{}", clean_output.model_content());
    assert!(
        !clean_output.model_content().contains("Diagnostics:"),
        "formatting a clean file must not create phantom diagnostics: {}",
        clean_output.model_content()
    );
}
