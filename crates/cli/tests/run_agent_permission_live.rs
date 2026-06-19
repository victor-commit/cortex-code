//! Black-box regression for per-agent permission rulesets over real `cortex run --agent` turns
//! (VAL-PERM-009): an agent whose `[[agent.<name>.permission]]` ruleset denies `write **` is blocked
//! while a different agent that allows `write **` succeeds, on the SAME config.
//!
//! Root cause this guards against: the headless engine was built from the top-level `[[permission]]`
//! layers only (`load_layered`), so an agent-scoped deny gated nothing. The fix builds the BASE
//! ruleset from `load_agent_rulesets(&inputs).for_agent(<agent>)` when an `--agent` is active, so the
//! active agent's overrides govern the turn.
//!
//! Two scenarios are proven end-to-end:
//! 1. Same config, two agents: agent `build` (allow `write **`) writes the target file while agent
//!    `plan` (deny `write **`) does not — the identical prompt/target decided differently per agent.
//! 2. Isolation (the deny, not persona, is the gate): the eager `build` persona, when its agent
//!    ruleset denies `write **`, has its write tool call refused with a surfaced `permission denied`
//!    while the same persona under an allow ruleset writes the file — so the ruleset alone flips the
//!    outcome.
//!
//! The deterministic mechanism (the engine is built from the agent ruleset and classifies the same
//! write differently per agent) is proven offline by the `cli::permission` unit tests; these live
//! tests confirm the CLI `--agent` wiring reaches a real turn. They are gated on `OPENROUTER_API_KEY`
//! and skip cleanly when it is absent, so the offline `cargo test --workspace` gate stays green;
//! mission validation runs them with the key exported.
//!
//! Process hygiene: each `cortex` child is killed by exact PID if it exceeds the hang-guard timeout;
//! every fixture uses a unique scratch directory under the system temp dir.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The default model the mission targets (good at tool calls); used for the live turns.
const MODEL: &str = "google/gemini-2.5-flash";

/// The target file the agent is asked to create (relative to the run's working dir).
const TARGET: &str = "out.txt";

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A scratch layered-config tree: an isolated `home` (holding the per-run DB) and a `proj` working
/// dir under it (holding the project `cortex.toml` and the target file). `proj` is a child of
/// `home`, so the project-config walk discovers `proj/cortex.toml` and stops at `home`.
fn layered_tree(tag: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "cortex-agent-perm-live-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let proj = home.join("proj");
    fs::create_dir_all(&proj).expect("create project working dir");
    (home, proj)
}

/// A `cortex` command rooted at the project working dir with `home` as its HOME, so the project
/// config under `proj/cortex.toml` is discovered and the per-run DB lives in the isolated home
/// (never touching the real environment). Other config/data env is cleared so the fixture is
/// hermetic and stdout stays pure.
fn cortex_in(home: &Path, proj: &Path, key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.current_dir(proj);
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env("HOME", home);
    c.env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

/// The project config declaring per-agent write rules: agent `plan` denies `write **`, agent `build`
/// allows it. Both agents share the same single config, so only the active agent's ruleset differs.
const PLAN_DENY_BUILD_ALLOW: &str = "\
[[agent.plan.permission]]
tool = \"write\"
pattern = \"**\"
action = \"deny\"

[[agent.build.permission]]
tool = \"write\"
pattern = \"**\"
action = \"allow\"
";

/// A direct prompt instructing the model to create the target file with the write tool.
fn write_prompt() -> String {
    format!(
        "Immediately use the write tool to create a file named {TARGET} in the current directory \
         containing exactly the text AGENTWRITE and nothing else. Use the write tool now — do not \
         ask, plan, or explain first."
    )
}

/// Spawn the binary and collect its output, enforcing `timeout` as a hang guard; the exact child
/// PID is killed if the timeout is exceeded so no orphan is left behind.
fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {timeout:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

/// VAL-PERM-009: with agent `plan` denying `write **` and agent `build` allowing it in the SAME
/// config, `cortex run --agent build` writes the target file while `cortex run --agent plan` does
/// not — the identical prompt/target is decided differently by the active agent's ruleset.
#[test]
fn agent_build_allow_writes_while_agent_plan_deny_blocks_on_the_same_config() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };

    // --- agent `build` (allow write **): the file is created. ---
    let (build_home, build_proj) = layered_tree("perm009-build");
    fs::write(build_proj.join("cortex.toml"), PLAN_DENY_BUILD_ALLOW).unwrap();
    let out = run(
        cortex_in(
            &build_home,
            &build_proj,
            &key,
            &["run", "--agent", "build", "--model", MODEL, &write_prompt()],
        ),
        Duration::from_secs(120),
    );
    assert!(
        out.status.success(),
        "the build turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        build_proj.join(TARGET).exists(),
        "agent `build` allows `write **`, so the target file is created"
    );

    // --- agent `plan` (deny write **): the same prompt/target produces no file. ---
    let (plan_home, plan_proj) = layered_tree("perm009-plan");
    fs::write(plan_proj.join("cortex.toml"), PLAN_DENY_BUILD_ALLOW).unwrap();
    let out = run(
        cortex_in(
            &plan_home,
            &plan_proj,
            &key,
            &["run", "--agent", "plan", "--model", MODEL, &write_prompt()],
        ),
        Duration::from_secs(120),
    );
    assert!(
        out.status.success(),
        "the plan turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !plan_proj.join(TARGET).exists(),
        "agent `plan` denies `write **`, so the same write produces no file"
    );
}

/// VAL-PERM-009 isolation: the ruleset (not the persona) is the gate. The eager `build` persona,
/// under an agent ruleset that denies `write **`, has its write tool call refused with a surfaced
/// `permission denied`; under an allow ruleset the same persona writes the file. Proven with
/// `--format json` so the denied tool result (carrying the refusal text) is observable on stdout.
#[test]
fn agent_deny_blocks_the_eager_build_persona_with_a_surfaced_refusal() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };

    // build under an agent ruleset that DENIES write: the write tool call is refused.
    let (home, proj) = layered_tree("perm009-iso-deny");
    fs::write(
        proj.join("cortex.toml"),
        "[[agent.build.permission]]\ntool = \"write\"\npattern = \"**\"\naction = \"deny\"\n",
    )
    .unwrap();
    let out = run(
        cortex_in(
            &home,
            &proj,
            &key,
            &[
                "run",
                "--agent",
                "build",
                "--model",
                MODEL,
                "--format",
                "json",
                &write_prompt(),
            ],
        ),
        Duration::from_secs(120),
    );
    assert!(
        out.status.success(),
        "the deny turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("permission denied"),
        "the eager build persona's write must be refused by the agent deny ruleset, surfacing \
         `permission denied` in the JSON tool results, got: {stdout}"
    );
    assert!(
        !proj.join(TARGET).exists(),
        "the denied write leaves no file"
    );

    // build under an agent ruleset that ALLOWS write: the same persona writes the file.
    let (home, proj) = layered_tree("perm009-iso-allow");
    fs::write(
        proj.join("cortex.toml"),
        "[[agent.build.permission]]\ntool = \"write\"\npattern = \"**\"\naction = \"allow\"\n",
    )
    .unwrap();
    let out = run(
        cortex_in(
            &home,
            &proj,
            &key,
            &["run", "--agent", "build", "--model", MODEL, &write_prompt()],
        ),
        Duration::from_secs(120),
    );
    assert!(
        out.status.success(),
        "the allow turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        proj.join(TARGET).exists(),
        "with the agent ruleset flipped to allow, the same persona writes the file"
    );
}
