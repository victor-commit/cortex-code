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
//! - **Target tools** (`task`, `webfetch`, `websearch`) scope by the requested target string (the
//!   subagent name, URL, or query), so callers can allow/deny specific network searches without
//!   treating them as file paths.
//! - **`task`** scopes by its requested subagent name, so a rule like `task:explore` governs
//!   whether that exact subagent type may be spawned (VAL-AGENT-013/014).
//!
//! These builders produce a [`PermissionRequest`] with the right subject; the engine then resolves
//! it against the active (per-agent, possibly subagent-capped) ruleset.

use crate::Ruleset;
use crate::action::Action;
use crate::engine::PermissionRequest;

/// Canonical capability name for file-mutating tools.
pub const CAPABILITY_EDIT: &str = "edit";
/// Canonical capability name for read/search tools.
pub const CAPABILITY_READ: &str = "read";
/// Canonical capability name for shell execution.
pub const CAPABILITY_BASH: &str = "bash";
/// Canonical capability name for subagent delegation.
pub const CAPABILITY_TASK: &str = "task";
/// Canonical capability name for todo planning updates.
pub const CAPABILITY_TODO: &str = "todo";
/// Canonical capability name for skill activation.
pub const CAPABILITY_SKILL: &str = "skill";
/// Canonical capability name for language-server queries.
pub const CAPABILITY_LSP: &str = "lsp";
/// Canonical capability name for interactive question asks.
pub const CAPABILITY_QUESTION: &str = "question";
/// Canonical capability name for entering plan mode.
pub const CAPABILITY_PLAN_ENTER: &str = "plan_enter";
/// Canonical capability name for leaving plan mode.
pub const CAPABILITY_PLAN_EXIT: &str = "plan_exit";
/// Canonical capability name for repeated-call loop safeguards.
pub const CAPABILITY_DOOM_LOOP: &str = "doom_loop";
/// Canonical capability name for outside-workspace path access.
pub const CAPABILITY_EXTERNAL_DIRECTORY: &str = "external_directory";

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

/// The path-scoped actions whose permission subject is a normalized path.
const PATH_TOOLS: [&str; 8] = [
    "read",
    "write",
    "edit",
    "apply_patch",
    "glob",
    "grep",
    "format",
    CAPABILITY_EXTERNAL_DIRECTORY,
];

/// The tool whose permission subject is the shell command it runs.
const COMMAND_TOOL: &str = CAPABILITY_BASH;

/// Tools whose permission subject is the requested target string.
const TARGET_TOOLS: [&str; 12] = [
    CAPABILITY_TASK,
    "webfetch",
    "websearch",
    CAPABILITY_TODO,
    CAPABILITY_SKILL,
    CAPABILITY_LSP,
    CAPABILITY_QUESTION,
    CAPABILITY_PLAN_ENTER,
    CAPABILITY_PLAN_EXIT,
    CAPABILITY_DOOM_LOOP,
    "plan",
    "plan_exit",
];

/// Canonicalize a permission tool/capability selector token.
///
/// Compatibility aliases use hyphenated tokens in some callers (`plan-enter`, `plan-exit`,
/// `loop-safeguard`) while the Rust side uses snake_case. This canonical form preserves existing
/// tool names and normalizes only known aliases.
#[must_use]
pub fn canonical_tool_name(tool: &str) -> &str {
    match tool {
        "plan-enter" => CAPABILITY_PLAN_ENTER,
        "plan-exit" => CAPABILITY_PLAN_EXIT,
        "loop-safeguard" => CAPABILITY_DOOM_LOOP,
        "todowrite" => CAPABILITY_TODO,
        "plan_enter" => CAPABILITY_PLAN_ENTER,
        "plan_exit" => CAPABILITY_PLAN_EXIT,
        "doom_loop" => CAPABILITY_DOOM_LOOP,
        other => other,
    }
}

