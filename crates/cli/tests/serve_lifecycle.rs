use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static SERVE_TEST_LOCK: Mutex<()> = Mutex::new(());
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);
// A cold /lsp probe (and the stale-while-revalidate background refresh it spawns) can run for tens
// of seconds while it enumerates language-server availability, so teardown must wait long enough for
// a SIGTERM'd server to drain an in-flight refresh before it is force-killed. This is a max-wait
// bound only: a healthy server with no in-flight probe exits and releases its port immediately.
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn serve_health_loopback_bind_and_shutdown_release_are_black_box_visible() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let port = free_port();
    let mut server = ServerProcess::start(port);

    let response = wait_for_health(port, &mut server.child);
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&response.body).expect("health JSON"),
        serde_json::json!({"status": "ok"})
    );

    let pids = listener_pids(port);
    assert_eq!(pids, vec![server.child.id()]);
    let details = listener_details(port);
    assert!(
        details.contains(&format!("127.0.0.1:{port}")),
        "listener details should show canonical loopback address, got:\n{details}"
    );
    assert!(
        !details.contains(&format!("0.0.0.0:{port}")) && !details.contains(&format!("*:{port}")),
        "listener must not bind a wildcard address:\n{details}"
    );

    server.stop();
    wait_for_port_released(port);

    let mut relaunched = ServerProcess::start(port);
    let relaunched_response = wait_for_health(port, &mut relaunched.child);
    assert_eq!(relaunched_response.status, 200);
    relaunched.stop();
    wait_for_port_released(port);
}

#[test]
fn serve_session_crud_routes_are_black_box_visible_and_db_backed() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let port = free_port();
    let scratch = Scratch::new("serve-session-crud");
    let db_path = scratch.db_path();
    let mut server = ServerProcess::start_with_db_path(port, &db_path);

    let created_response = request_json(port, "POST", "/session", Some("{}"));
    assert_eq!(created_response.status, 201);
    assert_json_content_type(&created_response);
    let created: db::Session =
        serde_json::from_str(&created_response.body).expect("created session JSON");
    assert!(
        !created.id.is_empty(),
        "created session id must be non-empty"
    );

    let list_response = request_json(port, "GET", "/session", None);
    assert_eq!(list_response.status, 200);
    assert_json_content_type(&list_response);
    let listed: Vec<db::Session> =
        serde_json::from_str(&list_response.body).expect("session list JSON");
    assert!(
        listed.iter().any(|session| session.id == created.id),
        "freshly created session should be listed: {listed:?}"
    );

    let get_response = request_json(port, "GET", &format!("/session/{}", created.id), None);
    assert_eq!(get_response.status, 200);
    assert_json_content_type(&get_response);
    let fetched: db::Session =
        serde_json::from_str(&get_response.body).expect("fetched session JSON");
    assert_eq!(fetched.id, created.id);

    let delete_response = request_json(port, "DELETE", &format!("/session/{}", created.id), None);
    assert_eq!(delete_response.status, 204);

    let deleted_get = request_json(port, "GET", &format!("/session/{}", created.id), None);
    assert_eq!(deleted_get.status, 404);
    assert_json_content_type(&deleted_get);

    let list_after_delete = request_json(port, "GET", "/session", None);
    assert_eq!(list_after_delete.status, 200);
    let listed_after_delete: Vec<db::Session> =
        serde_json::from_str(&list_after_delete.body).expect("post-delete session list JSON");
    assert!(
        listed_after_delete
            .iter()
            .all(|session| session.id != created.id),
        "deleted session should be absent from list: {listed_after_delete:?}"
    );

    let persisted_response = request_json(port, "POST", "/session", Some("{}"));
    assert_eq!(persisted_response.status, 201);
    let persisted: db::Session =
        serde_json::from_str(&persisted_response.body).expect("persisted session JSON");
    assert_ne!(persisted.id, created.id);

    server.stop();
    wait_for_port_released(port);

    let mut restarted = ServerProcess::start_with_db_path(port, &db_path);
    let restarted_list_response = request_json(port, "GET", "/session", None);
    assert_eq!(restarted_list_response.status, 200);
    let restarted_list: Vec<db::Session> =
        serde_json::from_str(&restarted_list_response.body).expect("restarted list JSON");
    assert!(
        restarted_list
            .iter()
            .any(|session| session.id == persisted.id),
        "session created before restart should be persisted and listed after restart: {restarted_list:?}"
    );
    let cleanup_delete = request_json(port, "DELETE", &format!("/session/{}", persisted.id), None);
    assert_eq!(cleanup_delete.status, 204);
    restarted.stop();
    wait_for_port_released(port);

    let db = db::Database::open(&db_path).expect("open server db after shutdown");
    assert!(
        db.sessions()
            .get(&created.id)
            .expect("query deleted session")
            .is_none(),
        "deleted session should be absent from persisted db"
    );
    assert!(
        db.sessions()
            .get(&persisted.id)
            .expect("query cleanup session")
            .is_none(),
        "cleanup-deleted restarted session should be absent from persisted db"
    );
}

