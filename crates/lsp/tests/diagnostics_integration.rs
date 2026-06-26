use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use lsp::{
    FeedbackManager, FeedbackOutcome, InstallPlan, LspConfigSet, LspServerConfig,
    WorkspaceDiagnosticsOutcome,
};

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

fn workspace_diagnostics_for(
    outcome: WorkspaceDiagnosticsOutcome,
) -> Vec<lsp::WorkspaceFileDiagnostics> {
    match outcome {
        WorkspaceDiagnosticsOutcome::Diagnostics(diagnostics) => diagnostics,
        other => panic!("expected workspace diagnostics outcome, got {other:?}"),
    }
}

fn fake_lsp_script() -> &'static str {
    r#"#!/usr/bin/env python3
import json
import os
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlparse, unquote

mode = sys.argv[1]
write_lock = threading.Lock()
documents = {}
root_uri = None

def read_message():
    headers = {}
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    while line not in (b"\r\n", b"\n", b""):
        name, value = line.decode("ascii").split(":", 1)
        headers[name.lower()] = value.strip()
        line = sys.stdin.buffer.readline()
    length = int(headers.get("content-length", "0"))
    if length <= 0:
        return None
    return json.loads(sys.stdin.buffer.read(length))

def send(message):
    body = json.dumps(message, separators=(",", ":")).encode("utf-8")
    with write_lock:
        sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii"))
        sys.stdout.buffer.write(body)
        sys.stdout.buffer.flush()

def diagnostic(message):
    return [{
        "range": {
            "start": {"line": 0, "character": 0},
            "end": {"line": 0, "character": 3},
        },
        "severity": 1,
        "code": "E_FAKE",
        "source": "fake-lsp",
        "message": message,
    }]

def publish(uri, diagnostics, version=None):
    params = {"uri": uri, "diagnostics": diagnostics}
    if mode == "versioned" and version is not None:
        params["version"] = version
    send({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": params,
    })

def delayed_publish(delay, uri, diagnostics, version=None):
    def worker():
        time.sleep(delay)
        publish(uri, diagnostics, version)
    threading.Thread(target=worker, daemon=True).start()

def uri_to_path(uri):
    parsed = urlparse(uri)
    if parsed.scheme != "file":
        return None
    return Path(unquote(parsed.path))

def path_to_uri(path):
    return path.resolve().as_uri()

def workspace_documents():
    if mode != "workspace_pull":
        return dict(documents)
    if not root_uri:
        return dict(documents)
    root = uri_to_path(root_uri)
    if root is None:
        return dict(documents)
    discovered = {}
    for current, _dirs, files in os.walk(root):
        for name in files:
            if not name.endswith(".fake"):
                continue
            path = Path(current) / name
            try:
                text = path.read_text()
            except Exception:
                continue
            discovered[path_to_uri(path)] = text
    return discovered

def workspace_items():
    items = []
    for uri, text in sorted(workspace_documents().items()):
        diagnostics = []
        if "BAD" in text:
            diagnostics = diagnostic(f"workspace BAD diagnostic from {uri.rsplit('/', 1)[-1]}")
        items.append({
            "uri": uri,
            "version": 1,
            "kind": "full",
            "items": diagnostics,
        })
    return items

while True:
    message = read_message()
    if message is None:
        break
    method = message.get("method")
    if method == "initialize" and "id" in message:
        root_uri = message.get("params", {}).get("rootUri")
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "capabilities": {
                    "textDocumentSync": 1,
                    "diagnosticProvider": {
                        "interFileDependencies": True,
                        "workspaceDiagnostics": True,
                    },
                },
                "serverInfo": {"name": "fake-lsp"},
            },
        })
    elif method == "initialized":
        pass
    elif method == "textDocument/diagnostic" and "id" in message:
        uri = message.get("params", {}).get("textDocument", {}).get("uri", "")
        text = documents.get(uri, "")
        diagnostics = diagnostic("pull BAD diagnostic") if "BAD" in text else []
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "kind": "full",
                "items": diagnostics,
            },
        })
    elif method == "workspace/diagnostic" and "id" in message:
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "items": workspace_items(),
            },
        })
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        text_document = message["params"].get("textDocument", {})
        uri = text_document["uri"]
        version = text_document.get("version", 1)
        if method == "textDocument/didOpen":
            text = text_document.get("text", "")
        else:
            changes = message["params"].get("contentChanges", [])
            text = changes[0].get("text", "") if changes else ""
        documents[uri] = text

        if mode in ("pull_only", "workspace_pull"):
            pass
        elif "BAD" in text and mode == "early_empty_then_delayed_bad":
            publish(uri, [], version)
            delayed_publish(1.5, uri, diagnostic("delayed BAD diagnostic"), version)
        elif "BAD" in text:
            publish(uri, diagnostic("fresh BAD diagnostic"), version)
            if mode == "unversioned":
                delayed_publish(1.4, uri, diagnostic("stale queued diagnostic"), version)
        else:
            if mode == "early_empty_then_delayed_bad":
                publish(uri, [], version)
            elif mode == "versioned":
                publish(uri, diagnostic("stale old-version diagnostic"), version - 1)
                delayed_publish(1.5, uri, [], version)
            else:
                delayed_publish(1.5, uri, [], version)
    elif "id" in message:
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
"#
}

