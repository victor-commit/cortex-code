//! Black-box integration coverage for the TUI control, LSP/VCS/formatter, PTY, and
//! project/workspace/worktree/path route families
//! (VAL-GAP-SERVER-013/014/015/016).
//!
//! TUI control exercises the full enqueue → `/tui/control/next` → apply → `/tui/control/response`
//! round-trip server-side (the attached TUI is proven separately with tuistory). LSP/formatter
//! status mirrors the CLI, VCS diff is read-only while apply mutates disk only when requested, PTY
//! routes drive a real pseudo-terminal child, and project/workspace/worktree/path routes reflect
//! real git/filesystem state with clean 4xx for invalid paths.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---- TUI control routes (VAL-GAP-SERVER-013) ----

#[tokio::test]
async fn tui_control_round_trip_reports_success() {
    let scratch = Scratch::new("server-tui-control-ok");
    let app = isolated_app(&scratch, &scratch.workspace());

    // The issuing client posts a control and blocks until the attached TUI acknowledges it.
    let issuer = {
        let app = app.clone();
        tokio::spawn(async move {
            send(
                &app,
                "POST",
                "/tui/show-toast",
                Some(r#"{"message":"hello from server","variant":"success"}"#),
            )
            .await
        })
    };

    let control = poll_control(&app).await.expect("control request delivered");
    assert_eq!(control["action"], "show-toast");
    assert_eq!(control["body"]["message"], "hello from server");
    let id = control["id"].as_str().expect("control id").to_owned();

    let response = send(
        &app,
        "POST",
        "/tui/control/response",
        Some(&format!(r#"{{"id":"{id}","success":true}}"#)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let ack: serde_json::Value = json_body(response).await;
    assert_eq!(ack["status"], "ok");
    assert_eq!(ack["matched"], true);

    let issued = issuer.await.expect("issuer task");
    assert_eq!(issued.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(issued).await;
    assert_eq!(body["status"], "ok", "TUI applied the control: {body:?}");
    assert_eq!(body["action"], "show-toast");
}

#[tokio::test]
async fn tui_control_round_trip_reports_error_outcome() {
    let scratch = Scratch::new("server-tui-control-err");
    let app = isolated_app(&scratch, &scratch.workspace());

    let issuer = {
        let app = app.clone();
        tokio::spawn(async move {
            send(
                &app,
                "POST",
                "/tui/select-session",
                Some(r#"{"session_id":"ses-does-not-exist"}"#),
            )
            .await
        })
    };

    let control = poll_control(&app).await.expect("control request delivered");
    assert_eq!(control["action"], "select-session");
    let id = control["id"].as_str().expect("control id").to_owned();

    // The attached TUI reports a failure (e.g. session not found).
    let response = send(
        &app,
        "POST",
        "/tui/control/response",
        Some(&format!(
            r#"{{"id":"{id}","success":false,"error":"session not found"}}"#
        )),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let issued = issuer.await.expect("issuer task");
    assert_eq!(issued.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(issued).await;
    assert_eq!(body["status"], "error", "control failed: {body:?}");
    assert_eq!(body["error"], "session not found");
}

#[tokio::test]
async fn tui_control_next_is_204_when_empty_and_carries_action_body() {
    let scratch = Scratch::new("server-tui-control-empty");
    let app = isolated_app(&scratch, &scratch.workspace());

    let response = send(&app, "GET", "/tui/control/next", None).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // append-prompt carries its text through to the attached TUI.
    let issuer = {
        let app = app.clone();
        tokio::spawn(async move {
            send(
                &app,
                "POST",
                "/tui/append-prompt",
                Some(r#"{"text":"server-typed text"}"#),
            )
            .await
        })
    };
    let control = poll_control(&app).await.expect("append-prompt delivered");
    assert_eq!(control["action"], "append-prompt");
    assert_eq!(control["body"]["text"], "server-typed text");
    let id = control["id"].as_str().expect("id").to_owned();
    let _ = send(
        &app,
        "POST",
        "/tui/control/response",
        Some(&format!(r#"{{"id":"{id}","success":true}}"#)),
    )
    .await;
    let issued = issuer.await.expect("issuer");
    assert_eq!(issued.status(), StatusCode::OK);
}

#[tokio::test]
async fn tui_control_response_for_unknown_id_is_unmatched() {
    let scratch = Scratch::new("server-tui-control-unknown");
    let app = isolated_app(&scratch, &scratch.workspace());

    let response = send(
        &app,
        "POST",
        "/tui/control/response",
        Some(r#"{"id":"ctl-nope","success":true}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["matched"], false);
}

// ---- LSP / formatter / VCS routes (VAL-GAP-SERVER-014) ----

#[tokio::test]
async fn lsp_status_lists_configured_servers() {
    let scratch = Scratch::new("server-lsp-status");
    let app = isolated_app(&scratch, &scratch.workspace());

    let response = send(&app, "GET", "/lsp", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let rows: serde_json::Value = json_body(response).await;
    let rows = rows.as_array().expect("lsp status array");
    let rust = rows
        .iter()
        .find(|row| row["language"] == "rust")
        .expect("rust server listed");
    assert_eq!(rust["program"], "rust-analyzer");
    assert!(
        rust["state"].is_string(),
        "each row should report a state: {rust:?}"
    );
}

#[tokio::test]
async fn formatter_status_lists_formatters() {
    let scratch = Scratch::new("server-formatter-status");
    let app = isolated_app(&scratch, &scratch.workspace());

    let response = send(&app, "GET", "/formatter", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let rows: serde_json::Value = json_body(response).await;
    let rows = rows.as_array().expect("formatter status array");
    let rustfmt = rows
        .iter()
        .find(|row| row["formatter"] == "rustfmt")
        .expect("rustfmt listed");
    assert!(rustfmt["state"].is_string());
    assert!(rustfmt["source"].is_string());
    assert!(
        rustfmt["extensions"]
            .as_array()
            .is_some_and(|exts| exts.iter().any(|ext| ext == "rs" || ext == ".rs")),
        "rustfmt should match .rs files: {rustfmt:?}"
    );
}

#[tokio::test]
async fn vcs_diff_reports_working_tree_changes() {
    let scratch = Scratch::new("server-vcs-diff");
    let workspace = scratch.workspace();
    init_repo(&workspace);
    std::fs::write(workspace.join("a.txt"), "one\n").expect("write a");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "init"]);
    std::fs::write(workspace.join("a.txt"), "two\n").expect("modify a");

    let app = isolated_app(&scratch, &workspace);
    let response = send(&app, "GET", "/vcs/diff", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["repository"], true);
    let diff = body["diff"].as_str().expect("diff text");
    assert!(diff.contains("a.txt"), "diff names the file: {diff}");
    assert!(diff.contains("+two"), "diff shows the new content: {diff}");
    assert!(
        body["files"]
            .as_array()
            .is_some_and(|files| files.iter().any(|file| file["path"] == "a.txt")),
        "files list includes a.txt: {body:?}"
    );
}

#[tokio::test]
async fn vcs_diff_outside_repository_is_empty() {
    let scratch = Scratch::new("server-vcs-nonrepo");
    let app = isolated_app(&scratch, &scratch.workspace());
    let response = send(&app, "GET", "/vcs/diff", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["repository"], false);
    assert_eq!(body["diff"], "");
}

#[tokio::test]
async fn vcs_apply_mutates_disk_only_when_requested() {
    let scratch = Scratch::new("server-vcs-apply");
    let workspace = scratch.workspace();
    init_repo(&workspace);
    std::fs::write(workspace.join("a.txt"), "one\n").expect("write a");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "init"]);

    // Capture a diff that turns "one" into "two".
    std::fs::write(workspace.join("a.txt"), "two\n").expect("modify a");
    let app = isolated_app(&scratch, &workspace);
    let diff_body: serde_json::Value = json_body(send(&app, "GET", "/vcs/diff", None).await).await;
    let diff = diff_body["diff"].as_str().expect("diff text").to_owned();

    // Restore the working tree so the patch applies against "one".
    std::fs::write(workspace.join("a.txt"), "one\n").expect("restore a");

    // `check: true` validates without mutating disk.
    let payload = serde_json::json!({ "diff": diff, "check": true }).to_string();
    let response = send(&app, "POST", "/vcs/apply", Some(&payload)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["checked"], true);
    assert_eq!(body["applied"], false);
    assert_eq!(
        std::fs::read_to_string(workspace.join("a.txt")).unwrap(),
        "one\n",
        "check must not mutate the file"
    );

    // A real apply mutates disk exactly as the patch requests.
    let payload = serde_json::json!({ "diff": diff }).to_string();
    let response = send(&app, "POST", "/vcs/apply", Some(&payload)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["applied"], true);
    assert_eq!(
        std::fs::read_to_string(workspace.join("a.txt")).unwrap(),
        "two\n",
        "apply must write the patched content"
    );
}

#[tokio::test]
async fn vcs_apply_rejects_bad_patch_and_empty_body() {
    let scratch = Scratch::new("server-vcs-apply-bad");
    let workspace = scratch.workspace();
    init_repo(&workspace);
    std::fs::write(workspace.join("a.txt"), "one\n").expect("write a");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "init"]);

    let app = isolated_app(&scratch, &workspace);
    let response = send(&app, "POST", "/vcs/apply", Some(r#"{"diff":""}"#)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let bad = serde_json::json!({ "diff": "this is not a valid unified diff\n" }).to_string();
    let response = send(&app, "POST", "/vcs/apply", Some(&bad)).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// ---- PTY routes (VAL-GAP-SERVER-015) ----

#[tokio::test]
async fn pty_create_streams_output_and_exits() {
    let scratch = Scratch::new("server-pty-output");
    let app = isolated_app(&scratch, &scratch.workspace());

    let create = serde_json::json!({ "command": "printf", "args": ["pty-hello-123"] }).to_string();
    let response = send(&app, "POST", "/pty", Some(&create)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let summary: serde_json::Value = json_body(response).await;
    let id = summary["id"].as_str().expect("pty id").to_owned();
    assert!(
        summary["pid"].as_u64().is_some(),
        "pty reports a pid: {summary:?}"
    );

    let output = collect_pty_stream(&app, &id).await;
    assert!(
        output.contains("pty-hello-123"),
        "stream should carry printf output, got: {output:?}"
    );
}

#[tokio::test]
async fn pty_accepts_input_and_echoes_through_the_stream() {
    let scratch = Scratch::new("server-pty-input");
    let app = isolated_app(&scratch, &scratch.workspace());

    let create =
        serde_json::json!({ "command": "sh", "args": ["-c", "read line; printf 'echoed:%s' \"$line\""] })
            .to_string();
    let id = json_body::<serde_json::Value>(send(&app, "POST", "/pty", Some(&create)).await).await
        ["id"]
        .as_str()
        .expect("pty id")
        .to_owned();

    let input = serde_json::json!({ "data": "world\n" }).to_string();
    let response = send(&app, "POST", &format!("/pty/{id}/input"), Some(&input)).await;
    assert_eq!(response.status(), StatusCode::OK);

    let output = collect_pty_stream(&app, &id).await;
    assert!(
        output.contains("echoed:world"),
        "input should reach the child and echo back, got: {output:?}"
    );
}

#[tokio::test]
async fn pty_resize_updates_state_and_close_removes_session() {
    let scratch = Scratch::new("server-pty-resize");
    let app = isolated_app(&scratch, &scratch.workspace());

    // A child that blocks on stdin so it stays alive across resize/close.
    let create = serde_json::json!({ "command": "sh", "args": ["-c", "read x"] }).to_string();
    let id = json_body::<serde_json::Value>(send(&app, "POST", "/pty", Some(&create)).await).await
        ["id"]
        .as_str()
        .expect("pty id")
        .to_owned();

    let resize = serde_json::json!({ "cols": 120, "rows": 40 }).to_string();
    let response = send(&app, "POST", &format!("/pty/{id}/resize"), Some(&resize)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["cols"], 120);
    assert_eq!(body["rows"], 40);

    let got: serde_json::Value =
        json_body(send(&app, "GET", &format!("/pty/{id}"), None).await).await;
    assert_eq!(got["cols"], 120, "resize state persists: {got:?}");

    // Closing kills the child by its handle and drops the session.
    let response = send(&app, "DELETE", &format!("/pty/{id}"), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["status"], "closed");

    let response = send(&app, "GET", &format!("/pty/{id}"), None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pty_create_rejects_unknown_command() {
    let scratch = Scratch::new("server-pty-bad");
    let app = isolated_app(&scratch, &scratch.workspace());
    let create = serde_json::json!({ "command": "cortex-no-such-binary-xyz-987" }).to_string();
    let response = send(&app, "POST", "/pty", Some(&create)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = send(&app, "GET", "/pty/pty-missing", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---- project / workspace / worktree / path routes (VAL-GAP-SERVER-016) ----

#[tokio::test]
async fn project_routes_reflect_git_state() {
    let scratch = Scratch::new("server-project");
    let workspace = scratch.workspace();
    init_repo(&workspace);
    std::fs::write(workspace.join("README.md"), "# project\n").expect("write");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "init"]);

    let app = isolated_app(&scratch, &workspace);
    let list: serde_json::Value = json_body(send(&app, "GET", "/project", None).await).await;
    let projects = list.as_array().expect("project array");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["vcs"], "git");

    let current: serde_json::Value =
        json_body(send(&app, "GET", "/project/current", None).await).await;
    assert_eq!(current["vcs"], "git");
    assert!(
        current["worktree"]
            .as_str()
            .is_some_and(|wt| wt.contains("workspace")),
        "worktree reflects the git toplevel: {current:?}"
    );
    assert_eq!(current["id"], projects[0]["id"]);
}

#[tokio::test]
async fn workspace_and_worktree_routes_reflect_real_state() {
    let scratch = Scratch::new("server-workspace");
    let workspace = scratch.workspace();
    init_repo(&workspace);
    std::fs::write(workspace.join("f.txt"), "x\n").expect("write");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "init"]);

    let app = isolated_app(&scratch, &workspace);
    let ws: serde_json::Value = json_body(send(&app, "GET", "/workspace", None).await).await;
    assert_eq!(ws["vcs"], "git");
    assert!(ws["worktrees"].as_u64().unwrap_or(0) >= 1);

    let worktrees: serde_json::Value = json_body(send(&app, "GET", "/worktree", None).await).await;
    let worktrees = worktrees.as_array().expect("worktree array");
    assert!(!worktrees.is_empty(), "main worktree should be listed");
    assert!(
        worktrees
            .iter()
            .any(|wt| wt["path"].as_str().is_some_and(|p| p.contains("workspace"))),
        "the workspace path should appear in the worktree list: {worktrees:?}"
    );
}

#[tokio::test]
async fn worktree_lists_linked_worktrees() {
    let scratch = Scratch::new("server-worktree-linked");
    let workspace = scratch.workspace();
    init_repo(&workspace);
    std::fs::write(workspace.join("f.txt"), "x\n").expect("write");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "init"]);
    let linked = scratch.dir.join("linked-tree");
    git(
        &workspace,
        &["worktree", "add", linked.to_str().unwrap(), "-b", "feature"],
    );

    let app = isolated_app(&scratch, &workspace);
    let worktrees: serde_json::Value = json_body(send(&app, "GET", "/worktree", None).await).await;
    let worktrees = worktrees.as_array().expect("worktree array");
    assert!(
        worktrees.len() >= 2,
        "main + linked worktree should be listed: {worktrees:?}"
    );
    assert!(
        worktrees.iter().any(|wt| wt["path"]
            .as_str()
            .is_some_and(|p| p.contains("linked-tree"))),
        "linked worktree should appear: {worktrees:?}"
    );
}

#[tokio::test]
async fn path_route_summary_metadata_and_invalid_paths() {
    let scratch = Scratch::new("server-path");
    let workspace = scratch.workspace();
    init_repo(&workspace);
    std::fs::write(workspace.join("present.txt"), "hi\n").expect("write");
    std::fs::create_dir_all(workspace.join("sub")).expect("subdir");
    // A secret outside the workspace must never be reachable through the path route.
    std::fs::write(scratch.dir.join("secret.txt"), "top secret").expect("write secret");

    let app = isolated_app(&scratch, &workspace);

    // Summary with no query.
    let summary: serde_json::Value = json_body(send(&app, "GET", "/path", None).await).await;
    assert!(summary["directory"].as_str().is_some());
    assert!(summary["worktree"].as_str().is_some());
    assert!(
        summary["config"]
            .as_str()
            .is_some_and(|c| c.contains("cortex.toml"))
    );

    // Metadata for an existing file.
    let meta: serde_json::Value =
        json_body(send(&app, "GET", "/path?path=present.txt", None).await).await;
    assert_eq!(meta["type"], "file");
    assert_eq!(meta["exists"], true);

    // Metadata for an existing directory.
    let meta: serde_json::Value = json_body(send(&app, "GET", "/path?path=sub", None).await).await;
    assert_eq!(meta["type"], "directory");

    // Missing path -> 404.
    let response = send(&app, "GET", "/path?path=missing.txt", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Traversal outside the workspace -> 403.
    let response = send(&app, "GET", "/path?path=../secret.txt", None).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ---- helpers ----

fn isolated_app(scratch: &Scratch, workspace: &Path) -> axum::Router {
    server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace.to_path_buf())
            .with_config_env(Vec::new()),
    )
}

/// Poll `/tui/control/next` until a control request is delivered or a short budget elapses.
async fn poll_control(app: &axum::Router) -> Option<serde_json::Value> {
    for _ in 0..200 {
        let response = send(app, "GET", "/tui/control/next", None).await;
        if response.status() == StatusCode::OK {
            return Some(json_body(response).await);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    None
}

/// Read a PTY SSE output stream to completion and decode the base64 `output` frames into a string.
async fn collect_pty_stream(app: &axum::Router, id: &str) -> String {
    let response = send(app, "GET", &format!("/pty/{id}/stream"), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 4_194_304)
        .await
        .expect("stream body");
    let raw = String::from_utf8_lossy(&bytes);
    let mut decoded = String::new();
    for line in raw.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if let Some(encoded) = value.get("data").and_then(serde_json::Value::as_str) {
            if let Ok(chunk) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                decoded.push_str(&String::from_utf8_lossy(&chunk));
            }
        }
    }
    decoded
}

fn init_repo(workspace: &Path) {
    git(workspace, &["init", "-q"]);
    git(workspace, &["config", "user.name", "Test"]);
    git(workspace, &["config", "user.email", "test@example.invalid"]);
}

fn git(workspace: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .expect("request"),
        None => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            builder.body(Body::empty()).expect("request")
        }
    };
    app.clone().oneshot(request).await.expect("response")
}

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 1_048_576)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
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

    fn workspace(&self) -> PathBuf {
        let workspace = self.dir.join("workspace");
        std::fs::create_dir_all(&workspace).expect("create workspace dir");
        workspace
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
