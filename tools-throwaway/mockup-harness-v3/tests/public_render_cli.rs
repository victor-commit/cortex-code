use std::path::Path;
use std::process::{Command, Output};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn run(arguments: &[&str]) -> Output {
    harness()
        .args(arguments)
        .output()
        .expect("the public renderer CLI must run")
}

fn assert_registry_failure(arguments: &[&str], expected_identity: &str) {
    let output = run(arguments);
    assert_eq!(
        output.status.code(),
        Some(3),
        "{arguments:?} must fail during registry preflight\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "registry failures must not mix output into stdout"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected_identity),
        "registry failures must name the offending selector identity"
    );
}

#[test]
fn implemented_selector_succeeds_without_placeholder_artifacts() {
    let output = run(&["--implemented"]);

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(
        !Path::new("/projects/cortex-code/plans/assets/mockups-v3/input/permission-shell.txt")
            .exists(),
        "an unimplemented screen must never receive a placeholder artifact"
    );
}

#[test]
fn every_render_selector_preflights_missing_renderers_before_writing() {
    assert_registry_failure(&["--all"], "home");
    assert_registry_failure(&["permission-shell"], "permission-shell");
    assert_registry_failure(&["input"], "slash-menu");
    assert_registry_failure(&["--flow", "daily-work"], "home");

    for path in [
        "/projects/cortex-code/plans/assets/mockups-v3/session/home.txt",
        "/projects/cortex-code/plans/assets/mockups-v3/input/permission-shell.txt",
        "/projects/cortex-code/plans/assets/mockups-v3-gates/daily-work/01-home.txt",
    ] {
        assert!(
            !Path::new(path).exists(),
            "preflight must finish before any artifact is written: {path}"
        );
    }
}

#[test]
fn unknown_selectors_are_usage_errors_without_writes() {
    for arguments in [
        vec!["not-a-screen"],
        vec!["--flow", "not-a-flow"],
        vec!["--flow"],
        vec!["--all", "--flow", "daily-work"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.is_empty(), "{arguments:?}");
        if let Some(selector) = arguments.last() {
            assert!(
                stderr.contains(selector),
                "usage diagnostics must name the invalid selector"
            );
        }
    }
}

#[test]
fn audit_selectors_emit_json_only_on_stdout() {
    let single = run(&["--audit-json", "permission-shell"]);
    assert!(single.status.success());
    assert!(single.stderr.is_empty());
    let single_json = String::from_utf8(single.stdout).expect("audit output must be UTF-8");
    assert!(single_json.starts_with("{\"document_type\":\"typed-render-audit\""));
    assert!(single_json.contains("\"screen_id\":\"permission-shell\""));
    assert!(single_json.contains("\"category\":\"input\""));
    assert!(single_json.contains("\"flow\":\"input-and-approval\""));

    let all = run(&["--audit-json", "--all"]);
    assert!(all.status.success());
    assert!(all.stderr.is_empty());
    let all_json = String::from_utf8(all.stdout).expect("audit output must be UTF-8");
    assert!(all_json.starts_with("{\"document_type\":\"typed-render-audit-collection\""));
    assert!(all_json.contains("\"screen_count\":86"));
    assert!(all_json.contains("\"screen_id\":\"home\""));
    assert!(all_json.contains("\"screen_id\":\"tool-envelope\""));
}
