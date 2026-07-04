//! The `cortex replay` subcommand: reconstruct a persisted transcript from SQLite.
//!
//! Replay is a local persistence read only. It never calls the model or opens network sockets, and
//! it works identically under `CORTEX_NO_NET=1`.

use std::io::{self, Write};

use db::{Database, DbError, DbPathInputs, Message};
use errors::Exit;
use serde::Serialize;

use crate::cli::{OutputFormat, ReplayArgs};
use crate::style::{self, Styler};

/// Dispatch a `cortex replay` invocation.
pub fn run(args: ReplayArgs) -> Exit {
    let db = match open_database() {
        Ok(db) => db,
        Err(exit) => return exit,
    };

    let session = match db.sessions().require(&args.id) {
        Ok(session) => session,
        Err(err) => return db_error(&err),
    };
    let messages = match db.messages().list_for_session(&session.id) {
        Ok(messages) => messages,
        Err(err) => return db_error(&err),
    };

    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&ReplayJson {
            session_id: session.id,
            messages: messages
                .iter()
                .map(|message| ReplayMessage {
                    role: message.role.clone(),
                    text: message.content.clone(),
                })
                .collect(),
        }),
        OutputFormat::Text => {
            // On a TTY the transcript is re-rendered with colored `[role]` marks + a hanging
            // indent and a session summary footer; off a TTY the historical `[role] content\n`
            // format is emitted byte-identical to the machine baseline (VAL-CLI-029..032).
            let styler = Styler::for_stdout();
            if styler.is_enabled() {
                emit_text(&render_text_styled(
                    &session.id,
                    &messages,
                    styler,
                    style::terminal_width(),
                ))
            } else {
                emit_text(&render_text(&messages))
            }
        }
    }
}

fn open_database() -> Result<Database, Exit> {
    let inputs = DbPathInputs::from_process();
    let Some(path) = inputs.resolve() else {
        eprintln!(
            "cortex replay: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Err(Exit::Failure);
    };
    Database::open(&path).map_err(|err| db_error(&err))
}

#[derive(Serialize)]
struct ReplayJson {
    session_id: String,
    messages: Vec<ReplayMessage>,
}

#[derive(Serialize)]
struct ReplayMessage {
    role: String,
    text: String,
}

fn render_text(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        out.push_str(&format!("[{}] {}\n", message.role, message.content));
    }
    out
}

/// Render the human/TTY transcript: each message as `[role] content` with the brackets GHOST, the
/// role colored (user AMBER, assistant ORANGE), and the body TXT; wrapped continuation lines are
/// hang-indented to align under the content column. A FAINT `session … · N messages · N turns`
/// footer closes the view. Runs only when the styler is enabled (a real TTY); [`render_text`]
/// carries the machine bytes.
fn render_text_styled(
    session_id: &str,
    messages: &[Message],
    styler: Styler,
    width: usize,
) -> String {
    let mut out = String::new();
    for message in messages {
        let role_color = match message.role.as_str() {
            "user" => style::AMBER,
            "assistant" => style::ORANGE,
            _ => style::DIM,
        };
        // `[role] ` prefix width sets the hanging indent for continuation lines.
        let prefix_w = 1 + message.role.chars().count() + 2;
        let indent = " ".repeat(prefix_w);
        let avail = width.saturating_sub(prefix_w).max(1);
        let wrapped = wrap_words(&message.content, avail);

        out.push_str(&styler.paint("[", style::GHOST));
        out.push_str(&styler.paint(&message.role, role_color));
        out.push_str(&styler.paint("]", style::GHOST));
        out.push(' ');
        out.push_str(&styler.paint(wrapped.first().map_or("", String::as_str), style::TXT));
        out.push('\n');
        for continuation in wrapped.iter().skip(1) {
            out.push_str(&indent);
            out.push_str(&styler.paint(continuation, style::TXT));
            out.push('\n');
        }
    }

    let turns = messages
        .iter()
        .filter(|message| message.role == "assistant")
        .count();
    out.push('\n');
    let footer = format!(
        "session {} {} {} messages {} {} turns",
        session_id,
        style::SEPARATOR,
        messages.len(),
        style::SEPARATOR,
        turns,
    );
    out.push_str(&styler.paint(&footer, style::FAINT));
    out.push('\n');
    out
}

/// Greedy word-wrap `text` to a maximum visible `width`, treating existing newlines as hard breaks.
/// A single word longer than `width` is left intact (no mid-word break).
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for segment in text.split('\n') {
        let mut current = String::new();
        let mut current_w = 0usize;
        for word in segment.split(' ') {
            let word_w = word.chars().count();
            if current.is_empty() {
                current.push_str(word);
                current_w = word_w;
            } else if current_w + 1 + word_w <= width {
                current.push(' ');
                current.push_str(word);
                current_w += 1 + word_w;
            } else {
                lines.push(std::mem::take(&mut current));
                current.push_str(word);
                current_w = word_w;
            }
        }
        lines.push(current);
    }
    lines
}

fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex replay: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

fn db_error(err: &DbError) -> Exit {
    eprintln!("cortex replay: {err}");
    Exit::Failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> Message {
        Message {
            id: format!("msg_{role}"),
            session_id: "ses_1".to_owned(),
            role: role.to_owned(),
            content: content.to_owned(),
            parts: Vec::new(),
            created_at: "2026-01-01 00:00:00".to_owned(),
        }
    }

    fn fg(color: style::Rgb) -> String {
        format!("\x1b[38;2;{};{};{}m", color.0, color.1, color.2)
    }

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

    #[test]
    fn text_output_preserves_role_labels_and_order() {
        let messages = vec![
            Message {
                id: "msg_1".to_owned(),
                session_id: "ses_1".to_owned(),
                role: "user".to_owned(),
                content: "first".to_owned(),
                parts: Vec::new(),
                created_at: "2026-01-01 00:00:00".to_owned(),
            },
            Message {
                id: "msg_2".to_owned(),
                session_id: "ses_1".to_owned(),
                role: "assistant".to_owned(),
                content: "second".to_owned(),
                parts: Vec::new(),
                created_at: "2026-01-01 00:00:01".to_owned(),
            },
        ];
        let rendered = render_text(&messages);
        assert_eq!(rendered, "[user] first\n[assistant] second\n");
    }

    // VAL-CLI-029: brackets GHOST, role colored (user AMBER, assistant ORANGE), body TXT.
    #[test]
    fn styled_replay_colors_role_rows() {
        let messages = vec![message("user", "hello"), message("assistant", "hi there")];
        let styled = render_text_styled("ses_abc", &messages, Styler::new(true), 92);
        assert!(styled.contains(&format!("{}[\x1b[0m", fg(style::GHOST))), "bracket GHOST");
        assert!(styled.contains(&format!("{}user\x1b[0m", fg(style::AMBER))), "user role AMBER");
        assert!(
            styled.contains(&format!("{}assistant\x1b[0m", fg(style::ORANGE))),
            "assistant role ORANGE"
        );
        assert!(styled.contains(&format!("{}hello\x1b[0m", fg(style::TXT))), "body TXT");
    }

    // VAL-CLI-030: wrapped continuation lines are indented to align under the content column
    // (the first character after `[role] `).
    #[test]
    fn styled_replay_hang_indents_continuation() {
        let long = "one two three four five six seven eight nine ten eleven twelve";
        let messages = vec![message("assistant", long)];
        // A narrow width forces wrapping.
        let styled = render_text_styled("ses_abc", &messages, Styler::new(true), 24);
        let plain = strip_ansi(&styled);
        let lines: Vec<&str> = plain.lines().collect();
        // First content line starts after `[assistant] ` (12 cols); continuations align there.
        let prefix_w = "[assistant] ".len();
        let continuation = lines
            .iter()
            .find(|l| l.starts_with(&" ".repeat(prefix_w)) && !l.trim().is_empty())
            .expect("a hang-indented continuation line");
        assert_eq!(
            continuation.len() - continuation.trim_start().len(),
            prefix_w,
            "continuation indent aligns under the content column: {continuation:?}"
        );
    }

    // VAL-CLI-031: a FAINT footer reports the real id and message/turn counts.
    #[test]
    fn styled_replay_prints_session_footer() {
        let messages = vec![
            message("user", "a"),
            message("assistant", "b"),
            message("user", "c"),
            message("assistant", "d"),
        ];
        let styled = render_text_styled("ses_7f3a91", &messages, Styler::new(true), 92);
        let plain = strip_ansi(&styled);
        assert!(
            plain.contains("session ses_7f3a91 · 4 messages · 2 turns"),
            "footer: {plain}"
        );
        assert!(
            styled.contains(&format!("{}session ses_7f3a91", fg(style::FAINT))),
            "footer is FAINT"
        );
    }

    // VAL-CLI-032 (styling side): the machine renderer is byte-stable and a disabled styler never
    // routes through the styled path (zero ANSI, single line per message, no footer).
    #[test]
    fn styled_replay_disabled_is_ansi_free_passthrough() {
        let messages = vec![message("user", "first"), message("assistant", "second")];
        let styled = render_text_styled("ses_abc", &messages, Styler::new(false), 92);
        assert!(!styled.contains('\x1b'), "disabled styler emits no ANSI");
        // The machine bytes are unchanged from the historical format.
        assert_eq!(render_text(&messages), "[user] first\n[assistant] second\n");
    }
}
