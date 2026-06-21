use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use lsp::{FeedbackManager, LspConfigSet, NavigationOutcome, NavigationRequest, ServerRunState};

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
        "pub fn add_one(input: i32) -> i32 {\n    input + 1\n}\n\npub fn call_add() -> i32 {\n    add_one(41)\n}\n\npub fn call_add_again() -> i32 {\n    add_one(1)\n}\n",
    )
    .expect("write lib.rs");
    file
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

    let hover = manager.navigate(NavigationRequest::Hover {
        file_path: file.clone(),
        line: 6,
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
        file_path: file.clone(),
        line: 6,
        character: 7,
    });
    let NavigationOutcome::Locations(definition_locations) = definition else {
        panic!("expected definition locations, got {definition:?}");
    };
    assert!(
        definition_locations.iter().any(|location| {
            location.uri.ends_with("/src/lib.rs") && location.range.start.line == 0
        }),
        "definition should point at add_one declaration, got {definition_locations:?}"
    );

    let references = manager.navigate(NavigationRequest::References {
        file_path: file.clone(),
        line: 6,
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
    for expected in [1, 6, 10] {
        assert!(
            reference_lines.contains(&expected),
            "reference lines should include {expected}, got {reference_lines:?}"
        );
    }

    let document_symbols = manager.navigate(NavigationRequest::DocumentSymbols {
        file_path: file.clone(),
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
        file_path: Some(file),
    });
    let NavigationOutcome::WorkspaceSymbols(symbols) = workspace_symbols else {
        panic!("expected workspace symbols, got {workspace_symbols:?}");
    };
    assert!(
        symbols.iter().any(|symbol| symbol.name == "add_one"),
        "workspace symbols should include add_one, got {symbols:?}"
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

    let pid = manager
        .statuses()
        .into_iter()
        .find_map(|status| match status.state {
            ServerRunState::Running(pid) if status.language_id == "rust" => Some(pid),
            _ => None,
        })
        .expect("rust-analyzer pid recorded");
    let status = Command::new("kill")
        .arg(pid.to_string())
        .status()
        .expect("kill lsp pid");
    assert!(status.success(), "kill should signal the test-owned PID");

    std::thread::sleep(Duration::from_millis(150));
    let after_crash = manager.navigate(NavigationRequest::Hover {
        file_path: file,
        line: 6,
        character: 7,
    });
    assert_eq!(after_crash, NavigationOutcome::Unavailable);

    manager.shutdown_all();
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
