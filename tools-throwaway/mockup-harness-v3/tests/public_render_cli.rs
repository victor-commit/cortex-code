use std::process::{Command, Output};
use std::{
    fs::{self, OpenOptions},
    path::Path,
    sync::{Mutex, MutexGuard, OnceLock},
};

static RENDER_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn renderer_test_lock() -> MutexGuard<'static, ()> {
    RENDER_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("public renderer test lock must not be poisoned")
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
fn implemented_selector_publishes_all_current_pilot_artifacts() {
    let _test_lock = renderer_test_lock();
    let output = run(&["--implemented"]);

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    for (category, screen) in [
        ("session", "home"),
        ("session", "onboarding"),
        ("session", "chat-turn"),
        ("session", "chat-long-scroll"),
        ("session", "streaming"),
        ("session", "reasoning"),
        ("session", "reasoning-collapsed"),
        ("session", "tool-running"),
        ("session", "tool-success"),
        ("session", "tool-success-expanded"),
        ("session", "tool-calls"),
        ("session", "tool-task"),
        ("session", "tool-task-expanded"),
        ("session", "rich-response"),
        ("session", "compact"),
        ("input", "file-mention"),
        ("input", "composer-multiline"),
        ("input", "model-picker"),
        ("input", "permission-shell"),
        ("input", "permission-deny-reason"),
        ("input", "session-picker"),
        ("views", "timeline"),
    ] {
        assert!(
            Path::new(&format!(
                "/projects/cortex-code/plans/assets/mockups-v3/{category}/{screen}.txt"
            ))
            .is_file(),
            "each implemented pilot state must publish canonical TXT"
        );
        assert!(
            Path::new(&format!(
                "/projects/cortex-code/plans/assets/mockups-v3/{category}/{screen}.png"
            ))
            .is_file(),
            "each implemented pilot state must publish canonical PNG"
        );
    }
    assert!(
        Path::new("/projects/cortex-code/plans/assets/mockups-v3/input/permission-shell.txt")
            .is_file(),
        "the shell permission pilot must publish its canonical artifact"
    );
}

#[test]
fn every_render_selector_preflights_missing_renderers_before_writing() {
    let _test_lock = renderer_test_lock();
    assert_registry_failure(&["--all"], "tool-failed");
    assert_registry_failure(&["input"], "slash-menu");

    let path = "/projects/cortex-code/plans/assets/mockups-v3/input/slash-menu.txt";
    assert!(
        !Path::new(path).exists(),
        "preflight must finish before any artifact is written: {path}"
    );
}

#[test]
fn implemented_screen_selectors_publish_atomic_canonical_pairs() {
    let _test_lock = renderer_test_lock();
    for (category, screen) in [
        ("session", "home"),
        ("session", "onboarding"),
        ("session", "chat-turn"),
        ("session", "chat-long-scroll"),
        ("session", "streaming"),
        ("session", "reasoning"),
        ("session", "reasoning-collapsed"),
        ("session", "tool-running"),
        ("session", "tool-success"),
        ("session", "tool-success-expanded"),
        ("session", "tool-calls"),
        ("session", "tool-task"),
        ("session", "tool-task-expanded"),
        ("session", "rich-response"),
        ("session", "compact"),
        ("input", "file-mention"),
        ("input", "composer-multiline"),
        ("input", "model-picker"),
        ("input", "permission-shell"),
        ("input", "permission-deny-reason"),
        ("input", "session-picker"),
        ("views", "timeline"),
    ] {
        let output = run(&[screen]);
        assert!(output.status.success(), "{screen}: {:?}", output);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert!(Path::new(&format!(
            "/projects/cortex-code/plans/assets/mockups-v3/{category}/{screen}.txt"
        ))
        .is_file());
        assert!(Path::new(&format!(
            "/projects/cortex-code/plans/assets/mockups-v3/{category}/{screen}.png"
        ))
        .is_file());
    }
}

#[test]
fn busy_public_renderer_exits_75_without_replacing_the_canonical_gallery() {
    let _test_lock = renderer_test_lock();
    let output_root = Path::new("/projects/cortex-code/plans/assets/mockups-v3");
    let lock = output_root
        .parent()
        .expect("canonical root has a parent")
        .join(".mockups-v3.render.lock");
    let prior_manifest = fs::read(output_root.join(".renderer-manifest-v1"))
        .expect("the implemented gallery must already have an ownership manifest");
    let lock_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .expect("test must own the coordinator lock");
    let output = run(&["--implemented"]);
    drop(lock_file);
    fs::remove_file(&lock).expect("test must remove its coordinator lock");

    assert_eq!(output.status.code(), Some(75));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("render coordinator busy"));
    assert_eq!(
        fs::read(output_root.join(".renderer-manifest-v1"))
            .expect("busy exit must preserve the canonical ownership manifest"),
        prior_manifest,
        "busy exit must publish nothing"
    );
}

#[test]
fn invalid_full_render_keeps_the_last_complete_canonical_publication() {
    let _test_lock = renderer_test_lock();
    let output_root = Path::new("/projects/cortex-code/plans/assets/mockups-v3");
    let initial = run(&["--implemented"]);
    assert!(initial.status.success());
    let prior_manifest = fs::read(output_root.join(".renderer-manifest-v1"))
        .expect("the successful implementation set must publish a manifest");

    let rejected = run(&["--all"]);

    assert_eq!(rejected.status.code(), Some(3));
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr)
        .contains("canonical ID has no implemented renderer: tool-failed"));
    assert_eq!(
        fs::read(output_root.join(".renderer-manifest-v1"))
            .expect("a rejected selector must retain the complete gallery"),
        prior_manifest,
        "preflight failure must not replace a complete published root"
    );
}

#[test]
fn unknown_selectors_are_usage_errors_without_writes() {
    let _test_lock = renderer_test_lock();
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
    let _test_lock = renderer_test_lock();
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
