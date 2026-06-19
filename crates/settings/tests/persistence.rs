//! `persist_global_string` round-trip against a real, isolated global config file.
//!
//! Backs the durable preferences seam used by the TUI theme picker: a value written to the global
//! layer is read back by `load` on a later invocation, and unrelated keys already present in the
//! file survive the upsert.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use settings::{ConfigInputs, load, persist_global_string};

/// A self-cleaning scratch tree with an isolated HOME (so the test never touches the real one).
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-persist-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join("home")).expect("create home");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn global_config(&self) -> PathBuf {
        self.home().join(".config/cortex/config.toml")
    }

    fn inputs(&self) -> ConfigInputs {
        ConfigInputs {
            home: Some(self.home()),
            xdg_config_home: None,
            cwd: self.home(),
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// VAL-TUI-063: a persisted theme is read back on a later load (survives "restart").
#[test]
fn persisted_value_is_read_back_on_the_next_load() {
    let scratch = Scratch::new();

    let path =
        persist_global_string(&scratch.inputs(), "theme", "monokai").expect("persist succeeds");
    assert_eq!(path, scratch.global_config(), "writes to the global config");
    assert!(path.exists(), "the global config file is created");

    let config = load(scratch.inputs()).expect("config loads");
    assert_eq!(
        config.get("theme").and_then(|v| v.as_str()),
        Some("monokai"),
        "the persisted theme is observable after a fresh load"
    );
}

#[test]
fn persisting_preserves_other_keys_and_overwrites_the_same_key() {
    let scratch = Scratch::new();

    // A pre-existing key in the global file must survive the theme upsert.
    fs::create_dir_all(scratch.global_config().parent().unwrap()).unwrap();
    fs::write(
        &scratch.global_config(),
        "model = \"anthropic/claude-sonnet-4\"\n",
    )
    .unwrap();

    persist_global_string(&scratch.inputs(), "theme", "light").expect("persist theme");
    let config = load(scratch.inputs()).expect("load after first persist");
    assert_eq!(
        config.get("model").and_then(|v| v.as_str()),
        Some("anthropic/claude-sonnet-4"),
        "the unrelated model key is preserved"
    );
    assert_eq!(config.get("theme").and_then(|v| v.as_str()), Some("light"));

    // Re-persisting the same key overwrites it (a later selection wins).
    persist_global_string(&scratch.inputs(), "theme", "ocean_dark").expect("persist theme again");
    let config = load(scratch.inputs()).expect("load after second persist");
    assert_eq!(
        config.get("theme").and_then(|v| v.as_str()),
        Some("ocean_dark"),
        "the latest persisted theme wins"
    );
    assert_eq!(
        config.get("model").and_then(|v| v.as_str()),
        Some("anthropic/claude-sonnet-4"),
        "the unrelated key still survives the second write"
    );
}

#[test]
fn missing_config_home_is_a_clear_error() {
    let inputs = ConfigInputs {
        home: None,
        xdg_config_home: None,
        cwd: std::env::temp_dir(),
        env: Vec::new(),
        cli_overrides: BTreeMap::new(),
    };
    let error = persist_global_string(&inputs, "theme", "dark").expect_err("no config home");
    assert!(
        error.to_string().contains("global config"),
        "the error explains there is no global config location: {error}"
    );
}
