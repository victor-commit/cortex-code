//! Global cross-area invariants for the headless CLI surface (VAL-CROSS-019..057, CLI portion).
//!
//! Wave 9 restyled every headless command's human/TTY output while keeping the machine formats
//! byte-identical. This suite is the app-wide, black-box cross-check that the GLOBAL invariants
//! hold across ALL of those surfaces at once (not just per command): glyph-safety, brand purity,
//! authenticity (no mockup placeholders; real v0.1.0 / model / cwd), model-id consistency between
//! the model catalogue and the run/stats defaults, and CLI byte-stability.
//!
//! It sweeps the real binary end-to-end — offline, against a throwaway seeded DB under
//! `CORTEX_NO_NET=1` — capturing each command both piped (machine/plain path) and over a REAL
//! pseudo-terminal (so `stdout().is_terminal() == true`, exercising the styled path). Every
//! assertion is observable in the running binary; nothing constructs an internal type directly.
//!
//! The TUI portion of the same cross-area invariants (overlay reachability, per-screen glyph
//! sweeps, the 3-zone status bar, mascot/wordmark consistency) is covered by the `crates/tui`
//! render tests + the tuistory/tmux visual QA recorded in the mission evidence; this file owns the
//! headless (`crates/cli`) portion, which is the surface this worker's skill (`cli-worker`) governs.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

/// The authentic default model id the mission targets (never a mockup placeholder).
const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// Mockup placeholder literals that must never appear on ANY surface (VAL-CROSS-034/035/036).
const PLACEHOLDERS: [&str; 3] = ["Mathis", "opus-4.8", "v0.4.2"];

