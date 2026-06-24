//! Black-box checks for `cortex search` (VAL-FINISH-045..050, VAL-FINISH-075).
//!
//! These tests prove real working-tree matches, path+line location parity with ripgrep, clean
//! deterministic no-match behavior, CWD scoping, strict JSON output, no-network operation, and
//! ignore/binary behavior consistent with ripgrep defaults.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-search-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let home = root.join("home");
        fs::create_dir_all(&home).expect("create HOME");
        Self { root, home }
    }

    fn mkdir(&self, rel: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::create_dir_all(&path).expect("create directory");
        path
    }

    fn command_in(&self, cwd: &Path, args: &[&str]) -> Command {
        self.command_in_with_env(cwd, args, &[])
    }

    fn command_in_with_env(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .arg("search")
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
            )
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.root.join(".config"))
            .env_remove("OPENROUTER_API_KEY");
        for (key, value) in env {
            command.env(key, value);
        }
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn spawn(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.spawn().expect("spawn cortex")
}

fn collect(mut child: Child) -> Output {
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
            panic!("cortex search did not terminate within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn run(command: Command) -> Output {
    collect(spawn(command))
}

fn stdout_str(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf8 stdout")
}

fn stderr_str(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn rg_lines(cwd: &Path, query: &str) -> Vec<String> {
    let output = Command::new("rg")
        .args([
            "--line-number",
            "--no-heading",
            "--color",
            "never",
            "-e",
            query,
        ])
        .current_dir(cwd)
        .output()
        .expect("run rg");
    assert!(
        output.status.code() == Some(0) || output.status.code() == Some(1),
        "unexpected rg failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

fn stdout_lines(output: &Output) -> Vec<String> {
    let mut lines: Vec<String> = stdout_str(output).lines().map(str::to_owned).collect();
    lines.sort();
    lines
}

fn strace_usable() -> bool {
    let probe = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "--version",
        ])
        .output();
    match probe {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            out.status.success() && !stderr.to_lowercase().contains("ptrace")
        }
        Err(_) => false,
    }
}

fn count_connect(trace: &str) -> usize {
    trace.matches("connect(").count()
}

#[test]
fn search_returns_session_message_hits_from_the_shared_store() {
    let sandbox = Sandbox::new("session-hits");
    let project = sandbox.mkdir("project");
    let db_path = sandbox.root.join("data/cortex.db");
    let token = "NEEDLE_M7_SESSION_TOKEN";
    let (session_id, message_id) = {
        let db = db::Database::open(&db_path).expect("open db");
        let session = db
            .sessions()
            .create(Some("session search"))
            .expect("session");
        let message = db
            .messages()
            .create(
                &session.id,
                "assistant",
                &format!("Cross-surface marker: {token}"),
            )
            .expect("message");
        (session.id, message.id)
    };
    let db_path_text = db_path.to_string_lossy().to_string();

    let text = run(sandbox.command_in_with_env(
        &project,
        &[token],
        &[("CORTEX_DB_PATH", db_path_text.as_str())],
    ));
    assert_eq!(text.status.code(), Some(0), "stderr: {}", stderr_str(&text));
    let text_stdout = stdout_str(&text);
    assert!(
        text_stdout.contains(&format!("session:{session_id}:")),
        "text output should include the session match path: {text_stdout}"
    );
    assert!(
        text_stdout.contains(token),
        "text output should include the matched token: {text_stdout}"
    );

    let json = run(sandbox.command_in_with_env(
        &project,
        &[token, "--format", "json"],
        &[("CORTEX_DB_PATH", db_path_text.as_str())],
    ));
    assert_eq!(json.status.code(), Some(0), "stderr: {}", stderr_str(&json));
    let doc: Value = serde_json::from_slice(&json.stdout).expect("strict JSON");
    let matches = doc["matches"].as_array().expect("matches array");
    let session_hit = matches.iter().find(|entry| {
        entry["source"] == "session"
            && entry["session_id"] == session_id
            && entry["message_id"] == message_id
    });
    let Some(session_hit) = session_hit else {
        panic!("expected a session search hit, got: {doc:?}");
    };
    assert_eq!(
        session_hit["path"],
        format!("session:{session_id}"),
        "session hit path should encode the owning session id"
    );
    assert!(
        session_hit["matched_text"]
            .as_str()
            .is_some_and(|text| text.contains(token)),
        "session hit should carry the original message content"
    );
}

#[test]
fn search_regex_semantics_apply_to_workspace_and_session_sources() {
    let sandbox = Sandbox::new("regex-parity");
    let project = sandbox.mkdir("project");
    let db_path = sandbox.root.join("data/cortex.db");
    let query = "REGEX_PARITY_[0-9]+";
    let token = "REGEX_PARITY_42";
    std::fs::write(
        project.join("workspace.txt"),
        format!("workspace source carries {token}\n"),
    )
    .expect("write workspace fixture");
    let db_path_text = db_path.to_string_lossy().to_string();

    let session_id = {
        let db = db::Database::open(&db_path).expect("open db");
        let session = db
            .sessions()
            .create(Some("regex parity"))
            .expect("create session");
        db.messages()
            .create(
                &session.id,
                "assistant",
                &format!("session source carries {token}"),
            )
            .expect("create message");
        session.id
    };

    let out = run(sandbox.command_in_with_env(
        &project,
        &[query, "--format", "json"],
        &[("CORTEX_DB_PATH", db_path_text.as_str())],
    ));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    let doc: Value = serde_json::from_slice(&out.stdout).expect("strict JSON");
    let matches = doc["matches"].as_array().expect("matches array");
    assert!(
        matches.iter().any(|entry| {
            entry["source"] == "workspace"
                && entry["path"] == "workspace.txt"
                && entry["matched_text"]
                    .as_str()
                    .is_some_and(|text| text.contains(token))
        }),
        "expected workspace regex hit in output: {doc:?}"
    );
    assert!(
        matches.iter().any(|entry| {
            entry["source"] == "session"
                && entry["session_id"] == session_id
                && entry["matched_text"]
                    .as_str()
                    .is_some_and(|text| text.contains(token))
        }),
        "expected session regex hit in output: {doc:?}"
    );
}

#[test]
fn search_uses_regex_not_literal_substring_for_session_messages() {
    let sandbox = Sandbox::new("session-regex");
    let project = sandbox.mkdir("project");
    let db_path = sandbox.root.join("data/cortex.db");
    let query = "session-regex-[0-9]+";
    let token = "session-regex-789";
    let db_path_text = db_path.to_string_lossy().to_string();

    {
        let db = db::Database::open(&db_path).expect("open db");
        let session = db
            .sessions()
            .create(Some("session regex"))
            .expect("create session");
        db.messages()
            .create(&session.id, "assistant", token)
            .expect("create message");
    }

    let out = run(sandbox.command_in_with_env(
        &project,
        &[query],
        &[("CORTEX_DB_PATH", db_path_text.as_str())],
    ));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    let stdout = stdout_str(&out);
    assert!(
        stdout.contains("session:"),
        "session regex hit should be emitted: {stdout}"
    );
    assert!(
        stdout.contains(token),
        "matched session line should include message content: {stdout}"
    );
}

#[test]
fn search_invalid_regex_is_deterministic_with_workspace_and_session_sources() {
    let sandbox = Sandbox::new("invalid-regex");
    let project = sandbox.mkdir("project");
    let db_path = sandbox.root.join("data/cortex.db");
    let db_path_text = db_path.to_string_lossy().to_string();
    std::fs::write(project.join("workspace.txt"), "workspace marker\n").expect("write fixture");
    {
        let db = db::Database::open(&db_path).expect("open db");
        let session = db
            .sessions()
            .create(Some("invalid regex"))
            .expect("create session");
        db.messages()
            .create(&session.id, "assistant", "session marker")
            .expect("create message");
    }

    let text = run(sandbox.command_in_with_env(
        &project,
        &["("],
        &[("CORTEX_DB_PATH", db_path_text.as_str())],
    ));
    let json = run(sandbox.command_in_with_env(
        &project,
        &["(", "--format", "json"],
        &[("CORTEX_DB_PATH", db_path_text.as_str())],
    ));

    assert_ne!(text.status.code(), Some(0), "text mode should fail");
    assert_ne!(json.status.code(), Some(0), "json mode should fail");
    assert!(
        text.stdout.is_empty(),
        "invalid query should not emit stdout"
    );
    assert!(
        json.stdout.is_empty(),
        "invalid query should not emit stdout"
    );

    let text_stderr = stderr_str(&text);
    let json_stderr = stderr_str(&json);
    assert!(
        text_stderr.contains("cortex search:"),
        "text mode should report a prefixed error: {text_stderr}"
    );
    assert!(
        json_stderr.contains("cortex search:"),
        "json mode should report a prefixed error: {json_stderr}"
    );
    assert_eq!(
        text_stderr, json_stderr,
        "invalid-query diagnostics should be stable across output modes"
    );
}

// VAL-FINISH-045 + VAL-FINISH-046.
#[test]
fn search_returns_real_matches_with_path_and_line_parity_to_rg() {
    let sandbox = Sandbox::new("match");
    let project = sandbox.mkdir("project");
    let needle = "NEEDLE_M7_A";
    fs::write(
        project.join("find.txt"),
        format!("line one\nline two\n{needle} on line three\n"),
    )
    .expect("write fixture");

    let out = run(sandbox.command_in(&project, &[needle]));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(
        stderr_str(&out).is_empty(),
        "successful search should keep stderr empty"
    );

    let stdout = stdout_str(&out);
    assert!(
        stdout.contains("find.txt:3:"),
        "match should include file path and line location: {stdout}"
    );
    assert!(
        stdout.contains(needle),
        "match should include the matched line text: {stdout}"
    );

    assert_eq!(stdout_lines(&out), rg_lines(&project, needle));
}

// VAL-FINISH-047.
#[test]
fn search_no_matches_is_clean_and_deterministic() {
    let sandbox = Sandbox::new("nomatch");
    let project = sandbox.mkdir("project");
    fs::write(project.join("a.txt"), "hello world\n").expect("write fixture");
    let query = "this-token-does-not-exist-0xdeadbeef";

    let first = run(sandbox.command_in(&project, &[query]));
    let second = run(sandbox.command_in(&project, &[query]));
    assert_eq!(
        first.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&first)
    );
    assert_eq!(
        second.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&second)
    );
    assert!(
        first.stdout.is_empty(),
        "no-match result should be clean/empty, got: {}",
        stdout_str(&first)
    );
    assert!(
        second.stdout.is_empty(),
        "repeated no-match result should be clean/empty, got: {}",
        stdout_str(&second)
    );
    assert!(first.stderr.is_empty(), "stderr should stay empty");
    assert!(second.stderr.is_empty(), "stderr should stay empty");
    assert_eq!(first.stdout, second.stdout, "repeat output must match");
    assert_eq!(
        first.status.code(),
        second.status.code(),
        "repeat exit code must match"
    );
}

