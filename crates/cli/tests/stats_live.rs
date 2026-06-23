//! Live, black-box proof for `cortex stats` against REAL `cortex run` turns.
//!
//! Covers:
//! - VAL-FINISH-019: totals reflect real persisted usage/cost from prior runs.
//! - VAL-FINISH-020: totals grow strictly after another real run.
//! - VAL-FINISH-022: stats survive process restart (separate invocations).
//! - VAL-FINISH-023: `--format json` is strict parseable JSON.
//! - VAL-FINISH-024: model/session breakdown sums to the reported total.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const TURN_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-stats-live-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

fn cortex(key: &str, home: &PathBuf, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("OPENROUTER_API_KEY", key)
        .env_remove("CORTEX_NO_NET")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_MODEL")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
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
        if start.elapsed() > TURN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command did not terminate within {TURN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn json_doc(output: Output, what: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{what} must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is strict JSON")
}

fn total_tokens(doc: &Value) -> u64 {
    doc["totals"]["total_tokens"]
        .as_u64()
        .expect("totals.total_tokens is integer")
}

fn total_cost(doc: &Value) -> f64 {
    doc["totals"]["cost"]
        .as_f64()
        .expect("totals.cost is numeric")
}

#[test]
fn stats_accumulate_real_usage_and_attribute_breakdowns() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();

    // First real turn (process A) writes real usage to persistence.
    let _turn1 = json_doc(
        run(cortex(
            &key,
            &home,
            &[
                "run",
                "--format",
                "json",
                "Reply with exactly: STATS-LIVE-TURN-ONE",
            ],
        )),
        "first run",
    );

    // Separate process (process B) reads persisted totals.
    let stats_after_first = json_doc(
        run(cortex(&key, &home, &["stats", "--format", "json"])),
        "stats after first run",
    );
    assert!(
        stats_after_first["totals"]["input_tokens"]
            .as_u64()
            .expect("input_tokens")
            >= 1
    );
    assert!(
        stats_after_first["totals"]["output_tokens"]
            .as_u64()
            .expect("output_tokens")
            >= 1
    );
    assert!(total_tokens(&stats_after_first) >= 2);
    assert!(
        total_cost(&stats_after_first) > 0.0,
        "paid default model should report positive cost"
    );

    // Second real turn, then stats must grow strictly.
    let _turn2 = json_doc(
        run(cortex(
            &key,
            &home,
            &[
                "run",
                "--format",
                "json",
                "Reply with exactly: STATS-LIVE-TURN-TWO",
            ],
        )),
        "second run",
    );
    let stats_after_second = json_doc(
        run(cortex(&key, &home, &["stats", "--format", "json"])),
        "stats after second run",
    );

    assert!(
        total_tokens(&stats_after_second) > total_tokens(&stats_after_first),
        "total tokens must grow after an additional real run"
    );
    assert!(
        total_cost(&stats_after_second) > total_cost(&stats_after_first),
        "total cost must grow after an additional real run"
    );

    let model_rows = stats_after_second["models"]
        .as_array()
        .expect("models array");
    assert!(
        model_rows.iter().any(|row| row["model"] == DEFAULT_MODEL),
        "model breakdown should attribute usage to the default model"
    );

    let model_token_sum: u64 = model_rows
        .iter()
        .map(|row| {
            row["usage"]["total_tokens"]
                .as_u64()
                .expect("model total tokens")
        })
        .sum();
    let model_cost_sum: f64 = model_rows
        .iter()
        .map(|row| row["usage"]["cost"].as_f64().expect("model cost"))
        .sum();
    assert_eq!(model_token_sum, total_tokens(&stats_after_second));
    assert!((model_cost_sum - total_cost(&stats_after_second)).abs() < 1e-9);

    let session_rows = stats_after_second["sessions"]
        .as_array()
        .expect("sessions array");
    let session_token_sum: u64 = session_rows
        .iter()
        .map(|row| {
            row["usage"]["total_tokens"]
                .as_u64()
                .expect("session total tokens")
        })
        .sum();
    let session_cost_sum: f64 = session_rows
        .iter()
        .map(|row| row["usage"]["cost"].as_f64().expect("session cost"))
        .sum();
    assert_eq!(session_token_sum, total_tokens(&stats_after_second));
    assert!((session_cost_sum - total_cost(&stats_after_second)).abs() < 1e-9);
}