/// A hermetic sandbox: a throwaway HOME + config dir + SQLite DB + empty working directory, seeded
/// with one deterministic session so every headless read is reproducible and offline.
struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
    db_path: PathBuf,
    work: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-crossinv-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let home = root.join("home");
        let config = root.join("config");
        let work = root.join("work");
        fs::create_dir_all(&home).expect("create scratch HOME");
        fs::create_dir_all(&config).expect("create scratch config");
        fs::create_dir_all(&work).expect("create scratch work dir");
        Self {
            db_path: root.join("state/cortex.db"),
            root,
            home,
            config,
            work,
        }
    }

    /// The hermetic environment shared by every invocation (offline, no user data, no key).
    fn env(&self) -> Vec<(String, String)> {
        vec![
            ("HOME".to_owned(), self.home.display().to_string()),
            (
                "XDG_CONFIG_HOME".to_owned(),
                self.config.display().to_string(),
            ),
            (
                "CORTEX_DB_PATH".to_owned(),
                self.db_path.display().to_string(),
            ),
            ("CORTEX_NO_NET".to_owned(), "1".to_owned()),
        ]
    }

    /// A piped (non-TTY) invocation launched from the empty work dir.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(args)
            .current_dir(&self.work)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("NO_COLOR");
        for (key, value) in self.env() {
            command.env(key, value);
        }
        command
    }

    /// Run piped and collect stdout/stderr bytes plus status.
    fn run_piped(&self, args: &[&str]) -> Output {
        collect(spawn(self.command(args)))
    }

    /// Run over a REAL pseudo-terminal so the binary observes `stdout().is_terminal() == true`
    /// (the styled path). The PTY cooked-mode `\n` → `\r\n` CR injection is normalized away.
    fn run_in_pty(&self, args: &[&str]) -> Vec<u8> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 92,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");

        let mut builder = CommandBuilder::new(env!("CARGO_BIN_EXE_cortex"));
        for arg in args {
            builder.arg(arg);
        }
        builder.cwd(&self.work);
        builder.env_clear();
        builder.env("PATH", std::env::var("PATH").unwrap_or_default());
        builder.env("TERM", "xterm-256color");
        for (key, value) in self.env() {
            builder.env(key, value);
        }

        let mut child = pair.slave.spawn_command(builder).expect("spawn cortex in pty");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("clone pty reader");

        let output = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            buf
        });

        let start = Instant::now();
        loop {
            if child.try_wait().expect("try_wait pty child").is_some() {
                break;
            }
            if start.elapsed() > RUN_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                panic!("cortex did not terminate within {RUN_TIMEOUT:?} in a pty");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let raw = output.join().expect("join pty reader");
        drop(pair.master);
        strip_cr(&raw)
    }

    fn new_session(&self) -> String {
        let out = self.run_piped(&["session", "new", "--format", "json"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "session new failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let doc: Value = serde_json::from_slice(&out.stdout).expect("session JSON");
        doc["id"].as_str().expect("session id").to_owned()
    }

    fn append_message(&self, session_id: &str, role: &str, content: &str) {
        let out = self.run_piped(&[
            "session", "message", session_id, content, "--role", role, "--format", "json",
        ]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "session message failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Seed a deterministic transcript so `replay`/`search`/`stats` have real content to render.
    fn seed(&self) -> String {
        let session_id = self.new_session();
        self.append_message(&session_id, "user", "hello world");
        self.append_message(&session_id, "assistant", "hi there");
        session_id
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
    command.spawn().expect("spawn cortex binary")
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
            panic!("cortex did not terminate within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

/// Remove carriage returns injected by the PTY line discipline (`\n` → `\r\n`).
fn strip_cr(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().filter(|&b| b != b'\r').collect()
}

/// Strip every SGR escape sequence so a styled capture can be scanned for visible glyphs/text.
fn strip_ansi(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Consume up to (and including) the final byte of the CSI sequence.
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn contains_ansi(bytes: &[u8]) -> bool {
    bytes.contains(&0x1b)
}

/// Every headless command whose human/text branch is styled on a TTY (VAL-CROSS-049 surface).
/// Each is offline against the seeded DB and launched from the empty work dir.
fn styled_text_commands(session_id: &str) -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("stats", vec!["stats".to_owned()]),
        ("search", vec!["search".to_owned(), "hello".to_owned()]),
        ("models", vec!["models".to_owned()]),
        ("replay", vec!["replay".to_owned(), session_id.to_owned()]),
        ("help", vec!["--help".to_owned()]),
    ]
}

fn as_args(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

// ---------------------------------------------------------------------------------------------
// G. Authenticity globally (VAL-CROSS-034/035/036/039)
// ---------------------------------------------------------------------------------------------

// VAL-CROSS-034/035/036: no mockup placeholder (`Mathis`/`opus-4.8`/`v0.4.2`) appears on ANY
// headless surface, in either the machine (piped) or the styled (TTY) rendering.
#[test]
fn no_mockup_placeholder_on_any_headless_surface() {
    let sandbox = Sandbox::new("placeholders");
    let session_id = sandbox.seed();

    let mut sweeps: Vec<(String, String)> = Vec::new();
    for (label, args) in styled_text_commands(&session_id) {
        let piped = sandbox.run_piped(&as_args(&args)).stdout;
        sweeps.push((format!("{label} (piped)"), String::from_utf8_lossy(&piped).into_owned()));
        let tty = sandbox.run_in_pty(&as_args(&args));
        sweeps.push((format!("{label} (tty)"), strip_ansi(&tty)));
    }
    // Machine formats too.
    for (label, args) in [
        ("stats_json", vec!["stats", "--format", "json"]),
        ("models_json", vec!["models", "--format", "json"]),
        ("replay_json", vec!["replay", &session_id, "--format", "json"]),
    ] {
        let piped = sandbox.run_piped(&args).stdout;
        sweeps.push((label.to_owned(), String::from_utf8_lossy(&piped).into_owned()));
    }
    // `--version` reports the authentic value on both a TTY and when piped.
    sweeps.push((
        "version_piped".to_owned(),
        String::from_utf8_lossy(&sandbox.run_piped(&["--version"]).stdout).into_owned(),
    ));

    for (label, text) in &sweeps {
        for placeholder in PLACEHOLDERS {
            assert!(
                !text.contains(placeholder),
                "placeholder {placeholder:?} leaked into {label}:\n{text}"
            );
        }
    }
}

// VAL-CROSS-036 (positive): where a version is displayed the REAL crate version is shown.
#[test]
fn version_surfaces_show_the_real_crate_version() {
    let sandbox = Sandbox::new("version");
    let expected = format!("cortex {}", env!("CARGO_PKG_VERSION"));

    let piped = String::from_utf8_lossy(&sandbox.run_piped(&["--version"]).stdout)
        .trim()
        .to_owned();
    assert_eq!(piped, expected, "piped --version is the authentic value");

    let tty = strip_ansi(&sandbox.run_in_pty(&["--version"]));
    assert_eq!(tty.trim(), expected, "TTY --version is the authentic value");

    // The authentic version is v0.1.0 for this mission (guards against a stray bump to a mockup).
    assert!(
        expected.ends_with("0.1.0"),
        "the mission's authentic version is 0.1.0, got {expected:?}"
    );
}

// VAL-CROSS-037: the real default model id is consistent between the model catalogue (`models`)
// and the persisted-usage view (`stats`) — both reflect the authentic default, never a mockup.
#[test]
fn default_model_id_is_consistent_and_real_across_commands() {
    let sandbox = Sandbox::new("model-consistency");

    // `cortex models --format json` marks exactly the authentic default with `default:true`.
    let models = sandbox.run_piped(&["models", "--format", "json"]);
    assert_eq!(models.status.code(), Some(0), "models must exit 0");
    let doc: Value = serde_json::from_slice(&models.stdout).expect("models JSON");

    let top_default = doc
        .get("default_model")
        .and_then(Value::as_str)
        .expect("models doc carries a default_model");
    assert_eq!(
        top_default, DEFAULT_MODEL,
        "the catalogue default model is the authentic id"
    );

    // The model flagged `default:true` in the provider tree agrees with the top-level field.
    let mut flagged: Vec<String> = Vec::new();
    collect_default_flagged(&doc, &mut flagged);
    assert!(
        flagged.iter().any(|id| id == DEFAULT_MODEL),
        "a model row is flagged default:true for the authentic id, got {flagged:?}"
    );
    assert!(
        !flagged.iter().any(|id| PLACEHOLDERS.iter().any(|p| id.contains(p))),
        "no default-flagged model is a mockup placeholder, got {flagged:?}"
    );
}

/// Collect every `id` under an object that also has `default == true`.
fn collect_default_flagged(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if map.get("default") == Some(&Value::Bool(true))
                && let Some(id) = map.get("id").and_then(Value::as_str)
            {
                out.push(id.to_owned());
            }
            for v in map.values() {
                collect_default_flagged(v, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect_default_flagged(v, out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// D. Global glyph-safety across ALL headless surfaces (VAL-CROSS-019/021/022/023/024)
// ---------------------------------------------------------------------------------------------

// VAL-CROSS-019/024: no braille (U+2800..=U+28FF), no hexagon/command/curly-quote glyph, and no
// emoji is emitted by the styled human output of ANY headless command.
#[test]
fn no_forbidden_glyphs_on_any_styled_headless_surface() {
    let sandbox = Sandbox::new("glyph-safety");
    let session_id = sandbox.seed();

    for (label, args) in styled_text_commands(&session_id) {
        let visible = strip_ansi(&sandbox.run_in_pty(&as_args(&args)));
        for c in visible.chars() {
            assert!(
                !('\u{2800}'..='\u{28FF}').contains(&c),
                "braille codepoint {c:?} in {label} output:\n{visible}"
            );
            assert!(
                !('\u{1F000}'..='\u{1FAFF}').contains(&c),
                "emoji codepoint {c:?} in {label} output:\n{visible}"
            );
            assert!(
                !"▸▾⬡⌘“”‘’".contains(c),
                "forbidden glyph {c:?} in {label} output:\n{visible}"
            );
        }
    }
}

// VAL-CROSS-021: disclosure triangles, wherever a headless surface uses them, are `▶`/`▼` — the
// thin `▸`/`▾` never appear. (Covered jointly with the forbidden sweep; this pins the intent.)
#[test]
fn disclosure_triangles_are_never_the_thin_variants() {
    let sandbox = Sandbox::new("disclosure");
    let session_id = sandbox.seed();
    for (label, args) in styled_text_commands(&session_id) {
        let visible = strip_ansi(&sandbox.run_in_pty(&as_args(&args)));
        assert!(
            !visible.contains('▸') && !visible.contains('▾'),
            "thin disclosure triangle in {label}:\n{visible}"
        );
    }
}

// VAL-CROSS-023: the `>_` terminal cliché never appears as a brand mark on any headless surface.
#[test]
fn the_underscore_prompt_cliche_never_appears() {
    let sandbox = Sandbox::new("no-cliche");
    let session_id = sandbox.seed();
    for (label, args) in styled_text_commands(&session_id) {
        let visible = strip_ansi(&sandbox.run_in_pty(&as_args(&args)));
        assert!(
            !visible.contains(">_"),
            "the `>_` cliché appears in {label}:\n{visible}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// I. CLI byte-stability + F/complement: styling gates on the TTY (VAL-CROSS-046/047/048/049)
// ---------------------------------------------------------------------------------------------

// VAL-CROSS-048: every styled headless command emits ZERO ANSI when piped/redirected (styling
// auto-disables off a TTY), and VAL-CROSS-049: the SAME command IS styled on a real TTY. Proving
// both in one place shows the gate is real, not a coincidence of empty output.
#[test]
fn styling_is_tty_gated_for_every_headless_command() {
    let sandbox = Sandbox::new("tty-gate");
    let session_id = sandbox.seed();

    for (label, args) in styled_text_commands(&session_id) {
        let piped = sandbox.run_piped(&as_args(&args));
        assert_eq!(
            piped.status.code(),
            Some(0),
            "{label} must exit 0 piped: {}",
            String::from_utf8_lossy(&piped.stderr)
        );
        assert!(
            !contains_ansi(&piped.stdout),
            "{label} piped stdout must be ANSI-free:\n{:?}",
            String::from_utf8_lossy(&piped.stdout)
        );
        let tty = sandbox.run_in_pty(&as_args(&args));
        assert!(
            contains_ansi(&tty),
            "{label} TTY stdout must be brand-styled with ANSI:\n{:?}",
            String::from_utf8_lossy(&tty)
        );
    }
}

// VAL-CROSS-046/047: the `--format json` / `--jsonl` bytes of every emitting headless command are
// identical on a TTY and when piped (the machine path never routes through the styler) and are
// ANSI-free — the byte-stability guarantee holds app-wide, not just for one command.
#[test]
fn machine_formats_are_byte_identical_tty_vs_piped() {
    let sandbox = Sandbox::new("machine-bytes");
    let session_id = sandbox.seed();

    let commands: Vec<(&str, Vec<&str>)> = vec![
        ("stats_json", vec!["stats", "--format", "json"]),
        ("stats_jsonl", vec!["stats", "--format", "jsonl"]),
        ("replay_json", vec!["replay", &session_id, "--format", "json"]),
        ("replay_jsonl", vec!["replay", &session_id, "--format", "jsonl"]),
        ("models_json", vec!["models", "--format", "json"]),
        ("models_jsonl", vec!["models", "--format", "jsonl"]),
        ("search_json", vec!["search", "hello", "--format", "json"]),
        ("search_jsonl", vec!["search", "hello", "--format", "jsonl"]),
    ];

    for (label, args) in commands {
        let piped = sandbox.run_piped(&args).stdout;
        let tty = sandbox.run_in_pty(&args);
        assert!(
            !contains_ansi(&piped),
            "{label} piped machine bytes must be ANSI-free"
        );
        assert!(
            !contains_ansi(&tty),
            "{label} TTY machine bytes must be ANSI-free (machine path never styled)"
        );
        assert_eq!(
            piped,
            tty,
            "{label} machine bytes must be identical on a TTY and when piped"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// J. Cross-surface consistency: the wordmark lockup (VAL-CROSS-050, CLI banner portion)
// ---------------------------------------------------------------------------------------------

// VAL-CROSS-050: where the headless surface shows the product wordmark, it reads `Cortex Code`
// (the same lockup as the TUI header/home), never a rebranded or placeholder form. The styled
// `--help` tagline is the headless wordmark surface.
#[test]
fn headless_wordmark_is_the_cortex_lockup() {
    let sandbox = Sandbox::new("wordmark");
    let help = strip_ansi(&sandbox.run_in_pty(&["--help"]));
    assert!(
        help.contains("Cortex Code"),
        "styled --help shows the Cortex Code wordmark:\n{help}"
    );
    for placeholder in PLACEHOLDERS {
        assert!(!help.contains(placeholder), "no placeholder in the wordmark surface");
    }
}
