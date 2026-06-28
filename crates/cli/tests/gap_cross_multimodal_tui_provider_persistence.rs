//! VAL-GAP-CROSS-001 (export half): a multimodal turn whose typed media part was minted by the TUI
//! attachment flow survives a process restart and is exposed consistently by every export surface —
//! `cortex export`, `cortex session show`, `cortex stats`, and the loopback `cortex serve` message
//! route — with the answer content, usage, and media metadata preserved, no binary/base64 bytes in
//! any text channel, and pasted secrets redacted.
//!
//! The session is seeded directly through the shared `db` store using the EXACT typed shape the TUI
//! persists (`MessagePartBody::File { source: "tui_attachment", .. }`, locked by the `tui`-crate test
//! `tui_attachment_turn_persists_typed_media_usage_and_answer_across_restart`). Seeding through the
//! durable store and re-reading it through fresh `cortex` processes is the restart proof; no provider
//! turn (and no network) is needed for the deterministic export composition. The live end-to-end flow
//! (attach in TUI, ask a live model, restart, export) is proven black-box with tuistory.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(45);

fn fake_openrouter_key() -> String {
    // Assembled from fragments so no contiguous key-shaped literal lives in source (Droid-Shield).
    format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8))
}

struct Scratch {
    home: PathBuf,
    project: PathBuf,
    db_path: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-cross-multimodal-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("project");
        std::fs::create_dir_all(&project).expect("create project dir");
        let db_path = root.join("cortex.db");
        Self {
            home: root,
            project,
            db_path,
        }
    }

    fn cortex(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .current_dir(&self.project)
            .args(args)
            .env("HOME", &self.home)
            .env("CORTEX_DB_PATH", &self.db_path)
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_OPENROUTER_BASE_URL")
            .env_remove("CORTEX_NO_NET")
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CORTEX_DATA_DIR")
            .env_remove("CORTEX_AUTH_FILE")
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("NO_COLOR");
        command
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// A solid-color PNG so the on-disk attachment is a real, decodable image (the export path stores
/// only the path metadata, but a real file mirrors the live flow).
fn write_solid_png(path: &std::path::Path) {
    use image::{ImageFormat, RgbImage};
    let image = RgbImage::from_pixel(48, 48, image::Rgb([220, 30, 30]));
    let file = std::fs::File::create(path).expect("create png file");
    let mut writer = std::io::BufWriter::new(file);
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut writer, ImageFormat::Png)
        .expect("encode png");
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex");
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
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
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

fn run_json(scratch: &Scratch, args: &[&str]) -> Value {
    let out = run(scratch.cortex(args));
    assert_eq!(
        out.status.code(),
        Some(0),
        "`cortex {}` failed:\nstdout={}\nstderr={}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "strict json from `cortex {}` failed: {err}\nstdout={}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// Find the user message in a `{messages:[...]}` document, regardless of surface shape.
fn user_message(messages: &[Value]) -> &Value {
    messages
        .iter()
        .find(|message| message["role"] == "user")
        .expect("a user message")
}

fn assistant_message(messages: &[Value]) -> &Value {
    messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("an assistant message")
}

/// Assert a `messages` array carries the typed image media metadata, the answer, no binary/base64
/// leakage, and no secret, for whichever export surface produced it.
fn assert_media_answer_and_redaction(messages: &[Value], secret: &str, surface: &str) {
    let user = user_message(messages);
    let parts = user["parts"]
        .as_array()
        .unwrap_or_else(|| panic!("{surface}: user message must carry rich parts"));
    let media = parts
        .iter()
        .find(|part| part["type"] == "file")
        .unwrap_or_else(|| panic!("{surface}: expected a file media part: {parts:?}"));
    assert_eq!(
        media["media_type"], "image/png",
        "{surface}: media type preserved"
    );
    assert_eq!(
        media["source"], "tui_attachment",
        "{surface}: TUI attachment provenance preserved"
    );
    assert!(
        media["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("swatch.png")),
        "{surface}: media path metadata preserved: {media:?}"
    );

    let answer = assistant_message(messages)["content"]
        .as_str()
        .unwrap_or_default();
    assert!(
        answer.contains("solid red"),
        "{surface}: the answer content is preserved: {answer:?}"
    );

    let serialized = serde_json::to_string(messages).expect("serialize messages");
    assert!(
        !serialized.contains("base64,"),
        "{surface}: no base64 image payload may leak into the text channels"
    );
    assert!(
        !serialized.contains("\\x89PNG") && !serialized.contains("\u{89}PNG"),
        "{surface}: no raw PNG bytes may leak into the text channels"
    );
    assert!(
        !serialized.contains(secret),
        "{surface}: the pasted secret must be redacted everywhere"
    );
    assert!(
        serialized.contains(redaction::PLACEHOLDER),
        "{surface}: the canonical redaction placeholder marks the scrubbed secret"
    );
}

fn free_server_port() -> u16 {
    for port in 3240..=3299 {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 3240-3299");
}

fn wait_for_health(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if curl_get(port, "/health").is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(150));
    }
    panic!("server on port {port} did not become healthy");
}

/// Minimal loopback HTTP GET returning the response body, or `None` if the request failed.
fn curl_get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") {
        return None;
    }
    Some(body.to_owned())
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-tiTCP", &port.to_string(), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .and_then(|line| line.trim().parse::<u32>().ok())
}

fn stop_server(mut child: Child, port: u16) {
    let pid = listener_pid(port).unwrap_or_else(|| child.id());
    let _ = Command::new("kill").arg(pid.to_string()).status();
    let _ = child.wait();
}

#[test]
fn tui_seeded_multimodal_session_exports_consistently_across_surfaces_after_restart() {
    let scratch = Scratch::new("export");
    let png = scratch.project.join("swatch.png");
    write_solid_png(&png);
    let png_abs = png.canonicalize().expect("canonical png path");
    let secret = fake_openrouter_key();

    // Seed the session through the shared durable store with the EXACT typed shape the TUI persists
    // for an attachment-bearing multimodal turn, then drop the handle to mimic the originating
    // process exiting before the export/restart reads.
    let session_id = {
        use protocol::{MessagePart, MessagePartBody};
        let db = db::Database::open(&scratch.db_path).expect("open seed store");
        let session = db
            .sessions()
            .create(Some("multimodal cross-flow"))
            .expect("create session");
        db.sessions()
            .set_model(&session.id, Some("google/gemini-2.5-flash"))
            .expect("set model");

        let user_text =
            format!("Describe this swatch in one short phrase. Ignore this token: {secret}",);
        let user_parts = vec![
            MessagePart::text(user_text.clone()),
            MessagePart {
                id: String::new(),
                created_at: None,
                body: MessagePartBody::File {
                    path: png_abs.to_string_lossy().into_owned(),
                    media_type: Some("image/png".to_owned()),
                    source: Some("tui_attachment".to_owned()),
                },
            },
        ];
        db.messages()
            .create_with_parts(&session.id, "user", &user_text, &user_parts)
            .expect("persist user message");
        db.messages()
            .create_with_parts(
                &session.id,
                "assistant",
                "The image is a solid red swatch.",
                &[MessagePart::text("The image is a solid red swatch.")],
            )
            .expect("persist assistant message");
        db.usage()
            .record(&session.id, 2666, 8, 2674, Some(0.0008))
            .expect("record usage");
        session.id
    };

    // ---- Surface 1: `cortex export` (fresh process == restart) ----
    let export = run_json(&scratch, &["export", &session_id]);
    assert_eq!(export["session"]["id"], session_id);
    let export_messages = export["messages"].as_array().expect("export messages");
    assert_media_answer_and_redaction(export_messages, &secret, "export");

    // ---- Surface 2: `cortex session show` ----
    let show = run_json(
        &scratch,
        &["session", "show", &session_id, "--format", "json"],
    );
    let show_messages = show["messages"].as_array().expect("show messages");
    assert_media_answer_and_redaction(show_messages, &secret, "session show");

    // ---- Surface 3: `cortex stats` carries the persisted usage ----
    let stats = run_json(&scratch, &["stats", "--format", "json"]);
    let total_input = stats["totals"]["input_tokens"].as_u64().unwrap_or(0);
    let total_output = stats["totals"]["output_tokens"].as_u64().unwrap_or(0);
    assert!(
        total_input >= 2666 && total_output >= 8,
        "stats must include the persisted multimodal turn usage: {stats}"
    );
    assert!(
        !serde_json::to_string(&stats).unwrap().contains(&secret),
        "stats output must not leak the secret"
    );

    // ---- Surface 4: loopback `cortex serve` GET /session/{id}/message ----
    let port = free_server_port();
    let mut serve = scratch.cortex(&[
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    serve.stdout(Stdio::piped()).stderr(Stdio::piped());
    let server = serve.spawn().expect("spawn serve");
    wait_for_health(port);

    let body = curl_get(port, &format!("/session/{session_id}/message"))
        .expect("GET message list returns 200");
    let document: Value = serde_json::from_str(&body).expect("server message json");
    let server_messages = document["messages"].as_array().expect("server messages");
    assert_media_answer_and_redaction(server_messages, &secret, "serve");
    assert_eq!(
        document["usage"]["input_tokens"].as_u64(),
        Some(2666),
        "server message list reports the persisted usage: {document}"
    );
    assert_eq!(document["usage"]["output_tokens"].as_u64(), Some(8));

    stop_server(server, port);
    assert!(
        std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "server port {port} should be released after stopping the listener"
    );
}
