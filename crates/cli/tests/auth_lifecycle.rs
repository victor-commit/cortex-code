use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "cortex-cli-auth-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = home.join("project");
        let data_dir = home.join("data");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&project).expect("create project");
        Self {
            home,
            project,
            data_dir,
        }
    }

    fn auth_file(&self) -> PathBuf {
        self.data_dir.join("auth.json")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn fake_key(tag: &str) -> String {
    format!("sk-{}-{}{}", "or-v1", tag, "abcd1234".repeat(6))
}

fn cortex(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("XDG_CONFIG_HOME", sandbox.home.join(".config"));
    command.env("CORTEX_DATA_DIR", &sandbox.data_dir);
    command.env_remove("OPENROUTER_API_KEY");
    command.env_remove("CORTEX_AUTH_FILE");
    command
}

fn run(mut command: Command, stdin_text: Option<&str>, timeout: Duration) -> Output {
    command.stdin(if stdin_text.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    if let Some(stdin_text) = stdin_text {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin
            .write_all(stdin_text.as_bytes())
            .expect("write stdin payload");
        stdin.flush().expect("flush stdin payload");
    }

    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command did not terminate within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

fn no_panic(stderr: &str) {
    assert!(
        !stderr.contains("panicked at") && !stderr.contains("RUST_BACKTRACE"),
        "must not panic: {stderr}"
    );
}

#[test]
fn auth_login_with_flag_writes_0600_store_and_list_shows_provider_without_secret() {
    let sandbox = Sandbox::new("flag");
    let key = fake_key("flag");

    let login = run(
        cortex(
            &sandbox,
            &[
                "auth",
                "login",
                "openrouter",
                "--api-key",
                &key,
                "--format",
                "json",
            ],
        ),
        None,
        Duration::from_secs(10),
    );
    assert_eq!(login.status.code(), Some(0));
    let login_stdout = String::from_utf8_lossy(&login.stdout);
    let login_stderr = String::from_utf8_lossy(&login.stderr);
    assert!(!login_stdout.contains(&key), "stdout must redact/omit key");
    assert!(!login_stderr.contains(&key), "stderr must redact/omit key");
    no_panic(&login_stderr);

    let auth_file = sandbox.auth_file();
    assert!(auth_file.exists(), "login must create the auth store file");
    assert!(
        !auth_file.starts_with(&sandbox.project),
        "auth store must live outside the project working tree"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(&auth_file)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "auth store must be 0600");
    }

    let list = run(
        cortex(&sandbox, &["auth", "list"]),
        None,
        Duration::from_secs(10),
    );
    assert_eq!(list.status.code(), Some(0));
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        list_stdout.lines().any(|line| line.trim() == "openrouter"),
        "auth list should include openrouter after login: {list_stdout}"
    );
    assert!(
        !list_stdout.contains(&key),
        "auth list must never print the secret value"
    );

    let list_json = run(
        cortex(&sandbox, &["auth", "list", "--format", "json"]),
        None,
        Duration::from_secs(10),
    );
    assert_eq!(list_json.status.code(), Some(0));
    let document: serde_json::Value =
        serde_json::from_slice(&list_json.stdout).expect("auth list json");
    assert_eq!(
        document["providers"][0]["id"].as_str(),
        Some("openrouter"),
        "json list should contain the stored provider id"
    );
}

#[test]
fn auth_login_accepts_env_or_stdin_and_logout_removes_the_stored_key() {
    let sandbox = Sandbox::new("env-stdin");
    let env_key = fake_key("env");
    let stdin_key = fake_key("stdin");

    let mut env_login = cortex(&sandbox, &["auth", "login", "openrouter"]);
    env_login.env("OPENROUTER_API_KEY", &env_key);
    let env_login = run(env_login, None, Duration::from_secs(10));
    assert_eq!(
        env_login.status.code(),
        Some(0),
        "env-based login should succeed"
    );
    assert!(
        !String::from_utf8_lossy(&env_login.stdout).contains(&env_key),
        "env login output must not leak the key"
    );

    let stdin_login = run(
        cortex(
            &sandbox,
            &["auth", "login", "openrouter", "--format", "json"],
        ),
        Some(&format!("{stdin_key}\n")),
        Duration::from_secs(10),
    );
    assert_eq!(
        stdin_login.status.code(),
        Some(0),
        "stdin-based login should succeed"
    );
    assert!(
        !String::from_utf8_lossy(&stdin_login.stdout).contains(&stdin_key),
        "stdin login output must not leak the key"
    );

    let logout = run(
        cortex(&sandbox, &["auth", "logout", "openrouter"]),
        None,
        Duration::from_secs(10),
    );
    assert_eq!(logout.status.code(), Some(0));
    let logout_stdout = String::from_utf8_lossy(&logout.stdout);
    assert!(
        logout_stdout.contains("removed stored credential"),
        "logout should report a successful removal: {logout_stdout}"
    );

    let list_after = run(
        cortex(&sandbox, &["auth", "list"]),
        None,
        Duration::from_secs(10),
    );
    assert_eq!(list_after.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&list_after.stdout).contains("openrouter"),
        "provider should be absent from auth list after logout"
    );

    let auth_file = sandbox.auth_file();
    if auth_file.exists() {
        let body = std::fs::read_to_string(&auth_file).expect("read auth file");
        assert!(
            !body.contains(&stdin_key) && !body.contains(&env_key),
            "removed provider key must be absent from the auth file"
        );
    }
}

#[test]
fn auth_logout_unknown_provider_is_clean_and_deterministic() {
    let sandbox = Sandbox::new("unknown");

    let first = run(
        cortex(&sandbox, &["auth", "logout", "no-such-provider"]),
        None,
        Duration::from_secs(10),
    );
    let second = run(
        cortex(&sandbox, &["auth", "logout", "no-such-provider"]),
        None,
        Duration::from_secs(10),
    );

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(
        first.stdout, second.stdout,
        "unknown-provider logout should be deterministic across repeats"
    );
    let output = String::from_utf8_lossy(&first.stdout);
    assert!(
        output.contains("not logged in") || output.contains("nothing to remove"),
        "unknown-provider logout should be a clear no-op message: {output}"
    );
    no_panic(&String::from_utf8_lossy(&first.stderr));
    no_panic(&String::from_utf8_lossy(&second.stderr));
}
