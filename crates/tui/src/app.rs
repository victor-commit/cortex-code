//! Application state and key handling for the Cortex Code TUI scaffold.
//!
//! The prompt's editing model is the ported `tui_widgets::TextArea`, which provides
//! readline-style editing and multi-line text. Rendering is done with `ratatui`
//! (see [`crate::ui`]); this module only owns state and translates key events.

use std::cell::Cell;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use engine::{Agent, RunnerEvent};
use llm::{Message, Usage};
use tui_components::toast::{Toast, ToastLevel};
use tui_widgets::{CursorPosition, TextArea, TextAreaKey, TextAreaModifiers};

use crate::keybind::{KeyChord, KeybindAction, Keybinds};
use crate::mention;
use crate::shell;
use crate::slash;
use crate::tool_card::ToolCard;
use crate::turn::TurnMessage;

/// The cap on the number of files surfaced in the `@`-mention picker at once, so a broad query (or a
/// bare `@`) cannot build an unbounded row list; the popup scrolls within this set.
const MENTION_MATCH_LIMIT: usize = 200;

/// How long a transient toast stays on screen before it auto-dismisses. Short enough to read as a
/// momentary notification, long enough to be seen; the same window is used for every level so the
/// auto-dismiss timing is predictable.
const TOAST_DURATION: Duration = Duration::from_millis(2500);

/// The most toasts kept at once; older toasts are dropped so a burst of notifications cannot grow
/// without bound (and the overlay never overruns the screen).
const MAX_TOASTS: usize = 4;

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
    /// A command run in `!` shell mode, rendered as a `$ command` header followed by its output.
    Shell(crate::shell::ShellOutcome),
    /// The `/help` command reference: the built-in slash commands and keybindings, rendered as a
    /// distinct panel. Its content is generated at render time from the command registry, so the
    /// entry itself carries no data.
    Help,
    /// A turn-level error surfaced to the user.
    Error(String),
}

/// The kind of modal picker/view opened by a dialog-opening slash command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    /// The session picker (`/sessions`).
    Sessions,
    /// The model picker (`/models`).
    Models,
    /// The agent picker (`/agents`).
    Agents,
    /// The theme picker (`/themes`).
    Themes,
    /// The MCP status/list view (`/mcp`).
    Mcp,
}

/// A modal list dialog (a picker or status view) floating over the transcript.
///
/// The dialog opening built-ins populate it with their respective items; navigation moves the
/// highlight and Esc/Enter dismiss it. Item selection that mutates state (switching the active
/// model/agent/session) is a separate feature; opening and listing is this one's responsibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    /// Which picker/view this is.
    pub kind: DialogKind,
    /// The dialog title shown in its border.
    pub title: String,
    /// The selectable rows, in display order.
    pub items: Vec<String>,
    /// The highlighted row (clamped to `items`), initialized to the active item where one applies.
    pub selected: usize,
    /// A message shown in place of an empty list (e.g. the MCP "no servers configured" state).
    pub empty: Option<String>,
}

impl Dialog {
    /// The highlighted row index, clamped to the current item count.
    #[must_use]
    pub fn selected(&self) -> usize {
        if self.items.is_empty() {
            0
        } else {
            self.selected.min(self.items.len() - 1)
        }
    }
}

/// One backing entry behind a row of the open session picker, parallel to the dialog's item rows.
///
/// The session picker's first row is a synthetic "current session" entry with no persisted id
/// (switching/renaming/deleting it is a no-op); every other row carries the persisted session's id
/// and title so the switch/rename/delete actions can act on the real record.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionEntry {
    /// The persisted session id, or `None` for the synthetic "current session" row.
    id: Option<String>,
    /// The persisted session's title (used to seed a rename), if any.
    title: Option<String>,
}

/// The open `@`-file picker: the cached candidate file list (loaded when the picker opened) and the
/// highlighted row. The query and visible matches are derived from the prompt text + cursor (see
/// [`App::mention_query`] / [`App::mention_matches`]); this state only carries what cannot be
/// re-derived cheaply each frame (the file list) and the navigation highlight.
#[derive(Debug, Clone)]
pub struct MentionPicker {
    /// Every candidate project file (relative path), loaded once when the picker opened.
    files: Vec<String>,
    /// The highlighted match row (clamped to the current match count).
    selected: usize,
}

/// The open command palette: the search query typed into it and the highlighted row.
///
/// The palette is a modal, searchable overlay (opened with `Ctrl+P`) that lists every built-in
/// command and runs the selected one directly, independent of the prompt's `/` autocomplete. Its
/// query is kept separate from the prompt so dismissing the palette restores the untouched chat
/// input and returns focus to it.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    /// The search text typed into the palette (filters the command list).
    query: String,
    /// The highlighted match row (clamped to the current match count).
    selected: usize,
}

