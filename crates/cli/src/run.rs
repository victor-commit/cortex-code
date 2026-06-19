//! The headless `cortex run` path: a single real, streamed turn against OpenRouter.
//!
//! A positional prompt argument is sent to the durable [`SessionRunner`](engine::SessionRunner)
//! and the assistant's answer is streamed to stdout as it arrives. The default output is clean
//! human prose (the streamed answer text only — no JSON envelope or event scaffolding); `--format
//! json` emits a single document built from the completed turn, and `--jsonl` streams the runner's
//! normalized event vocabulary one object per line.
//!
//! Input-source precedence is fixed and deterministic: a positional prompt argument always wins,
//! and stdin is consulted only when no argument is given. When no prompt is available at all
//! (`</dev/null`, a closed pipe, or an interactive terminal with nothing piped in) the command
//! fails fast with [`Exit::MissingInput`] rather than hanging.
//!
//! The `CORTEX_NO_NET` kill-switch is consulted before a credential is resolved or any socket is
//! opened, so a run fails closed with a `net-disabled` diagnostic. When the credential is absent
//! the run fails with the canonical `no-credential` diagnostic; in neither case is any partial
//! answer written to stdout.

use std::io::{self, IsTerminal, Read, Write};

use engine::{EnvironmentBlock, RunOutcome, RunnerEvent, SessionRunner, SystemContext, Turn};
use errors::Exit;
use redaction::guard_outbound;
use serde::Serialize;

use crate::cli::{OutputFormat, RunArgs};

/// The outbound operation named in a `net-disabled` refusal raised before the turn starts.
const OUTBOUND_OPERATION: &str = "LLM turn";

/// Execute a single headless `cortex run` turn.
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

    let format = effective_format(args.jsonl, args.format);

    // Fail closed on the network kill-switch before resolving a credential or opening any socket,
    // so a refusal never touches the network even when a key is present.
    if let Err(diag) = guard_outbound(OUTBOUND_OPERATION) {
        eprintln!("cortex run: {diag}");
        return diag.exit();
    }

    let resolved = providers::resolve_model(args.model.as_deref());
    let inputs = providers::CredentialInputs::openrouter_from_process(None);
    let provider = match providers::resolve_openrouter(&inputs) {
        Ok(provider) => provider,
        Err(diag) => {
            eprintln!("cortex run: {diag}");
            return diag.exit();
        }
    };

    let system =
        SystemContext::for_model(&resolved.model_id, EnvironmentBlock::from_process()).render();
    let turn = Turn::new(&resolved.model_id, system, prompt);

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex run: failed to start the async runtime: {err}");
            return Exit::Failure;
        }
    };

    runtime.block_on(execute_turn(&provider, &resolved.model_id, turn, format))
}

/// Drive the turn through the runner and render its events in the requested format.
async fn execute_turn(
    provider: &dyn llm::Provider,
    model_id: &str,
    turn: Turn,
    format: OutputFormat,
) -> Exit {
    match format {
        OutputFormat::Text => stream_text(provider, turn).await,
        OutputFormat::Jsonl => stream_jsonl(provider, turn).await,
        OutputFormat::Json => emit_json(provider, model_id, turn).await,
    }
}

/// Stream the assistant answer as clean prose, flushing each fragment as it arrives so output is
/// incremental rather than a single end-of-turn dump.
async fn stream_text(provider: &dyn llm::Provider, turn: Turn) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let mut wrote_any = false;

    let outcome = SessionRunner::default()
        .run(provider, turn, |event| {
            if let RunnerEvent::TextDelta { text } = event {
                if !text.is_empty() {
                    let _ = handle.write_all(text.as_bytes());
                    let _ = handle.flush();
                    wrote_any = true;
                }
            }
        })
        .await;

    if let Some(err) = &outcome.error {
        if wrote_any {
            let _ = handle.write_all(b"\n");
            let _ = handle.flush();
        }
        drop(handle);
        eprintln!("cortex run: {err}");
        return err.exit();
    }

    // Terminate the streamed answer with a newline so shell capture and the next prompt are clean,
    // without disturbing the model's own trailing newline if it already emitted one.
    if wrote_any && !outcome.answer().ends_with('\n') {
        let _ = handle.write_all(b"\n");
        let _ = handle.flush();
    }
    Exit::Success
}

