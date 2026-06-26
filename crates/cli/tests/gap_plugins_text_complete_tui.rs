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

fn load_hooks(sandbox: &Sandbox) -> plugins::PluginEventHooks {
    let plugin = sandbox.project.join("plugin.mjs");
    fs::write(
        &plugin,
        r#"
export default async function plugin() {
  return {
    experimental: {
      text: {
        complete({ text, cursor }) {
          const beforeCursor = text.slice(0, cursor);
          const token = beforeCursor.match(/(^|\s)(\S*)$/)?.[2] ?? "";
          if (token.startsWith("hel")) {
            return ["hello"];
          }
          if (token.startsWith("wor")) {
            return ["world"];
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
    plugins::PluginEventHooks::load_or_disabled(&plugin_config, false, "tui")
        .expect("load plugin hooks")
}

fn move_cursor_to(app: &mut App, target: usize) {
    app.on_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    for _ in 0..target {
        app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    }
}

fn trigger_plugin_completion(app: &mut App) {
    app.on_key(KeyEvent::new(KeyCode::Char('\t'), KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
}

#[test]
fn tab_completion_replaces_only_active_span_across_prompt_positions() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("app-text-complete");
    let hooks = load_hooks(&sandbox);

    let mut app = App::new(Status {
        model: "plugin/stream-model".to_owned(),
        agent: "build".to_owned(),
        mode: "build".to_owned(),
        variant: "default".to_owned(),
        session: "new".to_owned(),
    });
    app.set_plugin_text_complete(Some(hooks));
    let cases = [
        ("hel world", 3usize, "hello world"),
        ("say hel there", 7usize, "say hello there"),
        ("say hello wor", 13usize, "say hello world"),
    ];
    for (input, cursor, expected) in cases {
        app.set_input_value(input);
        move_cursor_to(&mut app, cursor);
        trigger_plugin_completion(&mut app);
        assert_eq!(app.input_value(), expected);
    }
}

#[test]
fn editing_prompt_clears_stale_pending_plugin_completion() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("app-text-complete-stale");
    let hooks = load_hooks(&sandbox);

    let mut app = App::new(Status {
        model: "plugin/stream-model".to_owned(),
        agent: "build".to_owned(),
        mode: "build".to_owned(),
        variant: "default".to_owned(),
        session: "new".to_owned(),
    });
    app.set_plugin_text_complete(Some(hooks));
    app.set_input_value("hel world");
    move_cursor_to(&mut app, 3);

    app.on_key(KeyEvent::new(KeyCode::Char('\t'), KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    assert_eq!(app.input_value(), "helx world");
}