// VAL-FINISH-047 (empty-tree edge): searching from an empty directory yields a clean deterministic
// empty result instead of surfacing ripgrep's "No files were searched" diagnostic as an error.
#[test]
fn search_in_empty_directory_is_a_clean_empty_success() {
    let sandbox = Sandbox::new("empty-tree");
    let empty = sandbox.mkdir("empty");
    let query = "TOKEN-ABSENT-IN-EMPTY-DIR";

    let out = run(sandbox.command_in(&empty, &[query]));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(
        out.stdout.is_empty(),
        "empty-directory no-match should have empty stdout, got: {}",
        stdout_str(&out)
    );
    assert!(
        out.stderr.is_empty(),
        "empty-directory no-match should keep stderr empty, got: {}",
        stderr_str(&out)
    );
}

// VAL-FINISH-048.
#[test]
fn search_is_scoped_to_current_working_directory() {
    let sandbox = Sandbox::new("scope");
    let dir_a = sandbox.mkdir("A");
    let dir_b = sandbox.mkdir("B");
    let needle = "NEEDLE_M7_B";
    fs::write(dir_a.join("only-in-a.txt"), format!("{needle}\n")).expect("write fixture");
    fs::write(dir_b.join("only-in-b.txt"), "different content\n").expect("write fixture");

    let in_a = run(sandbox.command_in(&dir_a, &[needle]));
    assert_eq!(in_a.status.code(), Some(0), "stderr: {}", stderr_str(&in_a));
    assert!(
        stdout_str(&in_a).contains("only-in-a.txt"),
        "token should be found from directory A"
    );

    let in_b = run(sandbox.command_in(&dir_b, &[needle]));
    assert_eq!(in_b.status.code(), Some(0), "stderr: {}", stderr_str(&in_b));
    assert!(
        in_b.stdout.is_empty(),
        "token from directory A must not appear when searching from unrelated directory B"
    );
}