#[test]
fn serve_public_token_auth_and_preflight_are_black_box_visible() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let port = free_port();
    let scratch = Scratch::new("serve-auth-command");
    let token = format!(
        "server-token-blackbox-{}-{}",
        std::process::id(),
        SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let db_path = scratch.db_path();
    let mut server = ServerProcess::start_with_db_path_and_args(
        port,
        &db_path,
        &["--public", "--token", &token],
    );

    let missing = request_json(port, "GET", "/session", None);
    assert_eq!(missing.status, 401);
    assert_json_content_type(&missing);

    let wrong = request_json_with_headers(
        port,
        "GET",
        "/session",
        None,
        &[("Authorization", "Bearer wrong-token")],
    );
    assert_eq!(wrong.status, 401);
    assert_json_content_type(&wrong);

    let authorization = format!("Bearer {token}");
    let accepted = request_json_with_headers(
        port,
        "GET",
        "/session",
        None,
        &[("Authorization", authorization.as_str())],
    );
    assert_eq!(accepted.status, 200);
    assert_json_content_type(&accepted);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&accepted.body).expect("session list JSON"),
        serde_json::json!([])
    );

    let preflight = request_json_with_headers(
        port,
        "OPTIONS",
        "/session",
        None,
        &[
            ("Origin", "http://localhost:9999"),
            ("Access-Control-Request-Method", "POST"),
        ],
    );
    assert_eq!(preflight.status, 204);
    let preflight_headers = preflight.headers.to_ascii_lowercase();
    assert!(
        preflight_headers.contains("access-control-allow-origin: http://localhost:9999"),
        "preflight should echo the trusted loopback browser origin:\n{}",
        preflight.headers
    );
    assert!(
        preflight_headers.contains("access-control-allow-methods:")
            && preflight_headers.contains("post"),
        "preflight should advertise POST as allowed:\n{}",
        preflight.headers
    );
    assert!(
        preflight_headers.contains("access-control-allow-headers:")
            && preflight_headers.contains("authorization"),
        "preflight should advertise Authorization as allowed:\n{}",
        preflight.headers
    );

    let health = request_json(port, "GET", "/health", None);
    assert_eq!(health.status, 200);
    let openapi = request_json(port, "GET", "/openapi.json", None);
    assert_eq!(openapi.status, 200);
    assert!(
        openapi.body.contains("Authorization: Bearer <TOKEN>"),
        "OpenAPI should document the bearer scheme without embedding the configured token"
    );

    let response_capture = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        missing.headers,
        missing.body,
        wrong.headers,
        wrong.body,
        accepted.headers,
        accepted.body,
        preflight.headers,
        preflight.body,
        openapi.headers,
        openapi.body
    );
    let (stdout, stderr) = server.captured_output();
    wait_for_port_released(port);
    let captured = format!("{response_capture}\n{stdout}\n{stderr}");
    assert!(
        !captured.contains(&token),
        "configured server token leaked into response data or logs:\n{captured}"
    );
}

