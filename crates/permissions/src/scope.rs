//! Subject derivation: how a gated tool action produces the string its rules are matched against.
//!
//! The permission engine ([`crate::PermissionEngine`]) is subject-agnostic — it matches a rule
//! pattern against whatever subject string the caller supplies. This module is the scoping layer
//! that decides *what that subject is* for each tool class (architecture §3.4):
//!
//! - **File tools** (`read`, `write`, `edit`, `apply_patch`, `glob`, `grep`, `format`) scope by their
//!   **target path**, so a path glob like `src/**` or `secrets/**` governs the action by where it
//!   reads or writes (VAL-PERM-006). The path is normalized to a canonical relative form first, so
//!   `./src/a.txt` and `src/a.txt` scope identically.
//! - **`bash`** scopes by its **command**, so a command glob like `git status*` or `git push*`
//!   governs the action by what it runs (VAL-PERM-007). A compound command can be split into its
//!   segments with [`split_bash_commands`] so each is classified on its own; the `bash` tool feeds
//!   tree-sitter-extracted commands through the same matcher for robust, obfuscation-resistant
//!   classification.
//! - **`task`** scopes by its requested subagent name, so a rule like `task:explore` governs
//!   whether that exact subagent type may be spawned (VAL-AGENT-013/014).
//!
//! These builders produce a [`PermissionRequest`] with the right subject; the engine then resolves
//! it against the active (per-agent, possibly subagent-capped) ruleset.

use crate::Ruleset;
use crate::action::Action;
use crate::engine::PermissionRequest;

/// The two ways a gated tool derives the subject its permission rules match against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolScope {
    /// The subject is the tool's target file path (file tools).
    Path,
    /// The subject is the shell command the tool runs (`bash`).
    Command,
    /// The subject is the requested target name (`task`).
    Target,
}

/// The file tools whose permission subject is their target path.
const PATH_TOOLS: [&str; 7] = [
    "read",
    "write",
    "edit",
    "apply_patch",
    "glob",
    "grep",
    "format",
];

/// The tool whose permission subject is the shell command it runs.
const COMMAND_TOOL: &str = "bash";

/// Tools whose permission subject is the requested target name.
const TARGET_TOOLS: [&str; 1] = ["task"];

/// How `tool` derives its permission subject, or `None` for a tool with no path/command scoping.
#[must_use]
pub fn scope_of(tool: &str) -> Option<ToolScope> {
    if tool == COMMAND_TOOL {
        Some(ToolScope::Command)
    } else if PATH_TOOLS.contains(&tool) {
        Some(ToolScope::Path)
    } else if TARGET_TOOLS.contains(&tool) {
        Some(ToolScope::Target)
    } else {
        None
    }
}

/// Whether `tool` is a file tool scoped by target path.
#[must_use]
pub fn is_file_tool(tool: &str) -> bool {
    matches!(scope_of(tool), Some(ToolScope::Path))
}

/// Whether `tool` is the command tool scoped by its parsed command.
#[must_use]
pub fn is_command_tool(tool: &str) -> bool {
    matches!(scope_of(tool), Some(ToolScope::Command))
}

/// Normalize a file path to the canonical relative form used as a permission subject.
///
/// Trims surrounding whitespace, strips leading `./` segments, collapses repeated and interior
/// `/./`, and drops a trailing slash (except for the root). `..` segments are kept literally and
/// never resolved, so a glob can never be widened by path arithmetic. The result is what a path
/// glob (`src/**`, `secrets/**`) is matched against, so `./src/a.txt` and `src/a.txt` scope
/// identically (VAL-PERM-006).
#[must_use]
pub fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let absolute = trimmed.starts_with('/');

    let mut segments: Vec<&str> = Vec::new();
    for segment in trimmed.split('/') {
        match segment {
            // Drop empty segments (repeated/leading/trailing slashes) and bare `.` segments.
            "" | "." => {}
            other => segments.push(other),
        }
    }

    let body = segments.join("/");
    if absolute { format!("/{body}") } else { body }
}

/// The permission subject for a file-tool action against `path` (its normalized path).
#[must_use]
pub fn file_subject(path: &str) -> String {
    normalize_path(path)
}

/// The permission subject for a `bash` action running `command` (the trimmed command line).
#[must_use]
pub fn command_subject(command: &str) -> String {
    command.trim().to_owned()
}

/// Build a [`PermissionRequest`] for `tool` acting on `target`, scoping the subject by the tool.
///
/// A file tool's `target` is normalized as a path; `bash`'s `target` is the command line. A tool
/// with no path/command scope passes `target` through trimmed.
#[must_use]
pub fn request_for(tool: &str, target: &str) -> PermissionRequest {
    let subject = match scope_of(tool) {
        Some(ToolScope::Path) => file_subject(target),
        Some(ToolScope::Command) => command_subject(target),
        Some(ToolScope::Target) | None => target.trim().to_owned(),
    };
    PermissionRequest::new(tool, subject)
}

/// The shell operators that separate one command from the next in a compound command line.
const COMMAND_SEPARATORS: [&str; 4] = ["&&", "||", ";", "|"];