/// Stream the runner's normalized event vocabulary, one JSON object per line.
async fn stream_jsonl(provider: &dyn llm::Provider, turn: Turn) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    let outcome = SessionRunner::default()
        .run(provider, turn, |event| {
            if let Ok(line) = render_event_line(&event) {
                let _ = handle.write_all(line.as_bytes());
                let _ = handle.write_all(b"\n");
                let _ = handle.flush();
            }
        })
        .await;

    drop(handle);
    if let Some(err) = &outcome.error {
        eprintln!("cortex run: {err}");
        return err.exit();
    }
    Exit::Success
}

/// A single JSON document describing a completed turn for `--format json`.
#[derive(Serialize)]
struct RunDocument<'a> {
    role: &'a str,
    content: &'a str,
    model: &'a str,
    finish_reason: &'a llm::FinishReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<&'a llm::Usage>,
}

/// Run the turn to completion, then emit a single strict JSON document on stdout. No partial
/// document is written if the turn fails.
async fn emit_json(provider: &dyn llm::Provider, model_id: &str, turn: Turn) -> Exit {
    let outcome = SessionRunner::default().run(provider, turn, |_| {}).await;

    if let Some(err) = &outcome.error {
        eprintln!("cortex run: {err}");
        return err.exit();
    }

    let line = match render_json_document(model_id, &outcome) {
        Ok(line) => line,
        Err(err) => {
            eprintln!("cortex run: failed to serialize output: {err}");
            return Exit::Failure;
        }
    };

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(line.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

/// Build the single JSON document describing a completed turn (role, content, model, finish
/// reason, and the optional usage object).
fn build_run_document<'a>(model_id: &'a str, outcome: &'a RunOutcome) -> RunDocument<'a> {
    RunDocument {
        role: "assistant",
        content: outcome.answer(),
        model: model_id,
        finish_reason: &outcome.finish_reason,
        usage: outcome.usage.as_ref(),
    }
}

/// Serialize the completed-turn document to a single line terminated by exactly one newline, so
/// stdout carries one strict JSON value with no leading or trailing non-JSON bytes.
fn render_json_document(model_id: &str, outcome: &RunOutcome) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(&build_run_document(model_id, outcome))?;
    line.push('\n');
    Ok(line)
}

/// Render one normalized runner event as a single JSONL line (without the trailing newline). Every
/// event serializes to an object carrying a stable `type` discriminator.
fn render_event_line(event: &RunnerEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event)
}