#[test]
fn serve_busy_port_exits_nonzero_with_clear_diagnostic() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let port = free_port();
    let mut server = ServerProcess::start(port);
    assert_eq!(wait_for_health(port, &mut server.child).status, 200);
    let scratch = Scratch::new("serve-busy-command");

    let mut command = cortex_command(&[
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    command.env("CORTEX_DB_PATH", scratch.db_path());
    let output = run_to_output(command, WAIT_TIMEOUT);

    assert!(
        !output.status.success(),
        "busy serve unexpectedly succeeded"
    );
    assert!(
        output.stdout.is_empty(),
        "busy-port failure should not write stdout, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already in use"), "{stderr}");
    assert!(stderr.contains(&port.to_string()), "{stderr}");
    assert!(!stderr.contains("panicked at"), "{stderr}");
    assert!(!stderr.contains("RUST_BACKTRACE"), "{stderr}");
    assert_eq!(wait_for_health(port, &mut server.child).status, 200);

    server.stop();
    wait_for_port_released(port);
}

#[test]
fn serve_catalog_discovery_fails_soft_offline_in_node_workspace() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let scratch = Scratch::new("serve-offline-catalog");

    // A workspace that looks like a real node package: package.json + node_modules. This is the
    // shape that surfaced the original HTTP 500 (e.g. running `cortex serve` from the sdk/ dir).
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    std::fs::write(
        workspace.join("package.json"),
        r#"{"name":"offline-catalog-fixture","version":"1.0.0","private":true,"dependencies":{"left-pad":"^1.3.0"}}"#,
    )
    .expect("write package.json");
    let node_module = workspace.join("node_modules").join("left-pad");
    std::fs::create_dir_all(&node_module).expect("create node_modules entry");
    std::fs::write(
        node_module.join("package.json"),
        r#"{"name":"left-pad","version":"1.3.0","main":"index.js"}"#,
    )
    .expect("write node_modules package.json");
    std::fs::write(node_module.join("index.js"), "module.exports = () => {};\n")
        .expect("write node_modules index.js");
    // A project config declaring an unresolvable package plugin. Under CORTEX_NO_NET this cannot be
    // fetched/installed, so plugin/workspace-adapter discovery used to fail hard and 500 the
    // catalog routes; with the fail-soft fix the plugin is skipped and discovery returns 200.
    std::fs::write(
        workspace.join("cortex.toml"),
        "plugin = [\"@cortex-regression/offline-missing-pkg\"]\n",
    )
    .expect("write cortex.toml");

    let data_dir = scratch.dir.join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let db_path = scratch.db_path();

    let port = free_port();
    let mut server =
        ServerProcess::start_offline_in_workspace(port, &workspace, &db_path, &data_dir);

    // Every catalog/discovery GET endpoint that resolves plugins must be 200 (not 500), even
    // offline in a node workspace with an unresolvable plugin.
    let discovery_routes = [
        "/agent",
        "/model",
        "/config/providers",
        "/provider",
        "/skill",
        "/command",
        "/project/discovery",
        "/experimental/workspace/adapter",
    ];
    for route in discovery_routes {
        let response = request_json(port, "GET", route, None);
        assert_eq!(
            response.status, 200,
            "offline catalog/discovery route {route} should fail soft with 200, got {} body: {}",
            response.status, response.body
        );
    }

    // The resolvable subset must still be present: built-in agents are returned despite the
    // unresolvable plugin being skipped.
    let agents = request_json(port, "GET", "/agent", None);
    let agents_json: serde_json::Value =
        serde_json::from_str(&agents.body).expect("agent catalog JSON");
    let agent_array = agents_json.as_array().expect("agent catalog array");
    assert!(
        agent_array
            .iter()
            .any(|agent| agent["name"] == "build" && agent["source"] == "builtin"),
        "built-in agents should still surface from offline discovery: {agent_array:?}"
    );

    server.stop();
    wait_for_port_released(port);
}