/// Return the capability namespace token for a concrete tool/capability.
///
/// Existing tool-specific rules still work (`write:*`, `apply_patch:*`), and capability rules can
/// now intentionally fan out (`edit:*` applies to write/edit/apply_patch/format).
#[must_use]
pub fn capability_for_tool(tool: &str) -> Option<&'static str> {
    match canonical_tool_name(tool) {
        "write" | "edit" | "apply_patch" | "format" => Some(CAPABILITY_EDIT),
        "read" | "glob" | "grep" => Some(CAPABILITY_READ),
        CAPABILITY_BASH => Some(CAPABILITY_BASH),
        CAPABILITY_TASK => Some(CAPABILITY_TASK),
        CAPABILITY_TODO => Some(CAPABILITY_TODO),
        CAPABILITY_SKILL => Some(CAPABILITY_SKILL),
        CAPABILITY_LSP => Some(CAPABILITY_LSP),
        CAPABILITY_QUESTION => Some(CAPABILITY_QUESTION),
        CAPABILITY_PLAN_ENTER | "plan" => Some(CAPABILITY_PLAN_ENTER),
        CAPABILITY_PLAN_EXIT => Some(CAPABILITY_PLAN_EXIT),
        CAPABILITY_DOOM_LOOP => Some(CAPABILITY_DOOM_LOOP),
        CAPABILITY_EXTERNAL_DIRECTORY => Some(CAPABILITY_EXTERNAL_DIRECTORY),
        _ => None,
    }
}

