use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use lsp::{
    FeedbackManager, LspConfigSet, LspServerConfig, NavigationOutcome, NavigationRequest,
    ServerRunState,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-lsp-nav-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create temp src dir");
    dir
}

fn has_command(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn write_rust_project(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    let file = dir.join("src/lib.rs");
    std::fs::write(
        &file,
        "pub trait Greeter {\n    fn greet(&self) -> &'static str;\n}\n\npub struct Robot;\n\nimpl Greeter for Robot {\n    fn greet(&self) -> &'static str {\n        \"beep\"\n    }\n}\n\npub fn add_one(input: i32) -> i32 {\n    input + 1\n}\n\npub fn call_add() -> i32 {\n    add_one(41)\n}\n\npub fn call_add_again() -> i32 {\n    add_one(1)\n}\n\npub fn invoke_greet() -> &'static str {\n    let robot = Robot;\n    robot.greet()\n}\n",
    )
    .expect("write lib.rs");
    file
}

fn write_retry_proxy(dir: &Path) -> PathBuf {
    let script = dir.join("rust_analyzer_retry_proxy.py");
    std::fs::write(&script, RUST_ANALYZER_RETRY_PROXY).expect("write retry proxy");
    script
}

fn retry_proxy_config(script: &Path, log: &Path) -> LspConfigSet {
    LspConfigSet::empty().with_server(
        LspServerConfig::new("rust", "python3")
            .with_arg(script.display().to_string())
            .with_arg("rust-analyzer")
            .with_arg(log.display().to_string())
            .with_extension("rs")
            .with_root_marker("Cargo.toml")
            .with_request_timeout(Duration::from_secs(10)),
    )
}

fn assert_rust_analyzer_results(manager: &mut FeedbackManager, file: &Path) {
    let hover = manager.navigate(NavigationRequest::Hover {
        file_path: file.to_path_buf(),
        line: 18,
        character: 7,
    });
    let NavigationOutcome::Hover(Some(hover)) = hover else {
        panic!("expected hover content, got {hover:?}");
    };
    assert!(
        hover.contents.contains("add_one") && hover.contents.contains("i32"),
        "hover should name the real function signature/type, got:\n{}",
        hover.contents
    );

    let definition = manager.navigate(NavigationRequest::Definition {
        file_path: file.to_path_buf(),
        line: 18,
        character: 7,
    });
    let NavigationOutcome::Locations(definition_locations) = definition else {
        panic!("expected definition locations, got {definition:?}");
    };
    assert!(
        definition_locations.iter().any(|location| {
            location.uri.ends_with("/src/lib.rs") && location.range.start.line == 12
        }),
        "definition should point at add_one declaration, got {definition_locations:?}"
    );

    let references = manager.navigate(NavigationRequest::References {
        file_path: file.to_path_buf(),
        line: 18,
        character: 7,
        include_declaration: true,
    });
    let NavigationOutcome::Locations(reference_locations) = references else {
        panic!("expected reference locations, got {references:?}");
    };
    let reference_lines: Vec<u32> = reference_locations
        .iter()
        .filter(|location| location.uri.ends_with("/src/lib.rs"))
        .map(|location| location.range.start.line + 1)
        .collect();
    for expected in [13, 18, 22] {
        assert!(
            reference_lines.contains(&expected),
            "reference lines should include {expected}, got {reference_lines:?}"
        );
    }

    let implementation = manager.navigate(NavigationRequest::Implementation {
        file_path: file.to_path_buf(),
        line: 2,
        character: 8,
    });
    let NavigationOutcome::Locations(implementation_locations) = implementation else {
        panic!("expected implementation locations, got {implementation:?}");
    };
    assert!(
        implementation_locations.iter().any(|location| {
            location.uri.ends_with("/src/lib.rs") && location.range.start.line == 7
        }),
        "implementation should point at Greeter for Robot::greet, got {implementation_locations:?}"
    );

    let prepared = manager.navigate(NavigationRequest::PrepareCallHierarchy {
        file_path: file.to_path_buf(),
        line: 13,
        character: 8,
    });
    let NavigationOutcome::CallHierarchyItems(prepared_items) = prepared else {
        panic!("expected call hierarchy items, got {prepared:?}");
    };
    assert!(
        prepared_items.iter().any(|item| item.name == "add_one"),
        "prepareCallHierarchy should include add_one, got {prepared_items:?}"
    );

    let incoming = manager.navigate(NavigationRequest::IncomingCalls {
        file_path: file.to_path_buf(),
        line: 13,
        character: 8,
    });
    let NavigationOutcome::IncomingCalls(incoming_calls) = incoming else {
        panic!("expected incoming calls, got {incoming:?}");
    };
    let incoming_names: Vec<&str> = incoming_calls
        .iter()
        .map(|call| call.from.name.as_str())
        .collect();
    for expected in ["call_add", "call_add_again"] {
        assert!(
            incoming_names.contains(&expected),
            "incoming call names should include {expected}, got {incoming_names:?}"
        );
    }

    let outgoing = manager.navigate(NavigationRequest::OutgoingCalls {
        file_path: file.to_path_buf(),
        line: 17,
        character: 8,
    });
    let NavigationOutcome::OutgoingCalls(outgoing_calls) = outgoing else {
        panic!("expected outgoing calls, got {outgoing:?}");
    };
    assert!(
        outgoing_calls.iter().any(|call| call.to.name == "add_one"),
        "outgoing calls should include add_one, got {outgoing_calls:?}"
    );

    let document_symbols = manager.navigate(NavigationRequest::DocumentSymbols {
        file_path: file.to_path_buf(),
    });
    let NavigationOutcome::DocumentSymbols(symbols) = document_symbols else {
        panic!("expected document symbols, got {document_symbols:?}");
    };
    assert!(
        symbols.iter().any(|symbol| symbol.name == "add_one"
            && symbol.kind.is_some_and(|kind| kind.label() == "function")),
        "document symbols should list add_one as a function, got {symbols:?}"
    );

    let workspace_symbols = manager.navigate(NavigationRequest::WorkspaceSymbols {
        query: "add_one".to_owned(),
        file_path: Some(file.to_path_buf()),
    });
    let NavigationOutcome::WorkspaceSymbols(symbols) = workspace_symbols else {
        panic!("expected workspace symbols, got {workspace_symbols:?}");
    };
    assert!(
        symbols.iter().any(|symbol| symbol.name == "add_one"),
        "workspace symbols should include add_one, got {symbols:?}"
    );
}

fn rust_server_pid(manager: &mut FeedbackManager) -> u32 {
    manager
        .statuses()
        .into_iter()
        .find_map(|status| match status.state {
            ServerRunState::Running(pid) if status.language_id == "rust" => Some(pid),
            _ => None,
        })
        .expect("rust language server pid recorded")
}

fn assert_pid_gone(pid: u32) {
    assert!(
        Command::new("ps")
            .args(["-p", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| !status.success()),
        "test-owned LSP PID {pid} should be gone after shutdown"
    );
}

#[test]
fn rust_analyzer_returns_hover_definition_references_and_symbols() {
    if !has_command("rust-analyzer") {
        eprintln!("skipping real navigation test: rust-analyzer is not on PATH");
        return;
    }

    let dir = temp_dir("rust");
    let file = write_rust_project(&dir);
    let mut manager =
        FeedbackManager::new(&dir, LspConfigSet::defaults()).with_budget(Duration::from_secs(20));

    assert_rust_analyzer_results(&mut manager, &file);

    let workspace_symbols = manager.navigate(NavigationRequest::WorkspaceSymbols {
        query: "add_one".to_owned(),
        file_path: None,
    });
    let NavigationOutcome::WorkspaceSymbols(symbols) = workspace_symbols else {
        panic!("expected workspace symbols without a file hint, got {workspace_symbols:?}");
    };
    assert!(
        symbols.iter().any(|symbol| symbol.name == "add_one"),
        "workspace symbols without a file hint should include add_one, got {symbols:?}"
    );
}

#[test]
fn rust_analyzer_retries_readiness_errors_for_all_navigation_operations() {
    if !has_command("rust-analyzer") {
        eprintln!("skipping real retry navigation test: rust-analyzer is not on PATH");
        return;
    }
    if !has_command("python3") {
        eprintln!("skipping real retry navigation test: python3 is not on PATH");
        return;
    }

    let dir = temp_dir("retry");
    let file = write_rust_project(&dir);
    let script = write_retry_proxy(&dir);
    let log = dir.join("retry-log.json");
    let mut manager = FeedbackManager::new(&dir, retry_proxy_config(&script, &log))
        .with_budget(Duration::from_secs(20));

    assert_rust_analyzer_results(&mut manager, &file);

    let workspace_symbols_without_hint = manager.navigate(NavigationRequest::WorkspaceSymbols {
        query: "add_one".to_owned(),
        file_path: None,
    });
    assert!(
        matches!(
            workspace_symbols_without_hint,
            NavigationOutcome::WorkspaceSymbols(_)
        ),
        "workspace symbols without a file hint should use the already-running real rust-analyzer server, got {workspace_symbols_without_hint:?}"
    );

    let pid = rust_server_pid(&mut manager);
    manager.shutdown_all();
    assert_pid_gone(pid);

    let log_text = std::fs::read_to_string(&log).expect("retry proxy wrote its log");
    let log_json: serde_json::Value =
        serde_json::from_str(&log_text).expect("retry proxy log is json");
    for method in [
        "textDocument/hover",
        "textDocument/definition",
        "textDocument/references",
        "textDocument/implementation",
        "textDocument/prepareCallHierarchy",
        "callHierarchy/incomingCalls",
        "callHierarchy/outgoingCalls",
        "textDocument/documentSymbol",
        "workspace/symbol",
    ] {
        assert_eq!(
            log_json
                .get("injected")
                .and_then(|injected| injected.get(method))
                .and_then(serde_json::Value::as_u64),
            Some(1),
            "proxy should inject exactly one retryable readiness error for {method}; log was {log_text}"
        );
    }
    assert_eq!(
        log_json
            .get("server_exit")
            .and_then(serde_json::Value::as_i64),
        Some(0),
        "real rust-analyzer should shut down cleanly behind the retry proxy; log was {log_text}"
    );
}

#[test]
fn missing_language_server_is_a_clean_no_server_navigation_result() {
    let dir = temp_dir("missing");
    let file = dir.join("notes.md");
    std::fs::write(&file, "# Notes\n").expect("write markdown");

    let mut manager =
        FeedbackManager::new(&dir, LspConfigSet::defaults()).with_budget(Duration::from_secs(2));
    assert_eq!(
        manager.navigate(NavigationRequest::DocumentSymbols { file_path: file }),
        NavigationOutcome::NoServer
    );
}

#[test]
fn killed_language_server_is_reported_unavailable_and_reaped() {
    if !has_command("rust-analyzer") {
        eprintln!("skipping crash resilience test: rust-analyzer is not on PATH");
        return;
    }

    let dir = temp_dir("crash");
    let file = write_rust_project(&dir);
    let mut manager =
        FeedbackManager::new(&dir, LspConfigSet::defaults()).with_budget(Duration::from_secs(20));

    let started = manager.navigate(NavigationRequest::DocumentSymbols {
        file_path: file.clone(),
    });
    assert!(
        matches!(started, NavigationOutcome::DocumentSymbols(_)),
        "server should start and answer before crash, got {started:?}"
    );

    let pid = rust_server_pid(&mut manager);
    let status = Command::new("kill")
        .arg(pid.to_string())
        .status()
        .expect("kill lsp pid");
    assert!(status.success(), "kill should signal the test-owned PID");

    std::thread::sleep(Duration::from_millis(150));
    let after_crash = manager.navigate(NavigationRequest::Hover {
        file_path: file,
        line: 18,
        character: 7,
    });
    assert_eq!(after_crash, NavigationOutcome::Unavailable);

    manager.shutdown_all();
    assert_pid_gone(pid);
}

const RUST_ANALYZER_RETRY_PROXY: &str = r#"
import json
import subprocess
import sys
import threading

RETRY_METHODS = {
    "textDocument/hover": -32802,
    "textDocument/definition": -32802,
    "textDocument/references": -32802,
    "textDocument/documentSymbol": -32802,
    "textDocument/implementation": -32802,
    "textDocument/prepareCallHierarchy": -32802,
    "callHierarchy/incomingCalls": -32802,
    "callHierarchy/outgoingCalls": -32802,
    "workspace/symbol": -32802,
}

injected = {}


def read_message(stream):
    headers = {}
    while True:
        line = stream.readline()
        if line == b"":
            return None
        if line in (b"\r\n", b"\n"):
            break
        name, _, value = line.decode("ascii", "replace").partition(":")
        headers[name.lower()] = value.strip()
    length = int(headers.get("content-length", "0"))
    if length <= 0:
        return None
    body = stream.read(length)
    if len(body) != length:
        return None
    return json.loads(body.decode("utf-8"))


def write_message(stream, message):
    body = json.dumps(message, separators=(",", ":")).encode("utf-8")
    stream.write(b"Content-Length: " + str(len(body)).encode("ascii") + b"\r\n\r\n" + body)
    stream.flush()


server = subprocess.Popen(
    [sys.argv[1]],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)


def relay_server_to_client():
    while True:
        message = read_message(server.stdout)
        if message is None:
            break
        write_message(sys.stdout.buffer, message)


def drain_stderr():
    while server.stderr.read(8192):
        pass


relay = threading.Thread(target=relay_server_to_client, daemon=True)
relay.start()
threading.Thread(target=drain_stderr, daemon=True).start()
server_exit = None

try:
    while True:
        message = read_message(sys.stdin.buffer)
        if message is None:
            break
        method = message.get("method")
        if method in RETRY_METHODS and "id" in message and injected.get(method, 0) == 0:
            injected[method] = 1
            write_message(
                sys.stdout.buffer,
                {
                    "jsonrpc": "2.0",
                    "id": message["id"],
                    "error": {
                        "code": RETRY_METHODS[method],
                        "message": "test-injected rust-analyzer readiness cancellation",
                    },
                },
            )
            continue
        write_message(server.stdin, message)
        if method == "exit":
            break
finally:
    try:
        server.stdin.close()
    except Exception:
        pass
    try:
        server_exit = server.wait(timeout=5)
    except subprocess.TimeoutExpired:
        server.kill()
        server_exit = server.wait(timeout=5)
    relay.join(timeout=1)
    with open(sys.argv[2], "w", encoding="utf-8") as handle:
        json.dump({"injected": injected, "server_exit": server_exit}, handle)
"#;
