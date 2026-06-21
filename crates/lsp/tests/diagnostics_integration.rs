use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use lsp::{FeedbackManager, FeedbackOutcome, InstallPlan, LspConfigSet, LspServerConfig};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("cortex-lsp-{tag}-{}-{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn has_command(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn diagnostics_for(outcome: FeedbackOutcome) -> Vec<lsp::FileDiagnostic> {
    match outcome {
        FeedbackOutcome::Diagnostics(diagnostics) => diagnostics,
        other => panic!("expected diagnostics outcome, got {other:?}"),
    }
}

#[test]
fn rust_analyzer_reports_real_rust_type_error() {
    if !has_command("rust-analyzer") {
        eprintln!("skipping real LSP test: rust-analyzer is not on PATH");
        return;
    }

    let dir = temp_dir("rust");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    let file = dir.join("src/main.rs");
    std::fs::write(
        &file,
        "fn main() {\n    let x: i32 = \"nope\";\n    println!(\"{}\", x);\n}\n",
    )
    .expect("write bad rust");

    let mut manager = FeedbackManager::new(&dir, LspConfigSet::defaults());
    let diagnostics = diagnostics_for(manager.collect(&file));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == "error"
                && diagnostic.code.as_deref() == Some("E0308")
                && diagnostic.message.contains("mismatched types")
                && diagnostic.line == 2
                && diagnostic.character <= 18
                && diagnostic.end_character >= 23),
        "expected real rust-analyzer E0308 diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn ruff_server_reports_real_error_warning_ranges_and_then_clears() {
    if !has_command("ruff") {
        eprintln!("skipping real LSP test: ruff is not on PATH");
        return;
    }

    let dir = temp_dir("ruff");
    let file = dir.join("bad.py");
    std::fs::write(&file, "import os\nprint(undefined_name)\n").expect("write bad python");

    let mut manager = FeedbackManager::new(&dir, LspConfigSet::defaults());
    let diagnostics = diagnostics_for(manager.collect(&file));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == "error"
                && diagnostic.code.as_deref() == Some("F821")
                && diagnostic.message.contains("Undefined name")
                && diagnostic.line == 2
                && diagnostic.character <= 7
                && diagnostic.end_character >= 20),
        "expected real Ruff F821 diagnostic with precise range, got {diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == "warning"
                && diagnostic.code.as_deref() == Some("F401")
                && diagnostic.message.contains("unused")
                && diagnostic.line == 1
                && diagnostic.character <= 8
                && diagnostic.end_character >= 9),
        "expected real Ruff F401 warning with precise range, got {diagnostics:?}"
    );

    std::fs::write(&file, "print('ok')\n").expect("write clean python");
    let clean = diagnostics_for(manager.collect(&file));
    assert!(
        clean.is_empty(),
        "clean file must not have diagnostics: {clean:?}"
    );
}

#[test]
fn default_mapping_routes_rust_python_and_typescript_by_extension() {
    let set = LspConfigSet::defaults();
    assert_eq!(language(&set, Path::new("src/main.rs")), Some("rust"));
    assert_eq!(language(&set, Path::new("script.py")), Some("python"));
    assert_eq!(language(&set, Path::new("web/app.ts")), Some("typescript"));
    assert_eq!(language(&set, Path::new("notes.txt")), None);
}

#[test]
fn missing_typescript_server_is_clean_no_server_when_no_net_is_set() {
    if std::env::var("CORTEX_NO_NET").as_deref() != Ok("1") {
        eprintln!("skipping no-net install guard test: CORTEX_NO_NET=1 is not set");
        return;
    }
    if has_command("typescript-language-server") {
        eprintln!(
            "skipping no-net install guard test: typescript-language-server is already installed"
        );
        return;
    }

    let dir = temp_dir("no-net-ts");
    let file = dir.join("bad.ts");
    std::fs::write(&file, "const x: number = 'nope';\n").expect("write ts fixture");

    let mut manager = FeedbackManager::new(&dir, LspConfigSet::defaults());
    assert_eq!(manager.collect(&file), FeedbackOutcome::NoServer);
    assert_eq!(
        std::fs::read_to_string(&file).expect("write/edit still persists without LSP"),
        "const x: number = 'nope';\n"
    );
}

#[test]
fn net_disabled_missing_installable_server_is_no_server_without_blocking_the_write() {
    if std::env::var("CORTEX_NO_NET").as_deref() != Ok("1") {
        eprintln!("skipping no-net install guard test: CORTEX_NO_NET=1 is not set");
        return;
    }

    let dir = temp_dir("no-net-custom");
    let file = dir.join("fixture.zzz");
    std::fs::write(&file, "broken but persisted\n").expect("write fixture");
    let missing_program = format!(
        "cortex-missing-lsp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let config = LspConfigSet::empty().with_server(
        LspServerConfig::new("zzz", missing_program)
            .with_extension("zzz")
            .with_install(InstallPlan::Npm {
                package: "missing-lsp-package".to_owned(),
            }),
    );

    let mut manager = FeedbackManager::new(&dir, config);
    assert_eq!(manager.collect(&file), FeedbackOutcome::NoServer);
    assert_eq!(
        std::fs::read_to_string(&file).expect("mutation persists without install"),
        "broken but persisted\n"
    );
}

fn language<'a>(set: &'a LspConfigSet, path: &Path) -> Option<&'a str> {
    set.resolve_for_path(path)
        .map(|server| server.language_id())
}
