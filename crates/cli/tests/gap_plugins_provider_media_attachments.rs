use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(40);

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-plugin-media-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("project");
        let data_dir = root.join("data");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&project).expect("create project");
        fs::create_dir_all(root.join(".config")).expect("create config dir");
        fs::create_dir_all(root.join(".local/share")).expect("create xdg dir");
        Self {
            home: root,
            project,
            data_dir,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.current_dir(&self.project);
        command.env("HOME", &self.home);
        command.env("XDG_CONFIG_HOME", self.home.join(".config"));
        command.env("XDG_DATA_HOME", self.home.join(".local/share"));
        command.env("CORTEX_DATA_DIR", &self.data_dir);
        command
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_AUTH_FILE")
            .env_remove("CORTEX_NO_NET")
            .env_remove("CORTEX_OPENROUTER_BASE_URL")
            .env_remove("CORTEX_MODEL")
            .env_remove("CORTEX_SMALL_MODEL")
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

    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
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
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping plugin media gap test: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

#[test]
fn plugin_provider_observes_media_and_rejects_when_attachment_is_disabled() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("provider-media");
    let marker = sandbox.project.join("hooks.jsonl");
    let plugin = sandbox.project.join("plugin.mjs");
    fs::write(
        &plugin,
        r#"
import { appendFile } from "node:fs/promises";

function hasMedia(value) {
  if (value === null || value === undefined) return false;
  if (Array.isArray(value)) return value.some(hasMedia);
  if (typeof value !== "object") return false;
  const t = typeof value.type === "string" ? value.type.toLowerCase() : "";
  if (t.includes("image") || t.includes("document") || t.includes("file")) return true;
  return Object.values(value).some(hasMedia);
}

export default async function plugin(input, options) {
  return {
    async provider() {
      return {
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [
          { id: "plugin/attach-model", name: "Attach", attachment: true, context: 8192, output: 1024 },
          { id: "plugin/no-attachment-model", name: "No Attach", attachment: false, context: 8192, output: 1024 }
        ],
        async complete(request) {
          await appendFile(options.marker, JSON.stringify({
            model: request.model,
            hasMedia: hasMedia(request),
            messages: request.messages
          }) + "\n");
          return { content: "ok" };
        }
      };
    }
  };
}
"#,
    )
    .expect("write plugin");
    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            r#"
plugin = [[ "{}", {{ marker = "{}" }} ]]
"#,
            plugin.display(),
            marker.display()
        ),
    )
    .expect("write config");
    fs::write(
        sandbox.project.join("sample.png"),
        [
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
            b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0, 0,
            0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05, 0x01,
            0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60,
            0x82,
        ],
    )
    .expect("write sample png");

    let prompt = "Analyze this media reference and reply ok: ![tiny](./sample.png)";

    let mut attach_cmd = sandbox.command();
    attach_cmd.args(["run", "--format", "json", "--model", "plugin/attach-model"]);
    attach_cmd.arg(prompt);
    let attach_out = run(attach_cmd);
    assert_success("attach-model run", &attach_out);
    let attach_json: Value = serde_json::from_slice(&attach_out.stdout).expect("attach run json");
    assert_eq!(attach_json["content"], "ok");

    let entries = fs::read_to_string(&marker)
        .expect("hook log")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("hook line json"))
        .collect::<Vec<_>>();
    let attach_entry = entries
        .iter()
        .find(|entry| entry["model"] == "plugin/attach-model")
        .expect("attach-model hook entry");
    assert_eq!(
        attach_entry["hasMedia"], true,
        "attachment-capable model should observe media in request: {attach_entry}"
    );

    let mut reject_cmd = sandbox.command();
    reject_cmd.args([
        "run",
        "--format",
        "json",
        "--model",
        "plugin/no-attachment-model",
    ]);
    reject_cmd.arg(prompt);
    let reject_out = run(reject_cmd);
    assert!(
        !reject_out.status.success(),
        "attachment-disabled model should fail when media is present\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&reject_out.stdout),
        String::from_utf8_lossy(&reject_out.stderr)
    );
    let stderr = String::from_utf8_lossy(&reject_out.stderr);
    assert!(
        stderr.contains("attachment=false"),
        "expected clear attachment=false diagnostic, got:\n{stderr}"
    );
    assert!(
        stderr.contains("plugin/no-attachment-model"),
        "expected model id in diagnostic, got:\n{stderr}"
    );
}