#[test]
fn serve_summarize_and_compact_audit_the_no_net_refusal() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let scratch = Scratch::new("serve-no-net-audit");
    let workspace = scratch.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    let data_dir = scratch.dir.join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    let db_path = scratch.db_path();
    let audit_log = scratch.dir.join("audit.jsonl");
    let port = free_port();
    let mut server =
        ServerProcess::start_no_net_with_audit(port, &workspace, &db_path, &data_dir, &audit_log);

    let created = request_json(port, "POST", "/session", Some("{}"));
    assert_eq!(created.status, 201);
    let session: serde_json::Value =
        serde_json::from_str(&created.body).expect("created session JSON");
    let id = session["id"].as_str().expect("session id").to_owned();

    // Both auxiliary turns must be refused by the kill-switch (503) before any socket is opened,
    // exactly like a normal prompt turn.
    let summarize = request_json(
        port,
        "POST",
        &format!("/session/{id}/summarize"),
        Some("{}"),
    );
    assert_eq!(
        summarize.status, 503,
        "summarize must fail closed under CORTEX_NO_NET: {}",
        summarize.body
    );
    let compact = request_json(port, "POST", &format!("/session/{id}/compact"), Some("{}"));
    assert_eq!(
        compact.status, 503,
        "compact must fail closed under CORTEX_NO_NET: {}",
        compact.body
    );

    server.stop();
    wait_for_port_released(port);

    // Each refusal left an auditable blocked-network attempt (a `network_call` record attributed to
    // the llm actor with the net-disabled outcome), so an auxiliary turn is not a hole in the
    // no-net audit trail.
    let raw = std::fs::read_to_string(&audit_log).expect("read audit log");
    let records: Vec<serde_json::Value> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("audit JSONL record"))
        .collect();
    let blocked = records
        .iter()
        .filter(|record| {
            record["type"] == "network_call"
                && record["actor"] == "llm"
                && record["outcome"] == "net-disabled"
        })
        .count();
    assert!(
        blocked >= 2,
        "summarize and compact should each audit a no-net refusal, got {blocked} in: {records:?}"
    );
}

#[test]
fn serve_lsp_status_stays_prompt_under_tight_sequential_loop() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let scratch = Scratch::new("serve-lsp-loop");
    let db_path = scratch.db_path();
    let port = free_port();
    let mut server = ServerProcess::start_with_db_path(port, &db_path);

    // Warm the cache once with a tolerant read timeout (the first probe is heavy).
    let warm = request_http_with_timeout(port, "GET", "/lsp", None, Duration::from_secs(30))
        .expect("warm /lsp request");
    assert_eq!(
        warm.status, 200,
        "first /lsp probe should be 200: {}",
        warm.body
    );

    // A tight sequential loop must stay prompt and never surface a transport error: each cached
    // call comfortably completes within the standard 2s client read timeout.
    let loop_start = Instant::now();
    for iteration in 0..12 {
        let response = request_json(port, "GET", "/lsp", None);
        assert_eq!(
            response.status, 200,
            "cached /lsp call {iteration} should be 200, got {} body: {}",
            response.status, response.body
        );
    }
    let elapsed = loop_start.elapsed();
    assert!(
        elapsed < Duration::from_secs(8),
        "12 cached /lsp calls should be prompt, took {elapsed:?}"
    );

    server.stop();
    wait_for_port_released(port);
}

