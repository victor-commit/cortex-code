use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(40);
const NET_DISABLED_CODE: i32 = 4;

#[derive(Debug, Clone)]
struct ScriptedResponse {
    status_line: &'static str,
    headers: Vec<(String, String)>,
    body: String,
}

impl ScriptedResponse {
    fn event_stream(frames: Vec<String>) -> Self {
        Self {
            status_line: "HTTP/1.1 200 OK",
            headers: vec![
                ("content-type".to_owned(), "text/event-stream".to_owned()),
                ("cache-control".to_owned(), "no-cache".to_owned()),
                ("connection".to_owned(), "close".to_owned()),
            ],
            body: sse_body(&frames),
        }
    }
}

struct ModelFixture {
    base_url: String,
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl ModelFixture {
    fn start(responses: Vec<ScriptedResponse>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let responses = Arc::new(Mutex::new(responses));
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = {
            let responses = Arc::clone(&responses);
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                while Instant::now() < deadline && !stop_thread.load(Ordering::Relaxed) {
                    if responses.lock().expect("responses lock").is_empty() {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = read_http_request(&mut stream);
                            let response = responses.lock().expect("responses lock").remove(0);
                            let _ = write_http_response(&mut stream, &response);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            stop,
            handle,
        }
    }

    fn finish(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

struct SkillFixture {
    base_url: String,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl SkillFixture {
    fn start(routes: BTreeMap<String, String>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_thread = Arc::clone(&hits);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline && !stop_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        hits_thread.fetch_add(1, Ordering::Relaxed);
                        let path = read_request_path(&mut stream);
                        let response = if let Some(body) = routes.get(&path) {
                            ScriptedResponse {
                                status_line: "HTTP/1.1 200 OK",
                                headers: vec![
                                    ("content-type".to_owned(), "application/json".to_owned()),
                                    ("connection".to_owned(), "close".to_owned()),
                                ],
                                body: body.clone(),
                            }
                        } else {
                            ScriptedResponse {
                                status_line: "HTTP/1.1 404 Not Found",
                                headers: vec![("connection".to_owned(), "close".to_owned())],
                                body: "not found".to_owned(),
                            }
                        };
                        let _ = write_http_response(&mut stream, &response);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/catalog/"),
            hits,
            stop,
            handle,
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    fn finish(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

fn bind_loopback_listener() -> TcpListener {
    for port in 3200..=3299 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in mission range 3200-3299");
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let already = buffer.len().saturating_sub(header_end);
    let mut remaining = content_length.saturating_sub(already);
    while remaining > 0 {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        remaining = remaining.saturating_sub(read);
    }
    Ok(())
}

fn read_request_path(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).unwrap_or(0);
        if read == 0 {
            break buffer.len();
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let already = buffer.len().saturating_sub(header_end);
    let mut remaining = content_length.saturating_sub(already);
    while remaining > 0 {
        let read = stream.read(&mut chunk).unwrap_or(0);
        if read == 0 {
            break;
        }
        remaining = remaining.saturating_sub(read);
    }
    let request = String::from_utf8_lossy(&buffer);
    let mut parts = request.lines().next().unwrap_or("").split_whitespace();
    let _method = parts.next();
    parts.next().unwrap_or("/").to_owned()
}

fn write_http_response(stream: &mut TcpStream, response: &ScriptedResponse) -> std::io::Result<()> {
    let mut rendered = String::new();
    rendered.push_str(response.status_line);
    rendered.push_str("\r\n");
    for (name, value) in &response.headers {
        rendered.push_str(name);
        rendered.push_str(": ");
        rendered.push_str(value);
        rendered.push_str("\r\n");
    }
    rendered.push_str(&format!("content-length: {}\r\n\r\n", response.body.len()));
    rendered.push_str(&response.body);
    stream.write_all(rendered.as_bytes())?;
    stream.flush()
}

fn sse_body(frames: &[String]) -> String {
    let mut body = String::new();
    for frame in frames {
        body.push_str("data: ");
        body.push_str(frame);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn text_delta_frame(text: &str) -> String {
    serde_json::json!({
        "id": "fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": { "content": text },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn done_frame(reason: &str) -> String {
    serde_json::json!({
        "id": "fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": reason
        }]
    })
    .to_string()
}

fn tool_call_frame(call_id: &str, name: &str, arguments: serde_json::Value) -> String {
    serde_json::json!({
        "id": "fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string()
                    }
                }]
            },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn scratch_home(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-gap-skills-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

fn write_skill(path: &Path, name: &str, description: &str, guidance: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(
        path,
        format!("---\nname: {name}\ndescription: {description}\n---\n{guidance}\n"),
    )
    .expect("write skill");
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            let mut stdout = Vec::new();
            child
                .stdout
                .take()
                .expect("stdout")
                .read_to_end(&mut stdout)
                .expect("stdout bytes");
            let mut stderr = Vec::new();
            child
                .stderr
                .take()
                .expect("stderr")
                .read_to_end(&mut stderr)
                .expect("stderr bytes");
            return Output {
                status,
                stdout,
                stderr,
            };
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn cortex_run(home: &Path, cwd: &Path, base_url: &str, prompt: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(cwd)
        .arg("run")
        .arg("--format")
        .arg("json")
        .arg(prompt)
        .env("HOME", home)
        .env("OPENROUTER_API_KEY", "test-openrouter-key")
        .env("CORTEX_OPENROUTER_BASE_URL", base_url)
        .env_remove("CORTEX_NO_NET")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    run(command)
}

fn cortex_run_no_net(home: &Path, cwd: &Path, base_url: &str, prompt: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(cwd)
        .arg("run")
        .arg(prompt)
        .env("HOME", home)
        .env("OPENROUTER_API_KEY", "test-openrouter-key")
        .env("CORTEX_OPENROUTER_BASE_URL", base_url)
        .env("CORTEX_NO_NET", "1")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    run(command)
}

fn assert_success_json(out: Output) -> serde_json::Value {
    assert_eq!(
        out.status.code(),
        Some(0),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("valid run JSON")
}

fn skill_names(doc: &serde_json::Value) -> Vec<String> {
    let definition = doc["tool_definitions"]
        .as_array()
        .expect("tool definitions")
        .iter()
        .find(|definition| definition["name"] == "skill")
        .expect("skill tool definition");
    let description = definition["parameters"]["properties"]["name"]["description"]
        .as_str()
        .expect("skill name description");
    let marker = "Available skills:";
    let Some(index) = description.find(marker) else {
        return Vec::new();
    };
    description[index + marker.len()..]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn skill_tool_text(doc: &serde_json::Value) -> String {
    doc["tool_results"]
        .as_array()
        .expect("tool results")
        .iter()
        .filter(|result| result["name"] == "skill")
        .flat_map(|result| {
            result["parts"]
                .as_array()
                .expect("skill parts")
                .iter()
                .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn discovers_recursive_project_and_claude_agents_skills() {
    let home = scratch_home("discover-local");
    let project = home.join("project");
    fs::create_dir_all(&project).expect("create project");
    write_skill(
        &project
            .join("skills")
            .join("level-one")
            .join("level-two")
            .join("recursive")
            .join("SKILL.md"),
        "recursive-skill",
        "Deep recursive skill",
        "Append SKILL-RECURSIVE.",
    );
    write_skill(
        &project
            .join(".claude")
            .join("skills")
            .join("claude")
            .join("SKILL.md"),
        "claude-skill",
        "Claude skill",
        "Append SKILL-CLAUDE.",
    );
    write_skill(
        &project
            .join(".agents")
            .join("skills")
            .join("agent")
            .join("SKILL.md"),
        "agent-skill",
        "Agent skill",
        "Append SKILL-AGENT.",
    );
    write_skill(
        &home
            .join(".claude")
            .join("skills")
            .join("global-claude")
            .join("SKILL.md"),
        "global-claude",
        "Global claude skill",
        "Append SKILL-GLOBAL-CLAUDE.",
    );
    write_skill(
        &home
            .join(".agents")
            .join("skills")
            .join("global-agent")
            .join("SKILL.md"),
        "global-agent",
        "Global agent skill",
        "Append SKILL-GLOBAL-AGENT.",
    );

    let model = ModelFixture::start(vec![ScriptedResponse::event_stream(vec![
        text_delta_frame("ok"),
        done_frame("stop"),
    ])]);
    let doc = assert_success_json(cortex_run(&home, &project, &model.base_url, "Say ok"));
    model.finish();

    let names = skill_names(&doc);
    for expected in [
        "recursive-skill",
        "claude-skill",
        "agent-skill",
        "global-claude",
        "global-agent",
    ] {
        assert!(names.contains(&expected.to_owned()), "missing {expected}");
    }
}

#[test]
fn remote_skills_are_fetched_and_cached_across_runs() {
    let home = scratch_home("remote-cache");
    let project = home.join("project");
    fs::create_dir_all(&project).expect("create project");
    let index = serde_json::json!({
        "skills": [{
            "name": "remote-skill",
            "files": ["SKILL.md", "references/guide.md"]
        }]
    })
    .to_string();
    let routes = BTreeMap::from([
        ("/catalog/index.json".to_owned(), index),
        (
            "/catalog/remote-skill/SKILL.md".to_owned(),
            "---\nname: remote-skill\ndescription: Remote cached skill\n---\nAppend SKILL-REMOTE.\n"
                .to_owned(),
        ),
        (
            "/catalog/remote-skill/references/guide.md".to_owned(),
            "# guide\n".to_owned(),
        ),
    ]);
    let skill_fixture = SkillFixture::start(routes);
    fs::write(
        project.join("cortex.toml"),
        format!(
            r#"
[skills]
urls = ["{}"]
"#,
            skill_fixture.base_url
        ),
    )
    .expect("write config");
    let model = ModelFixture::start(vec![
        ScriptedResponse::event_stream(vec![text_delta_frame("one"), done_frame("stop")]),
        ScriptedResponse::event_stream(vec![text_delta_frame("two"), done_frame("stop")]),
    ]);

    let first = assert_success_json(cortex_run(&home, &project, &model.base_url, "Say one"));
    assert!(
        skill_names(&first).contains(&"remote-skill".to_owned()),
        "remote skill should be available after first fetch"
    );
    assert!(
        skill_fixture.hits() > 0,
        "remote fixture should observe fetch requests"
    );
    skill_fixture.finish();

    let second = assert_success_json(cortex_run(&home, &project, &model.base_url, "Say two"));
    model.finish();
    assert!(
        skill_names(&second).contains(&"remote-skill".to_owned()),
        "remote skill should remain available from cache when source is offline"
    );
}

#[test]
fn remote_skill_fetch_respects_no_net_before_connecting() {
    let home = scratch_home("remote-no-net");
    let project = home.join("project");
    fs::create_dir_all(&project).expect("create project");
    let routes = BTreeMap::from([(
        "/catalog/index.json".to_owned(),
        serde_json::json!({ "skills": [] }).to_string(),
    )]);
    let skill_fixture = SkillFixture::start(routes);
    fs::write(
        project.join("cortex.toml"),
        format!(
            r#"
[skills]
urls = ["{}"]
"#,
            skill_fixture.base_url
        ),
    )
    .expect("write config");
    let model = ModelFixture::start(vec![ScriptedResponse::event_stream(vec![
        text_delta_frame("unused"),
        done_frame("stop"),
    ])]);

    let out = cortex_run_no_net(&home, &project, &model.base_url, "Say blocked");
    model.finish();
    assert_eq!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "no-net run should fail with dedicated exit code"
    );
    assert_eq!(
        skill_fixture.hits(),
        0,
        "remote skill URL must not be contacted while CORTEX_NO_NET=1"
    );
    skill_fixture.finish();
}

#[test]
fn skill_tool_returns_base_dir_file_inventory_and_permission_status() {
    let home = scratch_home("skill-tool-output");
    let project = home.join("project");
    fs::create_dir_all(&project).expect("create project");
    let skill_dir = project.join("skills").join("inventory");
    write_skill(
        &skill_dir.join("SKILL.md"),
        "inventory-skill",
        "Skill with supporting files",
        "Append SKILL-INVENTORY.",
    );
    fs::create_dir_all(skill_dir.join("references")).expect("create refs");
    fs::create_dir_all(skill_dir.join("secrets")).expect("create secrets");
    fs::write(skill_dir.join("references").join("guide.md"), "# guide\n").expect("write guide");
    fs::write(skill_dir.join("secrets").join("private.md"), "TOP SECRET\n").expect("write secret");
    fs::write(
        project.join("cortex.toml"),
        r#"
[[permission]]
tool = "skill"
pattern = "*"
action = "allow"

[[permission]]
tool = "read"
pattern = "*"
action = "allow"

[[permission]]
tool = "read"
pattern = "skills/inventory/secrets/**"
action = "deny"
"#,
    )
    .expect("write permissions");
    let model = ModelFixture::start(vec![
        ScriptedResponse::event_stream(vec![
            tool_call_frame(
                "call_skill",
                "skill",
                serde_json::json!({ "name": "inventory-skill" }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::event_stream(vec![text_delta_frame("done"), done_frame("stop")]),
    ]);

    let doc = assert_success_json(cortex_run(
        &home,
        &project,
        &model.base_url,
        "Use the inventory skill before answering.",
    ));
    model.finish();

    let text = skill_tool_text(&doc);
    assert!(text.contains("Base directory for this skill:"), "{text}");
    assert!(
        text.contains("<file status=\"allowed\">references/guide.md</file>"),
        "{text}"
    );
    assert!(
        text.contains("<file status=\"permission_denied\">secrets/private.md</file>"),
        "{text}"
    );
    assert!(
        !text.contains("TOP SECRET"),
        "skill inventory should never include denied file contents: {text}"
    );
}
