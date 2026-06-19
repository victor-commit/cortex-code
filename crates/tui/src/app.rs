//! Application state and key handling for the Cortex Code TUI scaffold.
//!
//! The prompt's editing model is the ported `tui_widgets::TextArea`, which provides
//! readline-style editing and multi-line text. Rendering is done with `ratatui`
//! (see [`crate::ui`]); this module only owns state and translates key events.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use engine::RunnerEvent;
use llm::{Message, Usage};
use tui_widgets::{CursorPosition, TextArea, TextAreaKey, TextAreaModifiers};

use crate::turn::TurnMessage;

/// Status line contents reported in the footer.
#[derive(Debug, Clone)]
pub struct Status {
    /// Active model id (e.g. `google/gemini-2.5-flash`).
    pub model: String,
    /// Active agent name.
    pub agent: String,
    /// Active mode.
    pub mode: String,
    /// Session indicator (e.g. `new` for an unsaved, empty session).
    pub session: String,
}

/// The in-progress assistant turn's two distinct text channels: model reasoning (kept separate from
/// and rendered distinctly from the answer) and the visible answer text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssistantTurn {
    /// Model reasoning/thinking, rendered in a distinct labeled region. Empty when none is emitted,
    /// so no empty-reasoning artifact is shown.
    pub reasoning: String,
    /// The assistant-visible answer text, grown incrementally as deltas arrive.
    pub answer: String,
}

/// One rendered transcript entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A submitted user prompt.
    User(String),
    /// An assistant reply (its reasoning region and answer text).
    Assistant(AssistantTurn),
    /// A turn-level error surfaced to the user.
    Error(String),
}

/// Cumulative token/cost usage shown in the footer, accumulated across the session's turns.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageSummary {
    /// Whether any turn has reported usage yet (controls the before/after footer rendering).
    pub reported: bool,
    /// Cumulative input/prompt tokens.
    pub input_tokens: u64,
    /// Cumulative output/completion tokens.
    pub output_tokens: u64,
    /// Cumulative total tokens.
    pub total_tokens: u64,
    /// Cumulative cost, when any turn reported one.
    pub cost: Option<f64>,
}

impl UsageSummary {
    /// Fold one turn's usage into the running totals.
    fn accumulate(&mut self, usage: &Usage) {
        self.reported = true;
        self.input_tokens += u64::from(usage.input_tokens);
        self.output_tokens += u64::from(usage.output_tokens);
        self.total_tokens += u64::from(usage.total_tokens);
        if let Some(cost) = usage.cost {
            self.cost = Some(self.cost.unwrap_or(0.0) + cost);
        }
    }
}

/// The interactive TUI application state.
pub struct App {
    /// The bottom prompt's editing model.
    pub input: TextArea,
    /// Footer status contents.
    pub status: Status,
    /// Set once the user requests to quit (`/exit`, `/quit`, or `Ctrl+C`).
    pub should_quit: bool,
    /// The rendered conversation so far (user prompts, assistant replies, errors).
    pub transcript: Vec<Entry>,
    /// Cumulative usage shown in the footer.
    pub usage: UsageSummary,
    /// A prompt accepted by `submit` and waiting for the event loop to start a turn.
    pending_prompt: Option<String>,
    /// The active turn's event channel, present only while a turn is in flight.
    turn_rx: Option<Receiver<TurnMessage>>,
    /// When the active turn started, used to animate the busy indicator. `None` when idle.
    spinner_start: Option<Instant>,
    /// The user prompt of the in-flight turn, appended to history once the turn finishes.
    turn_prompt: Option<String>,
    /// Whether the in-flight turn already surfaced an error (so finalization skips the answer).
    turn_errored: bool,
    /// Canonical prior-turn messages projected into each new turn for conversational context.
    history: Vec<Message>,
}

impl App {
    /// Creates a fresh application with an empty prompt and the given status.
    pub fn new(status: Status) -> Self {
        Self {
            input: TextArea::new(),
            status,
            should_quit: false,
            transcript: Vec::new(),
            usage: UsageSummary::default(),
            pending_prompt: None,
            turn_rx: None,
            spinner_start: None,
            turn_prompt: None,
            turn_errored: false,
            history: Vec::new(),
        }
    }

