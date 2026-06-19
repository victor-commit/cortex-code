//! Integration coverage for pattern-based permission scoping (VAL-PERM-006, VAL-PERM-007).
//!
//! File tools scope by their target path under glob patterns, and `bash` scopes by its parsed
//! command under command patterns. These exercise the public scoping builders (`request_for`,
//! `classify_command`, `split_bash_commands`) through the real [`PermissionEngine`], asserting the
//! same tool is allowed for an allowed subject and denied for a denied one under one config.

use permissions::{
    Action, Outcome, PermissionEngine, Rule, Ruleset, classify_command, request_for,
    split_bash_commands,
};

/// VAL-PERM-006: file-tool glob patterns scope permissions by path.
#[test]
fn file_tool_glob_patterns_scope_by_path() {
    // One config: allow write under src/**, deny write under secrets/**.
    let ruleset = Ruleset::new(vec![
        Rule::new("write", "src/**", Action::Allow),
        Rule::new("write", "secrets/**", Action::Deny),
    ]);
    let engine = PermissionEngine::new(ruleset);

    // A write to an allowed path proceeds; a write to a denied path is blocked — same tool, one
    // config, decided by the target path.
    assert_eq!(
        engine.evaluate(&request_for("write", "src/a.txt")),
        Outcome::Allowed,
        "write to src/a.txt is allowed by the src/** glob"
    );
    assert_eq!(
        engine.evaluate(&request_for("write", "secrets/b.txt")),
        Outcome::Denied,
        "write to secrets/b.txt is blocked by the secrets/** glob"
    );

    // A leading ./ on the path scopes identically (normalized before matching).
    assert_eq!(
        engine.evaluate(&request_for("write", "./src/nested/a.txt")),
        Outcome::Allowed,
    );
    assert_eq!(
        engine.evaluate(&request_for("write", "./secrets/nested/b.txt")),
        Outcome::Denied,
    );
}

/// The path scope governs every file tool, not just write (read/edit/apply_patch/glob/grep).
#[test]
fn path_scope_governs_all_file_tools() {
    let ruleset = Ruleset::new(vec![Rule::new("*", "secrets/**", Action::Deny)]);
    let engine = PermissionEngine::new(ruleset);
    for tool in ["read", "write", "edit", "apply_patch", "glob", "grep"] {
        assert_eq!(
            engine.evaluate(&request_for(tool, "secrets/key.txt")),
            Outcome::Denied,
            "{tool} to a denied path is blocked"
        );
    }
}

/// VAL-PERM-007: bash command patterns scope permissions by command.
#[test]
fn bash_command_patterns_scope_by_command() {
    // One config: allow `git status*`, deny `git push*`.
    let ruleset = Ruleset::new(vec![
        Rule::new("bash", "git status*", Action::Allow),
        Rule::new("bash", "git push*", Action::Deny),
    ]);
    let engine = PermissionEngine::new(ruleset);

    // The same bash tool: a matching-allowed command runs, a matching-denied command is blocked.
    assert_eq!(
        engine.evaluate(&request_for("bash", "git status --short")),
        Outcome::Allowed,
        "git status matches the allow pattern"
    );
    assert_eq!(
        engine.evaluate(&request_for("bash", "git push origin main")),
        Outcome::Denied,
        "git push matches the deny pattern"
    );
}

/// A compound bash command is classified by its most restrictive segment, so a denied segment
/// dominates the whole command (the primitive the enforcement layer turns into atomic refusal).
#[test]
fn compound_bash_command_is_classified_by_its_most_restrictive_segment() {
    let ruleset = Ruleset::new(vec![
        Rule::new("bash", "*", Action::Allow),
        Rule::new("bash", "rm *", Action::Deny),
    ]);

    // `echo hi && rm victim.txt` splits into two commands; the rm segment is denied.
    assert_eq!(
        split_bash_commands("echo hi && rm victim.txt"),
        vec!["echo hi".to_owned(), "rm victim.txt".to_owned()]
    );
    assert_eq!(
        classify_command(&ruleset, "echo hi && rm victim.txt"),
        Action::Deny,
        "a denied segment dominates the compound command"
    );
    // A benign command merely containing the verb as text is not denied.
    assert_eq!(
        classify_command(&ruleset, "echo rm"),
        Action::Allow,
        "the literal text rm in an argument is not the rm command"
    );
}
