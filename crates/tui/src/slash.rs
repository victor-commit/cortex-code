//! Slash-command engine for the TUI prompt.
//!
//! This module owns the registry of built-in slash commands and the pure filtering
//! logic that backs the autocomplete menu. Dispatch (the visible effect of running a
//! command) is owned by [`crate::app::App`], which routes a selected command to its
//! handler; the registry here is the single source of truth for what commands exist
//! and how they are described in the menu.

/// A built-in slash command exposed in the autocomplete menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    /// The command name without the leading `/` (always lowercase).
    pub name: &'static str,
    /// A one-line description shown beside the name in the menu.
    pub description: &'static str,
}

/// The built-in slash commands, in the order they appear when the menu first opens.
pub const BUILTINS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        description: "Show the command reference and keybindings",
    },
    SlashCommand {
        name: "new",
        description: "Start a fresh session",
    },
    SlashCommand {
        name: "sessions",
        description: "Open the session picker",
    },
    SlashCommand {
        name: "models",
        description: "Open the model picker",
    },
    SlashCommand {
        name: "model",
        description: "Open the model picker",
    },
    SlashCommand {
        name: "agents",
        description: "Open the agent picker",
    },
    SlashCommand {
        name: "skills",
        description: "Open the skill picker",
    },
    SlashCommand {
        name: "variant",
        description: "Open the variant picker",
    },
    SlashCommand {
        name: "themes",
        description: "Open the theme picker",
    },
    SlashCommand {
        name: "theme",
        description: "Open the theme picker",
    },
    SlashCommand {
        name: "status",
        description: "Open the status view",
    },
    SlashCommand {
        name: "timeline",
        description: "Open the session timeline",
    },
    SlashCommand {
        name: "fork",
        description: "Open the fork dialog",
    },
    SlashCommand {
        name: "subagents",
        description: "Open the subagent picker",
    },
    SlashCommand {
        name: "queue",
        description: "Open queued prompt manager",
    },
    SlashCommand {
        name: "sidebar-toggle",
        description: "Toggle sidebar visibility",
    },
    SlashCommand {
        name: "sidebar-context",
        description: "Show sidebar context panel",
    },
    SlashCommand {
        name: "sidebar-files",
        description: "Show sidebar files panel",
    },
    SlashCommand {
        name: "sidebar-lsp",
        description: "Show sidebar LSP panel",
    },
    SlashCommand {
        name: "sidebar-mcp",
        description: "Show sidebar MCP panel",
    },
    SlashCommand {
        name: "sidebar-todo",
        description: "Show sidebar todo panel",
    },
    SlashCommand {
        name: "sidebar-subagents",
        description: "Show sidebar subagents panel",
    },
    SlashCommand {
        name: "sidebar-queue",
        description: "Show sidebar queue panel",
    },
    SlashCommand {
        name: "rename",
        description: "Rename a persisted session",
    },
    SlashCommand {
        name: "attach",
        description: "Attach a local file (png, jpg, pdf, svg)",
    },
    SlashCommand {
        name: "paste",
        description: "Attach an image from the clipboard",
    },
    SlashCommand {
        name: "init",
        description: "Initialize the project (AGENTS.md)",
    },
    SlashCommand {
        name: "review",
        description: "Run a code review",
    },
    SlashCommand {
        name: "diff",
        description: "Open the diff viewer",
    },
    SlashCommand {
        name: "compact",
        description: "Compact the conversation",
    },
    SlashCommand {
        name: "export",
        description: "Export the current session",
    },
    SlashCommand {
        name: "editor",
        description: "Compose the prompt in an external editor",
    },
    SlashCommand {
        name: "share",
        description: "Create a local share reference",
    },
    SlashCommand {
        name: "unshare",
        description: "Revoke local share references",
    },
    SlashCommand {
        name: "mcp",
        description: "Open the MCP status view",
    },
    SlashCommand {
        name: "details",
        description: "Toggle detail on the selected tool card",
    },
    SlashCommand {
        name: "undo",
        description: "Undo the last change",
    },
    SlashCommand {
        name: "redo",
        description: "Redo the last undone change",
    },
    SlashCommand {
        name: "exit",
        description: "Exit Cortex Code",
    },
];

/// Filter the built-ins by `query` (the text typed after `/`).
///
/// An empty query returns every command in registry order. A non-empty query keeps
/// commands whose name contains it (case-insensitive), ranking prefix matches ahead of
/// interior substring matches so the most relevant command highlights first.
#[must_use]
#[allow(dead_code)]
pub fn filter(query: &str) -> Vec<&'static SlashCommand> {
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return BUILTINS.iter().collect();
    }
    let mut prefix = Vec::new();
    let mut substring = Vec::new();
    for command in BUILTINS {
        if command.name.starts_with(&needle) {
            prefix.push(command);
        } else if command.name.contains(&needle) {
            substring.push(command);
        }
    }
    prefix.extend(substring);
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_all_builtins_in_order() {
        let all = filter("");
        assert_eq!(all.len(), BUILTINS.len());
        assert_eq!(all[0].name, "help");
        assert_eq!(all.last().unwrap().name, "exit");
    }

    #[test]
    fn prefix_filters_to_matching_command() {
        // VAL-TUI-029: "/mod" narrows to `models` and excludes unrelated commands.
        let matches = filter("mod");
        let names: Vec<&str> = matches.iter().map(|c| c.name).collect();
        assert!(names.contains(&"models"), "models present: {names:?}");
        assert!(!names.contains(&"help"), "help hidden: {names:?}");
    }

    #[test]
    fn filtering_is_case_insensitive() {
        let matches = filter("MOD");
        assert!(matches.iter().any(|c| c.name == "models"));
    }

    #[test]
    fn exact_name_matches_itself() {
        let matches = filter("exit");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "exit");
    }

    #[test]
    fn unknown_query_yields_no_matches() {
        // VAL-TUI-048: an unrecognized command filters to nothing (graceful no-match).
        assert!(filter("zzznotreal").is_empty());
    }

    #[test]
    fn prefix_matches_rank_before_interior_substring_matches() {
        // "e" prefixes `export`/`editor`/`exit` but only appears interior to e.g. `new`/`agents`.
        let matches = filter("e");
        let first = matches[0].name;
        assert!(
            first.starts_with('e'),
            "a prefix match ranks first, got {first}"
        );
    }
}