    /// Whether a turn is currently streaming (the busy/spinner indicator is shown).
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.turn_rx.is_some()
    }

    /// The animated busy-spinner glyph for the current instant, or `None` when idle.
    #[must_use]
    pub fn spinner_glyph(&self) -> Option<&'static str> {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let start = self.spinner_start?;
        let frame = (start.elapsed().as_millis() / 100) as usize % FRAMES.len();
        Some(FRAMES[frame])
    }

    /// A snapshot of the conversational history sent with the next turn.
    #[must_use]
    pub fn history_snapshot(&self) -> Vec<Message> {
        self.history.clone()
    }

    /// Take the prompt accepted by `submit` (if any), clearing it so a turn starts exactly once.
    #[must_use]
    pub fn take_pending_prompt(&mut self) -> Option<String> {
        self.pending_prompt.take()
    }

    /// Begin a streaming turn for `prompt`: record the user message, open an in-progress assistant
    /// entry, attach the worker's event channel, and start the busy indicator.
    pub fn begin_turn(&mut self, prompt: String, rx: Receiver<TurnMessage>) {
        self.transcript.push(Entry::User(prompt.clone()));
        self.transcript
            .push(Entry::Assistant(AssistantTurn::default()));
        self.turn_prompt = Some(prompt);
        self.turn_errored = false;
        self.turn_rx = Some(rx);
        self.spinner_start = Some(Instant::now());
    }

    /// Drain every pending turn message from the worker, applying each to the transcript, usage,
    /// and busy state. Returns whether any message was applied (so the caller can redraw).
    pub fn poll_turn(&mut self) -> bool {
        let mut applied = false;
        loop {
            let message = match self.turn_rx.as_ref() {
                Some(rx) => rx.try_recv(),
                None => return applied,
            };
            match message {
                Ok(message) => {
                    self.apply(message);
                    applied = true;
                }
                Err(TryRecvError::Empty) => return applied,
                Err(TryRecvError::Disconnected) => {
                    // The worker vanished without a Finished marker; finalize defensively.
                    self.finish_turn();
                    return true;
                }
            }
        }
    }

    /// Apply a single turn message to the application state.
    fn apply(&mut self, message: TurnMessage) {
        match message {
            TurnMessage::Event(event) => self.apply_event(event),
            TurnMessage::SetupError(message) => self.set_turn_error(message),
            TurnMessage::Finished => self.finish_turn(),
        }
    }

    /// Apply one normalized runner event.
    fn apply_event(&mut self, event: RunnerEvent) {
        match event {
            RunnerEvent::TextDelta { text } => self.append_answer(&text),
            RunnerEvent::Reasoning { text } => self.append_reasoning(&text),
            RunnerEvent::Usage(usage) => self.usage.accumulate(&usage),
            RunnerEvent::Error { message, .. } => self.set_turn_error(message),
            // Tool calls/results and the terminal Done are not surfaced by this view; the worker's
            // Finished marker drives turn finalization.
            RunnerEvent::ToolCall { .. }
            | RunnerEvent::ToolResult { .. }
            | RunnerEvent::Done { .. } => {}
        }
    }

    /// Append a streamed answer fragment to the in-progress assistant entry.
    fn append_answer(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.transcript.last_mut() {
            Some(Entry::Assistant(turn)) => turn.answer.push_str(text),
            _ => self.transcript.push(Entry::Assistant(AssistantTurn {
                reasoning: String::new(),
                answer: text.to_owned(),
            })),
        }
    }

    /// Append a streamed reasoning fragment to the in-progress assistant entry's reasoning region.
    fn append_reasoning(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.transcript.last_mut() {
            Some(Entry::Assistant(turn)) => turn.reasoning.push_str(text),
            _ => self.transcript.push(Entry::Assistant(AssistantTurn {
                reasoning: text.to_owned(),
                answer: String::new(),
            })),
        }
    }

    /// Surface a turn error: drop an empty in-progress assistant entry, then record the error.
    fn set_turn_error(&mut self, message: String) {
        if let Some(Entry::Assistant(turn)) = self.transcript.last() {
            if turn.answer.is_empty() && turn.reasoning.is_empty() {
                self.transcript.pop();
            }
        }
        self.transcript.push(Entry::Error(message));
        self.turn_errored = true;
    }

    /// Finalize a completed turn: clear the busy indicator and fold the exchange into history so the
    /// next turn carries conversational context.
    fn finish_turn(&mut self) {
        self.turn_rx = None;
        self.spinner_start = None;
        let prompt = self.turn_prompt.take();
        let errored = std::mem::take(&mut self.turn_errored);
        if let Some(prompt) = prompt {
            self.history.push(Message::user(prompt));
            if !errored {
                if let Some(Entry::Assistant(turn)) = self.transcript.last() {
                    if !turn.answer.is_empty() {
                        self.history.push(Message::assistant(turn.answer.clone()));
                    }
                }
            }
        }
    }

    /// The current prompt text (all lines joined with `\n`).
    pub fn input_value(&self) -> String {
        self.input.value()
    }

    /// The logical cursor position within the prompt.
    pub fn cursor(&self) -> CursorPosition {
        self.input.cursor_position()
    }

    /// Handles a single key press, mutating prompt or app state as appropriate.
    ///
    /// Editing keys (characters, backspace, arrows, `Ctrl+A`/`Ctrl+E`, `Home`/`End`)
    /// only affect the prompt. `Enter` submits, `Shift+Enter` inserts a soft newline,
    /// and `Ctrl+C` quits.
    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        match key.code {
            // Quit keybind.
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            // Readline caret movement: Ctrl+A -> line start, Ctrl+E -> line end.
            KeyCode::Char('a') if ctrl => {
                self.input.handle_key(TextAreaKey::Home, modifiers(false));
            }
            KeyCode::Char('e') if ctrl => {
                self.input.handle_key(TextAreaKey::End, modifiers(false));
            }
            // Ordinary text entry (ignore other ctrl/alt chords).
            KeyCode::Char(c) if !ctrl && !alt => self.input.insert_char(c),
            // Shift+Enter inserts a soft newline and grows the input without submitting.
            KeyCode::Enter if shift => self.input.insert_newline(),
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.delete_backward();
            }
            KeyCode::Delete => {
                self.input.delete_forward();
            }
            KeyCode::Left => {
                self.input.handle_key(TextAreaKey::Left, modifiers(shift));
            }
            KeyCode::Right => {
                self.input.handle_key(TextAreaKey::Right, modifiers(shift));
            }
            KeyCode::Up => {
                self.input.handle_key(TextAreaKey::Up, modifiers(shift));
            }
            KeyCode::Down => {
                self.input.handle_key(TextAreaKey::Down, modifiers(shift));
            }
            KeyCode::Home => {
                self.input.handle_key(TextAreaKey::Home, modifiers(shift));
            }
            KeyCode::End => {
                self.input.handle_key(TextAreaKey::End, modifiers(shift));
            }
            _ => {}
        }
    }

    /// Handles `Enter` on the current prompt. Slash exit commands quit; any other non-empty text is
    /// accepted as a [pending prompt](App::take_pending_prompt) the event loop turns into a
    /// streaming turn. Submitting is ignored while a turn is already in flight.
    fn submit(&mut self) {
        let value = self.input.value();
        let trimmed = value.trim();
        if trimmed == "/exit" || trimmed == "/quit" {
            self.should_quit = true;
            return;
        }
        if self.is_busy() || trimmed.is_empty() {
            return;
        }
        self.pending_prompt = Some(trimmed.to_owned());
        self.input.set_value("");
    }
}

