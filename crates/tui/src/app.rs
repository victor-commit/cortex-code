//! Application state and key handling for the Cortex Code TUI scaffold.
//!
//! The prompt's editing model is the ported `tui_widgets::TextArea`, which provides
//! readline-style editing and multi-line text. Rendering is done with `ratatui`
//! (see [`crate::ui`]); this module only owns state and translates key events.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use engine::RunnerEvent;
use llm::{Message, Usage};
use tui_widgets::{CursorPosition, TextArea, TextAreaKey, TextAreaModifiers};

use crate::slash;
use crate::tool_card::ToolCard;
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
    /// A file changed by a tool this turn, rendered as a colored unified diff.
    Diff(crate::diff::FileDiff),
    /// An agent tool call rendered as a labeled card (running → completed/failed, expandable).
    Tool(ToolCard),
    /// A system notice (e.g. a dispatched slash command's acknowledgment, or an
    /// unknown-command message) shown distinctly from chat content.
    Notice(String),
    /// The `/help` command reference: the built-in slash commands and keybindings, rendered as a
    /// distinct panel. Its content is generated at render time from the command registry, so the
    /// entry itself carries no data.
    Help,
    /// A turn-level error surfaced to the user.
    Error(String),
}

/// The instruction sent to the model when the `/init` flow runs (project initialization).
const INIT_PROMPT: &str = "Initialize this project. Analyze the repository structure and the \
    important files, then create (or update) an AGENTS.md file at the project root that documents \
    the project's purpose, layout, key build/test/lint commands, and conventions. Read the \
    existing files first so the summary is accurate.";

/// The instruction sent to the model when the `/review` flow runs (code review).
const REVIEW_PROMPT: &str = "Review the current state of the code in this repository. Inspect the \
    recent changes and the relevant files, then report any bugs, correctness problems, security \
    issues, and concrete improvements you find, grouped by severity.";

