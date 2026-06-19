//! M2 capstone: a real `cortex run` turn proves the agent CODES FOR REAL, not just narrates
//! (VAL-TOOLS-040).
//!
//! Given a real OpenRouter turn (default model `google/gemini-2.5-flash`) and a natural-language
//! instruction to create a working source file, the agent must autonomously select and drive a
//! mutating tool (`write`/`edit`, or `apply_patch` for GPT-family models) under the permission
//! engine, with a pre-mutation checkpoint recorded, so the requested file actually exists on disk
//! afterward with correct, functional content. This is the end-to-end proof that the full
//! tools + permissions + checkpoints stack works together over a live turn — the individual
//! mechanisms are proven by the `tools`/`permissions`/`checkpoints` unit suites and the
//! `run_permission_live` black-box tests; this file locks the whole stack as one regression.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent, so the offline
//! `cargo test --workspace` gate stays green; mission validation runs them with the key exported
//! (`set -a; . /projects/cortex-code/.env.local; set +a`).
//!
//! Process hygiene: each `cortex` child is killed by exact PID if it exceeds the hang-guard timeout;
//! every fixture is fully hermetic (isolated HOME, checkpoint store, and database under a unique
//! scratch directory beneath the system temp dir), so a run never touches the real environment.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The mission's default model, which is good at tool calls; used for the live turns.
const MODEL: &str = "google/gemini-2.5-flash";

/// The exact VAL-TOOLS-040 evidence prompt: a natural-language instruction to create a working
/// source file (no tool named, so the agent must select and drive the tool itself).
const CREATE_PROMPT: &str = "create add.py with a function add(a,b) returning their sum";

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A hermetic scratch tree for one fixture: an isolated `home` (DB + global config never touch the
/// real environment), a `proj` working directory the agent operates in (relative tool paths resolve
/// against it), an isolated checkpoint store base, and an isolated data dir.
struct Sandbox {
    proj: PathBuf,
    home: PathBuf,
    ckpt: PathBuf,
    data: PathBuf,
}

/// Build a unique sandbox under the system temp dir.
fn sandbox(tag: &str) -> Sandbox {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "cortex-capstone-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let sb = Sandbox {
        proj: root.join("proj"),
        home: root.join("home"),
        ckpt: root.join("ckpt"),
        data: root.join("data"),
    };
    for dir in [&sb.proj, &sb.home, &sb.ckpt, &sb.data] {
        fs::create_dir_all(dir).expect("create sandbox dir");
    }
    sb
}

