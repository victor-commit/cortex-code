//! Keybind system for the TUI: leader-key chords, the actions they trigger, and user overrides
//! loaded from the `[keybinds]` config table.
//!
//! A binding is a *leader chord* (default `Ctrl+G`) followed by a single *action key*. Pressing the
//! leader arms a pending state (the which-key overlay lists the active bindings); the next key
//! either runs the matching action or cancels. The default bindings and the leader are both
//! overridable from config, so a user can remap any action key or change the leader entirely.

use crossterm::event::{KeyCode, KeyModifiers};
use toml::value::Table;

/// An action a leader binding can trigger. Each maps to a visible TUI effect (a fresh session, a
/// picker/dialog, or a panel toggle) and carries the metadata the which-key overlay and config
/// override loader need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeybindAction {
    /// Start a fresh session (clears the transcript), like `/new`.
    NewSession,
    /// Open the model picker, like `/models`.
    ModelList,
    /// Show or hide the session sidebar panel.
    ToggleSidebar,
    /// Open the session picker, like `/sessions`.
    Sessions,
    /// Open the agent picker, like `/agents`.
    Agents,
    /// Open the theme picker, like `/themes`.
    Themes,
    /// Undo the last file-affecting change, like `/undo`.
    Undo,
    /// Redo the last undone change, like `/redo`.
    Redo,
}

impl KeybindAction {
    /// Every action, in the order they appear in the which-key overlay and default bindings.
    pub const ALL: &'static [KeybindAction] = &[
        KeybindAction::NewSession,
        KeybindAction::ModelList,
        KeybindAction::ToggleSidebar,
        KeybindAction::Sessions,
        KeybindAction::Agents,
        KeybindAction::Themes,
        KeybindAction::Undo,
        KeybindAction::Redo,
    ];

    /// The config key under `[keybinds]` that overrides this action's key (e.g. `new_session`).
    #[must_use]
    pub fn config_key(self) -> &'static str {
        match self {
            KeybindAction::NewSession => "new_session",
            KeybindAction::ModelList => "model_list",
            KeybindAction::ToggleSidebar => "toggle_sidebar",
            KeybindAction::Sessions => "sessions",
            KeybindAction::Agents => "agents",
            KeybindAction::Themes => "themes",
            KeybindAction::Undo => "undo",
            KeybindAction::Redo => "redo",
        }
    }

    /// A short human-readable label shown beside the key in the which-key overlay and `/help`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            KeybindAction::NewSession => "New session",
            KeybindAction::ModelList => "Open the model picker",
            KeybindAction::ToggleSidebar => "Toggle the sidebar",
            KeybindAction::Sessions => "Open the session picker",
            KeybindAction::Agents => "Open the agent picker",
            KeybindAction::Themes => "Open the theme picker",
            KeybindAction::Undo => "Undo the last change",
            KeybindAction::Redo => "Redo the last undone change",
        }
    }

    /// This action's default (unconfigured) action key.
    #[must_use]
    fn default_key(self) -> KeyChord {
        let ch = match self {
            KeybindAction::NewSession => 'n',
            KeybindAction::ModelList => 'm',
            KeybindAction::ToggleSidebar => 'b',
            KeybindAction::Sessions => 's',
            KeybindAction::Agents => 'a',
            KeybindAction::Themes => 't',
            KeybindAction::Undo => 'u',
            KeybindAction::Redo => 'r',
        };
        KeyChord::plain(ch)
    }
}

/// A single key press, possibly with modifiers, used both for the leader and for action keys.
///
/// Alphabetic characters are normalized to lowercase and never carry an explicit `shift` (case is
/// already encoded in the character), so a configured `"n"` matches the `n` key whether or not the
/// terminal reports a shift modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    /// The base key.
    pub code: KeyCode,
    /// Whether Ctrl is held.
    pub ctrl: bool,
    /// Whether Alt is held.
    pub alt: bool,
    /// Whether Shift is held (never set for normalized alphabetic characters).
    pub shift: bool,
}

