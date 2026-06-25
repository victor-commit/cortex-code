//! Black-box coverage for gap-config-schema-sources-substitution
//! (VAL-GAP-CONFIG-002/003/004/005/006/007/010).

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(12);

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-config-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("home/proj/sub")).expect("create scratch tree");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn nested(&self) -> PathBuf {
        self.root.join("home/proj/sub")
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(path, contents).expect("write file");
    }

    fn write_project(&self, contents: &str) {
        self.write("home/proj/cortex.toml", contents);
    }

    fn raw(&self, env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.env("HOME", self.home());
        for (key, value) in env {
            command.env(key, value);
        }
        command.current_dir(self.nested());
        command.args(["config", "--format", "json"]);
        run(command)
    }

    fn config_json(&self, env: &[(&str, &str)]) -> Value {
        let output = self.raw(env);
        assert_eq!(
            output.status.code(),
            Some(0),
            "config command must succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("stdout is strict JSON")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
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
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

struct HttpFixture {
    url: String,
    hits: Arc<AtomicUsize>,
    handle: std::thread::JoinHandle<()>,
}

impl HttpFixture {
    fn start(body: &str) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set fixture nonblocking");
        let port = listener.local_addr().expect("fixture addr").port();
        let body = body.to_owned();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_thread = Arc::clone(&hits);
        let handle = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        hits_thread.fetch_add(1, Ordering::Relaxed);
                        drain_http_request(&mut stream);
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}/.well-known/cortex.toml"),
            hits,
            handle,
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    fn join(self) {
        let _ = self.handle.join();
    }
}

fn bind_loopback_listener() -> TcpListener {
    for port in 3200..=3299 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in 3200-3299")
}

fn drain_http_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("set timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    while bytes.windows(4).all(|window| window != b"\r\n\r\n") {
        let read = stream.read(&mut chunk).unwrap_or(0);
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
}

#[test]
fn json_and_jsonc_configs_are_rejected_with_migration_guidance() {
    let scratch = Scratch::new("json-jsonc");
    scratch.write(
        "home/proj/cortex.jsonc",
        "{\n  // comment\n  \"model\": \"x\"\n}\n",
    );
    let output = scratch.raw(&[]);
    assert_eq!(output.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported config file format"),
        "{stderr}"
    );
    assert!(stderr.contains("TOML"), "{stderr}");
    assert!(stderr.contains("cortex.jsonc"), "{stderr}");
}

#[test]
fn embedded_substitutions_are_resolved_relative_to_the_config_file_and_tilde_expands() {
    let scratch = Scratch::new("embedded-subst");
    scratch.write("home/proj/segment.txt", "SEGMENT");
    scratch.write_project(
        r#"
message = "prefix-{env:EMBED_VAR}-{file:./segment.txt}-suffix"
cache_path = "~/cache/data.txt"
"#,
    );
    let report = scratch.config_json(&[("EMBED_VAR", "VALUE")]);
    assert_eq!(report["message"], "prefix-VALUE-SEGMENT-suffix");
    assert_eq!(
        report["cache_path"],
        Value::String(
            scratch
                .home()
                .join("cache/data.txt")
                .to_string_lossy()
                .to_string()
        )
    );
}

#[test]
fn inline_config_content_overrides_file_layer_and_invalid_content_errors() {
    let scratch = Scratch::new("inline-content");
    scratch.write_project(r#"model = "from-file""#);
    let report = scratch.config_json(&[("CORTEX_CONFIG_CONTENT", r#"model = "from-inline""#)]);
    assert_eq!(report["model"], "from-inline");

    let output = scratch.raw(&[("CORTEX_CONFIG_CONTENT", "model = ")]);
    assert_eq!(output.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("CORTEX_CONFIG_CONTENT"), "{stderr}");
}

#[test]
fn remote_well_known_config_is_fetched_and_cached() {
    let scratch = Scratch::new("remote-cache");
    let fixture = HttpFixture::start(r#"model = "from-remote""#);
    scratch.write_project(&format!(
        r#"
[config_sources]
remote_well_known_url = "{}"
"#,
        fixture.url
    ));

    let first = scratch.config_json(&[]);
    assert_eq!(first["model"], "from-remote");
    fixture.join();

    // The fixture is gone, so this run can only succeed if the fetched remote config was cached.
    let second = scratch.config_json(&[]);
    assert_eq!(second["model"], "from-remote");
}

#[test]
fn remote_well_known_is_no_net_safe_and_does_not_connect() {
    let scratch = Scratch::new("remote-no-net");
    let fixture = HttpFixture::start(r#"model = "from-remote""#);
    scratch.write_project(&format!(
        r#"
[config_sources]
remote_well_known_url = "{}"
"#,
        fixture.url
    ));

    let output = scratch.raw(&[("CORTEX_NO_NET", "1")]);
    assert_eq!(output.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("network is disabled"), "{stderr}");
    assert!(stderr.contains("remote well-known config"), "{stderr}");

    // No request reached the fixture, proving the kill-switch blocked before outbound connect.
    assert_eq!(fixture.hits(), 0);
    fixture.join();
}

#[test]
fn managed_config_locks_keys_and_local_parse_errors_remain_visible() {
    let scratch = Scratch::new("managed-locks");
    let managed_path = scratch.root.join("managed.toml");
    fs::write(&managed_path, r#"model = "managed-model""#).expect("write managed file");
    scratch.write_project(&format!(
        r#"
model = "project-model"
[config_sources]
managed_file = "{}"
managed_locked_keys = ["model"]
"#,
        managed_path.display()
    ));

    let report = scratch.config_json(&[]);
    assert_eq!(report["model"], "managed-model");

    scratch.write_project("model = \n");
    let output = scratch.raw(&[]);
    assert_eq!(output.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cortex.toml"), "{stderr}");
}

#[test]
fn typed_validation_covers_provider_formatter_lsp_mcp_agent_and_permission_sections() {
    let cases = [
        ("provider = \"bad\"\n", "provider"),
        ("formatter = 12\n", "formatter"),
        ("lsp = 12\n", "lsp"),
        ("[mcp]\ndemo = \"oops\"\n", "mcp.demo"),
        ("permission = \"deny\"\n", "permission"),
        (
            "[agent.plan]\npermission = \"deny\"\n",
            "agent.plan.permission",
        ),
    ];

    for (body, key) in cases {
        let scratch = Scratch::new("typed-validation");
        scratch.write_project(body);
        let output = scratch.raw(&[]);
        assert_eq!(
            output.status.code(),
            Some(5),
            "invalid {key} fixture should fail"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(key),
            "stderr should name `{key}`, got: {stderr}"
        );
    }
}
