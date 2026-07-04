//! Brand-styled `cortex --help` for the human/TTY surface.
//!
//! On a real terminal the top-level `--help` is rendered in the mono-orange brand palette
//! (tagline + `Usage:` + `Options:`/`Commands:` sections, a curated subcommand shortlist, and an
//! `…and N more` overflow hint) instead of clap's stock cyan/green help. When stdout is not a
//! terminal (piped/redirected) — or `NO_COLOR` suppresses color — this is skipped entirely and
//! clap renders its default, ANSI-free help with the derive-generated wording unchanged
//! (VAL-CLI-036). Subcommand help (`cortex <cmd> --help`) is never intercepted; clap handles it.

use std::io::Write;
use std::process::ExitCode;

use clap::CommandFactory;
use errors::Exit;

use crate::cli::Cli;
use crate::style::{self, Style, Styler};

/// The curated subcommand shortlist shown on the styled help, in presentation order. The remaining
/// real subcommands are summarized by the `…and N more` overflow line.
const SHORTLIST: [&str; 11] = [
    "run", "tui", "serve", "session", "stats", "search", "replay", "models", "auth", "config",
    "mcp",
];

/// If the invocation is a top-level `--help`/`-h` request on a styling-enabled TTY, render the
/// brand-styled help to stdout and return the process exit code. Otherwise return `None` so the
/// caller falls through to clap's normal parse (which renders the default help off a TTY).
#[must_use]
pub fn maybe_render_styled_help() -> Option<ExitCode> {
    let styler = Styler::for_stdout();
    if !styler.is_enabled() {
        return None;
    }
    if !top_level_help_requested(std::env::args().skip(1)) {
        return None;
    }
    let body = render_help(styler);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(body.as_bytes());
    let _ = handle.flush();
    Some(Exit::Success.into())
}

/// Whether the arguments request the TOP-LEVEL help (a `-h`/`--help` that appears before any
/// subcommand). Global valueless flags (`-v`/`--verbose`/`--pure`) may precede it, including when
/// grouped into a single short-flag cluster (`-vh`/`-hv`), which clap treats as `-v -h`; the first
/// bare token is a subcommand, so help after it belongs to that subcommand and is left to clap.
fn top_level_help_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for arg in args {
        match arg.as_ref() {
            "-h" | "--help" => return true,
            "-V" | "--version" => return false,
            // A long global flag (e.g. `--pure`): valueless, keep scanning.
            token if token.starts_with("--") => continue,
            // A short-flag cluster of global valueless flags (`-v`, plus clap's auto `-h`/`-V`),
            // e.g. `-vh`/`-hv`/`-vvh`. Scan the cluster left-to-right the way clap does: the first
            // `h` renders help, the first `V` defers to version, `v` is skipped, and any other
            // short flag is unknown here so we defer to clap (which will diagnose it).
            token if token.starts_with('-') && token.len() > 1 => {
                for c in token[1..].chars() {
                    match c {
                        'v' => {}
                        'h' => return true,
                        'V' => return false,
                        _ => return false,
                    }
                }
            }
            token if token.starts_with('-') => continue,
            _ => return false,
        }
    }
    false
}

/// Build the styled help text (with a trailing newline).
fn render_help(styler: Styler) -> String {
    let cmd = Cli::command();
    let mut out = String::new();

    // Tagline (the derive `about`).
    if let Some(about) = cmd.get_about() {
        out.push_str(&styler.paint(&about.to_string(), style::TXT));
        out.push('\n');
    }
    out.push('\n');

    // Usage.
    out.push_str(&styler.paint("Usage:", style::ORANGE));
    out.push(' ');
    out.push_str(&styler.paint("cortex [OPTIONS] [COMMAND]", style::TXT));
    out.push('\n');
    out.push('\n');

    // Options.
    let option_rows = option_rows(&cmd);
    let option_key_w = option_rows
        .iter()
        .map(|(flags, _)| flags.chars().count())
        .max()
        .unwrap_or(0);
    out.push_str(&styler.apply("Options:", Style::fg(style::AMBER).bold()));
    out.push('\n');
    for (flags, desc) in &option_rows {
        out.push_str("  ");
        out.push_str(&styler.paint(&pad(flags, option_key_w), style::ORANGE));
        out.push_str("  ");
        out.push_str(&styler.paint(desc, style::TXT));
        out.push('\n');
    }
    out.push('\n');

    // Commands: the curated shortlist + an overflow hint.
    let subcommands: Vec<(&str, String)> = cmd
        .get_subcommands()
        .filter(|sc| !sc.is_hide_set() && sc.get_name() != "help")
        .map(|sc| {
            (
                sc.get_name(),
                sc.get_about().map(|a| short_desc(&a.to_string())).unwrap_or_default(),
            )
        })
        .collect();
    let total = subcommands.len();

    let mut command_rows: Vec<(&str, String)> = Vec::new();
    for name in SHORTLIST {
        if let Some((sc_name, desc)) = subcommands.iter().find(|(sc_name, _)| *sc_name == name) {
            command_rows.push((sc_name, desc.clone()));
        }
    }
    let command_key_w = command_rows
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0);

    out.push_str(&styler.apply("Commands:", Style::fg(style::AMBER).bold()));
    out.push('\n');
    for (name, desc) in &command_rows {
        out.push_str("  ");
        out.push_str(&styler.paint(&pad(name, command_key_w), style::ORANGE));
        out.push_str("  ");
        out.push_str(&styler.paint(desc, style::TXT));
        out.push('\n');
    }

    let more = total.saturating_sub(command_rows.len());
    if more > 0 {
        out.push_str("  ");
        out.push_str(&styler.paint(
            &format!("{}and {more} more {} run ", style::ELLIPSIS, style::EM_DASH),
            style::FAINT,
        ));
        out.push_str(&styler.paint("cortex help <command>", style::ORANGE));
        out.push('\n');
    }

    out
}

