use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui::{App, Status};

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping text-complete TUI gap test: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

struct Sandbox {
    root: PathBuf,
    project: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-plugin-text-complete-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("project");
        fs::create_dir_all(&project).expect("create project");
        Self { root, project }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn tab_triggers_plugin_experimental_text_complete_and_inserts_candidate() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("app-text-complete");
    let plugin = sandbox.project.join("plugin.mjs");
    fs::write(
        &plugin,
        r#"
export default async function plugin() {
  return {
    experimental: {
      text: {
        complete({ text }) {
          if (text.startsWith("hel")) {
            return ["hello from plugin"];
          }
          return [];
        }
      }
    }
  };
}
"#,
    )
    .expect("write plugin");

    let config: toml::Value =
        toml::from_str(&format!(r#"plugin = [[ "{}", {{}} ]]"#, plugin.display()))
            .expect("parse plugin config");
    let plugin_config =
        plugins::PluginConfig::from_toml_value(config.get("plugin"), &sandbox.project)
            .expect("plugin config");
    let hooks = plugins::PluginEventHooks::load_or_disabled(&plugin_config, false, "tui")
        .expect("load plugin hooks");

    let mut app = App::new(Status {
        model: "plugin/stream-model".to_owned(),
        agent: "build".to_owned(),
        mode: "build".to_owned(),
        session: "new".to_owned(),
    });
    app.set_plugin_text_complete(Some(hooks));
    app.set_input_value("hel");

    app.on_key(KeyEvent::new(KeyCode::Char('\t'), KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    assert_eq!(app.input_value(), "hello from plugin");
}