fn fake_config(dir: &Path, mode: &str) -> LspConfigSet {
    let script = dir.join("fake_lsp.py");
    std::fs::write(&script, fake_lsp_script()).expect("write fake lsp script");
    LspConfigSet::empty().with_server(
        LspServerConfig::new("fake", "python3")
            .with_args([script.to_string_lossy().to_string(), mode.to_owned()])
            .with_extension("fake")
            .with_request_timeout(Duration::from_secs(5)),
    )
}

fn stale_publish_is_never_returned_for_clean_fix(mode: &str) {
    if !has_command("python3") {
        eprintln!("skipping fake LSP freshness test: python3 is not on PATH");
        return;
    }

    let dir = temp_dir(mode);
    let file = dir.join("main.fake");
    std::fs::write(&file, "BAD\n").expect("write bad fixture");
    let mut manager =
        FeedbackManager::new(&dir, fake_config(&dir, mode)).with_budget(Duration::from_secs(4));

    let bad = diagnostics_for(manager.collect(&file));
    assert!(
        bad.iter()
            .any(|diagnostic| diagnostic.message == "fresh BAD diagnostic"),
        "first mutation should report the fresh bad diagnostic, got {bad:?}"
    );

    // In unversioned mode the fake server deliberately publishes one stale diagnostic after the
    // first collection has returned. The next collection must drain/ignore it and wait for the
    // clean post-change publish instead.
    std::thread::sleep(Duration::from_millis(1600));
    std::fs::write(&file, "GOOD\n").expect("write clean fixture");
    let clean = diagnostics_for(manager.collect(&file));
    assert!(
        clean.is_empty(),
        "fresh clean publish must clear older diagnostics in {mode} mode, got {clean:?}"
    );
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
fn versioned_publish_diagnostics_waits_for_current_clean_publish() {
    stale_publish_is_never_returned_for_clean_fix("versioned");
}

#[test]
fn unversioned_publish_diagnostics_uses_post_change_generation_boundary() {
    stale_publish_is_never_returned_for_clean_fix("unversioned");
}

#[test]
fn warmed_open_document_waits_past_early_empty_publish_for_delayed_diagnostics() {
    if !has_command("python3") {
        eprintln!("skipping fake LSP warmed diagnostic test: python3 is not on PATH");
        return;
    }

    let dir = temp_dir("early-empty");
    let file = dir.join("main.fake");
    std::fs::write(&file, "GOOD\n").expect("write clean fixture");
    let mut manager = FeedbackManager::new(&dir, fake_config(&dir, "early_empty_then_delayed_bad"))
        .with_budget(Duration::from_secs(4));

    let warm = diagnostics_for(manager.collect(&file));
    assert!(
        warm.is_empty(),
        "initial clean publish should warm/open the document without diagnostics: {warm:?}"
    );

    std::fs::write(&file, "BAD\n").expect("write bad fixture");
    let diagnostics = diagnostics_for(manager.collect(&file));
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message == "delayed BAD diagnostic"
                || diagnostic.message == "pull BAD diagnostic"
        }),
        "didChange must not treat the early empty target-URI publish as final; got {diagnostics:?}"
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

#[test]
fn pull_diagnostics_request_reports_errors_without_publish_notifications() {
    if !has_command("python3") {
        eprintln!("skipping pull diagnostics test: python3 is not on PATH");
        return;
    }

    let dir = temp_dir("pull-only");
    let file = dir.join("main.fake");
    std::fs::write(&file, "BAD\n").expect("write bad fixture");
    let mut manager = FeedbackManager::new(&dir, fake_config(&dir, "pull_only"))
        .with_budget(Duration::from_secs(3));

    let diagnostics = diagnostics_for(manager.collect(&file));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "pull BAD diagnostic"),
        "expected pull diagnostics to surface without publish notifications, got {diagnostics:?}"
    );
}

#[test]
fn workspace_pull_diagnostics_aggregate_multiple_files_and_clear_incrementally() {
    if !has_command("python3") {
        eprintln!("skipping workspace diagnostics test: python3 is not on PATH");
        return;
    }

    let dir = temp_dir("workspace-pull");
    let alpha = dir.join("alpha.fake");
    let beta = dir.join("beta.fake");
    std::fs::write(&alpha, "BAD alpha\n").expect("write alpha");
    std::fs::write(&beta, "BAD beta\n").expect("write beta");

    let mut manager = FeedbackManager::new(&dir, fake_config(&dir, "workspace_pull"))
        .with_budget(Duration::from_secs(4));

    let first = workspace_diagnostics_for(manager.collect_workspace(&alpha));
    let first_rendered = format!("{first:?}");
    assert!(
        first_rendered.contains("alpha.fake") && first_rendered.contains("beta.fake"),
        "workspace diagnostics should include both files, got {first_rendered}"
    );
    assert!(
        first_rendered.contains("workspace BAD diagnostic"),
        "workspace diagnostics should include diagnostic messages, got {first_rendered}"
    );

    std::fs::write(&alpha, "GOOD alpha\n").expect("clear alpha");
    let second = workspace_diagnostics_for(manager.collect_workspace(&alpha));
    let second_rendered = format!("{second:?}");
    assert!(
        !second_rendered.contains("workspace BAD diagnostic from alpha.fake"),
        "clearing alpha should remove only alpha diagnostics, got {second_rendered}"
    );
    assert!(
        second_rendered.contains("workspace BAD diagnostic from beta.fake"),
        "clearing alpha must keep beta diagnostics, got {second_rendered}"
    );
}

fn language<'a>(set: &'a LspConfigSet, path: &Path) -> Option<&'a str> {
    set.resolve_for_path(path)
        .map(|server| server.language_id())
}
