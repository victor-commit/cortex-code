use std::io::{self, IsTerminal, Read, Write};

use cortex_errors::Exit;
use serde::Serialize;

use crate::cli::{OutputFormat, RunArgs};

/// Execute the headless `cortex run` echo/minimal path (pre-LLM).
///
/// Input-source precedence is fixed and deterministic: a positional prompt argument always
/// wins, and stdin is consulted only when no argument is given. When the argument is present,
/// stdin is never read, so the two are never blended and an interactive terminal can never
/// block the command.
///
/// When there is no argument and stdin yields no prompt (`</dev/null`, a closed pipe, or an
/// interactive terminal with nothing piped in), the command fails fast with [`Exit::MissingInput`]
/// and a `no input` diagnostic on stderr rather than hanging.
pub fn run(args: RunArgs) -> Exit {
    let stdin = io::stdin();
    let stdin_is_terminal = stdin.is_terminal();
    let prompt = match resolve_prompt(args.prompt, stdin_is_terminal, stdin.lock()) {
        Ok(Some(prompt)) => prompt,
        Ok(None) => {
            eprintln!("cortex run: no input (pass a prompt argument or pipe text on stdin)");
            return Exit::MissingInput;
        }
        Err(err) => {
            eprintln!("cortex run: failed to read stdin: {err}");
            return Exit::Failure;
        }
    };

    let stdout = io::stdout();
    let format = effective_format(args.jsonl, args.format);
    let colorize = should_colorize(format, stdout.is_terminal(), no_color_requested());
    let rendered = render(&prompt, format, colorize);

    let mut handle = stdout.lock();
    if handle
        .write_all(rendered.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

/// Collapse the two output-selection inputs into a single effective format. The `--jsonl`
/// shorthand is mutually exclusive with `--format` at the arg layer, so it wins only when set.
fn effective_format(jsonl: bool, format: OutputFormat) -> OutputFormat {
    if jsonl { OutputFormat::Jsonl } else { format }
}

/// Decide whether human (text) output may carry ANSI styling.
///
/// Styling is applied only when ALL hold: the format is human `text` (machine `json`/`jsonl`
/// is never colorized), stdout is an interactive TTY (auto-detected, so piped output stays
/// clean), and `NO_COLOR` was not requested (per the NO_COLOR convention).
fn should_colorize(format: OutputFormat, stdout_is_terminal: bool, no_color: bool) -> bool {
    format == OutputFormat::Text && stdout_is_terminal && !no_color
}

/// `NO_COLOR` is honored when present and non-empty, matching <https://no-color.org/>.
fn no_color_requested() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

/// Render the resolved prompt into the bytes written to stdout for the chosen format.
fn render(prompt: &str, format: OutputFormat, colorize: bool) -> String {
    match format {
        OutputFormat::Text => render_text(prompt, colorize),
        OutputFormat::Json => render_json(prompt),
        OutputFormat::Jsonl => render_jsonl(prompt),
    }
}

/// ANSI cyan applied to the echoed line when colorizing; never emitted on a pipe.
const STYLE_PREFIX: &str = "\u{1b}[36m";
const STYLE_SUFFIX: &str = "\u{1b}[0m";

fn render_text(prompt: &str, colorize: bool) -> String {
    if colorize {
        format!("{STYLE_PREFIX}{prompt}{STYLE_SUFFIX}\n")
    } else {
        format!("{prompt}\n")
    }
}

/// The single JSON document emitted by `--format json` on the echo path. The key set is fixed
/// so downstream scripts can rely on it across runs (VAL-SKEL-060).
#[derive(Serialize)]
struct EchoDocument<'a> {
    r#type: &'a str,
    input: &'a str,
    output: &'a str,
}

fn render_json(prompt: &str) -> String {
    let document = EchoDocument {
        r#type: "echo",
        input: prompt,
        output: prompt,
    };
    let mut line = serialize(&document);
    line.push('\n');
    line
}

/// A single newline-delimited event in `--jsonl` output. Internally tagged with `type` so the
/// vocabulary stays stable and discriminable as richer turns are added in later milestones.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum EchoEvent<'a> {
    Text { text: &'a str },
    Done,
}

fn render_jsonl(prompt: &str) -> String {
    let events = [EchoEvent::Text { text: prompt }, EchoEvent::Done];
    let mut out = String::new();
    for event in &events {
        out.push_str(&serialize(event));
        out.push('\n');
    }
    out
}

/// Serialize a value that is statically known to be JSON-serializable (only `&str` fields, so
/// serialization is infallible) into a compact single-line string.
fn serialize<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("echo output is always serializable")
}

/// Resolve the prompt according to the documented precedence (argument over stdin).
///
/// `stdin_is_terminal` is passed in (rather than derived from `source`) because `IsTerminal`
/// is a sealed trait that cannot be implemented for test doubles. Returns `Ok(None)` when no
/// prompt is available.
fn resolve_prompt<R: Read>(
    arg: Option<String>,
    stdin_is_terminal: bool,
    mut source: R,
) -> io::Result<Option<String>> {
    if let Some(arg) = arg {
        return Ok(Some(arg));
    }

    // No argument: read stdin. An interactive terminal with nothing piped in would block
    // forever, so treat it as "no input" instead of reading.
    if stdin_is_terminal {
        return Ok(None);
    }

    let mut buffer = String::new();
    source.read_to_string(&mut buffer)?;
    Ok(normalize_stdin(&buffer))
}

