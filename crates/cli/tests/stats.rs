//! Black-box checks for `cortex stats` JSON/text output against persisted SQLite usage rows.
//! Coverage: fresh DB zero totals, strict JSON, per-model/session breakdown, and internal sums.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use db::Database;
use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

struct Sandbox {
    root: PathBuf,
    project: PathBuf,
    db_path: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-stats-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let project = root.join("project");
        std::fs::create_dir_all(&project).expect("create project dir");
        let db_path = root.join("state/cortex.db");
        Self {
            root,
            project,
            db_path,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .arg("stats")
            .args(args)
            .current_dir(&self.project)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join(".config"))
            .env("CORTEX_DB_PATH", &self.db_path)
            .env("CORTEX_NO_NET", "1");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run(mut command: Command) -> Output {
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
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex stats did not terminate within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn parse_stats_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is strict JSON")
}

fn total_tokens(value: &Value) -> u64 {
    value["total_tokens"]
        .as_u64()
        .expect("total_tokens is an integer")
}

fn total_cost(value: &Value) -> f64 {
    value["cost"].as_f64().expect("cost is numeric")
}

// VAL-FINISH-021 + VAL-FINISH-023: a fresh data dir emits strict JSON with zero totals, numeric
// cost, and empty model/session breakdown arrays (no phantom usage).
#[test]
fn fresh_db_reports_zero_usage_and_empty_breakdowns() {
    let sandbox = Sandbox::new("fresh");

    let out = run(sandbox.command(&["--format", "json"]));
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = parse_stats_json(&out);

    assert_eq!(doc["totals"]["input_tokens"], 0);
    assert_eq!(doc["totals"]["output_tokens"], 0);
    assert_eq!(doc["totals"]["total_tokens"], 0);
    assert_eq!(doc["totals"]["cost"], 0.0);
    assert_eq!(
        doc["models"].as_array().expect("models array").len(),
        0,
        "fresh DB should have no model breakdown"
    );
    assert_eq!(
        doc["sessions"].as_array().expect("sessions array").len(),
        0,
        "fresh DB should have no session breakdown"
    );
}

// VAL-FINISH-024: model/session breakdown sums are internally consistent with the overall totals.
#[test]
fn breakdowns_sum_to_reported_totals() {
    let sandbox = Sandbox::new("breakdown");
    let db = Database::open(&sandbox.db_path).expect("open db");

    let s1 = db.sessions().create(None).expect("session 1");
    db.sessions()
        .set_model(&s1.id, Some("google/gemini-2.5-flash"))
        .expect("set model s1");
    db.usage()
        .record(&s1.id, 5, 7, 12, Some(0.10))
        .expect("usage s1 turn1");
    db.usage()
        .record(&s1.id, 3, 2, 5, Some(0.05))
        .expect("usage s1 turn2");

    let s2 = db.sessions().create(None).expect("session 2");
    db.sessions()
        .set_model(&s2.id, Some("anthropic/claude-sonnet-4"))
        .expect("set model s2");
    db.usage()
        .record(&s2.id, 11, 13, 24, Some(0.20))
        .expect("usage s2 turn1");

    let out = run(sandbox.command(&["--format", "json"]));
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = parse_stats_json(&out);
    let totals = &doc["totals"];

    assert_eq!(totals["input_tokens"], 19);
    assert_eq!(totals["output_tokens"], 22);
    assert_eq!(totals["total_tokens"], 41);
    assert!((totals["cost"].as_f64().expect("cost") - 0.35).abs() < 1e-9);

    let model_rows = doc["models"].as_array().expect("models array");
    let model_token_sum: u64 = model_rows
        .iter()
        .map(|row| total_tokens(&row["usage"]))
        .sum();
    let model_cost_sum: f64 = model_rows.iter().map(|row| total_cost(&row["usage"])).sum();
    assert_eq!(model_token_sum, total_tokens(totals));
    assert!((model_cost_sum - total_cost(totals)).abs() < 1e-9);
    assert!(
        model_rows
            .iter()
            .any(|row| row["model"] == "google/gemini-2.5-flash"),
        "default model must be attributed in the model breakdown"
    );
    assert!(
        model_rows
            .iter()
            .any(|row| row["model"] == "anthropic/claude-sonnet-4"),
        "second model must be attributed in the model breakdown"
    );

    let session_rows = doc["sessions"].as_array().expect("sessions array");
    let session_token_sum: u64 = session_rows
        .iter()
        .map(|row| total_tokens(&row["usage"]))
        .sum();
    let session_cost_sum: f64 = session_rows
        .iter()
        .map(|row| total_cost(&row["usage"]))
        .sum();
    assert_eq!(session_token_sum, total_tokens(totals));
    assert!((session_cost_sum - total_cost(totals)).abs() < 1e-9);
    assert!(
        session_rows
            .iter()
            .any(|row| row["session_id"] == s1.id && row["model"] == "google/gemini-2.5-flash")
    );
    assert!(
        session_rows
            .iter()
            .any(|row| row["session_id"] == s2.id && row["model"] == "anthropic/claude-sonnet-4")
    );
}
