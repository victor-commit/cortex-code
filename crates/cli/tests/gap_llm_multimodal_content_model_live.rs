use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(70);
const MAX_ATTEMPTS: usize = 3;
const LIVE_RETRY_BACKOFF: Duration = Duration::from_millis(400);
static LIVE_TEST_LOCK: Mutex<()> = Mutex::new(());
const PNG_BYTES: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0xd, 0xa, 0x1a, 0xa, 0x0, 0x0, 0x0, 0xd, 0x49, 0x48, 0x44, 0x52, 0x0,
    0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x1, 0x8, 0x4, 0x0, 0x0, 0x0, 0xb5, 0x1c, 0xc, 0x2, 0x0, 0x0,
    0x0, 0xb, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc, 0xff, 0x1f, 0x0, 0x3, 0x3, 0x2, 0x0,
    0xee, 0xd9, 0xe2, 0xa4, 0x0, 0x0, 0x0, 0x0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn live_key_and_lock() -> Option<(String, std::sync::MutexGuard<'static, ()>)> {
    let key = live_key()?;
    let guard = match LIVE_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    Some((key, guard))
}

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-llm-multimodal-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("proj");
        fs::create_dir_all(&project).expect("create project dir");
        fs::create_dir_all(root.join(".config")).expect("create config dir");
        fs::create_dir_all(root.join(".local/share")).expect("create data dir");
        Self {
            home: root,
            project,
        }
    }

    fn cortex(&self, key: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.current_dir(&self.project);
        command.env("OPENROUTER_API_KEY", key);
        command.env("HOME", &self.home);
        command.env("XDG_CONFIG_HOME", self.home.join(".config"));
        command.env("XDG_DATA_HOME", self.home.join(".local/share"));
        command
            .env_remove("CORTEX_DATA_DIR")
            .env_remove("CORTEX_DB_PATH")
            .env_remove("CORTEX_NO_NET")
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("NO_COLOR");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn command");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    let out_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn png_fixture(path: &Path) {
    fs::write(path, PNG_BYTES).expect("write png fixture");
}

fn output_debug(output: &Output) -> String {
    format!(
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn multimodal_user_input_reaches_model_and_persists_attachment_metadata_live() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping live multimodal test: OPENROUTER_API_KEY not set");
        return;
    };

    let sandbox = Sandbox::new("user-media");
    let image_path = sandbox.project.join("vision.png");
    png_fixture(&image_path);

    let mut success: Option<Value> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        let prompt = "Describe this image in one short sentence: ![vision-image](./vision.png)";
        let mut command = sandbox.cortex(&key);
        command.args(["run", "--format", "json", "--model", MODEL]);
        command.arg(prompt);
        let output = run(command);
        if output.status.success()
            && let Ok(json) = serde_json::from_slice::<Value>(&output.stdout)
        {
            success = Some(json);
            break;
        }
        if attempt < MAX_ATTEMPTS {
            thread::sleep(LIVE_RETRY_BACKOFF);
        } else {
            panic!(
                "run did not complete with multimodal input after {MAX_ATTEMPTS} attempts\n{}",
                output_debug(&output)
            );
        }
    }
    let run_json = success.expect("captured successful run json");
    let session_id = run_json["session"]
        .as_str()
        .expect("run json session id")
        .to_owned();

    let mut show_cmd = sandbox.cortex(&key);
    show_cmd.args(["session", "show", &session_id, "--format", "json"]);
    let show_output = run(show_cmd);
    assert!(
        show_output.status.success(),
        "session show failed\n{}",
        output_debug(&show_output)
    );
    let detail: Value = serde_json::from_slice(&show_output.stdout).expect("session show json");
    let user_message = detail["messages"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .find(|message| message["role"].as_str() == Some("user"))
        })
        .expect("user message");
    let parts = user_message["parts"]
        .as_array()
        .expect("user message parts");
    assert!(
        parts.iter().any(|part| {
            part["type"] == "file"
                && part["media_type"] == "image/png"
                && part["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("vision.png"))
        }),
        "expected persisted file media metadata in user message parts: {parts:?}"
    );
    assert!(
        !user_message["content"]
            .as_str()
            .unwrap_or_default()
            .contains("base64,"),
        "user text content must not contain binary/base64 payloads"
    );
}

#[test]
fn tool_result_media_is_consumed_in_follow_up_model_step_live() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping live multimodal tool-result test: OPENROUTER_API_KEY not set");
        return;
    };

    let sandbox = Sandbox::new("tool-media");
    let image_path = sandbox.project.join("tool.png");
    png_fixture(&image_path);

    for attempt in 1..=MAX_ATTEMPTS {
        let prompt = "Use the read tool on ./tool.png, then answer in one short sentence what image you saw.";
        let mut command = sandbox.cortex(&key);
        command.args(["run", "--format", "json", "--model", MODEL]);
        command.arg(prompt);
        let output = run(command);
        if output.status.success()
            && let Ok(json) = serde_json::from_slice::<Value>(&output.stdout)
        {
            let content_matches = json["content"]
                .as_str()
                .is_some_and(|content| !content.trim().is_empty());
            let saw_media_tool_part = json["tool_results"].as_array().is_some_and(|results| {
                results.iter().any(|result| {
                    result["parts"].as_array().is_some_and(|parts| {
                        parts.iter().any(|part| {
                            matches!(part["type"].as_str(), Some("image") | Some("document"))
                        })
                    })
                })
            });
            if content_matches && saw_media_tool_part {
                return;
            }
        }

        if attempt < MAX_ATTEMPTS {
            thread::sleep(LIVE_RETRY_BACKOFF);
        } else {
            panic!(
                "run did not prove tool-result media flow after {MAX_ATTEMPTS} attempts\n{}",
                output_debug(&output)
            );
        }
    }
}
