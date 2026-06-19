//! Black-box checks for argument/usage error handling, diagnostic routing, and the
//! stable exit-code policy of the `cortex` binary (VAL-SKEL-009..012).

use std::process::{Command, Output};

fn cortex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(args)
        .output()
        .expect("spawn cortex binary")
}

#[test]
fn unknown_subcommand_errors_to_stderr_only() {
    let out = cortex(&["bogus-subcommand"]);

    assert!(
        !out.status.success(),
        "unknown subcommand must exit non-zero"
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "usage errors exit with the stable code 2"
    );
    assert!(
        out.stdout.is_empty(),
        "stdout must be byte-empty on a usage error"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.is_empty(), "stderr must carry the diagnostic");
    assert!(
        stderr.contains("bogus-subcommand"),
        "stderr should name the unknown subcommand, got: {stderr}"
    );
}

#[test]
fn unknown_flag_errors_to_stderr_only() {
    let out = cortex(&["run", "--not-a-flag"]);

    assert!(!out.status.success(), "unknown flag must exit non-zero");
    assert_eq!(
        out.status.code(),
        Some(2),
        "usage errors exit with the stable code 2"
    );
    assert!(
        out.stdout.is_empty(),
        "stdout must be byte-empty on a usage error"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--not-a-flag"),
        "stderr should name the offending flag, got: {stderr}"
    );
}

#[test]
fn version_prints_to_stdout_and_exits_zero() {
    let out = cortex(&["--version"]);

    assert!(out.status.success(), "--version must exit 0");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "no diagnostics on a successful --version"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.to_lowercase().contains("cortex"),
        "version line should name the product, got: {stdout}"
    );
}

#[test]
fn short_version_matches_long_version() {
    let long = cortex(&["--version"]);
    let short = cortex(&["-V"]);

    assert!(long.status.success() && short.status.success());
    assert_eq!(long.status.code(), Some(0));
    assert_eq!(short.status.code(), Some(0));
    assert_eq!(long.stdout, short.stdout, "-V must match --version output");
}

#[test]
fn help_prints_to_stdout_and_exits_zero() {
    for flag in ["--help", "-h"] {
        let out = cortex(&[flag]);
        assert!(out.status.success(), "{flag} must exit 0");
        assert_eq!(out.status.code(), Some(0));
        assert!(out.stderr.is_empty(), "{flag} must not write to stderr");
        assert!(!out.stdout.is_empty(), "{flag} must write help to stdout");
    }
}

#[test]
fn usage_exit_code_is_deterministic_across_runs() {
    let first = cortex(&["bogus-subcommand"]);
    let second = cortex(&["bogus-subcommand"]);
    assert_eq!(
        first.status.code(),
        second.status.code(),
        "the same failure class must yield the same exit code on repeated runs"
    );
}

#[test]
fn argument_error_classes_share_the_usage_code() {
    let unknown_sub = cortex(&["bogus-subcommand"]);
    let unknown_flag = cortex(&["run", "--not-a-flag"]);
    let bad_value = cortex(&["run", "--format", "not-a-format"]);

    assert_eq!(unknown_sub.status.code(), Some(2));
    assert_eq!(unknown_flag.status.code(), Some(2));
    assert_eq!(bad_value.status.code(), Some(2));
    for out in [&unknown_sub, &unknown_flag, &bad_value] {
        assert!(out.stdout.is_empty(), "argument errors keep stdout clean");
        assert!(!out.stderr.is_empty(), "argument errors report on stderr");
    }
}