#[test]
fn serve_lsp_status_concurrent_cold_start_is_single_flight() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let scratch = Scratch::new("serve-lsp-coldstart");
    let db_path = scratch.db_path();
    let port = free_port();
    let mut server = ServerProcess::start_with_db_path(port, &db_path);

    // Fire a burst of concurrent first-ever /lsp requests at the cold cache. With single-flight the
    // first caller runs the one heavy probe and the rest await its result, so every caller observes
    // the same rows and the burst collapses into roughly one probe window. Without single-flight a
    // broken serialize-then-recompute path would run the probe per caller and blow past the bound
    // (or trip each request's read timeout).
    let burst = 8;
    let loop_start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..burst {
        handles.push(thread::spawn(move || {
            request_http_with_timeout(port, "GET", "/lsp", None, Duration::from_secs(30))
                .map(|response| (response.status, response.body))
        }));
    }
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.join().expect("join concurrent /lsp thread"));
    }
    let elapsed = loop_start.elapsed();

    let (first_status, first_body) = results[0]
        .as_ref()
        .expect("first concurrent /lsp result")
        .clone();
    assert_eq!(
        first_status, 200,
        "cold /lsp should be 200, got {first_status} body: {first_body}"
    );
    for (index, result) in results.iter().enumerate() {
        let (status, body) = result
            .as_ref()
            .unwrap_or_else(|err| panic!("concurrent cold /lsp request {index} failed: {err}"));
        assert_eq!(
            *status, 200,
            "concurrent cold /lsp request {index} should be 200, got {status}"
        );
        assert_eq!(
            *body, first_body,
            "all concurrent cold callers must observe the single-flight result (request {index})"
        );
    }
    assert!(
        elapsed < Duration::from_secs(60),
        "concurrent cold-start should collapse into one probe window, took {elapsed:?}"
    );

    server.stop();
    wait_for_port_released(port);
}

#[test]
fn serve_lsp_status_revalidation_recovers_across_stale_windows() {
    let _guard = SERVE_TEST_LOCK.lock().expect("serve test lock");
    let scratch = Scratch::new("serve-lsp-revalidate");
    let db_path = scratch.db_path();
    let port = free_port();
    let mut server = ServerProcess::start_with_db_path(port, &db_path);

    // Warm the cache once with a tolerant read timeout (the first probe is heavy).
    let warm = request_http_with_timeout(port, "GET", "/lsp", None, Duration::from_secs(30))
        .expect("warm /lsp request");
    assert_eq!(
        warm.status, 200,
        "first /lsp probe should be 200: {}",
        warm.body
    );

    // Across several TTL windows the stale-while-revalidate refresh must keep recovering: each
    // window lets the 5s TTL lapse, then a request serves the stale rows immediately and spawns a
    // background refresh. The background refresh holds the single refresh slot via an RAII guard
    // that releases it on completion (or on panic/early-exit), so the slot is never latched; every
    // window staying a prompt 200 proves revalidation keeps proceeding and the stale path is never
    // serialized behind the heavy probe. (The panic-recovery path itself is proven deterministically
    // by the server-crate unit test `refresh_slot_recovers_when_holding_task_panics`.)
    for window in 0..3 {
        thread::sleep(Duration::from_secs(6));
        let stale = request_json(port, "GET", "/lsp", None);
        assert_eq!(
            stale.status, 200,
            "stale-window {window} /lsp should be a prompt 200, got {} body: {}",
            stale.status, stale.body
        );
        // Let the background refresh make progress, then the next read must still be a prompt 200.
        thread::sleep(Duration::from_millis(500));
        let revalidated = request_json(port, "GET", "/lsp", None);
        assert_eq!(
            revalidated.status, 200,
            "post-refresh window {window} /lsp should be a prompt 200, got {} body: {}",
            revalidated.status, revalidated.body
        );
    }

    server.stop();
    wait_for_port_released(port);
}

struct ServerProcess {
    child: Child,
    port: u16,
    stopped: bool,
    _scratch: Option<Scratch>,
}

impl ServerProcess {
    fn start(port: u16) -> Self {
        let scratch = Scratch::new("serve-process");
        let db_path = scratch.db_path();
        Self::start_with_db_path_and_scratch(port, &db_path, Some(scratch))
    }

    fn start_with_db_path(port: u16, db_path: &Path) -> Self {
        Self::start_with_db_path_and_scratch(port, db_path, None)
    }

    fn start_with_db_path_and_args(port: u16, db_path: &Path, extra_args: &[&str]) -> Self {
        Self::start_with_db_path_and_scratch_and_args(port, db_path, None, extra_args)
    }