/// Collapse the two output-selection inputs into a single effective format. The `--jsonl`
/// shorthand is mutually exclusive with `--format` at the arg layer, so it wins only when set.
fn effective_format(jsonl: bool, format: OutputFormat) -> OutputFormat {
    if jsonl { OutputFormat::Jsonl } else { format }
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
    fn unicode_stdin_is_preserved_without_corruption() {
        assert_eq!(
            normalize_stdin("日本語-café-🚀\n"),
            Some("日本語-café-🚀".to_owned())
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

    /// A completed-turn outcome with assistant text and integer usage, mirroring a real turn.
    fn sample_outcome() -> RunOutcome {
        RunOutcome {
            message: llm::Message::assistant("Hello! How can I help you today?"),
            usage: Some(llm::Usage {
                input_tokens: 9,
                output_tokens: 8,
                total_tokens: 17,
                cost: Some(0.0000056),
            }),
            finish_reason: llm::FinishReason::Stop,
            steps: 1,
            error: None,
        }
    }

    // VAL-LLM-005: `--format json` emits exactly one strict JSON value with a single trailing
    // newline and no other framing bytes, so a strict parser accepts it whole.
    #[test]
    fn json_document_is_a_single_strict_value_with_one_trailing_newline() {
        let rendered =
            render_json_document("google/gemini-2.5-flash", &sample_outcome()).expect("render");
        assert!(rendered.ends_with('\n'), "document must end with a newline");
        assert_eq!(
            rendered.matches('\n').count(),
            1,
            "the document is a single line (no interleaved newlines)"
        );
        let value: serde_json::Value =
            serde_json::from_str(&rendered).expect("strict JSON parse of the whole document");
        assert!(value.is_object(), "the document is a single JSON object");
    }

    // VAL-LLM-004 / VAL-LLM-006: the document carries the assistant role, the model text as
    // content, the model id, and a usage object with integer input/output token counts.
    #[test]
    fn json_document_carries_assistant_role_content_model_and_integer_usage() {
        let rendered =
            render_json_document("google/gemini-2.5-flash", &sample_outcome()).expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(value["role"], "assistant");
        assert_eq!(value["content"], "Hello! How can I help you today?");
        assert_eq!(value["model"], "google/gemini-2.5-flash");
        assert_eq!(value["finish_reason"], "stop");
        assert_eq!(value["usage"]["input_tokens"].as_u64(), Some(9));
        assert_eq!(value["usage"]["output_tokens"].as_u64(), Some(8));
        assert!(
            value["usage"]["input_tokens"].is_u64() && value["usage"]["output_tokens"].is_u64(),
            "token counts must be integers"
        );
    }

    // The usage object is optional: when the provider reports none, it is omitted yet the document
    // stays a valid, strictly-parseable JSON object with the assistant role and content.
    #[test]
    fn json_document_omits_usage_when_absent_but_stays_valid() {
        let mut outcome = sample_outcome();
        outcome.usage = None;
        let rendered = render_json_document("m", &outcome).expect("render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(value["role"], "assistant");
        assert_eq!(value["content"], "Hello! How can I help you today?");
        assert!(value.get("usage").is_none(), "absent usage is omitted");
    }

    // VAL-LLM-007 / VAL-LLM-008: the JSONL stream renders >=2 independently-parseable lines, each
    // carrying a stable `type` discriminator; at least one text event and a terminal usage/done are
    // present, the stream ends with the terminal event, and concatenated text deltas reconstruct
    // the answer.
    #[test]
    fn jsonl_lines_are_typed_terminate_with_done_and_reconstruct_the_answer() {
        let events = [
            RunnerEvent::TextDelta {
                text: "Hello".to_owned(),
            },
            RunnerEvent::TextDelta {
                text: ", world!".to_owned(),
            },
            RunnerEvent::Usage(llm::Usage {
                input_tokens: 5,
                output_tokens: 3,
                total_tokens: 8,
                cost: None,
            }),
            RunnerEvent::Done {
                reason: llm::FinishReason::Stop,
            },
        ];
        let lines: Vec<String> = events
            .iter()
            .map(|event| render_event_line(event).expect("render"))
            .collect();
        assert!(lines.len() >= 2, "incremental events span >=2 lines");

        let mut reconstructed = String::new();
        let mut types = Vec::new();
        for line in &lines {
            let value: serde_json::Value =
                serde_json::from_str(line).expect("each line parses as JSON");
            let kind = value["type"]
                .as_str()
                .expect("every event carries a `type` discriminator")
                .to_owned();
            if kind == "text_delta" {
                reconstructed.push_str(value["text"].as_str().expect("text fragment"));
            }
            types.push(kind);
        }
        assert_eq!(
            reconstructed, "Hello, world!",
            "concatenated text deltas reconstruct the answer"
        );
        assert!(
            types.iter().any(|t| t == "text_delta"),
            "at least one text event present"
        );
        assert!(
            types.iter().any(|t| t == "usage" || t == "done"),
            "a usage/final/done event present"
        );
        assert_eq!(
            types.last().map(String::as_str),
            Some("done"),
            "the stream ends with the terminal event"
        );
    }
}