/// Split a (possibly compound) shell command into its individual command segments.
///
/// Segments are separated by `&&`, `||`, `;`, `|`, and newlines, with surrounding whitespace
/// trimmed and empty segments dropped. Separators inside single or double quotes are ignored, so
/// `echo "a && b"` stays one segment. This is the subject-splitting primitive the `bash` tool and
/// the enforcement layer use to classify each segment of a compound command independently
/// (architecture §3.4); the `bash` tool additionally tree-sitter-parses each segment for
/// obfuscation-resistant root-command extraction.
#[must_use]
pub fn split_bash_commands(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];

        if let Some(open) = quote {
            current.push(ch);
            if ch == open {
                quote = None;
            }
            index += 1;
            continue;
        }

        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                current.push(ch);
                index += 1;
            }
            '\n' => {
                push_segment(&mut segments, &mut current);
                index += 1;
            }
            _ => {
                if let Some(sep) = COMMAND_SEPARATORS
                    .iter()
                    .find(|sep| matches_at(&chars, index, sep))
                {
                    push_segment(&mut segments, &mut current);
                    index += sep.chars().count();
                } else {
                    current.push(ch);
                    index += 1;
                }
            }
        }
    }
    push_segment(&mut segments, &mut current);
    segments
}

/// Classify a (possibly compound) `bash` command against `ruleset`, returning the most restrictive
/// action across its segments.
///
/// Each segment from [`split_bash_commands`] is classified on its own and the results are folded
/// with [`Action::most_restrictive`], so a single denied (or merely asked) segment dominates the
/// whole command. This is the classification primitive the enforcement layer turns into an atomic
/// refusal of the entire command (no segment runs) when any segment is not allowed. An empty
/// command classifies against the empty subject.
#[must_use]
pub fn classify_command(ruleset: &Ruleset, command: &str) -> Action {
    let segments = split_bash_commands(command);
    if segments.is_empty() {
        return ruleset.classify(COMMAND_TOOL, &command_subject(command));
    }
    segments
        .iter()
        .map(|segment| ruleset.classify(COMMAND_TOOL, segment))
        .reduce(Action::most_restrictive)
        .unwrap_or(Action::Ask)
}

/// Push the trimmed accumulated segment onto `segments` when non-empty, then clear it.
fn push_segment(segments: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_owned());
    }
    current.clear();
}

/// Whether the separator `sep` occurs at `index` in `chars`.
fn matches_at(chars: &[char], index: usize, sep: &str) -> bool {
    let sep_chars: Vec<char> = sep.chars().collect();
    if index + sep_chars.len() > chars.len() {
        return false;
    }
    chars[index..index + sep_chars.len()] == sep_chars[..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::Rule;

    #[test]
    fn scope_of_classifies_file_and_command_tools() {
        for tool in [
            "read",
            "write",
            "edit",
            "apply_patch",
            "glob",
            "grep",
            "format",
        ] {
            assert_eq!(
                scope_of(tool),
                Some(ToolScope::Path),
                "{tool} scopes by path"
            );
            assert!(is_file_tool(tool));
        }
        assert_eq!(scope_of("bash"), Some(ToolScope::Command));
        assert!(is_command_tool("bash"));
        assert_eq!(scope_of("task"), Some(ToolScope::Target));
        // A tool with no path/command scope (e.g. todo) has no scope.
        assert_eq!(scope_of("todo"), None);
        assert!(!is_file_tool("todo") && !is_command_tool("todo"));
    }

    #[test]
    fn normalize_path_strips_dot_and_collapses_slashes() {
        assert_eq!(normalize_path("./src/a.txt"), "src/a.txt");
        assert_eq!(normalize_path("src/a.txt"), "src/a.txt");
        assert_eq!(normalize_path("src//nested/./a.txt"), "src/nested/a.txt");
        assert_eq!(normalize_path("src/dir/"), "src/dir");
        assert_eq!(normalize_path("  ./secrets/b.txt  "), "secrets/b.txt");
        assert_eq!(normalize_path("/abs//path/"), "/abs/path");
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn normalize_path_keeps_dotdot_literal() {
        // `..` is never resolved, so a glob cannot be widened by path arithmetic.
        assert_eq!(normalize_path("src/../secrets/x"), "src/../secrets/x");
    }

    #[test]
    fn request_for_scopes_subject_by_tool() {
        let write = request_for("write", "./src/a.txt");
        assert_eq!(write.tool, "write");
        assert_eq!(write.subject, "src/a.txt");

        let bash = request_for("bash", "  git status --short  ");
        assert_eq!(bash.tool, "bash");
        assert_eq!(bash.subject, "git status --short");

        let task = request_for("task", "  explore  ");
        assert_eq!(task.tool, "task");
        assert_eq!(task.subject, "explore");
    }

    #[test]
    fn split_bash_commands_splits_on_operators() {
        assert_eq!(
            split_bash_commands("echo hi && rm victim.txt"),
            vec!["echo hi".to_owned(), "rm victim.txt".to_owned()]
        );
        assert_eq!(
            split_bash_commands("a; b | c || d"),
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned()
            ]
        );
        assert_eq!(
            split_bash_commands("first\nsecond"),
            vec!["first".to_owned(), "second".to_owned()]
        );
    }

    #[test]
    fn split_bash_commands_ignores_separators_inside_quotes() {
        assert_eq!(
            split_bash_commands(r#"echo "a && b""#),
            vec![r#"echo "a && b""#.to_owned()]
        );
        assert_eq!(
            split_bash_commands("echo 'x | y' && ls"),
            vec!["echo 'x | y'".to_owned(), "ls".to_owned()]
        );
    }

    #[test]
    fn classify_command_takes_the_most_restrictive_segment() {
        // allow everything, but deny rm: a compound with an rm segment is denied as a whole.
        let ruleset = Ruleset::new(vec![
            Rule::new("bash", "*", Action::Allow),
            Rule::new("bash", "rm *", Action::Deny),
        ]);
        assert_eq!(
            classify_command(&ruleset, "echo hi && rm victim.txt"),
            Action::Deny
        );
        assert_eq!(classify_command(&ruleset, "echo hi && ls"), Action::Allow);
        // A benign string containing the verb in an argument is not a separate command.
        assert_eq!(classify_command(&ruleset, "echo 'rm me'"), Action::Allow);
    }
}