    fn start_with_db_path_and_scratch(port: u16, db_path: &Path, scratch: Option<Scratch>) -> Self {
        Self::start_with_db_path_and_scratch_and_args(port, db_path, scratch, &[])
    }

    fn start_with_db_path_and_scratch_and_args(
        port: u16,
        db_path: &Path,
        scratch: Option<Scratch>,
        extra_args: &[&str],
    ) -> Self {
        let port_arg = port.to_string();
        let mut args = vec![
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            port_arg.as_str(),
        ];
        args.extend_from_slice(extra_args);
        let mut command = cortex_command(&args);
        command.env("CORTEX_DB_PATH", db_path);
        let mut child = command.spawn().expect("spawn cortex serve");
        let _ = wait_for_health(port, &mut child);
        Self {
            child,
            port,
            stopped: false,
            _scratch: scratch,
        }
    }

    /// Start `cortex serve` rooted at `workspace` under `CORTEX_NO_NET=1`, with isolated db/data
    /// dirs. Used by the offline catalog/discovery regression test: discovery routes must fail soft
    /// (200 with the resolvable subset) even when the workspace declares an unresolvable plugin and
    /// the network kill-switch is engaged.
    fn start_offline_in_workspace(
        port: u16,
        workspace: &Path,
        db_path: &Path,
        data_dir: &Path,
    ) -> Self {
        let port_arg = port.to_string();
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            port_arg.as_str(),
        ]);
        command.current_dir(workspace);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env("CORTEX_NO_NET", "1")
            .env("CORTEX_DB_PATH", db_path)
            .env("CORTEX_DATA_DIR", data_dir);
        let mut child = command.spawn().expect("spawn cortex serve");
        let _ = wait_for_health(port, &mut child);
        Self {
            child,
            port,
            stopped: false,
            _scratch: None,
        }
    }

    /// Start `cortex serve` rooted at `workspace` under `CORTEX_NO_NET=1` with the audit JSONL
    /// pointed at `audit_log`. Used to prove auxiliary turns (/summarize, /compact) fail closed and
    /// leave an auditable blocked-network record when the kill-switch is engaged.
    fn start_no_net_with_audit(
        port: u16,
        workspace: &Path,
        db_path: &Path,
        data_dir: &Path,
        audit_log: &Path,
    ) -> Self {
        let port_arg = port.to_string();
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            port_arg.as_str(),
        ]);
        command.current_dir(workspace);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env("CORTEX_NO_NET", "1")
            .env("CORTEX_AUDIT_LOG", audit_log)
            .env("CORTEX_DB_PATH", db_path)
            .env("CORTEX_DATA_DIR", data_dir);
        let mut child = command.spawn().expect("spawn cortex serve");
        let _ = wait_for_health(port, &mut child);
        Self {
            child,
            port,
            stopped: false,
            _scratch: None,
        }
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(pid) = single_listener_pid(self.port) {
            kill_pid(pid, None);
        }
        wait_for_child_exit(&mut self.child, WAIT_TIMEOUT);
        self.stopped = true;
    }

    fn captured_output(&mut self) -> (String, String) {
        self.stop();
        let mut stdout = String::new();
        if let Some(mut handle) = self.child.stdout.take() {
            handle.read_to_string(&mut stdout).expect("read stdout");
        }
        let mut stderr = String::new();
        if let Some(mut handle) = self.child.stderr.take() {
            handle.read_to_string(&mut stderr).expect("read stderr");
        }
        (stdout, stderr)
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

struct HttpResponse {
    status: u16,
    headers: String,
    body: String,
}

fn cortex_command(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_DATA_DIR");
    command
}

fn wait_for_health(port: u16, child: &mut Child) -> HttpResponse {
    let started = Instant::now();
    let mut last_error = String::new();
    while started.elapsed() < WAIT_TIMEOUT {
        if let Some(status) = child.try_wait().expect("try wait") {
            panic!("server exited before health on port {port}: {status}");
        }
        match request_health(port) {
            Ok(response) if response.status == 200 => return response,
            Ok(response) => {
                last_error = format!("HTTP {}", response.status);
            }
            Err(err) => {
                last_error = err.to_string();
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("server on port {port} did not become healthy: {last_error}");
}

fn request_health(port: u16) -> std::io::Result<HttpResponse> {
    request_http(port, "GET", "/health", None)
}

fn request_json(port: u16, method: &str, path: &str, body: Option<&str>) -> HttpResponse {
    request_http(port, method, path, body).unwrap_or_else(|err| {
        panic!("HTTP request {method} {path} to port {port} failed: {err}");
    })
}

fn request_json_with_headers(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> HttpResponse {
    request_http_with_headers(port, method, path, body, headers).unwrap_or_else(|err| {
        panic!("HTTP request {method} {path} to port {port} failed: {err}");
    })
}

fn request_http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    request_http_with_headers(port, method, path, body, &[])
}

fn request_http_with_headers(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> std::io::Result<HttpResponse> {
    request_http_with_headers_and_timeout(port, method, path, body, headers, Duration::from_secs(2))
}

fn request_http_with_timeout(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    read_timeout: Duration,
) -> std::io::Result<HttpResponse> {
    request_http_with_headers_and_timeout(port, method, path, body, &[], read_timeout)
}

fn request_http_with_headers_and_timeout(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
    read_timeout: Duration,
) -> std::io::Result<HttpResponse> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(read_timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let body = body.unwrap_or("");
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes())?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Ok(HttpResponse {
        status,
        headers: head.to_owned(),
        body: body.to_owned(),
    })
}

fn assert_json_content_type(response: &HttpResponse) {
    let headers = response.headers.to_ascii_lowercase();
    assert!(
        headers.contains("content-type: application/json"),
        "expected application/json response, got headers:\n{}",
        response.headers
    );
}

fn run_to_output(mut command: Command, timeout: Duration) -> Output {
    let mut child = command.spawn().expect("spawn command");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(25));
    };

    let mut stdout = Vec::new();
    if let Some(mut handle) = child.stdout.take() {
        handle.read_to_end(&mut stdout).expect("read stdout");
    }
    let mut stderr = Vec::new();
    if let Some(mut handle) = child.stderr.take() {
        handle.read_to_end(&mut stderr).expect("read stderr");
    }
    Output {
        status,
        stdout,
        stderr,
    }
}

fn free_port() -> u16 {
    for port in (server::MIN_PORT..=server::MAX_PORT).rev() {
        if listener_pids(port).is_empty() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port in 3200-3299");
}

fn listener_pids(port: u16) -> Vec<u32> {
    let output = Command::new("lsof")
        .arg(format!("-tiTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .expect("run lsof");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn single_listener_pid(port: u16) -> Option<u32> {
    let pids = listener_pids(port);
    match pids.as_slice() {
        [pid] => Some(*pid),
        [] => None,
        many => panic!("expected a single listener pid for {port}, got {many:?}"),
    }
}

fn listener_details(port: u16) -> String {
    let output = Command::new("lsof")
        .arg("-nP")
        .arg(format!("-iTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .expect("run lsof details");
    assert!(
        output.status.success(),
        "lsof details failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn kill_pid(pid: u32, signal: Option<&str>) {
    let mut command = Command::new("kill");
    if let Some(signal) = signal {
        command.arg(signal);
    }
    let status = command.arg(pid.to_string()).status().expect("run kill");
    assert!(status.success(), "kill {pid} failed with {status}");
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) {
    let started = Instant::now();
    loop {
        if child.try_wait().expect("try wait").is_some() {
            return;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("server child did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_port_released(port: u16) {
    let started = Instant::now();
    while started.elapsed() < WAIT_TIMEOUT {
        if listener_pids(port).is_empty() {
            assert!(
                TcpListener::bind(("127.0.0.1", port)).is_ok(),
                "port {port} has no lsof listener but cannot be rebound"
            );
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("port {port} still has a listener after shutdown");
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cortex-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self { dir }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("cortex.db")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
