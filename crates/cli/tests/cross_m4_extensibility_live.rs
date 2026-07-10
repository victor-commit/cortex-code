//! Live integration checks for the M4 cross-surface extensibility seam.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and exercise real `cortex run` turns. They focus on
//! the durable artifact that another surface (the TUI session picker) consumes: a `task` subagent
//! leaves its own persisted child session with an observable parent link.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(220);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

struct Scratch {
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
}

fn scratch(tag: &str) -> Scratch {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "cortex-cross-m4-live-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let home = root.join("home");
    let project = root.join("project");
    fs::create_dir_all(&home).expect("create scratch home");
    fs::create_dir_all(&project).expect("create scratch project");
    Scratch {
        home,
        project,
        db: root.join("data/cortex.db"),
    }
}

fn cortex_in(scratch: &Scratch, key: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("OPENROUTER_API_KEY", key);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_NO_NET")
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
            panic!("cortex command did not terminate within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn json_doc(out: Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "command should succeed (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    serde_json::from_str(stdout.trim_end()).expect("stdout is one strict JSON document")
}

fn content(doc: &serde_json::Value) -> &str {
    doc["content"].as_str().expect("assistant content")
}

fn tool_result_text(doc: &serde_json::Value, tool_name: &str) -> String {
    doc["tool_results"]
        .as_array()
        .expect("tool results")
        .iter()
        .find(|result| result["name"] == tool_name)
        .expect("target tool result")
        .get("parts")
        .and_then(serde_json::Value::as_array)
        .expect("parts")
        .iter()
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn child_session_id(task_text: &str) -> String {
    let (_, rest) = task_text
        .split_once("completed in session `")
        .expect("task result names the child session");
    let (id, _) = rest.split_once('`').expect("session id is backtick-closed");
    id.to_owned()
}

#[test]
fn task_subagent_persists_a_listable_child_session_with_parent_link() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let scratch = scratch("task");
    let parent = format!("cross-parent-{}", std::process::id());
    let token = "SUBTASK-LISTABLE-7325";

    let doc = json_doc(run(cortex_in(
        &scratch,
        &key,
        &[
            "run",
            "--session",
            &parent,
            "--model",
            MODEL,
            "--format",
            "json",
            &format!(
                "You MUST call the task tool with agent explore exactly once before answering. \
                 Delegate this exact prompt to the subagent: 'Reply with the exact token {token} \
                 and nothing else.' After the task tool returns, answer with one sentence that \
                 includes {token}."
            ),
        ],
    )));

    let task_text = tool_result_text(&doc, "task");
    let child = child_session_id(&task_text);
    assert_ne!(child, parent, "subagent must get a distinct session");
    assert!(
        task_text.contains(&format!("parent `{parent}`")),
        "task result exposes the parent link: {task_text}"
    );
    assert!(
        content(&doc).contains(token),
        "parent answer should incorporate the subagent result: {}",
        content(&doc)
    );

    let listed = json_doc(run(cortex_in(
        &scratch,
        &key,
        &["session", "list", "--format", "json"],
    )));
    let sessions = listed["sessions"].as_array().expect("sessions array");
    let parent_row = sessions
        .iter()
        .find(|session| session["id"] == parent)
        .expect("parent listed");
    assert_eq!(parent_row["id"], parent);
    let child_row = sessions
        .iter()
        .find(|session| session["id"] == child)
        .expect("child listed");
    assert_eq!(
        child_row["parent_id"], parent,
        "child sessions must expose their structured durable parent link"
    );
    let title = child_row["title"].as_str().expect("child title");
    assert!(
        title.contains("subagent:explore") && title.contains(&format!("parent:{parent}")),
        "child title is the durable parent linkage consumed by the TUI picker: {title}"
    );

    let shown = json_doc(run(cortex_in(
        &scratch,
        &key,
        &["session", "show", &child, "--format", "json"],
    )));
    let messages = shown["messages"].as_array().expect("child messages");
    assert!(
        messages.iter().any(|message| message["content"]
            .as_str()
            .is_some_and(|text| text.contains(token))),
        "child session should persist the delegated prompt and answer: {shown}"
    );
}