/// The option rows (flag spec + short description) for the top-level command. `-h/--help` and
/// `-V/--version` are appended explicitly so they appear whether or not clap has materialized them
/// on the un-built factory command.
fn option_rows(cmd: &clap::Command) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    for arg in cmd.get_arguments() {
        let id = arg.get_id().as_str();
        if arg.is_positional() || arg.is_hide_set() || id == "help" || id == "version" {
            continue;
        }
        let desc = arg.get_help().map(|h| short_desc(&h.to_string())).unwrap_or_default();
        rows.push((flag_spec(arg), desc));
    }
    rows.push(("-h, --help".to_owned(), "Print help".to_owned()));
    rows.push(("-V, --version".to_owned(), "Print version".to_owned()));
    rows
}

/// The displayed flag spec for an option, e.g. `-v, --verbose...` or `    --pure` (a long-only
/// flag is indented to align with the `-x, ` short-flag prefix).
fn flag_spec(arg: &clap::Arg) -> String {
    let short = arg.get_short().map(|c| format!("-{c}"));
    let long = arg.get_long().map(|l| format!("--{l}"));
    let mut spec = match (short, long) {
        (Some(short), Some(long)) => format!("{short}, {long}"),
        (Some(short), None) => short,
        (None, Some(long)) => format!("    {long}"),
        (None, None) => String::new(),
    };
    if matches!(arg.get_action(), clap::ArgAction::Count) {
        spec.push_str("...");
    }
    spec
}

/// The concise one-line description for the shortlist: the first line of a (possibly multi-line)
/// description. The whole first line is kept — a mid-line `". "` sentence break is NOT cut, so a
/// description whose first sentence ends early (e.g. `Do X. Then Y.`) is not truncated to just
/// `Do X.`. Subsequent paragraphs of a long `about` are still dropped (they belong to
/// `cortex help <command>`), keeping the shortlist to one line per entry.
fn short_desc(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_owned()
}