impl KeyChord {
    /// A plain (unmodified) character chord.
    #[must_use]
    fn plain(ch: char) -> KeyChord {
        KeyChord {
            code: KeyCode::Char(ch.to_ascii_lowercase()),
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// Build a chord from a live key event, applying the alphabetic normalization so it can be
    /// compared against parsed/configured chords.
    #[must_use]
    pub fn from_event(code: KeyCode, mods: KeyModifiers) -> KeyChord {
        let ctrl = mods.contains(KeyModifiers::CONTROL);
        let alt = mods.contains(KeyModifiers::ALT);
        let shift = mods.contains(KeyModifiers::SHIFT);
        match code {
            KeyCode::Char(c) if c.is_ascii_alphabetic() => KeyChord {
                code: KeyCode::Char(c.to_ascii_lowercase()),
                ctrl,
                alt,
                shift: false,
            },
            other => KeyChord {
                code: other,
                ctrl,
                alt,
                shift,
            },
        }
    }

    /// Parse a chord from a config string such as `"n"`, `"ctrl+g"`, `"alt+space"`, `"f1"`, or
    /// `"esc"`. Returns `None` for an empty or unrecognized specification.
    ///
    /// Tokens are `+`-separated and case-insensitive: any number of `ctrl`/`alt`/`shift`/`meta`
    /// modifiers followed by exactly one base key (a single character, or a named key like
    /// `space`/`tab`/`enter`/`esc`/`f1`..`f12`).
    #[must_use]
    pub fn parse(spec: &str) -> Option<KeyChord> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut base: Option<KeyCode> = None;

        for token in spec.split('+') {
            let token = token.trim();
            if token.is_empty() {
                return None;
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" | "option" => alt = true,
                "shift" => shift = true,
                // `meta`/`cmd`/`super` are accepted and folded into Alt for portability.
                "meta" | "cmd" | "super" => alt = true,
                other => {
                    if base.is_some() {
                        return None;
                    }
                    base = Some(parse_base_key(other)?);
                }
            }
        }

        let code = base?;
        // Normalize alphabetic base keys: lowercase, shift implied by case.
        if let KeyCode::Char(c) = code {
            if c.is_ascii_alphabetic() {
                return Some(KeyChord {
                    code: KeyCode::Char(c.to_ascii_lowercase()),
                    ctrl,
                    alt,
                    shift: false,
                });
            }
        }
        Some(KeyChord {
            code,
            ctrl,
            alt,
            shift,
        })
    }

    /// A compact human-readable rendering of the chord (e.g. `Ctrl+G`, `n`, `Space`).
    #[must_use]
    pub fn display(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        match self.code {
            KeyCode::Char(' ') => out.push_str("Space"),
            KeyCode::Char(c) => out.push(c.to_ascii_uppercase()),
            KeyCode::Esc => out.push_str("Esc"),
            KeyCode::Tab => out.push_str("Tab"),
            KeyCode::Enter => out.push_str("Enter"),
            KeyCode::F(n) => {
                out.push('F');
                out.push_str(&n.to_string());
            }
            other => out.push_str(&format!("{other:?}")),
        }
        out
    }
}

/// Parse a base (non-modifier) key token into a [`KeyCode`].
fn parse_base_key(token: &str) -> Option<KeyCode> {
    match token {
        "space" => Some(KeyCode::Char(' ')),
        "tab" => Some(KeyCode::Tab),
        "enter" | "return" => Some(KeyCode::Enter),
        "esc" | "escape" => Some(KeyCode::Esc),
        "backspace" => Some(KeyCode::Backspace),
        "delete" | "del" => Some(KeyCode::Delete),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        _ => {
            if let Some(num) = token.strip_prefix('f') {
                if let Ok(n) = num.parse::<u8>() {
                    if (1..=12).contains(&n) {
                        return Some(KeyCode::F(n));
                    }
                }
            }
            let mut chars = token.chars();
            let first = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Some(KeyCode::Char(first))
        }
    }
}

/// The active keybind set: the leader chord plus the ordered action bindings shown in the which-key
/// overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybinds {
    /// The chord that arms a leader sequence (default `Ctrl+G`).
    pub leader: KeyChord,
    /// The action key for each action, in display order. Each action appears exactly once.
    bindings: Vec<(KeyChord, KeybindAction)>,
}

impl Default for Keybinds {
    fn default() -> Self {
        let leader = KeyChord::parse("ctrl+g").expect("default leader chord parses");
        let bindings = KeybindAction::ALL
            .iter()
            .map(|action| (action.default_key(), *action))
            .collect();
        Self { leader, bindings }
    }
}

impl Keybinds {
    /// The default keybinds with any `[keybinds]` config overrides applied. A missing or malformed
    /// override leaves that binding at its default, so the TUI always has a usable set.
    #[must_use]
    pub fn load() -> Self {
        let mut keybinds = Self::default();
        if let Some(table) = keybind_table() {
            keybinds.apply_overrides(&table);
        }
        keybinds
    }

    /// Apply a `[keybinds]` config table over the defaults: a `leader` entry replaces the leader,
    /// and any `<action.config_key>` entry replaces that action's key. Unparseable values are
    /// ignored (the default stays).
    pub fn apply_overrides(&mut self, table: &Table) {
        if let Some(spec) = table.get("leader").and_then(toml::Value::as_str) {
            if let Some(chord) = KeyChord::parse(spec) {
                self.leader = chord;
            }
        }
        for (chord, action) in &mut self.bindings {
            if let Some(spec) = table.get(action.config_key()).and_then(toml::Value::as_str) {
                if let Some(parsed) = KeyChord::parse(spec) {
                    *chord = parsed;
                }
            }
        }
    }

    /// The action bound to `chord` (an action key pressed after the leader), or `None` if none.
    #[must_use]
    pub fn action_for(&self, chord: KeyChord) -> Option<KeybindAction> {
        self.bindings
            .iter()
            .find(|(bound, _)| *bound == chord)
            .map(|(_, action)| *action)
    }

