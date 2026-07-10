//! Live integration checks for the M4 cross-surface extensibility seam.
//!
//! These tests use real OpenRouter turns against one disposable project and data directory per
//! flow. They prove project memory and skill guidance shape both headless and TUI answers, changes
//! on disk take effect in later processes, and a delegated task leaves a durable child session that
//! the TUI can inspect.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(220);
const TUI_TIMEOUT_MS: u64 = 180_000;

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

static LIVE_TEST_LOCK: Mutex<()> = Mutex::new(());
static TUI_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Scratch {
    root: PathBuf,
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
    fs::create_dir_all(root.join("data")).expect("create scratch data");
    let db = root.join("data").join("cortex.db");
    Scratch {
        root,
        home,
        project,
        db,
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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

fn tool_was_called(doc: &serde_json::Value, tool_name: &str) -> bool {
    doc["tool_results"]
        .as_array()
        .is_some_and(|results| results.iter().any(|result| result["name"] == tool_name))
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

fn write_skill(scratch: &Scratch, name: &str, description: &str, guidance: &str) -> PathBuf {
    let path = scratch.project.join("skills").join(name).join("SKILL.md");
    fs::create_dir_all(path.parent().expect("skill parent")).expect("create skill parent");
    fs::write(
        &path,
        format!("---\nname: {name}\ndescription: {description}\n---\n{guidance}\n"),
    )
    .expect("write skill");
    path
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

struct TuiSession {
    name: String,
}

impl TuiSession {
    fn launch(scratch: &Scratch, key: &str, persisted_session: Option<&str>) -> Self {
        let number = TUI_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("cortex-cross-m4-{}-{number}", std::process::id());
        let command = match persisted_session {
            Some(session) => format!(
                "{} tui --session {session} --no-anim",
                env!("CARGO_BIN_EXE_cortex")
            ),
            None => format!("{} tui --no-anim", env!("CARGO_BIN_EXE_cortex")),
        };
        let output = Command::new("tuistory")
            .arg("launch")
            .arg("--session")
            .arg(&name)
            .arg("--background")
            .arg("--cwd")
            .arg(&scratch.project)
            .arg("--cols")
            .arg("240")
            .arg("--rows")
            .arg("60")
            .arg("--env")
            .arg(format!("HOME={}", scratch.home.display()))
            .arg("--env")
            .arg(format!(
                "XDG_CONFIG_HOME={}",
                scratch.home.join(".config").display()
            ))
            .arg("--env")
            .arg(format!(
                "XDG_DATA_HOME={}",
                scratch.home.join(".local/share").display()
            ))
            .arg("--env")
            .arg(format!("CORTEX_DB_PATH={}", scratch.db.display()))
            .arg("--env")
            .arg(format!("OPENROUTER_API_KEY={key}"))
            .arg("--env")
            .arg("CORTEX_DATA_DIR=")
            .arg("--env")
            .arg("CORTEX_AUTH_FILE=")
            .arg("--env")
            .arg("CORTEX_NO_NET=")
            .arg("--env")
            .arg("CORTEX_OPENROUTER_BASE_URL=")
            .arg("--env")
            .arg("CORTEX_NO_ANIM=1")
            .arg("--env")
            .arg("RUST_LOG=")
            .arg("--env")
            .arg("CORTEX_VERBOSE=")
            .arg("--env")
            .arg("NO_COLOR=1")
            .arg(command)
            .output()
            .expect("launch tuistory session");
        assert!(
            output.status.success(),
            "tuistory launch failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Self { name }
    }

    fn type_text(&self, text: &str) {
        let output = Command::new("tuistory")
            .args(["-s", &self.name, "type", text])
            .output()
            .expect("type TUI input");
        assert!(
            output.status.success(),
            "tuistory type failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn press_enter(&self) {
        let output = Command::new("tuistory")
            .args(["-s", &self.name, "press", "enter"])
            .output()
            .expect("press TUI enter");
        assert!(
            output.status.success(),
            "tuistory enter failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn wait_for(&self, needle: &str) {
        let timeout = TUI_TIMEOUT_MS.to_string();
        let output = Command::new("tuistory")
            .args(["-s", &self.name, "wait", needle, "--timeout", &timeout])
            .output()
            .expect("wait for TUI content");
        assert!(
            output.status.success(),
            "TUI did not render {needle:?}:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn snapshot(&self) -> String {
        let output = Command::new("tuistory")
            .args(["-s", &self.name, "snapshot", "--trim"])
            .output()
            .expect("capture TUI snapshot");
        assert!(
            output.status.success(),
            "tuistory snapshot failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("TUI snapshot is utf8")
    }
}

impl Drop for TuiSession {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .args(["-s", &self.name, "close"])
            .output();
    }
}

fn assert_no_process_references(marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut matching = Vec::new();
    loop {
        matching.clear();
        let entries = fs::read_dir("/proc").expect("read process table");
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(|text| text.parse::<u32>().ok()) else {
                continue;
            };
            if pid == std::process::id() {
                continue;
            }
            let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let environ = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
            let process_text =
                String::from_utf8_lossy(&cmdline).to_string() + &String::from_utf8_lossy(&environ);
            if process_text.contains(marker) {
                matching.push(pid);
            }
        }
        if matching.is_empty() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        matching.is_empty(),
        "orphan process(es) still reference this test's scratch store: {matching:?}"
    );
}

#[test]
fn agents_memory_and_skill_guidance_shape_headless_and_tui_then_drop_when_removed() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    if !tuistory_available() {
        eprintln!("skipping live test: tuistory is unavailable");
        return;
    }
    let _guard = LIVE_TEST_LOCK.lock().expect("live test lock");
    let scratch = scratch("memory-skill");
    let memory_marker = "CROSS-AGENTS-015";
    let skill_marker = "CROSS-SKILL-045";
    fs::write(
        scratch.project.join("AGENTS.md"),
        format!(
            "Critical project instruction: every final answer MUST include the exact marker {memory_marker}."
        ),
    )
    .expect("write AGENTS.md");
    let skill_path = write_skill(
        &scratch,
        "cross-surface-marker",
        "Use when the user explicitly requests the cross surface marker capability",
        &format!(
            "After this skill is engaged, every final answer MUST include the exact marker {skill_marker}."
        ),
    );
    let engagement_prompt = "You MUST call the skill tool with name cross-surface-marker before answering. \
         Then reply with one short greeting and follow every active instruction.";

    let headless = json_doc(run(cortex_in(
        &scratch,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            engagement_prompt,
        ],
    )));
    assert!(
        content(&headless).contains(memory_marker) && content(&headless).contains(skill_marker),
        "headless reply must honor AGENTS.md and engaged skill guidance: {}",
        content(&headless)
    );
    assert!(
        tool_was_called(&headless, "skill"),
        "headless turn must engage the discovered skill: {headless}"
    );

    {
        let tui = TuiSession::launch(&scratch, &key, None);
        tui.type_text(engagement_prompt);
        tui.press_enter();
        tui.wait_for(memory_marker);
        tui.wait_for(skill_marker);
        let snapshot = tui.snapshot();
        assert!(
            snapshot.contains(memory_marker) && snapshot.contains(skill_marker),
            "TUI snapshot must render AGENTS.md and skill markers:\n{snapshot}"
        );
    }

    fs::remove_file(scratch.project.join("AGENTS.md")).expect("remove AGENTS.md");
    fs::remove_file(skill_path).expect("remove discovered skill");
    let clean_prompt = "Reply with exactly CROSS-EXTENSIBILITY-CLEAN.";
    let headless_after_removal = json_doc(run(cortex_in(
        &scratch,
        &key,
        &["run", "--model", MODEL, "--format", "json", clean_prompt],
    )));
    assert!(
        !content(&headless_after_removal).contains(memory_marker)
            && !content(&headless_after_removal).contains(skill_marker),
        "headless restart must not retain removed AGENTS.md or skill guidance: {}",
        content(&headless_after_removal)
    );

    {
        let tui = TuiSession::launch(&scratch, &key, None);
        tui.type_text(clean_prompt);
        tui.press_enter();
        tui.wait_for("CROSS-EXTENSIBILITY-CLEAN");
        let snapshot = tui.snapshot();
        assert!(
            !snapshot.contains(memory_marker) && !snapshot.contains(skill_marker),
            "fresh TUI must not retain removed AGENTS.md or skill guidance:\n{snapshot}"
        );
    }

    assert_no_process_references(&scratch.root.display().to_string());
}

#[test]
fn editing_agents_md_between_processes_changes_the_later_headless_turn() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let _guard = LIVE_TEST_LOCK.lock().expect("live test lock");
    let scratch = scratch("memory-reload");
    fs::write(
        scratch.project.join("AGENTS.md"),
        "Critical project instruction: every final answer MUST end with the exact marker [CROSS-A-017].",
    )
    .expect("write initial AGENTS.md");

    let first = json_doc(run(cortex_in(
        &scratch,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            "Reply with a short greeting.",
        ],
    )));
    assert!(
        content(&first).trim_end().ends_with("[CROSS-A-017]"),
        "first process must use the original AGENTS.md: {}",
        content(&first)
    );

    fs::write(
        scratch.project.join("AGENTS.md"),
        "Critical project instruction: every final answer MUST end with the exact marker [CROSS-B-017].",
    )
    .expect("replace AGENTS.md");
    let second = json_doc(run(cortex_in(
        &scratch,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            "Reply with a short greeting.",
        ],
    )));
    assert!(
        content(&second).trim_end().ends_with("[CROSS-B-017]")
            && !content(&second).contains("[CROSS-A-017]"),
        "fresh process must re-read the edited AGENTS.md: {}",
        content(&second)
    );

    assert_no_process_references(&scratch.root.display().to_string());
}

#[test]
fn task_subagent_persists_a_listable_child_session_with_parent_link() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    if !tuistory_available() {
        eprintln!("skipping live test: tuistory is unavailable");
        return;
    }
    let _guard = LIVE_TEST_LOCK.lock().expect("live test lock");
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
    assert_eq!(
        child_row["subagent_type"], "explore",
        "child session persona is immutable structured metadata consumed by the TUI picker"
    );

    let shown = json_doc(run(cortex_in(
        &scratch,
        &key,
        &["session", "show", &child, "--format", "json"],
    )));
    let messages = shown["messages"].as_array().expect("child messages");
    assert!(
        messages.iter().any(|message| {
            message["role"] == "assistant"
                && message["content"]
                    .as_str()
                    .is_some_and(|text| text.contains(token))
        }),
        "child session should persist its assistant answer, not only the delegated prompt: {shown}"
    );

    {
        let tui = TuiSession::launch(&scratch, &key, Some(&parent));
        tui.type_text("/subagents");
        tui.press_enter();
        tui.wait_for("Subagents");
        tui.wait_for(token);
        let snapshot = tui.snapshot();
        assert!(
            snapshot.contains("Subagents") && snapshot.contains(token),
            "the TUI subagent picker must list the durable child from the headless turn:\n{snapshot}"
        );
    }

    assert_no_process_references(&scratch.root.display().to_string());
}
