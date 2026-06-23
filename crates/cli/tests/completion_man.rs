//! Black-box checks for shell completions and man-page generation (VAL-FINISH-032..038).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn cortex(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(args)
        .output()
        .expect("spawn cortex binary")
}

fn command_available(command: &str) -> bool {
    Command::new(command).arg("--version").output().is_ok()
}

fn temp_file(name: &str, suffix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    path.push(format!(
        "cortex-{name}-{}-{nanos}{suffix}",
        std::process::id()
    ));
    path
}

#[test]
fn completion_bash_is_non_empty_and_loadable() {
    let out = cortex(&["completion", "bash"]);
    assert!(out.status.success(), "completion bash must exit 0");
    assert!(
        !out.stdout.is_empty(),
        "completion output must be non-empty"
    );

    let script = String::from_utf8_lossy(&out.stdout);
    assert!(
        script.contains("complete") && script.contains("cortex"),
        "bash completion should reference completion registration for cortex:\n{script}"
    );

    let path = temp_file("completion", ".bash");
    fs::write(&path, out.stdout).expect("write bash completion");

    let syntax = Command::new("bash")
        .arg("-n")
        .arg(&path)
        .output()
        .expect("run bash -n");
    assert!(
        syntax.status.success(),
        "bash -n should pass, stderr:\n{}",
        String::from_utf8_lossy(&syntax.stderr)
    );

    let source = Command::new("bash")
        .arg("-lc")
        .arg(format!("source \"{}\"", path.display()))
        .output()
        .expect("source bash completion");
    assert!(
        source.status.success(),
        "sourcing bash completion should succeed, stderr:\n{}",
        String::from_utf8_lossy(&source.stderr)
    );
}

#[test]
fn completion_zsh_and_fish_have_expected_shapes() {
    let zsh = cortex(&["completion", "zsh"]);
    assert!(zsh.status.success(), "completion zsh must exit 0");
    assert!(!zsh.stdout.is_empty(), "zsh completion should be non-empty");
    let zsh_script = String::from_utf8_lossy(&zsh.stdout);
    assert!(
        zsh_script.contains("#compdef cortex") || zsh_script.contains("_cortex"),
        "zsh completion should include a compdef/function marker:\n{zsh_script}"
    );
    if command_available("zsh") {
        let path = temp_file("completion", ".zsh");
        fs::write(&path, &zsh.stdout).expect("write zsh completion");
        let source = Command::new("zsh")
            .arg("-c")
            .arg(format!(
                "autoload -U compinit; source \"{}\"",
                path.display()
            ))
            .output()
            .expect("source zsh completion");
        assert!(
            source.status.success(),
            "sourcing zsh completion should succeed, stderr:\n{}",
            String::from_utf8_lossy(&source.stderr)
        );
    }

    let fish = cortex(&["completion", "fish"]);
    assert!(fish.status.success(), "completion fish must exit 0");
    assert!(
        !fish.stdout.is_empty(),
        "fish completion should be non-empty"
    );
    let fish_script = String::from_utf8_lossy(&fish.stdout);
    assert!(
        fish_script.contains("complete -c cortex"),
        "fish completion should register `complete -c cortex`:\n{fish_script}"
    );
    if command_available("fish") {
        let path = temp_file("completion", ".fish");
        fs::write(&path, &fish.stdout).expect("write fish completion");
        let source = Command::new("fish")
            .arg("-c")
            .arg(format!("source \"{}\"", path.display()))
            .output()
            .expect("source fish completion");
        assert!(
            source.status.success(),
            "sourcing fish completion should succeed, stderr:\n{}",
            String::from_utf8_lossy(&source.stderr)
        );
    }
}

#[test]
fn completion_scripts_include_real_subcommands() {
    let out = cortex(&["completion", "bash"]);
    assert!(out.status.success(), "completion bash must exit 0");
    let script = String::from_utf8_lossy(&out.stdout);
    for token in [
        "run",
        "serve",
        "models",
        "auth",
        "export",
        "import",
        "stats",
        "search",
        "replay",
        "completion",
    ] {
        assert!(
            script.contains(token),
            "completion should include subcommand token `{token}`:\n{script}"
        );
    }
}

#[test]
fn unsupported_completion_shell_fails_cleanly() {
    let out = cortex(&["completion", "powershell-typo"]);
    assert!(
        !out.status.success(),
        "unsupported completion shell should exit non-zero"
    );
    assert!(
        out.stdout.is_empty(),
        "stdout must remain empty on completion-usage errors"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    for shell in ["bash", "zsh", "fish"] {
        assert!(
            stderr.contains(shell),
            "stderr should name supported shell `{shell}`:\n{stderr}"
        );
    }
}

#[test]
fn man_generation_is_local_and_valid_roff() {
    let out = cortex(&["man"]);
    assert!(out.status.success(), "cortex man must exit 0");
    assert!(!out.stdout.is_empty(), "man output must be non-empty");
    let roff = String::from_utf8_lossy(&out.stdout);
    assert!(
        roff.contains(".TH CORTEX"),
        "man output should contain a roff title line for CORTEX:\n{roff}"
    );
    assert!(
        roff.contains(".SH NAME"),
        "man output should include a NAME section:\n{roff}"
    );
    assert!(
        roff.to_lowercase().contains("cortex"),
        "man output should mention the program name:\n{roff}"
    );

    if command_available("man") {
        let path = temp_file("cortex", ".1");
        fs::write(&path, &out.stdout).expect("write man page");
        let render = Command::new("man")
            .env("MANPAGER", "cat")
            .env("PAGER", "cat")
            .arg("-l")
            .arg(&path)
            .output()
            .expect("render generated man page");
        assert!(
            render.status.success(),
            "`man -l` should render generated roff successfully, stderr:\n{}",
            String::from_utf8_lossy(&render.stderr)
        );
    }
}

#[test]
fn completion_and_man_succeed_under_no_net_kill_switch() {
    let completion = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .env("CORTEX_NO_NET", "1")
        .args(["completion", "bash"])
        .output()
        .expect("run completion under CORTEX_NO_NET");
    assert!(completion.status.success(), "completion should stay local");

    let man = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .env("CORTEX_NO_NET", "1")
        .arg("man")
        .output()
        .expect("run man generation under CORTEX_NO_NET");
    assert!(man.status.success(), "man generation should stay local");
}