    /// The bindings in display order, as `(key, action)` pairs, for the which-key overlay / `/help`.
    #[must_use]
    pub fn entries(&self) -> &[(KeyChord, KeybindAction)] {
        &self.bindings
    }
}

/// Read the `[keybinds]` table from the effective configuration, or `None` when no config resolves
/// or the table is absent.
fn keybind_table() -> Option<Table> {
    let inputs = settings::ConfigInputs::from_process(std::collections::BTreeMap::new()).ok()?;
    let config = settings::load(inputs).ok()?;
    config
        .get("keybinds")
        .and_then(toml::Value::as_table)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(toml: &str) -> Table {
        toml::from_str(toml).expect("valid toml fixture")
    }

    #[test]
    fn parses_plain_letter_normalized_to_lowercase() {
        let chord = KeyChord::parse("N").unwrap();
        assert_eq!(chord, KeyChord::plain('n'));
        assert!(!chord.shift, "case is folded, not reported as shift");
    }

    #[test]
    fn parses_ctrl_chord() {
        let chord = KeyChord::parse("ctrl+g").unwrap();
        assert_eq!(chord.code, KeyCode::Char('g'));
        assert!(chord.ctrl);
        assert!(!chord.alt);
    }

    #[test]
    fn parses_named_keys() {
        assert_eq!(KeyChord::parse("space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(KeyChord::parse("esc").unwrap().code, KeyCode::Esc);
        assert_eq!(KeyChord::parse("f5").unwrap().code, KeyCode::F(5));
    }

    #[test]
    fn rejects_empty_and_multi_base() {
        assert!(KeyChord::parse("").is_none());
        assert!(KeyChord::parse("   ").is_none());
        assert!(KeyChord::parse("ctrl+a+b").is_none());
    }

    #[test]
    fn from_event_matches_parsed_chord() {
        // Pressing Ctrl+G yields the same chord the default leader parses to.
        let pressed = KeyChord::from_event(KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(pressed, KeyChord::parse("ctrl+g").unwrap());

        // A shifted letter still matches the lowercase configured key.
        let pressed_upper = KeyChord::from_event(KeyCode::Char('N'), KeyModifiers::SHIFT);
        assert_eq!(pressed_upper, KeyChord::parse("n").unwrap());
    }

    #[test]
    fn defaults_bind_the_required_actions() {
        let kb = Keybinds::default();
        assert_eq!(kb.leader, KeyChord::parse("ctrl+g").unwrap());
        assert_eq!(
            kb.action_for(KeyChord::plain('n')),
            Some(KeybindAction::NewSession)
        );
        assert_eq!(
            kb.action_for(KeyChord::plain('m')),
            Some(KeybindAction::ModelList)
        );
        assert_eq!(
            kb.action_for(KeyChord::plain('b')),
            Some(KeybindAction::ToggleSidebar)
        );
    }

    #[test]
    fn defaults_bind_undo_and_redo() {
        // VAL-TUI-087: undo/redo are reachable by keybind (leader + key), sharing the slash effect.
        let kb = Keybinds::default();
        assert_eq!(
            kb.action_for(KeyChord::plain('u')),
            Some(KeybindAction::Undo)
        );
        assert_eq!(
            kb.action_for(KeyChord::plain('r')),
            Some(KeybindAction::Redo)
        );
    }

    #[test]
    fn override_remaps_an_action_and_drops_the_default() {
        // VAL-TUI-069: a remapped action takes effect and the old default no longer triggers it.
        let mut kb = Keybinds::default();
        kb.apply_overrides(&table(r#"new_session = "x""#));
        assert_eq!(
            kb.action_for(KeyChord::plain('x')),
            Some(KeybindAction::NewSession),
            "the override key triggers the action"
        );
        assert_eq!(
            kb.action_for(KeyChord::plain('n')),
            None,
            "the default key no longer triggers it"
        );
    }

    #[test]
    fn override_can_change_the_leader() {
        let mut kb = Keybinds::default();
        kb.apply_overrides(&table(r#"leader = "ctrl+b""#));
        assert_eq!(kb.leader, KeyChord::parse("ctrl+b").unwrap());
    }

    #[test]
    fn malformed_override_keeps_the_default() {
        let mut kb = Keybinds::default();
        kb.apply_overrides(&table(r#"new_session = "ctrl+a+b""#));
        assert_eq!(
            kb.action_for(KeyChord::plain('n')),
            Some(KeybindAction::NewSession)
        );
    }

    #[test]
    fn display_is_human_readable() {
        assert_eq!(KeyChord::parse("ctrl+g").unwrap().display(), "Ctrl+G");
        assert_eq!(KeyChord::plain('n').display(), "N");
        assert_eq!(KeyChord::parse("space").unwrap().display(), "Space");
    }
}