/// A `cortex` command rooted at the sandbox project dir, fully isolated: the live key is supplied,
/// HOME / the checkpoint base / the data dir point at the sandbox (so the DB and shadow store never
/// touch the real environment), and styling/verbosity/net-kill env is cleared so stdout stays clean
/// and relative tool paths resolve against `proj`. Running `cortex run` and a later `cortex undo`
/// through this helper addresses the same isolated checkpoint store (same cwd + `CORTEX_CHECKPOINT_DIR`).
fn cortex(sb: &Sandbox, key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.current_dir(&sb.proj);
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env("HOME", &sb.home);
    c.env("CORTEX_CHECKPOINT_DIR", &sb.ckpt);
    c.env("CORTEX_DATA_DIR", &sb.data);
    c.env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
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

/// Parse a `--format jsonl` stream into one JSON value per non-empty line.
fn jsonl_values(bytes: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect()
}

/// Whether any event is a SUCCESSFUL mutating-tool result — the on-disk effect was produced by a
/// real `write`/`edit`/`apply_patch` tool call (not a prose description of the change).
fn successful_mutation(events: &[serde_json::Value]) -> bool {
    events.iter().any(|event| {
        event["type"] == "tool_result"
            && matches!(
                event["name"].as_str(),
                Some("write") | Some("edit") | Some("apply_patch")
            )
            && event["is_error"] == serde_json::Value::Bool(false)
    })
}

/// Resolve an available Python interpreter (`python3`, then `python`), or `None` so the functional
/// run-the-code sub-check is skipped cleanly on a host without Python (the file-content check still
/// runs).
fn python_interpreter() -> Option<&'static str> {
    for bin in ["python3", "python"] {
        let ok = Command::new(bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if ok {
            return Some(bin);
        }
    }
    None
}

/// Assert the created `add.py` is FUNCTIONAL: importing it and calling `add(2, 3)` prints `5`
/// (VAL-TOOLS-040 evidence). Skipped cleanly when no Python interpreter is present, after first
/// confirming the file at least defines an `add` function.
fn assert_add_py_runs(proj: &Path) {
    let source = fs::read_to_string(proj.join("add.py")).expect("read add.py");
    assert!(
        source.contains("def add"),
        "add.py must define an add function, got:\n{source}"
    );

    let Some(python) = python_interpreter() else {
        eprintln!("note: no Python interpreter found; skipped the import-and-run sub-check");
        return;
    };
    let out = Command::new(python)
        .current_dir(proj)
        .args(["-c", "import add; print(add.add(2,3))"])
        .output()
        .expect("run python add check");
    assert!(
        out.status.success(),
        "python import-and-run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "5",
        "add(2,3) must print 5, proving the agent wrote correct, functional content"
    );
}

/// VAL-TOOLS-040 (the capstone): a real OpenRouter turn driven by the natural-language instruction
/// "create add.py …" produces an on-disk `add.py` via a REAL mutating tool call, the file is
/// functional (`add(2,3)` prints `5`), and the mutation is undoable through the pre-mutation
/// checkpoint (`cortex undo` removes the freshly-created file). One live turn proves the full
/// tools + permissions + checkpoints stack end-to-end.
#[test]
fn agent_codes_for_real_creates_a_working_file_and_is_undoable() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let sb = sandbox("codes-for-real");

    let out = run(
        cortex(
            &sb,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--format",
                "jsonl",
                // Attach the permission engine so the mutation is genuinely permission-gated
                // (the allow rule lets the create proceed; the deny path is covered below).
                "--permission-allow",
                "*:**",
                CREATE_PROMPT,
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );

    // The requested file actually exists on disk afterward.
    assert!(
        sb.proj.join("add.py").exists(),
        "the agent must create add.py on disk, not merely describe it"
    );

    // The on-disk effect was produced by a real mutating tool call, not a prose description.
    let events = jsonl_values(&out.stdout);
    assert!(
        successful_mutation(&events),
        "add.py must be produced by a successful write/edit/apply_patch tool call, not narration; \
         events: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // The content is correct and functional: add(2, 3) prints 5.
    assert_add_py_runs(&sb.proj);

    // A pre-mutation checkpoint was recorded, so the create is undoable: `cortex undo` restores the
    // pre-turn state (the file was absent before), removing add.py.
    let undo = run(
        cortex(&sb, &key, &["undo", "--format", "json"]),
        Duration::from_secs(30),
    );
    assert!(
        undo.status.success(),
        "cortex undo succeeds (stderr: {})",
        String::from_utf8_lossy(&undo.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&undo.stdout).expect("undo emits a JSON report");
    assert_eq!(
        report["status"],
        "restored",
        "undo must restore the pre-mutation checkpoint, got: {}",
        String::from_utf8_lossy(&undo.stdout)
    );
    assert!(
        report["paths"]
            .as_array()
            .is_some_and(|paths| paths.iter().any(|path| path == "add.py")),
        "the undo must report add.py among the restored paths, got: {}",
        String::from_utf8_lossy(&undo.stdout)
    );
    assert!(
        !sb.proj.join("add.py").exists(),
        "undoing the pre-mutation checkpoint must remove the freshly-created add.py"
    );
}

/// VAL-TOOLS-040 (permission gating): the SAME create instruction under a ruleset that denies every
/// mutating tool produces NO on-disk file — the create is genuinely gated by the permission engine,
/// and no mutating tool call ever succeeds. This proves the mutation is permission-gated rather than
/// running unconditionally.
#[test]
fn the_create_mutation_is_permission_gated() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let sb = sandbox("gated");

    let out = run(
        cortex(
            &sb,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--format",
                "jsonl",
                "--permission-allow",
                "*:**",
                // Deny every path through which the file could be created.
                "--permission-deny",
                "write:**",
                "--permission-deny",
                "edit:**",
                "--permission-deny",
                "apply_patch:**",
                "--permission-deny",
                "bash:*",
                CREATE_PROMPT,
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes even though the mutation is denied (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !sb.proj.join("add.py").exists(),
        "with every mutating tool denied, the permission engine must keep add.py off disk"
    );
    let events = jsonl_values(&out.stdout);
    assert!(
        !successful_mutation(&events),
        "no mutating tool call may succeed when all are denied; events: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
