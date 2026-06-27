//! Black-box integration coverage for the file, find, and catalog route families
//! (VAL-GAP-SERVER-009/010/011): safe file list/content/status routes with external-directory
//! gating and binary/media metadata; bounded text/file/symbol search that skips ignored and binary
//! files; and agent/model/command/skill catalogs that surface built-in, custom, config, and
//! plugin-registered entries.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---- File routes (VAL-GAP-SERVER-009) ----

#[tokio::test]
async fn file_list_returns_entries_with_type_and_ignored_flags() {
    let scratch = Scratch::new("server-file-list");
    let workspace = scratch.workspace();
    std::fs::write(workspace.join("readme.md"), "# hi\n").expect("write readme");
    std::fs::create_dir_all(workspace.join("src")).expect("src dir");
    std::fs::create_dir_all(workspace.join("node_modules")).expect("node_modules dir");

    let app = isolated_app(&scratch, &workspace);

    let response = send(&app, "GET", "/file", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let entries = body.as_array().expect("file list array");

    let find = |name: &str| entries.iter().find(|entry| entry["name"] == name);
    let src = find("src").expect("src directory listed");
    assert_eq!(src["type"], "directory");
    assert_eq!(src["ignored"], false);
    let readme = find("readme.md").expect("readme listed");
    assert_eq!(readme["type"], "file");
    assert_eq!(readme["ignored"], false);
    assert!(
        readme["path"]
            .as_str()
            .is_some_and(|p| p.contains("readme.md"))
    );
    let node_modules = find("node_modules").expect("node_modules listed");
    assert_eq!(
        node_modules["ignored"], true,
        "conventionally ignored directories should be flagged: {entries:?}"
    );

    // Listing a sub-path scopes to that directory.
    std::fs::write(workspace.join("src/main.rs"), "fn main() {}\n").expect("write main");
    let response = send(&app, "GET", "/file?path=src", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body.as_array()
            .expect("array")
            .iter()
            .any(|entry| entry["name"] == "main.rs"),
        "sub-path listing should show that directory's entries: {body:?}"
    );
}

#[tokio::test]
async fn file_content_returns_text_media_binary_and_clean_errors() {
    let scratch = Scratch::new("server-file-content");
    let workspace = scratch.workspace();
    std::fs::write(workspace.join("hello.txt"), "hello world\n").expect("write text");
    // A real PNG header so media sniffing classifies it as an image.
    let png = [
        0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
    ];
    std::fs::write(workspace.join("pic.png"), png).expect("write png");
    // A non-media binary file (contains a NUL byte).
    std::fs::write(workspace.join("blob.bin"), [0u8, 1, 2, 3, 255, 7]).expect("write blob");
    // A secret living OUTSIDE the workspace that must never be readable through the route.
    let secret = scratch.dir.join("secret.txt");
    std::fs::write(&secret, "top secret").expect("write secret");

    let app = isolated_app(&scratch, &workspace);

    // Text file.
    let response = send(&app, "GET", "/file/content?path=hello.txt", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["type"], "text");
    assert!(
        body["content"]
            .as_str()
            .is_some_and(|c| c.contains("hello world"))
    );
    assert!(body.get("encoding").is_none() || body["encoding"].is_null());

    // Media file: base64-encoded binary with a recognized mime type.
    let response = send(&app, "GET", "/file/content?path=pic.png", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["type"], "binary");
    assert_eq!(body["encoding"], "base64");
    assert_eq!(body["mimeType"], "image/png");
    assert!(!body["content"].as_str().unwrap_or_default().is_empty());

    // Non-media binary: generic octet-stream, still base64.
    let response = send(&app, "GET", "/file/content?path=blob.bin", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body["type"], "binary");
    assert_eq!(body["mimeType"], "application/octet-stream");

    // Missing file inside the workspace -> 404.
    let response = send(&app, "GET", "/file/content?path=nope.txt", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Traversal escape -> 403, and the secret is never disclosed or mutated.
    let response = send(&app, "GET", "/file/content?path=../secret.txt", None).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let denied = body_text(response).await;
    assert!(
        !denied.contains("top secret"),
        "denied response must not leak external file content: {denied}"
    );

    // Absolute path outside the workspace -> 403.
    let response = send(
        &app,
        "GET",
        &format!("/file/content?path={}", secret.display()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // A traversal escape to a path that does not exist is still denied (not a leaky 404).
    let response = send(
        &app,
        "GET",
        "/file/content?path=../escaped-nonexistent.txt",
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // No mutation occurred.
    assert_eq!(
        std::fs::read_to_string(&secret).expect("secret intact"),
        "top secret"
    );

    // A directory is not file content.
    std::fs::create_dir_all(workspace.join("adir")).expect("adir");
    let response = send(&app, "GET", "/file/content?path=adir", None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn file_status_reports_git_working_tree_changes() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping file_status git proof: git not available");
        return;
    }
    let scratch = Scratch::new("server-file-status");
    let workspace = scratch.workspace();
    git(&workspace, &["init", "-q"]);
    std::fs::write(workspace.join("tracked.txt"), "one\n").expect("write tracked");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-q", "-m", "init"]);
    // Modify a tracked file and add an untracked one.
    std::fs::write(workspace.join("tracked.txt"), "one\ntwo\n").expect("modify tracked");
    std::fs::write(workspace.join("fresh.txt"), "new\n").expect("write fresh");

    let app = isolated_app(&scratch, &workspace);
    let response = send(&app, "GET", "/file/status", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let entries = body.as_array().expect("status array");

    let tracked = entries
        .iter()
        .find(|entry| {
            entry["path"]
                .as_str()
                .is_some_and(|p| p.contains("tracked.txt"))
        })
        .expect("tracked.txt reported");
    assert_eq!(tracked["status"], "modified");
    assert!(
        tracked["added"].as_u64().unwrap_or(0) >= 1,
        "modified file should report added lines: {tracked:?}"
    );
    let fresh = entries
        .iter()
        .find(|entry| {
            entry["path"]
                .as_str()
                .is_some_and(|p| p.contains("fresh.txt"))
        })
        .expect("fresh.txt reported");
    assert_eq!(fresh["status"], "added");
}

#[tokio::test]
async fn file_status_on_non_git_workspace_is_empty() {
    let scratch = Scratch::new("server-file-status-nongit");
    let workspace = scratch.workspace();
    std::fs::write(workspace.join("a.txt"), "x\n").expect("write");
    let app = isolated_app(&scratch, &workspace);
    let response = send(&app, "GET", "/file/status", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body, serde_json::json!([]));
}

// ---- Find routes (VAL-GAP-SERVER-010) ----

#[tokio::test]
async fn find_text_returns_bounded_matches_skipping_ignored_and_binary() {
    let scratch = Scratch::new("server-find-text");
    let workspace = scratch.workspace();
    std::fs::write(workspace.join("a.txt"), "alpha NEEDLE beta\n").expect("write a");
    std::fs::write(workspace.join("b.txt"), "NEEDLE again\nmore NEEDLE here\n").expect("write b");
    // Ignored directory via a ripgrep .ignore file (respected without a git repo).
    std::fs::write(workspace.join(".ignore"), "ignored/\n").expect("write ignore");
    std::fs::create_dir_all(workspace.join("ignored")).expect("ignored dir");
    std::fs::write(workspace.join("ignored/c.txt"), "NEEDLE hidden\n").expect("write ignored");
    // Binary file containing the needle bytes; ripgrep skips binary content by default.
    let mut binary = b"NEEDLE".to_vec();
    binary.push(0);
    binary.extend_from_slice(b"morebytes");
    std::fs::write(workspace.join("blob.bin"), binary).expect("write binary");

    let app = isolated_app(&scratch, &workspace);

    let response = send(&app, "GET", "/find?pattern=NEEDLE", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body_large(response).await;
    let matches = body.as_array().expect("matches array");
    assert!(
        matches.len() >= 3,
        "expected matches in a.txt and b.txt: {matches:?}"
    );
    for entry in matches {
        let path = entry["path"]["text"].as_str().unwrap_or_default();
        assert!(
            !path.contains("ignored"),
            "ignored directory matches must be skipped: {path}"
        );
        assert!(
            !path.contains("blob.bin"),
            "binary file matches must be skipped: {path}"
        );
        assert!(entry["line_number"].as_u64().is_some());
        assert!(entry["lines"]["text"].as_str().is_some());
    }

    // The limit is honored.
    let response = send(&app, "GET", "/find?pattern=NEEDLE&limit=1", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    assert_eq!(body.as_array().map(Vec::len), Some(1));

    // A missing pattern is a clean client error.
    let response = send(&app, "GET", "/find", None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn find_file_matches_names_and_directories() {
    let scratch = Scratch::new("server-find-file");
    let workspace = scratch.workspace();
    std::fs::write(workspace.join("alpha_module.rs"), "").expect("write alpha");
    std::fs::create_dir_all(workspace.join("nested")).expect("nested dir");
    std::fs::write(workspace.join("nested/beta_module.rs"), "").expect("write beta");

    let app = isolated_app(&scratch, &workspace);

    let response = send(&app, "GET", "/find/file?query=alpha", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let paths: Vec<String> = body
        .as_array()
        .expect("path array")
        .iter()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect();
    assert!(
        paths.iter().any(|path| path.contains("alpha_module.rs")),
        "alpha file should match: {paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path.contains("beta_module.rs")),
        "non-matching files should be excluded: {paths:?}"
    );

    // Nested file is found by its base name.
    let response = send(&app, "GET", "/find/file?query=beta", None).await;
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body.as_array().expect("array").iter().any(|value| value
            .as_str()
            .is_some_and(|p| p.contains("nested/beta_module.rs"))),
        "nested file should be found: {body:?}"
    );

    // Directories are surfaced when requested.
    let response = send(&app, "GET", "/find/file?query=nested&dirs=true", None).await;
    let body: serde_json::Value = json_body(response).await;
    assert!(
        body.as_array()
            .expect("array")
            .iter()
            .any(|value| value.as_str() == Some("nested")),
        "dirs=true should surface the matching directory: {body:?}"
    );
}

#[tokio::test]
async fn find_symbol_locates_source_definitions() {
    let scratch = Scratch::new("server-find-symbol");
    let workspace = scratch.workspace();
    std::fs::write(
        workspace.join("lib.rs"),
        "fn alpha_helper() {}\nstruct BetaThing;\n",
    )
    .expect("write source");

    let app = isolated_app(&scratch, &workspace);

    let response = send(&app, "GET", "/find/symbol?query=alpha_helper", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let symbol = body
        .as_array()
        .expect("symbol array")
        .iter()
        .find(|entry| entry["name"] == "alpha_helper")
        .expect("alpha_helper symbol");
    assert_eq!(symbol["kind"], 12, "fn maps to the LSP Function kind");
    assert!(
        symbol["location"]["uri"]
            .as_str()
            .is_some_and(|uri| uri.contains("lib.rs")),
        "symbol location should point at the source file: {symbol:?}"
    );

    let response = send(&app, "GET", "/find/symbol?query=BetaThing", None).await;
    let body: serde_json::Value = json_body(response).await;
    let symbol = body
        .as_array()
        .expect("array")
        .iter()
        .find(|entry| entry["name"] == "BetaThing")
        .expect("BetaThing symbol");
    assert_eq!(symbol["kind"], 23, "struct maps to the LSP Struct kind");

    // Empty query is a clean client error.
    let response = send(&app, "GET", "/find/symbol", None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ---- Catalog routes (VAL-GAP-SERVER-011) ----

#[tokio::test]
async fn agent_catalog_lists_builtin_and_custom_agents() {
    let scratch = Scratch::new("server-agent-catalog");
    let workspace = scratch.workspace();
    let agents_dir = workspace.join(".cortex").join("agents");
    std::fs::create_dir_all(&agents_dir).expect("agents dir");
    std::fs::write(
        agents_dir.join("server-test-scout.md"),
        "---\nname: server-test-scout\ndescription: A scouting agent\nmode: subagent\n---\nInvestigate and report.\n",
    )
    .expect("write custom agent");

    let app = isolated_app(&scratch, &workspace);
    let response = send(&app, "GET", "/agent", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body_large(response).await;
    let agents = body.as_array().expect("agent array");

    for builtin in ["build", "plan", "general", "explore"] {
        let entry = agents
            .iter()
            .find(|agent| agent["name"] == builtin)
            .unwrap_or_else(|| panic!("built-in agent `{builtin}` should be listed: {agents:?}"));
        assert_eq!(entry["source"], "builtin");
        assert!(entry["description"].as_str().is_some_and(|d| !d.is_empty()));
    }

    let custom = agents
        .iter()
        .find(|agent| agent["name"] == "server-test-scout")
        .expect("custom agent listed");
    assert_eq!(custom["source"], "custom");
    assert_eq!(custom["description"], "A scouting agent");
}

#[tokio::test]
async fn model_catalog_lists_offline_models_with_default() {
    let scratch = Scratch::new("server-model-catalog");
    let workspace = scratch.workspace();
    let app = isolated_app(&scratch, &workspace);

    let response = send(&app, "GET", "/model", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body_large(response).await;
    let default_model = body["default_model"].as_str().expect("default model");
    assert!(
        default_model
            .split_once('/')
            .is_some_and(|(p, m)| !p.is_empty() && !m.is_empty()),
        "default model should be fully qualified: {default_model}"
    );
    let models = body["models"].as_array().expect("models array");
    assert!(!models.is_empty(), "offline catalog should expose models");
    assert!(
        models
            .iter()
            .all(|model| model["id"].as_str().is_some_and(|id| !id.is_empty())),
        "every catalog model should carry an id: {models:?}"
    );
    assert!(
        models
            .iter()
            .any(|model| model["id"].as_str() == Some(default_model)),
        "the default model should appear in the catalog: {default_model}"
    );
}

#[tokio::test]
async fn command_and_skill_catalogs_list_config_entries() {
    let scratch = Scratch::new("server-command-skill-catalog");
    let workspace = scratch.workspace();
    std::fs::write(
        workspace.join("cortex.toml"),
        "[command.deploy]\nprompt = \"Deploy the app\"\ndescription = \"Deploy the application\"\n",
    )
    .expect("write config");
    let skills_dir = workspace.join(".cortex").join("skills");
    std::fs::create_dir_all(&skills_dir).expect("skills dir");
    std::fs::write(
        skills_dir.join("server-test-skill.md"),
        "---\nname: server-test-skill\ndescription: A helper skill for tests\n---\nAlways do the thing.\n",
    )
    .expect("write skill");

    let app = isolated_app(&scratch, &workspace);

    let response = send(&app, "GET", "/command", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let command = body
        .as_array()
        .expect("command array")
        .iter()
        .find(|entry| entry["name"] == "deploy")
        .expect("config command listed");
    assert_eq!(command["source"], "config");
    assert_eq!(command["description"], "Deploy the application");

    let response = send(&app, "GET", "/skill", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = json_body(response).await;
    let skill = body
        .as_array()
        .expect("skill array")
        .iter()
        .find(|entry| entry["name"] == "server-test-skill")
        .expect("config skill listed");
    assert_eq!(skill["source"], "config");
    assert_eq!(skill["description"], "A helper skill for tests");
}

#[tokio::test]
async fn catalogs_surface_plugin_registered_entries() {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping plugin catalog proof: no Bun, Node, or Deno on PATH");
        return;
    }
    let scratch = Scratch::new("server-plugin-catalog");
    let workspace = scratch.workspace();
    std::fs::write(
        workspace.join("plugin.mjs"),
        r#"
export default async function plugin(input, options) {
  return {
    async agent({ register }) {
      register({ id: "plugin-scout", name: "Plugin Scout", description: "A plugin agent" });
    },
    async command({ register }) {
      register({ id: "plugin-deploy", name: "Plugin Deploy", description: "A plugin command" });
    },
    async skill({ register }) {
      register({ id: "plugin-helper", name: "Plugin Helper", description: "A plugin skill" });
    },
    async provider() {
      return {
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [{ id: "plugin/example-model", name: "Plugin Example", tool_call: false }]
      };
    }
  };
}
"#,
    )
    .expect("write plugin");
    std::fs::write(
        workspace.join("cortex.toml"),
        "plugin = [[\"./plugin.mjs\", {}]]\n",
    )
    .expect("write config");

    // Process env (not isolated) so the plugin JS runtime is discoverable on PATH.
    let app = server::app_with_state(
        server::ServerState::new(scratch.db_path()).with_cwd(workspace.clone()),
    );

    let body: serde_json::Value = json_body_large(send(&app, "GET", "/agent", None).await).await;
    assert!(
        body.as_array()
            .expect("array")
            .iter()
            .any(|entry| { entry["name"] == "plugin-scout" && entry["source"] == "plugin" }),
        "plugin agent should surface in the catalog: {body:?}"
    );

    let body: serde_json::Value = json_body_large(send(&app, "GET", "/command", None).await).await;
    assert!(
        body.as_array()
            .expect("array")
            .iter()
            .any(|entry| { entry["name"] == "plugin-deploy" && entry["source"] == "plugin" }),
        "plugin command should surface in the catalog: {body:?}"
    );

    let body: serde_json::Value = json_body_large(send(&app, "GET", "/skill", None).await).await;
    assert!(
        body.as_array()
            .expect("array")
            .iter()
            .any(|entry| { entry["name"] == "plugin-helper" && entry["source"] == "plugin" }),
        "plugin skill should surface in the catalog: {body:?}"
    );

    let body: serde_json::Value = json_body_large(send(&app, "GET", "/model", None).await).await;
    assert!(
        body["models"]
            .as_array()
            .expect("models array")
            .iter()
            .any(|model| model["id"].as_str() == Some("plugin/example-model")),
        "plugin model should surface in the model catalog: {body:?}"
    );

    let body: serde_json::Value = json_body_large(send(&app, "GET", "/provider", None).await).await;
    assert!(
        body["providers"]
            .as_array()
            .expect("providers array")
            .iter()
            .any(|provider| provider["id"] == "plugin-provider"),
        "plugin provider should surface in /provider: {body:?}"
    );

    let body: serde_json::Value =
        json_body_large(send(&app, "GET", "/config/providers", None).await).await;
    assert!(
        body["providers"]
            .as_array()
            .expect("providers array")
            .iter()
            .any(|provider| provider["id"] == "plugin-provider"),
        "plugin provider should reconcile into /config/providers: {body:?}"
    );
}

// ---- helpers ----

fn isolated_app(scratch: &Scratch, workspace: &Path) -> axum::Router {
    server::app_with_state(
        server::ServerState::new(scratch.db_path())
            .with_cwd(workspace.to_path_buf())
            .with_config_env(Vec::new()),
    )
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

async fn body_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 262_144).await.expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}

async fn json_body<T>(response: axum::response::Response) -> T
where
    T: serde::de::DeserializeOwned,
{
    let bytes = to_bytes(response.into_body(), 262_144).await.expect("body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

async fn json_body_large<T>(response: axum::response::Response) -> T
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
