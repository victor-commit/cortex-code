//! Tree-sitter extraction of the real command invocations from a (possibly compound) shell
//! command, so permission matching classifies a command by its real structure rather than naive
//! string matching.
//!
//! The bash grammar (tree-sitter-bash) parses the command into a syntax tree; every `command` node
//! is a real invocation (`command_name` plus its arguments), including ones nested inside
//! command substitutions, pipelines, lists, and subshells. Extracting those nodes means a dangerous
//! verb obscured by quoting/compound structure (e.g. `echo hi && rm x`, `$(rm x)`) is still
//! surfaced as its real command `rm x`, while a benign string that merely contains the verb (e.g.
//! `echo rm`) is surfaced as its real command `echo rm` — and so never matches a deny rule on `rm`.
//!
//! If the grammar cannot be loaded or the parse yields no command node, this falls back to the
//! quote-aware [`permissions::split_bash_commands`] splitter so gating is never silently skipped.

use tree_sitter::{Node, Parser};

/// Extract the real command invocations from `command` for permission matching.
///
/// Each returned string is one command invocation (command name plus arguments) as it appears in
/// the source, suitable as a permission subject. The list is never empty for a non-empty command:
/// it falls back to the quote-aware splitter, then to the whole trimmed command, so a command is
/// always classified by *something* rather than slipping through ungated.
#[must_use]
pub fn extract_commands(command: &str) -> Vec<String> {
    if command.trim().is_empty() {
        return Vec::new();
    }

    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .is_err()
    {
        return fallback(command);
    }
    let Some(tree) = parser.parse(command, None) else {
        return fallback(command);
    };

    let mut commands = Vec::new();
    collect(tree.root_node(), command.as_bytes(), &mut commands);
    if commands.is_empty() {
        return fallback(command);
    }
    commands
}

/// Recursively collect the text of every `command` node under `node`.
///
/// Recursion continues through a matched `command` node so nested invocations inside command
/// substitutions (`$(...)`/backticks) and process substitutions are surfaced too.
fn collect(node: Node, src: &[u8], out: &mut Vec<String>) {
    if node.kind() == "command" {
        if let Ok(text) = node.utf8_text(src) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_owned());
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, src, out);
    }
}

/// The non-tree-sitter fallback: the quote-aware compound splitter, then the whole trimmed command.
fn fallback(command: &str) -> Vec<String> {
    let segments = permissions::split_bash_commands(command);
    if segments.is_empty() {
        vec![command.trim().to_owned()]
    } else {
        segments
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_each_segment_of_a_compound_command() {
        let commands = extract_commands("echo hi && rm victim.txt");
        assert!(commands.iter().any(|c| c == "echo hi"), "got: {commands:?}");
        assert!(
            commands.iter().any(|c| c == "rm victim.txt"),
            "the real rm invocation must be surfaced: {commands:?}"
        );
    }

    #[test]
    fn a_benign_string_containing_the_verb_is_not_a_command() {
        // `echo rm` is a single `echo` invocation whose argument is the literal "rm"; it must NOT
        // be surfaced as an `rm` command.
        let commands = extract_commands("echo rm");
        assert_eq!(commands, vec!["echo rm".to_owned()]);
        assert!(
            !commands.iter().any(|c| c == "rm" || c.starts_with("rm ")),
            "a benign 'echo rm' must not parse to an rm command: {commands:?}"
        );
    }

    #[test]
    fn surfaces_a_command_obscured_by_substitution() {
        let commands = extract_commands("echo $(rm victim.txt)");
        assert!(
            commands.iter().any(|c| c == "rm victim.txt"),
            "the rm hidden in a command substitution must be surfaced: {commands:?}"
        );
    }

    #[test]
    fn surfaces_commands_across_a_pipeline() {
        let commands = extract_commands("cat file | grep needle | wc -l");
        assert!(
            commands.iter().any(|c| c == "cat file"),
            "got: {commands:?}"
        );
        assert!(
            commands.iter().any(|c| c == "grep needle"),
            "got: {commands:?}"
        );
        assert!(commands.iter().any(|c| c == "wc -l"), "got: {commands:?}");
    }

    #[test]
    fn a_quoted_separator_stays_one_command() {
        let commands = extract_commands(r#"echo "a && b""#);
        assert_eq!(commands, vec![r#"echo "a && b""#.to_owned()]);
    }

    #[test]
    fn empty_command_yields_no_commands() {
        assert!(extract_commands("   ").is_empty());
    }
}
