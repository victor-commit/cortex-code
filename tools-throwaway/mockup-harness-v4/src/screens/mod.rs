//! Pure Cortex F1–F7 screens (auth + conversation + tools + pickers + permissions + views + orchestration).
//! Copper brand helpers are forbidden in new paints.

pub mod auth;
pub mod chat_turn;
pub mod conversation;
pub mod login;
pub mod orchestration;
pub mod permissions;
pub mod pickers;
pub mod tools;
pub mod views;

use crate::chrome::canvas::Canvas;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PilotSpec {
    pub id: &'static str,
    pub category: &'static str,
}

/// Ordered pure Cortex dump inventory for F1–F7 (28 + 17 + 11 + 16 + 14 = 86).
pub const PILOT_SCREENS: [PilotSpec; 86] = [
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
    // F2 conversation/session toolkit (10, includes onboarding)
    PilotSpec {
        id: "home",
        category: "session",
    },
    PilotSpec {
        id: "onboarding",
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
    // F4 pickers (17)
    PilotSpec {
        id: "slash-menu",
        category: "input",
    },
    PilotSpec {
        id: "command-palette",
        category: "input",
    },
    PilotSpec {
        id: "command-palette-submenu",
        category: "input",
    },
    PilotSpec {
        id: "model-picker",
        category: "input",
    },
    PilotSpec {
        id: "model-picker-filtered",
        category: "input",
    },
    PilotSpec {
        id: "agent-picker",
        category: "input",
    },
    PilotSpec {
        id: "skills-picker",
        category: "input",
    },
    PilotSpec {
        id: "session-picker",
        category: "input",
    },
    PilotSpec {
        id: "variant-picker",
        category: "input",
    },
    PilotSpec {
        id: "fork-picker",
        category: "input",
    },
    PilotSpec {
        id: "theme-picker",
        category: "input",
    },
    PilotSpec {
        id: "export-picker",
        category: "input",
    },
    PilotSpec {
        id: "attachment-picker",
        category: "input",
    },
    PilotSpec {
        id: "subagent-picker",
        category: "input",
    },
    PilotSpec {
        id: "file-mention",
        category: "input",
    },
    PilotSpec {
        id: "queue-manager",
        category: "input",
    },
    PilotSpec {
        id: "composer-multiline",
        category: "input",
    },
    // F5 permissions & recovery (11)
    PilotSpec {
        id: "permission-shell",
        category: "input",
    },
    PilotSpec {
        id: "permission-file-edit",
        category: "input",
    },
    PilotSpec {
        id: "permission-deny-reason",
        category: "input",
    },
    PilotSpec {
        id: "blocked-command",
        category: "session",
    },
    PilotSpec {
        id: "turn-error",
        category: "session",
    },
    PilotSpec {
        id: "error-network",
        category: "states",
    },
    PilotSpec {
        id: "error-context",
        category: "states",
    },
    PilotSpec {
        id: "rate-limit",
        category: "states",
    },
    PilotSpec {
        id: "interrupt",
        category: "states",
    },
    PilotSpec {
        id: "checkpoint-undo",
        category: "states",
    },
    PilotSpec {
        id: "share",
        category: "states",
    },
    // F6 views (16)
    PilotSpec {
        id: "status",
        category: "views",
    },
    PilotSpec {
        id: "timeline",
        category: "views",
    },
    PilotSpec {
        id: "mcp",
        category: "views",
    },
    PilotSpec {
        id: "diff-viewer",
        category: "views",
    },
    PilotSpec {
        id: "diff-viewer-staging",
        category: "views",
    },
    PilotSpec {
        id: "help",
        category: "views",
    },
    PilotSpec {
        id: "review",
        category: "views",
    },
    PilotSpec {
        id: "theme-gallery-dark",
        category: "views",
    },
    PilotSpec {
        id: "theme-gallery-light",
        category: "views",
    },
    PilotSpec {
        id: "sidebar-context",
        category: "views",
    },
    PilotSpec {
        id: "sidebar-files",
        category: "views",
    },
    PilotSpec {
        id: "sidebar-lsp",
        category: "views",
    },
    PilotSpec {
        id: "sidebar-todo",
        category: "views",
    },
    PilotSpec {
        id: "sidebar-subagents",
        category: "views",
    },
    PilotSpec {
        id: "sidebar-mcp",
        category: "views",
    },
    PilotSpec {
        id: "sidebar-queue",
        category: "views",
    },
    // F7 orchestration (14) — painted as Cortex Tasks / Dashboard / plan
    PilotSpec {
        id: "mode-switch",
        category: "orchestration",
    },
    PilotSpec {
        id: "mode-switch-refused",
        category: "orchestration",
    },
    PilotSpec {
        id: "workflow-launch",
        category: "orchestration",
    },
    PilotSpec {
        id: "workflow-launch-blocked",
        category: "orchestration",
    },
    PilotSpec {
        id: "workflow-monitor",
        category: "orchestration",
    },
    PilotSpec {
        id: "workflow-monitor-failure",
        category: "orchestration",
    },
    PilotSpec {
        id: "team-board",
        category: "orchestration",
    },
    PilotSpec {
        id: "reviewer-output",
        category: "orchestration",
    },
    PilotSpec {
        id: "reviewer-approved",
        category: "orchestration",
    },
    PilotSpec {
        id: "verification-ladder",
        category: "orchestration",
    },
    PilotSpec {
        id: "intent-scorer-pass",
        category: "orchestration",
    },
    PilotSpec {
        id: "intent-scorer-fail",
        category: "orchestration",
    },
    PilotSpec {
        id: "concurrency-metrics",
        category: "orchestration",
    },
    PilotSpec {
        id: "tool-envelope",
        category: "orchestration",
    },
];

pub fn render_pilot(id: &str) -> Result<Canvas, String> {
    match id {
        // Auth
        "login"
        | "connect-provider"
        | "connect-method"
        | "connect-oauth"
        | "connect-apikey"
        | "connect-connected"
        | "connect-disconnect" => auth::render(id),
        // Conversation
        "home"
        | "onboarding"
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
        // Pickers
        "slash-menu"
        | "command-palette"
        | "command-palette-submenu"
        | "model-picker"
        | "model-picker-filtered"
        | "agent-picker"
        | "skills-picker"
        | "session-picker"
        | "variant-picker"
        | "fork-picker"
        | "theme-picker"
        | "export-picker"
        | "attachment-picker"
        | "subagent-picker"
        | "file-mention"
        | "queue-manager"
        | "composer-multiline" => pickers::render(id),
        // Permissions & recovery
        "permission-shell"
        | "permission-file-edit"
        | "permission-deny-reason"
        | "blocked-command"
        | "turn-error"
        | "error-network"
        | "error-context"
        | "rate-limit"
        | "interrupt"
        | "checkpoint-undo"
        | "share" => permissions::render(id),
        // Views
        "status"
        | "timeline"
        | "mcp"
        | "diff-viewer"
        | "diff-viewer-staging"
        | "help"
        | "review"
        | "theme-gallery-dark"
        | "theme-gallery-light"
        | "sidebar-context"
        | "sidebar-files"
        | "sidebar-lsp"
        | "sidebar-todo"
        | "sidebar-subagents"
        | "sidebar-mcp"
        | "sidebar-queue" => views::render(id),
        // Orchestration / Tasks / Dashboard
        "mode-switch"
        | "mode-switch-refused"
        | "workflow-launch"
        | "workflow-launch-blocked"
        | "workflow-monitor"
        | "workflow-monitor-failure"
        | "team-board"
        | "reviewer-output"
        | "reviewer-approved"
        | "verification-ladder"
        | "intent-scorer-pass"
        | "intent-scorer-fail"
        | "concurrency-metrics"
        | "tool-envelope" => orchestration::render(id),
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
    fn f1_f7_inventory_count_and_render() {
        assert_eq!(list_pilots().len(), 86);
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
