//! Pure Cortex F1–F3 screens (auth + conversation + tools).
//! Copper brand helpers are forbidden in new paints.

pub mod auth;
pub mod chat_turn;
pub mod conversation;
pub mod login;
pub mod tools;

use crate::chrome::canvas::Canvas;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PilotSpec {
    pub id: &'static str,
    pub category: &'static str,
}

/// Ordered F1–F3 pure Cortex dump inventory for this batch.
pub const PILOT_SCREENS: [PilotSpec; 27] = [
    // F1 auth (7)
    PilotSpec {
        id: "login",
        category: "auth",
    },
    PilotSpec {
        id: "connect-provider",
        category: "auth",
    },
    PilotSpec {
        id: "connect-method",
        category: "auth",
    },
    PilotSpec {
        id: "connect-oauth",
        category: "auth",
    },
    PilotSpec {
        id: "connect-apikey",
        category: "auth",
    },
    PilotSpec {
        id: "connect-connected",
        category: "auth",
    },
    PilotSpec {
        id: "connect-disconnect",
        category: "auth",
    },
    // F2 conversation/session toolkit (9)
    PilotSpec {
        id: "home",
        category: "session",
    },
    PilotSpec {
        id: "chat-turn",
        category: "session",
    },
    PilotSpec {
        id: "chat-long-scroll",
        category: "session",
    },
    PilotSpec {
        id: "streaming",
        category: "session",
    },
    PilotSpec {
        id: "reasoning",
        category: "session",
    },
    PilotSpec {
        id: "reasoning-collapsed",
        category: "session",
    },
    PilotSpec {
        id: "rich-response",
        category: "session",
    },
    PilotSpec {
        id: "empty",
        category: "states",
    },
    PilotSpec {
        id: "toasts",
        category: "states",
    },
    // F3 tools & tasks (11)
    PilotSpec {
        id: "tool-calls",
        category: "session",
    },
    PilotSpec {
        id: "tool-running",
        category: "session",
    },
    PilotSpec {
        id: "tool-success",
        category: "session",
    },
    PilotSpec {
        id: "tool-success-expanded",
        category: "session",
    },
    PilotSpec {
        id: "tool-failed",
        category: "session",
    },
    PilotSpec {
        id: "tool-task",
        category: "session",
    },
    PilotSpec {
        id: "tool-task-expanded",
        category: "session",
    },
    PilotSpec {
        id: "shell",
        category: "session",
    },
    PilotSpec {
        id: "todo-list",
        category: "session",
    },
    PilotSpec {
        id: "compact",
        category: "session",
    },
    PilotSpec {
        id: "diff-edit",
        category: "session",
    },
];

pub fn render_pilot(id: &str) -> Result<Canvas, String> {
    match id {
        // Auth — includes login via auth::render
        "login"
        | "connect-provider"
        | "connect-method"
        | "connect-oauth"
        | "connect-apikey"
        | "connect-connected"
        | "connect-disconnect" => auth::render(id),
        // Conversation
        "home"
        | "chat-turn"
        | "chat-long-scroll"
        | "streaming"
        | "reasoning"
        | "reasoning-collapsed"
        | "rich-response"
        | "empty"
        | "toasts" => conversation::render(id),
        // Tools
        "tool-calls"
        | "tool-running"
        | "tool-success"
        | "tool-success-expanded"
        | "tool-failed"
        | "tool-task"
        | "tool-task-expanded"
        | "shell"
        | "todo-list"
        | "compact"
        | "diff-edit" => tools::render(id),
        other => Err(format!("unknown pure-Cortex pilot id: {other}")),
    }
}

pub fn list_pilots() -> &'static [PilotSpec] {
    &PILOT_SCREENS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::components::reject_copper_residual;

    #[test]
    fn f1_f3_inventory_count_and_render() {
        assert_eq!(list_pilots().len(), 27);
        for spec in list_pilots() {
            let canvas =
                render_pilot(spec.id).unwrap_or_else(|e| panic!("{}: {e}", spec.id));
            let text = canvas.to_text();
            reject_copper_residual(&text)
                .unwrap_or_else(|e| panic!("{}: {e}", spec.id));
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), 40, "row count for {}", spec.id);
            let bad = lines.iter().any(|l| l.chars().count() != 120);
            assert!(!bad, "row width for {}", spec.id);
        }
    }
}