/// Normalize raw stdin into an optional prompt: strip a single trailing line ending (the one
/// a shell pipe/`echo` typically appends) and treat the empty result as "no input".
fn normalize_stdin(raw: &str) -> Option<String> {
    let trimmed = strip_one_trailing_newline(raw);
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Strip at most one trailing line ending (`\n` or `\r\n`), preserving any earlier newlines.
fn strip_one_trailing_newline(s: &str) -> &str {
    match s.strip_suffix('\n') {
        Some(without_lf) => without_lf.strip_suffix('\r').unwrap_or(without_lf),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A non-terminal stdin double (a pipe/redirect), so `resolve_prompt` reads it.
    fn piped(data: &str) -> Cursor<Vec<u8>> {
        Cursor::new(data.as_bytes().to_vec())
    }

    #[test]
    fn strips_single_trailing_lf() {
        assert_eq!(strip_one_trailing_newline("hi\n"), "hi");
    }

    #[test]
    fn strips_single_trailing_crlf() {
        assert_eq!(strip_one_trailing_newline("hi\r\n"), "hi");
    }

    #[test]
    fn keeps_interior_and_extra_newlines() {
        assert_eq!(strip_one_trailing_newline("a\nb\n\n"), "a\nb\n");
        assert_eq!(strip_one_trailing_newline("no newline"), "no newline");
    }

    #[test]
    fn empty_stdin_is_no_input() {
        assert_eq!(normalize_stdin(""), None);
        assert_eq!(normalize_stdin("\n"), None);
        assert_eq!(normalize_stdin("\r\n"), None);
    }

    #[test]
    fn nonempty_stdin_is_normalized() {
        assert_eq!(
            normalize_stdin("ping from stdin\n"),
            Some("ping from stdin".to_owned())
        );
    }

    #[test]
    fn argument_takes_precedence_over_stdin() {
        let resolved = resolve_prompt(Some("FROM_ARG".to_owned()), false, piped("FROM_STDIN\n"))
            .expect("resolve");
        assert_eq!(resolved, Some("FROM_ARG".to_owned()));
    }

    #[test]
    fn argument_wins_even_on_an_interactive_terminal() {
        let resolved =
            resolve_prompt(Some("FROM_ARG".to_owned()), true, piped("")).expect("resolve");
        assert_eq!(resolved, Some("FROM_ARG".to_owned()));
    }

    #[test]
    fn stdin_used_when_no_argument() {
        let resolved = resolve_prompt(None, false, piped("from stdin\n")).expect("resolve");
        assert_eq!(resolved, Some("from stdin".to_owned()));
    }

    #[test]
    fn empty_stdin_without_argument_is_none() {
        let resolved = resolve_prompt(None, false, piped("")).expect("resolve");
        assert_eq!(resolved, None);
    }

    #[test]
    fn interactive_terminal_without_argument_is_none() {
        let resolved = resolve_prompt(None, true, piped("anything\n")).expect("resolve");
        assert_eq!(resolved, None);
    }

    #[test]
    fn empty_argument_is_preserved_as_explicit_prompt() {
        let resolved =
            resolve_prompt(Some(String::new()), false, piped("ignored\n")).expect("resolve");
        assert_eq!(resolved, Some(String::new()));
    }

    #[test]
    fn jsonl_shorthand_overrides_format() {
        assert_eq!(
            effective_format(true, OutputFormat::Text),
            OutputFormat::Jsonl
        );
        assert_eq!(
            effective_format(false, OutputFormat::Json),
            OutputFormat::Json
        );
        assert_eq!(
            effective_format(false, OutputFormat::Text),
            OutputFormat::Text
        );
    }

    #[test]
    fn color_only_for_text_on_a_terminal() {
        // Machine formats are never colorized, regardless of TTY or NO_COLOR.
        assert!(!should_colorize(OutputFormat::Json, true, false));
        assert!(!should_colorize(OutputFormat::Jsonl, true, false));
        // Text colorizes only on a TTY (piped output stays clean) ...
        assert!(!should_colorize(OutputFormat::Text, false, false));
        assert!(should_colorize(OutputFormat::Text, true, false));
        // ... and NO_COLOR disables styling even on a TTY.
        assert!(!should_colorize(OutputFormat::Text, true, true));
    }

    #[test]
    fn text_render_colorizes_only_when_asked() {
        let plain = render_text("hi", false);
        assert_eq!(plain, "hi\n");
        assert!(!plain.contains('\u{1b}'), "plain text carries no ANSI");

        let styled = render_text("hi", true);
        assert!(styled.contains('\u{1b}'), "styled text carries ANSI");
        assert!(styled.starts_with(STYLE_PREFIX) && styled.ends_with("\n"));
        assert!(styled.contains("hi"));
    }

    #[test]
    fn json_render_is_a_single_parseable_object_with_stable_keys() {
        let rendered = render_json("hello \"world\"\nsecond");
        assert!(rendered.ends_with('\n'));
        assert_eq!(rendered.trim_end_matches('\n').lines().count(), 1);
        assert!(!rendered.contains('\u{1b}'), "json is never colorized");

        let value: serde_json::Value =
            serde_json::from_str(rendered.trim_end()).expect("valid JSON");
        assert_eq!(value["type"], "echo");
        assert_eq!(value["input"], "hello \"world\"\nsecond");
        assert_eq!(value["output"], "hello \"world\"\nsecond");

        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["input", "output", "type"]);
    }

    #[test]
    fn jsonl_render_emits_one_object_per_line_ending_with_done() {
        let rendered = render_jsonl("payload");
        assert!(!rendered.contains('\u{1b}'), "jsonl is never colorized");

        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 2, "a text event followed by a done event");
        for line in &lines {
            let value: serde_json::Value = serde_json::from_str(line).expect("each line is JSON");
            assert!(value.is_object());
        }
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["type"], "text");
        assert_eq!(first["text"], "payload");
        let last: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(last["type"], "done");
    }
}