/// The open find-in-chat search bar: the live query, the active-match index, and the cached match
/// count for the `match N of M` indicator.
///
/// Find is opened with `Ctrl+F`; while open, typing edits the query, Enter/Down step to the next
/// match, Up steps to the previous match, and Esc closes it. The query highlights every occurrence
/// in the transcript and scrolls the active match into view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindState {
    /// The live search query.
    pub query: String,
    /// The active match index among the ordered matches (clamped to the match count).
    pub active: usize,
    /// The number of matches for the current query (cached for the indicator and navigation).
    pub matches: usize,
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
    /// The open modal dialog (a picker or status view), or `None` when no dialog is shown. While
    /// set it captures navigation/dismissal keys so the prompt is not edited beneath it.
    dialog: Option<Dialog>,
    /// The open `@`-file picker, or `None` when closed. While set it captures navigation, selection,
    /// and dismissal keys; a selection inserts an `@path` token and a typed range or whitespace
    /// dismisses it.
    mention: Option<MentionPicker>,
    /// The open command palette (`Ctrl+P`), or `None` when closed. While set it is modal: it owns a
    /// search query and navigation, runs the highlighted command on Enter, and Esc dismisses it and
    /// returns focus to the prompt.
    palette: Option<Palette>,
    /// Set when `/editor` is dispatched, signalling the event loop to suspend the TUI and open the
    /// external `$EDITOR`. Consumed once by [`App::take_editor_request`].
    editor_requested: bool,
    /// Whether the prompt is in `!` shell mode. Entered by typing `!` at the start of an empty
    /// prompt; while set, the prompt runs typed commands locally (not as model turns) and shows a
    /// distinct shell affordance. Exited by Esc or by backspacing out an empty command.
    shell_mode: bool,
    /// Canonical prior-turn messages projected into each new turn for conversational context.
    history: Vec<Message>,
    /// The active theme name driving the rendered colors. Switched (and previewed) through the
    /// theme picker; loaded from the effective config at launch so a persisted choice is honored.
    theme: String,
    /// The theme active when the theme picker opened, restored on cancel so a hovered preview does
    /// not stick. `None` whenever the picker is not open.
    theme_restore: Option<String>,
    /// The active keybind set (leader chord + action bindings), loaded from config at launch so
    /// user overrides take effect; defaults otherwise.
    keybinds: Keybinds,
    /// Whether the leader chord has been pressed and the next key selects a binding. While set, the
    /// which-key overlay lists the active bindings; the next key runs one or cancels.
    leader_pending: bool,
    /// Whether the session sidebar panel is shown. Toggled by the sidebar keybind.
    sidebar_visible: bool,
    /// The persisted-session metadata backing the open session picker, parallel to the dialog's
    /// item rows (index 0 is the synthetic "current session" row). Populated when the session
    /// picker opens; consulted by switch/rename/delete; cleared when the dialog closes.
    session_entries: Vec<SessionEntry>,
    /// While renaming a session in the picker, the in-progress new title text. `None` when not
    /// renaming. Entered with `r`, edited by typing, committed with Enter, cancelled with Esc.
    session_rename: Option<String>,
    /// How many lines the transcript is scrolled up from the bottom. 0 pins to the latest content
    /// (auto-scroll); larger values reveal earlier content. Clamped to the scrollable range when
    /// rendered, so it never strands the view past the available content.
    scroll: usize,
    /// The most recent transcript viewport `(width, height)` captured during rendering, so key
    /// handlers can page-scroll and locate find matches/messages without re-deriving the layout.
    viewport: Cell<(usize, usize)>,
    /// The open find-in-chat search bar, or `None` when find mode is closed.
    find: Option<FindState>,
    /// The selected transcript message (entry index) for message-level navigation, or `None` when
    /// no message is focused.
    selected_message: Option<usize>,
    /// A transient acknowledgement (e.g. a copy confirmation) shown above the prompt, cleared on the
    /// next key press so it reads as a momentary status note.
    ack: Option<String>,
    /// Text staged for a real terminal-clipboard copy (OSC 52), consumed once by the event loop.
    copy_request: Option<String>,
    /// Active transient toast notifications (oldest first), rendered as an auto-dismissing overlay.
    /// Error-level toasts render with distinct error styling; entries are removed once their display
    /// window elapses (see [`App::tick_toasts`]).
    toasts: Vec<Toast>,
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
            dialog: None,
            mention: None,
            palette: None,
            editor_requested: false,
            shell_mode: false,
            history: Vec::new(),
            theme: default_theme_name(),
            theme_restore: None,
            keybinds: Keybinds::default(),
            leader_pending: false,
            sidebar_visible: false,
            session_entries: Vec::new(),
            session_rename: None,
            scroll: 0,
            viewport: Cell::new((80, 24)),
            find: None,
            selected_message: None,
            ack: None,
            copy_request: None,
            toasts: Vec::new(),
        }
    }

    /// The active theme name driving the rendered colors (see [`crate::ui`]).
    #[must_use]
    pub fn theme(&self) -> &str {
        &self.theme
    }

    /// Set the active theme name, used at launch to apply the persisted choice from config.
    pub fn set_theme(&mut self, name: String) {
        self.theme = name;
    }

    /// Replace the active keybind set, used at launch to apply user overrides from config.
    pub fn set_keybinds(&mut self, keybinds: Keybinds) {
        self.keybinds = keybinds;
    }

    /// The active keybind set (for rendering the which-key overlay and `/help`).
    #[must_use]
    pub fn keybinds(&self) -> &Keybinds {
        &self.keybinds
    }

    /// Whether the leader chord is armed and the which-key overlay should be shown.
    #[must_use]
    pub fn leader_pending(&self) -> bool {
        self.leader_pending
    }

    /// Whether the session sidebar panel is currently shown.
    #[must_use]
    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    /// The current scroll offset (lines up from the bottom; 0 = pinned to the latest content).
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Record the transcript viewport size during rendering so key handlers can page-scroll and
    /// locate find matches/messages against the same layout.
    pub fn set_viewport(&self, width: usize, height: usize) {
        self.viewport.set((width.max(1), height.max(1)));
    }

    /// The last recorded transcript viewport `(width, height)`, each at least 1.
    fn viewport(&self) -> (usize, usize) {
        let (w, h) = self.viewport.get();
        (w.max(1), h.max(1))
    }

    /// The open find-in-chat state (for rendering the search bar and highlights), or `None`.
    #[must_use]
    pub fn find(&self) -> Option<&FindState> {
        self.find.as_ref()
    }

    /// The find indicator state as `(active_index, match_count)`; `(0, 0)` when find is closed.
    #[must_use]
    pub fn find_status(&self) -> (usize, usize) {
        match &self.find {
            Some(find) => (find.active, find.matches),
            None => (0, 0),
        }
    }

    /// The selected transcript message (entry index) for message-level navigation, or `None`.
    #[must_use]
    pub fn selected_message(&self) -> Option<usize> {
        self.selected_message
    }

    /// The transient acknowledgement shown above the prompt (e.g. a copy confirmation), or `None`.
    #[must_use]
    pub fn ack(&self) -> Option<&str> {
        self.ack.as_deref()
    }

    /// Take any text staged for a real terminal-clipboard copy, clearing it so it is sent once.
    #[must_use]
    pub fn take_copy_request(&mut self) -> Option<String> {
        self.copy_request.take()
    }

    /// Surface a transient toast notification. The toast appears as an auto-dismissing overlay and
    /// is removed on a timer (see [`App::tick_toasts`]); error-level toasts render with distinct
    /// error styling. A burst is bounded to [`MAX_TOASTS`] by dropping the oldest toasts.
    fn push_toast(&mut self, level: ToastLevel, message: impl Into<String>) {
        self.toasts
            .push(Toast::new(level, message).with_duration(TOAST_DURATION));
        let overflow = self.toasts.len().saturating_sub(MAX_TOASTS);
        if overflow > 0 {
            self.toasts.drain(0..overflow);
        }
    }

    /// Drop any toasts whose display window has elapsed. Driven by the event loop so toasts
    /// auto-dismiss on a timer regardless of whether the user is interacting.
    pub fn tick_toasts(&mut self) {
        self.toasts.retain(|toast| !toast.is_expired());
    }

    /// The active transient toasts (oldest first), for rendering the overlay.
    #[must_use]
    pub fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    /// Whether any toast is currently displayed (so the event loop can tick more frequently while
    /// one is on screen, keeping the auto-dismiss prompt and the depletion animation smooth).
    #[must_use]
    pub fn has_toasts(&self) -> bool {
        !self.toasts.is_empty()
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
        // Pin to the latest so the streaming reply stays in view.
        self.pin_to_bottom();
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
        self.push_toast(ToastLevel::Error, message.clone());
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

    /// The `@`-mention fragment being typed at the cursor, or `None` when the cursor is not inside an
    /// active mention.
    ///
    /// A mention is active when an `@` sits at the start of the cursor's line or directly after
    /// whitespace, and every character between that `@` and the cursor is an ordinary path character
    /// (no whitespace, and no `:`/`#` which begin a line range). The returned string is the text
    /// between the `@` and the cursor (empty for a bare `@`). Stopping at `:`/`#` means typing a
    /// line range after a selected path does not reopen the picker.
    #[must_use]
    pub fn mention_query(&self) -> Option<String> {
        let pos = self.input.cursor_position();
        let line = self.input.line(pos.row)?;
        let chars: Vec<char> = line.chars().collect();
        let col = pos.col.min(chars.len());

        let mut at: Option<usize> = None;
        let mut i = col;
        while i > 0 {
            i -= 1;
            let c = chars[i];
            if c == '@' {
                at = Some(i);
                break;
            }
            if c.is_whitespace() || c == ':' || c == '#' {
                break;
            }
        }
        let at = at?;
        if at > 0 && !chars[at - 1].is_whitespace() {
            return None;
        }
        Some(chars[at + 1..col].iter().collect())
    }

    /// Whether the `@`-file picker is currently open.
    #[must_use]
    pub fn mention_open(&self) -> bool {
        self.mention.is_some()
    }

    /// The picker's files fuzzy-ranked against the current query, best match first (empty when the
    /// picker is closed).
    #[must_use]
    pub fn mention_matches(&self) -> Vec<String> {
        let Some(picker) = self.mention.as_ref() else {
            return Vec::new();
        };
        let query = self.mention_query().unwrap_or_default();
        mention::fuzzy_filter(&picker.files, &query, MENTION_MATCH_LIMIT)
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// The highlighted match index, clamped to the current match count.
    #[must_use]
    pub fn mention_selected(&self) -> usize {
        let len = self.mention_matches().len();
        let raw = self.mention.as_ref().map_or(0, |picker| picker.selected);
        if len == 0 { 0 } else { raw.min(len - 1) }
    }

    /// Recompute the picker's open state after a prompt edit: open it (loading the file list) when
    /// the cursor enters a mention fragment, keep it open and reset the highlight while the fragment
    /// is edited, and close it when the cursor leaves the fragment.
    fn refresh_mention(&mut self) {
        match self.mention_query() {
            Some(_) => match self.mention.as_mut() {
                Some(picker) => picker.selected = 0,
                None => {
                    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                    self.mention = Some(MentionPicker {
                        files: mention::list_project_files(&cwd),
                        selected: 0,
                    });
                }
            },
            None => self.mention = None,
        }
    }

    /// Move the picker highlight to the next match (wrapping at the end).
    fn mention_select_next(&mut self) {
        let len = self.mention_matches().len();
        if len == 0 {
            return;
        }
        let next = (self.mention_selected() + 1) % len;
        if let Some(picker) = self.mention.as_mut() {
            picker.selected = next;
        }
    }

    /// Move the picker highlight to the previous match (wrapping at the start).
    fn mention_select_prev(&mut self) {
        let len = self.mention_matches().len();
        if len == 0 {
            return;
        }
        let prev = (self.mention_selected() + len - 1) % len;
        if let Some(picker) = self.mention.as_mut() {
            picker.selected = prev;
        }
    }

    /// Close the `@`-file picker without inserting anything, leaving the typed text unchanged (Esc).
    fn close_mention(&mut self) {
        self.mention = None;
    }

    /// Accept the highlighted file (Enter): replace the typed `@fragment` with an `@path` token and
    /// close the picker. A picker with no matches simply closes.
    fn accept_mention(&mut self) {
        let matches = self.mention_matches();
        if matches.is_empty() {
            self.close_mention();
            return;
        }
        let path = matches[self.mention_selected()].clone();
        self.insert_mention_path(&path);
        self.mention = None;
    }

    /// Replace the active `@fragment` (the text between the `@` and the cursor) with `path`, leaving
    /// the leading `@` in place so the result is an `@path` token and the cursor lands after it.
    fn insert_mention_path(&mut self, path: &str) {
        let Some(query) = self.mention_query() else {
            return;
        };
        for _ in 0..query.chars().count() {
            self.input.delete_backward();
        }
        for c in path.chars() {
            self.input.insert_char(c);
        }
    }

    /// Open the command palette (`Ctrl+P`): a modal, searchable overlay of every built-in command.
    fn open_palette(&mut self) {
        self.palette = Some(Palette::default());
    }

    /// Whether the command palette is currently open.
    #[must_use]
    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    /// The current palette search query (empty before anything is typed), or `None` when closed.
    #[must_use]
    pub fn palette_query(&self) -> Option<&str> {
        self.palette.as_ref().map(|palette| palette.query.as_str())
    }

    /// The commands matching the palette query, in highlight order. Empty when the palette is closed
    /// or nothing matches the query.
    #[must_use]
    pub fn palette_matches(&self) -> Vec<&'static slash::SlashCommand> {
        match self.palette.as_ref() {
            Some(palette) => slash::filter(&palette.query),
            None => Vec::new(),
        }
    }

    /// The highlighted palette row, clamped to the current match count.
    #[must_use]
    pub fn palette_selected(&self) -> usize {
        let len = self.palette_matches().len();
        let raw = self.palette.as_ref().map_or(0, |palette| palette.selected);
        if len == 0 { 0 } else { raw.min(len - 1) }
    }

    /// Move the palette highlight to the next match (wrapping at the end).
    fn palette_select_next(&mut self) {
        let len = self.palette_matches().len();
        if len == 0 {
            return;
        }
        let next = (self.palette_selected() + 1) % len;
        if let Some(palette) = self.palette.as_mut() {
            palette.selected = next;
        }
    }

    /// Move the palette highlight to the previous match (wrapping at the start).
    fn palette_select_prev(&mut self) {
        let len = self.palette_matches().len();
        if len == 0 {
            return;
        }
        let prev = (self.palette_selected() + len - 1) % len;
        if let Some(palette) = self.palette.as_mut() {
            palette.selected = prev;
        }
    }

    /// Append a character to the palette query, resetting the highlight to the top match so the
    /// most relevant command is selected as the query narrows.
    fn palette_input(&mut self, c: char) {
        if let Some(palette) = self.palette.as_mut() {
            palette.query.push(c);
            palette.selected = 0;
        }
    }

    /// Delete the last character of the palette query, resetting the highlight to the top match.
    fn palette_backspace(&mut self) {
        if let Some(palette) = self.palette.as_mut() {
            palette.query.pop();
            palette.selected = 0;
        }
    }

    /// Close the command palette (Esc), returning focus to the prompt without altering its contents.
    fn close_palette(&mut self) {
        self.palette = None;
    }

    /// Run the highlighted palette command (Enter): close the palette, then dispatch the command so
    /// its effect (a dialog, flow, or action) appears. With no match the palette simply closes.
    fn dispatch_palette(&mut self) {
        let matches = self.palette_matches();
        if matches.is_empty() {
            self.close_palette();
            return;
        }
        let command = matches[self.palette_selected()];
        self.close_palette();
        self.dispatch_command(command);
    }

    /// React to a prompt edit: reset the slash highlight and recompute the mention picker.
    fn on_prompt_edited(&mut self) {
        self.slash_selected = 0;
        self.refresh_mention();
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
    /// `/init`, `/review`, `/compact`, `/export`, `/exit`) run their handler or flow; the
    /// dialog-opening built-ins (`/sessions`, `/models`, `/agents`, `/themes`, `/mcp`) open their
    /// modal picker/view; `/editor` requests the external editor and `/share` produces a local
    /// share reference. Any unhandled name surfaces a notice acknowledging the dispatch.
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
            "sessions" => self.open_sessions_dialog(),
            "models" => self.open_models_dialog(),
            "agents" => self.open_agents_dialog(),
            "themes" => self.open_themes_dialog(),
            "mcp" => self.open_mcp_dialog(),
            "editor" => self.editor_requested = true,
            "share" => self.share_session(),
            _ => self.transcript.push(Entry::Notice(format!(
                "/{} — {}",
                command.name, command.description
            ))),
        }
    }

    /// Open the session picker (`/sessions`): the active (current) session followed by any
    /// persisted sessions, newest-first. Listing the database is best-effort — if no database
    /// exists yet or it cannot be read, only the current session is shown, so the picker always
    /// opens with at least the current session listed.
    fn open_sessions_dialog(&mut self) {
        let (items, entries) = session_rows(&self.status.session);
        self.session_entries = entries;
        self.session_rename = None;
        self.open_dialog(Dialog {
            kind: DialogKind::Sessions,
            title: "Sessions".to_owned(),
            items,
            selected: 0,
            empty: None,
        });
    }

    /// Open the model picker (`/models`): every model the OpenRouter catalogue serves, by id, with
    /// the active model pre-highlighted. The list always contains the default model.
    fn open_models_dialog(&mut self) {
        let items: Vec<String> = providers::Catalogue::embedded()
            .provider(providers::OPENROUTER_ID)
            .map(|provider| {
                provider
                    .models
                    .values()
                    .map(|model| model.id.clone())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        let selected = items
            .iter()
            .position(|id| *id == self.status.model)
            .unwrap_or(0);
        self.open_dialog(Dialog {
            kind: DialogKind::Models,
            title: "Models".to_owned(),
            items,
            selected,
            empty: Some("No models in the catalogue.".to_owned()),
        });
    }

    /// Open the agent picker (`/agents`): the built-in agent personas, with the active agent
    /// pre-highlighted.
    fn open_agents_dialog(&mut self) {
        let items: Vec<String> = Agent::ALL
            .iter()
            .map(|agent| format!("{}  —  {}", agent.name(), agent_description(agent.name())))
            .collect();
        let selected = Agent::ALL
            .iter()
            .position(|agent| agent.name() == self.status.agent)
            .unwrap_or(0);
        self.open_dialog(Dialog {
            kind: DialogKind::Agents,
            title: "Agents".to_owned(),
            items,
            selected,
            empty: None,
        });
    }

    /// Open the theme picker (`/themes`): the selectable themes, with the active theme
    /// pre-highlighted.
    fn open_themes_dialog(&mut self) {
        let items: Vec<String> = frame::style::ThemeColors::available_themes()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        let selected = items
            .iter()
            .position(|name| *name == self.theme)
            .unwrap_or(0);
        // Remember the active theme so cancelling restores it after any live preview.
        self.theme_restore = Some(self.theme.clone());
        self.open_dialog(Dialog {
            kind: DialogKind::Themes,
            title: "Themes".to_owned(),
            items,
            selected,
            empty: None,
        });
    }

    /// Open the MCP status/list view (`/mcp`): the configured MCP servers and their status. With no
    /// servers configured the view opens to a clear empty state rather than failing.
    fn open_mcp_dialog(&mut self) {
        let items = configured_mcp_servers();
        self.open_dialog(Dialog {
            kind: DialogKind::Mcp,
            title: "MCP servers".to_owned(),
            items,
            selected: 0,
            empty: Some("No MCP servers configured.".to_owned()),
        });
    }

    /// Produce a local (loopback) share reference for the current session (`/share`): mint a stable
    /// share id, persist a small reference file under the data directory (best-effort), and surface
    /// the loopback URL and identifier. Only local share is in scope (no external upload).
    fn share_session(&mut self) {
        let id = local_share_id();
        let url = format!("http://127.0.0.1:3200/share/{id}");
        let notice = match write_share_reference(&id, &self.status.session) {
            Ok(path) => format!(
                "Local share reference created: {url}  ·  id {id}  ·  saved to {}",
                path.display()
            ),
            Err(_) => format!("Local share reference created: {url}  ·  id {id}"),
        };
        self.transcript.push(Entry::Notice(notice));
        self.push_toast(ToastLevel::Success, "Share link created");
    }

    /// Open a modal dialog, replacing any currently-open one and closing the slash menu.
    fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
    }

    /// The open modal dialog, if any (for rendering).
    #[must_use]
    pub fn dialog(&self) -> Option<&Dialog> {
        self.dialog.as_ref()
    }

    /// Whether a modal dialog is currently open.
    #[must_use]
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Close the open dialog, returning focus to the prompt.
    fn close_dialog(&mut self) {
        self.dialog = None;
        self.session_entries.clear();
        self.session_rename = None;
    }

    /// Handle a key while a modal dialog is open. The dialog is modal: it owns navigation and
    /// dismissal, and swallows other keys so the prompt is not edited beneath it. `Ctrl+C` still
    /// quits so the user is never trapped, and `Esc`/`Enter` close the dialog and return focus to
    /// the prompt. The session picker adds in-place actions: `r` renames the highlighted session
    /// (a sub-mode where typing edits the new title, `Enter` commits, `Esc` cancels the rename),
    /// and `d`/`Delete` removes it.
    fn on_dialog_key(&mut self, key: KeyEvent, ctrl: bool) {
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        // The session rename sub-mode captures typing so the new title is edited in place; Enter
        // commits and Esc cancels the rename without closing the picker.
        if self.session_rename.is_some() {
            match key.code {
                KeyCode::Enter => self.commit_session_rename(),
                KeyCode::Esc => self.cancel_session_rename(),
                KeyCode::Backspace => {
                    if let Some(buffer) = self.session_rename.as_mut() {
                        buffer.pop();
                    }
                }
                KeyCode::Char(c) if !ctrl => {
                    if let Some(buffer) = self.session_rename.as_mut() {
                        buffer.push(c);
                    }
                }
                _ => {}
            }
            return;
        }

        let is_sessions = matches!(
            self.dialog.as_ref().map(|dialog| dialog.kind),
            Some(DialogKind::Sessions)
        );
        match key.code {
            KeyCode::Up => self.dialog_select_prev(),
            KeyCode::Down => self.dialog_select_next(),
            KeyCode::Enter => self.dialog_confirm(),
            KeyCode::Esc => self.dialog_cancel(),
            KeyCode::Char('r') if is_sessions => self.begin_session_rename(),
            KeyCode::Char('d') | KeyCode::Delete if is_sessions => self.delete_selected_session(),
            _ => {}
        }
    }

    /// Move the dialog highlight to the next row (wrapping at the end).
    fn dialog_select_next(&mut self) {
        if let Some(dialog) = self.dialog.as_mut() {
            let len = dialog.items.len();
            if len > 0 {
                dialog.selected = (dialog.selected() + 1) % len;
            }
        }
        self.sync_theme_preview();
    }

    /// Move the dialog highlight to the previous row (wrapping at the start).
    fn dialog_select_prev(&mut self) {
        if let Some(dialog) = self.dialog.as_mut() {
            let len = dialog.items.len();
            if len > 0 {
                dialog.selected = (dialog.selected() + len - 1) % len;
            }
        }
        self.sync_theme_preview();
    }

    /// While the theme picker is open, mirror the highlighted theme into the active theme so the
    /// rendered colors preview live as the user moves the highlight. A no-op for any other dialog.
    fn sync_theme_preview(&mut self) {
        if let Some(dialog) = self.dialog.as_ref() {
            if dialog.kind == DialogKind::Themes {
                if let Some(name) = dialog.items.get(dialog.selected()) {
                    self.theme = name.clone();
                }
            }
        }
    }

    /// Confirm the open dialog (Enter), dispatching by kind: the theme picker commits and persists
    /// the highlighted theme; the model/agent pickers switch the active model/agent (updating the
    /// footer and persisting the choice); the session picker switches to the highlighted session
    /// (loading its transcript). Every confirm closes the dialog and returns focus to the prompt.
    fn dialog_confirm(&mut self) {
        match self.dialog.as_ref().map(|dialog| dialog.kind) {
            Some(DialogKind::Themes) => {
                if let Some(name) = self.confirm_theme_selection() {
                    self.persist_theme(&name);
                    self.transcript
                        .push(Entry::Notice(format!("Theme set to {name}")));
                }
            }
            Some(DialogKind::Models) => {
                if let Some(model) = self.select_model() {
                    self.persist_config_string("model", &model);
                    self.transcript
                        .push(Entry::Notice(format!("Model set to {model}")));
                }
            }
            Some(DialogKind::Agents) => {
                if let Some(agent) = self.select_agent() {
                    self.persist_config_string("agent", &agent);
                    self.transcript
                        .push(Entry::Notice(format!("Agent set to {agent}")));
                }
            }
            Some(DialogKind::Sessions) => {
                if let Some(id) = self.selected_session_id() {
                    self.load_session(&id);
                }
            }
            _ => {}
        }
        self.close_dialog();
    }

    /// Cancel the open dialog (Esc). For the theme picker this restores the theme active when the
    /// picker opened so a hovered preview does not stick; other dialogs simply close.
    fn dialog_cancel(&mut self) {
        self.cancel_theme_preview();
        self.close_dialog();
    }

    /// Commit the highlighted theme as the active one, returning its name when the open dialog is
    /// the theme picker (so the caller persists it); `None` for any other dialog.
    fn confirm_theme_selection(&mut self) -> Option<String> {
        let dialog = self.dialog.as_ref()?;
        if dialog.kind != DialogKind::Themes {
            return None;
        }
        let name = dialog.items.get(dialog.selected())?.clone();
        self.theme = name.clone();
        self.theme_restore = None;
        Some(name)
    }

    /// Restore the theme active when the picker opened, discarding any live preview. A no-op unless
    /// the open dialog is the theme picker.
    fn cancel_theme_preview(&mut self) {
        let is_theme_picker = matches!(
            self.dialog.as_ref().map(|dialog| dialog.kind),
            Some(DialogKind::Themes)
        );
        if is_theme_picker {
            if let Some(name) = self.theme_restore.take() {
                self.theme = name;
            }
        }
    }

    /// Persist the selected theme to the global config so it is honored on the next launch.
    /// Best-effort: a write failure leaves the in-session theme switched but unpersisted.
    fn persist_theme(&self, name: &str) {
        self.persist_config_string("theme", name);
    }

    /// Persist a single top-level `key = value` string to the global config so it is honored on the
    /// next launch (the seam behind the theme/model/agent pickers). Best-effort: a write failure
    /// leaves the in-session choice switched but unpersisted.
    fn persist_config_string(&self, key: &str, value: &str) {
        if let Ok(inputs) = settings::ConfigInputs::from_process(std::collections::BTreeMap::new())
        {
            let _ = settings::persist_global_string(&inputs, key, value);
        }
    }

    /// Switch the active model to the highlighted row of the open model picker, updating the footer
    /// status. Returns the new model id when it actually changed (so the caller persists it), or
    /// `None` when the dialog is not the model picker or the highlighted model is already active.
    fn select_model(&mut self) -> Option<String> {
        let dialog = self.dialog.as_ref()?;
        if dialog.kind != DialogKind::Models {
            return None;
        }
        let model = dialog.items.get(dialog.selected())?.clone();
        if model == self.status.model {
            return None;
        }
        self.status.model = model.clone();
        Some(model)
    }

    /// Switch the active agent to the highlighted row of the open agent picker, updating the footer
    /// status. Returns the new agent name when it actually changed (so the caller persists it), or
    /// `None` when the dialog is not the agent picker or the highlighted agent is already active.
    fn select_agent(&mut self) -> Option<String> {
        let dialog = self.dialog.as_ref()?;
        if dialog.kind != DialogKind::Agents {
            return None;
        }
        let name = Agent::ALL.get(dialog.selected())?.name().to_owned();
        if name == self.status.agent {
            return None;
        }
        self.status.agent = name.clone();
        Some(name)
    }

    /// The persisted id of the highlighted session-picker row, or `None` for the synthetic
    /// "current session" row (and when the open dialog is not the session picker).
    fn selected_session_id(&self) -> Option<String> {
        let dialog = self.dialog.as_ref()?;
        if dialog.kind != DialogKind::Sessions {
            return None;
        }
        self.session_entries
            .get(dialog.selected())
            .and_then(|entry| entry.id.clone())
    }

    /// Load a persisted session's transcript into the view (`Enter` on a session-picker row): the
    /// stored user/assistant messages replace the current transcript and projected history, and the
    /// footer's session label updates. Ignored while a turn streams; a read failure surfaces a
    /// clear error rather than crashing.
    fn load_session(&mut self, id: &str) {
        if self.is_busy() {
            return;
        }
        let session = match load_persisted_session(id) {
            Ok(Some(loaded)) => loaded,
            Ok(None) => {
                self.note_error(format!("Session {id} no longer exists."));
                return;
            }
            Err(err) => {
                self.note_error(format!("Could not load session: {err}"));
                return;
            }
        };

        self.transcript.clear();
        self.history.clear();
        self.usage = UsageSummary::default();
        for message in &session.messages {
            match message.role.as_str() {
                "user" => {
                    self.transcript.push(Entry::User(message.content.clone()));
                    self.history.push(Message::user(message.content.clone()));
                }
                "assistant" => {
                    self.transcript.push(Entry::Assistant(AssistantTurn {
                        reasoning: String::new(),
                        answer: message.content.clone(),
                    }));
                    self.history
                        .push(Message::assistant(message.content.clone()));
                }
                _ => {}
            }
        }
        let label = session_status_label(&session.session);
        self.status.session = label.clone();
        self.transcript
            .push(Entry::Notice(format!("Switched to session {label}")));
    }

    /// Begin renaming the highlighted session-picker row, seeding the rename buffer with its current
    /// title. A no-op for the synthetic "current session" row (which has no persisted id).
    fn begin_session_rename(&mut self) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        let selected = dialog.selected();
        let Some(entry) = self.session_entries.get(selected) else {
            return;
        };
        if entry.id.is_none() {
            return;
        }
        self.session_rename = Some(entry.title.clone().unwrap_or_default());
    }

    /// Commit the in-progress session rename to the store and refresh the picker so the new label
    /// shows. An empty new title clears the session's title (falling back to its id as the label).
    fn commit_session_rename(&mut self) {
        let Some(new_title) = self.session_rename.take() else {
            return;
        };
        let Some(id) = self.selected_session_id() else {
            return;
        };
        let trimmed = new_title.trim();
        let title = (!trimmed.is_empty()).then_some(trimmed);
        match rename_persisted_session(&id, title) {
            Ok(()) => {
                let shown = title.map(str::to_owned).unwrap_or_else(|| id.clone());
                self.refresh_sessions_dialog();
                self.transcript
                    .push(Entry::Notice(format!("Renamed session to {shown}")));
            }
            Err(err) => self.note_error(format!("Rename failed: {err}")),
        }
    }

    /// Cancel an in-progress session rename, returning the picker to navigation without closing it.
    fn cancel_session_rename(&mut self) {
        self.session_rename = None;
    }

    /// Delete the highlighted session-picker row from the store and refresh the picker so the entry
    /// disappears. A no-op for the synthetic "current session" row.
    fn delete_selected_session(&mut self) {
        let Some(id) = self.selected_session_id() else {
            return;
        };
        match delete_persisted_session(&id) {
            Ok(_) => {
                self.refresh_sessions_dialog();
                self.transcript
                    .push(Entry::Notice(format!("Deleted session {id}")));
            }
            Err(err) => self.note_error(format!("Delete failed: {err}")),
        }
    }

    /// Rebuild the open session picker's rows from the store (after a rename/delete), keeping the
    /// highlight on a valid row. A no-op unless the session picker is open.
    fn refresh_sessions_dialog(&mut self) {
        if !matches!(
            self.dialog.as_ref().map(|dialog| dialog.kind),
            Some(DialogKind::Sessions)
        ) {
            return;
        }
        let (items, entries) = session_rows(&self.status.session);
        let new_len = items.len();
        self.session_entries = entries;
        if let Some(dialog) = self.dialog.as_mut() {
            let clamped = dialog.selected.min(new_len.saturating_sub(1));
            dialog.items = items;
            dialog.selected = clamped;
        }
    }

    /// The in-progress session-rename text, when the session picker is in rename mode (for the UI).
    #[must_use]
    pub fn session_rename(&self) -> Option<&str> {
        self.session_rename.as_deref()
    }

    /// Take the `/editor` request (if any), clearing it so the editor opens exactly once.
    #[must_use]
    pub fn take_editor_request(&mut self) -> bool {
        std::mem::take(&mut self.editor_requested)
    }

    /// Replace the prompt contents (used to land editor-composed text back in the prompt).
    pub fn set_input_value(&mut self, value: &str) {
        self.input.set_value(value);
    }

    /// Surface a transient error in the transcript and as a distinct error toast (used when the
    /// external editor fails, a session action fails, or `/export` cannot write).
    pub fn note_error(&mut self, message: String) {
        self.push_toast(ToastLevel::Error, message.clone());
        self.transcript.push(Entry::Error(message));
    }

    /// Run a leader keybind action: the same visible effects the matching slash commands produce,
    /// reached via the leader chord instead of the prompt.
    fn run_keybind_action(&mut self, action: KeybindAction) {
        match action {
            KeybindAction::NewSession => self.new_session(),
            KeybindAction::ModelList => self.open_models_dialog(),
            KeybindAction::ToggleSidebar => self.toggle_sidebar(),
            KeybindAction::Sessions => self.open_sessions_dialog(),
            KeybindAction::Agents => self.open_agents_dialog(),
            KeybindAction::Themes => self.open_themes_dialog(),
        }
    }

    /// Show or hide the session sidebar panel (the sidebar-toggle keybind).
    fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
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
        self.reset_view();
    }

    /// Reset transcript navigation back to the pinned-bottom, nothing-selected, find-closed state
    /// (used when the conversation changes underneath the view).
    fn reset_view(&mut self) {
        self.scroll = 0;
        self.find = None;
        self.selected_message = None;
        self.ack = None;
    }

    /// Pin the transcript to the latest content (used when new content is appended so it stays in
    /// view).
    fn pin_to_bottom(&mut self) {
        self.scroll = 0;
    }

    /// The number of lines the transcript can be scrolled up given the last viewport.
    fn max_scroll(&self) -> usize {
        let (width, height) = self.viewport();
        let total = crate::ui::transcript_lines(self, width).len();
        total.saturating_sub(height)
    }

    /// Set the scroll offset so the given rendered line is comfortably in view (near the top, with a
    /// small margin), clamped to the scrollable range.
    fn scroll_to_line(&mut self, target: usize) {
        let (width, height) = self.viewport();
        let total = crate::ui::transcript_lines(self, width).len();
        let max = total.saturating_sub(height);
        let margin = height / 3;
        let desired_start = target.saturating_sub(margin).min(max);
        self.scroll = max - desired_start;
    }

    /// Scroll the transcript up by one page (revealing earlier content), clamped to the top.
    fn scroll_page_up(&mut self) {
        let (_, height) = self.viewport();
        let page = height.saturating_sub(1).max(1);
        self.scroll = (self.scroll + page).min(self.max_scroll());
    }

    /// Scroll the transcript down by one page (toward the latest content), stopping at the bottom.
    fn scroll_page_down(&mut self) {
        let (_, height) = self.viewport();
        let page = height.saturating_sub(1).max(1);
        self.scroll = self.scroll.saturating_sub(page);
    }

    /// Open the find-in-chat search bar (Ctrl+F) with an empty query.
    fn open_find(&mut self) {
        if self.find.is_none() {
            self.find = Some(FindState::default());
        }
    }

    /// Close the find-in-chat search bar (Esc / Ctrl+F toggle), clearing the highlight.
    fn close_find(&mut self) {
        self.find = None;
    }

    /// Append a character to the find query and refresh the matches/highlight.
    fn find_input(&mut self, c: char) {
        if let Some(find) = self.find.as_mut() {
            find.query.push(c);
            find.active = 0;
        }
        self.recompute_find();
    }

    /// Delete the last character of the find query and refresh the matches/highlight.
    fn find_backspace(&mut self) {
        if let Some(find) = self.find.as_mut() {
            find.query.pop();
            find.active = 0;
        }
        self.recompute_find();
    }

    /// Recompute the find match count for the current query (against the last viewport), clamp the
    /// active index, and scroll the active match into view.
    fn recompute_find(&mut self) {
        let Some(query) = self.find.as_ref().map(|f| f.query.clone()) else {
            return;
        };
        let (width, _) = self.viewport();
        let lines = crate::ui::transcript_lines(self, width);
        let matches = crate::ui::transcript_matches(&lines, &query);
        let count = matches.len();
        let active = self
            .find
            .as_ref()
            .map(|f| {
                if count == 0 {
                    0
                } else {
                    f.active.min(count - 1)
                }
            })
            .unwrap_or(0);
        if let Some(target) = matches.get(active).map(|m| m.line) {
            self.scroll_to_line(target);
        }
        if let Some(find) = self.find.as_mut() {
            find.matches = count;
            find.active = active;
        }
    }

    /// Move to the next find match (wrapping), scrolling it into view.
    fn find_next(&mut self) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if find.matches == 0 {
            return;
        }
        find.active = (find.active + 1) % find.matches;
        self.scroll_to_active_match();
    }

    /// Move to the previous find match (wrapping), scrolling it into view.
    fn find_prev(&mut self) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if find.matches == 0 {
            return;
        }
        find.active = (find.active + find.matches - 1) % find.matches;
        self.scroll_to_active_match();
    }

    /// Scroll the current active find match into view (recomputing match positions for the layout).
    fn scroll_to_active_match(&mut self) {
        let Some((query, active)) = self.find.as_ref().map(|f| (f.query.clone(), f.active)) else {
            return;
        };
        let (width, _) = self.viewport();
        let lines = crate::ui::transcript_lines(self, width);
        let matches = crate::ui::transcript_matches(&lines, &query);
        if let Some(target) = matches.get(active).map(|m| m.line) {
            self.scroll_to_line(target);
        }
    }

    /// Select the previous (earlier) transcript message and scroll it into view. With nothing
    /// selected yet, selects the last message.
    fn select_prev_message(&mut self) {
        if self.transcript.is_empty() {
            return;
        }
        let last = self.transcript.len() - 1;
        let next = match self.selected_message {
            None => last,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.selected_message = Some(next);
        self.scroll_to_message(next);
    }

    /// Select the next (later) transcript message and scroll it into view. With nothing selected
    /// yet, selects the last message.
    fn select_next_message(&mut self) {
        if self.transcript.is_empty() {
            return;
        }
        let last = self.transcript.len() - 1;
        let next = match self.selected_message {
            None => last,
            Some(i) if i >= last => last,
            Some(i) => i + 1,
        };
        self.selected_message = Some(next);
        self.scroll_to_message(next);
    }

    /// Scroll the transcript so the given message (entry index) is in view.
    fn scroll_to_message(&mut self, idx: usize) {
        let (width, _) = self.viewport();
        let starts = crate::ui::entry_start_lines(self, width);
        if let Some(&target) = starts.get(idx) {
            self.scroll_to_line(target);
        }
    }

    /// Clear transcript navigation focus (Esc): drop any message selection (the scroll position is
    /// left untouched so the user keeps their place).
    fn clear_navigation(&mut self) {
        self.selected_message = None;
    }

    /// Copy the selected message (or, with none selected, the most recent message) to the terminal
    /// clipboard and acknowledge it. The acknowledgement is the user-visible proof; the actual copy
    /// is performed by the event loop via an OSC 52 sequence.
    fn copy_selected_message(&mut self) {
        let idx = self
            .selected_message
            .or_else(|| self.transcript.len().checked_sub(1));
        let Some(idx) = idx else {
            self.push_toast(ToastLevel::Error, "Nothing to copy yet.");
            return;
        };
        let Some(entry) = self.transcript.get(idx) else {
            self.push_toast(ToastLevel::Error, "Nothing to copy yet.");
            return;
        };
        let text = entry_copy_text(entry);
        self.ack = Some(format!("Copied message {} to clipboard", idx + 1));
        self.copy_request = Some(text);
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
            Ok(path) => {
                self.transcript.push(Entry::Notice(format!(
                    "Exported session to {}",
                    path.display()
                )));
                self.push_toast(ToastLevel::Success, "Session exported");
            }
            Err(err) => self.note_error(format!("Export failed: {err}")),
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
                Entry::Shell(outcome) => out.push_str(&format!(
                    "### Shell: `{}`\n\n```\n{}\n```\n\n",
                    outcome.command, outcome.output
                )),
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

        // A transient acknowledgement (e.g. a copy confirmation) lives until the next key press, so
        // clear it here; an action below may set a fresh one for this key.
        self.ack = None;

        // While the leader chord is armed the next key selects a binding: a matching action key runs
        // its action, Esc cancels, and any other key simply disarms (a harmless no-op). The chord is
        // consumed regardless, so the which-key overlay never lingers. Ctrl+C still quits.
        if self.leader_pending {
            self.leader_pending = false;
            if ctrl && key.code == KeyCode::Char('c') {
                self.should_quit = true;
                return;
            }
            if key.code != KeyCode::Esc {
                let chord = KeyChord::from_event(key.code, key.modifiers);
                if let Some(action) = self.keybinds.action_for(chord) {
                    self.run_keybind_action(action);
                }
            }
            return;
        }

        // A modal dialog (picker/status view) is modal: it owns navigation and dismissal, and
        // swallows other keys so the prompt is not edited beneath it. Ctrl+C still quits so the
        // user is never trapped, and Esc/Enter both close the dialog and return focus to the prompt.
        if self.dialog_open() {
            self.on_dialog_key(key, ctrl);
            return;
        }

        // The command palette is a modal, searchable overlay: it owns its own search query and
        // navigation, runs the highlighted command on Enter, and Esc dismisses it and returns focus
        // to the prompt (leaving the prompt's contents untouched). Typing edits the palette query
        // rather than the prompt beneath it; Ctrl+C still quits so the user is never trapped.
        if self.palette_open() {
            match key.code {
                KeyCode::Char('c') if ctrl => self.should_quit = true,
                KeyCode::Up => self.palette_select_prev(),
                KeyCode::Down => self.palette_select_next(),
                KeyCode::Enter => self.dispatch_palette(),
                KeyCode::Esc => self.close_palette(),
                KeyCode::Backspace => self.palette_backspace(),
                KeyCode::Char(c) if !ctrl && !alt => self.palette_input(c),
                _ => {}
            }
            return;
        }

        // The find-in-chat search bar is modal while open: typing edits the query (re-highlighting
        // matches), Enter/Down step to the next match, Up to the previous, Ctrl+F or Esc close it,
        // and Ctrl+C still quits so the user is never trapped.
        if self.find.is_some() {
            match key.code {
                KeyCode::Char('c') if ctrl => self.should_quit = true,
                KeyCode::Char('f') if ctrl => self.close_find(),
                KeyCode::Esc => self.close_find(),
                KeyCode::Enter | KeyCode::Down => self.find_next(),
                KeyCode::Up => self.find_prev(),
                KeyCode::Backspace => self.find_backspace(),
                KeyCode::Char(c) if !ctrl && !alt => self.find_input(c),
                _ => {}
            }
            return;
        }

        // While `!` shell mode is active the prompt runs typed commands locally rather than sending
        // them to the model: Enter executes the command, Shift+Enter inserts a soft newline, Esc
        // exits back to chat, and Backspace on an empty command also exits (otherwise it edits the
        // command). Ordinary text and caret keys edit the command without opening the slash/`@`
        // menus, which do not apply in shell mode.
        if self.shell_mode {
            match key.code {
                KeyCode::Char('c') if ctrl => self.should_quit = true,
                KeyCode::Char('a') if ctrl => {
                    self.input.handle_key(TextAreaKey::Home, modifiers(false));
                }
                KeyCode::Char('e') if ctrl => {
                    self.input.handle_key(TextAreaKey::End, modifiers(false));
                }
                KeyCode::Char(c) if !ctrl && !alt => self.input.insert_char(c),
                KeyCode::Enter if shift => self.input.insert_newline(),
                KeyCode::Enter => self.run_shell_command(),
                KeyCode::Esc => self.exit_shell_mode(),
                KeyCode::Backspace => {
                    if self.input.value().is_empty() {
                        self.exit_shell_mode();
                    } else {
                        self.input.delete_backward();
                    }
                }
                KeyCode::Delete => self.input.delete_forward(),
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
            return;
        }

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

        // While the `@`-file picker is open it owns vertical navigation, selection, and dismissal:
        // the arrows move the highlight, Enter inserts the highlighted file as an `@path` token, and
        // Esc closes the picker without inserting. Other keys fall through to ordinary editing, which
        // re-filters (or closes) the picker.
        if self.mention_open() {
            match key.code {
                KeyCode::Up => {
                    self.mention_select_prev();
                    return;
                }
                KeyCode::Down => {
                    self.mention_select_next();
                    return;
                }
                KeyCode::Enter if !shift => {
                    self.accept_mention();
                    return;
                }
                KeyCode::Esc => {
                    self.close_mention();
                    return;
                }
                _ => {}
            }
        }

        // The leader chord arms a leader sequence (showing the which-key overlay) so the next key
        // runs a binding. It is recognized only in ordinary prompt mode — the slash/`@` menus, which
        // fall through to editing above, keep their own keys.
        if !self.slash_menu_open() && !self.mention_open() {
            let chord = KeyChord::from_event(key.code, key.modifiers);
            if chord == self.keybinds.leader {
                self.leader_pending = true;
                return;
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
            // Ctrl+P opens the searchable command palette overlay.
            KeyCode::Char('p') if ctrl => self.open_palette(),
            // Ctrl+F opens find-in-chat (highlight + next/prev navigation) over the transcript.
            KeyCode::Char('f') if ctrl => self.open_find(),
            // Ctrl+Y copies the selected message (or the latest) to the clipboard, acknowledged.
            KeyCode::Char('y') if ctrl => self.copy_selected_message(),
            // Message-level navigation: Ctrl+K / Ctrl+N select the previous / next message.
            KeyCode::Char('k') if ctrl => self.select_prev_message(),
            KeyCode::Char('n') if ctrl => self.select_next_message(),
            // `!` at the start of an empty prompt enters shell mode (the `!` is the mode trigger,
            // not literal text); typed elsewhere it is ordinary text handled by the arm below.
            KeyCode::Char('!') if !ctrl && !alt && self.input.value().is_empty() => {
                self.shell_mode = true;
            }
            // Ordinary text entry (ignore other ctrl/alt chords). Editing the prompt re-filters the
            // slash menu and the `@`-file picker, so refresh both after the edit.
            KeyCode::Char(c) if !ctrl && !alt => {
                self.input.insert_char(c);
                self.on_prompt_edited();
            }
            // Shift+Enter inserts a soft newline and grows the input without submitting.
            KeyCode::Enter if shift => self.input.insert_newline(),
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.delete_backward();
                self.on_prompt_edited();
            }
            KeyCode::Delete => {
                self.input.delete_forward();
                self.on_prompt_edited();
            }
            KeyCode::Left => {
                self.input.handle_key(TextAreaKey::Left, modifiers(shift));
            }
            KeyCode::Right => {
                self.input.handle_key(TextAreaKey::Right, modifiers(shift));
            }
            // Alt+Up / Alt+Down select the previous / next message (must precede the plain arrows,
            // which move the prompt caret).
            KeyCode::Up if alt => self.select_prev_message(),
            KeyCode::Down if alt => self.select_next_message(),
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
            // Page-scroll the transcript to reach earlier / later content.
            KeyCode::PageUp => self.scroll_page_up(),
            KeyCode::PageDown => self.scroll_page_down(),
            // Esc clears the message selection focus.
            KeyCode::Esc => self.clear_navigation(),
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
        // Inline any `@`-mentioned file contents into the prompt sent to the model, while showing the
        // typed `@path` prompt in the transcript: the expanded text becomes the pending prompt (sent
        // and carried into history) and the typed text becomes the user-visible header.
        let typed = trimmed.to_owned();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let expanded = mention::expand(&typed, &cwd);
        if expanded != typed {
            self.pending_header = Some(typed);
        }
        self.pending_prompt = Some(expanded);
        self.mention = None;
        self.input.set_value("");
        // A new turn is about to append content; return to the latest and drop any stale
        // navigation focus so the streaming reply stays in view.
        self.reset_view();
    }

    /// Whether the prompt is in `!` shell mode (rendered with a distinct affordance).
    #[must_use]
    pub fn shell_mode(&self) -> bool {
        self.shell_mode
    }

    /// Run the typed shell-mode command (Enter): execute it locally in the working directory and
    /// record its output in the transcript, then clear the prompt while staying in shell mode so
    /// further commands can be run. An empty command is a no-op.
    fn run_shell_command(&mut self) {
        let command = self.input.value();
        let command = command.trim();
        if command.is_empty() {
            return;
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let outcome = shell::run(command, &cwd);
        self.transcript.push(Entry::Shell(outcome));
        self.input.set_value("");
        // Keep the freshly appended command output in view.
        self.pin_to_bottom();
    }

    /// Leave `!` shell mode and return to chat input (Esc, or backspacing out an empty command),
    /// clearing any partially-typed command.
    fn exit_shell_mode(&mut self) {
        self.shell_mode = false;
        self.input.set_value("");
    }
}

/// Builds [`TextAreaModifiers`] from a shift flag (ctrl/alt are never forwarded here).
fn modifiers(shift: bool) -> TextAreaModifiers {
    TextAreaModifiers::new(shift, false, false)
}

/// The plain-text content of a transcript entry, used when copying a message to the clipboard.
fn entry_copy_text(entry: &Entry) -> String {
    match entry {
        Entry::User(text) => text.clone(),
        Entry::Assistant(turn) => {
            if turn.answer.trim().is_empty() {
                turn.reasoning.clone()
            } else {
                turn.answer.clone()
            }
        }
        Entry::Diff(diff) => diff.path.clone(),
        Entry::Tool(card) => {
            let mut out = format!("{}: {}", card.name, card.summary);
            if !card.output.trim().is_empty() {
                out.push('\n');
                out.push_str(&card.output);
            }
            out
        }
        Entry::Notice(text) | Entry::Error(text) => text.clone(),
        Entry::Shell(outcome) => format!("$ {}\n{}", outcome.command, outcome.output),
        Entry::Help => "Help — commands & keybindings".to_owned(),
    }
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

/// A short, human-readable description for a built-in agent persona, shown in the agent picker.
fn agent_description(name: &str) -> &'static str {
    match name {
        "build" => "Default coding agent",
        "plan" => "Investigate and produce a plan",
        "general" => "General-purpose assistant",
        _ => "",
    }
}

/// The built-in default theme name (the first available theme), used before any config is loaded.
fn default_theme_name() -> String {
    frame::style::ThemeColors::available_themes()
        .first()
        .copied()
        .unwrap_or("dark")
        .to_owned()
}

/// The configured active theme name, read from the effective configuration's `theme` key, falling
/// back to the default theme when unset or unreadable.
pub(crate) fn active_theme_name() -> String {
    effective_config()
        .as_ref()
        .and_then(|cfg| cfg.get("theme"))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .unwrap_or_else(default_theme_name)
}

/// The configured MCP servers as display rows (`name — status`), read from the effective
/// configuration's `[mcp]` table. Empty when no servers are configured (M4 wires the live client;
/// here the view lists configured names with a configured/connection-pending status).
fn configured_mcp_servers() -> Vec<String> {
    let Some(config) = effective_config() else {
        return Vec::new();
    };
    let Some(table) = config.get("mcp").and_then(|value| value.as_table()) else {
        return Vec::new();
    };
    table
        .keys()
        .map(|name| format!("{name}  —  configured"))
        .collect()
}

/// A persisted session plus its messages, loaded for switching the active session in the picker.
struct LoadedSession {
    session: db::Session,
    messages: Vec<db::Message>,
}

/// Build the session picker's rows and their backing metadata in lockstep: the synthetic "current
/// session" row (no persisted id) followed by every persisted session, newest-first. The labels and
/// entries share the same order so a highlighted row maps directly to its [`SessionEntry`].
fn session_rows(current_label: &str) -> (Vec<String>, Vec<SessionEntry>) {
    let mut items = vec![format!("● current — {current_label} session")];
    let mut entries = vec![SessionEntry {
        id: None,
        title: None,
    }];
    for session in persisted_sessions() {
        items.push(session_picker_label(&session));
        entries.push(SessionEntry {
            id: Some(session.id.clone()),
            title: session.title.clone(),
        });
    }
    (items, entries)
}

/// Every persisted session newest-first, best-effort: an empty list when no database exists yet or
/// it cannot be read, so the picker still opens with just the current session.
fn persisted_sessions() -> Vec<db::Session> {
    let Some(path) = db::DbPathInputs::from_process().resolve() else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }
    let Ok(database) = db::Database::open(&path) else {
        return Vec::new();
    };
    database.sessions().list().unwrap_or_default()
}

/// A session picker row label: `id  —  title` when titled, otherwise the bare id.
fn session_picker_label(session: &db::Session) -> String {
    match &session.title {
        Some(title) if !title.trim().is_empty() => format!("{}  —  {title}", session.id),
        _ => session.id.clone(),
    }
}

/// The footer's session label for a switched-to session: its title when set, otherwise its id.
fn session_status_label(session: &db::Session) -> String {
    match &session.title {
        Some(title) if !title.trim().is_empty() => title.clone(),
        _ => session.id.clone(),
    }
}

/// Load a persisted session and its messages by id, or `None` when the session does not exist.
/// Best-effort failures (no resolvable store, an open/read error) surface as an `Err(String)`.
fn load_persisted_session(id: &str) -> Result<Option<LoadedSession>, String> {
    let database = open_session_store()?;
    let session = match database.sessions().get(id).map_err(|err| err.to_string())? {
        Some(session) => session,
        None => return Ok(None),
    };
    let messages = database
        .messages()
        .list_for_session(id)
        .map_err(|err| err.to_string())?;
    Ok(Some(LoadedSession { session, messages }))
}

/// Rename a persisted session (set or clear its title) by id.
fn rename_persisted_session(id: &str, title: Option<&str>) -> Result<(), String> {
    let database = open_session_store()?;
    database
        .sessions()
        .set_title(id, title)
        .map_err(|err| err.to_string())?;
    Ok(())
}

/// Delete a persisted session by id, returning whether a row was removed.
fn delete_persisted_session(id: &str) -> Result<bool, String> {
    let database = open_session_store()?;
    database
        .sessions()
        .delete(id)
        .map_err(|err| err.to_string())
}

/// Resolve and open the session store, erroring with a clear message when no location resolves.
fn open_session_store() -> Result<db::Database, String> {
    let path = db::DbPathInputs::from_process()
        .resolve()
        .ok_or_else(|| "no session store location could be resolved".to_owned())?;
    db::Database::open(&path).map_err(|err| err.to_string())
}

/// Load the effective configuration with no CLI overrides, or `None` if it cannot be resolved.
fn effective_config() -> Option<settings::EffectiveConfig> {
    settings::ConfigInputs::from_process(std::collections::BTreeMap::new())
        .ok()
        .and_then(|inputs| settings::load(inputs).ok())
}

/// Mint a stable, unique local share identifier for the current session.
fn local_share_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("local-{}-{nanos}", std::process::id())
}

/// Persist a small local share reference file under the data directory's `shares/` folder, returning
/// the path written. Local share only (loopback); no external upload.
fn write_share_reference(id: &str, session: &str) -> io::Result<PathBuf> {
    let dir = data_subdir("shares");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.json"));
    let body = format!(
        "{{\"id\":\"{id}\",\"session\":\"{session}\",\"url\":\"http://127.0.0.1:3200/share/{id}\"}}"
    );
    std::fs::write(&path, body)?;
    Ok(path)
}

/// Resolve a named subdirectory under the data directory, mirroring [`export_dir`]'s precedence.
fn data_subdir(name: &str) -> PathBuf {
    if let Some(dir) = env_non_empty("CORTEX_DATA_DIR") {
        return PathBuf::from(dir).join(name);
    }
    if let Some(dir) = env_non_empty("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("cortex").join(name);
    }
    if let Some(home) = env_non_empty("HOME") {
        return PathBuf::from(home).join(".local/share/cortex").join(name);
    }
    std::env::temp_dir().join("cortex").join(name)
}

/// Open the user's external editor to compose prompt text, seeded with the current prompt contents,
/// and return the composed text. Returns `Ok(None)` if the editor exits non-zero (composition
/// aborted), in which case the prompt is left unchanged.
///
/// The editor is resolved from `$VISUAL`, then `$EDITOR`, falling back to `vi`.
pub(crate) fn run_editor(seed: &str) -> io::Result<Option<String>> {
    run_editor_with(&resolve_editor(), seed)
}

/// The editor command string: `$VISUAL`, else `$EDITOR`, else `vi`.
fn resolve_editor() -> String {
    env_non_empty("VISUAL")
        .or_else(|| env_non_empty("EDITOR"))
        .unwrap_or_else(|| "vi".to_owned())
}

/// Run `editor` against a temporary file seeded with `seed`, returning the file's contents (with a
/// single trailing newline trimmed) when the editor exits cleanly, or `Ok(None)` when it aborts.
///
/// The editor command is launched through `sh -c "<editor> \"$1\""` with the file path passed as a
/// positional argument, so editors carrying flags (e.g. `code -w`) and paths with spaces are both
/// handled without manual quoting.
fn run_editor_with(editor: &str, seed: &str) -> io::Result<Option<String>> {
    let path = std::env::temp_dir().join(format!(
        "cortex-compose-{}-{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::write(&path, seed)?;

    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status()?;

    let contents = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);

    if !status.success() {
        return Ok(None);
    }
    let mut text = contents?;
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    Ok(Some(text))
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

    /// Replace the open picker's file list with a fixed set so selection/filtering tests are
    /// deterministic and independent of the working directory.
    fn inject_mention_files(a: &mut App, files: &[&str]) {
        let picker = a.mention.as_mut().expect("picker open");
        picker.files = files.iter().map(|f| (*f).to_owned()).collect();
        picker.selected = 0;
    }

    #[test]
    fn typing_at_opens_the_file_picker() {
        // VAL-TUI-049: typing `@` opens the file picker.
        let mut a = app();
        assert!(!a.mention_open());
        a.on_key(key(KeyCode::Char('@')));
        assert!(a.mention_open(), "the `@` file picker is open");
    }

    #[test]
    fn typing_after_at_filters_the_file_list() {
        // VAL-TUI-050: typing after `@` fuzzy-filters the file list.
        let mut a = app();
        a.on_key(key(KeyCode::Char('@')));
        inject_mention_files(
            &mut a,
            &["crates/cli/src/main.rs", "README.md", "Cargo.toml"],
        );
        type_str(&mut a, "main");
        let matches = a.mention_matches();
        assert_eq!(
            matches.first().map(String::as_str),
            Some("crates/cli/src/main.rs")
        );
        assert!(
            !matches.iter().any(|m| m == "README.md"),
            "got: {matches:?}"
        );
    }

    #[test]
    fn selecting_a_file_inserts_an_at_path_token_and_closes() {
        // VAL-TUI-051: selecting a file inserts an `@path` token and closes the picker.
        let mut a = app();
        a.on_key(key(KeyCode::Char('@')));
        inject_mention_files(&mut a, &["crates/cli/src/main.rs"]);
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.input_value(), "@crates/cli/src/main.rs");
        assert!(!a.mention_open(), "picker closed after selection");
    }

    #[test]
    fn mention_supports_a_line_range_suffix() {
        // VAL-TUI-052: a selected mention accepts a `:start-end` line-range suffix as one token.
        let mut a = app();
        a.on_key(key(KeyCode::Char('@')));
        inject_mention_files(&mut a, &["crates/cli/src/main.rs"]);
        a.on_key(key(KeyCode::Enter));
        type_str(&mut a, ":5-20");
        assert_eq!(a.input_value(), "@crates/cli/src/main.rs:5-20");
        assert!(
            !a.mention_open(),
            "typing a range does not reopen the picker"
        );
        let parsed = mention::parse(&a.input_value());
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].range, Some((5, 20)));
    }

    #[test]
    fn esc_closes_the_file_picker_without_inserting() {
        // VAL-TUI-053: Esc dismisses the picker leaving the prompt unchanged (no path inserted).
        let mut a = app();
        type_str(&mut a, "@app");
        assert!(a.mention_open());
        a.on_key(key(KeyCode::Esc));
        assert!(!a.mention_open(), "picker closed by Esc");
        assert_eq!(a.input_value(), "@app", "no path inserted on Esc");
    }

    #[test]
    fn whitespace_after_at_closes_the_picker() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('@')));
        assert!(a.mention_open());
        a.on_key(key(KeyCode::Char(' ')));
        assert!(
            !a.mention_open(),
            "a space ends the mention and closes the picker"
        );
    }

    #[test]
    fn bang_on_empty_prompt_enters_shell_mode() {
        // VAL-TUI-055: typing `!` at the start of an empty prompt enters shell mode (the `!` is the
        // mode trigger, not literal prompt text).
        let mut a = app();
        a.on_key(key(KeyCode::Char('!')));
        assert!(a.shell_mode(), "shell mode engaged");
        assert_eq!(a.input_value(), "", "the `!` is not inserted as text");
    }

    #[test]
    fn bang_in_nonempty_prompt_is_literal_text() {
        // `!` only triggers shell mode at the start of an empty prompt; elsewhere it is plain text.
        let mut a = app();
        type_str(&mut a, "hi");
        a.on_key(key(KeyCode::Char('!')));
        assert!(!a.shell_mode(), "no shell mode mid-text");
        assert_eq!(a.input_value(), "hi!");
    }

    #[test]
    fn shell_mode_runs_a_command_and_shows_output() {
        // VAL-TUI-056: a command run in shell mode executes and its output lands in the transcript.
        let mut a = app();
        a.on_key(key(KeyCode::Char('!')));
        type_str(&mut a, "echo cortex-shell-ok");
        a.on_key(key(KeyCode::Enter));

        match a.transcript.last() {
            Some(Entry::Shell(outcome)) => {
                assert!(outcome.success, "echo succeeds: {outcome:?}");
                assert!(
                    outcome.output.contains("cortex-shell-ok"),
                    "stdout shown: {outcome:?}"
                );
                assert_eq!(outcome.command, "echo cortex-shell-ok");
            }
            other => panic!("expected a shell entry, got {other:?}"),
        }
        // The command is not sent to the model, and the prompt clears but stays in shell mode.
        assert!(a.take_pending_prompt().is_none(), "no model turn queued");
        assert_eq!(a.input_value(), "");
        assert!(a.shell_mode(), "still in shell mode after a command");
    }

    #[test]
    fn esc_exits_shell_mode() {
        // VAL-TUI-057: Esc returns to chat input.
        let mut a = app();
        a.on_key(key(KeyCode::Char('!')));
        type_str(&mut a, "echo hi");
        a.on_key(key(KeyCode::Esc));
        assert!(!a.shell_mode(), "shell mode left by Esc");
        assert_eq!(a.input_value(), "", "partial command cleared");
    }

    #[test]
    fn backspace_out_of_empty_command_exits_shell_mode() {
        // VAL-TUI-057: backspacing on an empty command returns to chat; with text it edits instead.
        let mut a = app();
        a.on_key(key(KeyCode::Char('!')));
        type_str(&mut a, "ab");
        a.on_key(key(KeyCode::Backspace));
        assert!(
            a.shell_mode(),
            "still in shell mode while a command remains"
        );
        assert_eq!(a.input_value(), "a");
        a.on_key(key(KeyCode::Backspace)); // now empty
        assert!(
            a.shell_mode(),
            "empty command keeps shell mode until next backspace"
        );
        a.on_key(key(KeyCode::Backspace)); // empty -> exit
        assert!(
            !a.shell_mode(),
            "shell mode left by backspacing an empty command"
        );
    }

    #[test]
    fn empty_shell_command_is_a_noop() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('!')));
        a.on_key(key(KeyCode::Enter));
        assert!(a.shell_mode(), "still in shell mode");
        assert!(
            !matches!(a.transcript.last(), Some(Entry::Shell(_))),
            "no shell entry for an empty command"
        );
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
        // The dispatched command's effect appears: the model picker opens.
        let dialog = a.dialog().expect("the model picker opened");
        assert_eq!(dialog.kind, DialogKind::Models);
    }

    #[test]
    fn enter_after_moving_highlight_dispatches_the_selected_command() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('/')));
        a.on_key(key(KeyCode::Down)); // help -> new
        a.on_key(key(KeyCode::Down)); // new -> sessions
        a.on_key(key(KeyCode::Enter));
        let dialog = a.dialog().expect("the session picker opened");
        assert_eq!(dialog.kind, DialogKind::Sessions);
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

    /// Dispatch a slash command end-to-end (type it then press Enter).
    fn dispatch(a: &mut App, name: &str) {
        type_str(a, &format!("/{name}"));
        a.on_key(key(KeyCode::Enter));
    }

    #[test]
    fn dispatching_sessions_opens_the_session_picker() {
        // VAL-TUI-034: `/sessions` opens a session-picker listing at least the current session.
        let mut a = app();
        dispatch(&mut a, "sessions");
        let dialog = a.dialog().expect("the session picker opened");
        assert_eq!(dialog.kind, DialogKind::Sessions);
        assert!(
            dialog.items.iter().any(|item| item.contains("current")),
            "the current session is listed: {:?}",
            dialog.items
        );
        // Dispatch did not submit anything to the model.
        assert!(a.take_pending_prompt().is_none());
        assert_eq!(a.input_value(), "");
    }

    #[test]
    fn dispatching_models_opens_the_model_picker_including_the_default() {
        // VAL-TUI-035: `/models` opens a model picker listing models incl. the default.
        let mut a = app();
        dispatch(&mut a, "models");
        let dialog = a.dialog().expect("the model picker opened");
        assert_eq!(dialog.kind, DialogKind::Models);
        assert!(
            dialog.items.iter().any(|m| m == "google/gemini-2.5-flash"),
            "the default model is listed: {:?}",
            dialog.items
        );
        // The active model is pre-highlighted.
        assert_eq!(dialog.items[dialog.selected()], "google/gemini-2.5-flash");
    }

    #[test]
    fn dispatching_agents_opens_the_agent_picker() {
        // VAL-TUI-036: `/agents` opens an agent picker with multiple agents.
        let mut a = app();
        dispatch(&mut a, "agents");
        let dialog = a.dialog().expect("the agent picker opened");
        assert_eq!(dialog.kind, DialogKind::Agents);
        for expected in ["build", "plan", "general"] {
            assert!(
                dialog.items.iter().any(|item| item.contains(expected)),
                "agent {expected} listed: {:?}",
                dialog.items
            );
        }
    }

    #[test]
    fn dispatching_themes_opens_the_theme_picker() {
        // VAL-TUI-037: `/themes` opens a theme picker enumerating selectable themes.
        let mut a = app();
        dispatch(&mut a, "themes");
        let dialog = a.dialog().expect("the theme picker opened");
        assert_eq!(dialog.kind, DialogKind::Themes);
        assert!(
            dialog.items.len() > 1,
            "multiple themes: {:?}",
            dialog.items
        );
        assert!(dialog.items.iter().any(|t| t == "dark"));
    }

    #[test]
    fn theme_picker_preselects_the_active_theme() {
        let mut a = app();
        a.set_theme("monokai".to_owned());
        dispatch(&mut a, "themes");
        let dialog = a.dialog().expect("the theme picker opened");
        assert_eq!(
            dialog.items[dialog.selected()],
            "monokai",
            "the active theme is pre-highlighted"
        );
    }

    #[test]
    fn theme_picker_preview_recolors_then_cancel_restores() {
        // VAL-TUI-064: navigating previews a different theme; cancelling restores the pre-open one.
        let mut a = app();
        let original = a.theme().to_owned();
        dispatch(&mut a, "themes");
        a.on_key(key(KeyCode::Down));
        let previewed = a.theme().to_owned();
        assert_ne!(
            previewed, original,
            "moving the highlight previews a different theme"
        );
        a.on_key(key(KeyCode::Esc));
        assert!(!a.dialog_open(), "Esc closes the picker");
        assert_eq!(
            a.theme(),
            original,
            "cancel restores the theme active before the preview"
        );
    }

    #[test]
    fn confirming_a_theme_selection_switches_the_active_theme() {
        // VAL-TUI-062: confirming a different theme switches the active theme (drives the colors).
        let mut a = app();
        let original = a.theme().to_owned();
        dispatch(&mut a, "themes");
        a.on_key(key(KeyCode::Down));
        let chosen = a.theme().to_owned();
        assert_ne!(chosen, original, "a different theme is highlighted");
        // Commit the highlighted theme via the pure path (the on_key Enter path also persists).
        let confirmed = a
            .confirm_theme_selection()
            .expect("the theme picker confirms a selection");
        assert_eq!(confirmed, chosen, "the highlighted theme is committed");
        assert_eq!(a.theme(), chosen, "the active theme is the chosen one");
        assert!(
            a.theme_restore.is_none(),
            "confirming clears the cancel-restore snapshot"
        );
    }

    #[test]
    fn confirm_theme_selection_is_a_noop_for_other_dialogs() {
        let mut a = app();
        dispatch(&mut a, "agents");
        assert!(
            a.confirm_theme_selection().is_none(),
            "only the theme picker commits a theme on confirm"
        );
    }

    #[test]
    fn dispatching_mcp_opens_the_status_view_with_an_empty_state() {
        // VAL-TUI-046: `/mcp` opens the MCP view; with no servers it shows a clear empty state.
        let mut a = app();
        dispatch(&mut a, "mcp");
        let dialog = a.dialog().expect("the MCP view opened");
        assert_eq!(dialog.kind, DialogKind::Mcp);
        assert!(
            dialog.empty.as_deref() == Some("No MCP servers configured.")
                || !dialog.items.is_empty(),
            "an empty state or configured servers are shown"
        );
    }

    #[test]
    fn dispatching_editor_requests_the_external_editor_once() {
        // VAL-TUI-042: `/editor` requests the external editor (the event loop opens it).
        let mut a = app();
        dispatch(&mut a, "editor");
        assert!(!a.dialog_open(), "the editor is not a dialog");
        assert!(a.take_editor_request(), "an editor request is raised");
        assert!(
            !a.take_editor_request(),
            "the request is consumed exactly once"
        );
    }

    #[test]
    fn dispatching_share_produces_a_local_loopback_reference() {
        // VAL-TUI-045: `/share` produces a local (loopback) share reference.
        let mut a = app();
        dispatch(&mut a, "share");
        match a.transcript.last() {
            Some(Entry::Notice(text)) => {
                assert!(text.contains("share"), "a share reference: {text}");
                assert!(text.contains("127.0.0.1"), "a loopback reference: {text}");
            }
            other => panic!("expected a share notice, got {other:?}"),
        }
    }

    #[test]
    fn dialog_arrow_keys_move_the_highlight_and_esc_closes() {
        let mut a = app();
        dispatch(&mut a, "agents");
        assert!(a.dialog_open());
        assert_eq!(a.dialog().unwrap().selected(), 0);
        a.on_key(key(KeyCode::Down));
        assert_eq!(
            a.dialog().unwrap().selected(),
            1,
            "Down moves the highlight"
        );
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.dialog().unwrap().selected(), 0, "Up moves it back");
        a.on_key(key(KeyCode::Esc));
        assert!(!a.dialog_open(), "Esc closes the dialog");
    }

    #[test]
    fn an_open_dialog_is_modal_and_swallows_typing() {
        let mut a = app();
        dispatch(&mut a, "models");
        assert!(a.dialog_open());
        // Typing does not edit the prompt while the dialog is modal.
        type_str(&mut a, "hello");
        assert_eq!(
            a.input_value(),
            "",
            "the prompt is untouched beneath the dialog"
        );
        assert!(a.dialog_open(), "the dialog stays open");
    }

    #[test]
    fn enter_closes_an_open_dialog() {
        // A non-mutating dialog (agents) closes on Enter without side effects.
        let mut a = app();
        dispatch(&mut a, "agents");
        a.on_key(key(KeyCode::Enter));
        assert!(!a.dialog_open(), "Enter dismisses the dialog");
    }

    #[test]
    fn model_picker_selection_updates_the_active_model() {
        // VAL-TUI-074: selecting a different model from the model picker updates the active model
        // (which the footer renders). Exercise the pure selection path so the test does not write
        // to the global config (the on_key Enter path also persists).
        let mut a = app();
        dispatch(&mut a, "models");
        let before = a.status.model.clone();
        // Move the highlight to a different model than the pre-selected active one.
        a.on_key(key(KeyCode::Down));
        let highlighted = {
            let dialog = a.dialog().expect("the model picker is open");
            dialog.items[dialog.selected()].clone()
        };
        assert_ne!(highlighted, before, "a different model is highlighted");
        let chosen = a.select_model().expect("a changed model is committed");
        assert_eq!(chosen, highlighted, "the highlighted model is committed");
        assert_eq!(a.status.model, highlighted, "the active model updates");
    }

    #[test]
    fn model_picker_reselecting_the_active_model_is_a_noop() {
        let mut a = app();
        dispatch(&mut a, "models");
        // The active model is pre-highlighted, so confirming without moving changes nothing.
        assert!(
            a.select_model().is_none(),
            "confirming the already-active model reports no change"
        );
    }

    #[test]
    fn agent_picker_selection_updates_the_active_agent() {
        // VAL-TUI-075: selecting a different agent updates the active agent (rendered in the footer).
        let mut a = app();
        assert_eq!(a.status.agent, "build", "precondition: build is active");
        dispatch(&mut a, "agents");
        a.on_key(key(KeyCode::Down)); // build -> plan
        let chosen = a.select_agent().expect("a changed agent is committed");
        assert_eq!(chosen, "plan", "the highlighted agent is committed");
        assert_eq!(a.status.agent, "plan", "the active agent updates");
    }

    #[test]
    fn agent_picker_reselecting_the_active_agent_is_a_noop() {
        let mut a = app();
        dispatch(&mut a, "agents");
        assert!(
            a.select_agent().is_none(),
            "confirming the already-active agent reports no change"
        );
    }

    #[test]
    fn closing_each_picker_returns_focus_to_the_prompt() {
        // VAL-TUI-076: dismissing any dialog with Esc returns focus to the prompt so typing resumes.
        for command in ["sessions", "models", "agents", "themes", "mcp"] {
            let mut a = app();
            dispatch(&mut a, command);
            assert!(a.dialog_open(), "{command} opened a dialog");
            a.on_key(key(KeyCode::Esc));
            assert!(!a.dialog_open(), "{command} dialog closed on Esc");
            type_str(&mut a, "hello");
            assert_eq!(
                a.input_value(),
                "hello",
                "typing reaches the prompt after closing the {command} dialog"
            );
        }
    }

    #[test]
    fn session_picker_lists_the_current_session() {
        // VAL-TUI-070 (current-session floor): the picker always opens with at least the current
        // session, and the synthetic current row has no persisted id (so it is non-actionable).
        let mut a = app();
        dispatch(&mut a, "sessions");
        let dialog = a.dialog().expect("the session picker opened");
        assert_eq!(dialog.kind, DialogKind::Sessions);
        assert!(
            dialog.items[0].contains("current"),
            "the first row is the current session: {:?}",
            dialog.items
        );
        assert!(
            a.selected_session_id().is_none(),
            "the current row carries no persisted id"
        );
    }

    /// Manually drive the session picker into a state with one persisted entry, bypassing the store
    /// so the rename state-machine can be exercised without touching the filesystem.
    fn sessions_dialog_with_one_persisted(a: &mut App) {
        a.dialog = Some(Dialog {
            kind: DialogKind::Sessions,
            title: "Sessions".to_owned(),
            items: vec![
                "● current — new session".to_owned(),
                "ses_demo  —  old title".to_owned(),
            ],
            selected: 1,
            empty: None,
        });
        a.session_entries = vec![
            SessionEntry {
                id: None,
                title: None,
            },
            SessionEntry {
                id: Some("ses_demo".to_owned()),
                title: Some("old title".to_owned()),
            },
        ];
    }

    #[test]
    fn selected_session_id_targets_the_highlighted_persisted_row() {
        let mut a = app();
        sessions_dialog_with_one_persisted(&mut a);
        assert_eq!(
            a.selected_session_id().as_deref(),
            Some("ses_demo"),
            "the highlighted persisted row is the action target"
        );
    }

    #[test]
    fn pressing_r_enters_rename_mode_and_typing_edits_the_title() {
        // VAL-TUI-072 (input path): `r` opens rename mode seeded with the current title; typing and
        // backspace edit the in-progress name; Esc cancels back to navigation without closing.
        let mut a = app();
        sessions_dialog_with_one_persisted(&mut a);
        a.on_key(key(KeyCode::Char('r')));
        assert_eq!(
            a.session_rename(),
            Some("old title"),
            "rename mode seeds with the existing title"
        );
        // Clear the seed, then type a fresh name.
        for _ in 0.."old title".len() {
            a.on_key(key(KeyCode::Backspace));
        }
        type_str(&mut a, "fresh name");
        assert_eq!(a.session_rename(), Some("fresh name"));
        // Esc cancels the rename but keeps the picker open.
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.session_rename(), None, "Esc cancels the rename");
        assert!(a.dialog_open(), "cancelling a rename keeps the picker open");
    }

    #[test]
    fn rename_mode_is_unavailable_for_the_current_row() {
        let mut a = app();
        dispatch(&mut a, "sessions");
        // The current row (index 0) has no persisted id, so `r` does not enter rename mode.
        a.on_key(key(KeyCode::Char('r')));
        assert_eq!(
            a.session_rename(),
            None,
            "the synthetic current row cannot be renamed"
        );
    }

    #[test]
    fn rename_mode_swallows_navigation_and_action_keys() {
        // While renaming, typing edits the title rather than triggering navigation/delete.
        let mut a = app();
        sessions_dialog_with_one_persisted(&mut a);
        a.on_key(key(KeyCode::Char('r')));
        for _ in 0.."old title".len() {
            a.on_key(key(KeyCode::Backspace));
        }
        // 'd' would delete outside rename mode; here it must be captured as a character.
        type_str(&mut a, "d");
        assert_eq!(a.session_rename(), Some("d"), "'d' is typed, not a delete");
        assert!(a.dialog_open(), "the picker remains open");
    }

    #[test]
    fn ctrl_p_opens_the_command_palette() {
        // VAL-TUI-058: Ctrl+P opens a command-palette overlay listing commands.
        let mut a = app();
        assert!(!a.palette_open(), "no palette before Ctrl+P");
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(a.palette_open(), "Ctrl+P opens the palette");
        assert!(
            !a.palette_matches().is_empty(),
            "the palette lists commands when opened"
        );
        // It opens over the prompt without leaking the `p` keystroke into the input.
        assert_eq!(a.input_value(), "", "Ctrl+P does not type into the prompt");
    }

    #[test]
    fn typing_in_the_palette_filters_to_matches() {
        // VAL-TUI-059: typing narrows the palette to matching commands.
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        type_str(&mut a, "mod");
        assert_eq!(a.palette_query(), Some("mod"));
        let names: Vec<&str> = a.palette_matches().iter().map(|c| c.name).collect();
        assert!(names.contains(&"models"), "models matches: {names:?}");
        assert!(
            !names.contains(&"help"),
            "unrelated commands hidden: {names:?}"
        );
        // The query edits the palette, not the prompt beneath it.
        assert_eq!(a.input_value(), "", "the prompt stays untouched");
    }

    #[test]
    fn palette_backspace_widens_the_match_set() {
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        type_str(&mut a, "mod");
        let narrowed = a.palette_matches().len();
        a.on_key(key(KeyCode::Backspace));
        assert_eq!(a.palette_query(), Some("mo"));
        assert!(
            a.palette_matches().len() >= narrowed,
            "backspacing widens (or holds) the match set"
        );
    }

    #[test]
    fn palette_arrows_move_the_highlight() {
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(a.palette_selected(), 0);
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.palette_selected(), 1, "Down moves the highlight");
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.palette_selected(), 0, "Up moves it back");
    }

    #[test]
    fn selecting_a_palette_entry_executes_the_command() {
        // VAL-TUI-060: highlighting an entry and pressing Enter runs it and closes the palette.
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        type_str(&mut a, "themes");
        a.on_key(key(KeyCode::Enter));
        assert!(
            !a.palette_open(),
            "the palette closes after running a command"
        );
        let dialog = a.dialog().expect("the selected command's dialog opened");
        assert_eq!(dialog.kind, DialogKind::Themes);
    }

    #[test]
    fn palette_is_modal_and_does_not_edit_the_prompt() {
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        type_str(&mut a, "models");
        assert_eq!(
            a.input_value(),
            "",
            "typing edits the palette, not the prompt"
        );
        assert_eq!(a.palette_query(), Some("models"));
    }

    #[test]
    fn esc_closes_the_palette_and_returns_focus_to_the_prompt() {
        // VAL-TUI-061: Esc dismisses the palette and the prompt accepts typing again.
        let mut a = app();
        type_str(&mut a, "draft message");
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(a.palette_open());
        a.on_key(key(KeyCode::Esc));
        assert!(!a.palette_open(), "Esc closes the palette");
        // Focus is back on the prompt: its prior contents are intact and typing appends to them.
        assert_eq!(a.input_value(), "draft message", "the prompt is preserved");
        type_str(&mut a, "!");
        assert_eq!(
            a.input_value(),
            "draft message!",
            "the prompt accepts typing again"
        );
    }

    #[test]
    fn palette_enter_with_no_match_just_closes() {
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        type_str(&mut a, "zzznotreal");
        assert!(a.palette_matches().is_empty(), "no command matches");
        a.on_key(key(KeyCode::Enter));
        assert!(!a.palette_open(), "Enter on no-match closes the palette");
        assert!(!a.dialog_open(), "nothing is executed");
    }

    #[test]
    fn run_editor_with_lands_composed_text_in_the_prompt() {
        // VAL-TUI-042: a deterministic editor command composes text returned to the caller.
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "cortex-editor-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("ed.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'composed via editor' > \"$1\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let composed = run_editor_with(script.to_str().unwrap(), "seed text")
            .expect("the editor runs")
            .expect("a clean exit returns composed text");
        assert_eq!(composed, "composed via editor");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_editor_with_returns_none_when_the_editor_aborts() {
        // A non-zero editor exit leaves the prompt unchanged (no composed text).
        let aborted = run_editor_with("false", "seed").expect("the editor runs");
        assert!(aborted.is_none(), "an aborted edit composes nothing");
    }

    /// Press the default leader chord (Ctrl+G), arming the leader sequence.
    fn press_leader(a: &mut App) {
        a.on_key(key_mod(KeyCode::Char('g'), KeyModifiers::CONTROL));
    }

    #[test]
    fn leader_arms_the_which_key_overlay() {
        // VAL-TUI-068: pressing the leader shows the which-key overlay (the leader-pending state).
        let mut a = app();
        assert!(!a.leader_pending());
        press_leader(&mut a);
        assert!(
            a.leader_pending(),
            "the leader chord arms the which-key overlay"
        );
        // A non-binding key disarms it without side effects.
        a.on_key(key(KeyCode::Esc));
        assert!(!a.leader_pending(), "Esc cancels the leader sequence");
    }

    #[test]
    fn leader_new_session_clears_the_transcript() {
        // VAL-TUI-065: the leader new-session binding starts a fresh session.
        let mut a = app();
        a.transcript.push(Entry::Notice("populated".to_owned()));
        a.history.push(Message::user("hi"));
        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('n')));
        assert!(!a.leader_pending(), "the binding consumed the leader");
        assert!(
            a.transcript.is_empty(),
            "the transcript cleared to a fresh session"
        );
        assert!(a.history_snapshot().is_empty(), "history cleared too");
    }

    #[test]
    fn leader_model_list_opens_the_model_picker() {
        // VAL-TUI-066: the leader model-list binding opens the model picker.
        let mut a = app();
        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('m')));
        assert_eq!(
            a.dialog().map(|dialog| dialog.kind),
            Some(DialogKind::Models),
            "the model picker opened via the leader chord"
        );
    }

    #[test]
    fn leader_sidebar_toggles_on_then_off() {
        // VAL-TUI-067: the sidebar-toggle binding shows then hides the sidebar.
        let mut a = app();
        assert!(!a.sidebar_visible());
        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('b')));
        assert!(a.sidebar_visible(), "first toggle shows the sidebar");
        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('b')));
        assert!(!a.sidebar_visible(), "second toggle hides the sidebar");
    }

    #[test]
    fn unbound_leader_key_is_a_harmless_noop() {
        let mut a = app();
        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('z')));
        assert!(!a.leader_pending(), "the leader is consumed");
        assert!(!a.dialog_open());
        assert!(!a.sidebar_visible());
        assert!(
            a.input_value().is_empty(),
            "the key did not leak into the prompt"
        );
    }

    #[test]
    fn leader_chord_does_not_leak_into_the_prompt() {
        let mut a = app();
        press_leader(&mut a);
        assert!(
            a.input_value().is_empty(),
            "the leader chord is not typed text"
        );
    }

    #[test]
    fn overridden_keybind_takes_effect_over_the_default() {
        // VAL-TUI-069: a config override remaps new-session to `x`; `x` triggers it and the default
        // `n` no longer does.
        let mut a = app();
        let mut keybinds = Keybinds::default();
        keybinds.apply_overrides(
            &toml::from_str(r#"new_session = "x""#).expect("valid override table"),
        );
        a.set_keybinds(keybinds);

        a.transcript.push(Entry::Notice("populated".to_owned()));
        // The default key no longer triggers the action.
        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('n')));
        assert_eq!(
            a.transcript.len(),
            1,
            "the default `n` no longer clears the session"
        );

        // The overridden key does.
        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('x')));
        assert!(
            a.transcript.is_empty(),
            "the overridden `x` starts a fresh session"
        );
    }

    #[test]
    fn custom_leader_chord_arms_the_overlay() {
        let mut a = app();
        let mut keybinds = Keybinds::default();
        keybinds.apply_overrides(&toml::from_str(r#"leader = "ctrl+b""#).expect("valid table"));
        a.set_keybinds(keybinds);

        // The old default leader no longer arms anything.
        a.on_key(key_mod(KeyCode::Char('g'), KeyModifiers::CONTROL));
        assert!(
            !a.leader_pending(),
            "the old leader is inert under the override"
        );
        // The custom leader arms the overlay.
        a.on_key(key_mod(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert!(
            a.leader_pending(),
            "the custom leader arms the which-key overlay"
        );
    }

    #[test]
    fn ctrl_f_opens_find_and_esc_closes_it() {
        let mut a = app();
        a.transcript.push(Entry::Notice("apple pie".to_owned()));
        assert!(a.find().is_none());
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert!(a.find().is_some(), "Ctrl+F opens the find bar");
        a.on_key(key(KeyCode::Esc));
        assert!(a.find().is_none(), "Esc closes the find bar");
    }

    #[test]
    fn find_query_locates_matches() {
        // VAL-TUI-077: a query present in the transcript is located (and thus highlightable).
        let mut a = app();
        a.transcript.push(Entry::Notice("xq apple here".to_owned()));
        a.transcript
            .push(Entry::Notice("another apple line".to_owned()));
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for c in "apple".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.find_status(), (0, 2), "both apple occurrences are found");
    }

    #[test]
    fn find_next_and_prev_move_the_active_match() {
        // VAL-TUI-078: next/prev move the active match (wrapping at the ends).
        let mut a = app();
        a.transcript.push(Entry::Notice("apple one".to_owned()));
        a.transcript.push(Entry::Notice("apple two".to_owned()));
        a.transcript.push(Entry::Notice("apple three".to_owned()));
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for c in "apple".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.find_status(), (0, 3));
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.find_status().0, 1, "Down advances to the next match");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.find_status().0, 2, "Enter advances to the next match");
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.find_status().0, 0, "next wraps back to the first match");
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.find_status().0, 2, "prev wraps to the last match");
    }

    #[test]
    fn find_backspace_edits_the_query() {
        let mut a = app();
        a.transcript.push(Entry::Notice("apple".to_owned()));
        a.transcript.push(Entry::Notice("apricot".to_owned()));
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for c in "ap".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.find_status().1, 2, "`ap` matches both lines");
        for c in "ple".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.find_status().1, 1, "`apple` matches one line");
        for _ in 0..3 {
            a.on_key(key(KeyCode::Backspace));
        }
        assert_eq!(a.find_status().1, 2, "back to `ap` matches both again");
    }

    #[test]
    fn alt_arrows_navigate_messages() {
        // VAL-TUI-079: message navigation moves the selection between messages.
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::User("second".to_owned()));
        a.transcript.push(Entry::User("third".to_owned()));
        assert_eq!(a.selected_message(), None);
        a.on_key(key_mod(KeyCode::Up, KeyModifiers::ALT));
        assert_eq!(
            a.selected_message(),
            Some(2),
            "first selection is the last message"
        );
        a.on_key(key_mod(KeyCode::Up, KeyModifiers::ALT));
        assert_eq!(a.selected_message(), Some(1));
        a.on_key(key_mod(KeyCode::Down, KeyModifiers::ALT));
        assert_eq!(a.selected_message(), Some(2));
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.selected_message(), None, "Esc clears the selection");
    }

    #[test]
    fn ctrl_k_and_ctrl_n_navigate_messages() {
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::User("second".to_owned()));
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(a.selected_message(), Some(1));
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(a.selected_message(), Some(0));
        a.on_key(key_mod(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(a.selected_message(), Some(1));
    }

    #[test]
    fn page_up_reveals_earlier_then_page_down_returns_to_latest() {
        // VAL-TUI-080: page up reveals earlier content; page down returns to the latest.
        let mut a = app();
        for i in 0..100 {
            a.transcript.push(Entry::Notice(format!("line {i}")));
        }
        a.set_viewport(80, 10);
        assert_eq!(a.scroll(), 0, "starts pinned to the latest content");
        a.on_key(key(KeyCode::PageUp));
        let after_up = a.scroll();
        assert!(after_up > 0, "page up scrolls toward earlier content");
        assert!(after_up <= a.max_scroll(), "scroll stays within bounds");
        for _ in 0..200 {
            a.on_key(key(KeyCode::PageDown));
        }
        assert_eq!(a.scroll(), 0, "page down returns to the latest content");
        for _ in 0..200 {
            a.on_key(key(KeyCode::PageUp));
        }
        assert_eq!(a.scroll(), a.max_scroll(), "page up clamps at the top");
    }

    #[test]
    fn copying_a_selected_message_is_acknowledged_and_staged() {
        // VAL-TUI-081: copying a message is acknowledged (and the text is staged for the clipboard).
        let mut a = app();
        a.transcript.push(Entry::User("hello world".to_owned()));
        a.on_key(key_mod(KeyCode::Up, KeyModifiers::ALT));
        a.on_key(key_mod(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert!(a.ack().is_some(), "the copy is acknowledged");
        assert_eq!(
            a.take_copy_request().as_deref(),
            Some("hello world"),
            "the selected message text is staged for the clipboard"
        );
    }

    #[test]
    fn copy_without_selection_copies_the_latest_message() {
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::Notice("latest note".to_owned()));
        a.on_key(key_mod(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert_eq!(a.take_copy_request().as_deref(), Some("latest note"));
        assert!(a.ack().is_some());
    }

    #[test]
    fn copy_with_empty_transcript_is_graceful() {
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('y'), KeyModifiers::CONTROL));
        let toast = a
            .toasts()
            .last()
            .expect("a toast explains there is nothing to copy");
        assert_eq!(
            toast.level,
            ToastLevel::Error,
            "nothing-to-copy is surfaced as a distinct error toast"
        );
        assert!(
            a.take_copy_request().is_none(),
            "nothing is staged for the clipboard"
        );
    }

    #[test]
    fn share_surfaces_a_transient_toast() {
        // VAL-TUI-082: a notifying action (here `/share`) surfaces a transient toast.
        let mut a = app();
        dispatch(&mut a, "share");
        assert!(a.has_toasts(), "the share notification surfaces as a toast");
        assert_eq!(
            a.toasts().last().unwrap().level,
            ToastLevel::Success,
            "a successful action shows a success-level toast"
        );
    }

    #[test]
    fn error_toast_uses_error_level() {
        // VAL-TUI-084: an error notification is distinctly an error-level toast (rendered with the
        // error color/icon/label), separate from a success toast.
        let mut a = app();
        a.note_error("boom".to_owned());
        let toast = a.toasts().last().expect("an error toast is queued");
        assert_eq!(toast.level, ToastLevel::Error, "error styling is distinct");
        assert_eq!(toast.message, "boom");
        // The success path is a different level, so the two are visually distinguishable.
        a.push_toast(ToastLevel::Success, "ok");
        assert_eq!(a.toasts().last().unwrap().level, ToastLevel::Success);
    }

    #[test]
    fn toasts_auto_dismiss_when_expired() {
        // VAL-TUI-083: a toast auto-dismisses after its window elapses, leaving no residue.
        let mut a = app();
        a.push_toast(ToastLevel::Info, "hello");
        assert!(a.has_toasts(), "the toast is shown right after the action");
        // Force the toast past its lifetime, then tick: it should be reaped.
        for toast in &mut a.toasts {
            toast.duration = Duration::from_millis(0);
        }
        a.tick_toasts();
        assert!(
            !a.has_toasts(),
            "an expired toast is removed so it does not stick permanently"
        );
    }

    #[test]
    fn toast_queue_is_bounded() {
        let mut a = app();
        for i in 0..(MAX_TOASTS + 3) {
            a.push_toast(ToastLevel::Info, format!("toast {i}"));
        }
        assert_eq!(
            a.toasts().len(),
            MAX_TOASTS,
            "a burst of notifications is bounded to the retention cap"
        );
        // The oldest are dropped, so the newest survive.
        assert_eq!(a.toasts().last().unwrap().message, "toast 6");
    }

    #[test]
    fn submitting_resets_navigation_to_the_latest() {
        let mut a = app();
        for i in 0..40 {
            a.transcript.push(Entry::Notice(format!("note {i}")));
        }
        a.set_viewport(80, 10);
        // Select a message first (which scrolls it into view), then page up so the view is both
        // scrolled away from the bottom and has a selection.
        a.on_key(key_mod(KeyCode::Up, KeyModifiers::ALT));
        a.on_key(key(KeyCode::PageUp));
        assert!(a.scroll() > 0 && a.selected_message().is_some());
        for c in "hello".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.scroll(), 0, "submitting returns the view to the latest");
        assert_eq!(
            a.selected_message(),
            None,
            "submitting clears the selection"
        );
    }
}