/// The instruction sent to the model when the `/compact` flow runs (conversation compaction).
const COMPACTION_PROMPT: &str = "Summarize our conversation so far into a concise summary that \
    preserves the key facts, decisions, code changes, and any open tasks. Respond with only the \
    summary.";

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
    /// A prompt accepted by `submit` (or a slash flow) and waiting for the event loop to start a
    /// turn. For a flow this is the expanded instruction sent to the model, which may differ from
    /// the concise header shown in the transcript ([`App::pending_header`]).
    pending_prompt: Option<String>,
    /// The user-visible header for the queued turn, set by a slash flow so the transcript shows a
    /// concise label (e.g. `/init`) rather than the verbose instruction. `None` for a plain chat
    /// prompt, where the typed text is shown verbatim.
    pending_header: Option<String>,
    /// Whether the queued turn compacts the conversation into its summary on completion (set by the
    /// `/compact` flow). Consumed when the turn begins.
    pending_compaction: bool,
    /// The active turn's event channel, present only while a turn is in flight.
    turn_rx: Option<Receiver<TurnMessage>>,
    /// When the active turn started, used to animate the busy indicator. `None` when idle.
    spinner_start: Option<Instant>,
    /// The user prompt of the in-flight turn, appended to history once the turn finishes.
    turn_prompt: Option<String>,
    /// Whether the in-flight turn already surfaced an error (so finalization skips the answer).
    turn_errored: bool,
    /// Whether the in-flight turn is a compaction (its summary replaces the conversation history
    /// when it finishes). Set from [`App::pending_compaction`] when the turn begins.
    turn_compaction: bool,
    /// Whether tool cards are expanded to reveal their full arguments and output. Toggled by the
    /// expand/collapse keybind; applies to every tool card so the detail view is unambiguous.
    pub expand_tools: bool,
    /// Highlighted row in the open slash-command menu (index into the filtered matches). Reset to
    /// the top whenever the typed query changes; only meaningful while the menu is open.
    slash_selected: usize,
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
            pending_header: None,
            pending_compaction: false,
            turn_rx: None,
            spinner_start: None,
            turn_prompt: None,
            turn_errored: false,
            turn_compaction: false,
            expand_tools: false,
            slash_selected: 0,
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
    ///
    /// A slash flow may set a concise [header](App::pending_header) (shown in the transcript) and
    /// the [compaction](App::pending_compaction) flag (so the turn's summary replaces history when
    /// it finishes); both are consumed here. A plain chat prompt shows its text verbatim.
    pub fn begin_turn(&mut self, prompt: String, rx: Receiver<TurnMessage>) {
        let header = self.pending_header.take().unwrap_or_else(|| prompt.clone());
        self.transcript.push(Entry::User(header));
        self.transcript
            .push(Entry::Assistant(AssistantTurn::default()));
        self.turn_prompt = Some(prompt);
        self.turn_errored = false;
        self.turn_compaction = std::mem::take(&mut self.pending_compaction);
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
            TurnMessage::FileDiff(diff) => self.push_diff(*diff),
            TurnMessage::SetupError(message) => self.set_turn_error(message),
            TurnMessage::Finished => self.finish_turn(),
        }
    }

    /// Record a file-change diff in the transcript. A trailing empty in-progress assistant entry
    /// (opened when the turn began but not yet written to) is dropped first, so the diff is not
    /// preceded by an empty assistant block; any answer text that arrives after the tool call opens
    /// a fresh assistant entry.
    fn push_diff(&mut self, diff: crate::diff::FileDiff) {
        if let Some(Entry::Assistant(turn)) = self.transcript.last() {
            if turn.answer.is_empty() && turn.reasoning.is_empty() {
                self.transcript.pop();
            }
        }
        self.transcript.push(Entry::Diff(diff));
    }

    /// Apply one normalized runner event.
    fn apply_event(&mut self, event: RunnerEvent) {
        match event {
            RunnerEvent::TextDelta { text } => self.append_answer(&text),
            RunnerEvent::Reasoning { text } => self.append_reasoning(&text),
            RunnerEvent::Usage(usage) => self.usage.accumulate(&usage),
            RunnerEvent::Error { message, .. } => self.set_turn_error(message),
            RunnerEvent::ToolCall {
                id,
                name,
                arguments,
            } => self.push_tool_call(id, name, &arguments),
            RunnerEvent::ToolResult {
                id,
                parts,
                is_error,
                ..
            } => self.complete_tool_call(&id, &parts, is_error),
            // The terminal Done is not surfaced by this view; the worker's Finished marker drives
            // turn finalization.
            RunnerEvent::Done { .. } => {}
        }
    }

    /// Open a running tool card for a just-started tool call. A trailing empty in-progress assistant
    /// entry (opened when the turn began but not yet written to) is dropped first, so the card is not
    /// preceded by an empty assistant block; answer text arriving after the call opens a fresh entry.
    fn push_tool_call(&mut self, id: String, name: String, arguments: &str) {
        if let Some(Entry::Assistant(turn)) = self.transcript.last() {
            if turn.answer.is_empty() && turn.reasoning.is_empty() {
                self.transcript.pop();
            }
        }
        self.transcript
            .push(Entry::Tool(ToolCard::new(id, name, arguments)));
    }

    /// Transition the running tool card matching `id` to its terminal (completed/failed) state,
    /// recording the result text. The most recent matching card wins so re-used ids never collide.
    fn complete_tool_call(&mut self, id: &str, parts: &[protocol::ToolResultPart], is_error: bool) {
        if let Some(card) = self
            .transcript
            .iter_mut()
            .rev()
            .find_map(|entry| match entry {
                Entry::Tool(card) if card.id == id => Some(card),
                _ => None,
            })
        {
            card.complete(parts, is_error);
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
    ///
    /// A compaction turn (`/compact`) instead replaces the conversation history with its summary so
    /// subsequent turns carry a compact context, and annotates the transcript so the compaction is
    /// visible. A failed turn of either kind leaves history untouched.
    fn finish_turn(&mut self) {
        self.turn_rx = None;
        self.spinner_start = None;
        let prompt = self.turn_prompt.take();
        let errored = std::mem::take(&mut self.turn_errored);
        let compaction = std::mem::take(&mut self.turn_compaction);

        if compaction {
            if !errored {
                if let Some(summary) = self.latest_assistant_answer() {
                    self.history = vec![Message::system(format!(
                        "Summary of the conversation so far:\n{summary}"
                    ))];
                    self.transcript.push(Entry::Notice(
                        "Conversation compacted into a summary.".to_owned(),
                    ));
                }
            }
            return;
        }

        if let Some(prompt) = prompt {
            self.history.push(Message::user(prompt));
            if !errored {
                if let Some(answer) = self.latest_assistant_answer() {
                    self.history.push(Message::assistant(answer));
                }
            }
        }
    }

    /// The most recent assistant entry's answer text, if any. A tool/diff entry may sit after the
    /// answer, so this scans back rather than reading only the last entry.
    fn latest_assistant_answer(&self) -> Option<String> {
        self.transcript.iter().rev().find_map(|entry| match entry {
            Entry::Assistant(turn) if !turn.answer.is_empty() => Some(turn.answer.clone()),
            _ => None,
        })
    }

    /// The current prompt text (all lines joined with `\n`).
    pub fn input_value(&self) -> String {
        self.input.value()
    }

    /// The logical cursor position within the prompt.
    pub fn cursor(&self) -> CursorPosition {
        self.input.cursor_position()
    }

    /// The slash-command query currently being typed, or `None` when the prompt is not a
    /// single-token slash command.
    ///
    /// The menu is active only when the whole prompt starts with `/` and contains no whitespace
    /// (so `/`, `/mod` are queries while `hello`, `/foo bar`, or a `/` mid-line are not). The query
    /// is the text after the leading `/` (empty for a bare `/`).
    #[must_use]
    pub fn slash_query(&self) -> Option<String> {
        let value = self.input.value();
        let rest = value.strip_prefix('/')?;
        if rest.chars().any(char::is_whitespace) {
            return None;
        }
        Some(rest.to_owned())
    }

    /// Whether the slash-command autocomplete menu is currently open.
    #[must_use]
    pub fn slash_menu_open(&self) -> bool {
        self.slash_query().is_some()
    }

    /// The commands matching the current query, in highlight order. Empty when the menu is closed
    /// or nothing matches.
    #[must_use]
    pub fn slash_matches(&self) -> Vec<&'static slash::SlashCommand> {
        match self.slash_query() {
            Some(query) => slash::filter(&query),
            None => Vec::new(),
        }
    }

    /// The highlighted match index, clamped to the current match count.
    #[must_use]
    pub fn slash_selected(&self) -> usize {
        let len = self.slash_matches().len();
        if len == 0 {
            0
        } else {
            self.slash_selected.min(len - 1)
        }
    }

    /// Move the highlight to the next match (wrapping at the end).
    fn slash_select_next(&mut self) {
        let len = self.slash_matches().len();
        if len == 0 {
            return;
        }
        self.slash_selected = (self.slash_selected() + 1) % len;
    }

    /// Move the highlight to the previous match (wrapping at the start).
    fn slash_select_prev(&mut self) {
        let len = self.slash_matches().len();
        if len == 0 {
            return;
        }
        self.slash_selected = (self.slash_selected() + len - 1) % len;
    }

    /// Close the slash menu and return to a clean, editable prompt (Esc).
    fn close_slash_menu(&mut self) {
        self.input.set_value("");
        self.slash_selected = 0;
    }

    /// Dispatch the highlighted slash command (Enter). With matches, the highlighted command runs
    /// and the prompt clears; with no match, a clear "unknown command" notice is surfaced instead
    /// of submitting the bogus text to the model.
    fn dispatch_slash(&mut self) {
        let matches = self.slash_matches();
        if matches.is_empty() {
            let typed = self.input.value();
            self.transcript
                .push(Entry::Notice(format!("Unknown command: {typed}")));
            self.close_slash_menu();
            return;
        }
        let command = matches[self.slash_selected()];
        self.close_slash_menu();
        self.dispatch_command(command);
    }

    /// Apply a dispatched slash command's effect. The action/flow built-ins (`/help`, `/new`,
    /// `/init`, `/review`, `/compact`, `/export`, `/exit`) run their handler or flow; the remaining
    /// built-ins (pickers/dialogs wired by other features) surface a notice acknowledging the
    /// dispatch.
    fn dispatch_command(&mut self, command: &'static slash::SlashCommand) {
        match command.name {
            "exit" => self.should_quit = true,
            "help" => self.transcript.push(Entry::Help),
            "new" => self.new_session(),
            "export" => self.export_session(),
            "init" => self.start_flow(
                "/init",
                "/init — analyzing the repository to create AGENTS.md…",
                INIT_PROMPT,
            ),
            "review" => self.start_flow(
                "/review",
                "/review — reviewing the current code…",
                REVIEW_PROMPT,
            ),
            "compact" => self.start_compaction(),
            _ => self.transcript.push(Entry::Notice(format!(
                "/{} — {}",
                command.name, command.description
            ))),
        }
    }

    /// Start a fresh session (`/new`): clear the transcript, the conversational history, and the
    /// usage totals so the view returns to the empty home state. Ignored while a turn streams.
    fn new_session(&mut self) {
        if self.is_busy() {
            return;
        }
        self.transcript.clear();
        self.history.clear();
        self.usage = UsageSummary::default();
    }

    /// Queue a slash flow turn (`/init`, `/review`): push a notice describing the flow, then queue
    /// the expanded instruction (shown in the transcript under the concise `header`). The event
    /// loop turns the queued prompt into a real streaming turn. Ignored while a turn streams.
    fn start_flow(&mut self, header: &str, notice: &str, prompt: &str) {
        if self.is_busy() {
            return;
        }
        self.transcript.push(Entry::Notice(notice.to_owned()));
        self.pending_header = Some(header.to_owned());
        self.pending_prompt = Some(prompt.to_owned());
    }

    /// Start conversation compaction (`/compact`): queue a summarization turn whose summary replaces
    /// the history when it finishes. Ignored while a turn streams, and a no-op (with a notice) when
    /// there is no conversation yet to compact.
    fn start_compaction(&mut self) {
        if self.is_busy() {
            return;
        }
        if self.history.is_empty() {
            self.transcript.push(Entry::Notice(
                "Nothing to compact yet — start a conversation first.".to_owned(),
            ));
            return;
        }
        self.transcript.push(Entry::Notice(
            "/compact — compacting the conversation…".to_owned(),
        ));
        self.pending_header = Some("/compact".to_owned());
        self.pending_prompt = Some(COMPACTION_PROMPT.to_owned());
        self.pending_compaction = true;
    }

    /// Export the current session (`/export`): render the transcript to Markdown, write it to a
    /// file under the data directory's `exports/` folder, and confirm with the path. A write
    /// failure surfaces a clear error instead of crashing.
    fn export_session(&mut self) {
        let markdown = self.render_session_markdown();
        match write_export(&markdown) {
            Ok(path) => self.transcript.push(Entry::Notice(format!(
                "Exported session to {}",
                path.display()
            ))),
            Err(err) => self
                .transcript
                .push(Entry::Error(format!("Export failed: {err}"))),
        }
    }

    /// Render the current transcript to a self-contained Markdown document for `/export`.
    fn render_session_markdown(&self) -> String {
        let mut out = String::from("# Cortex Code session export\n\n");
        for entry in &self.transcript {
            match entry {
                Entry::User(text) => out.push_str(&format!("## You\n\n{text}\n\n")),
                Entry::Assistant(turn) => {
                    out.push_str("## Cortex\n\n");
                    if !turn.reasoning.trim().is_empty() {
                        out.push_str(&format!("> reasoning: {}\n\n", turn.reasoning.trim()));
                    }
                    if !turn.answer.trim().is_empty() {
                        out.push_str(&format!("{}\n\n", turn.answer.trim()));
                    }
                }
                Entry::Diff(diff) => {
                    out.push_str(&format!("### Change: {}\n\n", diff.path));
                }
                Entry::Tool(card) => {
                    out.push_str(&format!("### Tool: {} — {}\n\n", card.name, card.summary));
                }
                Entry::Notice(text) => out.push_str(&format!("> {text}\n\n")),
                Entry::Help => {}
                Entry::Error(text) => out.push_str(&format!("> Error: {text}\n\n")),
            }
        }
        out
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

        // While the slash menu is open it owns vertical navigation, dispatch, and dismissal so the
        // arrows move the highlight (not the caret) and Enter runs the command (not submit). All
        // other keys fall through to ordinary prompt editing, which re-filters the menu.
        if self.slash_menu_open() {
            match key.code {
                KeyCode::Up => {
                    self.slash_select_prev();
                    return;
                }
                KeyCode::Down => {
                    self.slash_select_next();
                    return;
                }
                KeyCode::Enter if !shift => {
                    self.dispatch_slash();
                    return;
                }
                KeyCode::Esc => {
                    self.close_slash_menu();
                    return;
                }
                _ => {}
            }
        }

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
            // Expand/collapse tool cards to reveal or hide their full arguments and output.
            KeyCode::Char('o') if ctrl => self.expand_tools = !self.expand_tools,
            // Ordinary text entry (ignore other ctrl/alt chords). Editing the prompt re-filters the
            // slash menu, so reset the highlight to the top match.
            KeyCode::Char(c) if !ctrl && !alt => {
                self.input.insert_char(c);
                self.slash_selected = 0;
            }
            // Shift+Enter inserts a soft newline and grows the input without submitting.
            KeyCode::Enter if shift => self.input.insert_newline(),
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.delete_backward();
                self.slash_selected = 0;
            }
            KeyCode::Delete => {
                self.input.delete_forward();
                self.slash_selected = 0;
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

/// Write a session export Markdown document to a timestamped file under the data directory's
/// `exports/` folder, returning the path written. The directory is created if needed.
fn write_export(markdown: &str) -> std::io::Result<PathBuf> {
    let dir = export_dir();
    std::fs::create_dir_all(&dir)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = dir.join(format!("cortex-session-{nanos}.md"));
    std::fs::write(&path, markdown)?;
    Ok(path)
}

/// Resolve the export directory, mirroring the data-directory precedence used elsewhere:
/// `CORTEX_DATA_DIR/exports`, else `XDG_DATA_HOME/cortex/exports`, else
/// `$HOME/.local/share/cortex/exports`, else a `cortex-exports` folder in the OS temp dir.
fn export_dir() -> PathBuf {
    if let Some(dir) = env_non_empty("CORTEX_DATA_DIR") {
        return PathBuf::from(dir).join("exports");
    }
    if let Some(dir) = env_non_empty("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("cortex").join("exports");
    }
    if let Some(home) = env_non_empty("HOME") {
        return PathBuf::from(home).join(".local/share/cortex/exports");
    }
    std::env::temp_dir().join("cortex-exports")
}

/// A trimmed, non-empty environment variable value, or `None`.
fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
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

    #[test]
    fn a_file_diff_is_recorded_in_the_transcript() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("edit the file".to_owned(), rx);

        let diff =
            crate::diff::compute_file_diff("src/main.rs", Some("a\nb\nc\n"), Some("a\nB\nc\n"))
                .expect("a diff");
        tx.send(TurnMessage::FileDiff(Box::new(diff))).unwrap();
        a.poll_turn();

        // The empty assistant entry opened at turn start is replaced by the diff entry (no empty
        // assistant block precedes it).
        match a.transcript.last() {
            Some(Entry::Diff(diff)) => {
                assert_eq!(diff.path, "src/main.rs");
                assert!(!diff.is_new);
            }
            other => panic!("expected a diff entry, got {other:?}"),
        }
        assert!(
            !a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Assistant(t) if t.answer.is_empty() && t.reasoning.is_empty())),
            "no empty assistant block should precede the diff"
        );
    }

    fn tool_part(text: &str) -> protocol::ToolResultPart {
        protocol::ToolResultPart::text(text)
    }

    fn last_tool(app: &App) -> &crate::tool_card::ToolCard {
        app.transcript
            .iter()
            .rev()
            .find_map(|e| match e {
                Entry::Tool(card) => Some(card),
                _ => None,
            })
            .expect("a tool card")
    }

    #[test]
    fn a_tool_call_opens_a_running_card_then_completes() {
        use crate::tool_card::ToolStatus;
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("read the file".to_owned(), rx);

        tx.send(TurnMessage::Event(RunnerEvent::ToolCall {
            id: "call_1".to_owned(),
            name: "read".to_owned(),
            arguments: r#"{"path":"src/main.rs"}"#.to_owned(),
        }))
        .unwrap();
        a.poll_turn();
        // A mid-execution snapshot shows the card in the running state, labeled and summarized.
        let card = last_tool(&a);
        assert_eq!(card.status, ToolStatus::Running);
        assert_eq!(card.name, "read");
        assert_eq!(card.summary, "src/main.rs");
        // The empty assistant block opened at turn start does not precede the card.
        assert!(
            !a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Assistant(t) if t.answer.is_empty() && t.reasoning.is_empty())),
            "no empty assistant block precedes the tool card"
        );

        tx.send(TurnMessage::Event(RunnerEvent::ToolResult {
            id: "call_1".to_owned(),
            name: "read".to_owned(),
            parts: vec![tool_part("1: fn main() {}")],
            is_error: false,
        }))
        .unwrap();
        a.poll_turn();
        // A later snapshot shows the same card completed with its output recorded.
        let card = last_tool(&a);
        assert_eq!(card.status, ToolStatus::Completed);
        assert!(card.output.contains("fn main"));
    }

    #[test]
    fn a_failed_tool_call_renders_an_error_card() {
        use crate::tool_card::ToolStatus;
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("read a missing file".to_owned(), rx);

        tx.send(TurnMessage::Event(RunnerEvent::ToolCall {
            id: "c".to_owned(),
            name: "read".to_owned(),
            arguments: r#"{"path":"nope.txt"}"#.to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Event(RunnerEvent::ToolResult {
            id: "c".to_owned(),
            name: "read".to_owned(),
            parts: vec![tool_part("file not found: nope.txt")],
            is_error: true,
        }))
        .unwrap();
        a.poll_turn();

        let card = last_tool(&a);
        assert_eq!(card.status, ToolStatus::Failed);
        assert!(card.output.contains("not found"));
    }

    #[test]
    fn ctrl_o_toggles_tool_card_expansion() {
        let mut a = app();
        assert!(!a.expand_tools);
        a.on_key(key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(a.expand_tools, "Ctrl+O expands");
        a.on_key(key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(!a.expand_tools, "Ctrl+O again collapses");
    }

    #[test]
    fn tool_result_for_unknown_id_is_ignored() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx);
        // A result with no matching open card must not panic or fabricate a card.
        tx.send(TurnMessage::Event(RunnerEvent::ToolResult {
            id: "missing".to_owned(),
            name: "read".to_owned(),
            parts: vec![tool_part("x")],
            is_error: false,
        }))
        .unwrap();
        a.poll_turn();
        assert!(!a.transcript.iter().any(|e| matches!(e, Entry::Tool(_))));
    }

    #[test]
    fn answer_after_a_diff_opens_a_fresh_assistant_entry_and_reaches_history() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("create a file".to_owned(), rx);

        let diff =
            crate::diff::compute_file_diff("new.txt", None, Some("hello\n")).expect("a diff");
        tx.send(TurnMessage::FileDiff(Box::new(diff))).unwrap();
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "Created the file.".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        // Order: user, diff, assistant answer.
        assert!(matches!(a.transcript[1], Entry::Diff(_)));
        assert!(
            matches!(a.transcript.last(), Some(Entry::Assistant(t)) if t.answer == "Created the file.")
        );
        // The post-diff answer is folded into history.
        let history = a.history_snapshot();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].content, "Created the file.");
    }

    fn type_str(a: &mut App, text: &str) {
        for c in text.chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_slash_on_empty_prompt_opens_the_menu() {
        // VAL-TUI-028: `/` on an empty prompt opens a menu of recognizable built-ins.
        let mut a = app();
        a.on_key(key(KeyCode::Char('/')));
        assert!(a.slash_menu_open(), "menu open after '/'");
        let names: Vec<&str> = a.slash_matches().iter().map(|c| c.name).collect();
        assert!(names.contains(&"help"));
        assert!(names.contains(&"new"));
        assert!(names.contains(&"models"));
    }

    #[test]
    fn slash_menu_not_opened_by_a_mid_text_slash() {
        let mut a = app();
        type_str(&mut a, "abc/");
        assert!(!a.slash_menu_open(), "a `/` after text is plain input");
    }

    #[test]
    fn typing_after_slash_filters_the_menu() {
        // VAL-TUI-029: continuing to type narrows the list to matching commands.
        let mut a = app();
        type_str(&mut a, "/mod");
        let names: Vec<&str> = a.slash_matches().iter().map(|c| c.name).collect();
        assert!(names.contains(&"models"), "models present: {names:?}");
        assert!(!names.contains(&"help"), "help filtered out: {names:?}");
    }

    #[test]
    fn arrow_keys_move_the_highlight_without_editing_the_prompt() {
        // VAL-TUI-030: arrows move the selection (not the caret) while the menu is open.
        let mut a = app();
        a.on_key(key(KeyCode::Char('/')));
        assert_eq!(a.slash_selected(), 0);
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.slash_selected(), 1, "Down moves the highlight");
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.slash_selected(), 2);
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.slash_selected(), 1, "Up moves the highlight back");
        // The prompt text is untouched by navigation.
        assert_eq!(a.input_value(), "/");
    }

    #[test]
    fn highlight_wraps_around_at_the_ends() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('/')));
        let count = a.slash_matches().len();
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.slash_selected(), count - 1, "Up at top wraps to bottom");
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.slash_selected(), 0, "Down at bottom wraps to top");
    }

    #[test]
    fn enter_dispatches_the_highlighted_command_and_clears_the_prompt() {
        // VAL-TUI-030: Enter dispatches the highlighted command (its effect appears) rather than
        // inserting literal text or submitting to the model.
        let mut a = app();
        type_str(&mut a, "/mod");
        a.on_key(key(KeyCode::Enter));
        // No chat turn was queued.
        assert!(a.take_pending_prompt().is_none(), "not submitted to model");
        // The prompt is cleared and the menu closed.
        assert_eq!(a.input_value(), "");
        assert!(!a.slash_menu_open());
        // A visible notice records the dispatched command.
        match a.transcript.last() {
            Some(Entry::Notice(text)) => assert!(text.contains("/models"), "notice: {text}"),
            other => panic!("expected a dispatch notice, got {other:?}"),
        }
    }

    #[test]
    fn enter_after_moving_highlight_dispatches_the_selected_command() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('/')));
        a.on_key(key(KeyCode::Down)); // help -> new
        a.on_key(key(KeyCode::Down)); // new -> sessions
        a.on_key(key(KeyCode::Enter));
        match a.transcript.last() {
            Some(Entry::Notice(text)) => assert!(text.contains("/sessions"), "notice: {text}"),
            other => panic!("expected a dispatch notice, got {other:?}"),
        }
    }

    #[test]
    fn dispatching_exit_quits() {
        let mut a = app();
        type_str(&mut a, "/exit");
        a.on_key(key(KeyCode::Enter));
        assert!(a.should_quit, "/exit dispatch quits");
    }

    #[test]
    fn esc_closes_the_menu_and_returns_to_an_editable_prompt() {
        // VAL-TUI-031: Esc dismisses the menu; the prompt then accepts typing again.
        let mut a = app();
        type_str(&mut a, "/mod");
        assert!(a.slash_menu_open());
        a.on_key(key(KeyCode::Esc));
        assert!(!a.slash_menu_open(), "menu closed after Esc");
        assert_eq!(a.input_value(), "", "prompt returned to a clean state");
        // Typing works again afterwards.
        type_str(&mut a, "hello");
        assert_eq!(a.input_value(), "hello");
        assert!(!a.slash_menu_open());
    }

    #[test]
    fn unknown_command_is_handled_gracefully() {
        // VAL-TUI-048: a nonexistent command shows a no-match menu and, on Enter, a clear unknown
        // message instead of crashing or submitting it to the model.
        let mut a = app();
        type_str(&mut a, "/zzznotreal");
        assert!(a.slash_menu_open(), "menu still open");
        assert!(
            a.slash_matches().is_empty(),
            "no matches for a bogus command"
        );
        a.on_key(key(KeyCode::Enter));
        assert!(a.take_pending_prompt().is_none(), "not submitted to model");
        assert_eq!(a.input_value(), "");
        match a.transcript.last() {
            Some(Entry::Notice(text)) => {
                assert!(text.contains("Unknown command"), "clear message: {text}");
                assert!(text.contains("/zzznotreal"));
            }
            other => panic!("expected an unknown-command notice, got {other:?}"),
        }
    }

    #[test]
    fn editing_the_query_resets_the_highlight() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('/')));
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.slash_selected(), 2);
        // Typing another character re-filters and returns to the top match.
        a.on_key(key(KeyCode::Char('m')));
        assert_eq!(a.slash_selected(), 0);
    }

    #[test]
    fn a_plain_prompt_still_submits_normally() {
        let mut a = app();
        type_str(&mut a, "just chatting");
        assert!(!a.slash_menu_open());
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.take_pending_prompt().as_deref(), Some("just chatting"));
    }

    /// Run a complete throwaway turn so the app accumulates transcript + history state.
    fn run_turn(a: &mut App, prompt: &str, answer: &str) {
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(prompt.to_owned(), rx);
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: answer.to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();
    }

    #[test]
    fn dispatching_help_shows_a_help_entry() {
        // VAL-TUI-032: `/help` shows a help / command reference.
        let mut a = app();
        type_str(&mut a, "/help");
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.take_pending_prompt().is_none(),
            "help is not a model turn"
        );
        assert!(
            matches!(a.transcript.last(), Some(Entry::Help)),
            "a help entry is shown: {:?}",
            a.transcript.last()
        );
    }

    #[test]
    fn dispatching_new_clears_the_session() {
        // VAL-TUI-033: `/new` starts a fresh session (empty transcript + history).
        let mut a = app();
        run_turn(&mut a, "remember the word marmalade", "noted");
        assert!(!a.transcript.is_empty(), "a populated transcript first");
        assert!(!a.history_snapshot().is_empty(), "history populated first");

        type_str(&mut a, "/new");
        a.on_key(key(KeyCode::Enter));

        assert!(a.transcript.is_empty(), "transcript cleared after /new");
        assert!(
            a.history_snapshot().is_empty(),
            "history cleared after /new"
        );
        assert!(!a.usage.reported, "usage reset after /new");
    }

    #[test]
    fn dispatching_init_queues_a_real_flow_turn() {
        // VAL-TUI-038: `/init` triggers project initialization (a real turn, not an ack).
        let mut a = app();
        type_str(&mut a, "/init");
        a.on_key(key(KeyCode::Enter));

        // A notice marks the flow starting.
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(t) if t.contains("/init"))),
            "an /init flow notice is shown"
        );
        // The expanded instruction is queued for the model.
        let prompt = a.take_pending_prompt().expect("an /init turn is queued");
        assert!(
            prompt.contains("AGENTS.md"),
            "the init instruction is sent: {prompt}"
        );
        // The transcript shows the concise header, not the verbose instruction.
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(prompt, rx);
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::User(t) if t == "/init")),
            "the concise /init header is shown as the user message"
        );
    }

    #[test]
    fn dispatching_review_queues_a_real_flow_turn() {
        // VAL-TUI-039: `/review` triggers a code-review flow.
        let mut a = app();
        type_str(&mut a, "/review");
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(t) if t.contains("/review"))),
            "a /review flow notice is shown"
        );
        let prompt = a.take_pending_prompt().expect("a /review turn is queued");
        assert!(
            prompt.to_lowercase().contains("review"),
            "the review instruction is sent: {prompt}"
        );
    }

    #[test]
    fn dispatching_compact_queues_a_compaction_turn_that_replaces_history() {
        // VAL-TUI-040: `/compact` compacts the conversation.
        let mut a = app();
        run_turn(&mut a, "fact one", "ok");
        run_turn(&mut a, "fact two", "ok");
        assert_eq!(a.history_snapshot().len(), 4, "two turns of history");

        type_str(&mut a, "/compact");
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(t) if t.contains("compacting"))),
            "a compaction-in-progress marker is shown"
        );

        let prompt = a
            .take_pending_prompt()
            .expect("a compaction turn is queued");
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(prompt, rx);
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "Summary: discussed fact one and fact two.".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        // History is replaced by a single compact summary message.
        let history = a.history_snapshot();
        assert_eq!(history.len(), 1, "history compacted to one message");
        assert!(
            history[0].content.contains("Summary: discussed"),
            "the summary becomes the new context: {}",
            history[0].content
        );
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(t) if t.contains("compacted"))),
            "a compaction-complete marker is shown"
        );
    }

    #[test]
    fn compact_with_no_history_is_a_clear_no_op() {
        let mut a = app();
        type_str(&mut a, "/compact");
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.take_pending_prompt().is_none(),
            "no turn is queued without a conversation"
        );
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(t) if t.contains("Nothing to compact"))),
            "a clear no-op notice is shown"
        );
    }

    #[test]
    fn dispatching_export_confirms_with_a_path() {
        // VAL-TUI-041: `/export` exports the current session and confirms with a path.
        let mut a = app();
        run_turn(&mut a, "hello", "hi there");
        type_str(&mut a, "/export");
        a.on_key(key(KeyCode::Enter));
        match a.transcript.last() {
            Some(Entry::Notice(text)) => {
                assert!(
                    text.contains("Exported session to"),
                    "an export confirmation with a path: {text}"
                );
            }
            other => panic!("expected an export confirmation notice, got {other:?}"),
        }
    }

    #[test]
    fn session_markdown_captures_the_exchange() {
        let mut a = app();
        a.transcript.push(Entry::User("a question".to_owned()));
        a.transcript.push(Entry::Assistant(AssistantTurn {
            reasoning: String::new(),
            answer: "an answer".to_owned(),
        }));
        let md = a.render_session_markdown();
        assert!(md.contains("# Cortex Code session export"));
        assert!(md.contains("## You") && md.contains("a question"));
        assert!(md.contains("## Cortex") && md.contains("an answer"));
    }

    #[test]
    fn flows_are_ignored_while_a_turn_is_in_flight() {
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("busy".to_owned(), rx);
        assert!(a.is_busy());
        type_str(&mut a, "/init");
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.take_pending_prompt().is_none(),
            "no flow turn is queued while busy"
        );
    }
}