/// How `tool` derives its permission subject, or `None` for a tool with no path/command scoping.
#[must_use]
pub fn scope_of(tool: &str) -> Option<ToolScope> {
    let tool = canonical_tool_name(tool);
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
    let trimmed = expand_home_tokens(path.trim());
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

fn expand_home_tokens(path: &str) -> String {
    let Some(home) = std::env::var_os("HOME") else {
        return path.to_owned();
    };
    let home = std::path::PathBuf::from(home)
        .to_string_lossy()
        .replace('\\', "/");
    if path == "~" {
        return home;
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return format!("{home}/{rest}");
    }
    if path == "$HOME" || path == "${HOME}" {
        return home;
    }
    if let Some(rest) = path.strip_prefix("$HOME/") {
        return format!("{home}/{rest}");
    }
    if let Some(rest) = path.strip_prefix("${HOME}/") {
        return format!("{home}/{rest}");
    }
    path.to_owned()
}

/// Expand well-known home aliases inside one shell-like command string.
///
/// This keeps command-based permission matching consistent when one side uses `~` or `$HOME`
/// while the other uses an absolute home path.
#[must_use]
pub fn expand_home_tokens_in_command(command: &str) -> String {
    if std::env::var_os("HOME").is_none() {
        return command.to_owned();
    }
    let mut out = String::new();
    let mut token = String::new();
    let mut quote: Option<char> = None;

    for ch in command.chars() {
        if let Some(open) = quote {
            token.push(ch);
            if ch == open {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                token.push(ch);
                quote = Some(ch);
            }
            ' ' | '\t' => {
                if !token.is_empty() {
                    out.push_str(&expand_home_tokens(&token));
                    token.clear();
                }
                out.push(ch);
            }
            _ => token.push(ch),
        }
    }
    if !token.is_empty() {
        out.push_str(&expand_home_tokens(&token));
    }
    out
}

/// Candidate command subjects for permission matching, including arity-normalized families.
#[must_use]
pub fn command_match_candidates(command: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let raw = command_subject(command);
    push_unique(&mut candidates, raw.clone());
    let expanded = command_subject(&expand_home_tokens_in_command(&raw));
    push_unique(&mut candidates, expanded.clone());
    if let Some(normalized) = normalize_bash_command_arity(&raw) {
        push_unique(&mut candidates, normalized);
    }
    if let Some(normalized) = normalize_bash_command_arity(&expanded) {
        push_unique(&mut candidates, normalized);
    }
    candidates
}

fn push_unique(items: &mut Vec<String>, value: String) {
    if !value.is_empty() && !items.iter().any(|item| item == &value) {
        items.push(value);
    }
}

fn normalize_bash_command_arity(command: &str) -> Option<String> {
    let words = split_shell_words(command);
    if words.is_empty() {
        return None;
    }

    let mut index = 0;
    while index < words.len() {
        let word = words[index].as_str();
        if is_env_assignment(word) || matches!(word, "sudo" | "env" | "command" | "time" | "nohup")
        {
            index += 1;
            continue;
        }
        break;
    }
    if index >= words.len() {
        return None;
    }
    let head = &words[index..];
    let normalized = match (head.first()?.as_str(), head.get(1).map(String::as_str)) {
        ("git", Some(cmd))
            if matches!(
                cmd,
                "add"
                    | "checkout"
                    | "switch"
                    | "commit"
                    | "push"
                    | "pull"
                    | "fetch"
                    | "clone"
                    | "merge"
                    | "rebase"
                    | "status"
                    | "reset"
                    | "restore"
            ) =>
        {
            format!("git {cmd}")
        }
        ("npm", Some(cmd))
            if matches!(
                cmd,
                "install" | "i" | "update" | "uninstall" | "run" | "test" | "publish" | "pack"
            ) =>
        {
            format!("npm {cmd}")
        }
        ("pnpm", Some(cmd))
            if matches!(
                cmd,
                "add" | "install" | "update" | "remove" | "run" | "test" | "publish"
            ) =>
        {
            format!("pnpm {cmd}")
        }
        ("yarn", Some(cmd))
            if matches!(
                cmd,
                "add" | "install" | "remove" | "up" | "upgrade" | "run" | "test" | "dlx"
            ) =>
        {
            format!("yarn {cmd}")
        }
        ("cargo", Some(cmd))
            if matches!(
                cmd,
                "build" | "check" | "test" | "run" | "fmt" | "clippy" | "clean" | "doc"
            ) =>
        {
            format!("cargo {cmd}")
        }
        ("pip", Some(cmd))
            if matches!(cmd, "install" | "uninstall" | "list" | "freeze" | "show") =>
        {
            format!("pip {cmd}")
        }
        ("python", Some("-m")) if head.get(2).is_some_and(|module| module == "pip") => {
            if let Some(cmd) = head.get(3) {
                format!("python -m pip {cmd}")
            } else {
                "python -m pip".to_owned()
            }
        }
        _ => return None,
    };
    Some(normalized)
}

fn split_shell_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match quote {
            Some(open) => match ch {
                '\\' if open == '"' => escaped = true,
                c if c == open => quote = None,
                _ => current.push(ch),
            },
            None => match ch {
                '\\' => escaped = true,
                '\'' | '"' => quote = Some(ch),
                ' ' | '\t' | '\n' => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(ch),
            },
        }
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
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
            "external_directory",
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
        for tool in [
            "task",
            "webfetch",
            "websearch",
            "todo",
            "skill",
            "lsp",
            "question",
            "plan_enter",
            "plan_exit",
            "doom_loop",
        ] {
            assert_eq!(
                scope_of(tool),
                Some(ToolScope::Target),
                "{tool} scopes by target string"
            );
        }
        assert_eq!(scope_of("unknown_tool"), None);
        assert!(!is_file_tool("unknown_tool") && !is_command_tool("unknown_tool"));
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
    fn normalize_path_expands_home_aliases_when_home_is_available() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = std::path::PathBuf::from(home)
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(
            normalize_path("~/secrets/.env"),
            format!("{home}/secrets/.env")
        );
        assert_eq!(
            normalize_path("$HOME/secrets/.env"),
            format!("{home}/secrets/.env")
        );
        assert_eq!(
            normalize_path("${HOME}/secrets/.env"),
            format!("{home}/secrets/.env")
        );
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

    #[test]
    fn command_match_candidates_include_arity_normalized_family() {
        let candidates = command_match_candidates("sudo git checkout feature/demo");
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate == "git checkout")
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate == "sudo git checkout feature/demo")
        );
    }

    #[test]
    fn command_match_candidates_expand_home_aliases() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = std::path::PathBuf::from(home)
            .to_string_lossy()
            .replace('\\', "/");
        let candidates = command_match_candidates("cat ~/secrets/token.txt");
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate == &format!("cat {home}/secrets/token.txt"))
        );
    }

    #[test]
    fn capability_mapping_fans_out_mutator_tools() {
        assert_eq!(capability_for_tool("write"), Some(CAPABILITY_EDIT));
        assert_eq!(capability_for_tool("apply_patch"), Some(CAPABILITY_EDIT));
        assert_eq!(
            capability_for_tool("plan-enter"),
            Some(CAPABILITY_PLAN_ENTER)
        );
        assert_eq!(canonical_tool_name("loop-safeguard"), CAPABILITY_DOOM_LOOP);
    }
}
