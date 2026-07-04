//! Black-box checks for the `CORTEX_NO_NET` fail-closed network kill-switch, exercised through
//! the `cortex net check` network-touching command path (VAL-SKEL-032..036).

use std::net::TcpListener;
use std::process::Command;

/// The dedicated exit code for a kill-switch refusal (`Exit::NetDisabled`).
const NET_DISABLED_CODE: i32 = 4;

/// A clean `cortex` invocation: the kill-switch and any inherited credential/verbosity are
/// removed so each test sets exactly the environment it intends to exercise.
fn cortex() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cortex"));
    cmd.env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    cmd
}

/// Obtain a loopback port that refuses connections, so a probe against it yields a deterministic
/// transport failure (never a hang): bind an OS-assigned ephemeral port, read it, then drop the
/// listener so the port is closed.
fn closed_local_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

/// The `net check` arguments aimed at a guaranteed-closed loopback port.
fn check_args(port: u16) -> Vec<String> {
    vec![
        "net".into(),
        "check".into(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--timeout".into(),
        "2".into(),
    ]
}

#[test]
fn no_net_refuses_before_any_socket_with_a_typed_net_disabled_diagnostic() {
    let port = closed_local_port();
    let out = cortex()
        .env("CORTEX_NO_NET", "1")
        .args(check_args(port))
        .output()
        .expect("spawn cortex");

    assert_eq!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "the kill-switch must exit with the dedicated net-disabled code"
    );
    assert!(
        out.stdout.is_empty(),
        "a refusal keeps stdout byte-empty for machine consumers"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("network disabled"),
        "stderr must read as a net-disabled error: {stderr}"
    );
    assert!(
        stderr.contains("CORTEX_NO_NET"),
        "stderr must name the kill-switch: {stderr}"
    );
    // Distinct from the other canonical error classes.
    assert!(
        !stderr.contains("transport unavailable"),
        "net-disabled must not read like a transport error: {stderr}"
    );
    assert!(
        !stderr.contains("missing credential"),
        "net-disabled must not read like a credential error: {stderr}"
    );
}

#[test]
fn no_net_fails_closed_even_when_a_credential_is_present() {
    let port = closed_local_port();
    let out = cortex()
        .env("CORTEX_NO_NET", "1")
        .env("OPENROUTER_API_KEY", "sk-or-v1-not-a-real-key")
        .args(check_args(port))
        .output()
        .expect("spawn cortex");

    assert_eq!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "a present credential must not let the command slip past the kill-switch"
    );
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("network disabled") && stderr.contains("CORTEX_NO_NET"),
        "must still be a net-disabled refusal: {stderr}"
    );
}

#[test]
fn without_the_switch_the_probe_reaches_a_real_socket_and_a_transport_error_is_distinct() {
    let port = closed_local_port();
    let out = cortex()
        .args(check_args(port))
        .output()
        .expect("spawn cortex");

    // The guard let the command through to a real connection attempt, which a closed port
    // refuses — a transport failure that is plainly NOT a net-disabled refusal.
    assert_ne!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "an unset switch must never produce the net-disabled code"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a refused connection is a generic transport failure"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("transport unavailable"),
        "stderr must read as a transport error: {stderr}"
    );
    assert!(
        !stderr.contains("network disabled"),
        "a transport error must not be confused with a net-disabled refusal: {stderr}"
    );
}

#[test]
fn local_only_commands_succeed_under_the_kill_switch() {
    // --version
    let version = cortex()
        .env("CORTEX_NO_NET", "1")
        .arg("--version")
        .output()
        .expect("spawn cortex");
    assert_eq!(
        version.status.code(),
        Some(0),
        "--version must still exit 0"
    );
    assert!(version.stderr.is_empty(), "--version emits no diagnostic");

    // --help
    let help = cortex()
        .env("CORTEX_NO_NET", "1")
        .arg("--help")
        .output()
        .expect("spawn cortex");
    assert_eq!(help.status.code(), Some(0), "--help must still exit 0");

    // NOTE: after the M0→M1 transition `cortex run` performs a real network turn, so it is no
    // longer a local-only command — under the kill-switch it fails closed with net-disabled (see
    // run_echo.rs). It is intentionally not asserted here.

    // config read
    let config = cortex()
        .env("CORTEX_NO_NET", "1")
        .arg("config")
        .output()
        .expect("spawn cortex");
    assert_eq!(config.status.code(), Some(0), "config reads are local-only");
}