// VAL-FINISH-049.
#[test]
fn search_json_is_strict_and_contains_match_fields() {
    let sandbox = Sandbox::new("json");
    let project = sandbox.mkdir("project");
    let needle = "NEEDLE_M7_C";
    fs::write(
        project.join("find.txt"),
        format!("first\nsecond {needle} line\n"),
    )
    .expect("write fixture");

    let out = run(sandbox.command_in(&project, &[needle, "--format", "json"]));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(
        out.stderr.is_empty(),
        "json mode must not emit chatter on stderr: {}",
        stderr_str(&out)
    );

    let doc: Value = serde_json::from_slice(&out.stdout).expect("strict JSON stdout");
    let matches = doc["matches"].as_array().expect("matches array");
    assert!(!matches.is_empty(), "json must contain at least one match");
    let first = &matches[0];
    assert!(first["path"].as_str().is_some(), "path must be a string");
    assert!(first["line"].as_u64().is_some(), "line must be numeric");
    let matched_text = first["matched_text"]
        .as_str()
        .expect("matched_text must be a string");
    assert!(
        matched_text.contains(needle),
        "matched_text should contain the searched token, got: {matched_text}"
    );
}

// VAL-FINISH-050.
#[test]
fn search_under_no_net_issues_zero_connect_syscalls() {
    let sandbox = Sandbox::new("none");
    let project = sandbox.mkdir("project");
    let needle = "NEEDLE_M7_D";
    fs::write(project.join("find.txt"), format!("{needle}\n")).expect("write fixture");

    let out = run(sandbox.command_in_with_env(&project, &[needle], &[("CORTEX_NO_NET", "1")]));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));

    if !strace_usable() {
        eprintln!("skipping strace connect assertion: strace unavailable or ptrace restricted");
        return;
    }

    let traced = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "search",
            needle,
        ])
        .current_dir(&project)
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        )
        .env("HOME", &sandbox.home)
        .env("XDG_CONFIG_HOME", sandbox.root.join(".config"))
        .env("CORTEX_NO_NET", "1")
        .output()
        .expect("run strace");
    assert_eq!(
        traced.status.code(),
        Some(0),
        "search under strace should succeed: {}",
        String::from_utf8_lossy(&traced.stderr)
    );
    let trace = String::from_utf8_lossy(&traced.stderr);
    assert_eq!(
        count_connect(&trace),
        0,
        "search should issue zero connect() syscalls:\n{trace}"
    );
}

