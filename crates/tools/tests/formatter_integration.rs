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

use tools::{MutationHooks, ToolContext, ToolRegistry};

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
}

impl MutationHooks for FormatterHooks {
    fn format(&self, path: &Path) {
        let _ = self.runner.format_path(path);
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