#[test]
fn only_the_documented_truthy_value_engages_the_switch() {
    let port = closed_local_port();

    // The documented value blocks.
    let blocked = cortex()
        .env("CORTEX_NO_NET", "1")
        .args(check_args(port))
        .output()
        .expect("spawn cortex");
    assert_eq!(blocked.status.code(), Some(NET_DISABLED_CODE));

    // Every other value (and empty) leaves the network enabled.
    for value in ["0", "", "true", "yes", "on", "2", "11"] {
        let out = cortex()
            .env("CORTEX_NO_NET", value)
            .args(check_args(port))
            .output()
            .expect("spawn cortex");
        assert_ne!(
            out.status.code(),
            Some(NET_DISABLED_CODE),
            "value {value:?} must not engage the kill-switch"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("network disabled"),
            "value {value:?} must not produce a net-disabled error: {stderr}"
        );
    }
}

#[test]
fn on_off_behavior_is_deterministic_across_runs() {
    let port = closed_local_port();

    // Engaged: same net-disabled outcome both runs.
    let blocked_a = cortex()
        .env("CORTEX_NO_NET", "1")
        .args(check_args(port))
        .output()
        .expect("spawn cortex");
    let blocked_b = cortex()
        .env("CORTEX_NO_NET", "1")
        .args(check_args(port))
        .output()
        .expect("spawn cortex");
    assert_eq!(blocked_a.status.code(), Some(NET_DISABLED_CODE));
    assert_eq!(
        blocked_a.status.code(),
        blocked_b.status.code(),
        "the engaged switch must be reproducible"
    );

    // Unset: same (non-net-disabled) outcome both runs.
    let open_a = cortex()
        .args(check_args(port))
        .output()
        .expect("spawn cortex");
    let open_b = cortex()
        .args(check_args(port))
        .output()
        .expect("spawn cortex");
    assert_ne!(open_a.status.code(), Some(NET_DISABLED_CODE));
    assert_eq!(
        open_a.status.code(),
        open_b.status.code(),
        "the disengaged switch must be reproducible"
    );
}

/// Whether `strace` can trace a child here (it may be absent or `ptrace` may be restricted).
fn strace_usable() -> bool {
    let probe = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "--version",
        ])
        .output();
    match probe {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            out.status.success() && !stderr.to_lowercase().contains("ptrace")
        }
        Err(_) => false,
    }
}

/// Run `cortex` under `strace`, returning the connect-trace captured on strace's stderr.
fn strace_connect_trace(args: &[String], no_net: Option<&str>) -> String {
    let mut cmd = Command::new("strace");
    cmd.args(["-f", "-e", "trace=connect", env!("CARGO_BIN_EXE_cortex")])
        .args(args)
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY");
    if let Some(value) = no_net {
        cmd.env("CORTEX_NO_NET", value);
    }
    let out = cmd.output().expect("spawn strace");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn count_connect(trace: &str) -> usize {
    trace.matches("connect(").count()
}

#[test]
fn kill_switch_makes_zero_connect_syscalls_to_remote_addresses() {
    if !strace_usable() {
        eprintln!("skipping strace assertion: strace unavailable or ptrace restricted");
        return;
    }
    let port = closed_local_port();
    let args = check_args(port);

    let blocked = strace_connect_trace(&args, Some("1"));
    assert_eq!(
        count_connect(&blocked),
        0,
        "under CORTEX_NO_NET=1 the command must issue zero connect() syscalls:\n{blocked}"
    );

    let open = strace_connect_trace(&args, None);
    assert!(
        count_connect(&open) >= 1,
        "with the switch unset the command must reach a real connect():\n{open}"
    );
}