// VAL-FINISH-075.
#[test]
fn search_respects_ignore_and_binary_defaults_like_rg() {
    let sandbox = Sandbox::new("ignore-binary");
    let repo = sandbox.mkdir("repo");
    let git_init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .output()
        .expect("run git init");
    assert!(
        git_init.status.success(),
        "git init should succeed: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );

    let needle = "NEEDLE_M7_E";
    fs::write(repo.join(".gitignore"), "node_modules/\n").expect("write .gitignore");
    fs::write(repo.join("find.txt"), format!("{needle}\n")).expect("write tracked file");
    fs::create_dir_all(repo.join("node_modules")).expect("create ignored dir");
    fs::write(repo.join("node_modules/dep.txt"), format!("{needle}\n"))
        .expect("write ignored file");
    fs::write(repo.join("blob.bin"), format!("abc\0{needle}\0xyz")).expect("write binary file");

    let out = run(sandbox.command_in(&repo, &[needle]));
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    let stdout = stdout_str(&out);
    assert!(
        stdout.contains("find.txt"),
        "tracked text file should be returned: {stdout}"
    );
    assert!(
        !stdout.contains("node_modules/dep.txt"),
        "ignored file should not be returned: {stdout}"
    );
    assert!(
        !stdout.contains("blob.bin"),
        "binary match should be skipped: {stdout}"
    );

    assert_eq!(
        stdout_lines(&out),
        rg_lines(&repo, needle),
        "cortex search should match ripgrep defaults for ignore/binary behavior"
    );
}