/// Builds [`TextAreaModifiers`] from a shift flag (ctrl/alt are never forwarded here).
fn modifiers(shift: bool) -> TextAreaModifiers {
    TextAreaModifiers::new(shift, false, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_mod(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    fn app() -> App {
        App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            session: "new".to_owned(),
        })
    }

    #[test]
    fn typing_echoes_into_the_prompt() {
        let mut a = app();
        for c in "hello world".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.input_value(), "hello world");
        assert!(!a.should_quit);
    }

    #[test]
    fn backspace_deletes_last_characters() {
        let mut a = app();
        for c in "hello world".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        for _ in 0..5 {
            a.on_key(key(KeyCode::Backspace));
        }
        assert_eq!(a.input_value(), "hello ");
    }

    #[test]
    fn left_then_insert_moves_caret() {
        let mut a = app();
        for c in "abc".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Left));
        a.on_key(key(KeyCode::Char('X')));
        // X inserted before the final 'c'.
        assert_eq!(a.input_value(), "abXc");
    }

    #[test]
    fn ctrl_a_and_ctrl_e_move_to_line_bounds() {
        let mut a = app();
        for c in "word".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key_mod(KeyCode::Char('a'), KeyModifiers::CONTROL));
        a.on_key(key(KeyCode::Char('>')));
        assert_eq!(a.input_value(), ">word");
        a.on_key(key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL));
        a.on_key(key(KeyCode::Char('<')));
        assert_eq!(a.input_value(), ">word<");
    }

    #[test]
    fn shift_enter_inserts_soft_newline_without_submitting() {
        let mut a = app();
        for c in "line one".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key_mod(KeyCode::Enter, KeyModifiers::SHIFT));
        for c in "line two".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.input_value(), "line one\nline two");
        assert_eq!(a.input.line_count(), 2);
        assert!(!a.should_quit);
    }

    #[test]
    fn enter_submits_and_clears_plain_text() {
        let mut a = app();
        for c in "hi".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.input_value(), "");
        assert!(!a.should_quit);
    }

    #[test]
    fn slash_exit_quits() {
        let mut a = app();
        for c in "/exit".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));
        assert!(a.should_quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.should_quit);
    }

    #[test]
    fn text_never_leaks_before_submit() {
        // Typed characters live only in the prompt model; there is no transcript
        // state in the scaffold, so nothing can leak. Assert the value is the only sink.
        let mut a = app();
        for c in "secret".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.input_value(), "secret");
    }

    fn usage(input: u32, output: u32, cost: Option<f64>) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
            cost,
        }
    }

    fn last_assistant(app: &App) -> &AssistantTurn {
        match app.transcript.last() {
            Some(Entry::Assistant(turn)) => turn,
            other => panic!("expected an assistant entry, got {other:?}"),
        }
    }

    #[test]
    fn enter_sets_a_pending_prompt() {
        let mut a = app();
        for c in "hi there".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.take_pending_prompt().as_deref(), Some("hi there"));
        // Taken exactly once.
        assert!(a.take_pending_prompt().is_none());
    }

    #[test]
    fn submit_is_ignored_while_a_turn_is_in_flight() {
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("first".to_owned(), rx);
        assert!(a.is_busy());
        for c in "second".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));
        // Nothing was submitted and the typed text is preserved.
        assert!(a.take_pending_prompt().is_none());
        assert_eq!(a.input_value(), "second");
    }

    #[test]
    fn streamed_deltas_grow_the_assistant_answer() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx);

        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "Hello".to_owned(),
        }))
        .unwrap();
        a.poll_turn();
        // An early snapshot shows partial text.
        assert_eq!(last_assistant(&a).answer, "Hello");

        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: ", world.".to_owned(),
        }))
        .unwrap();
        a.poll_turn();
        // A later snapshot shows strictly more text.
        assert_eq!(last_assistant(&a).answer, "Hello, world.");
    }

    #[test]
    fn reasoning_is_kept_distinct_from_the_answer() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx);

        tx.send(TurnMessage::Event(RunnerEvent::Reasoning {
            text: "let me think".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "the answer".to_owned(),
        }))
        .unwrap();
        a.poll_turn();

        let turn = last_assistant(&a);
        assert_eq!(turn.reasoning, "let me think");
        assert_eq!(turn.answer, "the answer");
    }

    #[test]
    fn no_reasoning_leaves_an_empty_region() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx);
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "plain answer".to_owned(),
        }))
        .unwrap();
        a.poll_turn();
        // Reasoning stays empty so the UI shows no empty-reasoning artifact.
        assert!(last_assistant(&a).reasoning.is_empty());
    }

    #[test]
    fn usage_appears_and_increments_after_a_turn() {
        let mut a = app();
        assert!(!a.usage.reported, "no usage before any turn");

        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx);
        tx.send(TurnMessage::Event(RunnerEvent::Usage(usage(
            10,
            20,
            Some(0.001),
        ))))
        .unwrap();
        a.poll_turn();

        assert!(a.usage.reported);
        assert_eq!(a.usage.input_tokens, 10);
        assert_eq!(a.usage.output_tokens, 20);
        assert_eq!(a.usage.total_tokens, 30);
        assert_eq!(a.usage.cost, Some(0.001));
    }

    #[test]
    fn busy_indicator_shows_during_a_turn_and_clears_on_finish() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx);
        assert!(a.is_busy());
        assert!(a.spinner_glyph().is_some());

        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "done".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        assert!(!a.is_busy(), "spinner clears after completion");
        assert!(a.spinner_glyph().is_none());
        assert_eq!(last_assistant(&a).answer, "done");
    }

    #[test]
    fn finished_turn_is_folded_into_history() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("question".to_owned(), rx);
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "reply".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        let history = a.history_snapshot();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "question");
        assert_eq!(history[1].content, "reply");
    }

    #[test]
    fn a_turn_error_is_surfaced_and_kept_out_of_history() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx);
        tx.send(TurnMessage::Event(RunnerEvent::Error {
            message: "provider exploded".to_owned(),
            kind: "upstream".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        assert!(!a.is_busy());
        assert!(matches!(a.transcript.last(), Some(Entry::Error(m)) if m == "provider exploded"));
        // The empty assistant entry was dropped, and only the user message reached history.
        let history = a.history_snapshot();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "q");
    }

    #[test]
    fn setup_error_is_surfaced_as_an_error_entry() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx);
        tx.send(TurnMessage::SetupError("no credential".to_owned()))
            .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        assert!(!a.is_busy());
        assert!(matches!(a.transcript.last(), Some(Entry::Error(m)) if m == "no credential"));
    }
}