/// Left-justify `text` (pad trailing spaces) to a minimum visible width.
fn pad(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if width > len {
        format!("{text}{}", " ".repeat(width - len))
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for e in chars.by_ref() {
                    if e == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn fg(color: style::Rgb) -> String {
        format!("\x1b[38;2;{};{};{}m", color.0, color.1, color.2)
    }

    #[test]
    fn detects_only_top_level_help() {
        assert!(top_level_help_requested(["--help"]));
        assert!(top_level_help_requested(["-h"]));
        assert!(top_level_help_requested(["-v", "--help"]));
        assert!(top_level_help_requested(["--pure", "-h"]));
        // A subcommand before the flag is subcommand help → clap handles it.
        assert!(!top_level_help_requested(["run", "--help"]));
        assert!(!top_level_help_requested(["models", "-h"]));
        // Version and plain invocations are not help.
        assert!(!top_level_help_requested(["--version"]));
        assert!(!top_level_help_requested(std::iter::empty::<&str>()));
    }

    // The combined short-flag cluster forms (`-vh`/`-hv`, and repeats like `-vvh`) request the
    // top-level styled help, at parity with the spelled-out `-v --help` — clap groups `-vh` as
    // `-v -h`, so the styled help must fire for both spellings.
    #[test]
    fn detects_combined_short_flag_help_clusters() {
        assert!(top_level_help_requested(["-vh"]), "-vh is -v -h → styled help");
        assert!(top_level_help_requested(["-hv"]), "-hv is -h -v → styled help");
        assert!(top_level_help_requested(["-vvh"]), "-vvh is -v -v -h → styled help");
        assert!(top_level_help_requested(["-h"]));
        // A bare `-v` cluster with no `h` is not a help request.
        assert!(!top_level_help_requested(["-v"]));
        assert!(!top_level_help_requested(["-vv"]));
        // `V` (version) inside a cluster defers to clap's version, never styled help.
        assert!(!top_level_help_requested(["-vV"]), "-vV requests version, not help");
        // A cluster with an unknown short flag is left to clap to diagnose.
        assert!(!top_level_help_requested(["-vx"]));
        // A subcommand before the cluster keeps subcommand help with clap.
        assert!(!top_level_help_requested(["run", "-vh"]));
    }

    // VAL-CLI-033: tagline + Usage + Options:/Commands: headers in the brand palette.
    #[test]
    fn styled_help_has_brand_sections() {
        let styled = render_help(Styler::new(true));
        let plain = strip_ansi(&styled);
        assert!(
            plain.contains("Cortex Code — terminal coding agent (TUI, headless run, local server)"),
            "tagline present: {plain}"
        );
        assert!(plain.contains("Usage: cortex [OPTIONS] [COMMAND]"), "usage: {plain}");
        assert!(plain.contains("Options:") && plain.contains("Commands:"), "sections: {plain}");
        // `Usage:` is ORANGE; the section headers are AMBER+bold (never clap's stock cyan/green).
        assert!(styled.contains(&format!("{}Usage:\x1b[0m", fg(style::ORANGE))));
        assert!(
            styled.contains("\x1b[1;38;2;242;183;120mCommands:\x1b[0m"),
            "Commands header is AMBER + bold: {styled:?}"
        );
        // Brand styling is truecolor only — never the ANSI-16 hues clap's stock help uses.
        assert!(!styled.contains("\x1b[32m") && !styled.contains("\x1b[36m"), "no stock ANSI-16 hues");
    }

    // VAL-CLI-034: the Commands section lists the real curated subcommands with real descriptions,
    // followed by an `…and N more — run cortex help <command>` overflow line.
    #[test]
    fn styled_help_lists_real_subcommands_and_overflow() {
        let styled = render_help(Styler::new(true));
        let plain = strip_ansi(&styled);
        for name in SHORTLIST {
            assert!(
                plain.lines().any(|l| l.trim_start().starts_with(name)),
                "shortlist names {name}: {plain}"
            );
        }
        // Real descriptions (derive-sourced), e.g. serve/stats.
        assert!(plain.contains("Start the local loopback HTTP/SSE server"), "serve desc: {plain}");
        assert!(plain.contains("Summarize persisted token usage and cost"), "stats desc: {plain}");
        // Overflow hint with a real remaining count.
        assert!(
            plain.contains("more — run cortex help <command>"),
            "overflow hint present: {plain}"
        );
        // The remaining count is total-real-subcommands minus the shown shortlist (11).
        let cmd = Cli::command();
        let total = cmd
            .get_subcommands()
            .filter(|sc| !sc.is_hide_set() && sc.get_name() != "help")
            .count();
        assert!(
            plain.contains(&format!("…and {} more", total - SHORTLIST.len())),
            "overflow count reflects the real subcommand total: {plain}"
        );
        // The `cortex help <command>` tail is accented ORANGE.
        assert!(styled.contains(&format!("{}cortex help <command>\x1b[0m", fg(style::ORANGE))));
    }

    #[test]
    fn disabled_styler_help_is_ansi_free() {
        let plain = render_help(Styler::new(false));
        assert!(!plain.contains('\x1b'), "disabled help carries no ANSI");
        assert!(plain.contains("Usage: cortex [OPTIONS] [COMMAND]"));
    }

    // The shortlist description keeps the full first line and does NOT cut at a mid-line sentence
    // break (`". "`), so meaningful text following an early period is not dropped.
    #[test]
    fn short_desc_keeps_the_full_first_line_without_sentence_truncation() {
        // A description whose first sentence ends early keeps the rest of the line.
        assert_eq!(
            short_desc("Do X. Then Y and Z are configured."),
            "Do X. Then Y and Z are configured.",
        );
        // Only the first line is kept (subsequent long-help paragraphs are dropped).
        assert_eq!(short_desc("First line.\n\nSecond paragraph."), "First line.");
        // A single concise description passes through unchanged (trimmed).
        assert_eq!(short_desc("  Start the local loopback HTTP/SSE server.  "), "Start the local loopback HTTP/SSE server.");
    }
}
