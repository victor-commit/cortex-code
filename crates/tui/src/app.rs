//! Application state and key handling for the Cortex Code TUI scaffold.
//!
//! The prompt's editing model is the ported `tui_widgets::TextArea`, which provides
//! readline-style editing and multi-line text. Rendering is done with `ratatui`
//! (see [`crate::ui`]); this module only owns state and translates key events.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use checkpoints::{RestoreOutcome, ShadowStore};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use engine::{Agent, RunnerEvent, extract_message_content_parts};
use llm::{Message, MessageContentPart, Usage};
use permissions::{Decision, ScopedDecision};
use plugins::PluginEventHooks;
use protocol::{MessagePart, MessagePartBody};
use tui_components::toast::{Toast, ToastLevel};
use tui_widgets::{CursorPosition, TextArea, TextAreaKey, TextAreaModifiers};

use crate::attachments::InputAttachment;
use crate::glyphs;
use crate::keybind::{KeyChord, KeybindAction, Keybinds};
use crate::mention;
use crate::permission::PermissionAsk;
use crate::remote::RemoteServer;
use crate::shell;
use crate::slash;
use crate::tool_card::ToolCard;
use crate::turn::TurnMessage;

/// The cap on the number of files surfaced in the `@`-mention picker at once, so a broad query (or a
/// bare `@`) cannot build an unbounded row list; the popup scrolls within this set.
const MENTION_MATCH_LIMIT: usize = 200;

/// The dim reason shown on the plan-mode `blocked` banner when a shell/build command is hard-denied
/// while planning (the read-only plan agent disables the local shell).
pub(crate) const SHELL_BLOCKED_WHILE_PLANNING: &str = "shell is disabled while planning";

/// How long a transient toast stays on screen before it auto-dismisses. Short enough to read as a
/// momentary notification, long enough to be seen; the same window is used for every level so the
/// auto-dismiss timing is predictable.
const TOAST_DURATION: Duration = Duration::from_millis(2500);

/// The most toasts kept at once; older toasts are dropped so a burst of notifications cannot grow
/// without bound (and the overlay never overruns the screen).
const MAX_TOASTS: usize = 4;
/// Short window where Esc-prefixed bytes/keys are treated as a possible noisy arrow sequence.
const NAV_ESCAPE_PROBE_WINDOW: Duration = Duration::from_millis(120);
/// Maximum buffered chars after Esc while deciding whether they are a noisy arrow sequence.
const NAV_ESCAPE_PROBE_MAX_CHARS: usize = 6;

/// Active poll cadence (and per-frame duration) while something is animating. Every animation
/// derives its phase from [`Instant::elapsed`] divided by this value, so the cadence and the frame
/// period are the same number and a variable wake interval never drifts the phase.
const ANIMATION_TICK_MS: u64 = 100;
/// Idle poll cadence: the loop blocks this long when nothing is animating, so an idle TUI never
/// busy-spins (the wakeups only drive cheap housekeeping like remote polling and probe expiry).
const IDLE_POLL_MS: u64 = 200;
/// Half-period of the streaming caret blink: the caret is shown for this long, then hidden for the
/// same span, so a full on/off cycle is twice this value.
const CARET_BLINK_HALF_PERIOD_MS: u128 = 500;
/// The fixed breathing-spinner frame shown as a static working mark when animations are disabled.
const STATIC_SPINNER_FRAME: usize = 2;
/// Duration of the picker selection-highlight transition: moving the highlight one row cross-fades
/// the inverse bar from the old row to the new one over this span, then settles instantly.
const SELECTION_TRANSITION_MS: u128 = 140;

/// Caret blink state as a pure function of elapsed time: visible for the first half-period, hidden
/// for the second, repeating. Same elapsed input always yields the same result (deterministic).
#[must_use]
fn caret_visible_at(elapsed_ms: u128) -> bool {
    (elapsed_ms / CARET_BLINK_HALF_PERIOD_MS) % 2 == 0
}

/// An in-flight picker selection-highlight transition: the inverse bar interpolates from row `from`
/// to row `to` starting at `start`. Only created while animations are enabled and the move was a
/// single adjacent step; it expires on its own after [`SELECTION_TRANSITION_MS`].
#[derive(Debug, Clone, Copy)]
struct SelectionTransition {
    from: usize,
    to: usize,
    start: Instant,
}

/// Status line contents reported in the footer.
#[derive(Debug, Clone)]
pub struct Status {
    /// Active model id (e.g. `google/gemini-2.5-flash`).
    pub model: String,
    /// Active agent name.
    pub agent: String,
    /// Active mode.
    pub mode: String,
    /// Active variant.
    pub variant: String,
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

/// A chat message submitted while a turn was still streaming. Rather than being dropped, it is held
/// here and promoted into its own turn once the active turn settles (queued, not lost).
#[derive(Debug, Clone, PartialEq, Eq)]
struct QueuedPrompt {
    /// The user-visible header shown in the transcript, set only when `@`-mention expansion changed
    /// the sent text; `None` means the typed text is the prompt verbatim.
    header: Option<String>,
    /// The expanded prompt actually sent to the model.
    prompt: String,
    /// Non-text attachments captured at submit-time for this queued message.
    attachments: Vec<InputAttachment>,
}

/// An interactive permission prompt awaiting the user's decision.
///
/// Surfaced when an ask-gated tool runs during a turn: the turn worker is blocked until the user
/// approves or denies, at which point the carried [`PermissionAsk`] is answered to unblock it. The
/// `selected` option (0 = Allow, 1 = Deny) tracks the highlighted choice for arrow/Enter selection.
#[derive(Debug)]
struct PendingPermission {
    /// The ask carrying the gated tool, its subject, and the reply channel to the worker.
    ask: PermissionAsk,
    /// The highlighted option: 0 = Allow once, 1 = Allow always, 2 = Deny.
    selected: usize,
    /// Optional corrective feedback entered when denying.
    feedback: String,
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
    /// A shell/build command that was hard-denied while plan mode is active, rendered as a red
    /// `▌ blocked` banner naming the command and a dim reason (`shell is disabled while planning`).
    /// The command is NOT executed; the banner is the only trace it leaves.
    Blocked { command: String, reason: String },
    /// A turn-level error surfaced to the user.
    Error(String),
    /// A setup/turn failure that could not start or complete (missing credential, bad model,
    /// upstream error). Rendered as the in-conversation `✦ Cortex` / `✗ error` block per
    /// `err_inline.png` (distinct from the incidental [`Entry::Error`] used for attachment/editor
    /// failures), never as a toast.
    TurnError(String),
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
    /// The skill picker (`/skills`).
    Skills,
    /// The variant picker (`/variant`).
    Variants,
    /// The theme picker (`/themes`).
    Themes,
    /// The subagent picker (`/subagents`).
    Subagents,
    /// Attachment file picker (`/attach`).
    Attachments,
    /// Fork picker (`/fork`) backed by timeline entries.
    Fork,
    /// Export format picker (`/export`).
    Export,
}

/// The session-export formats offered by the `/export` modal: a display name, a one-line
/// description, and the file extension written on confirm. The first entry is the default format
/// (it carries the `default` badge in the picker).
const EXPORT_FORMATS: &[(&str, &str, &str)] = &[
    ("Markdown", "readable .md — headings + code", "md"),
    ("JSON", "structured messages (.json)", "json"),
    ("HTML", "styled, shareable (.html)", "html"),
    ("Patch", "git-applyable diff (.patch)", "patch"),
];

/// Sidebar panel tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPanel {
    Context,
    Files,
    Lsp,
    Mcp,
    Todo,
    Subagents,
    Queue,
}

impl SidebarPanel {
    pub const ALL: &'static [SidebarPanel] = &[
        SidebarPanel::Context,
        SidebarPanel::Files,
        SidebarPanel::Lsp,
        SidebarPanel::Mcp,
        SidebarPanel::Todo,
        SidebarPanel::Subagents,
        SidebarPanel::Queue,
    ];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            SidebarPanel::Context => "Context",
            SidebarPanel::Files => "Files",
            SidebarPanel::Lsp => "LSP",
            SidebarPanel::Mcp => "MCP",
            SidebarPanel::Todo => "Todo",
            SidebarPanel::Subagents => "Subagents",
            SidebarPanel::Queue => "Queue",
        }
    }

    #[must_use]
    pub const fn short(self) -> &'static str {
        match self {
            SidebarPanel::Context => "Ctx",
            SidebarPanel::Files => "Files",
            SidebarPanel::Lsp => "LSP",
            SidebarPanel::Mcp => "MCP",
            SidebarPanel::Todo => "Todo",
            SidebarPanel::Subagents => "Subs",
            SidebarPanel::Queue => "Queue",
        }
    }

    /// The lowercase breadcrumb crumb shown after `✦ Cortex Code ·` in the split header (one of
    /// context/files/lsp/mcp/todo/subagents/queue), matching the active panel.
    #[must_use]
    pub const fn crumb(self) -> &'static str {
        match self {
            SidebarPanel::Context => "context",
            SidebarPanel::Files => "files",
            SidebarPanel::Lsp => "lsp",
            SidebarPanel::Mcp => "mcp",
            SidebarPanel::Todo => "todo",
            SidebarPanel::Subagents => "subagents",
            SidebarPanel::Queue => "queue",
        }
    }
}

/// One file listed in the sidebar `context` panel: a display name plus its estimated token
/// contribution to the conversation context (rendered as a right-aligned `k`/`M` figure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFileRow {
    /// The file's display name (basename).
    pub name: String,
    /// The formatted per-file token size (e.g. `8.1k`), right-aligned in the panel.
    pub tokens: String,
}

/// The worst diagnostic severity across a file's LSP diagnostics, driving the color of the file
/// row's `●` dot in the sidebar `lsp` panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspFileSeverity {
    /// The file has at least one error diagnostic — red dot.
    Error,
    /// The file has warnings but no errors — amber dot.
    Warning,
    /// The file was checked and has no diagnostics — a neutral dot plus a green `○ no problems` row.
    Clean,
}

/// One diagnostic under a file in the sidebar `lsp` panel: a one-based line number, whether it is
/// an error (`E`, red) or a warning (`W`, amber), and the message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspDiagnosticRow {
    /// One-based line number the diagnostic points at.
    pub line: u32,
    /// `true` for an error (`E`, red), `false` for a warning (`W`, amber).
    pub is_error: bool,
    /// The diagnostic message text.
    pub message: String,
}

/// One diagnosed (or clean) file in the sidebar `lsp` panel: the display filename, the error and
/// warning counts (from the diagnostics summary, authoritative for the panel note), and the
/// individual diagnostic rows beneath it (empty when the file is clean).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspFileGroup {
    /// The file's display name (basename).
    pub file: String,
    /// Error count reported for the file (summary count; may exceed the shown rows if truncated).
    pub errors: usize,
    /// Warning count reported for the file.
    pub warnings: usize,
    /// The individual diagnostic rows; empty when the file is clean.
    pub diagnostics: Vec<LspDiagnosticRow>,
}

impl LspFileGroup {
    /// The worst severity across the file's diagnostics, driving the file dot color. A file with
    /// no errors and no warnings is [`LspFileSeverity::Clean`].
    #[must_use]
    pub fn severity(&self) -> LspFileSeverity {
        let has_error = self.errors > 0 || self.diagnostics.iter().any(|d| d.is_error);
        let has_warning = self.warnings > 0 || self.diagnostics.iter().any(|d| !d.is_error);
        if has_error {
            LspFileSeverity::Error
        } else if has_warning {
            LspFileSeverity::Warning
        } else {
            LspFileSeverity::Clean
        }
    }
}

/// The semantic state of a child subagent row in the sidebar `subagents` panel, driving the color
/// of its right-aligned status word (running → ORANGE, queued → FAINT, done → OKG green, error →
/// ERR red).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentState {
    /// The subagent is actively working (status colored ORANGE).
    Running,
    /// The subagent is waiting to start (status colored FAINT grey).
    Queued,
    /// The subagent finished successfully (status colored OKG green).
    Done,
    /// The subagent finished with an error (status colored ERR red).
    Error,
}

/// One child subagent row in the sidebar `subagents` panel: an agent name, its task description
/// (shown DIM on the following line), a status word (`running 8s` / `queued` / `done 1.2s`), and
/// the [`SubagentState`] that colors the status. Colors are applied by the renderer; this carries
/// only the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarSubagentRow {
    /// The subagent's agent name (e.g. `explore`, `plan`, `general`), shown BRIGHT.
    pub name: String,
    /// The subagent's task description, shown DIM on the line beneath the name.
    pub task: String,
    /// The right-aligned status word with elapsed time (`running 8s`, `queued`, `done 1.2s`).
    pub status: String,
    /// The semantic state driving the status color.
    pub state: SubagentState,
}

/// The visual state of a `files` tree row: an expanded/collapsed directory (with its heavy `▼`/`▶`
/// disclosure glyph) or a leaf file that is modified (inline `●`) or unchanged (no marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTreeKind {
    /// A directory the tree currently shows expanded (`▼`), its children listed below.
    DirExpanded,
    /// A directory the tree currently shows collapsed (`▶`), its children hidden.
    DirCollapsed,
    /// A leaf file with uncommitted modifications (inline `●` marker).
    FileModified,
    /// A leaf file with no uncommitted changes (no marker).
    File,
}

/// One flattened row of the sidebar `files` collapsible tree: a depth (for `depth*2` indentation),
/// a display name, its [`FileTreeKind`], and the absolute path used to toggle a directory node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTreeRow {
    /// Nesting depth from the workspace root (0 = top level); children indent by `depth*2`.
    pub depth: usize,
    /// The entry's display name (basename); directories carry a trailing `/`.
    pub name: String,
    /// Whether the row is an expanded/collapsed directory or a modified/unmodified file.
    pub kind: FileTreeKind,
    /// The absolute on-disk path, used to flip a directory's expand/collapse state on toggle.
    path: PathBuf,
}

impl FileTreeRow {
    /// Whether this row is a directory (expanded or collapsed), i.e. a togglable node.
    #[must_use]
    pub fn is_dir(&self) -> bool {
        matches!(
            self.kind,
            FileTreeKind::DirExpanded | FileTreeKind::DirCollapsed
        )
    }
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

/// The state of the `/rename` dialog: the session being renamed, its current title (quoted in the
/// prompt), and the in-progress new title (edited by typing, committed with Enter, cancelled with
/// Esc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameDialog {
    /// The persisted id of the session being renamed.
    pub session_id: String,
    /// The session's current title, shown quoted in the prompt (`None` when it is untitled).
    pub current_title: Option<String>,
    /// The in-progress new title text.
    pub buffer: String,
}

/// The state of the `/compact` dialog: the before/after context comparison and the kept-summary
/// bullets shown (over a dimmed backdrop) before the user confirms compaction. Confirming (Enter)
/// runs the compaction; Esc cancels and leaves the conversation unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactDialog {
    /// The context size before compaction (formatted token count, e.g. `42,318 tokens`).
    pub before_tokens: String,
    /// A human count of the messages that will be summarized (e.g. `12 messages`).
    pub before_detail: String,
    /// The estimated context size after compaction (formatted, prefixed `~`, e.g. `~8,400 tokens`).
    pub after_tokens: String,
    /// A human description of what remains (e.g. `1 summary + 2 recent`).
    pub after_detail: String,
    /// The bullets describing what the summary keeps (recent prompt excerpts).
    pub keeps: Vec<String>,
}

/// One `/init` scan-log step: a completed step (rendered `✓` in OKG green) or the active/pending
/// step (rendered `›` in orange).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitStep {
    /// The step description (e.g. `detected Rust workspace (cargo)`).
    pub label: String,
    /// Whether the step has completed (`✓`) or is the active/pending one (`›`).
    pub done: bool,
}

/// The state of the `/init` dialog: the scanned project root and the real scan-log steps shown
/// (over a dimmed backdrop) before the user confirms writing AGENTS.md. Confirming (Enter) runs the
/// project-initialization flow; Esc cancels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitDialog {
    /// The scanned project root, shown as `Scanning <root> …`.
    pub root: String,
    /// The real scan-log steps (done `✓` / pending `›`).
    pub steps: Vec<InitStep>,
}

/// Diff viewer render mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffViewMode {
    /// Traditional unified diff (`+` / `-` in one stream).
    Unified,
    /// Side-by-side split diff (before | after).
    Split,
}

/// Which pane currently owns keyboard navigation in the diff viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffViewerFocus {
    /// Left file tree (select file).
    Files,
    /// Right diff pane (select hunk).
    Hunks,
}

/// Modal state for the `/diff` viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffViewer {
    /// File diffs shown in the file tree (one per path, latest change wins).
    pub files: Vec<crate::diff::FileDiff>,
    /// Selected file index in `files`.
    pub selected_file: usize,
    /// Selected hunk index in the selected file.
    pub selected_hunk: usize,
    /// Active render mode.
    pub mode: DiffViewMode,
    /// Active keyboard focus pane.
    pub focus: DiffViewerFocus,
    /// Per-file disclosure state: `expanded[i]` is whether file `i`'s hunk body is shown. Parallel
    /// to `files`, so it is kept in sync whenever a file is removed (after a revert).
    pub expanded: Vec<bool>,
}

impl DiffViewer {
    fn new(files: Vec<crate::diff::FileDiff>) -> Self {
        // The first changed file opens expanded (its hunks visible); the rest start collapsed,
        // matching the diff-viewer reference (one open file above collapsed `▶` file rows).
        let expanded = (0..files.len()).map(|i| i == 0).collect();
        Self {
            files,
            selected_file: 0,
            selected_hunk: 0,
            mode: DiffViewMode::Unified,
            focus: DiffViewerFocus::Files,
            expanded,
        }
    }

    #[must_use]
    pub fn selected_file(&self) -> Option<&crate::diff::FileDiff> {
        self.files.get(self.selected_file)
    }

    /// Whether file `index`'s hunk body is currently disclosed (`▼`) rather than collapsed (`▶`).
    #[must_use]
    pub fn is_expanded(&self, index: usize) -> bool {
        self.expanded.get(index).copied().unwrap_or(false)
    }

    fn clamp_selection(&mut self) {
        if self.files.is_empty() {
            self.selected_file = 0;
            self.selected_hunk = 0;
            return;
        }
        self.selected_file = self.selected_file.min(self.files.len() - 1);
        let hunk_count = self
            .files
            .get(self.selected_file)
            .map_or(0, |file| file.hunks.len());
        if hunk_count == 0 {
            self.selected_hunk = 0;
        } else {
            self.selected_hunk = self.selected_hunk.min(hunk_count - 1);
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

/// One row of the rich session picker: its display row (title + `agent · model` + `N msg · age`),
/// the backing persisted id/title (for open/rename/delete), and the lowercased text the search row
/// filters against. The full set is cached when the picker opens; the visible rows are this set
/// filtered by the query, so filtering never re-reads the store.
#[derive(Debug, Clone)]
struct SessionPick {
    /// The persisted session id, or `None` for the synthetic "current session" floor row.
    id: Option<String>,
    /// The persisted session's title (used to seed a rename), if any.
    title: Option<String>,
    /// The display row (key/desc/meta) handed to the modal renderer.
    row: crate::modal::ModalRow,
    /// Lowercased haystack (title + model) the search query matches against.
    hay: String,
}

/// One row of the rich model picker: its display row (`provider/model` + description + `ctx · price`),
/// the fully-qualified model id, the vendor prefix (for the `⇥` provider grouping), and the
/// lowercased search haystack. The full set is cached when the picker opens; the flat/grouped views
/// are derived from it filtered by the query.
#[derive(Debug, Clone)]
struct ModelPick {
    /// The fully-qualified model id (e.g. `google/gemini-2.5-flash`).
    id: String,
    /// The vendor prefix before the first `/` (e.g. `google`), used to group by provider.
    vendor: String,
    /// The display row (key/desc/meta) handed to the modal renderer. Its key carries a leading
    /// connected marker when the serving provider has a stored credential.
    row: crate::modal::ModalRow,
    /// Lowercased haystack (id + display name + description) the search query matches against, so
    /// type-to-filter narrows across providers by id or name.
    hay: String,
}

/// A background models.dev refresh result, published to the UI thread over an mpsc channel and
/// drained by [`App::poll_models_refresh`]. Mirrors the OAuth/turn background pattern so the live
/// fetch runs OFF the synchronous ratatui event loop (the loop never blocks on the network).
enum ModelsRefreshMessage {
    /// The live models.dev fetch (and best-effort cache write) succeeded; carries the curated
    /// catalogue that replaces the offline one in place. Boxed so the enum is not dominated by this
    /// large variant.
    Refreshed(Box<providers::Catalogue>),
    /// The fetch failed (transport/parse, or the process went offline again). The picker degrades
    /// silently to the offline list — no toast, no splatter.
    Failed,
}

/// One cached skill-picker entry: its name, description, and source label (`builtin`/`project`/
/// `plugin`). Kept so the rich rows (with the enabled `✓` marker) can be rebuilt on each Space
/// toggle without re-running discovery.
#[derive(Debug, Clone)]
struct SkillPick {
    /// The skill's model-facing name (the picker key).
    name: String,
    /// The skill's short description.
    desc: String,
    /// Where the skill came from: `builtin` (global/config dirs), `project` (under the cwd), or
    /// `plugin` (a plugin-provided slot).
    source: String,
}

/// One row of the subagent picker: the agent name (key), its task description, a status word with
/// elapsed time (`running 8s`/`queued`/`done 2s`), the raw state (for the semantic meta color), and
/// the backing child session id (for `↵ view`).
#[derive(Debug, Clone)]
struct SubagentPick {
    /// The backing child session id (opened on `↵ view`).
    id: String,
    /// The subagent's agent name (e.g. `explore`, `plan`, `general`).
    name: String,
    /// The subagent's task description.
    task: String,
    /// The status meta text (`running 8s`, `queued`, `done 2s`).
    status: String,
    /// The raw state (`running`/`queued`/`done`/`error`) driving the meta color.
    state: &'static str,
}

/// The five brand themes shown in the theme picker, in reference order: display name, one-word
/// descriptor, and a right-aligned character tag. Display names are accepted by
/// [`frame::style::ThemeColors::from_name`] (e.g. `vscode-dark` resolves to the `vscode` chrome).
const THEME_PICKS: &[(&str, &str, &str)] = &[
    ("dark", "cortex default", "warm"),
    ("light", "daytime", "cool"),
    ("vscode-dark", "familiar", "neutral"),
    ("monokai", "vivid", "high-contrast"),
    ("ocean", "cool blues", "calm"),
];

/// One timeline row in the `/timeline` and `/fork` dialogs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TimelineEntry {
    session_id: String,
    message_id: String,
    label: String,
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

/// A plugin-provided completion candidate staged by the first Tab press.
///
/// We remember the candidate together with the cursor and active span used for the completion
/// request so the second Tab can replace only that span (or insert at the cursor when the span is
/// empty), preserving surrounding prompt text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingTextCompletion {
    candidate: String,
    cursor: usize,
    span: Range<usize>,
}

/// The open command palette: the search query typed into it and the highlighted row.
///
/// The palette is a modal, searchable overlay (opened with `Ctrl+P`) that lists every built-in
/// command and runs the selected one directly, independent of the prompt's `/` autocomplete. Its
/// query is kept separate from the prompt so dismissing the palette restores the untouched chat
/// input and returns focus to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteScope {
    Root,
    Models,
    Skills,
    Variants,
    Subagents,
    QueuedPrompts,
}

/// A rendered command-palette row for the current scope/query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteMatch {
    /// Primary row label.
    pub label: String,
    /// Secondary descriptive text.
    pub description: String,
    /// Why this row cannot currently be executed.
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PaletteAction {
    DispatchSlash(String),
    OpenScope(PaletteScope),
    CloseScope,
    SetModel(String),
    SetVariant(String),
    SelectSkill(String),
    LoadSession(String),
    QueuePromote(usize),
    QueueDrop(usize),
    QueueClear,
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SlashAction {
    Builtin,
    CustomPrompt { prompt: String },
    McpPrompt { id: String, required: Vec<String> },
    PluginCommand { slot: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttentionEvent {
    Completion,
    Error,
    Permission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AttentionConfig {
    pub enabled: bool,
    pub quiet: bool,
    pub audio: bool,
    pub visual: bool,
    pub completion: bool,
    pub error: bool,
    pub permission: bool,
}

impl Default for AttentionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet: false,
            audio: true,
            visual: true,
            completion: true,
            error: true,
            permission: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashMatch {
    pub name: String,
    pub description: String,
    action: SlashAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CustomSlashCommand {
    name: String,
    description: String,
    prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PaletteItem {
    label: String,
    description: String,
    action: PaletteAction,
    disabled_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Palette {
    /// The search text typed into the palette (filters the command list).
    query: String,
    /// The highlighted match row (clamped to the current match count).
    selected: usize,
    /// Submenu stack (`Root` when newly opened).
    scopes: Vec<PaletteScope>,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            scopes: vec![PaletteScope::Root],
        }
    }
}

/// One prompt draft snapshot (text plus caret), used by history navigation and explicit stash.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptDraft {
    text: String,
    cursor: CursorPosition,
}

/// Prompt-history browsing state for the currently recalled history set.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptHistoryNavigation {
    ordered: Vec<String>,
    index: usize,
    stashed: PromptDraft,
}

/// Frecency metadata for one previously submitted prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PromptHistoryStats {
    count: u64,
    last_used: u64,
}

/// The open find-in-chat search bar: the live query, the active-match index, and the cached match
/// count for the `match N of M` indicator.
///
/// Find is opened with `Ctrl+F` in EDITING mode. While editing, every printable character (incl.
/// `n`/`N`) types into the query with incremental highlight; Enter commits into NAV mode. In NAV
/// mode `n` steps to the next match and `N` to the previous (both wrapping), while any other
/// printable char or Backspace returns to EDITING and refines the query. Esc/Ctrl+F close it. The
/// query highlights every occurrence in the transcript and scrolls the active match into view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindState {
    /// The live search query.
    pub query: String,
    /// The active match index among the ordered matches (clamped to the match count).
    pub active: usize,
    /// The number of matches for the current query (cached for the indicator and navigation).
    pub matches: usize,
    /// Whether the query is committed: `false` while EDITING (every printable char, incl. `n`/`N`,
    /// types into the query), `true` in NAV mode (`n`/`N` step matches). Defaults to `false` on open;
    /// any edit (printable char or Backspace) returns it to EDITING.
    pub committed: bool,
}

/// A short-lived probe for Esc-prefixed noisy arrow sequences (`Esc` + `[A`, `A[`, `Up`, ...).
///
/// In prompt mode, Esc normally clears message selection. Some terminals emit Alt+arrow as Esc plus
/// stray bytes/keys; this probe lets us treat that burst as inert (restoring the prior selection and
/// swallowing stray bytes) instead of leaking fragments into the prompt.
#[derive(Debug, Clone)]
struct NavEscapeProbe {
    started: Instant,
    previous_selection: Option<usize>,
    chars: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavEscapeProbeState {
    Candidate,
    CompleteVerticalArrow,
    NotCandidate,
}

fn nav_escape_probe_state(chars: &str) -> NavEscapeProbeState {
    let bytes = chars.as_bytes();
    let Some(first) = bytes.first().copied() else {
        return NavEscapeProbeState::Candidate;
    };
    match first {
        b'[' => {
            if bytes.len() == 1 {
                return NavEscapeProbeState::Candidate;
            }
            let mut i = 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b';') {
                i += 1;
            }
            if i == bytes.len() {
                return NavEscapeProbeState::Candidate;
            }
            if i == bytes.len() - 1 && matches!(bytes[i], b'A' | b'B') {
                return NavEscapeProbeState::CompleteVerticalArrow;
            }
            NavEscapeProbeState::NotCandidate
        }
        b'O' => {
            if bytes.len() == 1 {
                NavEscapeProbeState::Candidate
            } else if bytes.len() == 2 && matches!(bytes[1], b'A' | b'B') {
                NavEscapeProbeState::CompleteVerticalArrow
            } else {
                NavEscapeProbeState::NotCandidate
            }
        }
        b'A' | b'B' => {
            if bytes.len() == 1 {
                NavEscapeProbeState::Candidate
            } else if bytes.len() == 2 && matches!(bytes[1], b'[' | b'O') {
                NavEscapeProbeState::CompleteVerticalArrow
            } else {
                NavEscapeProbeState::NotCandidate
            }
        }
        _ => NavEscapeProbeState::NotCandidate,
    }
}

/// The instruction sent to the model when the `/init` flow runs (project initialization).
const INIT_PROMPT: &str = "Initialize this project. Analyze the repository structure and the \
    important files, then create (or update) an AGENTS.md file at the project root that documents \
    the project's purpose, layout, key build/test/lint commands, and conventions. Read the \
    existing files first so the summary is accurate.";

/// The instruction sent to the model when the `/compact` flow runs (conversation compaction).
const COMPACTION_PROMPT: &str = "Summarize our conversation so far into a concise summary that \
    preserves the key facts, decisions, code changes, and any open tasks. Respond with only the \
    summary.";

/// Built-in variant options surfaced by keybinds and palette flows.
const VARIANT_OPTIONS: &[&str] = &["default", "fast", "balanced", "quality"];

/// The primary commands the command palette lists first, in the reference order (pick_palette.png).
/// The remaining builtins follow (aliases excluded), so the palette leads with the everyday
/// commands while still exposing the full command set to fuzzy search.
const PALETTE_PRIMARY_COMMANDS: &[&str] = &[
    "new", "sessions", "models", "agents", "themes", "status", "diff", "compact", "export", "init",
];

/// Builtin command names the palette hides because they are pure aliases of a listed command
/// (`/model` → `/models`, `/theme` → `/themes`), so the palette never shows a duplicate row.
const PALETTE_ALIAS_COMMANDS: &[&str] = &["model", "theme"];

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

    /// Build footer usage totals from persisted session usage rows.
    fn from_totals(totals: db::UsageTotals) -> Self {
        let reported = totals.input_tokens > 0
            || totals.output_tokens > 0
            || totals.total_tokens > 0
            || totals.cost.is_some();
        Self {
            reported,
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            total_tokens: totals.total_tokens,
            cost: totals.cost,
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
    /// Attachments staged in the composer for the next submitted prompt.
    composer_attachments: Vec<InputAttachment>,
    /// The user-visible header for the queued turn, set by a slash flow so the transcript shows a
    /// concise label (e.g. `/init`) rather than the verbose instruction. `None` for a plain chat
    /// prompt, where the typed text is shown verbatim.
    pending_header: Option<String>,
    /// Attachments captured for the next turn waiting in `pending_prompt`.
    pending_attachments: Vec<InputAttachment>,
    /// Whether the queued turn compacts the conversation into its summary on completion (set by the
    /// `/compact` flow). Consumed when the turn begins.
    pending_compaction: bool,
    /// The active turn's event channel, present only while a turn is in flight.
    turn_rx: Option<Receiver<TurnMessage>>,
    /// When the active turn started, used to animate the busy indicator. `None` when idle.
    spinner_start: Option<Instant>,
    /// Whether reduced-motion is active: animations are suppressed and the UI renders its final,
    /// static frame (static spinner, no caret blink, no toast/selection motion). Set at launch from
    /// the `--no-anim` CLI flag or the `CORTEX_NO_ANIM` environment preference; consulted by
    /// [`animations_enabled`](App::animations_enabled), which every animation gates on.
    reduced_motion: bool,
    /// The in-flight picker selection-highlight transition, or `None` when the highlight is at rest.
    /// Recorded when the open dialog's selection moves one adjacent row with animations enabled, and
    /// read each frame by [`App::selection_anim_state`] to drive the cross-fade; expires on its own.
    selection_anim: Option<SelectionTransition>,
    /// The user prompt of the in-flight turn, appended to history once the turn finishes.
    turn_prompt: Option<String>,
    /// Attachments submitted with the in-flight user prompt.
    turn_attachments: Vec<InputAttachment>,
    /// Whether the in-flight turn already surfaced an error (so finalization skips the answer).
    turn_errored: bool,
    /// The latest usage event for the in-flight turn, persisted once the turn settles.
    turn_usage: Option<Usage>,
    /// Whether the in-flight turn is a compaction (its summary replaces the conversation history
    /// when it finishes). Set from [`App::pending_compaction`] when the turn begins.
    turn_compaction: bool,
    /// Whether the in-flight turn's user message has already been persisted.
    turn_user_persisted: bool,
    /// Streaming redaction state for assistant-visible answer deltas in the active turn.
    answer_redactor: redaction::StreamingRedactor,
    /// Streaming redaction state for reasoning deltas in the active turn, independent from answer.
    reasoning_redactor: redaction::StreamingRedactor,
    /// Tool-card ids whose detail region (arguments/output) is expanded.
    expanded_tool_ids: BTreeSet<String>,
    /// Highlighted row in the open slash-command menu (index into the filtered matches). Reset to
    /// the top whenever the typed query changes; only meaningful while the menu is open.
    slash_selected: usize,
    /// The open modal dialog (a picker or status view), or `None` when no dialog is shown. While
    /// set it captures navigation/dismissal keys so the prompt is not edited beneath it.
    dialog: Option<Dialog>,
    /// The open `/diff` viewer overlay, or `None` when closed. While set it is modal: keys drive
    /// file/hunk navigation, view-mode toggling, and targeted revert actions.
    diff_viewer: Option<DiffViewer>,
    /// The open full-screen Wave-2 view (status/mcp/…), or `None` when the conversation is shown.
    /// While set it takes over the whole frame with the shared view chrome and is modal: Esc closes
    /// it back to the conversation. See [`crate::view`].
    active_view: Option<crate::view::ViewKind>,
    /// Whether the Wave-7 themes-gallery canonical screen (`/theme-gallery`) is open. While set it
    /// takes over the whole frame with the fixed-structure gallery layout and is modal: Esc closes
    /// it back to the conversation. See [`crate::gallery`].
    theme_gallery_open: bool,
    /// Whether the first-run onboarding welcome screen is showing. Set at launch on a pristine
    /// first run (fresh config + no stored credentials) and cleared once the user presses Enter to
    /// start; while set it takes over the whole frame. See [`crate::onboarding`].
    onboarding: bool,
    /// The interactive auth/login screen state, present while the `cortex auth login` provider
    /// picker + masked-key screen is showing. While `Some` it takes over the whole frame and is
    /// modal (Esc cancels back to a normal launch). See [`crate::auth_login`].
    auth_login: Option<crate::auth_login::AuthLoginState>,
    /// The mid-session `/connect` provider-auth flow state, present while the `/connect` screen is
    /// showing. While `Some` it takes over the whole frame and is modal (Esc backs out a step / off
    /// the flow). See [`crate::connect`].
    connect: Option<crate::connect::ConnectState>,
    /// The `/status` view's data, snapshotted when the view opens (a git subprocess + store reads
    /// are too costly to repeat every redraw). Rebuilt on each `/status` open.
    status_snapshot: Option<crate::view::StatusData>,
    /// The `/timeline` view's event rail, snapshotted from the persisted session when the view
    /// opens. Rebuilt on each `/timeline` open.
    timeline_snapshot: Vec<crate::view::TimelineEvent>,
    /// The `/queue` view's prompt rows, snapshotted from the live queue when the view opens (so the
    /// view is stable while it is open). Rebuilt on each `/queue` open.
    queue_snapshot: Vec<String>,
    /// The highlighted row in the open `/queue` view (arrow-key navigation), clamped to the queue
    /// length.
    queue_selection: usize,
    /// Whether the first `d` of the `/queue` view's vim-style `dd` remove chord is armed; a second
    /// `d` removes the selected prompt and any other key disarms it.
    queue_pending_dd: bool,
    /// The open `@`-file picker, or `None` when closed. While set it captures navigation, selection,
    /// and dismissal keys; a selection inserts an `@path` token and a typed range or whitespace
    /// dismisses it.
    mention: Option<MentionPicker>,
    /// Plugin-driven text completion bridge (`experimental.text.complete`) for interactive prompt
    /// completions.
    plugin_text_complete: Option<PluginEventHooks>,
    /// A plugin completion candidate and its request context waiting for second-Tab insertion.
    pending_text_completion: Option<PendingTextCompletion>,
    /// The open command palette (`Ctrl+P`), or `None` when closed. While set it is modal: it owns a
    /// search query and navigation, runs the highlighted command on Enter, and Esc dismisses it and
    /// returns focus to the prompt.
    palette: Option<Palette>,
    /// Most recent keybind-configuration diagnostics gathered at launch.
    keybind_diagnostics: Vec<String>,
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
    /// Active sidebar tab.
    sidebar_panel: SidebarPanel,
    /// Cached flattened `files`-panel tree, rebuilt when the panel is (re)selected and on each
    /// expand/collapse toggle so rendering never touches the filesystem/git on a bare redraw.
    files_tree: Vec<FileTreeRow>,
    /// The workspace root the files tree is built from (the cwd captured when the panel opened).
    files_root: PathBuf,
    /// Directories the user explicitly EXPANDED in the files tree (override the default state).
    files_expanded: BTreeSet<PathBuf>,
    /// Directories the user explicitly COLLAPSED in the files tree (override the default state).
    files_collapsed: BTreeSet<PathBuf>,
    /// The focused row index in the files tree (the toggle target); clamped to the visible rows.
    files_cursor: usize,
    /// Whether the user has begun navigating the files tree; until then no selection bar is drawn
    /// so a freshly-opened panel design-matches the reference (no highlight).
    files_nav_active: bool,
    /// The persisted-session metadata backing the open session picker, parallel to the dialog's
    /// item rows (index 0 is the synthetic "current session" row). Populated when the session
    /// picker opens; consulted by switch/rename/delete; cleared when the dialog closes.
    session_entries: Vec<SessionEntry>,
    /// Timeline rows backing the `/timeline` and `/fork` dialogs.
    timeline_entries: Vec<TimelineEntry>,
    /// Model-picker row mapping: `Some(model_id)` for selectable model rows, `None` for provider
    /// headers/separators.
    model_rows: Vec<Option<String>>,
    /// The search query typed into a searchable picker (session/model). Filters the visible rows;
    /// reset when the picker opens or closes.
    picker_query: String,
    /// The filtered rich rows (key/desc/meta) for the open searchable picker, parallel to the
    /// dialog's item rows and backing arrays. Empty for non-searchable dialogs.
    picker_rows: Vec<crate::modal::ModalRow>,
    /// The fixed key-column width for the open searchable picker so descriptions line up across
    /// rows (`None` = per-row key length).
    picker_key_width: Option<usize>,
    /// The full session-picker set (unfiltered), cached when the session picker opens.
    session_all: Vec<SessionPick>,
    /// The full model-picker set (unfiltered flat list), cached when the model picker opens.
    model_all: Vec<ModelPick>,
    /// Whether the model picker groups rows under vendor headers (`⇥` toggles it); the default flat
    /// view lists every model in one block.
    model_group_by_provider: bool,
    /// The live-fetched models.dev catalogue for this session, or `None` before any refresh has
    /// landed. When present it is the base the `/models` picker draws from (in place of the offline
    /// cache→embedded snapshot), so a background refresh swaps the picker to the current catalogue
    /// without a manual `cortex models --refresh` or a reopen.
    models_catalogue: Option<providers::Catalogue>,
    /// The in-flight background models.dev refresh channel, present only while a refresh runs. The
    /// worker (a std::thread with its own runtime, mirroring the OAuth/turn pattern) publishes the
    /// curated catalogue here; [`App::poll_models_refresh`] drains it on the event loop. `Some`
    /// while refreshing drives the subtle "refreshing…" picker indicator.
    models_refresh_rx: Option<Receiver<ModelsRefreshMessage>>,
    /// Skill-picker row mapping: selected row -> skill name.
    skill_rows: Vec<String>,
    /// Skill-picker cached entries (name/desc/source), used to rebuild the rich rows with the
    /// enabled `✓` marker on each Space toggle. Populated when the skill picker opens.
    skill_picks: Vec<SkillPick>,
    /// The committed set of enabled skills (VAL-PICK-035): applied when the skill picker confirms.
    enabled_skills: BTreeSet<String>,
    /// The working copy of the enabled-skill set while the skill picker is open: Space toggles it,
    /// Enter commits it into `enabled_skills`, Esc discards it. `None` when the picker is closed.
    skill_pending: Option<BTreeSet<String>>,
    /// Variant-picker row mapping: selected row -> variant id.
    variant_rows: Vec<String>,
    /// Subagent-picker row mapping: selected row -> child session id (for `↵ view`), parallel to
    /// the dialog's item rows. Populated when the subagent picker opens; cleared when it closes.
    subagent_ids: Vec<String>,
    /// While renaming a session in the picker, the in-progress new title text. `None` when not
    /// renaming. Entered with `r`, edited by typing, committed with Enter, cancelled with Esc.
    session_rename: Option<String>,
    /// The open `/rename` dialog (a centered input over the dimmed conversation), or `None` when it
    /// is closed. Owns its own key input while open.
    rename_dialog: Option<RenameDialog>,
    /// The open `/compact` dialog (before/after context comparison over the dimmed conversation), or
    /// `None` when it is closed. Confirming runs the compaction; Esc cancels.
    compact_dialog: Option<CompactDialog>,
    /// The open `/init` dialog (the AGENTS.md scan log over the dimmed conversation), or `None` when
    /// it is closed. Confirming runs the project-initialization flow; Esc cancels.
    init_dialog: Option<InitDialog>,
    /// The `/review` view's data (changed-file count + findings), snapshotted from the real working
    /// tree when the view opens. Rebuilt on each `/review` open.
    review_snapshot: Option<crate::view::ReviewData>,
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
    /// Short-lived Esc-prefix probe used to swallow noisy arrow fragments and avoid spuriously
    /// clearing message selection when terminals emit Alt+arrow as Esc + bytes/arrow.
    nav_escape_probe: Option<NavEscapeProbe>,
    /// A transient acknowledgement (e.g. a copy confirmation) shown above the prompt, cleared on the
    /// next key press so it reads as a momentary status note.
    ack: Option<String>,
    /// Text staged for a real terminal-clipboard copy (OSC 52), consumed once by the event loop.
    copy_request: Option<String>,
    /// Active transient toast notifications (oldest first), rendered as an auto-dismissing overlay.
    /// Error-level toasts render with distinct error styling; entries are removed once their display
    /// window elapses (see [`App::tick_toasts`]).
    toasts: Vec<Toast>,
    /// Structured LSP diagnostics grouped by file, parsed from `write`/`edit`/`lsp` tool results,
    /// backing the sidebar `lsp` panel (per-file worst-severity dots + per-diagnostic E/W rows).
    sidebar_lsp_groups: Vec<LspFileGroup>,
    /// The shadow checkpoint store backing `/undo` and `/redo`, opened for the working directory at
    /// launch. `None` when no data location resolves (so undo/redo degrade to a graceful no-op note
    /// rather than failing). The undo/redo handlers re-read its on-disk journal each invocation, so
    /// it observes snapshots finalized by the background turn worker.
    checkpoints: Option<ShadowStore>,
    /// The live MCP manager connected at TUI launch. It owns server children and a live catalogue
    /// updated by MCP list-change notifications, shared with turn dispatch and the `/mcp` view.
    mcp_manager: Option<Arc<mcp::McpManager>>,
    /// The Tokio runtime that owns MCP background tasks (remote notifications, transports). Kept
    /// alive for the whole TUI process so live tool-list changes continue to arrive.
    mcp_runtime: Option<tokio::runtime::Runtime>,
    /// UI-thread receiver for typed MCP catalogue events published by the manager's event bus.
    mcp_events: Option<Receiver<events::Event>>,
    /// Chat messages submitted while a turn was streaming, in submission order. Each is promoted
    /// into its own turn once the active turn settles, so a mid-stream message is queued rather than
    /// dropped. Cleared when the active turn is interrupted.
    queued_prompts: VecDeque<QueuedPrompt>,
    /// The interrupt signal shared with the active turn worker. Setting it asks the worker to stop
    /// Prompt history frecency table (submitted prompt text -> usage metadata).
    prompt_history: BTreeMap<String, PromptHistoryStats>,
    /// Monotonic counter used for prompt-history recency ordering.
    prompt_history_tick: u64,
    /// Active history-browse state while recalling prompts.
    prompt_history_nav: Option<PromptHistoryNavigation>,
    /// Explicitly stashed prompt draft.
    prompt_stash: Option<PromptDraft>,
    /// Last skill selected from the palette skills submenu.
    selected_skill: Option<String>,
    /// Custom slash commands loaded from config (`[command]`) at startup.
    custom_commands: Vec<CustomSlashCommand>,
    /// Runtime attention/audio cue settings loaded from config.
    attention_config: AttentionConfig,
    /// Pending terminal bell count drained by the event loop.
    pending_bells: usize,
    /// Active transient attention badge shown in the footer while a cue is fresh.
    attention_badge: Option<(String, Instant)>,
    /// streaming at a safe boundary; `None` whenever no turn is in flight.
    cancel_flag: Option<Arc<AtomicBool>>,
    /// The active interactive permission prompt, or `None` when no ask is pending. While set it is
    /// modal: keys drive the approve/deny choice (the worker thread is blocked awaiting the reply).
    pending_permission: Option<PendingPermission>,
    /// The resolved database path durable session persistence writes to, bound at launch via
    /// [`set_session_store`](App::set_session_store). `None` disables persistence (the default in
    /// unit tests, so they never touch a real database); the real launch sets it so interactive
    /// turns are recorded in the shared store and become resumable across surfaces.
    db_path: Option<PathBuf>,
    /// Optional override for the local `/share` reference directory, used by tests to isolate
    /// filesystem side effects from the real user data directory.
    share_data_dir: Option<PathBuf>,
    /// The persisted id (`ses_…`) of the active conversation, or `None` until the first turn (or a
    /// model selection) lazily creates a session, or a session is loaded from the picker. This is
    /// the durable id a headless `cortex run --session <id>` resumes; it is surfaced in the footer
    /// once known so the same conversation is addressable across surfaces.
    session_id: Option<String>,
    /// Attached loopback server, when configured. Used to publish TUI turn events to `/event`,
    /// create server-backed `/share` URLs, and observe server-driven abort requests.
    remote_server: Option<RemoteServer>,
    /// Message ids already represented in the transcript/history. This lets the TUI poll the
    /// shared store and append messages created by the server without duplicating its own messages.
    seen_message_ids: BTreeSet<String>,
    /// Last time the attached-session store was polled for server-created messages.
    last_remote_sync: Option<Instant>,
    /// Last time the attached server was polled for a TUI control request.
    last_remote_control: Option<Instant>,
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
            composer_attachments: Vec::new(),
            pending_header: None,
            pending_attachments: Vec::new(),
            pending_compaction: false,
            turn_rx: None,
            spinner_start: None,
            reduced_motion: false,
            selection_anim: None,
            turn_prompt: None,
            turn_attachments: Vec::new(),
            turn_errored: false,
            turn_usage: None,
            turn_compaction: false,
            turn_user_persisted: false,
            answer_redactor: redaction::StreamingRedactor::new(),
            reasoning_redactor: redaction::StreamingRedactor::new(),
            expanded_tool_ids: BTreeSet::new(),
            slash_selected: 0,
            dialog: None,
            diff_viewer: None,
            active_view: None,
            theme_gallery_open: false,
            onboarding: false,
            auth_login: None,
            connect: None,
            status_snapshot: None,
            timeline_snapshot: Vec::new(),
            queue_snapshot: Vec::new(),
            queue_selection: 0,
            queue_pending_dd: false,
            mention: None,
            plugin_text_complete: None,
            pending_text_completion: None,
            palette: None,
            keybind_diagnostics: Vec::new(),
            editor_requested: false,
            shell_mode: false,
            history: Vec::new(),
            theme: default_theme_name(),
            theme_restore: None,
            keybinds: Keybinds::default(),
            leader_pending: false,
            sidebar_visible: false,
            sidebar_panel: SidebarPanel::Context,
            files_tree: Vec::new(),
            files_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            files_expanded: BTreeSet::new(),
            files_collapsed: BTreeSet::new(),
            files_cursor: 0,
            files_nav_active: false,
            session_entries: Vec::new(),
            timeline_entries: Vec::new(),
            model_rows: Vec::new(),
            picker_query: String::new(),
            picker_rows: Vec::new(),
            picker_key_width: None,
            session_all: Vec::new(),
            model_all: Vec::new(),
            model_group_by_provider: false,
            models_catalogue: None,
            models_refresh_rx: None,
            skill_rows: Vec::new(),
            skill_picks: Vec::new(),
            enabled_skills: BTreeSet::new(),
            skill_pending: None,
            variant_rows: Vec::new(),
            subagent_ids: Vec::new(),
            session_rename: None,
            rename_dialog: None,
            compact_dialog: None,
            init_dialog: None,
            review_snapshot: None,
            scroll: 0,
            viewport: Cell::new((80, 24)),
            find: None,
            selected_message: None,
            nav_escape_probe: None,
            ack: None,
            copy_request: None,
            toasts: Vec::new(),
            sidebar_lsp_groups: Vec::new(),
            checkpoints: None,
            mcp_manager: None,
            mcp_runtime: None,
            mcp_events: None,
            queued_prompts: VecDeque::new(),
            prompt_history: BTreeMap::new(),
            prompt_history_tick: 0,
            prompt_history_nav: None,
            prompt_stash: None,
            selected_skill: None,
            custom_commands: configured_custom_slash_commands(),
            attention_config: AttentionConfig::default(),
            pending_bells: 0,
            attention_badge: None,
            cancel_flag: None,
            pending_permission: None,
            db_path: None,
            share_data_dir: None,
            session_id: None,
            remote_server: None,
            seen_message_ids: BTreeSet::new(),
            last_remote_sync: None,
            last_remote_control: None,
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

    /// Replace keybind diagnostics captured at launch-time config parsing.
    pub fn set_keybind_diagnostics(&mut self, diagnostics: Vec<String>) {
        self.keybind_diagnostics = diagnostics.clone();
        for message in diagnostics {
            self.transcript
                .push(Entry::Notice(format!("Keybind config: {message}")));
        }
    }

    /// Attach the shadow checkpoint store backing `/undo` and `/redo`, used at launch to wire the
    /// store opened for the working directory. `None` leaves undo/redo as graceful no-ops.
    pub fn set_checkpoint_store(&mut self, store: Option<ShadowStore>) {
        self.checkpoints = store;
    }

    /// Attach the connected MCP runtime used by `/mcp` and future TUI turns. `None` leaves MCP as a
    /// configured-only view and keeps the built-in tool set unchanged.
    pub fn set_mcp_runtime(
        &mut self,
        manager: Option<Arc<mcp::McpManager>>,
        runtime: Option<tokio::runtime::Runtime>,
        events: Option<Receiver<events::Event>>,
    ) {
        self.mcp_manager = manager;
        self.mcp_runtime = runtime;
        self.mcp_events = events;
    }

    /// Clone the live MCP manager for a turn worker or shutdown path.
    #[must_use]
    pub fn mcp_manager(&self) -> Option<Arc<mcp::McpManager>> {
        self.mcp_manager.clone()
    }

    /// Take the live MCP manager and runtime for shutdown after the event loop exits.
    pub fn take_mcp_runtime(&mut self) -> Option<(Arc<mcp::McpManager>, tokio::runtime::Runtime)> {
        Some((self.mcp_manager.take()?, self.mcp_runtime.take()?))
    }

    /// Bind durable session persistence to the resolved database path, used at launch so the TUI's
    /// turns are recorded in the shared store and become resumable from headless `cortex run` and
    /// the session picker. `None` (the default) disables persistence so unit tests never touch a
    /// real database; opening per write degrades gracefully when the store cannot be opened.
    pub fn set_session_store(&mut self, path: Option<PathBuf>) {
        self.db_path = path;
    }

    /// Attach plugin text-completion hooks discovered at TUI startup.
    pub fn set_plugin_text_complete(&mut self, hooks: Option<PluginEventHooks>) {
        self.plugin_text_complete = hooks;
    }

    /// Set runtime attention/audio cue behavior from config.
    pub(crate) fn set_attention_config(&mut self, config: AttentionConfig) {
        self.attention_config = config;
    }

    /// Drain queued terminal-bell cues to the event loop.
    pub fn drain_pending_bells(&mut self) -> usize {
        let pending = self.pending_bells;
        self.pending_bells = 0;
        pending
    }

    /// Active attention badge text shown in the footer while a cue is fresh.
    #[must_use]
    pub fn attention_badge(&self) -> Option<&str> {
        self.attention_badge
            .as_ref()
            .map(|(label, _)| label.as_str())
    }

    /// Expire a stale attention badge after its display window elapses.
    pub fn tick_attention(&mut self) {
        if self
            .attention_badge
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            self.attention_badge = None;
        }
    }

    /// Attach or detach the loopback server bridge used by cross-surface validation.
    pub fn set_remote_server(&mut self, server: Option<RemoteServer>) {
        self.remote_server = server;
    }

    /// Load a persisted session at startup, used by `cortex tui --session <id>`.
    pub fn attach_session(&mut self, id: &str) {
        self.load_session(id);
    }

    /// The active model id (the value the footer shows and the next turn runs against), used to
    /// drive a turn against the session's chosen model rather than re-reading global config.
    #[must_use]
    pub fn active_model(&self) -> String {
        self.status.model.clone()
    }

    /// The active agent name (the value the footer shows and the next turn uses for its persona).
    #[must_use]
    pub fn active_agent(&self) -> String {
        self.status.agent.clone()
    }

    /// The active variant value (used to annotate turn request params and UI state).
    #[must_use]
    pub fn active_variant(&self) -> String {
        self.status.variant.clone()
    }

    /// The active keybind set (for rendering the which-key overlay and `/help`).
    #[must_use]
    pub fn keybinds(&self) -> &Keybinds {
        &self.keybinds
    }

    /// Non-fatal keybind diagnostics gathered while loading `[keybinds]`.
    #[must_use]
    pub fn keybind_diagnostics(&self) -> &[String] {
        &self.keybind_diagnostics
    }

    /// Runtime disabled reason (if any) for a keybind action.
    #[must_use]
    pub fn keybind_disabled_reason(&self, action: KeybindAction) -> Option<String> {
        self.keybind_action_disabled_reason(action)
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

    /// The active sidebar tab.
    #[must_use]
    pub fn sidebar_panel(&self) -> SidebarPanel {
        self.sidebar_panel
    }

    /// Every sidebar tab in display order.
    #[must_use]
    pub fn sidebar_panels(&self) -> &'static [SidebarPanel] {
        SidebarPanel::ALL
    }

    /// Sidebar rows for the active tab.
    #[must_use]
    pub fn sidebar_rows(&self) -> Vec<String> {
        match self.sidebar_panel {
            SidebarPanel::Context => self.sidebar_context_rows(),
            SidebarPanel::Files => self.sidebar_files_rows(),
            SidebarPanel::Lsp => self.sidebar_lsp_rows(),
            SidebarPanel::Mcp => self.sidebar_mcp_rows(),
            SidebarPanel::Todo => self.sidebar_todo_rows(),
            SidebarPanel::Subagents => self.sidebar_subagent_rows(),
            SidebarPanel::Queue => self.sidebar_queue_rows(),
        }
    }

    /// The right-aligned note embedded in the active sidebar panel's title border. Every value is
    /// real/dynamic session state (context size, git branch, diagnostic counts, tool/server counts,
    /// todo progress, subagent/queue counts) — never a mockup literal.
    #[must_use]
    pub fn sidebar_note(&self) -> String {
        match self.sidebar_panel {
            SidebarPanel::Context => format_tokens(self.context_total_tokens()),
            SidebarPanel::Files => match git_info() {
                Some(git) => format!("git: {}", git.branch),
                None => format!("git: {}", glyphs::EM_DASH),
            },
            SidebarPanel::Lsp => {
                let (errors, warnings) = self.sidebar_lsp_counts();
                format!("{errors}E {} {warnings}W", glyphs::SEPARATOR)
            }
            SidebarPanel::Mcp => {
                let tools = self.sidebar_mcp_tool_count();
                format!("{tools} tools")
            }
            SidebarPanel::Todo => {
                let (done, total) = self.sidebar_todo_progress();
                format!("{done}/{total}")
            }
            SidebarPanel::Subagents => self.subagent_footer_summary().0.to_string(),
            SidebarPanel::Queue => self.queued_prompts.len().to_string(),
        }
    }

    /// The faint subtitle/header row shown beneath the active sidebar panel's title border (above
    /// the divider rule). Real/dynamic session state, never a mockup literal.
    #[must_use]
    pub fn sidebar_subtitle(&self) -> String {
        match self.sidebar_panel {
            SidebarPanel::Context => {
                let files = self.context_files().len();
                let plural = if files == 1 { "file" } else { "files" };
                format!("{files} {plural} in context")
            }
            SidebarPanel::Files => crate::motd::current_dir_display(),
            SidebarPanel::Lsp => self.sidebar_lsp_server_name(),
            SidebarPanel::Mcp => {
                let servers = self.sidebar_mcp_server_count();
                let plural = if servers == 1 { "server" } else { "servers" };
                format!("{servers} {plural} connected")
            }
            SidebarPanel::Todo => "current task list".to_owned(),
            SidebarPanel::Subagents => "background tasks".to_owned(),
            SidebarPanel::Queue => "runs after this turn".to_owned(),
        }
    }

    /// Total error/warning counts for the LSP note, summed across the diagnosed files. The summary
    /// counts win, falling back to the visible-row counts when a file carries only detail rows.
    fn sidebar_lsp_counts(&self) -> (usize, usize) {
        let mut errors = 0;
        let mut warnings = 0;
        for group in &self.sidebar_lsp_groups {
            let row_errors = group.diagnostics.iter().filter(|d| d.is_error).count();
            let row_warnings = group.diagnostics.iter().filter(|d| !d.is_error).count();
            errors += group.errors.max(row_errors);
            warnings += group.warnings.max(row_warnings);
        }
        (errors, warnings)
    }

    /// The language server name shown as the LSP subtitle, or a neutral placeholder when no
    /// diagnostics have been gathered yet.
    fn sidebar_lsp_server_name(&self) -> String {
        if self.sidebar_lsp_groups.is_empty() {
            "no language server".to_owned()
        } else {
            "rust-analyzer".to_owned()
        }
    }

    /// The structured LSP diagnostics grouped by file for the sidebar `lsp` panel renderer.
    #[must_use]
    pub fn sidebar_lsp_files(&self) -> &[LspFileGroup] {
        &self.sidebar_lsp_groups
    }

    /// The number of MCP tools currently advertised across connected servers.
    fn sidebar_mcp_tool_count(&self) -> usize {
        self.mcp_manager
            .as_ref()
            .map_or(0, |manager| manager.tools().len())
    }

    /// The number of configured MCP servers.
    fn sidebar_mcp_server_count(&self) -> usize {
        self.mcp_manager
            .as_ref()
            .map_or_else(|| configured_mcp_servers().len(), |m| m.statuses().len())
    }

    /// The files currently in the conversation context: every file the session has attached or
    /// changed (staged attachments + the transcript's tool diffs), de-duplicated in discovery
    /// order. Each row carries the file's display name and its estimated token contribution
    /// (byte length / 4). Real/dynamic — an offline session with no attachments/edits is empty.
    #[must_use]
    pub fn context_files(&self) -> Vec<ContextFileRow> {
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        let mut push = |path: &str, rows: &mut Vec<ContextFileRow>| {
            if path.is_empty() || !seen.insert(path.to_owned()) {
                return;
            }
            let name = Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(path)
                .to_owned();
            rows.push(ContextFileRow {
                name,
                tokens: format_tokens(file_token_estimate(path)),
            });
        };
        // Files the model has already seen this turn stack first (most-recent edits), then any
        // staged composer attachments not already counted.
        for entry in &self.transcript {
            if let Entry::Diff(diff) = entry {
                push(&diff.path, &mut rows);
            }
        }
        for attachment in self
            .composer_attachments
            .iter()
            .chain(self.pending_attachments.iter())
            .chain(self.turn_attachments.iter())
        {
            push(&attachment.path, &mut rows);
        }
        rows
    }

    /// The total estimated token size of every file in context (the sum of the per-file estimates),
    /// shown as the context panel's right-aligned note.
    #[must_use]
    pub fn context_total_tokens(&self) -> u64 {
        let mut seen = BTreeSet::new();
        let mut total = 0;
        for entry in &self.transcript {
            if let Entry::Diff(diff) = entry
                && seen.insert(diff.path.clone())
            {
                total += file_token_estimate(&diff.path);
            }
        }
        for attachment in self
            .composer_attachments
            .iter()
            .chain(self.pending_attachments.iter())
            .chain(self.turn_attachments.iter())
        {
            if seen.insert(attachment.path.clone()) {
                total += file_token_estimate(&attachment.path);
            }
        }
        total
    }

    /// Completed/total todo counts for the active session's task list.
    fn sidebar_todo_progress(&self) -> (usize, usize) {
        let Some(session_id) = self.session_id.as_deref() else {
            return (0, 0);
        };
        let Some(db) = self.open_store() else {
            return (0, 0);
        };
        match db.todos().list_for_session(session_id) {
            Ok(items) => {
                let total = items.len();
                let done = items
                    .iter()
                    .filter(|item| item.status.as_str() == "completed")
                    .count();
                (done, total)
            }
            Err(_) => (0, 0),
        }
    }

    /// Footer summary for child subagents linked to the active session.
    #[must_use]
    pub fn subagent_footer_summary(&self) -> (usize, usize, usize) {
        let statuses = self.subagent_statuses();
        let total = statuses.len();
        let running = statuses
            .iter()
            .filter(|(_, _, state, _)| *state == "running" || *state == "queued")
            .count();
        let done = statuses
            .iter()
            .filter(|(_, _, state, _)| *state == "done")
            .count();
        (total, running, done)
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

    fn attention_enabled_for(&self, event: AttentionEvent) -> bool {
        if !self.attention_config.enabled || self.attention_config.quiet {
            return false;
        }
        match event {
            AttentionEvent::Completion => self.attention_config.completion,
            AttentionEvent::Error => self.attention_config.error,
            AttentionEvent::Permission => self.attention_config.permission,
        }
    }

    fn emit_attention(&mut self, event: AttentionEvent) {
        if !self.attention_enabled_for(event) {
            return;
        }
        if self.attention_config.audio {
            self.pending_bells = self.pending_bells.saturating_add(1);
        }
        if self.attention_config.visual {
            let label = match event {
                AttentionEvent::Completion => "completion",
                AttentionEvent::Error => "error",
                AttentionEvent::Permission => "permission",
            };
            self.attention_badge = Some((
                label.to_owned(),
                Instant::now() + Duration::from_millis(2200),
            ));
        }
    }

    /// Drop any toasts whose display window has elapsed. Driven by the event loop so toasts
    /// auto-dismiss on a timer regardless of whether the user is interacting.
    pub fn tick_toasts(&mut self) {
        self.toasts.retain(|toast| !toast.is_expired());
    }

    /// Test-only: drop every active toast immediately. Used by rendering tests that reach a panel
    /// via a panel-switch (which pops a transient `Sidebar: …` toast) so the incidental toast band
    /// does not overlay the panel chrome under assertion.
    #[cfg(test)]
    pub(crate) fn clear_toasts_for_test(&mut self) {
        self.toasts.clear();
    }

    /// Expire or resolve any active Esc-prefix probe on the event-loop tick so buffered text is
    /// not held indefinitely while idle.
    pub fn tick_nav_escape_probe(&mut self) {
        self.expire_nav_escape_probe();
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

    /// The busy-spinner glyph for the current instant, or `None` when idle.
    ///
    /// Uses the safe breathing frame set from [`crate::glyphs`] (the legacy braille spinner is
    /// gone) and derives its phase from elapsed time so it is correct across variable wake
    /// intervals. When animations are disabled it returns a single fixed frame (a static working
    /// mark) so the spinner is present but never cycles.
    #[must_use]
    pub fn spinner_glyph(&self) -> Option<&'static str> {
        let start = self.spinner_start?;
        if !self.animations_enabled() {
            return Some(crate::glyphs::SPINNER_BREATHE[STATIC_SPINNER_FRAME]);
        }
        Some(crate::glyphs::frame_at(
            &crate::glyphs::SPINNER_BREATHE,
            start.elapsed().as_millis(),
            ANIMATION_TICK_MS as u128,
        ))
    }

    /// The rotating spinner glyph shown on a running tool card for the current instant.
    ///
    /// Uses the safe half-circle rotation set (`◐◑◒◓`) from [`crate::glyphs`] — distinct from the
    /// working band's breathing spinner — and derives its phase from elapsed time. Returns a single
    /// fixed frame under reduced motion, and `None` only when idle (no turn in progress).
    #[must_use]
    pub fn tool_spinner_glyph(&self) -> Option<&'static str> {
        let start = self.spinner_start?;
        if !self.animations_enabled() {
            return Some(crate::glyphs::SPINNER_ROTATE[0]);
        }
        Some(crate::glyphs::frame_at(
            &crate::glyphs::SPINNER_ROTATE,
            start.elapsed().as_millis(),
            ANIMATION_TICK_MS as u128,
        ))
    }

    /// The elapsed whole seconds since the current turn began, for the working band's live timer.
    /// Zero when no turn is in progress.
    #[must_use]
    pub fn turn_elapsed_secs(&self) -> u64 {
        self.spinner_start
            .map_or(0, |start| start.elapsed().as_secs())
    }

    /// A short, dynamic detail label for the working band (`Working · <detail>`), reflecting the
    /// current turn phase: the running tool when one is in flight, otherwise that the model is
    /// thinking.
    #[must_use]
    pub fn working_detail(&self) -> String {
        let running_tool = self.transcript.iter().rev().find_map(|entry| match entry {
            Entry::Tool(card) if card.status == crate::tool_card::ToolStatus::Running => {
                Some(card.name.clone())
            }
            _ => None,
        });
        match running_tool {
            Some(name) => format!("running {name}"),
            None => "thinking".to_owned(),
        }
    }

    /// Whether animations are enabled (the inverse of reduced-motion). Every time-based animation
    /// gates on this: when it is false the UI renders its final, static frame — a fixed spinner
    /// mark, full streamed text with no token-by-token reveal, a solid (non-blinking) caret, and no
    /// toast or selection-highlight motion.
    #[must_use]
    pub fn animations_enabled(&self) -> bool {
        !self.reduced_motion
    }

    /// Whether reduced-motion is active (animations suppressed).
    #[must_use]
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    /// Set reduced-motion at launch from the `--no-anim` flag / `CORTEX_NO_ANIM` preference.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    /// Whether a time-based animation is currently running and therefore needs the ~100ms wake
    /// cadence to advance its phase. This is the wake predicate for animation: it is always false
    /// under reduced motion, so the loop never wakes merely to animate when motion is reduced.
    #[must_use]
    pub fn animation_active(&self) -> bool {
        self.animations_enabled()
            && (self.is_busy() || self.has_toasts() || self.selection_transition_active())
    }

    /// Record a picker selection move for the highlight transition. When animations are enabled and
    /// the move was a single adjacent step (not a wrap-around or multi-row jump) it starts a brief
    /// cross-fade from `from` to `to`; otherwise the highlight jumps instantly (state cleared).
    fn note_selection_move(&mut self, from: usize, to: usize) {
        if self.animations_enabled() && from != to && from.abs_diff(to) == 1 {
            self.selection_anim = Some(SelectionTransition {
                from,
                to,
                start: Instant::now(),
            });
        } else {
            self.selection_anim = None;
        }
    }

    /// Whether a selection-highlight transition is currently playing (and therefore needs the fast
    /// wake cadence). Always false under reduced motion and once the transition has elapsed.
    #[must_use]
    pub fn selection_transition_active(&self) -> bool {
        self.animations_enabled()
            && self
                .selection_anim
                .as_ref()
                .is_some_and(|t| t.start.elapsed().as_millis() < SELECTION_TRANSITION_MS)
    }

    /// The resolved selection transition for the current frame, or `None` when the highlight is at
    /// rest (or under reduced motion), in which case the selection renders instantly on its row.
    #[must_use]
    pub fn selection_anim_state(&self) -> Option<crate::modal::SelectionAnim> {
        if !self.animations_enabled() {
            return None;
        }
        let t = self.selection_anim.as_ref()?;
        let elapsed = t.start.elapsed().as_millis();
        if elapsed >= SELECTION_TRANSITION_MS {
            return None;
        }
        let phase = (elapsed as f32 / SELECTION_TRANSITION_MS as f32).clamp(0.0, 1.0);
        Some(crate::modal::SelectionAnim {
            from: t.from,
            to: t.to,
            phase,
        })
    }

    /// The event-loop poll timeout for the current state. The loop polls at the ~100ms animation
    /// cadence when [`animation_active`](App::animation_active) requests a wakeup to advance an
    /// animation phase, or when a turn is streaming / a toast is on screen so streamed deltas and
    /// the auto-dismiss timer are applied promptly even under reduced motion (where
    /// `animation_active` is false), or while a background models.dev refresh is in flight
    /// ([`models_refreshing`](App::models_refreshing)) so the off-thread catalogue swap and the
    /// "refreshing…" indicator clear land promptly rather than after a full idle tick (mirroring how
    /// an in-flight turn drives the cadence); otherwise it blocks for the cheap idle cadence so an
    /// idle TUI never busy-spins. The refresh is a bounded background task that clears its own signal
    /// once [`poll_models_refresh`](App::poll_models_refresh) drains the result, so the cadence
    /// returns to idle with no busy-spin. Routing the wake predicate through `animation_active` keeps
    /// the loop waking for any idle-only animation that later extends that predicate, without
    /// hardcoding a second timeout here.
    #[must_use]
    pub fn poll_timeout(&self) -> Duration {
        if self.animation_active()
            || self.is_busy()
            || self.has_toasts()
            || self.models_refreshing()
        {
            Duration::from_millis(ANIMATION_TICK_MS)
        } else {
            Duration::from_millis(IDLE_POLL_MS)
        }
    }

    /// Whether the streaming caret should be drawn at this instant (blink state). The caret is
    /// solid (always visible) when idle or under reduced motion, and blinks on a fixed cadence
    /// derived from elapsed time only while a turn streams with animations enabled.
    #[must_use]
    pub fn cursor_blink_visible(&self) -> bool {
        if !self.animations_enabled() {
            return true;
        }
        match self.spinner_start {
            Some(start) => caret_visible_at(start.elapsed().as_millis()),
            None => true,
        }
    }

    /// A snapshot of the conversational history sent with the next turn.
    #[must_use]
    pub fn history_snapshot(&self) -> Vec<Message> {
        self.history.clone()
    }

    /// Attachments currently staged in the prompt composer.
    #[must_use]
    pub fn composer_attachments(&self) -> &[InputAttachment] {
        &self.composer_attachments
    }

    /// Take the prompt accepted by `submit` (if any), clearing it so a turn starts exactly once.
    #[must_use]
    pub fn take_pending_prompt(&mut self) -> Option<String> {
        self.pending_prompt.take()
    }

    /// The queued turn's non-text media parts as an LLM request payload.
    #[must_use]
    pub fn pending_user_parts_for_turn(&self) -> Vec<MessageContentPart> {
        attachments_to_message_content_parts(&self.pending_attachments)
    }

    /// Build the attached-server bridge for the next local TUI turn.
    ///
    /// This ensures a durable session id exists before the worker starts streaming so every event
    /// posted to the server carries the same session id the transcript and database use.
    #[must_use]
    pub fn remote_bridge_for_turn(&mut self) -> Option<crate::turn::RemoteBridge> {
        let server = self.remote_server.clone()?;
        let session_id = self.ensure_session_id()?;
        Some(crate::turn::RemoteBridge::new(server, session_id))
    }

    /// Resolve the durable session id and database path a turn's tools should use.
    ///
    /// This is called before the worker thread starts so the `todo` tool is keyed to the same
    /// session that messages are persisted under. When persistence is disabled or unavailable, the
    /// worker falls back to its ephemeral in-memory store.
    #[must_use]
    pub fn tool_store_context_for_turn(&mut self) -> (Option<String>, Option<PathBuf>) {
        let session_id = self.ensure_session_id();
        let db_path = session_id.as_ref().and(self.db_path.clone());
        (session_id, db_path)
    }

    /// Begin a streaming turn for `prompt`: record the user message, open an in-progress assistant
    /// entry, attach the worker's event channel, and start the busy indicator.
    ///
    /// A slash flow may set a concise [header](App::pending_header) (shown in the transcript) and
    /// the [compaction](App::pending_compaction) flag (so the turn's summary replaces history when
    /// it finishes); both are consumed here. A plain chat prompt shows its text verbatim.
    ///
    /// `cancel` is the worker's interrupt seam, retained so [`interrupt_turn`](App::interrupt_turn)
    /// can stop the stream when the user presses the cancel key.
    pub fn begin_turn(
        &mut self,
        prompt: String,
        rx: Receiver<TurnMessage>,
        cancel: Arc<AtomicBool>,
    ) {
        let header = self.pending_header.take().unwrap_or_else(|| prompt.clone());
        let attachments = std::mem::take(&mut self.pending_attachments);
        self.transcript
            .push(Entry::User(redaction::redact(&render_user_entry(
                &header,
                &attachments,
            ))));
        self.transcript
            .push(Entry::Assistant(AssistantTurn::default()));
        self.turn_errored = false;
        self.turn_usage = None;
        self.turn_compaction = std::mem::take(&mut self.pending_compaction);
        self.turn_user_persisted = false;
        self.answer_redactor.reset();
        self.reasoning_redactor.reset();
        // Persist the user message to the durable store so this conversation is resumable from
        // headless `cortex run --session <id>` and the session picker. A compaction turn's internal
        // summarization instruction is not part of the visible conversation, so it is not persisted
        // (the full pre-compaction transcript stays the durable record). Attachment-bearing prompts
        // are persisted at turn finalization so failed turns can drop unsupported media safely.
        if !self.turn_compaction && attachments.is_empty() {
            let user_parts = user_message_parts(&prompt, &attachments);
            self.persist_message("user", &prompt, &user_parts);
            self.turn_user_persisted = true;
        }
        self.turn_prompt = Some(prompt);
        self.turn_attachments = attachments;
        self.turn_rx = Some(rx);
        self.cancel_flag = Some(cancel);
        self.spinner_start = Some(Instant::now());
        // Pin to the latest so the streaming reply stays in view.
        self.pin_to_bottom();
    }

    /// The number of chat messages queued behind the active turn (submitted mid-stream and waiting
    /// to run once it settles). Surfaced in the busy indicator so a queued message reads as accepted.
    #[must_use]
    pub fn queued_count(&self) -> usize {
        self.queued_prompts.len()
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

    /// Drain live MCP catalogue events and refresh the `/mcp` view if it is open. Returns whether
    /// any event was applied, so the event loop knows a redraw may reveal a changed tool set.
    pub fn poll_mcp(&mut self) -> bool {
        let mut changed_servers = Vec::new();
        if let Some(rx) = self.mcp_events.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(events::Event::McpToolsChanged(payload)) => {
                        changed_servers.push(payload.server);
                    }
                    Ok(events::Event::McpResourcesChanged(payload)) => {
                        changed_servers.push(payload.server);
                    }
                    Ok(events::Event::McpPromptsChanged(payload)) => {
                        changed_servers.push(payload.server);
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
        }

        if changed_servers.is_empty() {
            return false;
        }

        changed_servers.sort();
        changed_servers.dedup();
        // The full-screen `/mcp` view reads live server rows at render time, so no cached-row
        // refresh is needed here; just surface the catalogue-change toasts.
        for server in changed_servers {
            self.push_toast(ToastLevel::Info, format!("MCP catalogue updated: {server}"));
        }
        true
    }

    /// Tail every backgrounded `bash` card's on-disk log into its body, and transition it to
    /// [`ToolStatus`](crate::tool_card::ToolStatus)`::Completed` once its process id has exited
    /// (VAL-TOOL-023): a background job keeps live-streaming its real output instead of freezing on
    /// the static "job started" acknowledgement. A job's pid goes away either because the command
    /// finished naturally or because [`crate::turn`] reaped it by pid at the end of the turn that
    /// spawned it (`background_jobs().shutdown()`); either way the completion transition observes the
    /// job's real exit via [`ToolCard::bg_process_alive`], which compares the captured `/proc/<pid>`
    /// start time so a *reused* pid (freed then re-taken by an unrelated process) never keeps the
    /// card polling a live-but-unrelated pid. Returns whether any card changed — the event loop
    /// already redraws unconditionally each iteration, so this is informational only.
    pub fn poll_background_jobs(&mut self) -> bool {
        let mut changed = false;
        for entry in &mut self.transcript {
            let Entry::Tool(card) = entry else { continue };
            if !card.is_background_running() {
                continue;
            }
            let (Some(log_path), Some(_pid)) = (card.bg_log.clone(), card.bg_pid) else {
                continue;
            };
            let tailed = redaction::redact(&tail_background_log(&log_path));
            if card.bg_process_alive() {
                if card.output != tailed {
                    card.output = tailed;
                    changed = true;
                }
            } else {
                card.complete_background(tailed);
                changed = true;
            }
        }
        changed
    }

    /// Apply server-created messages for the attached session from the shared database.
    ///
    /// The server persists user messages when a remote turn starts and assistant messages when it
    /// completes. Polling the shared store makes those changes appear in the attached TUI without
    /// reopening `/sessions` or manually reloading the transcript.
    pub fn poll_remote_session(&mut self) -> bool {
        if self.remote_server.is_none() || self.is_busy() {
            return false;
        }
        if self
            .last_remote_sync
            .is_some_and(|last| last.elapsed() < Duration::from_millis(250))
        {
            return false;
        }
        self.last_remote_sync = Some(Instant::now());
        let Some(id) = self.session_id.clone() else {
            return false;
        };
        let loaded = match self.load_persisted_session(&id) {
            Ok(Some(loaded)) => loaded,
            _ => return false,
        };
        let previous_usage = self.usage;
        self.usage = UsageSummary::from_totals(loaded.usage);
        let mut changed = false;
        for message in loaded.messages {
            if self.seen_message_ids.insert(message.id.clone()) {
                self.append_loaded_message(&message);
                changed = true;
            }
        }
        if self.usage != previous_usage {
            changed = true;
        }
        if changed {
            self.pin_to_bottom();
        }
        changed
    }

    /// Observe a server-side abort marker for the active remote session and interrupt the local
    /// TUI turn promptly. The marker is removed after being consumed so one abort affects one turn.
    pub fn poll_remote_abort(&mut self) -> bool {
        if self.remote_server.is_none() {
            return false;
        }
        let Some(id) = self.session_id.as_deref() else {
            return false;
        };
        let marker = remote_abort_path(id);
        if !marker.is_file() {
            return false;
        }
        if !self.is_busy() {
            let _ = std::fs::remove_file(marker);
            return false;
        }
        let _ = std::fs::remove_file(marker);
        self.interrupt_turn();
        true
    }

    /// Poll the attached server for queued TUI control requests and apply them locally.
    ///
    /// Each request is applied to this TUI (append/clear/submit the prompt, open the model picker,
    /// show a toast, run a command, switch sessions) and its success/error outcome is reported back
    /// so the client that issued the control sees whether the attached TUI honored it. Several
    /// queued requests are drained per poll so a burst of controls applies promptly.
    pub fn poll_remote_control(&mut self) -> bool {
        let Some(server) = self.remote_server.clone() else {
            return false;
        };
        if self
            .last_remote_control
            .is_some_and(|last| last.elapsed() < Duration::from_millis(120))
        {
            return false;
        }
        self.last_remote_control = Some(Instant::now());

        let mut applied = false;
        for _ in 0..8 {
            match server.next_control() {
                Ok(Some(control)) => {
                    let outcome = self.apply_remote_control(&control.action, &control.body);
                    let (success, error) = match outcome {
                        Ok(()) => (true, None),
                        Err(message) => (false, Some(message)),
                    };
                    let _ = server.respond_control(&control.id, success, error.as_deref());
                    applied = true;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        applied
    }

    /// Apply a single server-issued control request to this TUI, returning an error message when the
    /// control cannot be honored (unknown action or missing required field).
    fn apply_remote_control(
        &mut self,
        action: &str,
        body: &serde_json::Value,
    ) -> Result<(), String> {
        match action {
            "append-prompt" => {
                let text = body
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| "append-prompt requires text".to_owned())?;
                let current = self.input.value();
                self.input.set_value(&format!("{current}{text}"));
                Ok(())
            }
            "clear-prompt" => {
                self.input.set_value("");
                self.composer_attachments.clear();
                Ok(())
            }
            "submit-prompt" => {
                self.submit();
                Ok(())
            }
            "open-models" => {
                self.open_models_dialog();
                Ok(())
            }
            "execute-command" => {
                let command = body
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .map(|command| command.trim().trim_start_matches('/'))
                    .filter(|command| !command.is_empty())
                    .ok_or_else(|| "execute-command requires a command".to_owned())?;
                self.dispatch_builtin_command(command, "remote command");
                Ok(())
            }
            "show-toast" => {
                let message = body
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .filter(|message| !message.trim().is_empty())
                    .ok_or_else(|| "show-toast requires a message".to_owned())?;
                let level = match body.get("variant").and_then(serde_json::Value::as_str) {
                    Some("success") => ToastLevel::Success,
                    Some("warning") | Some("warn") => ToastLevel::Warning,
                    Some("error") => ToastLevel::Error,
                    _ => ToastLevel::Info,
                };
                self.push_toast(level, message.to_owned());
                Ok(())
            }
            "select-session" => {
                let session_id = body
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| "select-session requires a session_id".to_owned())?;
                match self.load_persisted_session(session_id) {
                    Ok(Some(_)) => {
                        self.load_session(session_id);
                        Ok(())
                    }
                    Ok(None) => Err(format!("session {session_id} does not exist")),
                    Err(err) => Err(format!("could not load session {session_id}: {err}")),
                }
            }
            other => Err(format!("unknown TUI control action: {other}")),
        }
    }

    /// Apply a single turn message to the application state.
    fn apply(&mut self, message: TurnMessage) {
        match message {
            TurnMessage::Event(event) => self.apply_event(event),
            TurnMessage::FileDiff(diff) => self.push_diff(*diff),
            TurnMessage::Permission(ask) => self.open_permission(*ask),
            TurnMessage::SetupError(message) => self.set_turn_error(message),
            TurnMessage::Finished => self.finish_turn(),
        }
    }

    /// Record a file-change diff in the transcript.
    ///
    /// A file mutation surfaces as a `● Update <path>` tool card just before its diff arrives, so
    /// the diff is attached to that card (the most recent mutating card without one yet), turning it
    /// into a single expandable diff card (`▶`/`▼` disclosure + boxed red/green body) rather than a
    /// second, detached block. When no such card exists (e.g. a diff pushed without a tool call) the
    /// diff falls back to a standalone entry, dropping a trailing empty in-progress assistant entry
    /// first so it is not preceded by an empty assistant block.
    fn push_diff(&mut self, diff: crate::diff::FileDiff) {
        if let Some(card) = self
            .transcript
            .iter_mut()
            .rev()
            .find_map(|entry| match entry {
                Entry::Tool(card) if card.diff.is_none() && is_mutating_tool(&card.name) => {
                    Some(card)
                }
                _ => None,
            })
        {
            card.attach_diff(diff);
            return;
        }
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
            RunnerEvent::Reasoning { text, .. } => self.append_reasoning(&text),
            RunnerEvent::Usage(usage) => {
                self.turn_usage = Some(usage.clone());
                self.usage.accumulate(&usage);
            }
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
            RunnerEvent::ToolSetChanged { .. } => {}
            RunnerEvent::Retry { .. } => {}
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
    ///
    /// A successful backgrounded `bash` result (VAL-TOOL-023: the call's args carry
    /// `background: true`, tracked on the card as [`ToolCard::is_background`]) is the job-started
    /// acknowledgement, not a finished result — the card is instead handed its log path and pid via
    /// [`ToolCard::start_background`] and stays [`ToolStatus::Running`], so
    /// [`App::poll_background_jobs`] can tail the live log until the job's process actually exits.
    /// Gating on the structured `is_background` flag (not the truncatable `(background)` summary
    /// marker) keeps this correct for arbitrarily long commands and immune to a foreground command
    /// whose text merely mentions `(background)`. A result whose text doesn't carry a parseable
    /// `pid:`/`output:` pair (unexpected shape) falls back to completing normally, so the card never
    /// hangs running with nothing to poll.
    fn complete_tool_call(&mut self, id: &str, parts: &[protocol::ToolResultPart], is_error: bool) {
        let mut lsp_candidate: Option<(String, String, String)> = None;
        if let Some(card) = self
            .transcript
            .iter_mut()
            .rev()
            .find_map(|entry| match entry {
                Entry::Tool(card) if card.id == id => Some(card),
                _ => None,
            })
        {
            let is_background_bash = !is_error && card.is_background;
            let bg_job = is_background_bash
                .then(|| crate::tool_card::text_from_parts(parts))
                .and_then(|text| crate::tool_card::parse_background_job(&text));
            match bg_job {
                Some((pid, log_path)) => card.start_background(pid, log_path),
                None => card.complete(parts, is_error),
            }
            // The card summary is the tool's target (the path for write/edit, the first patch file
            // for apply_patch), used to attribute headerless `Diagnostic::render()` rows.
            lsp_candidate = Some((card.name.clone(), card.summary.clone(), card.output.clone()));
        }
        if let Some((name, target, output)) = lsp_candidate {
            self.capture_lsp_sidebar_note(&name, Some(&target), &output);
        }
    }

    /// Parse the diagnostics carried by a completed `edit`/`write`/`apply_patch`/`lsp`/`bash` tool
    /// result into structured per-file groups for the sidebar `lsp` panel.
    ///
    /// Real runtime tool cards emit `tools::Diagnostic::render()` lines
    /// (`error [E0308] (rustc) at line 5, character 10-15: <msg>`), NOT the dead
    /// `render_feedback_block` text — see `library/architecture-notes.md` for the cross-crate
    /// coupling gotcha. Three shapes are recognized so the panel populates from real output:
    /// - the mutating-tool feedback shape (`<summary> in <path>` + `Diagnostics:` + bare
    ///   `Diagnostic::render()` rows) — the rows carry no per-file header, so they are attributed to
    ///   `target_hint`, the file the tool just wrote;
    /// - the `cortex lsp diagnostics` CLI text (`lsp diagnostics for <path>\tstatus=<s>` + rows, or
    ///   `diagnostics=0` when clean);
    /// - the legacy `render_feedback_block` shape (`lsp diagnostics for <path>: <E> error(s),
    ///   <W> warning(s)` + `- <severity> ... at line <L>: <msg>`).
    ///
    /// Each file's group is upserted (a newer report replaces an older one) so the panel tracks the
    /// latest state, capped to the most recent files.
    pub(crate) fn capture_lsp_sidebar_note(
        &mut self,
        tool_name: &str,
        target_hint: Option<&str>,
        output: &str,
    ) {
        let name = tool_name.to_ascii_lowercase();
        let output_lower = output.to_ascii_lowercase();
        let looks_like_lsp =
            name == "lsp" || output_lower.contains("lsp") || output_lower.contains("diagnostic");
        if !looks_like_lsp {
            return;
        }
        // The basename of the mutating tool's target, used to attribute bare `Diagnostic::render()`
        // rows (which carry no per-file header) to the file the tool just wrote.
        let hint_file = target_hint.and_then(basename_of);
        let mut current: Option<usize> = None;
        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if let Some((file, errors, warnings)) = parse_lsp_summary(line) {
                current = Some(self.upsert_lsp_group(file, errors, warnings));
            } else if let Some(file) = parse_lsp_cli_header(line) {
                current = Some(self.upsert_lsp_group(file, 0, 0));
            } else if let Some(diagnostic) = parse_lsp_detail(line) {
                let index = match current {
                    Some(index) => index,
                    None => {
                        // Bare render rows with no header (mutating-tool feedback): attribute them
                        // to the tool's target file so the group still forms.
                        let Some(file) = hint_file.clone() else {
                            continue;
                        };
                        let index = self.upsert_lsp_group(file, 0, 0);
                        current = Some(index);
                        index
                    }
                };
                self.sidebar_lsp_groups[index].diagnostics.push(diagnostic);
            }
        }
        const MAX_LSP_FILES: usize = 8;
        if self.sidebar_lsp_groups.len() > MAX_LSP_FILES {
            let keep_from = self.sidebar_lsp_groups.len() - MAX_LSP_FILES;
            self.sidebar_lsp_groups.drain(0..keep_from);
        }
    }

    /// Replace any prior group for `file` with a fresh one (latest report wins) and return its
    /// index. Detail rows appended afterwards drive the worst-severity dot and E/W counts even when
    /// `errors`/`warnings` are `0` (the bare `Diagnostic::render()` shape has no summary counts).
    fn upsert_lsp_group(&mut self, file: String, errors: usize, warnings: usize) -> usize {
        self.sidebar_lsp_groups.retain(|group| group.file != file);
        self.sidebar_lsp_groups.push(LspFileGroup {
            file,
            errors,
            warnings,
            diagnostics: Vec::new(),
        });
        self.sidebar_lsp_groups.len() - 1
    }

    /// Append a streamed answer fragment to the in-progress assistant entry.
    fn append_answer(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = self.answer_redactor.push(text);
        self.push_answer_text(text);
    }

    /// Append already-redacted answer text to the in-progress assistant entry.
    fn push_answer_text(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        match self.transcript.last_mut() {
            Some(Entry::Assistant(turn)) => turn.answer.push_str(&text),
            _ => self.transcript.push(Entry::Assistant(AssistantTurn {
                reasoning: String::new(),
                answer: text,
            })),
        }
    }

    /// Append a streamed reasoning fragment to the in-progress assistant entry's reasoning region.
    fn append_reasoning(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = self.reasoning_redactor.push(text);
        self.push_reasoning_text(text);
    }

    /// Append already-redacted reasoning text to the in-progress assistant entry.
    fn push_reasoning_text(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        match self.transcript.last_mut() {
            Some(Entry::Assistant(turn)) => turn.reasoning.push_str(&text),
            _ => self.transcript.push(Entry::Assistant(AssistantTurn {
                reasoning: text,
                answer: String::new(),
            })),
        }
    }

    /// Flush any safe text held back for split-secret detection at the active turn boundary.
    fn flush_stream_redactors(&mut self) {
        let reasoning = self.reasoning_redactor.finish();
        self.push_reasoning_text(reasoning);
        let answer = self.answer_redactor.finish();
        self.push_answer_text(answer);
    }

    /// Surface a turn error: drop an empty in-progress assistant entry, then record the error as an
    /// in-conversation `✦ Cortex` / `✗ error` block (per `err_inline.png`). The failure is NEVER
    /// raised as a toast/notification — it lives only in the transcript so it reads as the
    /// assistant's response to the turn, matching the reference mockup.
    fn set_turn_error(&mut self, message: String) {
        self.flush_stream_redactors();
        let message = redaction::redact(&message);
        if let Some(Entry::Assistant(turn)) = self.transcript.last() {
            if turn.answer.is_empty() && turn.reasoning.is_empty() {
                self.transcript.pop();
            }
        }
        self.transcript.push(Entry::TurnError(message));
        self.turn_errored = true;
        self.emit_attention(AttentionEvent::Error);
    }

    /// Finalize a completed turn: clear the busy indicator and fold the exchange into history so the
    /// next turn carries conversational context.
    ///
    /// A compaction turn (`/compact`) instead replaces the conversation history with its summary so
    /// subsequent turns carry a compact context, and annotates the transcript so the compaction is
    /// visible. A failed turn of either kind leaves history untouched.
    ///
    /// On the way out it promotes the next queued mid-stream message (if any) into its own turn, so
    /// a message submitted while this turn streamed runs next rather than being dropped.
    fn finish_turn(&mut self) {
        self.flush_stream_redactors();
        self.turn_rx = None;
        self.cancel_flag = None;
        self.spinner_start = None;
        let prompt = self.turn_prompt.take();
        let turn_attachments = std::mem::take(&mut self.turn_attachments);
        let errored = std::mem::take(&mut self.turn_errored);
        let turn_usage = self.turn_usage.take();
        let compaction = std::mem::take(&mut self.turn_compaction);
        let user_persisted = std::mem::take(&mut self.turn_user_persisted);

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
                if let Some(usage) = turn_usage.as_ref() {
                    self.persist_usage(usage);
                }
                self.emit_attention(AttentionEvent::Completion);
            }
            self.start_next_queued();
            return;
        }

        if let Some(prompt) = prompt {
            let history_media_parts = if errored {
                Vec::new()
            } else {
                attachments_to_message_content_parts(&turn_attachments)
            };
            self.history
                .push(Message::user(prompt.clone()).with_parts(history_media_parts));
            if !user_persisted {
                let persisted_parts = if errored {
                    user_message_parts(&prompt, &[])
                } else {
                    user_message_parts(&prompt, &turn_attachments)
                };
                self.persist_message("user", &prompt, &persisted_parts);
            }
            if !errored {
                if let Some(answer) = self.latest_assistant_answer() {
                    self.history.push(Message::assistant(answer.clone()));
                    // Persist the assistant reply so a resumed session (headless or picker) sees the
                    // full exchange.
                    self.persist_message("assistant", &answer, &[]);
                }
                if let Some(usage) = turn_usage.as_ref() {
                    self.persist_usage(usage);
                }
                self.emit_attention(AttentionEvent::Completion);
            }
        }
        self.start_next_queued();
    }

    /// Promote the oldest queued mid-stream message (if any) into the next turn by staging it as the
    /// pending prompt the event loop starts. A no-op while a turn is in flight or the queue is empty.
    fn start_next_queued(&mut self) {
        if self.is_busy() {
            return;
        }
        if let Some(item) = self.queued_prompts.pop_front() {
            self.pending_header = item.header;
            self.pending_prompt = Some(item.prompt);
            self.pending_attachments = item.attachments;
        }
    }

    /// Interrupt the streaming turn: signal the worker to stop, return the UI to an idle, editable
    /// state, and preserve any partial output already rendered.
    ///
    /// The in-flight worker is asked to cancel (dropping the upstream stream at a safe boundary); the
    /// partial exchange is folded into history so a follow-up keeps context, an "interrupted" notice
    /// and toast are surfaced, and any messages queued behind this turn are cleared so the UI truly
    /// returns to idle rather than immediately launching another turn. A no-op when no turn is in
    /// flight.
    fn interrupt_turn(&mut self) {
        if !self.is_busy() {
            return;
        }
        if let Some(flag) = self.cancel_flag.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.turn_rx = None;
        self.spinner_start = None;
        self.flush_stream_redactors();
        let prompt = self.turn_prompt.take();
        let turn_attachments = std::mem::take(&mut self.turn_attachments);
        let errored = std::mem::take(&mut self.turn_errored);
        let _ = self.turn_usage.take();
        let _ = std::mem::take(&mut self.turn_compaction);
        let user_persisted = std::mem::take(&mut self.turn_user_persisted);
        if let Some(prompt) = prompt {
            let history_media_parts = if errored {
                Vec::new()
            } else {
                attachments_to_message_content_parts(&turn_attachments)
            };
            self.history
                .push(Message::user(prompt.clone()).with_parts(history_media_parts));
            if !user_persisted {
                let persisted_parts = if errored {
                    user_message_parts(&prompt, &[])
                } else {
                    user_message_parts(&prompt, &turn_attachments)
                };
                self.persist_message("user", &prompt, &persisted_parts);
            }
            if !errored {
                if let Some(answer) = self.latest_assistant_answer() {
                    self.history.push(Message::assistant(answer));
                }
            }
        }
        self.queued_prompts.clear();
        self.transcript
            .push(Entry::Notice("Turn interrupted.".to_owned()));
        self.push_toast(ToastLevel::Info, "Turn interrupted");
        self.pin_to_bottom();
    }

    /// Whether an interactive permission prompt is currently awaiting a decision. While open it is
    /// modal (it captures keys) and the turn worker is blocked awaiting the reply.
    #[must_use]
    pub fn permission_open(&self) -> bool {
        self.pending_permission.is_some()
    }

    fn permission_is_question(&self) -> bool {
        self.pending_permission
            .as_ref()
            .is_some_and(|pending| pending.ask.tool == "question")
    }

    /// The number of selectable options in the permission prompt: Allow once, Allow always, Deny.
    const PERMISSION_OPTIONS: usize = 3;

    /// The open permission prompt's `(tool, subject, selected, feedback)` for rendering, or `None`
    /// when no ask is pending (`selected`: 0 = Allow once, 1 = Allow always, 2 = Deny).
    #[must_use]
    pub fn permission_prompt(&self) -> Option<(&str, &str, usize, &str)> {
        self.pending_permission.as_ref().map(|p| {
            (
                p.ask.tool.as_str(),
                p.ask.subject.as_str(),
                p.selected,
                p.feedback.as_str(),
            )
        })
    }

    /// Surface an ask-gated tool's permission request as a modal prompt, defaulting the highlight to
    /// Allow once. The worker thread stays blocked until [`answer_permission`](App::answer_permission)
    /// replies, so the dialog draws over the (still-busy) turn until the user decides.
    fn open_permission(&mut self, ask: PermissionAsk) {
        self.pending_permission = Some(PendingPermission {
            ask,
            selected: 0,
            feedback: String::new(),
        });
        self.emit_attention(AttentionEvent::Permission);
        self.pin_to_bottom();
    }

    /// Open a permission prompt directly (test-only), used by rendering tests in sibling modules
    /// that cannot reach the private turn-message apply path.
    #[cfg(test)]
    pub(crate) fn open_permission_for_test(&mut self, ask: PermissionAsk) {
        self.open_permission(ask);
    }

    /// Move the permission highlight one step (`forward` = toward Deny), clamped at the ends so the
    /// arrows scan the three options without wrapping.
    fn permission_step(&mut self, forward: bool) {
        if let Some(pending) = self.pending_permission.as_mut() {
            pending.selected = if forward {
                (pending.selected + 1).min(Self::PERMISSION_OPTIONS - 1)
            } else {
                pending.selected.saturating_sub(1)
            };
        }
    }

    /// Confirm the highlighted permission option (Enter), answering with the matching scoped
    /// decision: Allow once, Allow always (remembered), or Deny.
    fn confirm_permission(&mut self) {
        let (selected, feedback, is_question) =
            self.pending_permission
                .as_ref()
                .map_or((0, String::new(), false), |pending| {
                    (
                        pending.selected,
                        pending.feedback.clone(),
                        pending.ask.tool == "question",
                    )
                });
        let feedback = feedback.trim().to_owned();
        self.answer_permission(match selected {
            1 => {
                if is_question && !feedback.is_empty() {
                    ScopedDecision::always(Decision::Allow).with_feedback(feedback)
                } else {
                    ScopedDecision::always(Decision::Allow)
                }
            }
            2 => {
                if feedback.is_empty() {
                    ScopedDecision::once(Decision::Deny)
                } else {
                    ScopedDecision::once(Decision::Deny).with_feedback(feedback)
                }
            }
            _ => {
                if is_question && !feedback.is_empty() {
                    ScopedDecision::once(Decision::Allow).with_feedback(feedback)
                } else {
                    ScopedDecision::once(Decision::Allow)
                }
            }
        });
    }

    /// Append one feedback character while feedback capture is active.
    fn permission_feedback_input(&mut self, ch: char) {
        if let Some(pending) = self.pending_permission.as_mut()
            && (pending.selected == 2 || pending.ask.tool == "question")
        {
            pending.feedback.push(ch);
        }
    }

    /// Remove one feedback character while feedback capture is active.
    fn permission_feedback_backspace(&mut self) {
        if let Some(pending) = self.pending_permission.as_mut()
            && (pending.selected == 2 || pending.ask.tool == "question")
        {
            pending.feedback.pop();
        }
    }

    /// Answer the pending permission prompt, unblocking the turn worker and recording the outcome.
    ///
    /// An approval lets the gated tool proceed (its effect/result then renders as the turn
    /// continues); an "always allow" additionally remembers the decision (persisted after the turn,
    /// so a later run — including a headless `cortex run` — honors it without re-asking); a denial
    /// blocks the tool (it returns a permission-denied result the model sees) while the turn carries
    /// on. Either way a notice + toast records the decision and the dialog is dismissed so the UI
    /// returns to its streaming state.
    fn answer_permission(&mut self, decision: ScopedDecision) {
        let Some(pending) = self.pending_permission.take() else {
            return;
        };
        let tool = pending.ask.tool.clone();
        let subject = pending.ask.subject.clone();
        let feedback = decision
            .feedback
            .as_deref()
            .map(redaction::redact)
            .filter(|text| !text.is_empty());
        let outcome_decision = decision.decision;
        let scope = decision.scope;
        pending.ask.answer(decision);
        let (label, level) = match (outcome_decision, scope, feedback) {
            (Decision::Allow, permissions::DecisionScope::Always, _) => (
                format!("Always allowed {tool}: {subject}"),
                ToastLevel::Success,
            ),
            (Decision::Allow, _, _) => (format!("Approved {tool}: {subject}"), ToastLevel::Success),
            (Decision::Deny, _, Some(feedback)) => (
                format!("Denied {tool}: {subject} (feedback: {feedback})"),
                ToastLevel::Warning,
            ),
            (Decision::Deny, _, None) => (format!("Denied {tool}: {subject}"), ToastLevel::Warning),
        };
        self.push_toast(level, label.clone());
        self.transcript.push(Entry::Notice(label));
        self.pin_to_bottom();
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

    fn slash_catalogue(&self) -> Vec<SlashMatch> {
        let mut commands = Vec::new();
        let mut seen = BTreeSet::new();

        for builtin in slash::BUILTINS {
            let key = builtin.name.to_ascii_lowercase();
            if seen.insert(key) {
                commands.push(SlashMatch {
                    name: builtin.name.to_owned(),
                    description: builtin.description.to_owned(),
                    action: SlashAction::Builtin,
                });
            }
        }

        for custom in &self.custom_commands {
            let key = custom.name.to_ascii_lowercase();
            if seen.insert(key) {
                commands.push(SlashMatch {
                    name: custom.name.clone(),
                    description: custom.description.clone(),
                    action: SlashAction::CustomPrompt {
                        prompt: custom.prompt.clone(),
                    },
                });
            }
        }

        if let Some(hooks) = self.plugin_text_complete.as_ref() {
            let mut plugin_commands = hooks.command_entries();
            plugin_commands.sort_by(|lhs, rhs| lhs.id.cmp(&rhs.id));
            for entry in plugin_commands {
                let raw_name = if entry.name.trim().is_empty() {
                    entry.id.clone()
                } else {
                    entry.name.clone()
                };
                let Some(name) = normalize_custom_command_name(&raw_name) else {
                    continue;
                };
                let key = name.to_ascii_lowercase();
                if seen.insert(key) {
                    let description = if entry.description.trim().is_empty() {
                        format!("Plugin command slot `{}`", entry.id)
                    } else {
                        entry.description
                    };
                    commands.push(SlashMatch {
                        name,
                        description,
                        action: SlashAction::PluginCommand { slot: entry.id },
                    });
                }
            }
        }

        if let Some(manager) = &self.mcp_manager {
            for prompt in manager.prompts() {
                let key = prompt.id.to_ascii_lowercase();
                if seen.insert(key) {
                    let description = prompt.description.unwrap_or_else(|| {
                        format!("Run MCP prompt `{}` from `{}`", prompt.name, prompt.server)
                    });
                    let required = prompt
                        .arguments
                        .iter()
                        .filter(|argument| argument.required)
                        .map(|argument| argument.name.clone())
                        .collect::<Vec<_>>();
                    commands.push(SlashMatch {
                        name: prompt.id.clone(),
                        description,
                        action: SlashAction::McpPrompt {
                            id: prompt.id,
                            required,
                        },
                    });
                }
            }
        }

        commands
    }

    fn filter_slash_matches(&self, query: &str) -> Vec<SlashMatch> {
        let needle = query.to_ascii_lowercase();
        let commands = self.slash_catalogue();
        if needle.is_empty() {
            return commands;
        }

        let mut prefix = Vec::new();
        let mut substring = Vec::new();
        for command in commands {
            let lower = command.name.to_ascii_lowercase();
            if lower.starts_with(&needle) {
                prefix.push(command);
            } else if lower.contains(&needle) {
                substring.push(command);
            }
        }
        prefix.extend(substring);
        prefix
    }

    /// The commands matching the current query, in highlight order. Empty when the menu is closed
    /// or nothing matches.
    #[must_use]
    pub fn slash_matches(&self) -> Vec<SlashMatch> {
        match self.slash_query() {
            Some(query) => self.filter_slash_matches(&query),
            None => Vec::new(),
        }
    }

    /// Every slash command currently discoverable in help: built-ins, config custom commands,
    /// plugin slots, and MCP prompt commands.
    #[must_use]
    pub fn slash_help_entries(&self) -> Vec<(String, String)> {
        self.slash_catalogue()
            .into_iter()
            .map(|command| (command.name, command.description))
            .collect()
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

    /// The total number of workspace files the open picker is choosing from (before filtering), or
    /// `0` when the picker is closed. Paired with [`Self::mention_matches`] it drives the popup's
    /// `shown/total` match-count tag.
    #[must_use]
    pub fn mention_file_total(&self) -> usize {
        self.mention.as_ref().map_or(0, |picker| picker.files.len())
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

    /// Character offset of the prompt cursor from the beginning of the full input.
    fn input_cursor_offset(&self) -> usize {
        let pos = self.input.cursor_position();
        let mut offset = 0usize;
        for row in 0..pos.row {
            if let Some(line) = self.input.line(row) {
                offset = offset.saturating_add(line.chars().count());
            }
            offset = offset.saturating_add(1);
        }
        offset.saturating_add(pos.col)
    }

    /// The active completion span around `cursor` (char offsets): contiguous non-whitespace text
    /// containing the cursor, or an empty insertion span when the cursor sits on whitespace.
    fn completion_span(text: &str, cursor: usize) -> Range<usize> {
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let cursor = cursor.min(len);

        let mut start = cursor;
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }

        let mut end = cursor;
        while end < len && !chars[end].is_whitespace() {
            end += 1;
        }

        start..end
    }

    /// Convert a char offset into a byte offset in `text`, clamping at EOF.
    fn char_to_byte_offset(text: &str, offset: usize) -> usize {
        text.char_indices()
            .nth(offset)
            .map_or(text.len(), |(byte, _)| byte)
    }

    /// Apply a staged completion candidate to `text`.
    ///
    /// When the recorded span still fits the current prompt, replace only that span. If not,
    /// insert at the recorded cursor position to preserve existing text.
    fn apply_pending_text_completion(text: &str, pending: &PendingTextCompletion) -> String {
        let char_len = text.chars().count();
        if pending.span.start <= pending.span.end && pending.span.end <= char_len {
            let start = Self::char_to_byte_offset(text, pending.span.start);
            let end = Self::char_to_byte_offset(text, pending.span.end);
            let mut out = String::with_capacity(
                text.len()
                    .saturating_sub(end.saturating_sub(start))
                    .saturating_add(pending.candidate.len()),
            );
            out.push_str(&text[..start]);
            out.push_str(&pending.candidate);
            out.push_str(&text[end..]);
            return out;
        }

        let insert_at = pending.cursor.min(char_len);
        let insert_byte = Self::char_to_byte_offset(text, insert_at);
        let mut out = String::with_capacity(text.len().saturating_add(pending.candidate.len()));
        out.push_str(&text[..insert_byte]);
        out.push_str(&pending.candidate);
        out.push_str(&text[insert_byte..]);
        out
    }

    /// Drive plugin-backed text completion (`experimental.text.complete`).
    ///
    /// First Tab fetches candidates and surfaces the first one; the next Tab inserts it into the
    /// prompt.
    fn trigger_text_completion(&mut self) {
        if let Some(pending) = self.pending_text_completion.take() {
            let updated = Self::apply_pending_text_completion(&self.input.value(), &pending);
            self.input.set_value(&updated);
            self.on_prompt_edited();
            self.ack = Some("Inserted plugin completion".to_owned());
            return;
        }
        let Some(hooks) = self.plugin_text_complete.as_ref() else {
            return;
        };
        let text = self.input.value();
        if text.trim().is_empty() {
            return;
        }
        let cursor = self.input_cursor_offset();
        let candidates = hooks.text_complete(&text, cursor);
        let Some(first) = candidates.first().cloned() else {
            return;
        };
        self.pending_text_completion = Some(PendingTextCompletion {
            candidate: first.clone(),
            cursor,
            span: Self::completion_span(&text, cursor),
        });
        self.transcript
            .push(Entry::Notice(format!("completion: {first}")));
        self.scroll = 0;
    }

    /// Open the command palette (`Ctrl+P`): a modal, searchable overlay of every built-in command.
    fn open_palette(&mut self) {
        self.open_palette_scope(PaletteScope::Root);
    }

    fn open_palette_scope(&mut self, scope: PaletteScope) {
        let mut palette = Palette::default();
        if scope != PaletteScope::Root {
            palette.scopes.push(scope);
        }
        self.palette = Some(palette);
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

    /// The active palette scope (submenu), or `None` when the palette is closed.
    #[must_use]
    pub fn palette_scope(&self) -> Option<PaletteScope> {
        self.palette
            .as_ref()
            .and_then(|palette| palette.scopes.last().copied())
    }

    /// The rows matching the active palette query/scope, in highlight order.
    #[must_use]
    pub fn palette_matches(&self) -> Vec<PaletteMatch> {
        self.palette_items()
            .into_iter()
            .map(|item| PaletteMatch {
                label: item.label,
                description: item.description,
                disabled_reason: item.disabled_reason,
            })
            .collect()
    }

    /// The highlighted palette row, clamped to the current match count.
    #[must_use]
    pub fn palette_selected(&self) -> usize {
        let len = self.palette_items().len();
        let raw = self.palette.as_ref().map_or(0, |palette| palette.selected);
        if len == 0 { 0 } else { raw.min(len - 1) }
    }

    /// Move the palette highlight to the next match (wrapping at the end).
    fn palette_select_next(&mut self) {
        let len = self.palette_items().len();
        if len == 0 {
            return;
        }
        let from = self.palette_selected();
        let next = (from + 1) % len;
        if let Some(palette) = self.palette.as_mut() {
            palette.selected = next;
        }
        self.note_selection_move(from, next);
    }

    /// Move the palette highlight to the previous match (wrapping at the start).
    fn palette_select_prev(&mut self) {
        let len = self.palette_items().len();
        if len == 0 {
            return;
        }
        let from = self.palette_selected();
        let prev = (from + len - 1) % len;
        if let Some(palette) = self.palette.as_mut() {
            palette.selected = prev;
        }
        self.note_selection_move(from, prev);
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
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        if palette.scopes.len() > 1 {
            palette.scopes.pop();
            palette.query.clear();
            palette.selected = 0;
        } else {
            self.palette = None;
        }
    }

    /// Build a root palette entry for the builtin command `name`, dispatching the real command on
    /// selection. `None` when no such builtin exists.
    fn palette_builtin_item(name: &str) -> Option<PaletteItem> {
        slash::BUILTINS
            .iter()
            .find(|command| command.name == name)
            .map(|command| PaletteItem {
                label: format!("/{}", command.name),
                description: command.description.to_owned(),
                action: PaletteAction::DispatchSlash(command.name.to_owned()),
                disabled_reason: None,
            })
    }

    fn palette_items(&self) -> Vec<PaletteItem> {
        let Some(palette) = self.palette.as_ref() else {
            return Vec::new();
        };
        let scope = palette.scopes.last().copied().unwrap_or(PaletteScope::Root);
        let mut items = Vec::new();
        if scope != PaletteScope::Root {
            items.push(PaletteItem {
                label: "← Back".to_owned(),
                description: "Return to the previous palette menu".to_owned(),
                action: PaletteAction::CloseScope,
                disabled_reason: None,
            });
        }
        match scope {
            PaletteScope::Root => {
                // Lead with the everyday commands in the reference order so the palette opens
                // looking like pick_palette.png, then append the rest of the registry (excluding
                // aliases already represented) so every builtin stays reachable via fuzzy search.
                for name in PALETTE_PRIMARY_COMMANDS {
                    if let Some(item) = Self::palette_builtin_item(name) {
                        items.push(item);
                    }
                }
                for command in slash::BUILTINS {
                    if PALETTE_PRIMARY_COMMANDS.contains(&command.name)
                        || PALETTE_ALIAS_COMMANDS.contains(&command.name)
                    {
                        continue;
                    }
                    if let Some(item) = Self::palette_builtin_item(command.name) {
                        items.push(item);
                    }
                }
                items.push(PaletteItem {
                    label: "Models…".to_owned(),
                    description: "Browse model groups and switch model".to_owned(),
                    action: PaletteAction::OpenScope(PaletteScope::Models),
                    disabled_reason: None,
                });
                items.push(PaletteItem {
                    label: "Skills…".to_owned(),
                    description: "Browse discovered skills".to_owned(),
                    action: PaletteAction::OpenScope(PaletteScope::Skills),
                    disabled_reason: None,
                });
                items.push(PaletteItem {
                    label: "Variants…".to_owned(),
                    description: "Pick the active variant".to_owned(),
                    action: PaletteAction::OpenScope(PaletteScope::Variants),
                    disabled_reason: None,
                });
                items.push(PaletteItem {
                    label: "Subagents…".to_owned(),
                    description: "Switch to a subagent session".to_owned(),
                    action: PaletteAction::OpenScope(PaletteScope::Subagents),
                    disabled_reason: None,
                });
                items.push(PaletteItem {
                    label: "Queued prompts…".to_owned(),
                    description: "Manage queued prompt backlog".to_owned(),
                    action: PaletteAction::OpenScope(PaletteScope::QueuedPrompts),
                    disabled_reason: None,
                });
            }
            PaletteScope::Models => {
                let effective = effective_config();
                let provider_config = effective
                    .as_ref()
                    .and_then(|cfg| providers::ProviderConfigSet::from_table(cfg.table()).ok());
                let merged_catalogue = provider_config.as_ref().map_or_else(
                    || providers::Catalogue::embedded().clone(),
                    |cfg| cfg.merged_catalogue(providers::Catalogue::embedded()),
                );
                for provider in merged_catalogue.providers() {
                    let enabled = provider_config
                        .as_ref()
                        .is_none_or(|cfg| cfg.provider_enabled(&provider.id));
                    items.push(PaletteItem {
                        label: format!("{} ({})", provider.name, provider.id),
                        description: "provider".to_owned(),
                        action: PaletteAction::Noop,
                        disabled_reason: Some("Provider group".to_owned()),
                    });
                    let mut models = provider.models.values().collect::<Vec<_>>();
                    models.sort_by(|lhs, rhs| lhs.id.cmp(&rhs.id));
                    for model in models {
                        let mut tags = Vec::new();
                        if model.id == self.status.model {
                            tags.push("current");
                        }
                        if model.cost.input == 0.0 && model.cost.output == 0.0 {
                            tags.push("free");
                        }
                        if !enabled {
                            tags.push("unavailable");
                        }
                        let desc = if tags.is_empty() {
                            "model".to_owned()
                        } else {
                            format!("model · {}", tags.join(", "))
                        };
                        items.push(PaletteItem {
                            label: format!("  {}", model.id),
                            description: desc,
                            action: PaletteAction::SetModel(model.id.clone()),
                            disabled_reason: (!enabled)
                                .then_some("Provider is disabled by configuration".to_owned()),
                        });
                    }
                }
            }
            PaletteScope::Skills => {
                if let Ok(inputs) =
                    settings::ConfigInputs::from_process(std::collections::BTreeMap::new())
                {
                    let discovery = skills::discover(&inputs);
                    if discovery.skills.is_empty() {
                        items.push(PaletteItem {
                            label: "No skills discovered".to_owned(),
                            description: "Add SKILL.md files under configured skill directories"
                                .to_owned(),
                            action: PaletteAction::Noop,
                            disabled_reason: Some("No discovered skills".to_owned()),
                        });
                    } else {
                        for skill in discovery.ordered_skills() {
                            items.push(PaletteItem {
                                label: skill.name.clone(),
                                description: skill.description.clone(),
                                action: PaletteAction::SelectSkill(skill.name),
                                disabled_reason: None,
                            });
                        }
                    }
                }
            }
            PaletteScope::Variants => {
                for variant in VARIANT_OPTIONS {
                    let mut description = "variant".to_owned();
                    if *variant == self.status.variant {
                        description.push_str(" · current");
                    }
                    items.push(PaletteItem {
                        label: (*variant).to_owned(),
                        description,
                        action: PaletteAction::SetVariant((*variant).to_owned()),
                        disabled_reason: None,
                    });
                }
            }
            PaletteScope::Subagents => {
                let statuses = self.subagent_statuses();
                if statuses.is_empty() {
                    items.push(PaletteItem {
                        label: "No subagent sessions".to_owned(),
                        description: "Spawn a subagent task to populate this list".to_owned(),
                        action: PaletteAction::Noop,
                        disabled_reason: Some("No subagent sessions are available".to_owned()),
                    });
                } else {
                    for (id, label, state, detail) in statuses {
                        items.push(PaletteItem {
                            label,
                            description: format!("{state} · {detail}"),
                            action: PaletteAction::LoadSession(id),
                            disabled_reason: None,
                        });
                    }
                }
            }
            PaletteScope::QueuedPrompts => {
                if self.queued_prompts.is_empty() {
                    items.push(PaletteItem {
                        label: "No queued prompts".to_owned(),
                        description: "Submit while a turn is busy to queue prompts".to_owned(),
                        action: PaletteAction::Noop,
                        disabled_reason: Some("There are no queued prompts".to_owned()),
                    });
                } else {
                    for (idx, queued) in self.queued_prompts.iter().enumerate() {
                        let text = queued
                            .header
                            .as_deref()
                            .unwrap_or(&queued.prompt)
                            .chars()
                            .take(64)
                            .collect::<String>();
                        items.push(PaletteItem {
                            label: format!("{}. {}", idx + 1, text),
                            description: "promote to next".to_owned(),
                            action: PaletteAction::QueuePromote(idx),
                            disabled_reason: None,
                        });
                        items.push(PaletteItem {
                            label: format!("    drop queued #{}", idx + 1),
                            description: "remove this queued prompt".to_owned(),
                            action: PaletteAction::QueueDrop(idx),
                            disabled_reason: None,
                        });
                    }
                    items.push(PaletteItem {
                        label: "Clear queued prompts".to_owned(),
                        description: "drop every queued prompt".to_owned(),
                        action: PaletteAction::QueueClear,
                        disabled_reason: None,
                    });
                }
            }
        }

        let needle = palette.query.trim();
        if needle.is_empty() {
            return items;
        }
        // Subsequence fuzzy match (matching anywhere, per VAL-PICK-061), consistent with the file
        // picker's fuzzy semantics: `mdl` matches `/models`. Match against either the label or the
        // description so a command stays reachable by name or by what it does.
        items
            .into_iter()
            .filter(|item| {
                mention::subsequence_match(&item.label, needle)
                    || mention::subsequence_match(&item.description, needle)
            })
            .collect()
    }

    fn promote_queued_prompt_index(&mut self, index: usize) {
        if index >= self.queued_prompts.len() {
            self.push_toast(ToastLevel::Info, "Queued prompt no longer exists");
            return;
        }
        if let Some(item) = self.queued_prompts.remove(index) {
            self.queued_prompts.push_front(item);
            self.push_toast(ToastLevel::Success, "Queued prompt promoted");
        }
    }

    fn drop_queued_prompt_index(&mut self, index: usize) {
        if index >= self.queued_prompts.len() {
            self.push_toast(ToastLevel::Info, "Queued prompt no longer exists");
            return;
        }
        self.queued_prompts.remove(index);
        self.push_toast(ToastLevel::Success, "Queued prompt dropped");
    }

    /// Edit the `/queue` view's selected prompt (`↵`): pull it back out of the queue into the
    /// composer (its typed text + captured attachments) and close the view so it can be revised and
    /// resubmitted. A no-op when the selection points past the queue.
    fn edit_selected_queued_prompt(&mut self) {
        let index = self.queue_selection;
        if index >= self.queued_prompts.len() {
            return;
        }
        let Some(item) = self.queued_prompts.remove(index) else {
            return;
        };
        let text = item.header.unwrap_or(item.prompt);
        self.composer_attachments = item.attachments;
        self.set_input_value(&text);
        self.close_active_view();
    }

    /// Reorder the `/queue` view's selected prompt by one slot (`Ctrl+Up`/`Ctrl+Down`), keeping the
    /// selection bar on the moved prompt and re-snapshotting the view rows. A no-op at the ends or
    /// with fewer than two queued prompts.
    fn reorder_selected_queued_prompt(&mut self, up: bool) {
        let len = self.queued_prompts.len();
        if len < 2 {
            return;
        }
        let index = self.queue_selection;
        let target = if up {
            if index == 0 {
                return;
            }
            index - 1
        } else {
            if index + 1 >= len {
                return;
            }
            index + 1
        };
        self.queued_prompts.swap(index, target);
        self.queue_selection = target;
        self.queue_snapshot = self.build_queue_rows();
        self.push_toast(ToastLevel::Success, "Queued prompt reordered");
    }

    /// Remove the `/queue` view's selected prompt (`dd`), reusing the shared per-index drop, then
    /// re-snapshot the view rows and clamp the selection so the bar never points past the end.
    fn remove_selected_queued_prompt(&mut self) {
        let index = self.queue_selection;
        if index >= self.queued_prompts.len() {
            return;
        }
        self.drop_queued_prompt_index(index);
        self.queue_snapshot = self.build_queue_rows();
        let len = self.queue_snapshot.len();
        if self.queue_selection >= len {
            self.queue_selection = len.saturating_sub(1);
        }
    }

    fn dispatch_palette_action(&mut self, action: PaletteAction) {
        match action {
            PaletteAction::DispatchSlash(name) => {
                if let Some(command) = slash::BUILTINS.iter().find(|command| command.name == name) {
                    self.dispatch_builtin_command(command.name, command.description);
                }
                self.palette = None;
            }
            PaletteAction::OpenScope(scope) => {
                if let Some(palette) = self.palette.as_mut() {
                    palette.scopes.push(scope);
                    palette.query.clear();
                    palette.selected = 0;
                }
            }
            PaletteAction::CloseScope => self.close_palette(),
            PaletteAction::SetModel(model) => {
                self.status.model = model.clone();
                self.persist_config_string("model", &model);
                self.persist_session_model(&model);
                self.transcript
                    .push(Entry::Notice(format!("Model set to {model}")));
                self.palette = None;
            }
            PaletteAction::SetVariant(variant) => {
                self.status.variant = variant.clone();
                self.persist_config_string("variant", &variant);
                self.transcript
                    .push(Entry::Notice(format!("Variant set to {variant}")));
                self.palette = None;
            }
            PaletteAction::SelectSkill(skill) => {
                self.selected_skill = Some(skill.clone());
                self.transcript
                    .push(Entry::Notice(format!("Skill selected: {skill}")));
                self.palette = None;
            }
            PaletteAction::LoadSession(id) => {
                self.load_session(&id);
                self.palette = None;
            }
            PaletteAction::QueuePromote(index) => self.promote_queued_prompt_index(index),
            PaletteAction::QueueDrop(index) => self.drop_queued_prompt_index(index),
            PaletteAction::QueueClear => self.clear_queued_prompts(),
            PaletteAction::Noop => {}
        }
    }

    /// Run the highlighted palette action (Enter). With no match the palette simply closes.
    fn dispatch_palette(&mut self) {
        let items = self.palette_items();
        if items.is_empty() {
            self.close_palette();
            return;
        }
        let selected = self.palette_selected();
        let Some(item) = items.get(selected) else {
            self.close_palette();
            return;
        };
        if let Some(reason) = item.disabled_reason.clone() {
            self.push_toast(ToastLevel::Info, reason);
            return;
        }
        self.dispatch_palette_action(item.action.clone());
    }

    /// React to a prompt edit: reset the slash highlight and recompute the mention picker.
    fn on_prompt_edited(&mut self) {
        self.slash_selected = 0;
        self.pending_text_completion = None;
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
        let command = matches[self.slash_selected()].clone();
        self.close_slash_menu();
        match command.action {
            SlashAction::Builtin => {
                self.dispatch_builtin_command(&command.name, &command.description);
            }
            SlashAction::CustomPrompt { prompt } => {
                self.start_flow(
                    &format!("/{}", command.name),
                    &format!("/{} — running custom command…", command.name),
                    &prompt,
                );
            }
            SlashAction::McpPrompt { id, required } => {
                self.run_mcp_prompt_command(&command.name, &id, &required);
            }
            SlashAction::PluginCommand { slot } => {
                self.transcript.push(Entry::Notice(format!(
                    "/{} — plugin command slot `{slot}` is available on this TUI surface",
                    command.name
                )));
                self.push_toast(ToastLevel::Info, "Plugin command slot");
            }
        }
    }

    /// Apply a dispatched slash command's effect. The action/flow built-ins (`/help`, `/new`,
    /// `/init`, `/review`, `/compact`, `/export`, `/exit`) run their handler or flow; the
    /// dialog-opening built-ins (`/sessions`, `/models`, `/agents`, `/themes`, `/mcp`) open their
    /// modal picker/view; `/editor` requests the external editor and `/share` produces a local
    /// share reference. Any unhandled name surfaces a notice acknowledging the dispatch.
    fn dispatch_builtin_command(&mut self, name: &str, description: &str) {
        match name {
            "exit" => self.should_quit = true,
            "help" => self.open_help_view(),
            "new" => self.new_session(),
            "export" => self.open_export_dialog(),
            "status" => self.open_status_view(),
            "init" => self.open_init_dialog(),
            "review" => self.open_review_view(),
            "diff" => self.open_diff_viewer(),
            "compact" => self.open_compact_dialog(),
            "sessions" => self.open_sessions_dialog(),
            "timeline" => self.open_timeline_view(),
            "fork" => self.open_fork_dialog(),
            "subagents" => self.open_subagents_dialog(),
            "queue" => self.open_queue_view(),
            "sidebar-toggle" => self.toggle_sidebar(),
            "sidebar-context" => self.set_sidebar_panel(SidebarPanel::Context),
            "sidebar-files" => self.set_sidebar_panel(SidebarPanel::Files),
            "sidebar-lsp" => self.set_sidebar_panel(SidebarPanel::Lsp),
            "sidebar-mcp" => self.set_sidebar_panel(SidebarPanel::Mcp),
            "sidebar-todo" => self.set_sidebar_panel(SidebarPanel::Todo),
            "sidebar-subagents" => self.set_sidebar_panel(SidebarPanel::Subagents),
            "sidebar-queue" => self.set_sidebar_panel(SidebarPanel::Queue),
            "rename" => self.open_rename_dialog(),
            "models" => self.open_models_dialog(),
            "model" => self.open_models_dialog(),
            "connect" => self.open_connect(),
            "agents" => self.open_agents_dialog(),
            "skills" => self.open_skills_dialog(),
            "variant" => self.open_variants_dialog(),
            "themes" => self.open_themes_dialog(),
            "theme" => self.open_themes_dialog(),
            "theme-gallery" => self.open_theme_gallery(),
            "attach" => self.open_attachments_dialog(),
            "paste" => {
                let _ = self.attach_from_clipboard_image();
            }
            "mcp" => self.open_mcp_view(),
            "editor" => self.editor_requested = true,
            "share" => self.share_session(),
            "unshare" => self.unshare_session(),
            "details" => self.toggle_tool_details(),
            "undo" => self.undo(),
            "redo" => self.redo(),
            _ => self
                .transcript
                .push(Entry::Notice(format!("/{} — {}", name, description))),
        }
    }

    /// Open the session picker (`/sessions`): the real persisted sessions (newest-first) as rich
    /// rows (title + `agent · model` + `N msg · age`) under a searchable modal. Listing the database
    /// is best-effort — when no session is persisted yet the synthetic "current session" floor row
    /// is shown so the picker always opens with at least one row. The highlight starts on the first
    /// actionable (persisted) row.
    fn open_sessions_dialog(&mut self) {
        self.session_rename = None;
        self.picker_query.clear();
        self.session_all = self.build_session_picks();
        self.open_dialog(Dialog {
            kind: DialogKind::Sessions,
            title: "Sessions".to_owned(),
            items: Vec::new(),
            selected: 0,
            empty: Some("No sessions yet.".to_owned()),
        });
        self.rebuild_session_view();
        let default = self
            .session_entries
            .iter()
            .position(|entry| entry.id.is_some())
            .unwrap_or(0);
        if let Some(dialog) = self.dialog.as_mut() {
            dialog.selected = default;
        }
    }

    /// Open the full-screen session timeline view (`/timeline`) with the shared view chrome. The
    /// event rail is snapshotted from the persisted session when the view opens (so a redraw never
    /// re-reads the store); with no persisted session it renders an explicit empty state.
    fn open_timeline_view(&mut self) {
        self.timeline_snapshot = self.build_timeline_events();
        self.open_view(crate::view::ViewKind::Timeline);
    }

    /// Open the full-screen prompt-queue view (`/queue`) with the shared view chrome. The queued
    /// prompts (submitted mid-stream and waiting to run) are snapshotted when the view opens so it
    /// stays stable while open; with nothing queued it renders an explicit empty state. The
    /// mid-stream queue manager submenu remains reachable via the command palette / keybind.
    fn open_queue_view(&mut self) {
        self.queue_snapshot = self.build_queue_rows();
        self.queue_selection = 0;
        self.queue_pending_dd = false;
        self.open_view(crate::view::ViewKind::Queue);
    }

    /// The queued prompts as single-line rows (the `@`-expansion header when present, else the sent
    /// prompt), for the `/queue` view snapshot.
    fn build_queue_rows(&self) -> Vec<String> {
        self.queued_prompts
            .iter()
            .map(|queued| {
                queued
                    .header
                    .as_deref()
                    .unwrap_or(&queued.prompt)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            })
            .collect()
    }

    /// The snapshotted `/queue` prompt rows (captured when the view opened), for rendering.
    #[must_use]
    pub fn queue_snapshot(&self) -> &[String] {
        &self.queue_snapshot
    }

    /// The highlighted row in the open `/queue` view, for rendering the selection bar.
    #[must_use]
    pub fn queue_selection(&self) -> usize {
        self.queue_selection
    }

    /// Open the fork picker (`/fork`) backed by the current session timeline.
    fn open_fork_dialog(&mut self) {
        self.open_fork_dialog_with_selected(None);
    }

    fn open_fork_dialog_with_selected(&mut self, preferred_selected: Option<usize>) {
        let Some(session_id) = self.session_id.clone() else {
            self.note_error("No persisted session is active yet.".to_owned());
            return;
        };
        if self.is_busy() {
            self.push_toast(ToastLevel::Info, "Cannot fork while a turn is running");
            return;
        }
        let (rows, entries, key_width) = match self.timeline_rows(&session_id) {
            Ok(rows) => rows,
            Err(err) => {
                self.note_error(format!("Fork unavailable: {err}"));
                return;
            }
        };
        let items: Vec<String> = entries.iter().map(|entry| entry.label.clone()).collect();
        let selected = preferred_selected
            .unwrap_or_else(|| entries.len().saturating_sub(1))
            .min(entries.len().saturating_sub(1));
        self.timeline_entries = entries;
        self.picker_rows = rows;
        self.picker_key_width = (key_width > 0).then_some(key_width);
        self.open_dialog(Dialog {
            kind: DialogKind::Fork,
            title: "Fork from message".to_owned(),
            items,
            selected,
            empty: Some("No turns available to fork.".to_owned()),
        });
    }

    /// Build the fork picker's rich message rows from the persisted session timeline: each row's key
    /// is `HH:MM  who` (real message timestamp + author, rendered AMBER) and its description is the
    /// message's first-line excerpt, so a branch point can be chosen by message. Returns the modal
    /// rows, their backing timeline entries, and the widest key (so the excerpts line up).
    fn timeline_rows(
        &self,
        session_id: &str,
    ) -> Result<(Vec<crate::modal::ModalRow>, Vec<TimelineEntry>, usize), String> {
        let db = self
            .open_store()
            .ok_or_else(|| "no session store is available".to_owned())?;
        let messages = db
            .messages()
            .list_for_session(session_id)
            .map_err(|err| err.to_string())?;
        let mut rows = Vec::with_capacity(messages.len());
        let mut entries = Vec::with_capacity(messages.len());
        let mut key_width = 0usize;
        for message in &messages {
            let who = match message.role.as_str() {
                "assistant" => "assistant",
                "user" => "you",
                other => other,
            };
            let time = hhmm(&message.created_at);
            let excerpt = first_line(&message.content);
            let key = if time.is_empty() {
                who.to_owned()
            } else {
                format!("{time}  {who}")
            };
            key_width = key_width.max(key.chars().count());
            let label = format!("{key} · {excerpt}");
            rows.push(crate::modal::ModalRow {
                key,
                desc: Some(excerpt),
                meta: None,
                meta_color: None,
            });
            entries.push(TimelineEntry {
                session_id: session_id.to_owned(),
                message_id: message.id.clone(),
                label,
            });
        }
        Ok((rows, entries, key_width))
    }

    /// Open the model picker (`/models`): the real catalogue models as rich rows (`provider/model` +
    /// a metadata-derived descriptor + `ctx · price`) under a searchable modal, with the active
    /// model pre-highlighted. Starts in the flat view; `⇥` regroups by vendor.
    fn open_models_dialog(&mut self) {
        self.picker_query.clear();
        self.model_group_by_provider = false;
        self.model_all = self.build_model_picks();
        self.open_dialog(Dialog {
            kind: DialogKind::Models,
            title: "Models".to_owned(),
            items: Vec::new(),
            selected: 0,
            empty: Some("No models in the catalogue.".to_owned()),
        });
        self.rebuild_model_view();
        let active = self.status.model.clone();
        self.select_model_row(&active);
        // Kick off a non-blocking live models.dev refresh so the picker reflects the CURRENT
        // catalogue without a manual `cortex models --refresh`: the offline list shows instantly
        // (built above) and the fetched catalogue swaps in place on channel receipt. Fail-closed
        // and coalesced inside the spawn.
        self.spawn_models_refresh();
    }

    /// Build the full model-picker set from the DYNAMIC merged catalogue (the live models.dev
    /// catalogue fetched in the background this session when present, else the on-disk cache /
    /// embedded snapshot resolved offline — never the network from the synchronous TUI — with
    /// configured providers/models folded on top). Every enabled provider's models become a rich row
    /// (real id/ctx/price + a metadata-derived descriptor and a connected marker). Models are grouped
    /// by vendor with the LATEST models first WITHIN each group (recency, not alphabetical). The
    /// disallowed Zen vendor is excluded throughout (the catalogue strips it before this projection
    /// ever sees it).
    fn build_model_picks(&self) -> Vec<ModelPick> {
        let theme = frame::style::ThemeColors::from_name(&self.theme);
        let (merged, provider_config) = self.merged_model_catalogue();
        Self::model_picks_from_catalogue(
            &merged,
            provider_config.as_ref(),
            &self.status.model,
            &providers::connected_providers(),
            theme.text,
        )
    }

    /// The merged model catalogue the `/models` picker draws from, plus the provider config used to
    /// filter disabled providers: the DYNAMIC models.dev catalogue — the live catalogue fetched in
    /// the background this session when present, else the OFFLINE cache→embedded snapshot (no network
    /// from the synchronous TUI) — with configured providers/models folded on top. The disallowed
    /// Zen vendor is excluded.
    fn merged_model_catalogue(
        &self,
    ) -> (providers::Catalogue, Option<providers::ProviderConfigSet>) {
        let provider_config = effective_config()
            .as_ref()
            .and_then(|cfg| providers::ProviderConfigSet::from_table(cfg.table()).ok());
        // Prefer the live-fetched catalogue landed by a background refresh this session; before one
        // lands (or when it failed), fall back to the OFFLINE cache→embedded snapshot resolved
        // without touching the network from the synchronous TUI.
        let base = self
            .models_catalogue
            .clone()
            .unwrap_or_else(providers::offline_catalogue);
        let merged = provider_config
            .as_ref()
            .map_or_else(|| base.clone(), |cfg| cfg.merged_catalogue(&base));
        (merged, provider_config)
    }

    /// Project a merged catalogue into the picker's rich rows: one row per model id (deduped to the
    /// resolving provider — the first in sorted provider order, matching `Catalogue::find_model`),
    /// grouped by vendor prefix with the newest models first within each group. Connected providers
    /// (a stored credential in `connected`) get a leading marker so they read distinctly from
    /// not-connected ones. A pure function of its inputs so the ordering/marking is unit-testable.
    fn model_picks_from_catalogue(
        merged: &providers::Catalogue,
        provider_config: Option<&providers::ProviderConfigSet>,
        default_model: &str,
        connected: &[String],
        meta_color: ratatui::style::Color,
    ) -> Vec<ModelPick> {
        struct Entry {
            vendor: String,
            id: String,
            name: String,
            release_date: Option<String>,
            desc: String,
            meta: String,
            connected: bool,
        }

        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut entries: Vec<Entry> = Vec::new();
        for provider in merged.providers() {
            let enabled = provider_config.is_none_or(|cfg| cfg.provider_enabled(&provider.id));
            if !enabled {
                continue;
            }
            let is_connected = connected.iter().any(|id| id == &provider.id);
            for model in provider.models.values() {
                if !seen.insert(model.id.clone()) {
                    continue;
                }
                let vendor = model
                    .id
                    .split_once('/')
                    .map_or_else(|| model.id.clone(), |(vendor, _)| vendor.to_owned());
                let is_default = model.id == default_model;
                entries.push(Entry {
                    vendor,
                    id: model.id.clone(),
                    name: model.name.clone(),
                    release_date: model.release_date.clone(),
                    desc: model_descriptor(model, is_default),
                    meta: format!(
                        "{} · {}",
                        format_context(model.limit.context),
                        format_price(model.cost.input)
                    ),
                    connected: is_connected,
                });
            }
        }

        // Vendor groups sort alphabetically (contiguous for the `⇥` grouped view); WITHIN each group
        // the newest models sort first — most recent `release_date` first, then id descending so
        // newer ids lead — deliberately NOT alphabetical.
        entries.sort_by(|a, b| {
            a.vendor
                .cmp(&b.vendor)
                .then_with(|| b.release_date.cmp(&a.release_date))
                .then_with(|| b.id.cmp(&a.id))
        });

        entries
            .into_iter()
            .map(|entry| {
                let marker = if entry.connected {
                    crate::glyphs::CONNECTED_MARKER
                } else {
                    " "
                };
                let key = format!("{marker} {}", entry.id);
                let hay = format!("{} {} {}", entry.id, entry.name, entry.desc).to_lowercase();
                ModelPick {
                    id: entry.id,
                    vendor: entry.vendor,
                    row: crate::modal::ModalRow {
                        key,
                        desc: Some(entry.desc),
                        meta: Some(entry.meta),
                        meta_color: Some(meta_color),
                    },
                    hay,
                }
            })
            .collect()
    }

    /// Rebuild the model picker's visible rows (and the `model_rows` selection map) from the cached
    /// full set filtered by the search query, in the flat or vendor-grouped layout. Grouped view
    /// inserts a non-selectable vendor header before each vendor's block.
    fn rebuild_model_view(&mut self) {
        let query = self.picker_query.to_lowercase();
        let grouped = self.model_group_by_provider;
        let mut items = Vec::new();
        let mut model_rows: Vec<Option<String>> = Vec::new();
        let mut rows = Vec::new();
        let mut key_width = 0usize;
        let mut current_vendor: Option<String> = None;
        for pick in &self.model_all {
            if !query.is_empty() && !pick.hay.contains(&query) {
                continue;
            }
            key_width = key_width.max(pick.row.key.chars().count());
            if grouped && current_vendor.as_deref() != Some(pick.vendor.as_str()) {
                current_vendor = Some(pick.vendor.clone());
                items.push(pick.vendor.clone());
                model_rows.push(None);
                rows.push(crate::modal::ModalRow::new(pick.vendor.clone()));
            }
            items.push(pick.id.clone());
            model_rows.push(Some(pick.id.clone()));
            rows.push(pick.row.clone());
        }
        self.picker_key_width = (key_width > 0).then_some(key_width);
        self.model_rows = model_rows;
        self.picker_rows = rows;
        if let Some(dialog) = self.dialog.as_mut() {
            let clamped = dialog.selected.min(items.len().saturating_sub(1));
            dialog.items = items;
            dialog.selected = clamped;
        }
        self.ensure_model_selection_selectable();
    }

    /// Toggle the model picker between the flat and vendor-grouped views (`⇥`), keeping the same
    /// model highlighted across the switch when it survives the current filter.
    fn toggle_model_grouping(&mut self) {
        let current = self
            .dialog
            .as_ref()
            .and_then(|dialog| self.model_rows.get(dialog.selected()).cloned())
            .flatten();
        self.model_group_by_provider = !self.model_group_by_provider;
        self.rebuild_model_view();
        match current {
            Some(id) => self.select_model_row(&id),
            None => self.ensure_model_selection_selectable(),
        }
    }

    /// Move the model-picker highlight to the row for `id`, or to the nearest selectable row when it
    /// is not present in the current (filtered/grouped) view.
    fn select_model_row(&mut self, id: &str) {
        let pos = self
            .model_rows
            .iter()
            .position(|row| row.as_deref() == Some(id));
        match pos {
            Some(pos) => {
                if let Some(dialog) = self.dialog.as_mut() {
                    dialog.selected = pos;
                }
            }
            None => self.ensure_model_selection_selectable(),
        }
    }

    /// Ensure the model-picker highlight sits on a selectable (non-header) row: if it currently
    /// rests on a vendor header or out of range, snap it to the next selectable row (or the first).
    fn ensure_model_selection_selectable(&mut self) {
        if self.model_rows.is_empty() {
            return;
        }
        let sel = self.dialog.as_ref().map_or(0, Dialog::selected);
        if self.model_rows.get(sel).is_some_and(Option::is_some) {
            return;
        }
        let target = self
            .model_rows
            .iter()
            .enumerate()
            .skip(sel)
            .find_map(|(idx, row)| row.is_some().then_some(idx))
            .or_else(|| self.model_rows.iter().position(Option::is_some));
        if let (Some(target), Some(dialog)) = (target, self.dialog.as_mut()) {
            dialog.selected = target;
        }
    }

    /// Open the agent picker (`/agents`): the built-in agent personas as rich key + description
    /// rows with a right-aligned, semantically-colored kind meta (`primary` AMBER / `subagent`
    /// FAINT), the active agent pre-highlighted. No search row.
    fn open_agents_dialog(&mut self) {
        let theme = frame::style::ThemeColors::from_name(&self.theme);
        let mut items = Vec::new();
        let mut rows = Vec::new();
        let mut key_width = 0usize;
        for agent in Agent::ALL {
            key_width = key_width.max(agent.name().chars().count());
            let (meta, meta_color) = if agent.is_primary() {
                ("primary".to_owned(), theme.secondary)
            } else {
                ("subagent".to_owned(), theme.text_muted)
            };
            items.push(agent.name().to_owned());
            rows.push(crate::modal::ModalRow {
                key: agent.name().to_owned(),
                desc: Some(agent.description().to_owned()),
                meta: Some(meta),
                meta_color: Some(meta_color),
            });
        }
        let selected = Agent::ALL
            .iter()
            .position(|agent| agent.name() == self.status.agent)
            .unwrap_or(0);
        self.picker_rows = rows;
        self.picker_key_width = (key_width > 0).then_some(key_width);
        self.open_dialog(Dialog {
            kind: DialogKind::Agents,
            title: "Agents".to_owned(),
            items,
            selected,
            empty: None,
        });
    }

    /// Open the skill picker (`/skills`): every discovered skill as a rich key + description row
    /// with a right-aligned `builtin`/`project`/`plugin` source meta and a leading `✓` marker for
    /// enabled skills. Space toggles the working set, Enter commits it, Esc discards it.
    fn open_skills_dialog(&mut self) {
        let cwd = std::env::current_dir().ok();
        let discovery = settings::ConfigInputs::from_process(std::collections::BTreeMap::new())
            .ok()
            .map(|inputs| skills::discover(&inputs));
        let mut names = Vec::new();
        let mut picks: Vec<SkillPick> = Vec::new();
        if let Some(discovery) = discovery {
            for skill in discovery.ordered_skills() {
                let source = skill_source_label(&skill.source, cwd.as_deref());
                names.push(skill.name.clone());
                picks.push(SkillPick {
                    name: skill.name.clone(),
                    desc: skill.description.clone(),
                    source,
                });
            }
        }
        if let Some(hooks) = self.plugin_text_complete.as_ref() {
            let mut plugin_skills = hooks.skill_entries();
            plugin_skills.sort_by(|lhs, rhs| lhs.id.cmp(&rhs.id));
            for skill in plugin_skills {
                let name = if skill.name.trim().is_empty() {
                    skill.id
                } else {
                    skill.name
                };
                let name = name.trim();
                if name.is_empty() || names.iter().any(|existing| existing == name) {
                    continue;
                }
                let description = if skill.description.trim().is_empty() {
                    "Plugin skill slot".to_owned()
                } else {
                    skill.description.trim().to_owned()
                };
                names.push(name.to_owned());
                picks.push(SkillPick {
                    name: name.to_owned(),
                    desc: description,
                    source: "plugin".to_owned(),
                });
            }
        }
        let items = names.clone();
        let selected = self
            .selected_skill
            .as_ref()
            .and_then(|selected| names.iter().position(|name| name == selected))
            .unwrap_or(0);
        self.skill_rows = names;
        self.skill_picks = picks;
        self.skill_pending = Some(self.enabled_skills.clone());
        self.rebuild_skill_view();
        self.open_dialog(Dialog {
            kind: DialogKind::Skills,
            title: "Skills".to_owned(),
            items,
            selected,
            empty: Some("No skills discovered.".to_owned()),
        });
    }

    /// Rebuild the skill picker's rich rows from the cached picks and the current working
    /// (pending) enabled set: each row is `<✓|space> name` + description + source meta, with the
    /// `✓` marker present only for enabled skills so the toggle state is visible.
    fn rebuild_skill_view(&mut self) {
        let pending = self.skill_pending.clone().unwrap_or_default();
        let mut key_width = 0usize;
        let mut rows = Vec::new();
        for pick in &self.skill_picks {
            let marker = if pending.contains(&pick.name) {
                crate::glyphs::MARK_DONE
            } else {
                " "
            };
            let key = format!("{marker} {}", pick.name);
            key_width = key_width.max(key.chars().count());
            rows.push(crate::modal::ModalRow {
                key,
                desc: Some(pick.desc.clone()),
                meta: Some(pick.source.clone()),
                meta_color: None,
            });
        }
        self.picker_key_width = (key_width > 0).then_some(key_width);
        self.picker_rows = rows;
    }

    /// Toggle the enabled state of the highlighted skill in the working set, then rebuild the rows
    /// so the `✓` marker reflects the change. A no-op unless the skill picker is open.
    fn toggle_skill_enabled(&mut self) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        if dialog.kind != DialogKind::Skills {
            return;
        }
        let idx = dialog.selected();
        let Some(name) = self.skill_picks.get(idx).map(|pick| pick.name.clone()) else {
            return;
        };
        if let Some(pending) = self.skill_pending.as_mut() {
            if !pending.remove(&name) {
                pending.insert(name);
            }
        }
        self.rebuild_skill_view();
    }

    /// Open the variant picker (`/variant`): the reasoning variants as key + description rows
    /// (no search row, no meta column), with the active variant pre-highlighted.
    fn open_variants_dialog(&mut self) {
        let rows = VARIANT_OPTIONS
            .iter()
            .map(|variant| (*variant).to_owned())
            .collect::<Vec<_>>();
        let mut items = Vec::new();
        let mut modal_rows = Vec::new();
        let mut selected = 0;
        let mut key_width = 0usize;
        for (index, variant) in rows.iter().enumerate() {
            key_width = key_width.max(variant.chars().count());
            if *variant == self.status.variant {
                selected = index;
            }
            items.push(variant.clone());
            modal_rows.push(crate::modal::ModalRow {
                key: variant.clone(),
                desc: Some(variant_description(variant).to_owned()),
                meta: None,
                meta_color: None,
            });
        }
        self.variant_rows = rows;
        self.picker_rows = modal_rows;
        self.picker_key_width = (key_width > 0).then_some(key_width);
        self.open_dialog(Dialog {
            kind: DialogKind::Variants,
            title: "Reasoning variant".to_owned(),
            items,
            selected,
            empty: None,
        });
    }

    /// Open the theme picker (`/themes`): the five brand themes as rich `name + descriptor` rows
    /// with a right-aligned character tag (`warm`/`cool`/…), plus any custom/plugin themes, with the
    /// active theme pre-highlighted. Moving the highlight live-previews the theme; Enter commits it;
    /// Esc restores the theme active when the picker opened.
    fn open_themes_dialog(&mut self) {
        let mut items: Vec<String> = Vec::new();
        let mut rows: Vec<crate::modal::ModalRow> = Vec::new();
        let mut covered: BTreeSet<String> = BTreeSet::new();
        for (display, descriptor, character) in THEME_PICKS {
            covered.insert(canonical_theme_key(display));
            items.push((*display).to_owned());
            rows.push(crate::modal::ModalRow {
                key: (*display).to_owned(),
                desc: Some((*descriptor).to_owned()),
                meta: Some((*character).to_owned()),
                meta_color: None,
            });
        }
        // Append any extra registered/custom/plugin themes not already covered by the brand set.
        let mut extras: Vec<String> = frame::style::ThemeColors::available_theme_names();
        if let Some(hooks) = self.plugin_text_complete.as_ref() {
            for entry in hooks.theme_entries() {
                let name = if entry.name.trim().is_empty() {
                    entry.id
                } else {
                    entry.name
                };
                let name = name.trim().to_owned();
                if !name.is_empty() {
                    extras.push(name);
                }
            }
        }
        extras.sort();
        extras.dedup();
        for name in extras {
            if covered.insert(canonical_theme_key(&name)) {
                items.push(name.clone());
                rows.push(crate::modal::ModalRow {
                    key: name,
                    desc: Some("custom theme".to_owned()),
                    meta: None,
                    meta_color: None,
                });
            }
        }
        let active = canonical_theme_key(&self.theme);
        let selected = items
            .iter()
            .position(|name| canonical_theme_key(name) == active)
            .unwrap_or(0);
        self.picker_rows = rows;
        self.picker_key_width = None;
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

    /// Open the subagent picker (`/subagents`): the live child subagents of the active session as
    /// rich `agent + task` rows with a right-aligned, semantically-colored status meta (running →
    /// AMBER, done → GREEN, queued → FAINT). Enter views the highlighted subagent; `^C` cancels a
    /// running one; Esc closes without changing any subagent's state.
    fn open_subagents_dialog(&mut self) {
        let theme = frame::style::ThemeColors::from_name(&self.theme);
        let picks = self.subagent_picks();
        let mut items = Vec::new();
        let mut rows = Vec::new();
        let mut ids = Vec::new();
        for pick in picks {
            let meta_color = match pick.state {
                "running" => theme.secondary,
                "done" => theme.success,
                "error" => theme.error,
                _ => theme.text_muted,
            };
            items.push(pick.name.clone());
            rows.push(crate::modal::ModalRow {
                key: pick.name,
                desc: Some(pick.task),
                meta: Some(pick.status),
                meta_color: Some(meta_color),
            });
            ids.push(pick.id);
        }
        self.subagent_ids = ids;
        self.picker_rows = rows;
        self.picker_key_width = None;
        self.open_dialog(Dialog {
            kind: DialogKind::Subagents,
            title: "Subagents".to_owned(),
            items,
            selected: 0,
            empty: Some("No subagents for this session.".to_owned()),
        });
    }

    /// The rich subagent-picker rows for the active session's child subagents: each carries the
    /// agent name, its task description, a status word with elapsed time, and the raw state so the
    /// caller can pick a semantic color. Empty when there is no active session or no children.
    fn subagent_picks(&self) -> Vec<SubagentPick> {
        let Some(db) = self.open_store() else {
            return Vec::new();
        };
        let Some(parent_id) = self.session_id.as_deref() else {
            return Vec::new();
        };
        let Ok(sessions) = db.sessions().children(parent_id) else {
            return Vec::new();
        };
        let now = current_epoch_secs();
        let mut picks = Vec::new();
        for session in sessions {
            let Some(title) = session.title.as_deref() else {
                continue;
            };
            if !title.contains("subagent:") {
                continue;
            }
            let (name, parsed_task) = parse_subagent_title(title);
            let messages = db
                .messages()
                .list_for_session(&session.id)
                .unwrap_or_default();
            let last = messages
                .iter()
                .rev()
                .find(|message| message.role == "assistant")
                .map(|message| message.content.clone());
            let state: &'static str = if last
                .as_deref()
                .is_some_and(|text| text.to_ascii_lowercase().contains("state: error"))
            {
                "error"
            } else if last.is_some() {
                "done"
            } else if messages.is_empty() {
                "queued"
            } else {
                "running"
            };
            let elapsed = match state {
                "done" | "error" => parse_epoch_secs(&session.updated_at)
                    .zip(parse_epoch_secs(&session.created_at))
                    .map(|(updated, created)| (updated - created).max(0)),
                "running" => {
                    parse_epoch_secs(&session.created_at).map(|created| (now - created).max(0))
                }
                _ => None,
            };
            let status = match (state, elapsed) {
                ("queued", _) => "queued".to_owned(),
                (word, Some(secs)) => format!("{word} {}", format_duration_secs(secs)),
                (word, None) => word.to_owned(),
            };
            let task = parsed_task
                .or_else(|| last.as_deref().map(first_line))
                .unwrap_or_else(|| "awaiting result".to_owned());
            picks.push(SubagentPick {
                id: session.id,
                name,
                task,
                status,
                state,
            });
        }
        picks
    }

    /// Cancel the highlighted subagent if it is running (`^C` in the subagent picker). There is no
    /// live in-process handle in the TUI, so this surfaces a cancel request as a notice/toast; a
    /// non-running highlight is a no-op. Never quits the app.
    fn cancel_selected_subagent(&mut self) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        if dialog.kind != DialogKind::Subagents {
            return;
        }
        let selected = dialog.selected();
        let name = self
            .picker_rows
            .get(selected)
            .map(|row| row.key.clone())
            .unwrap_or_default();
        let running = self
            .picker_rows
            .get(selected)
            .and_then(|row| row.meta.as_deref())
            .is_some_and(|meta| meta.starts_with("running"));
        if running {
            self.push_toast(ToastLevel::Info, format!("Cancelling subagent {name}"));
            self.transcript.push(Entry::Notice(format!(
                "Requested cancel of subagent {name}"
            )));
        }
    }

    /// Open the attachment picker (`/attach`): project files, newest-first sorting not required.
    ///
    /// The picker intentionally lists every project file so selecting an unsupported file type
    /// surfaces a visible validation error rather than silently hiding non-image/document files.
    fn open_attachments_dialog(&mut self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let items = mention::list_project_files(&cwd);
        self.open_dialog(Dialog {
            kind: DialogKind::Attachments,
            title: "Attach file (png, jpg, pdf, svg)".to_owned(),
            items,
            selected: 0,
            empty: Some("No files in the current project.".to_owned()),
        });
    }

    /// Confirm the currently highlighted attachment picker row, attempting to stage that file as an
    /// input attachment.
    fn confirm_attachment_selection(&mut self) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        if dialog.kind != DialogKind::Attachments {
            return;
        }
        let Some(path) = dialog.items.get(dialog.selected()).cloned() else {
            return;
        };
        let _ = self.attach_local_file_path(&path);
    }

    /// Try to stage a local file attachment in the composer.
    ///
    /// Returns `true` when an attachment was added, `false` on validation failure.
    fn attach_local_file_path(&mut self, raw_path: &str) -> bool {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match crate::attachments::resolve_local_file_attachment(&cwd, raw_path) {
            Ok(attachment) => {
                if self
                    .composer_attachments
                    .iter()
                    .any(|existing| existing.path == attachment.path)
                {
                    self.push_toast(ToastLevel::Info, "Attachment already added");
                    return true;
                }
                let label = attachment.chip_label();
                self.composer_attachments.push(attachment);
                self.push_toast(ToastLevel::Success, format!("Attached {label}"));
                true
            }
            Err(err) => {
                self.transcript
                    .push(Entry::Error(format!("Attachment error: {err}")));
                self.push_toast(ToastLevel::Error, "Attachment failed");
                false
            }
        }
    }

    /// Try to stage a clipboard image attachment in the composer.
    fn attach_from_clipboard_image(&mut self) -> bool {
        match crate::attachments::capture_clipboard_image_attachment() {
            Ok(attachment) => {
                if self
                    .composer_attachments
                    .iter()
                    .any(|existing| existing.path == attachment.path)
                {
                    self.push_toast(ToastLevel::Info, "Attachment already added");
                    return true;
                }
                let label = attachment.chip_label();
                self.composer_attachments.push(attachment);
                self.push_toast(
                    ToastLevel::Success,
                    format!("Attached {label} from clipboard"),
                );
                true
            }
            Err(err) => {
                self.transcript
                    .push(Entry::Error(format!("Clipboard attachment error: {err}")));
                self.push_toast(ToastLevel::Error, "Clipboard attachment failed");
                false
            }
        }
    }

    /// Open the full-screen MCP servers view (`/mcp`) with the shared view chrome. The body reads
    /// live server rows at render time; with no servers configured it shows a clear empty state.
    fn open_mcp_view(&mut self) {
        self.open_view(crate::view::ViewKind::Mcp);
    }

    /// Open the full-screen command-reference view (`/help`) with the shared view chrome: the
    /// two-column command grid (Session/Model & agent/Views/Editing) of real slash builtins + key
    /// affordances. This is the view the global `?` affordance opens (VAL-FOUND-033) as well as the
    /// `/help` command; Esc closes it back to the conversation.
    fn open_help_view(&mut self) {
        self.open_view(crate::view::ViewKind::Help);
    }

    /// Open the full-screen session-status view (`/status`) with the shared view chrome. The runtime
    /// state (session/model/usage/git/workspace) is snapshotted when the view opens so a redraw
    /// never re-runs the git subprocess or re-reads the store.
    fn open_status_view(&mut self) {
        self.status_snapshot = Some(self.build_status_data());
        self.open_view(crate::view::ViewKind::Status);
    }

    /// Open the full-screen code-review view (`/review`) with the shared view chrome, replacing the
    /// old silent review flow. The findings are derived from the real working tree (changed files +
    /// lightweight heuristic checks) and snapshotted when the view opens so a redraw never re-runs
    /// git or re-reads the files.
    fn open_review_view(&mut self) {
        self.review_snapshot = Some(self.build_review_data());
        self.open_view(crate::view::ViewKind::Review);
    }

    /// The snapshotted `/review` data (captured when the view opened), for rendering.
    #[must_use]
    pub fn review_snapshot(&self) -> Option<&crate::view::ReviewData> {
        self.review_snapshot.as_ref()
    }

    /// Build the `/review` view data from the real working tree: the base branch, the changed-file
    /// count (`git status --porcelain`), and the findings produced by [`review_findings`]. This is
    /// gathered once when the view opens.
    fn build_review_data(&self) -> crate::view::ReviewData {
        let branch = git_info().map_or_else(|| "main".to_owned(), |g| g.branch);
        let changed = changed_files();
        let findings = review_findings(&changed);
        crate::view::ReviewData {
            branch,
            files: changed.len(),
            findings,
        }
    }

    /// Open a full-screen Wave-2 view, taking over the frame with the shared chrome. Any open
    /// overlay (menu/dialog/diff viewer/palette) is dismissed first so the view is the sole surface.
    fn open_view(&mut self, kind: crate::view::ViewKind) {
        self.close_slash_menu();
        self.close_mention();
        if self.dialog.is_some() {
            self.close_dialog();
        }
        if self.diff_viewer.is_some() {
            self.close_diff_viewer();
        }
        if self.palette.is_some() {
            self.close_palette();
        }
        self.active_view = Some(kind);
    }

    /// The open full-screen view, if any (for rendering).
    #[must_use]
    pub fn active_view(&self) -> Option<crate::view::ViewKind> {
        self.active_view
    }

    /// Whether a full-screen view is currently open.
    #[must_use]
    pub fn active_view_open(&self) -> bool {
        self.active_view.is_some()
    }

    /// The snapshotted `/status` view data (captured when the view opened), for rendering.
    #[must_use]
    pub fn status_snapshot(&self) -> Option<&crate::view::StatusData> {
        self.status_snapshot.as_ref()
    }

    /// The snapshotted `/timeline` event rail (captured when the view opened), for rendering.
    #[must_use]
    pub fn timeline_snapshot(&self) -> &[crate::view::TimelineEvent] {
        &self.timeline_snapshot
    }

    /// Close the open full-screen view, returning to the conversation and its composer chrome.
    fn close_active_view(&mut self) {
        self.active_view = None;
    }

    /// Open the Wave-7 themes-gallery canonical screen (`/theme-gallery`). It is modal like a
    /// full-screen view, so any other transient overlays are closed first; the same active theme is
    /// used to render it, so switching the theme while it is open re-renders it in the new palette.
    fn open_theme_gallery(&mut self) {
        self.close_slash_menu();
        self.close_mention();
        if self.dialog.is_some() {
            self.close_dialog();
        }
        if self.diff_viewer.is_some() {
            self.close_diff_viewer();
        }
        if self.palette.is_some() {
            self.close_palette();
        }
        self.active_view = None;
        self.theme_gallery_open = true;
    }

    /// Whether the themes-gallery screen is currently open (for rendering).
    #[must_use]
    pub fn theme_gallery_open(&self) -> bool {
        self.theme_gallery_open
    }

    /// Close the themes-gallery screen, returning to the conversation.
    fn close_theme_gallery(&mut self) {
        self.theme_gallery_open = false;
    }

    /// Key handling for the open themes-gallery screen. It is modal: Esc closes it back to the
    /// conversation and `Ctrl+C` still quits so the user is never trapped. Other keys are swallowed.
    fn on_theme_gallery_key(&mut self, key: KeyEvent, ctrl: bool) {
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::Esc {
            self.close_theme_gallery();
        }
    }

    /// Whether the first-run onboarding welcome screen is showing (for rendering).
    #[must_use]
    pub fn onboarding_active(&self) -> bool {
        self.onboarding
    }

    /// Set whether the onboarding welcome screen is showing, used at launch to boot into
    /// onboarding on a pristine first run.
    pub fn set_onboarding(&mut self, onboarding: bool) {
        self.onboarding = onboarding;
    }

    /// Key handling for the first-run onboarding welcome screen. It is a start splash, not a trap:
    /// `Ctrl+C` still quits, `Enter` advances into the normal app surface (the welcome card is no
    /// longer shown), and any other key also dismisses the splash and is then forwarded to the
    /// normal handler so it acts on the surface beneath (e.g. `/` opens the command menu). This
    /// keeps a fresh-config launch from swallowing the user's first keystroke.
    ///
    /// Returns `true` when the key was fully handled here (an `Enter` advance or a `Ctrl+C` quit);
    /// returns `false` after dismissing onboarding so the caller lets the same key fall through to
    /// normal handling.
    fn on_onboarding_key(&mut self, key: KeyEvent, ctrl: bool) -> bool {
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return true;
        }
        self.onboarding = false;
        matches!(key.code, KeyCode::Enter)
    }

    /// Whether the interactive auth/login screen is showing (for rendering + key routing).
    #[must_use]
    pub fn auth_login_active(&self) -> bool {
        self.auth_login.is_some()
    }

    /// The auth/login screen state, or `None` when the screen is not open.
    #[must_use]
    pub fn auth_login_state(&self) -> Option<&crate::auth_login::AuthLoginState> {
        self.auth_login.as_ref()
    }

    /// Open the interactive auth/login screen, resolving each provider's real logged-in status. Used
    /// at launch when `cortex auth login` is invoked interactively (no provider argument on a TTY).
    pub fn open_auth_login(&mut self) {
        self.auth_login = Some(crate::auth_login::AuthLoginState::new());
    }

    /// Key handling for the interactive auth/login screen. It is modal: `Ctrl+C` quits and `Esc`
    /// cancels back out of the screen WITHOUT storing a credential (even if a key was typed). `Tab`
    /// cycles focus between the provider list and the key field; while the list is focused Up/Down
    /// move the selection; while the key field is focused printable keys edit the masked value and
    /// Backspace erases. `Enter` stores the selected provider's typed key to the 0600 store (never
    /// echoing it) and exits. The raw key never leaves [`crate::auth_login::AuthLoginState`].
    fn on_auth_login_key(&mut self, key: KeyEvent, ctrl: bool) {
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        let Some(state) = self.auth_login.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                // Cancel: discard the screen (and any typed key) without persisting anything.
                self.auth_login = None;
            }
            KeyCode::Tab => state.toggle_focus(),
            KeyCode::Up => state.move_selection(-1),
            KeyCode::Down => state.move_selection(1),
            KeyCode::Enter => {
                let _ = state.store_selected();
                self.auth_login = None;
            }
            KeyCode::Backspace => state.backspace(),
            KeyCode::Char(c) if !ctrl => state.type_char(c),
            _ => {}
        }
    }

    /// Whether the mid-session `/connect` provider-auth flow is showing.
    #[must_use]
    pub fn connect_active(&self) -> bool {
        self.connect.is_some()
    }

    /// The mid-session `/connect` flow state, when the screen is showing (for rendering).
    #[must_use]
    pub fn connect_state(&self) -> Option<&crate::connect::ConnectState> {
        self.connect.as_ref()
    }

    /// Open the mid-session `/connect` provider-auth flow: a provider picker over the generalized
    /// provider catalogue (all profiles minus the excluded Zen provider), then a method step and a
    /// masked API-key entry. Reachable any time from the `/connect` slash command (not boot-only).
    pub fn open_connect(&mut self) {
        self.connect = Some(crate::connect::ConnectState::new());
    }

    /// Open the `/connect` flow resolving connected state from an EXPLICIT (isolated) auth store,
    /// so tests exercise the flow hermetically without reading the ambient
    /// `~/.local/share/cortex/auth.json` (off-limits user data on this shared host).
    #[cfg(test)]
    pub(crate) fn open_connect_at(&mut self, auth_path: &std::path::Path) {
        self.connect = Some(crate::connect::ConnectState::new_at(auth_path));
    }

    /// Key handling for the `/connect` flow. It is modal: `Ctrl+C` quits; `Esc` backs out one step
    /// (and closes the flow from the provider step); arrows move the active list; `Enter` advances
    /// the step (and, on the key step, stores the masked key via [`crate::connect::ConnectState`]);
    /// `d` on the provider step disconnects the highlighted (connected) provider after a confirm;
    /// printable keys / Backspace edit the masked key on the key step. The raw key never leaves the
    /// connect state, and a store failure/empty key renders as the flow's inline `✗ error` block.
    fn on_connect_key(&mut self, key: KeyEvent, ctrl: bool) {
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        let Some(step) = self
            .connect
            .as_ref()
            .map(crate::connect::ConnectState::step)
        else {
            return;
        };
        use crate::connect::Step;
        match (step, key.code) {
            (_, KeyCode::Esc) => self.connect_back_or_close(step),
            (Step::Provider | Step::Method, KeyCode::Up) => self.connect_move(-1),
            (Step::Provider | Step::Method, KeyCode::Down) => self.connect_move(1),
            (Step::Provider, KeyCode::Enter) => {
                if let Some(state) = self.connect.as_mut() {
                    state.enter_provider();
                }
            }
            (Step::Provider, KeyCode::Char('d')) if !ctrl => {
                if let Some(state) = self.connect.as_mut() {
                    state.request_disconnect();
                }
            }
            (Step::Method, KeyCode::Enter) => {
                if let Some(state) = self.connect.as_mut() {
                    state.enter_method();
                }
            }
            (Step::ApiKey, KeyCode::Enter) => self.confirm_connect_api_key(),
            (Step::ConfirmDisconnect, KeyCode::Enter) => self.confirm_connect_disconnect(),
            (Step::ApiKey, KeyCode::Backspace) => {
                if let Some(state) = self.connect.as_mut() {
                    state.backspace();
                }
            }
            (Step::ApiKey, KeyCode::Char(c)) if !ctrl => {
                if let Some(state) = self.connect.as_mut() {
                    state.type_char(c);
                }
            }
            (Step::Connected | Step::Disconnected, KeyCode::Enter) => self.connect = None,
            _ => {}
        }
    }

    /// Move the highlighted row in the active `/connect` step by `delta`.
    fn connect_move(&mut self, delta: i32) {
        if let Some(state) = self.connect.as_mut() {
            state.move_selection(delta);
        }
    }

    /// Handle `Esc` in the `/connect` flow: back out one step, cancel a running OAuth handshake, or
    /// close the flow from the provider / connected steps.
    fn connect_back_or_close(&mut self, step: crate::connect::Step) {
        use crate::connect::Step;
        match step {
            Step::Provider | Step::Connected | Step::Disconnected => self.connect = None,
            Step::Method => {
                if let Some(state) = self.connect.as_mut() {
                    state.back_to_provider();
                }
            }
            Step::ApiKey => {
                if let Some(state) = self.connect.as_mut() {
                    state.back_to_method();
                }
            }
            Step::ConfirmDisconnect => {
                if let Some(state) = self.connect.as_mut() {
                    state.cancel_disconnect();
                }
            }
            Step::OAuthWaiting => {
                if let Some(state) = self.connect.as_mut() {
                    state.cancel_oauth();
                }
            }
        }
    }

    /// Confirm the `/connect` masked API-key entry: store it to the 0600 auth store and re-resolve.
    /// On success the provider is usable immediately (it reads as connected in `/models` and a turn
    /// resolves its credential without a restart — the picker and turns both read the store fresh, so
    /// nothing cached needs invalidating). An empty/failed store renders the flow's inline error.
    fn confirm_connect_api_key(&mut self) {
        if let Some(state) = self.connect.as_mut() {
            let _ = state.confirm_api_key();
        }
    }

    /// Confirm the `/connect` disconnect: remove the highlighted provider's stored credential from
    /// the 0600 auth store and re-resolve so it reads as not connected (reflected in `/models` on the
    /// next open). No secret is read or shown; a removal error renders the flow's inline error.
    fn confirm_connect_disconnect(&mut self) {
        if let Some(state) = self.connect.as_mut() {
            let _ = state.confirm_disconnect();
        }
    }

    /// Drain the background OAuth handshake worker into the `/connect` flow each tick, so the
    /// "waiting for authorization" screen advances to connected (structured `oauth` credential
    /// stored 0600, provider re-resolved) or to an inline error, without a restart. A no-op when the
    /// flow is closed or no OAuth handshake is running.
    pub fn poll_connect_oauth(&mut self) {
        if let Some(state) = self.connect.as_mut() {
            state.poll_oauth();
        }
    }

    /// Whether a background models.dev refresh is running, driving the picker's subtle
    /// "refreshing…" indicator.
    #[must_use]
    pub fn models_refreshing(&self) -> bool {
        self.models_refresh_rx.is_some()
    }

    /// The pure spawn-gate decision (separated so the fail-closed + coalescing rules are unit-testable
    /// without touching the process env or opening a socket): spawn a live refresh only when outbound
    /// network is permitted AND no refresh is already in flight.
    #[must_use]
    fn should_spawn_models_refresh(network_disabled: bool, refresh_in_flight: bool) -> bool {
        !network_disabled && !refresh_in_flight
    }

    /// Spawn a NON-BLOCKING background models.dev refresh, mirroring the OAuth/turn worker pattern:
    /// the fetch runs on a `std::thread` with its own current-thread runtime and publishes the
    /// curated catalogue back over an mpsc channel that [`App::poll_models_refresh`] drains on the
    /// event loop — so the synchronous ratatui loop NEVER blocks on the network.
    ///
    /// FAIL-CLOSED: returns immediately (spawning nothing, opening no socket) when the `CORTEX_NO_NET`
    /// kill-switch is engaged, so offline stays cache→embedded with no hang and no error surface;
    /// also a no-op when a refresh is already in flight (coalesced). The `CORTEX_MODELS_DEV_URL`
    /// override is honored via [`providers::models_dev_url`], so the live path can be pointed at a
    /// deterministic loopback fixture.
    pub fn spawn_models_refresh(&mut self) {
        if !Self::should_spawn_models_refresh(
            redaction::network_disabled(),
            self.models_refresh_rx.is_some(),
        ) {
            return;
        }
        // In-process unit tests must never touch the real network or write the shared on-disk cache
        // (an off-limits user path). The live path is exercised end-to-end by the real binary — the
        // `gap_tui_*` behavioral tests and tuistory QA against a `CORTEX_MODELS_DEV_URL` fixture.
        if cfg!(test) {
            return;
        }
        let url = providers::models_dev_url();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let message = match Self::fetch_models_catalogue(&url) {
                Some(catalogue) => {
                    // Warm the on-disk cache so later cold/offline opens are warm.
                    let _ = providers::write_models_cache(&catalogue);
                    ModelsRefreshMessage::Refreshed(Box::new(catalogue))
                }
                None => ModelsRefreshMessage::Failed,
            };
            // The UI may have dropped the receiver (app exited); a send failure is a benign no-op.
            let _ = tx.send(message);
        });
        self.models_refresh_rx = Some(rx);
    }

    /// Run the async models.dev fetch to completion on the background thread's own current-thread
    /// runtime, returning the curated catalogue on success or `None` on any transport/parse failure
    /// (or a runtime that could not be built). The outbound `CORTEX_NO_NET` guard is consulted inside
    /// [`providers::refresh_models_dev_catalogue`] as a second line of defense.
    fn fetch_models_catalogue(url: &str) -> Option<providers::Catalogue> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        runtime
            .block_on(providers::refresh_models_dev_catalogue(url))
            .ok()
    }

    /// Drain the background models.dev refresh worker each tick and apply its result, so an in-flight
    /// refresh swaps the `/models` picker to the fetched catalogue IN PLACE (rebuilt via
    /// [`build_model_picks`](App::build_model_picks)/[`rebuild_model_view`](App::rebuild_model_view))
    /// without a manual reopen, and clears the "refreshing…" indicator. A failed/again-offline fetch
    /// degrades silently to the offline list (no toast/splatter). A no-op when no refresh is running.
    pub fn poll_models_refresh(&mut self) {
        let message = match self.models_refresh_rx.as_ref() {
            Some(rx) => match rx.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.models_refresh_rx = None;
                    return;
                }
            },
            None => return,
        };
        self.models_refresh_rx = None;
        self.apply_models_refresh(message);
    }

    /// Apply a background refresh result: on success adopt the fetched catalogue for this session
    /// and, when the `/models` picker is open, rebuild it in place preserving the highlighted model;
    /// on failure keep the offline list unchanged with no error surface. Separated from the channel
    /// drain so the swap-in-place behavior is deterministically unit-testable.
    fn apply_models_refresh(&mut self, message: ModelsRefreshMessage) {
        let ModelsRefreshMessage::Refreshed(catalogue) = message else {
            // Failed/again-offline: degrade silently to the offline list already shown.
            return;
        };
        self.models_catalogue = Some(*catalogue);
        // If the picker is open, swap the fetched catalogue in place so it replaces the offline one
        // without a manual reopen, keeping the current highlight where it survives the new set.
        if self.dialog.as_ref().map(|dialog| dialog.kind) == Some(DialogKind::Models) {
            let selected = self
                .dialog
                .as_ref()
                .and_then(|dialog| self.model_rows.get(dialog.selected()).cloned())
                .flatten();
            self.model_all = self.build_model_picks();
            self.rebuild_model_view();
            match selected {
                Some(id) => self.select_model_row(&id),
                None => self.ensure_model_selection_selectable(),
            }
        }
    }

    /// Key handling for an open full-screen view. The view is modal: Esc closes it back to the
    /// conversation and `Ctrl+C` still quits so the user is never trapped. Other keys are swallowed
    /// (per-view navigation is added by each view's own feature).
    fn on_view_key(&mut self, key: KeyEvent, ctrl: bool) {
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::Esc {
            self.close_active_view();
            return;
        }
        // The global `?` affordance opens the help/command reference from any full-screen view too,
        // so the affordance behaves identically across screens (VAL-FOUND-033).
        if !ctrl && key.code == KeyCode::Char('?') {
            self.open_help_view();
            return;
        }
        // The `/queue` view is navigable and its footer-advertised actions operate on the selected
        // queued prompt: plain arrows/`j`/`k` move the highlighted (selection-bar) row, `↵` pulls
        // the selection back into the composer for editing, `Ctrl+Up`/`Ctrl+Down` reorder it, and
        // `dd` removes it (all reuse the shared queue-manager mutations).
        if self.active_view == Some(crate::view::ViewKind::Queue) {
            // The vim-style `dd` remove chord: the first `d` arms it, a second `d` removes the
            // selection, and any other key disarms it (handled by clearing the flag up front).
            let dd_armed = std::mem::take(&mut self.queue_pending_dd);
            match key.code {
                KeyCode::Up if ctrl => self.reorder_selected_queued_prompt(true),
                KeyCode::Down if ctrl => self.reorder_selected_queued_prompt(false),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.queue_selection = self.queue_selection.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let len = self.queue_snapshot.len();
                    if len > 0 {
                        self.queue_selection = (self.queue_selection + 1).min(len - 1);
                    }
                }
                KeyCode::Enter => self.edit_selected_queued_prompt(),
                KeyCode::Char('d') if !ctrl => {
                    if dd_armed {
                        self.remove_selected_queued_prompt();
                    } else {
                        self.queue_pending_dd = true;
                    }
                }
                _ => {}
            }
        }
    }

    /// Structured `/mcp` view rows (server name + status tone/word + tool count + exposed
    /// capabilities), or an empty vec when none are configured (the view then renders its empty
    /// state). A connected manager wins because it carries live status/tools; otherwise the view
    /// falls back to the configured server names so `/mcp` remains useful before connect.
    #[must_use]
    pub(crate) fn mcp_view_rows(&self) -> Vec<crate::view::McpServerRow> {
        if let Some(manager) = &self.mcp_manager {
            mcp_server_rows(
                manager.statuses(),
                manager.tools(),
                manager.resources(),
                manager.prompts(),
            )
        } else {
            configured_mcp_server_rows()
        }
    }

    /// Current MCP view rows. A connected manager wins because it carries live status and tool ids;
    /// otherwise the view falls back to the configured names so `/mcp` remains useful before connect.
    fn mcp_rows(&self) -> Vec<String> {
        if let Some(manager) = &self.mcp_manager {
            return mcp_status_rows(
                manager.statuses(),
                manager.tools(),
                manager.resources(),
                manager.prompts(),
            );
        }
        configured_mcp_servers()
    }

    /// Snapshot the live runtime state rendered by the `/status` view: the session title/age, the
    /// active agent/model/variant, the token usage against the model context window, the session
    /// cost, the permission posture, and the workspace/git state. All values are real/dynamic (never
    /// mockup placeholders); git and the store are read once here, not on every redraw.
    fn build_status_data(&self) -> crate::view::StatusData {
        let now = current_epoch_secs();
        let store = self.open_store();
        let record = self
            .session_id
            .as_ref()
            .zip(store.as_ref())
            .and_then(|(id, db)| db.sessions().get(id).ok().flatten());

        let session = record
            .as_ref()
            .and_then(|s| s.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.status.session.clone());

        let started = match (&self.session_id, &record) {
            (Some(id), Some(rec)) => {
                let count = self.session_message_count(id);
                let plural = if count == 1 { "message" } else { "messages" };
                let age = relative_age(&rec.created_at, now);
                if age.is_empty() {
                    format!("{count} {plural}")
                } else if age == "just now" {
                    format!("just now · {count} {plural}")
                } else {
                    format!("{age} ago · {count} {plural}")
                }
            }
            _ => "not yet saved".to_owned(),
        };

        let tokens = format!(
            "in {} · out {} · {} total",
            format_tokens(self.usage.input_tokens),
            format_tokens(self.usage.output_tokens),
            format_tokens(self.usage.total_tokens)
        );
        let cost = format!("${:.2} this session", self.usage.cost.unwrap_or(0.0));

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let permits = crate::turn::permission_posture(&cwd, &self.status.agent);

        crate::view::StatusData {
            session,
            started,
            agent: self.status.agent.clone(),
            model: self.status.model.clone(),
            variant: self.status.variant.clone(),
            context_used: self.usage.total_tokens,
            context_limit: self.model_context_limit(),
            tokens,
            cost,
            permits,
            workspace: crate::motd::current_dir_display(),
            git: git_info(),
        }
    }

    /// The active model's context window (in tokens) from the merged provider catalogue, or `0` when
    /// the model is not found (the usage bar then renders empty).
    fn model_context_limit(&self) -> u64 {
        let effective = effective_config();
        let provider_config = effective
            .as_ref()
            .and_then(|cfg| providers::ProviderConfigSet::from_table(cfg.table()).ok());
        let merged = provider_config.as_ref().map_or_else(
            || providers::Catalogue::embedded().clone(),
            |cfg| cfg.merged_catalogue(providers::Catalogue::embedded()),
        );
        for provider in merged.providers() {
            if let Some(model) = provider.models.values().find(|m| m.id == self.status.model) {
                return model.limit.context;
            }
        }
        0
    }

    /// Snapshot the `/timeline` event rail from the persisted session: a synthetic `session started`
    /// milestone followed by one event per persisted message (`●` you/assistant, `◆` other roles),
    /// each carrying its real `HH:MM` timestamp and a first-line excerpt. Best-effort: an empty vec
    /// when no session is persisted or the store cannot be opened.
    fn build_timeline_events(&self) -> Vec<crate::view::TimelineEvent> {
        use crate::view::{TimelineDot, TimelineEvent};
        let mut events = Vec::new();
        let Some(session_id) = self.session_id.clone() else {
            return events;
        };
        let Some(db) = self.open_store() else {
            return events;
        };

        if let Ok(Some(session)) = db.sessions().get(&session_id) {
            let model = short_model(session.model.as_deref().unwrap_or(&self.status.model));
            events.push(TimelineEvent {
                dot: TimelineDot::Filled,
                time: hhmm(&session.created_at),
                who: "session started".to_owned(),
                text: format!("{} · {model}", self.status.agent),
            });
        }

        if let Ok(messages) = db.messages().list_for_session(&session_id) {
            for message in &messages {
                let (dot, who) = match message.role.as_str() {
                    "user" => (TimelineDot::Filled, "you".to_owned()),
                    "assistant" => (TimelineDot::Filled, "assistant".to_owned()),
                    other => (TimelineDot::Diamond, other.to_owned()),
                };
                events.push(TimelineEvent {
                    dot,
                    time: hhmm(&message.created_at),
                    who,
                    text: first_line(&message.content),
                });
            }
        }

        events
    }

    /// Produce a local (loopback) share reference for the current session (`/share`): mint a stable
    /// share id, persist a small reference file under the data directory (best-effort), and surface
    /// the loopback URL and identifier. Only local share is in scope (no external upload).
    fn share_session(&mut self) {
        if let Some(server) = self.remote_server.clone()
            && let Some(session_id) = self.ensure_session_id()
        {
            match server.share_session(&session_id) {
                Ok(share) => {
                    self.transcript.push(Entry::Notice(format!(
                        "Local share reference created: {}  ·  id {}",
                        share.url, share.share_id
                    )));
                    self.push_toast(ToastLevel::Success, "Share link created");
                    return;
                }
                Err(err) => {
                    self.transcript
                        .push(Entry::Notice(format!("Server share failed: {err}")));
                    self.push_toast(ToastLevel::Error, "Share failed");
                    return;
                }
            }
        }
        let id = local_share_id();
        let url = format!("http://127.0.0.1:3200/share/{id}");
        let shares_dir = self.share_dir();
        let notice = match write_share_reference(&id, &self.status.session, &shares_dir) {
            Ok(path) => format!(
                "Local share reference created: {url}  ·  id {id}  ·  saved to {}",
                path.display()
            ),
            Err(_) => format!("Local share reference created: {url}  ·  id {id}"),
        };
        self.transcript.push(Entry::Notice(notice));
        self.push_toast(ToastLevel::Success, "Share link created");
    }

    /// Revoke local shares for the current session (`/unshare`), using the attached server when
    /// available or the local on-disk share references otherwise.
    fn unshare_session(&mut self) {
        if let Some(server) = self.remote_server.clone() {
            let Some(session_id) = self.session_id.clone() else {
                self.transcript.push(Entry::Notice(
                    "No persisted session yet, nothing to unshare.".to_owned(),
                ));
                self.push_toast(ToastLevel::Info, "Nothing to unshare");
                return;
            };
            match server.unshare_session(&session_id) {
                Ok(revoked) => {
                    let detail = if revoked {
                        "revoked"
                    } else {
                        "no active shares"
                    };
                    self.transcript.push(Entry::Notice(format!(
                        "Server unshare: {detail} ({session_id})"
                    )));
                    self.push_toast(ToastLevel::Success, "Share revoked");
                }
                Err(err) => self.note_error(format!("Server unshare failed: {err}")),
            }
            return;
        }

        let session_id = self
            .session_id
            .clone()
            .unwrap_or_else(|| self.status.session.clone());
        let shares_dir = self.share_dir();
        match revoke_local_share_references(&session_id, &shares_dir) {
            Ok(count) => {
                let detail = if count == 0 {
                    "no local share references found".to_owned()
                } else {
                    format!("revoked {count} local share reference(s)")
                };
                self.transcript
                    .push(Entry::Notice(format!("Unshared {session_id}: {detail}")));
                self.push_toast(ToastLevel::Success, "Share revoked");
            }
            Err(err) => self.note_error(format!("Unshare failed: {err}")),
        }
    }

    fn share_dir(&self) -> PathBuf {
        self.share_data_dir
            .clone()
            .unwrap_or_else(|| data_subdir("shares"))
    }

    fn run_mcp_prompt_command(&mut self, command_name: &str, id: &str, required: &[String]) {
        if !required.is_empty() {
            self.transcript.push(Entry::Notice(format!(
                "/{command_name} requires arguments ({}) and cannot run from slash yet.",
                required.join(", ")
            )));
            self.push_toast(ToastLevel::Info, "Prompt requires arguments");
            return;
        }
        let Some(manager) = self.mcp_manager.clone() else {
            self.transcript.push(Entry::Notice(
                "MCP is not connected, cannot run MCP prompt command.".to_owned(),
            ));
            self.push_toast(ToastLevel::Info, "MCP unavailable");
            return;
        };
        let Some(runtime) = self.mcp_runtime.as_ref() else {
            self.transcript.push(Entry::Notice(
                "MCP runtime is unavailable, cannot run MCP prompt command.".to_owned(),
            ));
            self.push_toast(ToastLevel::Info, "MCP unavailable");
            return;
        };
        match runtime.block_on(manager.get_prompt(id, serde_json::json!({}))) {
            Ok(result) => {
                let prompt = result.content.trim().to_owned();
                if prompt.is_empty() {
                    self.transcript.push(Entry::Notice(format!(
                        "/{command_name} returned an empty MCP prompt."
                    )));
                    self.push_toast(ToastLevel::Info, "Prompt was empty");
                    return;
                }
                self.start_flow(
                    &format!("/{command_name}"),
                    &format!("/{command_name} — running MCP prompt…"),
                    &prompt,
                );
            }
            Err(err) => self.note_error(format!("MCP prompt command failed: {err}")),
        }
    }

    /// Undo the last file-affecting change (`/undo`, the undo keybind, and the palette share this):
    /// restore the most recently changed file(s) to their pre-change bytes on disk. The outcome is
    /// reported as a transcript notice and a toast; an empty history (or an unresolved store) is
    /// handled gracefully with a clear "nothing to undo" note rather than an error or a crash.
    fn undo(&mut self) {
        self.restore_change(false);
    }

    /// Redo the last undone change (`/redo`, the redo keybind, and the palette share this):
    /// re-apply the most recently undone file change(s) on disk. Reported like [`App::undo`].
    fn redo(&mut self) {
        self.restore_change(true);
    }

    /// Shared undo/redo handler: walk the checkpoint timeline one step (back for undo, forward for
    /// redo) and report the outcome. `redo` selects the direction and the user-facing phrasing.
    ///
    /// Refused while a turn streams (the running turn's snapshot is not finalized yet), so an undo
    /// cannot race the in-flight edits. With no store or an empty history it is a graceful no-op
    /// note; a conflict (a file changed out-of-band since the snapshot) is reported without
    /// rewriting anything.
    fn restore_change(&mut self, redo: bool) {
        let verb = if redo { "redo" } else { "undo" };
        if self.is_busy() {
            self.push_toast(
                ToastLevel::Warning,
                format!("Cannot {verb} while a turn is running"),
            );
            return;
        }
        let outcome = match self.checkpoints.as_ref() {
            Some(store) => {
                if redo {
                    store.redo()
                } else {
                    store.undo()
                }
            }
            None => Ok(RestoreOutcome::Nothing),
        };
        match outcome {
            Ok(outcome) => {
                let (notice, level, toast) = restore_report(redo, &outcome);
                self.transcript.push(Entry::Notice(notice));
                self.push_toast(level, toast);
            }
            Err(err) => {
                self.transcript
                    .push(Entry::Notice(format!("Cannot {verb}: {err}")));
                self.push_toast(ToastLevel::Error, format!("Cannot {verb}"));
            }
        }
        self.pin_to_bottom();
    }

    /// Open a modal dialog, replacing any currently-open one and closing the slash menu.
    fn open_dialog(&mut self, dialog: Dialog) {
        if dialog.kind != DialogKind::Models {
            self.model_rows.clear();
        }
        if dialog.kind != DialogKind::Skills {
            self.skill_rows.clear();
        }
        if dialog.kind != DialogKind::Variants {
            self.variant_rows.clear();
        }
        self.selection_anim = None;
        self.dialog = Some(dialog);
    }

    /// Open the `/diff` viewer over the most recent file diffs in the transcript (one entry per
    /// path, latest change wins). If no file diffs exist, surface a clear notice instead of opening
    /// an empty modal.
    fn open_diff_viewer(&mut self) {
        let mut files: Vec<crate::diff::FileDiff> = Vec::new();
        let mut seen_paths: BTreeSet<String> = BTreeSet::new();
        for entry in self.transcript.iter().rev() {
            let diff = match entry {
                Entry::Diff(diff) => Some(diff),
                Entry::Tool(card) => card.diff.as_deref(),
                _ => None,
            };
            if let Some(diff) = diff
                && seen_paths.insert(diff.path.clone())
            {
                files.push(diff.clone());
            }
        }
        files.reverse();
        if files.is_empty() {
            self.transcript.push(Entry::Notice(
                "No file changes yet, run an edit first.".to_owned(),
            ));
            self.push_toast(ToastLevel::Info, "No diffs to show");
            return;
        }
        self.diff_viewer = Some(DiffViewer::new(files));
    }

    /// The open `/diff` viewer state, if any.
    #[must_use]
    pub fn diff_viewer(&self) -> Option<&DiffViewer> {
        self.diff_viewer.as_ref()
    }

    /// Whether the `/diff` viewer is currently open.
    #[must_use]
    pub fn diff_viewer_open(&self) -> bool {
        self.diff_viewer.is_some()
    }

    /// Close the open `/diff` viewer.
    fn close_diff_viewer(&mut self) {
        self.diff_viewer = None;
    }

    fn diff_select_next_file(&mut self) {
        if let Some(viewer) = self.diff_viewer.as_mut() {
            if viewer.files.is_empty() {
                return;
            }
            viewer.selected_file = (viewer.selected_file + 1) % viewer.files.len();
            viewer.selected_hunk = 0;
            viewer.clamp_selection();
        }
    }

    fn diff_select_prev_file(&mut self) {
        if let Some(viewer) = self.diff_viewer.as_mut() {
            if viewer.files.is_empty() {
                return;
            }
            let len = viewer.files.len();
            viewer.selected_file = (viewer.selected_file + len - 1) % len;
            viewer.selected_hunk = 0;
            viewer.clamp_selection();
        }
    }

    fn diff_select_next_hunk(&mut self) {
        if let Some(viewer) = self.diff_viewer.as_mut() {
            let hunk_count = viewer.selected_file().map_or(0, |file| file.hunks.len());
            if hunk_count == 0 {
                return;
            }
            viewer.selected_hunk = (viewer.selected_hunk + 1) % hunk_count;
        }
    }

    fn diff_select_prev_hunk(&mut self) {
        if let Some(viewer) = self.diff_viewer.as_mut() {
            let hunk_count = viewer.selected_file().map_or(0, |file| file.hunks.len());
            if hunk_count == 0 {
                return;
            }
            viewer.selected_hunk = (viewer.selected_hunk + hunk_count - 1) % hunk_count;
        }
    }

    fn diff_toggle_mode(&mut self) {
        if let Some(viewer) = self.diff_viewer.as_mut() {
            viewer.mode = match viewer.mode {
                DiffViewMode::Unified => DiffViewMode::Split,
                DiffViewMode::Split => DiffViewMode::Unified,
            };
        }
    }

    fn diff_toggle_focus(&mut self) {
        if let Some(viewer) = self.diff_viewer.as_mut() {
            viewer.focus = match viewer.focus {
                DiffViewerFocus::Files => DiffViewerFocus::Hunks,
                DiffViewerFocus::Hunks => DiffViewerFocus::Files,
            };
        }
    }

    /// Toggle the selected file's disclosure (expand/collapse its hunk body) in the `/diff` viewer.
    fn diff_toggle_expanded(&mut self) {
        if let Some(viewer) = self.diff_viewer.as_mut() {
            let index = viewer.selected_file;
            if let Some(state) = viewer.expanded.get_mut(index) {
                *state = !*state;
            }
        }
    }

    /// Revert the selected file or hunk from the open `/diff` viewer. Selection scope follows the
    /// active focus pane: file-tree focus reverts the whole file diff, hunk focus reverts only the
    /// selected hunk. The write is conflict-checked against the viewer's expected post-edit content.
    fn revert_diff_selection(&mut self) {
        if self.is_busy() {
            self.push_toast(ToastLevel::Warning, "Cannot revert while a turn is running");
            return;
        }
        let Some(viewer) = self.diff_viewer.as_ref() else {
            return;
        };
        let Some(diff) = viewer.selected_file().cloned() else {
            return;
        };
        let selected_hunk = viewer.selected_hunk;
        let focus = viewer.focus;
        let target = match focus {
            DiffViewerFocus::Files => None,
            DiffViewerFocus::Hunks => Some(selected_hunk),
        };
        // Record the revert as a fresh checkpoint turn so it is undoable and so the net-change
        // surfaces that read the shadow store (`cortex snapshot diff`, the server
        // `/session/{id}/diff` route) stay consistent with disk after the revert: the store keeps
        // the original turn's pre-image and this revert turn's post-image, so the reported net diff
        // collapses to only the hunks that remain. The pre-image MUST be captured before the revert
        // write; a no-op or failed revert finalizes nothing (the recorder is simply dropped). When
        // no store is attached the revert stays a plain on-disk write, exactly as before.
        let recorder = self.checkpoints.as_ref().and_then(|store| {
            let absolute = resolve_diff_path(&diff.path).ok()?;
            let label = self
                .session_id
                .clone()
                .unwrap_or_else(|| DIFF_REVERT_TURN_LABEL.to_owned());
            let recorder = store.begin_turn(label);
            recorder.record_pre_image(&absolute);
            Some(recorder)
        });
        match apply_diff_revert(&diff, target) {
            Ok(changed) => {
                if changed {
                    if let Some(recorder) = &recorder {
                        let _ = recorder.finalize();
                    }
                    let scope = match target {
                        Some(index) => format!("hunk {} in {}", index + 1, diff.path),
                        None => format!("file {}", diff.path),
                    };
                    self.transcript
                        .push(Entry::Notice(format!("Reverted {scope}.")));
                    self.push_toast(ToastLevel::Success, "Revert applied");
                    self.refresh_diff_viewer_file(&diff.path);
                } else {
                    self.push_toast(ToastLevel::Info, "Nothing changed");
                }
            }
            Err(err) => {
                self.transcript
                    .push(Entry::Notice(format!("Cannot revert {}: {err}", diff.path)));
                self.push_toast(ToastLevel::Error, "Revert failed");
            }
        }
    }

    /// Recompute one file's diff entry in the open viewer from disk after a revert action. If the
    /// file is now fully reverted, remove it from the viewer; when no files remain, close the view.
    fn refresh_diff_viewer_file(&mut self, path: &str) {
        let Some(viewer) = self.diff_viewer.as_mut() else {
            return;
        };
        let Some(index) = viewer.files.iter().position(|file| file.path == path) else {
            return;
        };
        let before = viewer.files[index].before.clone();
        let resolved = resolve_diff_path(path);
        let after = match resolved {
            Ok(path) => std::fs::read_to_string(path).ok(),
            Err(_) => None,
        };
        let updated = crate::diff::compute_file_diff(path, before.as_deref(), after.as_deref());
        if let Some(updated) = updated {
            viewer.files[index] = updated;
            viewer.clamp_selection();
        } else {
            viewer.files.remove(index);
            if index < viewer.expanded.len() {
                viewer.expanded.remove(index);
            }
            viewer.clamp_selection();
            if viewer.files.is_empty() {
                self.close_diff_viewer();
                self.transcript
                    .push(Entry::Notice("All diffs reverted.".to_owned()));
            }
        }
    }

    /// Key handling for the open `/diff` viewer modal.
    fn on_diff_viewer_key(&mut self, key: KeyEvent, ctrl: bool) {
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        let focus = self
            .diff_viewer
            .as_ref()
            .map(|viewer| viewer.focus)
            .unwrap_or(DiffViewerFocus::Files);
        match key.code {
            KeyCode::Esc => self.close_diff_viewer(),
            KeyCode::Tab => self.diff_toggle_focus(),
            KeyCode::Char('v') if !ctrl => self.diff_toggle_mode(),
            KeyCode::Char('r') if !ctrl => self.revert_diff_selection(),
            KeyCode::Char('u') if !ctrl => self.undo(),
            KeyCode::Enter => self.diff_toggle_expanded(),
            KeyCode::Up => match focus {
                DiffViewerFocus::Files => self.diff_select_prev_file(),
                DiffViewerFocus::Hunks => self.diff_select_prev_hunk(),
            },
            KeyCode::Down => match focus {
                DiffViewerFocus::Files => self.diff_select_next_file(),
                DiffViewerFocus::Hunks => self.diff_select_next_hunk(),
            },
            KeyCode::Left if focus == DiffViewerFocus::Hunks => self.diff_toggle_focus(),
            KeyCode::Right if focus == DiffViewerFocus::Files => self.diff_toggle_focus(),
            _ => {}
        }
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
        self.selection_anim = None;
        self.session_entries.clear();
        self.timeline_entries.clear();
        self.model_rows.clear();
        self.picker_query.clear();
        self.picker_rows.clear();
        self.picker_key_width = None;
        self.session_all.clear();
        self.model_all.clear();
        self.model_group_by_provider = false;
        self.skill_rows.clear();
        self.skill_picks.clear();
        self.skill_pending = None;
        self.variant_rows.clear();
        self.subagent_ids.clear();
        self.session_rename = None;
    }

    /// Handle a key while a modal dialog is open. The dialog is modal: it owns navigation and
    /// dismissal, and swallows other keys so the prompt is not edited beneath it. `Ctrl+C` still
    /// quits so the user is never trapped, and `Esc`/`Enter` close the dialog and return focus to
    /// the prompt. Searchable pickers (session/model) treat plain typing as a filter query and
    /// `Backspace` as edit; the model picker toggles vendor grouping with `Tab`. The session picker
    /// keeps its in-place actions on modifier keys so they do not collide with the filter: `Ctrl+N`
    /// starts a new session, `Ctrl+R` renames the highlighted session (a sub-mode where typing edits
    /// the new title, `Enter` commits, `Esc` cancels the rename), and `Ctrl+D`/`Delete` removes it.
    fn on_dialog_key(&mut self, key: KeyEvent, ctrl: bool) {
        if ctrl && key.code == KeyCode::Char('c') {
            // In the subagent picker `^C` cancels the highlighted running subagent (VAL-PICK-056)
            // rather than quitting the app.
            if self.dialog.as_ref().map(|dialog| dialog.kind) == Some(DialogKind::Subagents) {
                self.cancel_selected_subagent();
                return;
            }
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

        let kind = self.dialog.as_ref().map(|dialog| dialog.kind);
        let is_sessions = kind == Some(DialogKind::Sessions);
        let is_models = kind == Some(DialogKind::Models);
        let is_skills = kind == Some(DialogKind::Skills);
        let searchable = is_sessions || is_models;
        match key.code {
            KeyCode::Up => self.dialog_select_prev(),
            KeyCode::Down => self.dialog_select_next(),
            KeyCode::Enter => self.dialog_confirm(),
            KeyCode::Esc => self.dialog_cancel(),
            KeyCode::Char(' ') if is_skills => self.toggle_skill_enabled(),
            KeyCode::Tab if is_models => self.toggle_model_grouping(),
            KeyCode::Char('n') if ctrl && is_sessions => {
                self.new_session();
                self.close_dialog();
            }
            KeyCode::Char('r') if ctrl && is_sessions => self.begin_session_rename(),
            KeyCode::Char('d') if ctrl && is_sessions => self.delete_selected_session(),
            KeyCode::Delete if is_sessions => self.delete_selected_session(),
            KeyCode::Backspace if searchable => self.picker_search_pop(),
            KeyCode::Char(c) if searchable && !ctrl => self.picker_search_push(c),
            _ => {}
        }
    }

    /// Whether the open dialog is a searchable picker (session/model) whose typed query filters its
    /// rows in place.
    fn picker_is_searchable(&self) -> bool {
        matches!(
            self.dialog.as_ref().map(|dialog| dialog.kind),
            Some(DialogKind::Sessions | DialogKind::Models)
        )
    }

    /// Append a character to the open picker's search query and re-filter its rows (resetting the
    /// highlight to the top of the narrowed list).
    fn picker_search_push(&mut self, c: char) {
        if !self.picker_is_searchable() {
            return;
        }
        self.picker_query.push(c);
        if let Some(dialog) = self.dialog.as_mut() {
            dialog.selected = 0;
        }
        self.rebuild_picker_view();
    }

    /// Remove the last character from the open picker's search query and re-filter its rows; an
    /// emptied query restores the full list.
    fn picker_search_pop(&mut self) {
        if !self.picker_is_searchable() {
            return;
        }
        self.picker_query.pop();
        if let Some(dialog) = self.dialog.as_mut() {
            dialog.selected = 0;
        }
        self.rebuild_picker_view();
    }

    /// Re-filter the open searchable picker's rows for the current query.
    fn rebuild_picker_view(&mut self) {
        match self.dialog.as_ref().map(|dialog| dialog.kind) {
            Some(DialogKind::Sessions) => self.rebuild_session_view(),
            Some(DialogKind::Models) => self.rebuild_model_view(),
            _ => {}
        }
    }

    /// Move the dialog highlight to the next row (wrapping at the end).
    fn dialog_select_next(&mut self) {
        let before = self.dialog.as_ref().map(Dialog::selected);
        if let Some(dialog) = self.dialog.as_mut() {
            let len = dialog.items.len();
            if len > 0 {
                if dialog.kind == DialogKind::Models {
                    let mut next = dialog.selected();
                    for _ in 0..len {
                        next = (next + 1) % len;
                        if self.model_rows.get(next).is_some_and(Option::is_some) {
                            dialog.selected = next;
                            break;
                        }
                    }
                } else {
                    dialog.selected = (dialog.selected() + 1) % len;
                }
            }
        }
        let after = self.dialog.as_ref().map(Dialog::selected);
        if let (Some(from), Some(to)) = (before, after) {
            self.note_selection_move(from, to);
        }
        self.sync_theme_preview();
    }

    /// Move the dialog highlight to the previous row (wrapping at the start).
    fn dialog_select_prev(&mut self) {
        let before = self.dialog.as_ref().map(Dialog::selected);
        if let Some(dialog) = self.dialog.as_mut() {
            let len = dialog.items.len();
            if len > 0 {
                if dialog.kind == DialogKind::Models {
                    let mut prev = dialog.selected();
                    for _ in 0..len {
                        prev = (prev + len - 1) % len;
                        if self.model_rows.get(prev).is_some_and(Option::is_some) {
                            dialog.selected = prev;
                            break;
                        }
                    }
                } else {
                    dialog.selected = (dialog.selected() + len - 1) % len;
                }
            }
        }
        let after = self.dialog.as_ref().map(Dialog::selected);
        if let (Some(from), Some(to)) = (before, after) {
            self.note_selection_move(from, to);
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
                    // Also record the choice with the active session so a headless
                    // `cortex run --session <id>` resume (no `--model`) honors it (VAL-CROSS-006).
                    self.persist_session_model(&model);
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
            Some(DialogKind::Skills) => self.commit_skill_toggles(),
            Some(DialogKind::Variants) => {
                if let Some(variant) = self.select_variant() {
                    self.persist_config_string("variant", &variant);
                    self.transcript
                        .push(Entry::Notice(format!("Variant set to {variant}")));
                }
            }
            Some(DialogKind::Sessions) => {
                if let Some(id) = self.selected_session_id() {
                    self.load_session(&id);
                }
            }
            Some(DialogKind::Fork) => self.fork_from_timeline_selection(),
            Some(DialogKind::Export) => self.perform_export(),
            Some(DialogKind::Subagents) => {
                if let Some(id) = self
                    .dialog
                    .as_ref()
                    .map(Dialog::selected)
                    .and_then(|selected| self.subagent_ids.get(selected).cloned())
                {
                    self.close_dialog();
                    self.load_session(&id);
                    return;
                }
            }
            Some(DialogKind::Attachments) => {
                self.confirm_attachment_selection();
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

    /// Open the durable session store at the bound database path, or `None` when persistence is
    /// disabled (no path bound — the default in tests) or the store cannot be opened. Opening per
    /// operation matches the picker's read helpers and avoids holding a connection across the loop.
    fn open_store(&self) -> Option<db::Database> {
        let path = self.db_path.as_ref()?;
        db::Database::open(path).ok()
    }

    /// Ensure a durable session backs the current conversation, creating one lazily on first need,
    /// and return its id. The new id is surfaced in the footer so the conversation is addressable
    /// across surfaces. Best-effort: returns `None` (leaving the conversation in-memory only) when
    /// persistence is disabled or the store cannot be opened.
    fn ensure_session_id(&mut self) -> Option<String> {
        if let Some(id) = &self.session_id {
            return Some(id.clone());
        }
        let session = self.open_store()?.sessions().create(None).ok()?;
        self.session_id = Some(session.id.clone());
        self.status.session = session.id.clone();
        Some(session.id)
    }

    /// Persist one conversation message under the active session, creating the session lazily if
    /// needed. Best-effort: a disabled/unopenable store or a write error leaves the turn rendered
    /// in-memory without aborting it (the answer has already been shown).
    fn persist_message(&mut self, role: &str, content: &str, parts: &[MessagePart]) {
        let Some(id) = self.ensure_session_id() else {
            return;
        };
        if let Some(db) = self.open_store() {
            let persisted = if parts.is_empty() {
                db.messages().create(&id, role, content)
            } else {
                db.messages().create_with_parts(&id, role, content, parts)
            };
            if let Ok(message) = persisted {
                self.seen_message_ids.insert(message.id);
            }
        }
    }

    /// Persist one turn's usage under the active session so stats and cross-surface usage views
    /// include TUI-driven turns.
    fn persist_usage(&mut self, usage: &Usage) {
        let Some(id) = self.ensure_session_id() else {
            return;
        };
        if let Some(db) = self.open_store() {
            let _ = db.usage().record(
                &id,
                u64::from(usage.input_tokens),
                u64::from(usage.output_tokens),
                u64::from(usage.total_tokens),
                usage.cost,
            );
        }
    }

    /// Persist the model chosen for the active session (the durable seam behind the `/models`
    /// picker), creating the session lazily so the choice attaches even before the first turn. This
    /// is what a headless `cortex run --session <id>` resume reads back (VAL-CROSS-006). Best-effort.
    fn persist_session_model(&mut self, model: &str) {
        let Some(id) = self.ensure_session_id() else {
            return;
        };
        if let Some(db) = self.open_store() {
            let _ = db.sessions().set_model(&id, Some(model));
        }
    }

    /// Every persisted session newest-first from the bound store, best-effort: an empty list when
    /// no store is bound or it cannot be opened, so the picker still opens with just the current
    /// session. Reads through the same bound store the TUI writes to, so the picker reflects this
    /// session's own persisted turns and any created by another surface against the shared store.
    fn persisted_sessions(&self) -> Vec<db::Session> {
        match self.open_store() {
            Some(db) => db.sessions().list().unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// The number of persisted messages under a single session (best-effort; `0` when unreadable),
    /// used by the `/status` view. The session picker batches every count via
    /// [`Self::session_message_counts`] instead of calling this per row.
    fn session_message_count(&self, id: &str) -> usize {
        self.open_store()
            .and_then(|db| db.messages().list_for_session(id).ok())
            .map_or(0, |messages| messages.len())
    }

    /// Message counts for every persisted session read in one grouped store query, so the session
    /// picker tallies each row from a fixed number of store round-trips rather than one open per
    /// session. Best-effort: an empty map when no store is bound or it cannot be read.
    fn session_message_counts(&self) -> std::collections::HashMap<String, usize> {
        self.open_store()
            .and_then(|db| db.messages().counts_by_session().ok())
            .unwrap_or_default()
    }

    /// Build the full session-picker set: every persisted session (newest-first), each as a rich
    /// row (`title` + `agent · model` + `N msg · age`) with its backing id/title. When no sessions
    /// are persisted yet, fall back to the synthetic "current session" floor row so the picker is
    /// never empty (its `id` is `None`, so it is non-actionable).
    fn build_session_picks(&self) -> Vec<SessionPick> {
        let theme = frame::style::ThemeColors::from_name(&self.theme);
        let meta_color = theme.text;
        let agent = self.status.agent.clone();
        let persisted = self.persisted_sessions();
        if persisted.is_empty() {
            let key = format!("current — {} session", self.status.session);
            let desc = format!("{agent} · {}", short_model(&self.status.model));
            return vec![SessionPick {
                id: None,
                title: None,
                row: crate::modal::ModalRow {
                    key: key.clone(),
                    desc: Some(desc),
                    meta: None,
                    meta_color: None,
                },
                hay: key.to_lowercase(),
            }];
        }
        let counts = self.session_message_counts();
        let now = current_epoch_secs();
        persisted
            .into_iter()
            .map(|session| {
                let title = session
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| session.id.clone());
                let model = short_model(session.model.as_deref().unwrap_or(&self.status.model));
                let count = counts.get(&session.id).copied().unwrap_or(0);
                let age = relative_age(&session.updated_at, now);
                let meta = if age.is_empty() {
                    format!("{count} msg")
                } else {
                    format!("{count} msg · {age}")
                };
                let hay = format!("{title} {model}").to_lowercase();
                SessionPick {
                    id: Some(session.id.clone()),
                    title: session.title.clone(),
                    row: crate::modal::ModalRow {
                        key: title,
                        desc: Some(format!("{agent} · {model}")),
                        meta: Some(meta),
                        meta_color: Some(meta_color),
                    },
                    hay,
                }
            })
            .collect()
    }

    /// Rebuild the session picker's visible rows (and their backing arrays) from the cached full set
    /// filtered by the current search query, keeping the highlight on a valid row. Case-insensitive
    /// substring match over each row's title + model; an empty query shows the full list.
    fn rebuild_session_view(&mut self) {
        let query = self.picker_query.to_lowercase();
        let mut items = Vec::new();
        let mut entries = Vec::new();
        let mut rows = Vec::new();
        for pick in &self.session_all {
            if !query.is_empty() && !pick.hay.contains(&query) {
                continue;
            }
            items.push(pick.row.key.clone());
            entries.push(SessionEntry {
                id: pick.id.clone(),
                title: pick.title.clone(),
            });
            rows.push(pick.row.clone());
        }
        self.picker_key_width = rows
            .iter()
            .map(|row| row.key.chars().count())
            .max()
            .map(|w| w.min(44));
        self.session_entries = entries;
        self.picker_rows = rows;
        if let Some(dialog) = self.dialog.as_mut() {
            let clamped = dialog.selected.min(items.len().saturating_sub(1));
            dialog.items = items;
            dialog.selected = clamped;
        }
    }

    /// Load a persisted session, its messages, and its todos by id from the bound store, or `None`
    /// when the session does not exist. Best-effort failures surface as an `Err(String)`.
    fn load_persisted_session(&self, id: &str) -> Result<Option<LoadedSession>, String> {
        let database = self
            .open_store()
            .ok_or_else(|| "no session store is available".to_owned())?;
        let session = match database.sessions().get(id).map_err(|err| err.to_string())? {
            Some(session) => session,
            None => return Ok(None),
        };
        let messages = database
            .messages()
            .list_for_session(id)
            .map_err(|err| err.to_string())?;
        let usage = database
            .usage()
            .session_totals(id)
            .map_err(|err| err.to_string())?;
        let todos = database
            .todos()
            .list_for_session(id)
            .map_err(|err| err.to_string())?;
        Ok(Some(LoadedSession {
            session,
            messages,
            usage,
            todos,
        }))
    }

    /// Rename a persisted session (set or clear its title) by id in the bound store.
    fn rename_persisted_session(&self, id: &str, title: Option<&str>) -> Result<(), String> {
        let database = self
            .open_store()
            .ok_or_else(|| "no session store is available".to_owned())?;
        database
            .sessions()
            .set_title(id, title)
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    /// Delete a persisted session by id from the bound store, returning whether a row was removed.
    fn delete_persisted_session(&self, id: &str) -> Result<bool, String> {
        let database = self
            .open_store()
            .ok_or_else(|| "no session store is available".to_owned())?;
        database
            .sessions()
            .delete(id)
            .map_err(|err| err.to_string())
    }

    /// Switch the active model to the highlighted row of the open model picker, updating the footer
    /// status. Returns the new model id when it actually changed (so the caller persists it), or
    /// `None` when the dialog is not the model picker or the highlighted model is already active.
    fn select_model(&mut self) -> Option<String> {
        let dialog = self.dialog.as_ref()?;
        if dialog.kind != DialogKind::Models {
            return None;
        }
        let model = self.model_rows.get(dialog.selected())?.clone()?;
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

    /// Commit the skill picker's working (pending) enabled set as the active enabled-skill set,
    /// surfacing a notice of the applied set when it changed. Also mirrors the highlighted skill
    /// into `selected_skill` so the status view and next open stay consistent. A no-op unless the
    /// skill picker is open.
    fn commit_skill_toggles(&mut self) {
        if self.dialog.as_ref().map(|dialog| dialog.kind) != Some(DialogKind::Skills) {
            return;
        }
        if let Some(idx) = self.dialog.as_ref().map(Dialog::selected) {
            if let Some(name) = self.skill_rows.get(idx).cloned() {
                self.selected_skill = Some(name);
            }
        }
        let Some(pending) = self.skill_pending.take() else {
            return;
        };
        if pending == self.enabled_skills {
            return;
        }
        self.enabled_skills = pending;
        let notice = if self.enabled_skills.is_empty() {
            "Skills: none enabled".to_owned()
        } else {
            format!(
                "Skills enabled: {}",
                self.enabled_skills
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        self.transcript.push(Entry::Notice(notice));
    }

    /// Select the highlighted variant row from the variant picker.
    fn select_variant(&mut self) -> Option<String> {
        let dialog = self.dialog.as_ref()?;
        if dialog.kind != DialogKind::Variants {
            return None;
        }
        let variant = self.variant_rows.get(dialog.selected())?.clone();
        if variant == self.status.variant {
            return None;
        }
        self.status.variant = variant.clone();
        Some(variant)
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

    fn selected_timeline_index(&self) -> Option<usize> {
        let dialog = self.dialog.as_ref()?;
        if !matches!(dialog.kind, DialogKind::Fork) {
            return None;
        }
        if self.timeline_entries.is_empty() {
            return None;
        }
        Some(dialog.selected().min(self.timeline_entries.len() - 1))
    }

    fn append_loaded_message(&mut self, message: &db::Message) {
        let media_parts = extract_message_content_parts(&message.parts);
        let attachments = attachments_from_message_parts(&message.parts);
        match message.role.as_str() {
            "user" => {
                self.transcript.push(Entry::User(render_user_entry(
                    &message.content,
                    &attachments,
                )));
                self.history
                    .push(Message::user(message.content.clone()).with_parts(media_parts));
            }
            "assistant" => {
                self.transcript.push(Entry::Assistant(AssistantTurn {
                    reasoning: String::new(),
                    answer: message.content.clone(),
                }));
                self.history
                    .push(Message::assistant(message.content.clone()).with_parts(media_parts));
            }
            _ => {}
        }
    }

    fn append_loaded_todos(&mut self, todos: &[protocol::TodoItem]) {
        if todos.is_empty() {
            return;
        }
        let mut text = String::from("Todos");
        for (index, item) in todos.iter().enumerate() {
            text.push_str(&format!(
                "\n{}. [{}][{}] {}",
                index + 1,
                item.status.as_str(),
                item.priority.as_str(),
                item.content
            ));
        }
        self.transcript.push(Entry::Notice(text));
    }

    /// Load a persisted session's transcript into the view (`Enter` on a session-picker row): the
    /// stored user/assistant messages replace the current transcript and projected history, and the
    /// footer's session label updates. Ignored while a turn streams; a read failure surfaces a
    /// clear error rather than crashing.
    fn load_session(&mut self, id: &str) {
        if self.is_busy() {
            return;
        }
        let session = match self.load_persisted_session(id) {
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
        self.expanded_tool_ids.clear();
        self.seen_message_ids.clear();
        self.composer_attachments.clear();
        self.pending_attachments.clear();
        self.turn_attachments.clear();
        self.usage = UsageSummary::from_totals(session.usage);
        for message in &session.messages {
            self.seen_message_ids.insert(message.id.clone());
            self.append_loaded_message(message);
        }
        // Adopt the loaded session as the active durable session so subsequent turns append to it,
        // and honor its persisted model choice (if any) so the footer and the next turn match what
        // a headless resume would use.
        self.session_id = Some(session.session.id.clone());
        if let Some(model) = session
            .session
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
        {
            self.status.model = model.to_owned();
        }
        self.status.session = session.session.id.clone();
        self.transcript.push(Entry::Notice(format!(
            "Switched to session {}",
            session_picker_label(&session.session)
        )));
        self.append_loaded_todos(&session.todos);
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
        match self.rename_persisted_session(&id, title) {
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
        match self.delete_persisted_session(&id) {
            Ok(_) => {
                self.refresh_sessions_dialog();
                self.transcript
                    .push(Entry::Notice(format!("Deleted session {id}")));
            }
            Err(err) => self.note_error(format!("Delete failed: {err}")),
        }
    }

    /// Rebuild the open session picker's rows from the store (after a rename/delete), re-applying the
    /// active search filter and keeping the highlight on a valid row. A no-op unless the session
    /// picker is open.
    fn refresh_sessions_dialog(&mut self) {
        if !matches!(
            self.dialog.as_ref().map(|dialog| dialog.kind),
            Some(DialogKind::Sessions)
        ) {
            return;
        }
        self.session_all = self.build_session_picks();
        self.rebuild_session_view();
    }

    /// The in-progress session-rename text, when the session picker is in rename mode (for the UI).
    #[must_use]
    pub fn session_rename(&self) -> Option<&str> {
        self.session_rename.as_deref()
    }

    /// The filtered rich rows (key/desc/meta) for the open searchable picker (session/model), for
    /// the renderer. Empty for non-searchable dialogs.
    #[must_use]
    pub fn picker_rows(&self) -> &[crate::modal::ModalRow] {
        &self.picker_rows
    }

    /// The current search query typed into the open searchable picker (session/model).
    #[must_use]
    pub fn picker_search_query(&self) -> &str {
        &self.picker_query
    }

    /// The fixed key-column width for the open searchable picker so descriptions line up.
    #[must_use]
    pub fn picker_key_width(&self) -> Option<usize> {
        self.picker_key_width
    }

    /// Take the `/editor` request (if any), clearing it so the editor opens exactly once.
    #[must_use]
    pub fn take_editor_request(&mut self) -> bool {
        std::mem::take(&mut self.editor_requested)
    }

    /// Replace the prompt contents (used to land editor-composed text back in the prompt).
    pub fn set_input_value(&mut self, value: &str) {
        self.input.set_value(value);
        self.on_prompt_edited();
    }

    /// Surface a transient error in the transcript and as a distinct error toast (used when the
    /// external editor fails, a session action fails, or `/export` cannot write).
    pub fn note_error(&mut self, message: String) {
        let message = redaction::redact(&message);
        self.push_toast(ToastLevel::Error, message.clone());
        self.transcript.push(Entry::Error(message));
        self.emit_attention(AttentionEvent::Error);
    }

    /// Run a leader keybind action: the same visible effects the matching slash commands produce,
    /// reached via the leader chord instead of the prompt.
    fn run_keybind_action(&mut self, action: KeybindAction) {
        if let Some(reason) = self.keybind_action_disabled_reason(action) {
            self.push_toast(ToastLevel::Info, reason);
            return;
        }
        match action {
            KeybindAction::NewSession => self.new_session(),
            KeybindAction::ModelList => self.open_models_dialog(),
            KeybindAction::ToggleSidebar => self.toggle_sidebar(),
            KeybindAction::Sessions => self.open_sessions_dialog(),
            KeybindAction::Agents => self.open_agents_dialog(),
            KeybindAction::Themes => self.open_themes_dialog(),
            KeybindAction::Undo => self.undo(),
            KeybindAction::Redo => self.redo(),
            KeybindAction::CommandPalette => self.open_palette(),
            KeybindAction::MessagePrev => self.select_prev_message(),
            KeybindAction::MessageNext => self.select_next_message(),
            KeybindAction::ModelCycleNext => self.cycle_model_next(),
            KeybindAction::AgentCycleNext => self.cycle_agent_next(),
            KeybindAction::VariantCycleNext => self.cycle_variant_next(),
            KeybindAction::PromptHistoryPrev => self.recall_prompt_history_prev(),
            KeybindAction::PromptHistoryNext => self.recall_prompt_history_next(),
            KeybindAction::PromptStashSave => self.stash_prompt(),
            KeybindAction::PromptStashRestore => self.restore_prompt_stash(),
            KeybindAction::ToggleToolDetails => self.toggle_tool_details(),
            KeybindAction::QueuePromoteNext => self.promote_next_queued_prompt(),
            KeybindAction::QueueDropOne => self.drop_one_queued_prompt(),
            KeybindAction::QueueClear => self.clear_queued_prompts(),
            KeybindAction::PaletteModels => self.open_palette_scope(PaletteScope::Models),
            KeybindAction::PaletteSkills => self.open_palette_scope(PaletteScope::Skills),
            KeybindAction::PaletteVariants => self.open_palette_scope(PaletteScope::Variants),
            KeybindAction::PaletteSubagents => self.open_palette_scope(PaletteScope::Subagents),
            KeybindAction::PaletteQueuedPrompts => {
                self.open_palette_scope(PaletteScope::QueuedPrompts)
            }
        }
    }

    fn keybind_action_disabled_reason(&self, action: KeybindAction) -> Option<String> {
        match action {
            KeybindAction::MessagePrev | KeybindAction::MessageNext => self
                .transcript
                .is_empty()
                .then_some("No transcript messages yet".to_owned()),
            KeybindAction::PromptHistoryPrev => self
                .prompt_history
                .is_empty()
                .then_some("Prompt history is empty".to_owned()),
            KeybindAction::PromptHistoryNext => self
                .prompt_history_nav
                .is_none()
                .then_some("No prompt-history recall is active".to_owned()),
            KeybindAction::PromptStashRestore => self
                .prompt_stash
                .is_none()
                .then_some("No stashed prompt draft".to_owned()),
            KeybindAction::QueuePromoteNext
            | KeybindAction::QueueDropOne
            | KeybindAction::QueueClear => self
                .queued_prompts
                .is_empty()
                .then_some("There are no queued prompts".to_owned()),
            KeybindAction::PaletteSubagents => self
                .subagent_statuses()
                .is_empty()
                .then_some("No subagent sessions are available".to_owned()),
            KeybindAction::ModelCycleNext => self
                .available_model_ids()
                .is_empty()
                .then_some("No models are available".to_owned()),
            KeybindAction::ToggleToolDetails => {
                (!self.has_tool_cards()).then_some("No tool cards are available".to_owned())
            }
            _ => None,
        }
    }

    /// Show or hide the session sidebar panel (the sidebar-toggle keybind).
    fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        if self.sidebar_visible && self.sidebar_panel == SidebarPanel::Files {
            self.enter_files_panel();
        }
    }

    fn set_sidebar_panel(&mut self, panel: SidebarPanel) {
        self.sidebar_visible = true;
        self.sidebar_panel = panel;
        if panel == SidebarPanel::Files {
            self.enter_files_panel();
        }
        self.push_toast(ToastLevel::Info, format!("Sidebar: {}", panel.title()));
    }

    /// Refresh the files tree and reset navigation state when the Files panel is (re)opened so a
    /// freshly-reached panel shows the current working tree with no selection bar (design-match).
    fn enter_files_panel(&mut self) {
        self.files_cursor = 0;
        self.files_nav_active = false;
        self.files_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.rebuild_file_tree();
    }

    fn cycle_sidebar_panel(&mut self, forward: bool) {
        let tabs = SidebarPanel::ALL;
        let index = tabs
            .iter()
            .position(|panel| *panel == self.sidebar_panel)
            .unwrap_or(0);
        let next = if forward {
            (index + 1) % tabs.len()
        } else {
            (index + tabs.len() - 1) % tabs.len()
        };
        self.set_sidebar_panel(tabs[next]);
    }

    /// A plain-text projection of the `context` panel body (the styled rendering lives in `ui`);
    /// each file as `<name>  <tokens>`, followed by the add-files affordance.
    fn sidebar_context_rows(&self) -> Vec<String> {
        let mut rows: Vec<String> = self
            .context_files()
            .into_iter()
            .map(|file| format!("{} {}  {}", glyphs::PROMPT, file.name, file.tokens))
            .collect();
        rows.push("@ add files to context".to_owned());
        rows
    }

    /// A plain-text projection of the `files` tree body (the styled rendering lives in `ui`); each
    /// row indented by `depth*2` with its disclosure/modified glyph, or an empty-state line.
    fn sidebar_files_rows(&self) -> Vec<String> {
        let tree = self.file_tree_rows();
        if tree.is_empty() {
            return vec!["No files to show.".to_owned()];
        }
        tree.iter()
            .map(|row| {
                let indent = " ".repeat(row.depth * 2);
                let marker = match row.kind {
                    FileTreeKind::DirExpanded => glyphs::DISCLOSURE_EXPANDED,
                    FileTreeKind::DirCollapsed => glyphs::DISCLOSURE_COLLAPSED,
                    FileTreeKind::FileModified => glyphs::DOT_FILLED,
                    FileTreeKind::File => " ",
                };
                format!("{indent}{marker} {}", row.name)
            })
            .collect()
    }

    /// The cached, flattened `files`-panel tree (built on panel select / toggle). Callers render or
    /// navigate over this snapshot; it never touches the filesystem on a bare redraw.
    #[must_use]
    pub fn file_tree_rows(&self) -> &[FileTreeRow] {
        &self.files_tree
    }

    /// The focused files-tree row index (the toggle target), clamped to the current rows.
    #[must_use]
    pub fn files_cursor(&self) -> usize {
        self.files_cursor
            .min(self.files_tree.len().saturating_sub(1))
    }

    /// Whether a files-tree selection bar should be drawn (only after the user starts navigating,
    /// so a freshly-opened panel design-matches the reference with no highlight).
    #[must_use]
    pub fn files_nav_active(&self) -> bool {
        self.files_nav_active && !self.files_tree.is_empty()
    }

    /// Rebuild the cached files tree from the current working directory, honoring the user's
    /// expand/collapse overrides (and auto-expanding the top level plus any directory on the path
    /// to a git-modified file so the default view shows both disclosure states + `●` markers).
    fn rebuild_file_tree(&mut self) {
        let root = self.files_root.clone();
        let modified = git_modified_paths(&root);
        let mut rows = Vec::new();
        self.walk_file_tree(&root, 0, &modified, &mut rows);
        self.files_tree = rows;
        let len = self.files_tree.len();
        if len == 0 {
            self.files_cursor = 0;
        } else if self.files_cursor >= len {
            self.files_cursor = len - 1;
        }
    }

    /// Recursively append the sorted (directories first) children of `dir` to `rows`, descending
    /// only into directories that are currently expanded.
    fn walk_file_tree(
        &self,
        dir: &Path,
        depth: usize,
        modified: &BTreeSet<PathBuf>,
        rows: &mut Vec<FileTreeRow>,
    ) {
        // Bound the tree so a deep repo never floods the ~24-row panel with thousands of rows.
        if depth > 12 || rows.len() > 400 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_ignored_tree_entry(&name) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                dirs.push((name, path));
            } else {
                files.push((name, path));
            }
        }
        dirs.sort_by_key(|entry| entry.0.to_ascii_lowercase());
        files.sort_by_key(|entry| entry.0.to_ascii_lowercase());

        for (name, path) in dirs {
            let expanded = self.dir_is_expanded(&path, depth, modified);
            rows.push(FileTreeRow {
                depth,
                name: format!("{name}/"),
                kind: if expanded {
                    FileTreeKind::DirExpanded
                } else {
                    FileTreeKind::DirCollapsed
                },
                path: path.clone(),
            });
            if expanded {
                self.walk_file_tree(&path, depth + 1, modified, rows);
            }
        }
        for (name, path) in files {
            let kind = if modified.contains(&path) {
                FileTreeKind::FileModified
            } else {
                FileTreeKind::File
            };
            rows.push(FileTreeRow {
                depth,
                name,
                kind,
                path,
            });
        }
    }

    /// Whether directory `path` should render expanded: an explicit user override wins, else the
    /// default (top-level directories, and any directory on the path to a modified file).
    fn dir_is_expanded(&self, path: &Path, depth: usize, modified: &BTreeSet<PathBuf>) -> bool {
        if self.files_expanded.contains(path) {
            return true;
        }
        if self.files_collapsed.contains(path) {
            return false;
        }
        depth == 0 || modified.iter().any(|m| m.starts_with(path))
    }

    /// Move the files-tree cursor down (next visible row), activating the selection bar.
    fn files_tree_select_next(&mut self) {
        if self.files_tree.is_empty() {
            return;
        }
        self.files_nav_active = true;
        let last = self.files_tree.len() - 1;
        self.files_cursor = (self.files_cursor.min(last) + 1).min(last);
    }

    /// Move the files-tree cursor up (previous visible row), activating the selection bar.
    fn files_tree_select_prev(&mut self) {
        if self.files_tree.is_empty() {
            return;
        }
        self.files_nav_active = true;
        self.files_cursor = self
            .files_cursor
            .min(self.files_tree.len() - 1)
            .saturating_sub(1);
    }

    /// Toggle the focused directory node between expanded and collapsed, then rebuild the tree.
    /// Returns whether a directory was toggled (a leaf file is a no-op).
    fn files_tree_toggle_selected(&mut self) -> bool {
        let Some(row) = self.files_tree.get(self.files_cursor()).cloned() else {
            return false;
        };
        if !row.is_dir() {
            return false;
        }
        self.files_nav_active = true;
        match row.kind {
            FileTreeKind::DirExpanded => {
                self.files_expanded.remove(&row.path);
                self.files_collapsed.insert(row.path.clone());
            }
            FileTreeKind::DirCollapsed => {
                self.files_collapsed.remove(&row.path);
                self.files_expanded.insert(row.path.clone());
            }
            _ => return false,
        }
        self.rebuild_file_tree();
        true
    }

    fn sidebar_lsp_rows(&self) -> Vec<String> {
        if self.sidebar_lsp_groups.is_empty() {
            return vec!["No recent LSP diagnostics.".to_owned()];
        }
        let mut rows = Vec::new();
        for group in &self.sidebar_lsp_groups {
            rows.push(group.file.clone());
            if group.diagnostics.is_empty() {
                rows.push("no problems".to_owned());
            } else {
                for diagnostic in &group.diagnostics {
                    let letter = if diagnostic.is_error { "E" } else { "W" };
                    rows.push(format!(
                        "{} {letter} {}",
                        diagnostic.line, diagnostic.message
                    ));
                }
            }
        }
        rows
    }

    fn sidebar_mcp_rows(&self) -> Vec<String> {
        let rows = self.mcp_rows();
        if rows.is_empty() {
            vec!["No MCP servers configured.".to_owned()]
        } else {
            rows
        }
    }

    fn sidebar_todo_rows(&self) -> Vec<String> {
        let Some(session_id) = self.session_id.as_deref() else {
            return vec!["No active persisted session.".to_owned()];
        };
        let Some(db) = self.open_store() else {
            return vec!["No session store is available.".to_owned()];
        };
        match db.todos().list_for_session(session_id) {
            Ok(items) if items.is_empty() => vec!["No todo items for this session.".to_owned()],
            Ok(items) => items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    format!(
                        "{}. [{}|{}] {}",
                        index + 1,
                        item.status.as_str(),
                        item.priority.as_str(),
                        item.content
                    )
                })
                .collect(),
            Err(err) => vec![format!("Todo state unavailable: {err}")],
        }
    }

    fn sidebar_subagent_rows(&self) -> Vec<String> {
        let statuses = self.subagent_statuses();
        if statuses.is_empty() {
            return vec!["No subagent sessions for this parent.".to_owned()];
        }
        statuses
            .into_iter()
            .map(|(_, label, state, detail)| format!("[{state}] {label} — {detail}"))
            .collect()
    }

    /// The active session's todo items for the sidebar `todo` panel renderer, in list order. Empty
    /// when there is no persisted session, no store, or the list is empty/unreadable — the renderer
    /// then shows a muted placeholder. Real/dynamic session state.
    #[must_use]
    pub fn sidebar_todo_items(&self) -> Vec<protocol::TodoItem> {
        let Some(session_id) = self.session_id.as_deref() else {
            return Vec::new();
        };
        let Some(db) = self.open_store() else {
            return Vec::new();
        };
        db.todos().list_for_session(session_id).unwrap_or_default()
    }

    /// The active session's child subagents for the sidebar `subagents` panel renderer: each row
    /// carries the agent name, its task description, a status word with elapsed time, and a
    /// semantic [`SubagentState`] driving the status color. Empty when there is no active session
    /// or no children.
    #[must_use]
    pub fn sidebar_subagent_entries(&self) -> Vec<SidebarSubagentRow> {
        self.subagent_picks()
            .into_iter()
            .map(|pick| SidebarSubagentRow {
                name: pick.name,
                task: pick.task,
                status: pick.status,
                state: match pick.state {
                    "running" => SubagentState::Running,
                    "done" => SubagentState::Done,
                    "error" => SubagentState::Error,
                    _ => SubagentState::Queued,
                },
            })
            .collect()
    }

    /// The queued prompts (in run order) for the sidebar `queue` panel renderer: the display text
    /// of each queued message, the first being the run-next prompt. Each is collapsed to its first
    /// line so a multi-line prompt renders as one numbered row. Empty when nothing is queued.
    #[must_use]
    pub fn sidebar_queue_entries(&self) -> Vec<String> {
        self.queued_prompts
            .iter()
            .map(|queued| first_line(queued.header.as_deref().unwrap_or(&queued.prompt)))
            .collect()
    }

    fn sidebar_queue_rows(&self) -> Vec<String> {
        if self.queued_prompts.is_empty() {
            return vec!["No queued prompts.".to_owned()];
        }
        self.queued_prompts
            .iter()
            .enumerate()
            .map(|(index, queued)| {
                let text = queued
                    .header
                    .as_deref()
                    .unwrap_or(&queued.prompt)
                    .chars()
                    .take(80)
                    .collect::<String>();
                format!("{}. {}", index + 1, text)
            })
            .collect()
    }

    fn subagent_statuses(&self) -> Vec<(String, String, &'static str, String)> {
        let Some(db) = self.open_store() else {
            return Vec::new();
        };
        let Some(parent_id) = self.session_id.as_deref() else {
            return Vec::new();
        };
        let Ok(sessions) = db.sessions().children(parent_id) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for session in sessions {
            let label = session_picker_label(&session);
            if !label.contains("subagent:") {
                continue;
            }
            let messages = db
                .messages()
                .list_for_session(&session.id)
                .unwrap_or_default();
            let last = messages
                .iter()
                .rev()
                .find(|message| message.role == "assistant")
                .map(|message| message.content.clone());
            let state = if last
                .as_deref()
                .is_some_and(|text| text.to_ascii_lowercase().contains("state: error"))
            {
                "error"
            } else if last.is_some() {
                "done"
            } else if messages.is_empty() {
                "queued"
            } else {
                "running"
            };
            let detail = last
                .as_deref()
                .map(first_line)
                .unwrap_or_else(|| "awaiting assistant result".to_owned());
            rows.push((session.id, label, state, detail));
        }
        rows
    }

    fn available_model_ids(&self) -> Vec<String> {
        let effective = effective_config();
        let provider_config = effective
            .as_ref()
            .and_then(|cfg| providers::ProviderConfigSet::from_table(cfg.table()).ok());
        let merged_catalogue = provider_config.as_ref().map_or_else(
            || providers::Catalogue::embedded().clone(),
            |cfg| cfg.merged_catalogue(providers::Catalogue::embedded()),
        );
        let mut model_ids = Vec::new();
        for provider in merged_catalogue.providers() {
            let enabled = provider_config
                .as_ref()
                .is_none_or(|cfg| cfg.provider_enabled(&provider.id));
            if !enabled {
                continue;
            }
            model_ids.extend(provider.models.values().map(|model| model.id.clone()));
        }
        model_ids.sort();
        model_ids.dedup();
        model_ids
    }

    fn cycle_model_next(&mut self) {
        let models = self.available_model_ids();
        if models.is_empty() {
            self.push_toast(ToastLevel::Info, "No models are available");
            return;
        }
        let idx = models
            .iter()
            .position(|model| model == &self.status.model)
            .unwrap_or(0);
        let next = models[(idx + 1) % models.len()].clone();
        if next != self.status.model {
            self.status.model = next.clone();
            self.persist_config_string("model", &next);
            self.persist_session_model(&next);
            self.transcript
                .push(Entry::Notice(format!("Model set to {next}")));
        }
    }

    fn cycle_agent_next(&mut self) {
        if Agent::ALL.is_empty() {
            self.push_toast(ToastLevel::Info, "No agents are available");
            return;
        }
        let idx = Agent::ALL
            .iter()
            .position(|agent| agent.name() == self.status.agent)
            .unwrap_or(0);
        let next = Agent::ALL[(idx + 1) % Agent::ALL.len()].name().to_owned();
        if next != self.status.agent {
            self.status.agent = next.clone();
            self.persist_config_string("agent", &next);
            self.transcript
                .push(Entry::Notice(format!("Agent set to {next}")));
        }
    }

    fn cycle_variant_next(&mut self) {
        let idx = VARIANT_OPTIONS
            .iter()
            .position(|variant| *variant == self.status.variant)
            .unwrap_or(0);
        let next = VARIANT_OPTIONS[(idx + 1) % VARIANT_OPTIONS.len()].to_owned();
        if next != self.status.variant {
            self.status.variant = next.clone();
            self.persist_config_string("variant", &next);
            self.transcript
                .push(Entry::Notice(format!("Variant set to {next}")));
        }
    }

    fn stash_prompt(&mut self) {
        self.prompt_stash = Some(PromptDraft {
            text: self.input.value(),
            cursor: self.input.cursor_position(),
        });
        self.push_toast(ToastLevel::Success, "Prompt draft stashed");
    }

    fn restore_prompt_stash(&mut self) {
        let Some(stashed) = self.prompt_stash.clone() else {
            self.push_toast(ToastLevel::Info, "No stashed prompt draft");
            return;
        };
        self.input.set_value(&stashed.text);
        self.input.set_cursor_position(stashed.cursor);
        self.prompt_stash = None;
        self.prompt_history_nav = None;
        self.on_prompt_edited();
        self.push_toast(ToastLevel::Success, "Restored stashed prompt");
    }

    fn frecency_ranked_prompts(&self) -> Vec<String> {
        let mut entries = self
            .prompt_history
            .iter()
            .map(|(prompt, stats)| (prompt.clone(), *stats))
            .collect::<Vec<_>>();
        entries.sort_by(|(lhs_prompt, lhs), (rhs_prompt, rhs)| {
            rhs.count
                .cmp(&lhs.count)
                .then(rhs.last_used.cmp(&lhs.last_used))
                .then(lhs_prompt.cmp(rhs_prompt))
        });
        entries.into_iter().map(|(prompt, _)| prompt).collect()
    }

    fn record_prompt_history(&mut self, prompt: &str) {
        let normalized = prompt.trim();
        if normalized.is_empty() {
            return;
        }
        self.prompt_history_tick = self.prompt_history_tick.saturating_add(1);
        let entry = self
            .prompt_history
            .entry(normalized.to_owned())
            .or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last_used = self.prompt_history_tick;
    }

    fn begin_prompt_history_nav(&mut self) {
        let ordered = self.frecency_ranked_prompts();
        if ordered.is_empty() {
            self.push_toast(ToastLevel::Info, "Prompt history is empty");
            return;
        }
        let stashed = PromptDraft {
            text: self.input.value(),
            cursor: self.input.cursor_position(),
        };
        self.prompt_history_nav = Some(PromptHistoryNavigation {
            ordered,
            index: 0,
            stashed,
        });
        self.apply_prompt_history_nav();
    }

    fn apply_prompt_history_nav(&mut self) {
        let Some(nav) = self.prompt_history_nav.as_ref() else {
            return;
        };
        let Some(prompt) = nav.ordered.get(nav.index) else {
            return;
        };
        self.input.set_value(prompt);
        self.input
            .set_cursor_position(CursorPosition::new(0, prompt.chars().count()));
        self.on_prompt_edited();
    }

    fn recall_prompt_history_prev(&mut self) {
        if self.prompt_history_nav.is_none() {
            self.begin_prompt_history_nav();
            return;
        }
        if let Some(nav) = self.prompt_history_nav.as_mut() {
            if nav.index + 1 < nav.ordered.len() {
                nav.index += 1;
            }
        }
        self.apply_prompt_history_nav();
    }

    fn recall_prompt_history_next(&mut self) {
        let Some(nav) = self.prompt_history_nav.as_mut() else {
            self.push_toast(ToastLevel::Info, "No prompt-history recall is active");
            return;
        };
        if nav.index > 0 {
            nav.index -= 1;
            self.apply_prompt_history_nav();
            return;
        }
        let restore = nav.stashed.clone();
        self.prompt_history_nav = None;
        self.input.set_value(&restore.text);
        self.input.set_cursor_position(restore.cursor);
        self.on_prompt_edited();
    }

    fn promote_next_queued_prompt(&mut self) {
        if self.queued_prompts.is_empty() {
            self.push_toast(ToastLevel::Info, "There are no queued prompts");
            return;
        }
        if self.queued_prompts.len() == 1 {
            self.push_toast(ToastLevel::Info, "Only one queued prompt is present");
            return;
        }
        if let Some(prompt) = self.queued_prompts.pop_back() {
            self.queued_prompts.push_front(prompt);
        }
        self.push_toast(ToastLevel::Success, "Queued prompt promoted");
    }

    fn drop_one_queued_prompt(&mut self) {
        if self.queued_prompts.pop_back().is_some() {
            self.push_toast(ToastLevel::Success, "Dropped one queued prompt");
        } else {
            self.push_toast(ToastLevel::Info, "There are no queued prompts");
        }
    }

    fn clear_queued_prompts(&mut self) {
        let cleared = self.queued_prompts.len();
        self.queued_prompts.clear();
        self.push_toast(
            ToastLevel::Success,
            format!("Cleared {cleared} queued prompt(s)"),
        );
    }

    /// Start a fresh session (`/new`): clear the transcript, the conversational history, and the
    /// usage totals so the view returns to the empty home state. Ignored while a turn streams.
    fn new_session(&mut self) {
        if self.is_busy() {
            return;
        }
        self.transcript.clear();
        self.history.clear();
        self.expanded_tool_ids.clear();
        self.seen_message_ids.clear();
        self.usage = UsageSummary::default();
        self.composer_attachments.clear();
        self.pending_attachments.clear();
        self.turn_attachments.clear();
        self.queued_prompts.clear();
        // Detach from the current durable session so the next turn creates a fresh one rather than
        // appending to the prior conversation; the footer returns to the "new" placeholder until it
        // is created.
        self.session_id = None;
        self.status.session = "new".to_owned();
        self.reset_view();
    }

    /// Fork the selected timeline point into a new session and switch to it.
    fn fork_from_timeline_selection(&mut self) {
        let Some(selected) = self.selected_timeline_index() else {
            self.note_error("No timeline point selected for forking.".to_owned());
            return;
        };
        let Some(source) = self.timeline_entries.get(selected).cloned() else {
            self.note_error("No timeline point selected for forking.".to_owned());
            return;
        };
        let Some(db) = self.open_store() else {
            self.note_error("No session store is available for forking.".to_owned());
            return;
        };
        let messages = match db.messages().list_for_session(&source.session_id) {
            Ok(messages) => messages,
            Err(err) => {
                self.note_error(format!("Fork failed: {err}"));
                return;
            }
        };
        let Some(source_index) = messages
            .iter()
            .position(|message| message.id == source.message_id)
        else {
            self.note_error("Fork failed: selected timeline message no longer exists.".to_owned());
            return;
        };
        let title = format!("Fork of {} @ {}", source.session_id, source_index + 1);
        let forked = match db.sessions().create(Some(&title)) {
            Ok(session) => session,
            Err(err) => {
                self.note_error(format!("Fork failed: {err}"));
                return;
            }
        };
        for message in messages.iter().take(source_index + 1) {
            let persisted = if message.parts.is_empty() {
                db.messages()
                    .create(&forked.id, &message.role, &message.content)
            } else {
                let parts = message
                    .parts
                    .iter()
                    .cloned()
                    .map(|mut part| {
                        part.id.clear();
                        part.created_at = None;
                        part
                    })
                    .collect::<Vec<_>>();
                db.messages()
                    .create_with_parts(&forked.id, &message.role, &message.content, &parts)
            };
            if let Err(err) = persisted {
                self.note_error(format!("Fork failed: {err}"));
                return;
            }
        }
        self.load_session(&forked.id);
        self.transcript.push(Entry::Notice(format!(
            "Forked {} at point {} -> {}",
            source.session_id,
            source_index + 1,
            forked.id
        )));
    }

    /// Open the `/rename` dialog for the active session: a centered input pre-filled with the
    /// session's current title (quoted in the prompt), so the user can edit and save a new title.
    /// Surfaces a clear notice when no persisted session is active yet.
    fn open_rename_dialog(&mut self) {
        let Some(session_id) = self.session_id.clone() else {
            self.transcript.push(Entry::Notice(
                "No persisted session to rename yet.".to_owned(),
            ));
            self.push_toast(ToastLevel::Info, "No session to rename");
            return;
        };
        let current_title = self
            .open_store()
            .and_then(|db| db.sessions().get(&session_id).ok().flatten())
            .and_then(|session| session.title)
            .filter(|title| !title.trim().is_empty());
        // The input starts empty (the current title is shown for reference on its own line), so the
        // user types the new title fresh — matching the reference dialog.
        self.rename_dialog = Some(RenameDialog {
            session_id,
            current_title,
            buffer: String::new(),
        });
    }

    /// Whether the `/rename` dialog is open.
    #[must_use]
    pub fn rename_open(&self) -> bool {
        self.rename_dialog.is_some()
    }

    /// The open `/rename` dialog state, for the renderer.
    #[must_use]
    pub fn rename_dialog(&self) -> Option<&RenameDialog> {
        self.rename_dialog.as_ref()
    }

    /// Handle a key while the `/rename` dialog owns input: typing edits the new title, Enter commits
    /// the rename, Esc cancels, Backspace erases, and Ctrl+C still quits so the user is never trapped.
    fn on_rename_dialog_key(&mut self, key: KeyEvent, ctrl: bool) {
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Enter => self.commit_rename_dialog(),
            KeyCode::Esc => self.rename_dialog = None,
            KeyCode::Backspace => {
                if let Some(dialog) = self.rename_dialog.as_mut() {
                    dialog.buffer.pop();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(dialog) = self.rename_dialog.as_mut() {
                    dialog.buffer.push(c);
                }
            }
            _ => {}
        }
    }

    /// Commit the `/rename` dialog: persist the typed title to the active session (an empty title
    /// clears it, falling back to the session id as the label), confirm with a notice, and close.
    fn commit_rename_dialog(&mut self) {
        let Some(dialog) = self.rename_dialog.take() else {
            return;
        };
        let trimmed = dialog.buffer.trim();
        let title = (!trimmed.is_empty()).then_some(trimmed);
        match self.rename_persisted_session(&dialog.session_id, title) {
            Ok(()) => {
                let shown = title
                    .map(str::to_owned)
                    .unwrap_or_else(|| dialog.session_id.clone());
                self.transcript
                    .push(Entry::Notice(format!("Renamed session to {shown}")));
                self.push_toast(ToastLevel::Success, "Session renamed");
            }
            Err(err) => self.note_error(format!("Rename failed: {err}")),
        }
    }

    /// Open the `/compact` dialog: a centered before/after context comparison (over the dimmed
    /// conversation) shown before compaction runs, replacing the old silent compaction flow. A
    /// no-op (with a clear notice) when there is no conversation yet to compact. Confirming (Enter)
    /// runs the compaction; Esc cancels.
    fn open_compact_dialog(&mut self) {
        if self.is_busy() {
            return;
        }
        if self.history.is_empty() {
            self.transcript.push(Entry::Notice(
                "Nothing to compact yet - start a conversation first.".to_owned(),
            ));
            return;
        }
        self.compact_dialog = Some(self.build_compact_data());
    }

    /// Build the `/compact` dialog data from the real session state: the before-token count and
    /// message count, an estimated after size, and the kept-summary bullets drawn from the most
    /// recent user prompts (so the preview reflects the real conversation).
    fn build_compact_data(&self) -> CompactDialog {
        let messages = self.history.len();
        let before = self
            .usage
            .total_tokens
            .max(estimate_history_tokens(&self.history));
        // Compaction summarizes older messages into one summary while keeping the two most recent
        // exchanges; the after size is estimated as a fraction of the pre-compaction context and
        // never exceeds it, so the preview always reflects a reduction.
        let recent = messages.min(2);
        let after = (before / 5).max(1).min(before);
        let keeps = self
            .transcript
            .iter()
            .rev()
            .filter_map(|entry| match entry {
                Entry::User(text) => Some(first_line(text)),
                _ => None,
            })
            .filter(|line| !line.trim().is_empty())
            .take(2)
            .collect::<Vec<_>>();
        let keeps = if keeps.is_empty() {
            vec!["the active task and its context".to_owned()]
        } else {
            keeps.into_iter().rev().collect()
        };
        CompactDialog {
            before_tokens: format!("{} tokens", crate::view::format_thousands(before)),
            before_detail: format!("{messages} message{}", if messages == 1 { "" } else { "s" }),
            after_tokens: format!("~{} tokens", crate::view::format_thousands(after)),
            after_detail: format!("1 summary + {recent} recent",),
            keeps,
        }
    }

    /// Whether the `/compact` dialog is open.
    #[must_use]
    pub fn compact_open(&self) -> bool {
        self.compact_dialog.is_some()
    }

    /// The open `/compact` dialog state, for the renderer.
    #[must_use]
    pub fn compact_dialog(&self) -> Option<&CompactDialog> {
        self.compact_dialog.as_ref()
    }

    /// Handle a key while the `/compact` dialog owns input: Enter confirms (runs the compaction),
    /// Esc cancels, and Ctrl+C still quits so the user is never trapped.
    fn on_compact_dialog_key(&mut self, key: KeyEvent, ctrl: bool) {
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Enter => self.confirm_compact_dialog(),
            KeyCode::Esc => self.compact_dialog = None,
            _ => {}
        }
    }

    /// Confirm the `/compact` dialog: close it and queue the compaction turn whose summary replaces
    /// the conversation history when it finishes.
    fn confirm_compact_dialog(&mut self) {
        self.compact_dialog = None;
        self.start_compaction();
    }

    /// Open the `/init` dialog: a centered scan log (over the dimmed conversation) reflecting the
    /// real project analysis, shown before AGENTS.md is written, replacing the old silent file
    /// write. Confirming (Enter) runs the project-initialization flow; Esc cancels.
    fn open_init_dialog(&mut self) {
        if self.is_busy() {
            return;
        }
        self.init_dialog = Some(build_init_scan());
    }

    /// Whether the `/init` dialog is open.
    #[must_use]
    pub fn init_open(&self) -> bool {
        self.init_dialog.is_some()
    }

    /// The open `/init` dialog state, for the renderer.
    #[must_use]
    pub fn init_dialog(&self) -> Option<&InitDialog> {
        self.init_dialog.as_ref()
    }

    /// Handle a key while the `/init` dialog owns input: Enter (or `e` edit-first) confirms and runs
    /// the initialization flow, Esc cancels, and Ctrl+C still quits so the user is never trapped.
    fn on_init_dialog_key(&mut self, key: KeyEvent, ctrl: bool) {
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Enter => self.confirm_init_dialog(false),
            KeyCode::Char('e') if !ctrl => self.confirm_init_dialog(true),
            KeyCode::Esc => self.init_dialog = None,
            _ => {}
        }
    }

    /// Confirm the `/init` dialog: close it and queue the real project-initialization flow that
    /// analyzes the repository and writes AGENTS.md. `edit_first` additionally requests the external
    /// editor once the file is created.
    fn confirm_init_dialog(&mut self, edit_first: bool) {
        self.init_dialog = None;
        self.start_flow(
            "/init",
            "/init - analyzing the repository to create AGENTS.md…",
            INIT_PROMPT,
        );
        if edit_first {
            self.editor_requested = true;
        }
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

    /// Append a character to the find query and refresh the matches/highlight. Typing always returns
    /// the bar to EDITING mode so a refinement after a commit re-highlights incrementally.
    fn find_input(&mut self, c: char) {
        if let Some(find) = self.find.as_mut() {
            find.query.push(c);
            find.active = 0;
            find.committed = false;
        }
        self.recompute_find();
    }

    /// Commit the current find query (Enter), switching from EDITING to NAV mode. Keeps the current
    /// matches/highlight and the active (first) match so `n`/`N` can step through them.
    fn commit_find(&mut self) {
        if let Some(find) = self.find.as_mut() {
            find.committed = true;
        }
    }

    /// Delete the last character of the find query and refresh the matches/highlight. Editing the
    /// query always returns the bar to EDITING mode.
    fn find_backspace(&mut self) {
        if let Some(find) = self.find.as_mut() {
            find.query.pop();
            find.active = 0;
            find.committed = false;
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

    /// Flush any buffered Esc-probe chars back into the prompt so incomplete or unrecognized
    /// probes never discard typed input.
    fn flush_nav_escape_probe_text(&mut self) {
        let Some(mut probe) = self.nav_escape_probe.take() else {
            return;
        };
        if probe.chars.is_empty() {
            return;
        }
        for ch in probe.chars.drain(..) {
            self.input.insert_char(ch);
        }
        self.on_prompt_edited();
    }

    /// Finalize the active Esc probe: consume genuine vertical-arrow residue as inert, otherwise
    /// flush the buffered bytes back into the prompt.
    fn finalize_nav_escape_probe(&mut self) {
        let state = self
            .nav_escape_probe
            .as_ref()
            .map(|probe| nav_escape_probe_state(&probe.chars));
        match state {
            Some(NavEscapeProbeState::CompleteVerticalArrow) => self.restore_nav_escape_selection(),
            Some(_) => self.flush_nav_escape_probe_text(),
            None => {}
        }
    }

    /// Start an Esc-prefix probe in prompt mode. Used to detect noisy Alt+arrow encodings that
    /// arrive as Esc plus stray bytes/arrow keys, so they can be treated as inert.
    fn arm_nav_escape_probe(&mut self) {
        self.nav_escape_probe = Some(NavEscapeProbe {
            started: Instant::now(),
            previous_selection: self.selected_message,
            chars: String::new(),
        });
    }

    /// Expire the probe once its short detection window elapses.
    fn expire_nav_escape_probe(&mut self) {
        if self
            .nav_escape_probe
            .as_ref()
            .is_some_and(|probe| probe.started.elapsed() > NAV_ESCAPE_PROBE_WINDOW)
        {
            self.finalize_nav_escape_probe();
        }
    }

    /// Restore the selection captured before Esc and clear the active probe.
    fn restore_nav_escape_selection(&mut self) {
        if let Some(probe) = self.nav_escape_probe.take() {
            self.selected_message = probe.previous_selection;
        }
    }

    /// Consume a possible Esc-prefixed Up/Down key and treat it as inert.
    fn consume_nav_escape_probe_arrow(&mut self, code: KeyCode) -> bool {
        if !matches!(code, KeyCode::Up | KeyCode::Down) {
            return false;
        }
        self.expire_nav_escape_probe();
        if self.nav_escape_probe.is_none() {
            return false;
        }
        self.restore_nav_escape_selection();
        true
    }

    /// Consume possible Esc-prefixed CSI fragments and restore selection once the fragment
    /// identifies a vertical arrow (`[A`, `A[`, `OA`, etc.).
    fn consume_nav_escape_probe_char(&mut self, c: char) -> bool {
        self.expire_nav_escape_probe();
        let Some(probe) = self.nav_escape_probe.as_mut() else {
            return false;
        };
        probe.chars.push(c);
        let state = nav_escape_probe_state(&probe.chars);
        let overflow = probe.chars.len() >= NAV_ESCAPE_PROBE_MAX_CHARS;
        match state {
            NavEscapeProbeState::Candidate | NavEscapeProbeState::CompleteVerticalArrow => {
                if overflow {
                    self.finalize_nav_escape_probe();
                }
            }
            NavEscapeProbeState::NotCandidate => self.flush_nav_escape_probe_text(),
        }
        true
    }

    fn selected_or_latest_tool_card_id(&self) -> Option<String> {
        if let Some(index) = self.selected_message
            && let Some(Entry::Tool(card)) = self.transcript.get(index)
        {
            return Some(card.id.clone());
        }
        self.transcript.iter().rev().find_map(|entry| match entry {
            Entry::Tool(card) => Some(card.id.clone()),
            _ => None,
        })
    }

    fn has_tool_cards(&self) -> bool {
        self.transcript
            .iter()
            .any(|entry| matches!(entry, Entry::Tool(_)))
    }

    fn toggle_tool_details(&mut self) {
        let Some(id) = self.selected_or_latest_tool_card_id() else {
            self.push_toast(ToastLevel::Info, "No tool cards yet");
            return;
        };
        if !self.expanded_tool_ids.insert(id.clone()) {
            self.expanded_tool_ids.remove(&id);
        }
    }

    #[must_use]
    pub fn tool_card_expanded(&self, card: &ToolCard) -> bool {
        self.expanded_tool_ids.contains(&card.id)
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
        self.pending_attachments.clear();
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
        self.pending_attachments.clear();
        self.pending_compaction = true;
    }

    /// Open the export format picker (`/export`): a centered modal listing the available formats
    /// (Markdown/JSON/HTML/Patch) as selectable rows with the default format badged, rather than
    /// silently writing a file. The chosen format is written on confirm (see [`Self::perform_export`]).
    fn open_export_dialog(&mut self) {
        let theme = frame::style::ThemeColors::from_name(&self.theme);
        let mut items = Vec::with_capacity(EXPORT_FORMATS.len());
        let mut rows = Vec::with_capacity(EXPORT_FORMATS.len());
        let mut key_width = 0usize;
        for (index, (name, desc, _ext)) in EXPORT_FORMATS.iter().enumerate() {
            key_width = key_width.max(name.chars().count());
            let (meta, meta_color) = if index == 0 {
                (Some("default".to_owned()), Some(theme.secondary))
            } else {
                (None, None)
            };
            items.push((*name).to_owned());
            rows.push(crate::modal::ModalRow {
                key: (*name).to_owned(),
                desc: Some((*desc).to_owned()),
                meta,
                meta_color,
            });
        }
        self.picker_rows = rows;
        self.picker_key_width = (key_width > 0).then_some(key_width);
        self.open_dialog(Dialog {
            kind: DialogKind::Export,
            title: "Export session".to_owned(),
            items,
            selected: 0,
            empty: None,
        });
    }

    /// Export the current session in the highlighted format of the open export modal: render the
    /// transcript to that format, write it under the data directory's `exports/` folder, and confirm
    /// with the path. A write failure surfaces a clear error instead of crashing.
    fn perform_export(&mut self) {
        let index = self.dialog.as_ref().map_or(0, Dialog::selected);
        let (name, _desc, ext) = EXPORT_FORMATS
            .get(index)
            .copied()
            .unwrap_or(EXPORT_FORMATS[0]);
        let content = match ext {
            "json" => self.render_session_json(),
            "html" => self.render_session_html(),
            "patch" => self.render_session_patch(),
            _ => self.render_session_markdown(),
        };
        match write_export(&content, ext) {
            Ok(path) => {
                self.transcript.push(Entry::Notice(format!(
                    "Exported session ({name}) to {}",
                    path.display()
                )));
                self.push_toast(ToastLevel::Success, "Session exported");
            }
            Err(err) => self.note_error(format!("Export failed: {err}")),
        }
    }

    /// Render the current transcript to a structured JSON document for `/export` (role + content
    /// per message), mirroring the headless `cortex export` message shape.
    fn render_session_json(&self) -> String {
        let messages: Vec<serde_json::Value> = self
            .transcript
            .iter()
            .filter_map(|entry| match entry {
                Entry::User(text) => Some(serde_json::json!({"role": "user", "content": text})),
                Entry::Assistant(turn) => {
                    Some(serde_json::json!({"role": "assistant", "content": turn.answer}))
                }
                _ => None,
            })
            .collect();
        let document = serde_json::json!({
            "session": self.session_id,
            "messages": messages,
        });
        serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Render the current transcript to a self-contained HTML document for `/export`.
    fn render_session_html(&self) -> String {
        let mut body = String::new();
        for entry in &self.transcript {
            match entry {
                Entry::User(text) => body.push_str(&format!(
                    "<section class=\"user\"><h2>You</h2><pre>{}</pre></section>\n",
                    html_escape(text)
                )),
                Entry::Assistant(turn) => body.push_str(&format!(
                    "<section class=\"assistant\"><h2>Cortex</h2><pre>{}</pre></section>\n",
                    html_escape(turn.answer.trim())
                )),
                _ => {}
            }
        }
        format!(
            "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>Cortex Code session export</title>\n</head>\n<body>\n{body}</body>\n</html>\n"
        )
    }

    /// Render the current transcript's file changes to a git-applyable unified patch for `/export`.
    fn render_session_patch(&self) -> String {
        let mut out = String::new();
        for file in self.collect_session_diffs() {
            let header_path = &file.path;
            out.push_str(&format!("--- a/{header_path}\n+++ b/{header_path}\n"));
            for hunk in &file.hunks {
                out.push_str(&hunk.header());
                out.push('\n');
                for line in &hunk.lines {
                    let marker = match line.kind {
                        crate::diff::DiffLineKind::Added => '+',
                        crate::diff::DiffLineKind::Removed => '-',
                        _ => ' ',
                    };
                    out.push(marker);
                    out.push_str(&line.content);
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Collect the current transcript's file diffs (one per path, latest change wins), matching the
    /// `/diff` viewer's collection, for the patch export.
    fn collect_session_diffs(&self) -> Vec<crate::diff::FileDiff> {
        let mut files: Vec<crate::diff::FileDiff> = Vec::new();
        let mut seen_paths: BTreeSet<String> = BTreeSet::new();
        for entry in self.transcript.iter().rev() {
            let diff = match entry {
                Entry::Diff(diff) => Some(diff),
                Entry::Tool(card) => card.diff.as_deref(),
                _ => None,
            };
            if let Some(diff) = diff
                && seen_paths.insert(diff.path.clone())
            {
                files.push(diff.clone());
            }
        }
        files.reverse();
        files
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
                Entry::Blocked { command, reason } => {
                    out.push_str(&format!("> Blocked: `{command}` — {reason}\n\n"))
                }
                Entry::Error(text) => out.push_str(&format!("> Error: {text}\n\n")),
                Entry::TurnError(text) => out.push_str(&format!("> Error: {text}\n\n")),
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
        self.expire_nav_escape_probe();
        let plain_probe_key =
            !ctrl && !alt && matches!(key.code, KeyCode::Char(_) | KeyCode::Up | KeyCode::Down);
        if self
            .nav_escape_probe
            .as_ref()
            .is_some_and(|probe| !probe.chars.is_empty())
            && !plain_probe_key
        {
            self.finalize_nav_escape_probe();
        }

        // An interactive permission prompt is the most modal state: the turn worker is blocked
        // awaiting the decision, so it owns every key until answered. `y` approves once, `a` approves
        // and remembers ("always allow"), `n`/`d` deny, and while Deny is highlighted printable keys
        // are captured as corrective feedback text carried back to the running turn. Arrows move the
        // highlight, Enter confirms, and Esc denies (the turn then continues gracefully). Ctrl+C
        // still quits so the user is never trapped.
        if self.permission_open() {
            let is_question = self.permission_is_question();
            match key.code {
                KeyCode::Char('c') if ctrl => self.should_quit = true,
                KeyCode::Char(ch)
                    if !ctrl
                        && !alt
                        && self.pending_permission.as_ref().is_some_and(|pending| {
                            pending.selected == 2 || pending.ask.tool == "question"
                        }) =>
                {
                    self.permission_feedback_input(ch);
                }
                KeyCode::Char('y') if !ctrl && !alt && !is_question => {
                    self.answer_permission(ScopedDecision::once(Decision::Allow));
                }
                KeyCode::Char('a') if !ctrl && !alt && !is_question => {
                    self.answer_permission(ScopedDecision::always(Decision::Allow));
                }
                KeyCode::Char('n' | 'd') if !ctrl && !alt && !is_question => {
                    self.answer_permission(ScopedDecision::once(Decision::Deny));
                }
                KeyCode::Backspace => self.permission_feedback_backspace(),
                KeyCode::Left | KeyCode::Up => self.permission_step(false),
                KeyCode::Right | KeyCode::Down => self.permission_step(true),
                KeyCode::Enter => self.confirm_permission(),
                KeyCode::Esc => self.answer_permission(ScopedDecision::once(Decision::Deny)),
                _ => {}
            }
            return;
        }

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

        // The interactive auth/login screen takes over the frame while showing. It is modal: Esc
        // cancels out (without storing), Ctrl+C quits, and every other key drives the provider
        // picker / masked key field. It is checked before onboarding because it is launched
        // explicitly by `cortex auth login`, not as a first-run splash.
        if self.auth_login_active() {
            self.on_auth_login_key(key, ctrl);
            return;
        }

        // The mid-session `/connect` provider-auth flow takes over the frame while showing. It is
        // modal: Esc backs out a step (and closes the flow from the provider step), Ctrl+C quits,
        // and every other key drives the provider picker / method step / masked key field.
        if self.connect_active() {
            self.on_connect_key(key, ctrl);
            return;
        }

        // The first-run onboarding welcome screen takes over the frame while showing. It is a start
        // splash: Enter advances past it and Ctrl+C quits (both consumed here), while any other key
        // dismisses it and then falls through to act on the surface beneath, so the user's first
        // keystroke is never trapped.
        if self.onboarding_active() && self.on_onboarding_key(key, ctrl) {
            return;
        }

        // The Wave-7 themes-gallery screen takes over the frame while open: it is modal so the
        // prompt is not edited beneath it. Esc closes it back to the conversation; Ctrl+C quits.
        if self.theme_gallery_open() {
            self.on_theme_gallery_key(key, ctrl);
            return;
        }

        // A full-screen Wave-2 view (status/mcp/…) takes over the frame while open: it is modal so
        // the prompt is not edited beneath it. Esc closes it back to the conversation; Ctrl+C quits.
        if self.active_view_open() {
            self.on_view_key(key, ctrl);
            return;
        }

        // The `/diff` viewer is modal while open: it owns file/hunk navigation, mode toggles, and
        // targeted revert actions. Keys are swallowed so the prompt is never edited beneath it.
        if self.diff_viewer_open() {
            self.on_diff_viewer_key(key, ctrl);
            return;
        }

        // The `/rename` dialog is modal: it owns typing (editing the new title), Enter commits,
        // Esc cancels, and Ctrl+C still quits so the user is never trapped.
        if self.rename_open() {
            self.on_rename_dialog_key(key, ctrl);
            return;
        }

        // The `/compact` dialog is modal: Enter confirms (runs compaction), Esc cancels, and
        // Ctrl+C still quits so the user is never trapped.
        if self.compact_open() {
            self.on_compact_dialog_key(key, ctrl);
            return;
        }

        // The `/init` dialog is modal: Enter/`e` confirm (run the initialization flow), Esc cancels.
        if self.init_open() {
            self.on_init_dialog_key(key, ctrl);
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

        // The find-in-chat search bar is modal while open, with a committed-vs-editing (vim/less)
        // interaction so the `n next  N prev  esc close` hint is literally accurate WITHOUT shadowing
        // typed letters. EDITING (committed=false): every printable char — including `n`/`N` — types
        // into the query (incremental highlight) and Enter COMMITS. NAV (committed=true): `n` steps to
        // the next match, `N` to the previous (both wrapping), while Backspace or any other printable
        // char returns to EDITING and refines the query. Enter/Down step next and Up steps prev as
        // ergonomic extras; Ctrl+F or Esc close it in both modes, and Ctrl+C still quits.
        if self.find.is_some() {
            let committed = self.find.as_ref().is_some_and(|f| f.committed);
            match key.code {
                KeyCode::Char('c') if ctrl => self.should_quit = true,
                KeyCode::Char('f') if ctrl => self.close_find(),
                KeyCode::Esc => self.close_find(),
                KeyCode::Enter if shift => self.find_prev(),
                KeyCode::Enter if !committed => self.commit_find(),
                KeyCode::Enter | KeyCode::Down => self.find_next(),
                KeyCode::Up => self.find_prev(),
                KeyCode::Backspace => self.find_backspace(),
                KeyCode::Char('n') if committed && !ctrl && !alt => self.find_next(),
                KeyCode::Char('N') if committed && !ctrl && !alt => self.find_prev(),
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
        // the arrows move the highlight, Tab (`⇥ insert`, the advertised key) and Enter both insert
        // the highlighted file as an `@path` token, and Esc closes the picker without inserting.
        // Other keys fall through to ordinary editing, which re-filters (or closes) the picker.
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
                KeyCode::Tab | KeyCode::Enter if !shift => {
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

        // Files-tree navigation: while the Files sidebar panel is open and the composer is empty,
        // the arrows move a cursor through the tree and Enter/Space/Right/Left expand or collapse
        // the focused directory. Gating on an empty composer keeps ordinary text entry intact (all
        // these keys are no-ops on an empty single-line input), and works over any terminal.
        if self.sidebar_visible
            && self.sidebar_panel == SidebarPanel::Files
            && self.input.value().is_empty()
            && !ctrl
            && !alt
            && !self.slash_menu_open()
            && !self.mention_open()
        {
            match key.code {
                KeyCode::Up => {
                    self.files_tree_select_prev();
                    return;
                }
                KeyCode::Down => {
                    self.files_tree_select_next();
                    return;
                }
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right | KeyCode::Left => {
                    self.files_tree_toggle_selected();
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
            KeyCode::Char('o') if ctrl => self.toggle_tool_details(),
            // Ctrl+P opens the searchable command palette overlay.
            KeyCode::Char('p') if ctrl => self.open_palette(),
            // Ctrl+F opens find-in-chat (highlight + next/prev navigation) over the transcript.
            KeyCode::Char('f') if ctrl => self.open_find(),
            // Sidebar tabs: Ctrl+1..Ctrl+7 jump to a panel and show the sidebar.
            KeyCode::Char('1') if ctrl => self.set_sidebar_panel(SidebarPanel::Context),
            KeyCode::Char('2') if ctrl => self.set_sidebar_panel(SidebarPanel::Files),
            KeyCode::Char('3') if ctrl => self.set_sidebar_panel(SidebarPanel::Lsp),
            KeyCode::Char('4') if ctrl => self.set_sidebar_panel(SidebarPanel::Mcp),
            KeyCode::Char('5') if ctrl => self.set_sidebar_panel(SidebarPanel::Todo),
            KeyCode::Char('6') if ctrl => self.set_sidebar_panel(SidebarPanel::Subagents),
            KeyCode::Char('7') if ctrl => self.set_sidebar_panel(SidebarPanel::Queue),
            KeyCode::Char(']') if ctrl => self.cycle_sidebar_panel(true),
            KeyCode::Char('[') if ctrl => self.cycle_sidebar_panel(false),
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
            // `?` on an empty composer (not a text-entry mode) opens the help/command reference —
            // the global affordance the status bar advertises (VAL-FOUND-033). Typed into non-empty
            // text it is an ordinary character handled by the general arm below.
            KeyCode::Char('?') if !ctrl && !alt && self.input.value().is_empty() => {
                self.open_help_view();
            }
            KeyCode::Char('\t')
                if !ctrl && !alt && !self.slash_menu_open() && !self.mention_open() =>
            {
                self.trigger_text_completion();
            }
            // Ordinary text entry (ignore other ctrl/alt chords). Editing the prompt re-filters the
            // slash menu and the `@`-file picker, so refresh both after the edit.
            KeyCode::Char(c) if !ctrl && !alt => {
                if self.consume_nav_escape_probe_char(c) {
                    return;
                }
                self.input.insert_char(c);
                self.on_prompt_edited();
            }
            KeyCode::Tab
                if !ctrl && !alt && !shift && !self.slash_menu_open() && !self.mention_open() =>
            {
                self.trigger_text_completion()
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
            // Alt+Up / Alt+Down are intentionally inert for message navigation. Ctrl+K / Ctrl+N
            // are the supported, reliable message-nav keys across terminals.
            KeyCode::Up if alt => {}
            KeyCode::Down if alt => {}
            KeyCode::Up => {
                if key.modifiers == KeyModifiers::NONE
                    && self.consume_nav_escape_probe_arrow(KeyCode::Up)
                {
                    return;
                }
                self.input.handle_key(TextAreaKey::Up, modifiers(shift));
            }
            KeyCode::Down => {
                if key.modifiers == KeyModifiers::NONE
                    && self.consume_nav_escape_probe_arrow(KeyCode::Down)
                {
                    return;
                }
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
            // Esc interrupts a streaming turn (stopping it gracefully); otherwise it clears the
            // message selection focus.
            KeyCode::Esc => {
                if self.is_busy() {
                    self.interrupt_turn();
                } else {
                    self.arm_nav_escape_probe();
                    self.clear_navigation();
                }
            }
            _ => {}
        }
    }

    /// Handles `Enter` on the current prompt. Slash exit commands quit; any other non-empty text is
    /// accepted as a [pending prompt](App::take_pending_prompt) the event loop turns into a
    /// streaming turn. A message submitted while a turn is already streaming is queued (not dropped)
    /// and runs once the active turn settles.
    fn submit(&mut self) {
        let value = self.input.value();
        let trimmed = value.trim();
        if trimmed == "/exit" || trimmed == "/quit" {
            self.should_quit = true;
            return;
        }
        if self.dispatch_prompt_attachment_command(trimmed) {
            return;
        }
        if trimmed.is_empty() {
            return;
        }
        let attachments = std::mem::take(&mut self.composer_attachments);
        // Inline any `@`-mentioned file contents into the prompt sent to the model, while showing the
        // typed `@path` prompt in the transcript: the expanded text becomes the prompt (sent and
        // carried into history) and the typed text becomes the user-visible header.
        let typed = trimmed.to_owned();
        self.record_prompt_history(&typed);
        self.prompt_history_nav = None;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let expanded = mention::expand(&typed, &cwd);
        let header = (expanded != typed).then(|| typed.clone());

        if self.is_busy() {
            // A turn is streaming: queue this message so it is admitted (and run next) rather than
            // dropped, and acknowledge it so it reads as accepted.
            self.queued_prompts.push_back(QueuedPrompt {
                header,
                prompt: expanded,
                attachments,
            });
            self.mention = None;
            self.input.set_value("");
            self.push_toast(
                ToastLevel::Info,
                "Message queued — sends after the current turn",
            );
            return;
        }

        if let Some(header) = header {
            self.pending_header = Some(header);
        }
        self.pending_prompt = Some(expanded);
        self.pending_attachments = attachments;
        self.mention = None;
        self.input.set_value("");
        // A new turn is about to append content; return to the latest and drop any stale
        // navigation focus so the streaming reply stays in view.
        self.reset_view();
    }

    /// Handle prompt-level attachment commands typed directly into the prompt.
    ///
    /// Returns `true` when the input was treated as an attachment command and should not be sent to
    /// the model as a normal prompt.
    fn dispatch_prompt_attachment_command(&mut self, trimmed: &str) -> bool {
        if trimmed == "/attach" {
            self.open_attachments_dialog();
            self.input.set_value("");
            return true;
        }
        if let Some(path) = trimmed.strip_prefix("/attach ") {
            if self.attach_local_file_path(path) {
                self.input.set_value("");
            }
            return true;
        }
        if trimmed == "/paste" {
            if self.attach_from_clipboard_image() {
                self.input.set_value("");
            }
            return true;
        }
        false
    }

    /// Whether the prompt is in `!` shell mode (rendered with a distinct affordance).
    #[must_use]
    pub fn shell_mode(&self) -> bool {
        self.shell_mode
    }

    /// Whether plan mode is active — the read-only `plan` agent, which hard-denies edits and shell
    /// so the agent can only investigate and propose a plan. Drives the plan-mode chrome/breadcrumb,
    /// the amber footer banner, the `read-only · plan` status word, and the local shell hard-deny.
    #[must_use]
    pub fn plan_mode(&self) -> bool {
        self.status.agent == "plan"
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
        // Plan mode is read-only: the local shell is hard-denied exactly like the model's bash tool,
        // so a command typed in `!` shell mode is NOT executed — it surfaces a red `blocked` banner
        // and returns to the plan composer instead of running.
        if self.plan_mode() {
            self.transcript.push(Entry::Blocked {
                command: command.to_owned(),
                reason: SHELL_BLOCKED_WHILE_PLANNING.to_owned(),
            });
            self.input.set_value("");
            self.shell_mode = false;
            self.pin_to_bottom();
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

fn attachments_to_message_content_parts(
    attachments: &[InputAttachment],
) -> Vec<MessageContentPart> {
    attachments
        .iter()
        .map(InputAttachment::to_message_content_part)
        .collect()
}

fn attachments_from_message_parts(parts: &[MessagePart]) -> Vec<InputAttachment> {
    parts
        .iter()
        .filter_map(crate::attachments::attachment_from_message_part)
        .collect()
}

fn user_message_parts(content: &str, attachments: &[InputAttachment]) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    if !content.trim().is_empty() {
        parts.push(MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::Text {
                text: content.to_owned(),
            },
        });
    }
    parts.extend(attachments.iter().map(InputAttachment::to_message_part));
    parts
}

fn render_user_entry(content: &str, attachments: &[InputAttachment]) -> String {
    if attachments.is_empty() {
        return content.to_owned();
    }
    let mut lines = Vec::with_capacity(1 + attachments.len());
    lines.push(content.to_owned());
    lines.extend(attachments.iter().map(InputAttachment::transcript_label));
    lines.join("\n")
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
        Entry::Notice(text) | Entry::Error(text) | Entry::TurnError(text) => text.clone(),
        Entry::Shell(outcome) => format!("$ {}\n{}", outcome.command, outcome.output),
        Entry::Blocked { command, reason } => format!("blocked: {command} — {reason}"),
    }
}

/// Write a session export Markdown document to a timestamped file under the data directory's
/// `exports/` folder, returning the path written. The directory is created if needed.
fn write_export(content: &str, ext: &str) -> std::io::Result<PathBuf> {
    let dir = export_dir();
    std::fs::create_dir_all(&dir)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = dir.join(format!("cortex-session-{nanos}.{ext}"));
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Escape the HTML metacharacters in `text` so exported transcript content renders as literal text
/// rather than markup.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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

/// Checkpoint turn label used for a `/diff` viewer revert when no durable session id is available
/// (a not-yet-persisted session), so the revert is still recorded as an undoable, net-diff-visible
/// turn. A persisted session reverts under its own session id so the server's session-scoped diff
/// route reflects it.
const DIFF_REVERT_TURN_LABEL: &str = "tui-diff-revert";

/// Whether a tool by name mutates a file (and so produces a diff attachable to its card).
fn is_mutating_tool(name: &str) -> bool {
    matches!(name, "edit" | "multiedit" | "write" | "apply_patch")
}

fn resolve_diff_path(path: &str) -> Result<PathBuf, String> {
    let rel = PathBuf::from(path);
    if rel.is_absolute() {
        return Err("absolute paths are not revertable from the diff viewer".to_owned());
    }
    if rel.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("path escapes the current workspace".to_owned());
    }
    let cwd = std::env::current_dir().map_err(|err| err.to_string())?;
    Ok(cwd.join(rel))
}

fn split_text_lines(content: &str) -> (Vec<String>, bool) {
    if content.is_empty() {
        return (Vec::new(), false);
    }
    let trailing_newline = content.ends_with('\n');
    let mut lines: Vec<String> = content.split('\n').map(str::to_owned).collect();
    if trailing_newline {
        let _ = lines.pop();
    }
    (lines, trailing_newline)
}

fn join_text_lines(lines: &[String], trailing_newline: bool) -> String {
    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

fn apply_diff_revert(diff: &crate::diff::FileDiff, hunk: Option<usize>) -> Result<bool, String> {
    let path = resolve_diff_path(&diff.path)?;
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() != diff.after.as_deref() {
        return Err("file changed since the diff was captured".to_owned());
    }

    match hunk {
        None => {
            if current.as_deref() == diff.before.as_deref() {
                return Ok(false);
            }
            match &diff.before {
                Some(before) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
                    }
                    std::fs::write(&path, before).map_err(|err| err.to_string())?;
                }
                None => {
                    if path.exists() {
                        std::fs::remove_file(&path).map_err(|err| err.to_string())?;
                    }
                }
            }
            Ok(true)
        }
        Some(hunk_index) => {
            let hunk = diff
                .hunks
                .get(hunk_index)
                .ok_or_else(|| "selected hunk no longer exists".to_owned())?;
            let after = diff
                .after
                .as_deref()
                .ok_or_else(|| "missing post-edit content".to_owned())?;
            let (mut lines, trailing_newline) = split_text_lines(after);
            let start = hunk.new_start.saturating_sub(1);
            let end = start.saturating_add(hunk.new_len);
            if end > lines.len() {
                return Err("hunk range is out of bounds for the current file".to_owned());
            }

            let replacement = hunk.old_lines.clone();
            if lines[start..end] == replacement[..] {
                return Ok(false);
            }
            lines.splice(start..end, replacement.iter().cloned());
            let reverted = join_text_lines(&lines, trailing_newline);
            if current.as_deref() == Some(reverted.as_str()) {
                return Ok(false);
            }

            if diff.before.is_none() && reverted.is_empty() {
                if path.exists() {
                    std::fs::remove_file(&path).map_err(|err| err.to_string())?;
                    return Ok(true);
                }
                return Ok(false);
            }

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
            std::fs::write(&path, reverted).map_err(|err| err.to_string())?;
            Ok(true)
        }
    }
}

/// Map an undo/redo [`RestoreOutcome`] to the user-facing `(transcript notice, toast level, toast
/// message)` triple. Pure so the wording (and the graceful "nothing to do" path) is unit-testable
/// without a real store. `redo` selects the redo phrasing.
fn restore_report(redo: bool, outcome: &RestoreOutcome) -> (String, ToastLevel, String) {
    let (past, verb) = if redo {
        ("Redid", "redo")
    } else {
        ("Undid", "undo")
    };
    let action = if redo { "re-applied" } else { "reverted" };
    match outcome {
        RestoreOutcome::Restored { paths, .. } => (
            format!(
                "{past} last change — {action} {} file(s): {}",
                paths.len(),
                paths.join(", ")
            ),
            ToastLevel::Success,
            format!("Change {action}"),
        ),
        RestoreOutcome::Nothing => (
            format!("Nothing to {verb}."),
            ToastLevel::Info,
            format!("Nothing to {verb}"),
        ),
        RestoreOutcome::Conflict { conflicts, .. } => (
            format!(
                "Cannot {verb}: {} file(s) changed outside Cortex since the snapshot: {}",
                conflicts.len(),
                conflicts.join(", ")
            ),
            ToastLevel::Error,
            format!("Cannot {verb} (file changed)"),
        ),
    }
}

/// Parse an LSP diagnostics summary line into `(display filename, errors, warnings)`.
///
/// Recognizes the `render_feedback_block` shape `lsp diagnostics for <path>: <E> error(s),
/// <W> warning(s), <O> other`. Returns `None` for any other line.
fn parse_lsp_summary(line: &str) -> Option<(String, usize, usize)> {
    let lower = line.to_ascii_lowercase();
    let marker = "lsp diagnostics for ";
    let start = lower.find(marker)? + marker.len();
    let (path, tail) = line[start..].split_once(':')?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let file = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_owned();
    let tail = tail.to_ascii_lowercase();
    Some((
        file,
        count_before(&tail, "error"),
        count_before(&tail, "warning"),
    ))
}

/// The integer immediately preceding `keyword` in `text` (e.g. `2` in `2 error(s)`), or `0`.
fn count_before(text: &str, keyword: &str) -> usize {
    let Some(pos) = text.find(keyword) else {
        return 0;
    };
    let digits: String = text[..pos]
        .trim_end()
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .chars()
        .rev()
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// Parse an LSP diagnostic detail line into a structured [`LspDiagnosticRow`].
///
/// Recognizes BOTH the real runtime `tools::Diagnostic::render()` shape emitted by completed
/// edit/write/apply_patch/lsp tool cards — `<severity> [code] (source) at line <L>, character
/// <C>-<D>: <message>` (no leading `- `) — and the legacy `render_feedback_block` shape with a
/// leading `- `. The multi-line range form (`... at line <L>, character <C> to line <L2>, character
/// <D>: <message>`) is handled by anchoring on the FIRST `at line <N>` and the FIRST `: ` after it
/// (the location's separator always precedes any colon inside the message, so a message containing
/// a colon is preserved). Only `error`/`warning` severities map to the sidebar's E/W rows;
/// informational/hint diagnostics return `None`.
fn parse_lsp_detail(line: &str) -> Option<LspDiagnosticRow> {
    let rest = line.strip_prefix("- ").unwrap_or(line);
    let is_error = match rest
        .split_whitespace()
        .next()?
        .to_ascii_lowercase()
        .as_str()
    {
        "error" => true,
        "warning" | "warn" => false,
        _ => return None,
    };
    let at_line = rest.find("at line ")?;
    let after = &rest[at_line + "at line ".len()..];
    let line_no: u32 = after
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    let message = after
        .find(": ")
        .map(|idx| after[idx + 2..].trim().to_owned())?;
    Some(LspDiagnosticRow {
        line: line_no,
        is_error,
        message,
    })
}

/// Parse the `cortex lsp diagnostics` CLI text header line `lsp diagnostics for <path>\tstatus=<s>`
/// into the diagnosed file's basename, but ONLY for a conclusive `status=diagnostics`/`status=clean`
/// result. Distinct from [`parse_lsp_summary`], which parses the `render_feedback_block`
/// `<path>: <E> error(s)` shape. Returns `None` for that colon shape (so the two never collide) and
/// for `no_server`/`unavailable` statuses (which carry no diagnostics).
fn parse_lsp_cli_header(line: &str) -> Option<String> {
    let marker = "lsp diagnostics for ";
    let start = line.to_ascii_lowercase().find(marker)? + marker.len();
    let tail = &line[start..];
    // The `render_feedback_block` summary form is `<path>: <E> error(s)...`; that shape is handled
    // by `parse_lsp_summary` instead, so bail if it matches (or if the tail leads with a colon).
    if tail.trim_start().starts_with(':') || parse_lsp_summary(line).is_some() {
        return None;
    }
    // The CLI header is `<path>\tstatus=<s>`; only the `diagnostics`/`clean` statuses describe a
    // real result. `no_server`/`unavailable` carry no diagnostics and must NOT form a (falsely
    // clean) group, so require the tab-delimited status token to be present and conclusive.
    let (path, rest) = tail.split_once('\t')?;
    let status = rest
        .split_whitespace()
        .find_map(|token| token.strip_prefix("status="))?;
    if !matches!(status, "diagnostics" | "clean") {
        return None;
    }
    basename_of(path)
}

/// The basename of a path-like string (the segment after the last `/`), or the whole trimmed string
/// when it has no separator. Returns `None` for an empty string.
fn basename_of(path: &str) -> Option<String> {
    let file = std::path::Path::new(path.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| path.trim());
    if file.is_empty() {
        None
    } else {
        Some(file.to_owned())
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
/// configuration's `[mcp]` table. Empty when no servers are configured.
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

/// The status word + tone shown in the `/mcp` view for a connection state (`ready`→green,
/// `needs auth`→amber, `error`→red, `disabled`→neutral).
fn mcp_state_display(state: mcp::ConnectionState) -> (&'static str, crate::view::McpStatusTone) {
    use crate::view::McpStatusTone;
    match state {
        mcp::ConnectionState::Connected => ("ready", McpStatusTone::Ready),
        mcp::ConnectionState::NeedsAuth => ("needs auth", McpStatusTone::Degraded),
        mcp::ConnectionState::Failed => ("error", McpStatusTone::Error),
        mcp::ConnectionState::Disabled => ("disabled", McpStatusTone::Neutral),
    }
}

/// Build the structured `/mcp` view rows from live manager state: per server, the status tone/word,
/// the tool count (only when connected), and the comma-joined exposed capabilities (tools, then
/// resources by URI, then prompts by name — so the view's catalog matches `cortex mcp list`), or the
/// connection error detail when the server failed / needs auth.
fn mcp_server_rows(
    statuses: &[mcp::ServerStatus],
    tools: Vec<mcp::McpTool>,
    resources: Vec<mcp::McpResource>,
    prompts: Vec<mcp::McpPrompt>,
) -> Vec<crate::view::McpServerRow> {
    statuses
        .iter()
        .map(|status| {
            let (word, tone) = mcp_state_display(status.state);
            let connected = status.state == mcp::ConnectionState::Connected;
            let server_tools = tools
                .iter()
                .filter(|tool| tool.server == status.name)
                .map(|tool| tool.tool.clone())
                .collect::<Vec<_>>();
            let tool_count = connected.then_some(server_tools.len());

            let exposes = if status.state == mcp::ConnectionState::NeedsAuth {
                status.public_error().unwrap_or_default()
            } else {
                match status
                    .error
                    .as_deref()
                    .filter(|error| !error.trim().is_empty())
                {
                    Some(error) => error.trim().to_owned(),
                    None => {
                        let mut caps = server_tools;
                        caps.extend(
                            resources
                                .iter()
                                .filter(|resource| resource.server == status.name)
                                .map(|resource| resource.uri.clone()),
                        );
                        caps.extend(
                            prompts
                                .iter()
                                .filter(|prompt| prompt.server == status.name)
                                .map(|prompt| prompt.name.clone()),
                        );
                        caps.join(", ")
                    }
                }
            };

            crate::view::McpServerRow {
                name: status.name.clone(),
                status_word: word.to_owned(),
                tone,
                tool_count,
                exposes,
            }
        })
        .collect()
}

/// The configured MCP servers as structured `/mcp` view rows before a manager connects: each server
/// is shown with a neutral `configured` status and no tool count, from the effective config's
/// `[mcp]` table. Empty when no servers are configured.
fn configured_mcp_server_rows() -> Vec<crate::view::McpServerRow> {
    let Some(config) = effective_config() else {
        return Vec::new();
    };
    let Some(table) = config.get("mcp").and_then(|value| value.as_table()) else {
        return Vec::new();
    };
    table
        .keys()
        .map(|name| crate::view::McpServerRow {
            name: name.clone(),
            status_word: "configured".to_owned(),
            tone: crate::view::McpStatusTone::Neutral,
            tool_count: None,
            exposes: String::new(),
        })
        .collect()
}

/// Render live MCP manager state into `/mcp` rows, using the live catalogue rather than the
/// initial status snapshot so `mcp.tools.changed`/`mcp.resources.changed`/`mcp.prompts.changed`
/// updates become visible immediately. Each row surfaces the server's tools, resources (by URI),
/// and prompts (by name) so the TUI catalog matches the CLI `cortex mcp list` and the server
/// `/mcp` route.
fn mcp_status_rows(
    statuses: &[mcp::ServerStatus],
    tools: Vec<mcp::McpTool>,
    resources: Vec<mcp::McpResource>,
    prompts: Vec<mcp::McpPrompt>,
) -> Vec<String> {
    statuses
        .iter()
        .map(|status| {
            let tool_ids = tools
                .iter()
                .filter(|tool| tool.server == status.name)
                .map(|tool| tool.id.as_str())
                .collect::<Vec<_>>();
            let suffix = if tool_ids.is_empty() {
                "tools: (none)".to_owned()
            } else {
                format!("tools: {}", tool_ids.join(", "))
            };
            let resource_uris = resources
                .iter()
                .filter(|resource| resource.server == status.name)
                .map(|resource| resource.uri.as_str())
                .collect::<Vec<_>>();
            let resources_suffix = if resource_uris.is_empty() {
                String::new()
            } else {
                format!("  —  resources: {}", resource_uris.join(", "))
            };
            let prompt_names = prompts
                .iter()
                .filter(|prompt| prompt.server == status.name)
                .map(|prompt| prompt.name.as_str())
                .collect::<Vec<_>>();
            let prompts_suffix = if prompt_names.is_empty() {
                String::new()
            } else {
                format!("  —  prompts: {}", prompt_names.join(", "))
            };
            let detail = if status.state == mcp::ConnectionState::NeedsAuth {
                status
                    .public_error()
                    .map(|error| format!("  —  {error}"))
                    .unwrap_or_default()
            } else {
                status
                    .error
                    .as_deref()
                    .filter(|error| !error.trim().is_empty())
                    .map(|error| format!("  —  {error}"))
                    .unwrap_or_default()
            };
            format!(
                "{}  —  {}  —  {suffix}{resources_suffix}{prompts_suffix}{detail}",
                status.name,
                status.state.as_str()
            )
        })
        .collect()
}

/// A persisted session plus its messages and todos, loaded for switching the active session in the
/// picker or attaching at startup.
struct LoadedSession {
    session: db::Session,
    messages: Vec<db::Message>,
    usage: db::UsageTotals,
    todos: Vec<protocol::TodoItem>,
}

/// Build the session picker's rows and their backing metadata in lockstep: the synthetic "current
/// session" row (no persisted id) followed by every persisted session, newest-first. The labels and
/// entries share the same order so a highlighted row maps directly to its [`SessionEntry`].
/// A session label that preserves the canonical id and shows `id  —  title` when titled.
fn session_picker_label(session: &db::Session) -> String {
    match &session.title {
        Some(title) if !title.trim().is_empty() => format!("{}  —  {title}", session.id),
        _ => session.id.clone(),
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= 88 {
        line.to_owned()
    } else {
        let kept: String = line.chars().take(87).collect();
        format!("{kept}…")
    }
}

/// A canonical key for a theme name so display aliases compare equal to the registered id
/// (`vscode-dark`/`vscode_dark` == `vscode`, `ocean_dark` == `ocean`).
fn canonical_theme_key(name: &str) -> String {
    match name.trim().to_lowercase().as_str() {
        "vscode" | "vscode-dark" | "vscode_dark" => "vscode".to_owned(),
        "ocean" | "ocean_dark" => "ocean".to_owned(),
        other => other.to_owned(),
    }
}

/// Split a child-session title into `(agent_name, task)`. Subagent titles are either
/// `"<task> (subagent:<agent> parent:<id>)"` or the bare `"subagent:<agent> parent:<id>"` form.
fn parse_subagent_title(title: &str) -> (String, Option<String>) {
    let Some(idx) = title.find("subagent:") else {
        return (title.trim().to_owned(), None);
    };
    let after = &title[idx + "subagent:".len()..];
    let name = after
        .split_whitespace()
        .next()
        .unwrap_or("subagent")
        .to_owned();
    let task = if idx > 0 {
        let prefix = title[..idx].trim_end();
        let prefix = prefix.strip_suffix('(').unwrap_or(prefix).trim_end();
        (!prefix.is_empty()).then(|| prefix.to_owned())
    } else {
        None
    };
    (name, task)
}

/// A compact whole-seconds elapsed duration for a subagent status meta: `8s`, `2m`, `1h`.
fn format_duration_secs(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3_600)
    }
}

/// The model id without its vendor prefix (`google/gemini-2.5-flash` -> `gemini-2.5-flash`), for the
/// compact `agent · model` descriptor on session rows. Ids without a `/` are returned unchanged.
fn short_model(model: &str) -> String {
    model
        .rsplit_once('/')
        .map_or_else(|| model.to_owned(), |(_, tail)| tail.to_owned())
}

/// Humanize a token context window as the picker's `ctx` cell: `1048576 -> 1M`, `200000 -> 200K`.
fn format_context(context: u64) -> String {
    if context == 0 {
        "—".to_owned()
    } else if context >= 1_000_000 {
        let millions = context as f64 / 1_000_000.0;
        if (millions.round() - millions).abs() < 0.05 {
            format!("{}M", millions.round() as u64)
        } else {
            format!("{millions:.1}M")
        }
    } else if context >= 1_000 {
        format!("{}K", (context as f64 / 1_000.0).round() as u64)
    } else {
        context.to_string()
    }
}

/// Format a per-million-token input price as the picker's `price` cell: integers drop the decimals
/// (`$15`), fractional prices keep two (`$0.30`); a zero/unknown price shows `free`.
fn format_price(input_cost: f64) -> String {
    if input_cost <= 0.0 {
        "free".to_owned()
    } else if (input_cost.round() - input_cost).abs() < f64::EPSILON {
        format!("${}", input_cost.round() as u64)
    } else {
        format!("${input_cost:.2}")
    }
}

/// A short, honest description for a reasoning-variant option (the picker's description column).
/// The keys are the real built-in variant ids; the text describes the reasoning tradeoff each one
/// implies rather than a mockup marketing string.
fn variant_description(variant: &str) -> &'static str {
    match variant {
        "default" => "default reasoning effort",
        "fast" => "fastest, minimal reasoning",
        "balanced" => "balanced latency / quality",
        "quality" => "maximum reasoning effort",
        _ => "reasoning variant",
    }
}

/// Classify a discovered skill's source path as the picker's source meta: `project` when the file
/// lives under the current working directory, otherwise `builtin` (global/config skill dirs).
fn skill_source_label(source: &Path, cwd: Option<&Path>) -> String {
    let is_project = cwd.is_some_and(|cwd| source.starts_with(cwd));
    if is_project { "project" } else { "builtin" }.to_owned()
}

/// The current content of a backgrounded `bash` job's log file, read fresh each poll so the card
/// always shows the job's live output. A missing/unreadable log (e.g. the job hasn't written
/// anything yet) reads as empty rather than an error.
fn tail_background_log(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// A short, authentic model descriptor derived from real catalogue metadata (never a hardcoded
/// mockup label): the default model leads with `default`, reasoning models read `reasoning` and
/// attachment-capable models read `multimodal` (both shown when both apply), free models read
/// `free`, and the rest fall back to a capability word.
fn model_descriptor(model: &providers::ModelInfo, is_default: bool) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if is_default {
        parts.push("default");
    }
    if model.reasoning {
        parts.push("reasoning");
    } else if is_default {
        parts.push("fast");
    }
    if model.attachment {
        parts.push("multimodal");
    }
    if parts.is_empty() {
        if model.cost.input <= 0.0 {
            parts.push("free");
        } else {
            parts.push("chat");
        }
    }
    parts.join(" · ")
}

/// The current wall-clock time as whole seconds since the Unix epoch (UTC).
fn current_epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days from the Unix epoch (1970-01-01) for a proleptic-Gregorian `y-m-d` (Howard Hinnant's
/// `days_from_civil`), so a stored UTC timestamp can be diffed against `now` without a date crate.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse a stored `YYYY-MM-DD HH:MM:SS` UTC timestamp into whole seconds since the Unix epoch, or
/// `None` when it is not in that shape.
fn parse_epoch_secs(timestamp: &str) -> Option<i64> {
    let (date, time) = timestamp.split_once(' ')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// A compact relative age for a session's last-update timestamp against `now` (epoch secs): `just
/// now`, `5m`, `2h`, `3d`. Returns an empty string when the timestamp cannot be parsed.
fn relative_age(timestamp: &str, now: i64) -> String {
    let Some(then) = parse_epoch_secs(timestamp) else {
        return String::new();
    };
    let secs = (now - then).max(0);
    if secs < 60 {
        "just now".to_owned()
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// The `HH:MM` field of a stored `YYYY-MM-DD HH:MM:SS` UTC timestamp (for the timeline rail), or an
/// empty string when the timestamp is not in that shape.
fn hhmm(timestamp: &str) -> String {
    timestamp
        .split_once(' ')
        .map(|(_, time)| time.chars().take(5).collect())
        .unwrap_or_default()
}

/// A compact token count for the `/status` token breakdown: `<1000` verbatim, then `X.Xk`, `X.XM`.
fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// A best-effort per-file token estimate for the sidebar `context` panel: the file's on-disk byte
/// length divided by four (a common chars-per-token heuristic). Zero when the path cannot be
/// stat-ed. Cheap — only reads the directory entry's metadata, never the file contents.
fn file_token_estimate(path: &str) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len() / 4)
}

/// The set of absolute paths with uncommitted changes under `root` (`git status --porcelain`),
/// used to mark modified files in the `files` tree and to auto-expand their ancestor directories.
/// Empty when git is unavailable or the tree is clean.
fn git_modified_paths(root: &Path) -> BTreeSet<PathBuf> {
    let mut set = BTreeSet::new();
    let out = run_git(&["status", "--porcelain"]).unwrap_or_default();
    for line in out.lines() {
        // Porcelain v1: two status columns, a space, then the path (a rename is `old -> new`).
        let path = line.get(3..).unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let rel = path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"');
        set.insert(root.join(rel));
    }
    set
}

/// Whether a directory entry name should be omitted from the `files` tree: hidden dotfiles and the
/// heavy build/VCS directories that would flood the panel and are never useful context.
fn is_ignored_tree_entry(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "target" | "node_modules" | "dist" | "build")
}

/// The workspace git state for the `/status` view, read once when the view opens: the current branch
/// and whether the working tree has no uncommitted changes to *tracked* files (untracked files are
/// ignored, matching "nothing to commit"). `None` when the cwd is not a git repo.
fn git_info() -> Option<crate::view::GitInfo> {
    let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = branch.trim().to_owned();
    if branch.is_empty() {
        return None;
    }
    let status = run_git(&["status", "--porcelain", "--untracked-files=no"]).unwrap_or_default();
    Some(crate::view::GitInfo {
        branch,
        clean: status.trim().is_empty(),
    })
}

/// Run `git` with `args` in the current working directory, returning its stdout on success.
fn run_git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// The changed files in the working tree (`git status --porcelain`), de-duplicated and sorted, for
/// the `/review` view and the `/init` scan. A rename status (`old -> new`) reports the new path.
/// Best-effort: an empty vec when git is unavailable or the tree is clean.
fn changed_files() -> Vec<String> {
    let out = run_git(&["status", "--porcelain"]).unwrap_or_default();
    let mut files: Vec<String> = Vec::new();
    for line in out.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = line[3..].trim();
        let path = path.rsplit(" -> ").next().unwrap_or(path).trim();
        let path = path.trim_matches('"');
        if !path.is_empty() {
            files.push(path.to_owned());
        }
    }
    files.sort();
    files.dedup();
    files
}

/// A short display for a changed path in a review finding (`file` or `dir/file`), kept compact so
/// the location column stays readable.
fn review_location(path: &str) -> String {
    let mut tail: Vec<&str> = path.rsplit('/').take(2).collect();
    tail.reverse();
    tail.join("/")
}

/// Whether a changed file is a source file worth scanning for review findings.
fn is_reviewable_source(path: &str) -> bool {
    matches!(
        std::path::Path::new(path)
            .extension()
            .and_then(std::ffi::OsStr::to_str),
        Some("rs" | "toml" | "md" | "py" | "js" | "ts" | "sh")
    )
}

/// Lightweight, deterministic heuristics that turn the changed working-tree files into `/review`
/// findings, so the review view presents real (working-tree-derived) findings without a network
/// round-trip. Each changed source file is scanned for a small set of common patterns; at most one
/// finding per (file, category) is emitted, capped overall and ordered by severity. When the tree
/// is dirty but no pattern matches, each changed file is listed as an informational review item so
/// the view is never blank over a changed tree.
fn review_findings(changed: &[String]) -> Vec<crate::view::ReviewFinding> {
    use crate::view::{ReviewFinding, Severity};

    const MAX_FILES: usize = 40;
    const MAX_BYTES: usize = 256 * 1024;
    const MAX_FINDINGS: usize = 8;

    struct Rule {
        needle: &'static str,
        severity: Severity,
        text: &'static str,
    }
    const RULES: &[Rule] = &[
        Rule {
            needle: "unsafe ",
            severity: Severity::High,
            text: "unsafe block - audit the invariants",
        },
        Rule {
            needle: ".unwrap()",
            severity: Severity::Medium,
            text: "unwrap may panic - handle the error",
        },
        Rule {
            needle: ".expect(",
            severity: Severity::Medium,
            text: "expect may panic - handle the error",
        },
        Rule {
            needle: ".clone()",
            severity: Severity::Low,
            text: "possible redundant clone",
        },
        Rule {
            needle: "TODO",
            severity: Severity::Info,
            text: "unresolved TODO",
        },
        Rule {
            needle: "FIXME",
            severity: Severity::Info,
            text: "unresolved FIXME",
        },
    ];

    let mut findings: Vec<ReviewFinding> = Vec::new();
    for path in changed.iter().take(MAX_FILES) {
        if !is_reviewable_source(path) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        if content.len() > MAX_BYTES {
            continue;
        }
        let mut seen: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
        for (lineno, line) in content.lines().enumerate() {
            if seen.len() == RULES.len() {
                break;
            }
            for rule in RULES {
                if seen.contains(rule.text) {
                    continue;
                }
                if line.contains(rule.needle) {
                    seen.insert(rule.text);
                    findings.push(ReviewFinding {
                        severity: rule.severity,
                        location: format!("{}:{}", review_location(path), lineno + 1),
                        text: rule.text.to_owned(),
                    });
                }
            }
        }
    }

    if findings.is_empty() {
        for path in changed.iter().take(6) {
            findings.push(ReviewFinding {
                severity: Severity::Info,
                location: format!("{}:1", review_location(path)),
                text: "changed - review recommended".to_owned(),
            });
        }
    }

    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.location.cmp(&b.location))
    });
    findings.truncate(MAX_FINDINGS);
    findings
}

/// Build the `/init` scan log from the real repository: detect a cargo workspace/crate, count
/// crates and source files, note found docs and the test command, and mark the AGENTS.md
/// generation as the active (pending) step. Best-effort — steps reflect what is actually detected
/// in the cwd, so the dialog shows the real scan performed.
fn build_init_scan() -> InitDialog {
    let root = crate::motd::current_dir_display();
    let mut steps: Vec<InitStep> = Vec::new();

    let has_cargo = std::path::Path::new("Cargo.toml").exists();
    let cargo_ws = std::fs::read_to_string("Cargo.toml")
        .map(|s| s.contains("[workspace]"))
        .unwrap_or(false);
    if cargo_ws {
        steps.push(InitStep {
            label: "detected Rust workspace (cargo)".to_owned(),
            done: true,
        });
        let (crates, sources) = count_rust_workspace();
        steps.push(InitStep {
            label: format!(
                "{crates} crate{} · {} source files",
                if crates == 1 { "" } else { "s" },
                crate::view::format_thousands(sources as u64)
            ),
            done: true,
        });
    } else if has_cargo {
        steps.push(InitStep {
            label: "detected Rust crate (cargo)".to_owned(),
            done: true,
        });
    } else {
        steps.push(InitStep {
            label: "scanned the project root".to_owned(),
            done: true,
        });
    }

    let docs: Vec<&str> = ["README.md", "CONTRIBUTING.md", "AGENTS.md"]
        .into_iter()
        .filter(|f| std::path::Path::new(f).exists())
        .collect();
    if !docs.is_empty() {
        steps.push(InitStep {
            label: format!("found {}", docs.join(", ")),
            done: true,
        });
    }

    if has_cargo {
        steps.push(InitStep {
            label: "test cmd: cargo test --workspace".to_owned(),
            done: true,
        });
    }

    steps.push(InitStep {
        label: format!("generating AGENTS.md {}", crate::glyphs::ELLIPSIS),
        done: false,
    });

    InitDialog { root, steps }
}

/// Count the workspace's member crates (subdirectories of `crates/` with a `Cargo.toml`) and the
/// total number of Rust source files under `crates/` (skipping `target`/`.git`/`node_modules`), for
/// the `/init` scan log.
fn count_rust_workspace() -> (usize, usize) {
    let mut crates = 0usize;
    if let Ok(entries) = std::fs::read_dir("crates") {
        for entry in entries.flatten() {
            if entry.path().join("Cargo.toml").exists() {
                crates += 1;
            }
        }
    }
    let sources = count_rs_files(std::path::Path::new("crates"), 0);
    (crates, sources)
}

/// Recursively count `.rs` files under `dir`, skipping build/vcs directories and bounding the
/// recursion depth so the `/init` scan stays cheap.
fn count_rs_files(dir: &std::path::Path, depth: usize) -> usize {
    if depth > 12 {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(name.as_ref(), "target" | ".git" | "node_modules") {
                continue;
            }
            count += count_rs_files(&path, depth + 1);
        } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs") {
            count += 1;
        }
    }
    count
}

/// A rough token estimate for the conversation history (≈4 chars/token), used as a floor for the
/// `/compact` before-size when no live usage total is available yet.
fn estimate_history_tokens(history: &[Message]) -> u64 {
    let chars: usize = history.iter().map(|m| m.content.chars().count()).sum();
    (chars / 4) as u64
}

/// Load the effective configuration with no CLI overrides, or `None` if it cannot be resolved.
fn effective_config() -> Option<settings::EffectiveConfig> {
    settings::ConfigInputs::from_process(std::collections::BTreeMap::new())
        .ok()
        .and_then(|inputs| settings::load(inputs).ok())
}

pub(crate) fn configured_attention_config() -> AttentionConfig {
    let mut config = AttentionConfig::default();
    let Some(effective) = effective_config() else {
        return config;
    };

    if let Some(quiet) = effective
        .get("quiet")
        .and_then(toml::Value::as_bool)
        .or_else(|| {
            effective
                .get("tui")
                .and_then(toml::Value::as_table)
                .and_then(|table| table.get("quiet"))
                .and_then(toml::Value::as_bool)
        })
    {
        config.quiet = quiet;
    }

    let attention = effective
        .get("tui")
        .and_then(toml::Value::as_table)
        .and_then(|table| table.get("attention"))
        .and_then(toml::Value::as_table)
        .or_else(|| effective.get("attention").and_then(toml::Value::as_table));

    if let Some(table) = attention {
        if let Some(value) = table.get("enabled").and_then(toml::Value::as_bool) {
            config.enabled = value;
        }
        if let Some(value) = table.get("quiet").and_then(toml::Value::as_bool) {
            config.quiet = value;
        }
        if let Some(value) = table.get("audio").and_then(toml::Value::as_bool) {
            config.audio = value;
        }
        if let Some(value) = table.get("visual").and_then(toml::Value::as_bool) {
            config.visual = value;
        }
        if let Some(events) = table.get("events").and_then(toml::Value::as_array) {
            config.completion = false;
            config.error = false;
            config.permission = false;
            for event in events.iter().filter_map(toml::Value::as_str) {
                match event.trim().to_ascii_lowercase().as_str() {
                    "completion" => config.completion = true,
                    "error" => config.error = true,
                    "permission" => config.permission = true,
                    _ => {}
                }
            }
        }
    }

    config
}

/// Apply startup attention settings before surfacing any startup diagnostics so quiet/disabled
/// attention config takes effect before custom-theme load errors call `note_error`.
pub(crate) fn apply_startup_attention_and_theme_diagnostics(app: &mut App) {
    apply_attention_config_and_theme_diagnostics(
        app,
        configured_attention_config(),
        load_custom_themes_from_config(),
    );
}

fn apply_attention_config_and_theme_diagnostics(
    app: &mut App,
    attention_config: AttentionConfig,
    diagnostics: impl IntoIterator<Item = String>,
) {
    app.set_attention_config(attention_config);
    for diagnostic in diagnostics {
        app.note_error(diagnostic);
    }
}

/// Load custom JSON themes from config and register them with the frame theme catalogue.
///
/// Supported config keys:
/// - `theme_files = ["..."]`
/// - `custom_theme_files = ["..."]`
/// - `[tui] theme_files = ["..."]`
/// - `[tui] custom_theme_files = ["..."]`
pub(crate) fn load_custom_themes_from_config() -> Vec<String> {
    frame::style::ThemeColors::clear_custom_themes();
    let Some(config) = effective_config() else {
        return Vec::new();
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut paths = configured_custom_theme_files(config.table(), &cwd);
    paths.sort();
    paths.dedup();

    let mut diagnostics = Vec::new();
    for path in paths {
        let body = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(err) => {
                diagnostics.push(format!(
                    "custom theme file {} could not be read: {err}",
                    path.display()
                ));
                continue;
            }
        };
        match parse_custom_theme_document(&body) {
            Ok(themes) => {
                for (name, theme) in themes {
                    frame::style::ThemeColors::register_custom_theme(&name, theme);
                }
            }
            Err(err) => diagnostics.push(format!(
                "custom theme file {} is invalid: {err}",
                path.display()
            )),
        }
    }
    diagnostics
}

fn configured_custom_theme_files(config: &toml::Table, cwd: &std::path::Path) -> Vec<PathBuf> {
    let mut values = Vec::new();
    collect_theme_file_values(config.get("theme_files"), &mut values);
    collect_theme_file_values(config.get("custom_theme_files"), &mut values);
    if let Some(tui) = config.get("tui").and_then(toml::Value::as_table) {
        collect_theme_file_values(tui.get("theme_files"), &mut values);
        collect_theme_file_values(tui.get("custom_theme_files"), &mut values);
    }
    values
        .into_iter()
        .map(|raw| resolve_theme_file_path(&raw, cwd))
        .collect()
}

fn collect_theme_file_values(value: Option<&toml::Value>, out: &mut Vec<String>) {
    let Some(value) = value else {
        return;
    };
    if let Some(items) = value.as_array() {
        out.extend(
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned),
        );
    } else if let Some(item) = value.as_str() {
        let item = item.trim();
        if !item.is_empty() {
            out.push(item.to_owned());
        }
    }
}

fn resolve_theme_file_path(raw: &str, cwd: &std::path::Path) -> PathBuf {
    let expanded = if raw == "~" {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(raw))
    } else if let Some(rest) = raw.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(raw))
    } else {
        PathBuf::from(raw)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

fn parse_custom_theme_document(
    body: &str,
) -> Result<Vec<(String, frame::style::ThemeColors)>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|err| format!("JSON parse error: {err}"))?;
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .map(parse_custom_theme_value)
            .collect::<Result<Vec<_>, _>>(),
        serde_json::Value::Object(mut map) => {
            if let Some(themes) = map.remove("themes") {
                let list = themes
                    .as_array()
                    .ok_or_else(|| "`themes` must be an array".to_owned())?;
                return list
                    .iter()
                    .map(parse_custom_theme_value)
                    .collect::<Result<Vec<_>, _>>();
            }
            Ok(vec![parse_custom_theme_value(&serde_json::Value::Object(
                map,
            ))?])
        }
        _ => Err("expected a JSON object or array".to_owned()),
    }
}

fn parse_custom_theme_value(
    value: &serde_json::Value,
) -> Result<(String, frame::style::ThemeColors), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "each theme must be a JSON object".to_owned())?;
    let name = object
        .get("name")
        .or_else(|| object.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "theme object must include non-empty `name` or `id`".to_owned())?
        .to_ascii_lowercase();
    let mut theme = object
        .get("extends")
        .and_then(serde_json::Value::as_str)
        .map(frame::style::ThemeColors::from_name)
        .unwrap_or_else(frame::style::ThemeColors::dark);

    apply_color_key(object, &["primary"], &mut theme.primary)?;
    apply_color_key(object, &["secondary"], &mut theme.secondary)?;
    apply_color_key(object, &["accent"], &mut theme.accent)?;
    apply_color_key(object, &["background"], &mut theme.background)?;
    apply_color_key(object, &["text"], &mut theme.text)?;
    apply_color_key(object, &["text_dim", "textDim"], &mut theme.text_dim)?;
    apply_color_key(object, &["text_muted", "textMuted"], &mut theme.text_muted)?;
    apply_color_key(object, &["success"], &mut theme.success)?;
    apply_color_key(object, &["warning"], &mut theme.warning)?;
    apply_color_key(object, &["error"], &mut theme.error)?;
    apply_color_key(object, &["info"], &mut theme.info)?;
    apply_color_key(object, &["border"], &mut theme.border)?;
    apply_color_key(
        object,
        &["border_focus", "borderFocus"],
        &mut theme.border_focus,
    )?;
    apply_color_key(
        object,
        &["card_border", "cardBorder"],
        &mut theme.card_border,
    )?;
    apply_color_key(
        object,
        &["soft", "working_bg", "workingBand"],
        &mut theme.soft,
    )?;
    apply_color_key(
        object,
        &["search_hit", "searchHit", "hit"],
        &mut theme.search_hit,
    )?;
    apply_color_key(
        object,
        &["search_hit_current", "searchHitCurrent", "hitCurrent"],
        &mut theme.search_hit_current,
    )?;

    if let Some(surface) = object.get("surface").or_else(|| object.get("surfaces")) {
        let values = surface
            .as_array()
            .ok_or_else(|| "`surface` must be an array of four colors".to_owned())?;
        if values.len() != 4 {
            return Err("`surface` must contain exactly four colors".to_owned());
        }
        for (index, value) in values.iter().enumerate() {
            theme.surface[index] = parse_hex_color(value, "surface")?;
        }
    }

    Ok((name, theme))
}

fn apply_color_key(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
    target: &mut ratatui::style::Color,
) -> Result<(), String> {
    let value = keys.iter().find_map(|key| object.get(*key));
    if let Some(value) = value {
        *target = parse_hex_color(value, keys[0])?;
    }
    Ok(())
}

fn parse_hex_color(value: &serde_json::Value, key: &str) -> Result<ratatui::style::Color, String> {
    let Some(raw) = value.as_str() else {
        return Err(format!("`{key}` must be a color string like #RRGGBB"));
    };
    let trimmed = raw.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("`{key}` must be #RRGGBB (got `{raw}`)"));
    }
    let parse = |slice: &str| {
        u8::from_str_radix(slice, 16).map_err(|err| format!("invalid `{key}` color `{raw}`: {err}"))
    };
    Ok(ratatui::style::Color::Rgb(
        parse(&hex[0..2])?,
        parse(&hex[2..4])?,
        parse(&hex[4..6])?,
    ))
}

fn configured_custom_slash_commands() -> Vec<CustomSlashCommand> {
    let Some(config) = effective_config() else {
        return Vec::new();
    };
    parse_custom_slash_commands(config.get("command"))
}

fn parse_custom_slash_commands(value: Option<&toml::Value>) -> Vec<CustomSlashCommand> {
    let Some(toml::Value::Table(table)) = value else {
        return Vec::new();
    };

    let mut commands = Vec::new();
    for (raw_name, entry) in table {
        let Some(name) = normalize_custom_command_name(raw_name) else {
            continue;
        };
        let Some(command) = parse_custom_command_entry(&name, entry) else {
            continue;
        };
        commands.push(command);
    }
    commands
}

fn normalize_custom_command_name(raw: &str) -> Option<String> {
    let name = raw.trim().trim_start_matches('/');
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return None;
    }
    Some(name.to_ascii_lowercase())
}

fn parse_custom_command_entry(name: &str, entry: &toml::Value) -> Option<CustomSlashCommand> {
    match entry {
        toml::Value::String(prompt) => {
            let prompt = prompt.trim();
            (!prompt.is_empty()).then(|| CustomSlashCommand {
                name: name.to_owned(),
                description: "Run custom command".to_owned(),
                prompt: prompt.to_owned(),
            })
        }
        toml::Value::Table(table) => {
            if table.get("disabled").and_then(toml::Value::as_bool) == Some(true) {
                return None;
            }
            let prompt = table
                .get("prompt")
                .and_then(toml::Value::as_str)
                .or_else(|| table.get("template").and_then(toml::Value::as_str))
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())?;
            let description = table
                .get("description")
                .and_then(toml::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .unwrap_or("Run custom command");
            Some(CustomSlashCommand {
                name: name.to_owned(),
                description: description.to_owned(),
                prompt: prompt.to_owned(),
            })
        }
        _ => None,
    }
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
fn write_share_reference(id: &str, session: &str, dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{id}.json"));
    let body = format!(
        "{{\"id\":\"{id}\",\"session\":\"{session}\",\"url\":\"http://127.0.0.1:3200/share/{id}\"}}"
    );
    std::fs::write(&path, body)?;
    Ok(path)
}

fn revoke_local_share_references(session: &str, dir: &Path) -> io::Result<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut removed = 0usize;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if value.get("session").and_then(serde_json::Value::as_str) == Some(session)
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

fn remote_abort_path(session_id: &str) -> PathBuf {
    data_subdir("remote-abort").join(format!("{session_id}.abort"))
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ATTACH_COUNTER: AtomicUsize = AtomicUsize::new(0);
    static DIFF_COUNTER: AtomicUsize = AtomicUsize::new(0);
    static SHARE_COUNTER: AtomicUsize = AtomicUsize::new(0);
    static AUTH_COUNTER: AtomicUsize = AtomicUsize::new(0);

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
            variant: "default".to_owned(),
            session: "new".to_owned(),
        })
    }

    #[test]
    fn connect_command_opens_the_flow_mid_session() {
        // VAL-CONN-001: dispatching /connect opens the provider-connect flow mid-session (not
        // boot-only), and Esc from the provider step closes it back to the conversation.
        let mut a = app();
        assert!(!a.connect_active(), "the flow is closed initially");
        a.dispatch_builtin_command("connect", "Connect a provider (API key or OAuth)");
        assert!(a.connect_active(), "/connect opens the connect flow");
        a.on_key(key(KeyCode::Esc));
        assert!(!a.connect_active(), "esc closes the flow");
    }

    #[test]
    fn connect_disconnect_key_is_a_safe_noop_when_nothing_is_connected() {
        // VAL-CONN-009: `d` requests a disconnect only for a connected provider. With no stored
        // credential, pressing `d` is a safe no-op that keeps the provider step and the flow open —
        // the confirm-before-remove step is only reached for a connected provider (exercised
        // deterministically in the connect-state tests).
        //
        // Hermetic: the flow is built against an isolated, empty auth store (a disposable tmp path)
        // so the connected-provider check never reads the ambient ~/.local/share/cortex/auth.json,
        // which on this shared host holds a real credential. Without this isolation the highlighted
        // provider would read as connected and `d` would (incorrectly, for this test) advance to the
        // confirm step. The `d` key still travels the real App::on_key -> on_connect_key routing.
        let auth_path = isolated_auth_path("connect-disconnect-noop");
        let mut a = app();
        a.open_connect_at(&auth_path);
        assert!(a.connect_active(), "/connect flow is open");
        a.on_key(key(KeyCode::Char('d')));
        assert!(
            a.connect_active(),
            "the flow stays open after a no-op disconnect key"
        );
        assert_eq!(
            a.connect_state().map(crate::connect::ConnectState::step),
            Some(crate::connect::Step::Provider),
            "a no-op disconnect keeps the provider step"
        );
    }

    #[test]
    fn enter_advances_past_onboarding() {
        // VAL-ONB-011: pressing Enter on the onboarding welcome screen advances out of onboarding
        // without leaking a submission into the prompt beneath.
        let mut a = app();
        a.set_onboarding(true);
        assert!(a.onboarding_active(), "onboarding shows when set");
        a.on_key(key(KeyCode::Enter));
        assert!(!a.onboarding_active(), "Enter dismisses onboarding");
        assert!(
            a.input_value().is_empty(),
            "Enter starts fresh: nothing is typed into the prompt"
        );
    }

    #[test]
    fn onboarding_key_dismisses_and_forwards_and_ctrl_c_quits() {
        // The welcome screen is a start splash, not a trap: a non-Enter key dismisses it and is
        // forwarded to the surface beneath (so a fresh-config launch never swallows the first
        // keystroke, e.g. a slash command), while Ctrl+C still quits.
        let mut a = app();
        a.set_onboarding(true);
        a.on_key(key(KeyCode::Char('x')));
        assert!(
            !a.onboarding_active(),
            "a printable key dismisses onboarding"
        );
        assert_eq!(
            a.input_value(),
            "x",
            "the dismissing key is forwarded into the prompt beneath"
        );

        let mut b = app();
        b.set_onboarding(true);
        b.on_key(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(b.should_quit, "Ctrl+C quits from onboarding");
    }

    #[test]
    fn default_app_is_not_in_onboarding() {
        // A normally-constructed app (e.g. every other test / a configured launch) is not gated
        // into onboarding.
        assert!(!app().onboarding_active());
    }

    /// A disposable, guaranteed-empty auth-store path (unique per call) under the temp dir. The file
    /// is never created, so resolving connected providers from it yields the empty set — letting a
    /// connect/auth flow be built hermetically, independent of the ambient
    /// `~/.local/share/cortex/auth.json`. Nothing is written, so there is nothing to clean up.
    fn isolated_auth_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cortex-tui-auth-{test_name}-{}-{}/auth.json",
            std::process::id(),
            AUTH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn isolated_share_dir(test_name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "cortex-tui-share-{test_name}-{}-{}",
            std::process::id(),
            SHARE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let shares_dir = root.join("shares");
        std::fs::create_dir_all(&shares_dir).expect("create share dir");
        (root, shares_dir)
    }

    fn share_reference_count(shares_dir: &std::path::Path) -> usize {
        if !shares_dir.is_dir() {
            return 0;
        }
        std::fs::read_dir(shares_dir)
            .expect("read share dir")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .count()
    }

    fn temp_attachment(ext: &str, bytes: &[u8]) -> std::path::PathBuf {
        let name = format!(
            "cortex-tui-attachment-{}-{}.{}",
            std::process::id(),
            ATTACH_COUNTER.fetch_add(1, Ordering::Relaxed),
            ext
        );
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, bytes).expect("write temp attachment");
        path
    }

    fn temp_relative_diff_file(before: &str, after: &str) -> (String, PathBuf) {
        let rel = format!(
            "target/cortex-tui-diff-{}-{}.txt",
            std::process::id(),
            DIFF_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = PathBuf::from(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create diff test dir");
        }
        std::fs::write(&path, after).expect("write current file bytes");
        assert_ne!(before, after, "fixture must be a real diff");
        (rel, path)
    }

    /// A fresh, unset interrupt flag for tests that begin a turn without a real worker.
    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn fake_secret() -> String {
        format!("sk-{}-{}", "or-v1", "facefeed".repeat(8))
    }

    /// Deliver a permission ask to the app (as the worker would), returning the receiver the test
    /// reads the answered decision from.
    fn deliver_permission(
        a: &mut App,
        tool: &str,
        subject: &str,
    ) -> std::sync::mpsc::Receiver<ScopedDecision> {
        let (reply, wait) = std::sync::mpsc::channel();
        a.apply(TurnMessage::Permission(Box::new(PermissionAsk::new(
            tool, subject, reply,
        ))));
        wait
    }

    // A delivered permission ask opens a modal prompt naming the tool + subject, defaulting the
    // highlight to Allow once.
    #[test]
    fn permission_ask_opens_a_modal_prompt() {
        let mut a = app();
        let _wait = deliver_permission(&mut a, "write", "marker.txt");
        assert!(a.permission_open());
        assert_eq!(a.permission_prompt(), Some(("write", "marker.txt", 0, "")));
    }

    #[test]
    fn attention_badge_fires_for_documented_events_and_respects_quiet_mode() {
        let mut a = app();
        a.set_attention_config(AttentionConfig {
            enabled: true,
            quiet: false,
            audio: true,
            visual: true,
            completion: true,
            error: true,
            permission: true,
        });

        // Documented completion event.
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("ping".to_owned(), rx, flag());
        tx.send(TurnMessage::Finished).expect("finish turn");
        a.poll_turn();
        assert_eq!(a.attention_badge(), Some("completion"));
        assert!(
            a.drain_pending_bells() > 0,
            "completion cue schedules an audible bell"
        );

        // Documented permission event.
        let _wait = deliver_permission(&mut a, "write", "marker.txt");
        assert_eq!(a.attention_badge(), Some("permission"));
        let _ = a.drain_pending_bells();

        // Non-documented shell actions do not trigger an attention badge.
        a.attention_badge = None;
        a.set_input_value("!echo hi");
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.attention_badge().is_none(),
            "shell mode command is not a documented attention cue source"
        );

        // Quiet mode suppresses documented cues.
        a.set_attention_config(AttentionConfig {
            quiet: true,
            ..AttentionConfig::default()
        });
        a.attention_badge = None;
        a.set_turn_error("boom".to_owned());
        assert!(
            a.attention_badge().is_none(),
            "quiet mode suppresses visual attention cues"
        );
        assert_eq!(
            a.drain_pending_bells(),
            0,
            "quiet mode suppresses audible attention cues"
        );
    }

    #[test]
    fn startup_theme_diagnostics_still_surface_errors_when_attention_is_quiet_or_disabled() {
        let secret = fake_secret();
        let diagnostic = format!("custom theme file broken.json is invalid: leaked {secret}");

        let quiet = AttentionConfig {
            quiet: true,
            ..AttentionConfig::default()
        };
        let disabled = AttentionConfig {
            enabled: false,
            ..AttentionConfig::default()
        };

        for config in [quiet, disabled] {
            let mut a = app();
            apply_attention_config_and_theme_diagnostics(&mut a, config, [diagnostic.clone()]);

            assert!(
                a.attention_badge().is_none(),
                "quiet/disabled startup config suppresses visual cues"
            );
            assert_eq!(
                a.drain_pending_bells(),
                0,
                "quiet/disabled startup config suppresses audible cues"
            );

            let error = match a.transcript.last() {
                Some(Entry::Error(message)) => message,
                other => panic!("expected startup diagnostic error entry, got {other:?}"),
            };
            assert!(
                !error.contains(&secret),
                "startup diagnostic must be redacted: {error}"
            );
            assert!(
                error.contains(redaction::PLACEHOLDER),
                "startup diagnostic should include redaction placeholder: {error}"
            );
            let toast = a.toasts().last().expect("startup diagnostic toast exists");
            assert_eq!(toast.level, ToastLevel::Error);
            assert_eq!(toast.message, error.as_str());
        }
    }

    #[test]
    fn startup_theme_diagnostics_emit_error_attention_when_enabled() {
        let mut a = app();
        apply_attention_config_and_theme_diagnostics(
            &mut a,
            AttentionConfig {
                enabled: true,
                quiet: false,
                audio: true,
                visual: true,
                completion: false,
                error: true,
                permission: false,
            },
            ["custom theme file broken.json is invalid: JSON parse error".to_owned()],
        );

        assert_eq!(a.attention_badge(), Some("error"));
        assert!(
            a.drain_pending_bells() > 0,
            "enabled startup error attention config should queue a bell"
        );
    }

    // Pressing `y` approves the ask once: the decision is a one-shot Allow, the prompt is dismissed,
    // and a notice records the approval.
    #[test]
    fn pressing_y_approves_the_ask() {
        let mut a = app();
        let wait = deliver_permission(&mut a, "write", "marker.txt");
        a.on_key(key(KeyCode::Char('y')));
        assert_eq!(wait.recv().unwrap(), ScopedDecision::once(Decision::Allow));
        assert!(!a.permission_open());
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(text) if text.contains("Approved")))
        );
    }

    // Pressing `a` approves AND remembers the ask: the decision carries the Always scope so the
    // engine records it for persistence, and a notice records the "always allowed" choice.
    #[test]
    fn pressing_a_always_allows_the_ask() {
        let mut a = app();
        let wait = deliver_permission(&mut a, "bash", "echo hi");
        a.on_key(key(KeyCode::Char('a')));
        assert_eq!(
            wait.recv().unwrap(),
            ScopedDecision::always(Decision::Allow)
        );
        assert!(!a.permission_open());
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(text) if text.contains("Always allowed")))
        );
    }

    // Pressing `n` denies the ask: the decision is a one-shot Deny and a notice records the denial.
    #[test]
    fn pressing_n_denies_the_ask() {
        let mut a = app();
        let wait = deliver_permission(&mut a, "bash", "rm -rf .");
        a.on_key(key(KeyCode::Char('n')));
        assert_eq!(wait.recv().unwrap(), ScopedDecision::once(Decision::Deny));
        assert!(!a.permission_open());
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(text) if text.contains("Denied")))
        );
    }

    // When Deny is highlighted, typed text is captured as corrective feedback and returned with the
    // deny decision so the running turn can recover with a corrected action.
    #[test]
    fn deny_can_include_corrective_feedback() {
        let mut a = app();
        let wait = deliver_permission(&mut a, "bash", "rm -rf .");
        a.on_key(key(KeyCode::Right));
        a.on_key(key(KeyCode::Right));
        for ch in "Use ls first".chars() {
            a.on_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(
            a.permission_prompt(),
            Some(("bash", "rm -rf .", 2, "Use ls first"))
        );
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            wait.recv().unwrap(),
            ScopedDecision::once(Decision::Deny).with_feedback("Use ls first")
        );
        assert!(
            a.transcript.iter().any(
                |e| matches!(e, Entry::Notice(text) if text.contains("feedback: Use ls first"))
            ),
            "feedback should be visible in transcript notices"
        );
    }

    // While Deny is selected, printable keys are feedback text and must never trigger Allow
    // hotkeys, even for `y`/`a`.
    #[test]
    fn deny_feedback_mode_treats_y_and_a_as_text() {
        let mut a = app();
        let wait = deliver_permission(&mut a, "bash", "rm -rf .");

        a.on_key(key(KeyCode::Right));
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.permission_prompt(), Some(("bash", "rm -rf .", 2, "")));

        a.on_key(key(KeyCode::Char('y')));
        assert!(a.permission_open());
        assert_eq!(a.permission_prompt(), Some(("bash", "rm -rf .", 2, "y")));

        a.on_key(key(KeyCode::Char('a')));
        assert!(a.permission_open());
        assert_eq!(a.permission_prompt(), Some(("bash", "rm -rf .", 2, "ya")));

        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            wait.recv().unwrap(),
            ScopedDecision::once(Decision::Deny).with_feedback("ya")
        );
    }

    // Question asks capture typed answer text even when Allow is selected and submit it as
    // approval feedback so the turn can resume with the user's answer.
    #[test]
    fn question_prompt_submits_typed_answer_with_allow() {
        let mut a = app();
        let wait = deliver_permission(&mut a, "question", "Q1: Which option?");
        assert_eq!(
            a.permission_prompt(),
            Some(("question", "Q1: Which option?", 0, ""))
        );

        for ch in "Use option B".chars() {
            a.on_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(
            a.permission_prompt(),
            Some(("question", "Q1: Which option?", 0, "Use option B"))
        );

        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            wait.recv().unwrap(),
            ScopedDecision::once(Decision::Allow).with_feedback("Use option B")
        );
        assert!(!a.permission_open());
    }

    // Esc denies the ask (so the turn continues gracefully rather than the dialog lingering).
    #[test]
    fn esc_denies_the_ask() {
        let mut a = app();
        let wait = deliver_permission(&mut a, "write", "x.txt");
        a.on_key(key(KeyCode::Esc));
        assert_eq!(wait.recv().unwrap(), ScopedDecision::once(Decision::Deny));
        assert!(!a.permission_open());
    }

    // Arrow selection then Enter confirms the highlighted option: Right steps Allow once → Allow
    // always → Deny (clamped), Enter then denies; the default highlight (Allow once) is confirmed by
    // Enter with no movement, and the middle option confirms an "always allow".
    #[test]
    fn arrow_then_enter_confirms_the_selected_option() {
        let mut a = app();
        let deny = deliver_permission(&mut a, "edit", "a.rs");
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.permission_prompt(), Some(("edit", "a.rs", 1, "")));
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.permission_prompt(), Some(("edit", "a.rs", 2, "")));
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.permission_prompt(), Some(("edit", "a.rs", 2, "")));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(deny.recv().unwrap(), ScopedDecision::once(Decision::Deny));

        let mut b = app();
        let allow = deliver_permission(&mut b, "edit", "b.rs");
        b.on_key(key(KeyCode::Enter));
        assert_eq!(allow.recv().unwrap(), ScopedDecision::once(Decision::Allow));

        let mut c = app();
        let always = deliver_permission(&mut c, "edit", "c.rs");
        c.on_key(key(KeyCode::Right));
        assert_eq!(c.permission_prompt(), Some(("edit", "c.rs", 1, "")));
        c.on_key(key(KeyCode::Enter));
        assert_eq!(
            always.recv().unwrap(),
            ScopedDecision::always(Decision::Allow)
        );
    }

    // While a permission prompt is open it is modal: a printable key drives the dialog rather than
    // editing the prompt beneath it.
    #[test]
    fn permission_prompt_is_modal_over_the_input() {
        let mut a = app();
        let _wait = deliver_permission(&mut a, "write", "z.txt");
        a.on_key(key(KeyCode::Char('y')));
        assert_eq!(
            a.input_value(),
            "",
            "the keypress answered the prompt, not the input"
        );
    }

    // The permission prompt renders as a titled modal naming the tool, its target, and the
    // Allow/Deny options over the frame (VAL-TUI-091).
    #[test]
    fn permission_prompt_renders_over_the_frame() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        let _wait = deliver_permission(&mut a, "write", "marker.txt");

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, &a)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();

        assert!(
            rendered.contains("Permission required"),
            "the modal title is drawn:\n{rendered}"
        );
        assert!(
            rendered.contains("write"),
            "the gated tool is named:\n{rendered}"
        );
        assert!(
            rendered.contains("marker.txt"),
            "the target is named:\n{rendered}"
        );
        assert!(
            rendered.contains("Allow") && rendered.contains("Deny"),
            "both options are offered:\n{rendered}"
        );
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
    fn completion_span_targets_only_the_active_token() {
        assert_eq!(App::completion_span("hel world", 3), 0..3);
        assert_eq!(App::completion_span("say hel there", 7), 4..7);
        assert_eq!(App::completion_span("say hello wor", 13), 10..13);
        assert_eq!(App::completion_span("say  there", 4), 4..4);
    }

    #[test]
    fn pending_completion_replaces_span_or_inserts_at_cursor() {
        let start = PendingTextCompletion {
            candidate: "hello".to_owned(),
            cursor: 3,
            span: 0..3,
        };
        assert_eq!(
            App::apply_pending_text_completion("hel world", &start),
            "hello world"
        );

        let middle = PendingTextCompletion {
            candidate: "hello".to_owned(),
            cursor: 7,
            span: 4..7,
        };
        assert_eq!(
            App::apply_pending_text_completion("say hel there", &middle),
            "say hello there"
        );

        let end = PendingTextCompletion {
            candidate: "world".to_owned(),
            cursor: 13,
            span: 10..13,
        };
        assert_eq!(
            App::apply_pending_text_completion("say hello wor", &end),
            "say hello world"
        );

        let insert = PendingTextCompletion {
            candidate: "hello".to_owned(),
            cursor: 4,
            span: 4..4,
        };
        assert_eq!(
            App::apply_pending_text_completion("say  there", &insert),
            "say hello there"
        );
    }

    #[test]
    fn editing_prompt_clears_stale_pending_completion_state() {
        let mut a = app();
        a.set_input_value("hel");
        a.pending_text_completion = Some(PendingTextCompletion {
            candidate: "hello".to_owned(),
            cursor: 3,
            span: 0..3,
        });

        a.on_key(key(KeyCode::End));
        a.on_key(key(KeyCode::Char('p')));

        assert!(a.pending_text_completion.is_none());
        assert_eq!(a.input_value(), "help");
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
    fn tab_inserts_the_highlighted_file_mention() {
        // VAL-CONV-041: the `@` palette advertises `⇥ insert`, so Tab inserts the highlighted file.
        let mut a = app();
        a.on_key(key(KeyCode::Char('@')));
        inject_mention_files(&mut a, &["crates/cli/src/main.rs"]);
        a.on_key(key(KeyCode::Tab));
        assert_eq!(a.input_value(), "@crates/cli/src/main.rs");
        assert!(!a.mention_open(), "picker closed after Tab insertion");
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

    #[test]
    fn plan_mode_is_the_read_only_plan_agent() {
        // VAL-PERM-015/017: plan mode is exactly the read-only `plan` agent; the default build agent
        // is not plan mode.
        let mut a = app();
        assert!(!a.plan_mode(), "the default build agent is not plan mode");
        a.status.agent = "plan".to_owned();
        assert!(a.plan_mode(), "the plan agent is plan mode");
        a.status.agent = "explore".to_owned();
        assert!(
            !a.plan_mode(),
            "explore is read-only but not the plan banner"
        );
    }

    #[test]
    fn plan_mode_hard_denies_a_shell_command_with_a_blocked_banner() {
        // VAL-PERM-019/020: a `!` shell command issued while planning is NOT executed — it surfaces a
        // `blocked` banner naming the command + the dim reason, and leaves no shell output entry.
        let mut a = app();
        a.status.agent = "plan".to_owned();
        a.on_key(key(KeyCode::Char('!')));
        assert!(a.shell_mode(), "shell mode engaged");
        type_str(&mut a, "cargo build --release");
        a.on_key(key(KeyCode::Enter));

        assert!(
            !a.transcript.iter().any(|e| matches!(e, Entry::Shell(_))),
            "the command must NOT run while planning (no shell output entry)"
        );
        let blocked = a
            .transcript
            .iter()
            .rev()
            .find_map(|e| match e {
                Entry::Blocked { command, reason } => Some((command.clone(), reason.clone())),
                _ => None,
            })
            .expect("a blocked banner is surfaced");
        assert_eq!(
            blocked.0, "cargo build --release",
            "the banner names the command"
        );
        assert_eq!(
            blocked.1, SHELL_BLOCKED_WHILE_PLANNING,
            "the banner carries the dim while-planning reason"
        );
        assert!(!a.shell_mode(), "the composer returns to the plan state");
        assert_eq!(a.input_value(), "", "the blocked command is cleared");
    }

    #[test]
    fn exiting_plan_mode_reenables_the_shell() {
        // VAL-PERM-022: once plan mode is left (switch back to build), a `!` shell command runs again
        // (it is no longer hard-blocked by plan mode).
        let mut a = app();
        a.status.agent = "plan".to_owned();
        a.status.agent = "build".to_owned();
        assert!(!a.plan_mode(), "precondition: back in build");
        a.on_key(key(KeyCode::Char('!')));
        type_str(&mut a, "echo hi");
        a.on_key(key(KeyCode::Enter));
        assert!(
            matches!(a.transcript.last(), Some(Entry::Shell(outcome)) if outcome.command == "echo hi"),
            "the shell command runs once plan mode is exited: {:?}",
            a.transcript.last()
        );
    }

    fn usage(input: u32, output: u32, cost: Option<f64>) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            reasoning_output_tokens: None,
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
    fn a_mid_stream_submit_is_queued_not_dropped() {
        // VAL-TUI-089: a message submitted while a turn streams is admitted (queued), not dropped.
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("first".to_owned(), rx, flag());
        assert!(a.is_busy());

        for c in "second".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));

        // The message is accepted: the prompt is cleared and the message is queued behind the turn
        // rather than started immediately or dropped.
        assert_eq!(a.input_value(), "");
        assert_eq!(a.queued_count(), 1);
        assert!(
            a.take_pending_prompt().is_none(),
            "the queued message does not start while the first turn is in flight"
        );

        // Once the active turn settles, the queued message is promoted into its own turn.
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();
        assert_eq!(a.queued_count(), 0);
        assert_eq!(a.take_pending_prompt().as_deref(), Some("second"));
    }

    #[test]
    fn queued_messages_run_in_submission_order() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("first".to_owned(), rx, flag());

        for msg in ["alpha", "beta"] {
            for c in msg.chars() {
                a.on_key(key(KeyCode::Char(c)));
            }
            a.on_key(key(KeyCode::Enter));
        }
        assert_eq!(a.queued_count(), 2);

        // Finishing the first turn drains the oldest queued message.
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();
        assert_eq!(a.take_pending_prompt().as_deref(), Some("alpha"));
        assert_eq!(a.queued_count(), 1);
    }

    #[test]
    fn esc_interrupts_a_streaming_turn_and_returns_to_idle() {
        // VAL-TUI-090: the interrupt key stops the turn and returns the UI to an idle editable
        // state with partial output preserved.
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        let cancel = flag();
        a.begin_turn("count to 200".to_owned(), rx, cancel.clone());
        // Some partial output streamed before the interrupt.
        a.apply(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "1\n2\n3".to_owned(),
        }));
        assert!(a.is_busy());

        a.on_key(key(KeyCode::Esc));

        // The worker was signalled, the busy indicator cleared, and the UI is editable again.
        assert!(
            cancel.load(Ordering::Relaxed),
            "the worker was asked to cancel"
        );
        assert!(!a.is_busy(), "the turn is no longer streaming");
        // Partial output is preserved and an interruption notice is shown.
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Assistant(t) if t.answer.contains('2'))),
            "partial answer preserved: {:?}",
            a.transcript
        );
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(m) if m.contains("interrupted"))),
            "an interruption notice is recorded"
        );
        // A fresh message can be submitted right away.
        for c in "again".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.take_pending_prompt().as_deref(), Some("again"));
    }

    #[test]
    fn interrupt_clears_queued_messages() {
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("first".to_owned(), rx, flag());
        for c in "queued".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.queued_count(), 1);

        a.on_key(key(KeyCode::Esc));
        assert!(!a.is_busy());
        assert_eq!(
            a.queued_count(),
            0,
            "interrupt returns the UI fully to idle"
        );
        assert!(a.take_pending_prompt().is_none());
    }

    #[test]
    fn esc_when_idle_does_not_interrupt() {
        // With no turn in flight Esc clears navigation rather than interrupting (no panic, no-op).
        let mut a = app();
        assert!(!a.is_busy());
        a.on_key(key(KeyCode::Esc));
        assert!(!a.is_busy());
        assert_eq!(a.queued_count(), 0);
    }

    #[test]
    fn streamed_deltas_grow_the_assistant_answer() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());

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
        a.begin_turn("q".to_owned(), rx, flag());

        tx.send(TurnMessage::Event(RunnerEvent::Reasoning {
            text: "let me think".to_owned(),
            metadata: None,
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
    fn transcript_redacts_user_answer_reasoning_and_errors() {
        let secret = fake_secret();
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(format!("prompt carries {secret}"), rx, flag());

        tx.send(TurnMessage::Event(RunnerEvent::Reasoning {
            text: format!("reasoning mentions {secret}"),
            metadata: None,
        }))
        .unwrap();
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: format!("answer mentions {secret}"),
        }))
        .unwrap();
        a.poll_turn();

        match &a.transcript[0] {
            Entry::User(text) => {
                assert!(!text.contains(&secret), "user entry leaked: {text}");
                assert!(text.contains(redaction::PLACEHOLDER), "user entry: {text}");
            }
            other => panic!("expected user entry, got {other:?}"),
        }
        let turn = last_assistant(&a);
        assert!(
            !turn.reasoning.contains(&secret) && !turn.answer.contains(&secret),
            "assistant entry leaked: {turn:?}"
        );
        assert!(turn.reasoning.contains(redaction::PLACEHOLDER), "{turn:?}");
        assert!(turn.answer.contains(redaction::PLACEHOLDER), "{turn:?}");

        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("second".to_owned(), rx, flag());
        tx.send(TurnMessage::SetupError(format!("setup mentions {secret}")))
            .unwrap();
        a.poll_turn();
        assert!(matches!(
            a.transcript.last(),
            Some(Entry::TurnError(message))
                if !message.contains(&secret) && message.contains(redaction::PLACEHOLDER)
        ));
    }

    #[test]
    fn streamed_answer_and_reasoning_redact_secret_split_across_deltas() {
        let body = "facefeed".repeat(8);
        let secret = format!("sk-{}-{}", "or-v1", body);
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());

        for fragment in [
            "answer before ",
            "sk-",
            "or-v1-",
            &body[..16],
            &body[16..],
            " answer after",
        ] {
            tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
                text: fragment.to_owned(),
            }))
            .unwrap();
            a.poll_turn();
        }
        for fragment in [
            "reason before ",
            "sk-",
            "or-v1-",
            &body[..24],
            &body[24..],
            " reason after",
        ] {
            tx.send(TurnMessage::Event(RunnerEvent::Reasoning {
                text: fragment.to_owned(),
                metadata: None,
            }))
            .unwrap();
            a.poll_turn();
        }
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        let turn = last_assistant(&a);
        assert!(
            !turn.answer.contains(&secret) && !turn.reasoning.contains(&secret),
            "split secret leaked in final transcript: {turn:?}"
        );
        assert!(
            turn.answer.contains(redaction::PLACEHOLDER)
                && turn.reasoning.contains(redaction::PLACEHOLDER),
            "split secret should be marked with the canonical placeholder: {turn:?}"
        );
        assert!(
            turn.answer.contains("answer before") && turn.answer.contains("answer after"),
            "non-secret answer text reconstructs: {turn:?}"
        );
        assert!(
            turn.reasoning.contains("reason before") && turn.reasoning.contains("reason after"),
            "non-secret reasoning text reconstructs: {turn:?}"
        );
    }

    #[test]
    fn streamed_redaction_state_resets_between_turns() {
        let body = "facefeed".repeat(8);
        let mut a = app();

        let (first_tx, first_rx) = std::sync::mpsc::channel();
        a.begin_turn("first".to_owned(), first_rx, flag());
        first_tx
            .send(TurnMessage::Event(RunnerEvent::TextDelta {
                text: "sk-".to_owned(),
            }))
            .unwrap();
        a.poll_turn();
        first_tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();
        assert_eq!(
            last_assistant(&a).answer,
            "sk-",
            "safe partial key text is flushed at turn end"
        );

        let (second_tx, second_rx) = std::sync::mpsc::channel();
        a.begin_turn("second".to_owned(), second_rx, flag());
        second_tx
            .send(TurnMessage::Event(RunnerEvent::TextDelta {
                text: format!("or-v1-{body}"),
            }))
            .unwrap();
        second_tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();
        assert!(
            last_assistant(&a).answer.contains(&format!("or-v1-{body}")),
            "a new turn must not inherit the prior turn's partial redaction state"
        );
    }

    #[test]
    fn no_reasoning_leaves_an_empty_region() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());
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
        a.begin_turn("q".to_owned(), rx, flag());
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
        a.begin_turn("q".to_owned(), rx, flag());
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
    fn animations_enabled_tracks_reduced_motion() {
        let mut a = app();
        assert!(a.animations_enabled(), "animations are on by default");
        assert!(!a.reduced_motion());

        a.set_reduced_motion(true);
        assert!(
            !a.animations_enabled(),
            "reduced motion disables animations"
        );
        assert!(a.reduced_motion());

        a.set_reduced_motion(false);
        assert!(a.animations_enabled());
    }

    #[test]
    fn selection_move_starts_a_highlight_transition_gated_by_animations() {
        // VAL-FOUND-035: an adjacent selection move plays a brief highlight transition when
        // animations are enabled (the loop wakes to advance it), and it resolves to a from→to
        // cross-fade phase in [0,1). A wrap/multi-row jump and reduced motion are instant.
        let mut a = app();
        a.note_selection_move(2, 3);
        assert!(
            a.selection_transition_active(),
            "an adjacent move starts a transition"
        );
        assert!(
            a.animation_active(),
            "the event loop wakes on the fast cadence to advance the transition"
        );
        let anim = a.selection_anim_state().expect("resolved transition state");
        assert_eq!((anim.from, anim.to), (2, 3));
        assert!(
            anim.phase >= 0.0 && anim.phase < 1.0,
            "phase in [0,1): {}",
            anim.phase
        );

        // A wrap-around / multi-row jump snaps instantly (no transition to animate).
        a.note_selection_move(9, 0);
        assert!(!a.selection_transition_active());
        assert!(a.selection_anim_state().is_none());

        // Under reduced motion (`--no-anim`), even an adjacent move is instant (VAL-CROSS-030).
        a.set_reduced_motion(true);
        a.note_selection_move(1, 2);
        assert!(
            a.selection_anim_state().is_none(),
            "no transition under --no-anim"
        );
        assert!(!a.selection_transition_active());
    }

    #[test]
    fn spinner_is_present_but_static_under_reduced_motion() {
        let mut a = app();
        a.set_reduced_motion(true);
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());

        let glyph = a
            .spinner_glyph()
            .expect("a static working mark is shown while busy");
        assert_eq!(
            glyph,
            crate::glyphs::SPINNER_BREATHE[STATIC_SPINNER_FRAME],
            "the spinner shows one fixed frame, never cycling, when animations are off"
        );
        // The breathing mark is from the safe set (never braille).
        assert!(!crate::glyphs::contains_braille(glyph));
    }

    #[test]
    fn spinner_phase_is_a_pure_function_of_elapsed_time() {
        // With animations enabled the spinner derives its frame from elapsed time, so the same
        // elapsed value always maps to the same frame and the phase advances at the tick cadence.
        let frames = &crate::glyphs::SPINNER_BREATHE;
        let period = ANIMATION_TICK_MS as u128;
        assert_eq!(
            crate::glyphs::frame_at(frames, 0, period),
            crate::glyphs::frame_at(frames, 0, period)
        );
        assert_eq!(
            crate::glyphs::frame_at(frames, 250, period),
            crate::glyphs::frame_at(frames, 250, period)
        );
        assert_ne!(
            crate::glyphs::frame_at(frames, 0, period),
            crate::glyphs::frame_at(frames, period, period),
            "advancing one tick advances the frame"
        );
    }

    #[test]
    fn caret_blink_is_deterministic_and_gated_by_reduced_motion() {
        // Pure function of elapsed time: visible for the first half-period, hidden for the next.
        assert!(caret_visible_at(0));
        assert!(caret_visible_at(CARET_BLINK_HALF_PERIOD_MS - 1));
        assert!(!caret_visible_at(CARET_BLINK_HALF_PERIOD_MS));
        assert!(!caret_visible_at(2 * CARET_BLINK_HALF_PERIOD_MS - 1));
        assert!(caret_visible_at(2 * CARET_BLINK_HALF_PERIOD_MS));

        // Under reduced motion the caret is always solid (visible), even mid-turn — no blink.
        let mut a = app();
        a.set_reduced_motion(true);
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());
        assert!(a.cursor_blink_visible(), "no blink when animations are off");

        // Idle (no turn) the caret is solid regardless of motion setting.
        let idle = app();
        assert!(idle.cursor_blink_visible());
    }

    #[test]
    fn poll_timeout_uses_active_cadence_when_busy_and_idle_cadence_otherwise() {
        let mut a = app();
        assert_eq!(
            a.poll_timeout(),
            Duration::from_millis(IDLE_POLL_MS),
            "an idle TUI blocks at the cheap cadence (no busy-spin)"
        );

        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());
        assert_eq!(
            a.poll_timeout(),
            Duration::from_millis(ANIMATION_TICK_MS),
            "a streaming turn polls at the ~100ms animation cadence"
        );
        assert_eq!(
            ANIMATION_TICK_MS, 100,
            "the active animation tick is ~100ms"
        );
    }

    #[test]
    fn models_refresh_in_flight_drives_the_active_poll_cadence() {
        // An in-flight background models.dev refresh is a busy signal for the event-loop poll
        // cadence, so the in-place catalogue swap + "refreshing…" indicator clear land promptly
        // (no ~200ms idle-tick lag) — mirroring how a streaming turn tightens the cadence. It must
        // NOT busy-spin when idle: the cadence returns to the cheap idle timeout once the refresh
        // lands.
        let mut a = app();
        assert!(!a.models_refreshing(), "precondition: no refresh in flight");
        assert_eq!(
            a.poll_timeout(),
            Duration::from_millis(IDLE_POLL_MS),
            "an idle TUI with no refresh blocks at the cheap cadence (no busy-spin)"
        );

        // Simulate an in-flight refresh by injecting the channel the worker would publish on.
        let (tx, rx) = std::sync::mpsc::channel();
        a.models_refresh_rx = Some(rx);
        assert!(a.models_refreshing(), "the refresh is in flight");
        assert_eq!(
            a.poll_timeout(),
            Duration::from_millis(ANIMATION_TICK_MS),
            "an in-flight models refresh polls at the tighter active cadence"
        );
        assert!(
            !a.is_busy() && !a.has_toasts(),
            "the refresh tightens the cadence without spoofing a streaming turn or a toast"
        );

        // The result lands and is drained; the cadence returns to idle (no busy-spin).
        tx.send(ModelsRefreshMessage::Failed).unwrap();
        a.poll_models_refresh();
        assert!(!a.models_refreshing(), "the refresh is no longer in flight");
        assert_eq!(
            a.poll_timeout(),
            Duration::from_millis(IDLE_POLL_MS),
            "the cadence returns to idle once the refresh lands (no busy-spin)"
        );
    }

    #[test]
    fn reduced_motion_idle_does_not_request_animation_wakeups() {
        // The wake predicate is the seam that prevents idle busy-spin: it is false whenever
        // animations are off, so the loop never wakes merely to advance an animation.
        let mut a = app();
        assert!(!a.animation_active(), "idle is not animating");

        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());
        assert!(a.animation_active(), "a streaming turn is animating");

        a.set_reduced_motion(true);
        assert!(
            !a.animation_active(),
            "reduced motion suppresses animation wakeups even while a turn streams"
        );
        // Even under reduced motion the loop still polls at the active cadence so streamed deltas
        // are applied and the static frame stays current.
        assert_eq!(a.poll_timeout(), Duration::from_millis(ANIMATION_TICK_MS));
    }

    #[test]
    fn finished_turn_is_folded_into_history() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("question".to_owned(), rx, flag());
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
        a.begin_turn("q".to_owned(), rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::Error {
            message: "provider exploded".to_owned(),
            kind: "upstream".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        assert!(!a.is_busy());
        assert!(
            matches!(a.transcript.last(), Some(Entry::TurnError(m)) if m == "provider exploded")
        );
        // A turn error is an in-conversation block, never a toast/notification (VAL-ERR-002).
        assert!(a.toasts().is_empty(), "turn error must not raise a toast");
        // The empty assistant entry was dropped, and only the user message reached history.
        let history = a.history_snapshot();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "q");
    }

    #[test]
    fn setup_error_is_surfaced_as_an_error_entry() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());
        tx.send(TurnMessage::SetupError("no credential".to_owned()))
            .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        assert!(!a.is_busy());
        assert!(matches!(a.transcript.last(), Some(Entry::TurnError(m)) if m == "no credential"));
        // The setup failure is rendered inline, never as a toast/notification (VAL-ERR-002).
        assert!(a.toasts().is_empty(), "setup error must not raise a toast");
    }

    #[test]
    fn a_file_diff_is_recorded_in_the_transcript() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("edit the file".to_owned(), rx, flag());

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

    #[test]
    fn a_file_diff_attaches_to_the_matching_update_tool_card() {
        // A file mutation opens an `● Update <path>` card just before its diff arrives; the diff
        // attaches to that card (making it an expandable diff card) rather than becoming a second,
        // detached block, and the card's `+N -N` meta is refreshed from the real diff.
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("edit the file".to_owned(), rx, flag());

        tx.send(TurnMessage::Event(RunnerEvent::ToolCall {
            id: "call_1".to_owned(),
            name: "edit".to_owned(),
            arguments: r#"{"path":"src/main.rs","oldString":"a","newString":"B\nc"}"#.to_owned(),
        }))
        .unwrap();
        a.poll_turn();

        let diff = crate::diff::compute_file_diff("src/main.rs", Some("a\nb\nc\n"), Some("B\nc\n"))
            .expect("a diff");
        let (adds, dels) = (diff.additions, diff.removals);
        tx.send(TurnMessage::FileDiff(Box::new(diff))).unwrap();
        a.poll_turn();

        // No standalone diff entry was created; the diff lives on the tool card.
        assert!(
            !a.transcript.iter().any(|e| matches!(e, Entry::Diff(_))),
            "diff should attach to the card, not push a standalone entry"
        );
        let card = last_tool(&a);
        let attached = card.diff.as_ref().expect("diff attached to the card");
        assert_eq!(attached.path, "src/main.rs");
        assert_eq!(
            card.diff_stat,
            Some((adds, dels)),
            "meta refreshed from the real diff"
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
        a.begin_turn("read the file".to_owned(), rx, flag());

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
    fn background_bash_card_streams_while_running_then_completes_on_pid_exit() {
        // VAL-TOOL-023: a background bash tool result does not complete the card — it keeps the
        // ORANGE running chrome, streams the tailed job log into its body while the process lives,
        // and transitions to done (with the final tailed output) once the job's pid exits.
        use crate::tool_card::ToolStatus;

        // A real short-lived child stands in for the background job's process so `/proc/<pid>`
        // liveness is exercised for real; a temp file stands in for the job's streamed log.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn a stand-in background process");
        let pid = child.id();
        let log = std::env::temp_dir().join(format!("cortex-bgstream-test-{pid}.log"));
        std::fs::write(&log, "Compiling cortex v0.1.0\n").expect("seed the job log");

        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("build in the background".to_owned(), rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::ToolCall {
            id: "bg".to_owned(),
            name: "bash".to_owned(),
            arguments: r#"{"command":"cargo build --release","background":true}"#.to_owned(),
        }))
        .unwrap();
        // The bash tool's background-start acknowledgement carries the pid + log path.
        let ack = format!(
            "[background job started]\njob id: bash-1\npid: {pid}\ncwd: /tmp\noutput: {}\n\
             The command is running in the background.",
            log.display()
        );
        tx.send(TurnMessage::Event(RunnerEvent::ToolResult {
            id: "bg".to_owned(),
            name: "bash".to_owned(),
            parts: vec![tool_part(&ack)],
            is_error: false,
        }))
        .unwrap();
        a.poll_turn();

        // The result did NOT complete the card: it stays running, marked (background), and tracks
        // the job's pid/log so it can be tailed.
        let card = last_tool(&a);
        assert_eq!(
            card.status,
            ToolStatus::Running,
            "background card stays running"
        );
        assert!(
            card.summary.contains("(background)"),
            "summary: {}",
            card.summary
        );
        assert_eq!(card.bg_pid, Some(pid));
        assert_eq!(card.bg_log.as_deref(), Some(log.as_path()));

        // While the process is alive, polling tails the log into the card body (live streaming).
        std::fs::write(&log, "Compiling cortex v0.1.0\n   Compiling deps\n")
            .expect("append to the job log");
        a.poll_background_jobs();
        let card = last_tool(&a);
        assert_eq!(
            card.status,
            ToolStatus::Running,
            "still running while the pid is alive"
        );
        assert!(
            card.output.contains("Compiling deps"),
            "the streamed body tails the log: {}",
            card.output
        );

        // The job produces its final output, then its process exits (reaped).
        std::fs::write(
            &log,
            "Compiling cortex v0.1.0\n   Compiling deps\n    Finished release\n",
        )
        .expect("finalize the job log");
        child.kill().expect("kill the stand-in process");
        child.wait().expect("reap the stand-in process");
        for _ in 0..200 {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Now polling sees the pid gone and transitions the card to done with the final output.
        a.poll_background_jobs();
        let card = last_tool(&a);
        assert_eq!(
            card.status,
            ToolStatus::Completed,
            "the card completes once the job's pid exits"
        );
        assert!(card.bg_pid.is_none(), "a completed job is no longer polled");
        assert!(card.duration_ms.is_some(), "completion records a duration");
        assert!(
            card.output.contains("Finished release"),
            "the final body is the last tailed content: {}",
            card.output
        );

        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn long_background_bash_command_still_streams_and_completes() {
        // VAL-TOOL-023 robustness: a background command longer than the SUMMARY_MAX (~100 char)
        // truncation boundary LOSES the `(background)` marker from its summary, so the entry into
        // background-streaming mode must be gated on the structured `background: true` arg (the
        // card's `is_background` flag), NOT the truncatable summary text. Such a card must still
        // stay ORANGE running, stream its log, and complete on the job's real exit.
        use crate::tool_card::ToolStatus;

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn a stand-in background process");
        let pid = child.id();
        let log = std::env::temp_dir().join(format!("cortex-bglong-test-{pid}.log"));
        std::fs::write(&log, "step 1\n").expect("seed the job log");

        // A command well over 88 chars, so its `<cmd> (background)` summary is truncated and the
        // trailing `(background)` marker is dropped.
        let long_cmd = format!("echo {} && sleep 30", "abcdefghij".repeat(12));
        assert!(
            long_cmd.len() > 88,
            "command must exceed the truncation window"
        );
        let args = format!(
            r#"{{"command":{},"background":true}}"#,
            serde_json::Value::String(long_cmd.clone())
        );

        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("run a long background build".to_owned(), rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::ToolCall {
            id: "bglong".to_owned(),
            name: "bash".to_owned(),
            arguments: args,
        }))
        .unwrap();
        let ack = format!(
            "[background job started]\njob id: bash-1\npid: {pid}\ncwd: /tmp\noutput: {}\n\
             The command is running in the background.",
            log.display()
        );
        tx.send(TurnMessage::Event(RunnerEvent::ToolResult {
            id: "bglong".to_owned(),
            name: "bash".to_owned(),
            parts: vec![tool_part(&ack)],
            is_error: false,
        }))
        .unwrap();
        a.poll_turn();

        // Precondition of the bug: the summary lost its `(background)` marker to truncation, so a
        // summary-substring gate would (wrongly) complete the card immediately. The flag-based gate
        // still enters background-streaming mode.
        let card = last_tool(&a);
        assert!(
            !card.summary.contains("(background)"),
            "the long summary is truncated past the marker: {}",
            card.summary
        );
        assert!(
            card.is_background,
            "the card is flagged background from its args"
        );
        assert_eq!(
            card.status,
            ToolStatus::Running,
            "long background card stays running"
        );
        assert_eq!(card.bg_pid, Some(pid));
        assert_eq!(card.bg_log.as_deref(), Some(log.as_path()));

        // It streams the live log while the process is alive.
        std::fs::write(&log, "step 1\nstep 2\n").expect("append to the job log");
        a.poll_background_jobs();
        let card = last_tool(&a);
        assert_eq!(
            card.status,
            ToolStatus::Running,
            "still running while the pid is alive"
        );
        assert!(
            card.output.contains("step 2"),
            "the body tails the log: {}",
            card.output
        );

        // And it completes once the job's process exits.
        std::fs::write(&log, "step 1\nstep 2\nstep 3 done\n").expect("finalize the job log");
        child.kill().expect("kill the stand-in process");
        child.wait().expect("reap the stand-in process");
        for _ in 0..200 {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        a.poll_background_jobs();
        let card = last_tool(&a);
        assert_eq!(
            card.status,
            ToolStatus::Completed,
            "the long background card completes once its pid exits"
        );
        assert!(
            card.output.contains("step 3 done"),
            "final body: {}",
            card.output
        );

        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn foreground_bash_mentioning_background_does_not_enter_background_mode() {
        // VAL-TOOL-023 misfire guard: a FOREGROUND bash (no `background: true` arg) whose command
        // text contains "(background)" AND whose stdout carries a `pid:`/`output:` ack shape must
        // NOT enter background-streaming mode — the contrived substring/ack must not fool the gate.
        // It completes normally with its output.
        use crate::tool_card::ToolStatus;

        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("echo a fake ack".to_owned(), rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::ToolCall {
            id: "fg".to_owned(),
            name: "bash".to_owned(),
            arguments: r#"{"command":"echo 'starting job (background)'"}"#.to_owned(),
        }))
        .unwrap();
        // A foreground result whose text mimics a background ack (has `(background)`, `pid:`,
        // `output:` lines). This would fool a summary-substring gate.
        let fake = "[background job started]\nstarting job (background)\npid: 4242\ncwd: /tmp\n\
                    output: /tmp/does-not-exist.log";
        tx.send(TurnMessage::Event(RunnerEvent::ToolResult {
            id: "fg".to_owned(),
            name: "bash".to_owned(),
            parts: vec![tool_part(fake)],
            is_error: false,
        }))
        .unwrap();
        a.poll_turn();

        let card = last_tool(&a);
        assert!(
            card.summary.contains("(background)"),
            "the summary text does contain the marker (the misfire trap): {}",
            card.summary
        );
        assert!(
            !card.is_background,
            "no background arg → not a background card"
        );
        assert_eq!(
            card.status,
            ToolStatus::Completed,
            "a foreground bash completes normally, never entering background mode"
        );
        assert!(card.bg_pid.is_none(), "no background pid is tracked");
        assert!(card.bg_log.is_none(), "no background log is tracked");
        assert!(
            !card.is_background_running(),
            "the card is not a live background job"
        );
        assert!(
            card.output.contains("pid: 4242"),
            "the ack text is the completed output"
        );
    }

    #[test]
    fn tool_cards_redact_secret_arguments_and_outputs() {
        let secret = fake_secret();
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("run a tool".to_owned(), rx, flag());

        tx.send(TurnMessage::Event(RunnerEvent::ToolCall {
            id: "call_secret".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({ "command": format!("echo {secret}") }).to_string(),
        }))
        .unwrap();
        tx.send(TurnMessage::Event(RunnerEvent::ToolResult {
            id: "call_secret".to_owned(),
            name: "bash".to_owned(),
            parts: vec![tool_part(&format!("stdout: {secret}"))],
            is_error: false,
        }))
        .unwrap();
        a.poll_turn();

        let card = last_tool(&a);
        for rendered in [&card.summary, &card.args_detail, &card.output] {
            assert!(!rendered.contains(&secret), "tool card leaked: {rendered}");
            assert!(
                rendered.contains(redaction::PLACEHOLDER),
                "tool card should show placeholder: {rendered}"
            );
        }
    }

    #[test]
    fn a_failed_tool_call_renders_an_error_card() {
        use crate::tool_card::ToolStatus;
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("read a missing file".to_owned(), rx, flag());

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
        a.transcript.push(Entry::Tool(ToolCard::new(
            "tool_a",
            "read",
            r#"{"path":"a.txt"}"#,
        )));
        a.transcript.push(Entry::Tool(ToolCard::new(
            "tool_b",
            "read",
            r#"{"path":"b.txt"}"#,
        )));
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
        a.on_key(key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(
            !a.transcript.iter().any(|entry| matches!(
                entry,
                Entry::Tool(card) if card.id == "tool_a" && a.tool_card_expanded(card)
            )),
            "first card stays collapsed"
        );
        assert!(
            a.transcript.iter().any(|entry| matches!(
                entry,
                Entry::Tool(card) if card.id == "tool_b" && a.tool_card_expanded(card)
            )),
            "selected card expands"
        );
        a.on_key(key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(
            !a.transcript.iter().any(|entry| matches!(
                entry,
                Entry::Tool(card) if card.id == "tool_b" && a.tool_card_expanded(card)
            )),
            "Ctrl+O again collapses the selected card"
        );
    }

    #[test]
    fn tool_result_for_unknown_id_is_ignored() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("q".to_owned(), rx, flag());
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
        a.begin_turn("create a file".to_owned(), rx, flag());

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
        let names: Vec<String> = a.slash_matches().iter().map(|c| c.name.clone()).collect();
        assert!(names.iter().any(|name| name == "help"));
        assert!(names.iter().any(|name| name == "new"));
        assert!(names.iter().any(|name| name == "models"));
    }

    #[test]
    fn slash_menu_includes_gap_command_surfaces() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('/')));
        let names: Vec<String> = a
            .slash_matches()
            .iter()
            .map(|command| command.name.clone())
            .collect();
        for required in [
            "skills",
            "variant",
            "timeline",
            "fork",
            "subagents",
            "queue",
            "status",
            "rename",
            "unshare",
            "details",
        ] {
            assert!(
                names.iter().any(|name| name == required),
                "missing /{required} in slash menu: {names:?}"
            );
        }
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
        let names: Vec<String> = a.slash_matches().iter().map(|c| c.name.clone()).collect();
        assert!(
            names.iter().any(|name| name == "models"),
            "models present: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name == "help"),
            "help filtered out: {names:?}"
        );
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
    fn status_variant_and_skills_commands_open_dialogs() {
        let mut a = app();
        // `/status` now opens the full-screen status view (shared Wave-2 chrome), not a modal.
        type_str(&mut a, "/status");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.active_view(), Some(crate::view::ViewKind::Status));
        assert!(
            a.dialog().is_none(),
            "the status view is not a modal dialog"
        );
        a.on_key(key(KeyCode::Esc));
        assert!(a.active_view().is_none(), "Esc dismisses the status view");

        type_str(&mut a, "/variant");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            a.dialog().map(|dialog| dialog.kind),
            Some(DialogKind::Variants)
        );
        a.on_key(key(KeyCode::Esc));

        type_str(&mut a, "/skills");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            a.dialog().map(|dialog| dialog.kind),
            Some(DialogKind::Skills)
        );
    }

    #[test]
    fn rename_command_opens_rename_dialog_and_commits_new_title() {
        // VAL-VIEW-038/039/040: `/rename` opens the standalone rename dialog for the active session
        // (not the session picker); typing edits the new title live and Enter commits + persists it.
        let path = session_store_path("rename-command");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        run_turn(&mut a, "hello", "hi");
        let session_id = a.session_id.clone().expect("active session");

        type_str(&mut a, "/rename");
        a.on_key(key(KeyCode::Enter));

        assert!(
            a.rename_open(),
            "/rename opens the standalone rename dialog for the active session"
        );
        assert!(
            a.dialog().is_none(),
            "the rename dialog is not the session picker"
        );

        // Typing edits the in-progress new title live.
        type_str(&mut a, "fresh session title");
        assert!(
            a.rename_dialog()
                .is_some_and(|dialog| dialog.buffer.ends_with("fresh session title")),
            "typing appends to the rename buffer"
        );

        // Enter commits and closes; the new title is persisted to the store.
        a.on_key(key(KeyCode::Enter));
        assert!(!a.rename_open(), "Enter closes the rename dialog");
        let db = db::Database::open(&path).expect("reopen store");
        let session = db
            .sessions()
            .get(&session_id)
            .expect("get")
            .expect("session");
        assert_eq!(session.title.as_deref(), Some("fresh session title"));
    }

    #[test]
    fn rename_dialog_esc_cancels_without_persisting() {
        let path = session_store_path("rename-cancel");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        run_turn(&mut a, "hello", "hi");
        let session_id = a.session_id.clone().expect("active session");

        dispatch(&mut a, "rename");
        assert!(a.rename_open());
        type_str(&mut a, "discarded");
        a.on_key(key(KeyCode::Esc));
        assert!(!a.rename_open(), "Esc closes the rename dialog");
        let db = db::Database::open(&path).expect("reopen store");
        let session = db
            .sessions()
            .get(&session_id)
            .expect("get")
            .expect("session");
        assert!(
            session.title.is_none(),
            "Esc discards the typed title without persisting a rename"
        );
    }

    #[test]
    fn fork_dialog_uses_rich_message_rows_titled_fork_from_message() {
        // VAL-VIEW-011/012: `/fork` opens a modal titled `Fork from message` listing the session's
        // past messages as rich rows (a key column + excerpt), one row per persisted message.
        let path = session_store_path("fork-rows");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        run_turn(&mut a, "first question", "first answer");
        run_turn(&mut a, "second question", "second answer");

        dispatch(&mut a, "fork");
        let dialog = a.dialog().expect("fork dialog open");
        assert_eq!(dialog.kind, DialogKind::Fork);
        assert_eq!(dialog.title, "Fork from message");
        assert_eq!(
            a.picker_rows().len(),
            4,
            "one rich row per persisted message"
        );
        // Each row carries a `who` (you/assistant) in its key and an excerpt in its description.
        let first = &a.picker_rows()[0];
        assert!(
            first.key.contains("you"),
            "first row is the user turn: {first:?}"
        );
        assert_eq!(first.desc.as_deref(), Some("first question"));
    }

    #[test]
    fn export_dialog_lists_formats_with_default_badge() {
        // VAL-VIEW-035/036: `/export` opens a modal titled `Export session` listing the formats as
        // selectable rows, with the default (Markdown) row carrying a `default` badge as meta.
        let mut a = app();
        dispatch(&mut a, "export");
        let dialog = a.dialog().expect("export dialog open");
        assert_eq!(dialog.kind, DialogKind::Export);
        assert_eq!(dialog.title, "Export session");
        let rows = a.picker_rows();
        assert_eq!(rows.len(), 4, "Markdown/JSON/HTML/Patch");
        assert_eq!(rows[0].key, "Markdown");
        assert_eq!(
            rows[0].meta.as_deref(),
            Some("default"),
            "the default format is badged"
        );
        assert!(
            rows[1..].iter().all(|row| row.meta.is_none()),
            "only the default format carries a badge"
        );
        // Selection starts on the default and moves with Down.
        assert_eq!(dialog.selected(), 0);
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.dialog().expect("still open").selected(), 1);
        // Esc dismisses without exporting.
        a.on_key(key(KeyCode::Esc));
        assert!(!a.dialog_open(), "Esc closes the export dialog");
    }

    #[test]
    fn export_format_renderers_produce_expected_shapes() {
        // The chosen export format renders the transcript into that format (JSON messages, HTML
        // sections, unified patch) so Enter writes the selected format's content.
        let mut a = app();
        a.transcript.push(Entry::User("ping".to_owned()));
        a.transcript.push(Entry::Assistant(AssistantTurn {
            reasoning: String::new(),
            answer: "pong".to_owned(),
        }));

        let json = a.render_session_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON export");
        let messages = parsed["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "ping");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "pong");

        let html = a.render_session_html();
        assert!(html.starts_with("<!doctype html>"), "html export: {html}");
        assert!(html.contains("<pre>ping</pre>"));
        assert!(html.contains("<pre>pong</pre>"));

        // HTML metacharacters in content are escaped.
        let mut b = app();
        b.transcript.push(Entry::User("a < b && c > d".to_owned()));
        let escaped = b.render_session_html();
        assert!(
            escaped.contains("a &lt; b &amp;&amp; c &gt; d"),
            "escaped: {escaped}"
        );
    }

    fn render_screen(a: &App, width: u16, height: u16) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, a)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn export_dialog_renders_default_badge_and_destination_note() {
        // VAL-VIEW-035/036: the export modal renders its title, all format rows, the `default`
        // badge, and the `→ ~/cortex/exports` destination note.
        let mut a = app();
        dispatch(&mut a, "export");
        let screen = render_screen(&a, 92, 30);
        for needle in [
            "Export session",
            "Markdown",
            "JSON",
            "HTML",
            "Patch",
            "default",
            "→ ~/cortex/exports",
        ] {
            assert!(
                screen.contains(needle),
                "export modal shows `{needle}`:\n{screen}"
            );
        }
        // Safe glyphs only: no tofu, no braille, no thin triangles.
        assert!(!screen.contains('▸') && !screen.contains('▾'));
    }

    #[test]
    fn fork_dialog_renders_title_and_new_branch_note() {
        // VAL-VIEW-011/012: the fork modal renders `Fork from message` and the `new branch` note.
        let path = session_store_path("fork-render");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        run_turn(&mut a, "cap the streaming buffer", "added a 64KB cap");
        dispatch(&mut a, "fork");
        let screen = render_screen(&a, 92, 30);
        assert!(
            screen.contains("Fork from message"),
            "fork title:\n{screen}"
        );
        assert!(screen.contains("new branch"), "branch note:\n{screen}");
        // The user turn's excerpt is listed.
        assert!(
            screen.contains("cap the streaming buffer"),
            "message excerpt listed:\n{screen}"
        );
    }

    #[test]
    fn rename_dialog_renders_current_title_input_and_footer() {
        // VAL-VIEW-038/039: the rename dialog quotes the current title (straight quotes), shows the
        // input with a `▏` caret, and offers save/cancel.
        let path = session_store_path("rename-render");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        run_turn(&mut a, "hello", "hi");
        let id = a.session_id.clone().expect("session id");
        {
            let db = db::Database::open(&path).expect("open store");
            db.sessions()
                .set_title(&id, Some("fix streaming buffer limit"))
                .expect("seed title");
        }
        dispatch(&mut a, "rename");
        assert!(a.rename_open(), "rename dialog open");
        let screen = render_screen(&a, 92, 30);
        assert!(screen.contains("Rename session"), "title:\n{screen}");
        assert!(
            screen.contains("fix streaming buffer limit"),
            "current title quoted:\n{screen}"
        );
        assert!(
            screen.contains('"'),
            "straight quotes around the current title"
        );
        assert!(screen.contains('▏'), "the input caret renders:\n{screen}");
        assert!(
            screen.contains("save") && screen.contains("cancel"),
            "save/cancel hints:\n{screen}"
        );
        // No curly/smart quotes.
        for curly in ['“', '”', '‘', '’'] {
            assert!(!screen.contains(curly), "no curly quote {curly:?}");
        }
    }

    #[test]
    fn details_command_toggles_only_the_selected_tool_card() {
        let mut a = app();
        a.transcript.push(Entry::Tool(ToolCard::new(
            "tool_1",
            "read",
            r#"{"path":"a.txt"}"#,
        )));
        a.transcript.push(Entry::Tool(ToolCard::new(
            "tool_2",
            "read",
            r#"{"path":"b.txt"}"#,
        )));
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));

        type_str(&mut a, "/details");
        a.on_key(key(KeyCode::Enter));

        let first = a
            .transcript
            .iter()
            .find_map(|entry| match entry {
                Entry::Tool(card) if card.id == "tool_1" => Some(card),
                _ => None,
            })
            .expect("tool_1");
        let second = a
            .transcript
            .iter()
            .find_map(|entry| match entry {
                Entry::Tool(card) if card.id == "tool_2" => Some(card),
                _ => None,
            })
            .expect("tool_2");

        assert!(!a.tool_card_expanded(first));
        assert!(a.tool_card_expanded(second));
    }

    #[test]
    fn config_custom_command_is_listed_and_dispatched() {
        let mut a = app();
        a.custom_commands = vec![CustomSlashCommand {
            name: "daily".to_owned(),
            description: "Run daily custom routine".to_owned(),
            prompt: "Summarize today's progress.".to_owned(),
        }];

        type_str(&mut a, "/daily");
        let names: Vec<String> = a
            .slash_matches()
            .iter()
            .map(|command| command.name.clone())
            .collect();
        assert!(names.iter().any(|name| name == "daily"));

        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            a.take_pending_prompt().as_deref(),
            Some("Summarize today's progress.")
        );
        assert!(
            a.transcript.iter().any(
                |entry| matches!(entry, Entry::Notice(text) if text.contains("custom command"))
            ),
            "custom-command notice should be visible"
        );
    }

    #[test]
    fn parse_custom_commands_reads_prompt_description_and_template_fields() {
        let value: toml::Value = toml::from_str(
            r#"
            [command.daily]
            description = "Run daily routine"
            prompt = "Summarize today's progress."

            [command.audit]
            template = "Review this repository for security issues."
            "#,
        )
        .expect("valid toml");
        let parsed = parse_custom_slash_commands(value.get("command"));
        assert_eq!(parsed.len(), 2);
        assert!(parsed.iter().any(|command| command.name == "daily"
            && command.description == "Run daily routine"
            && command.prompt.contains("Summarize")));
        assert!(parsed.iter().any(|command| command.name == "audit"
            && command.description == "Run custom command"
            && command.prompt.contains("security issues")));
    }

    #[test]
    fn parse_custom_theme_document_accepts_partial_overrides() {
        let parsed = parse_custom_theme_document(
            r##"
            {
              "name": "nebula_custom",
              "extends": "dark",
              "primary": "#ff00ff",
              "surface": ["#0a0a0a", "#141414", "#1e1e1e", "#282828"]
            }
            "##,
        )
        .expect("theme document parses");
        assert_eq!(parsed.len(), 1);
        let (name, theme) = &parsed[0];
        assert_eq!(name, "nebula_custom");
        assert_eq!(theme.primary, ratatui::style::Color::Rgb(255, 0, 255));
        assert_eq!(theme.surface[0], ratatui::style::Color::Rgb(10, 10, 10));
    }

    #[test]
    fn parse_custom_theme_document_rejects_invalid_hex_color() {
        let err = parse_custom_theme_document(
            r##"
            {
              "name": "broken",
              "primary": "#zz00zz"
            }
            "##,
        )
        .expect_err("invalid color should fail");
        assert!(
            err.contains("primary"),
            "error points to offending color key: {err}"
        );
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

    #[test]
    fn attach_command_rejects_unsupported_files_without_clearing_prompt_text() {
        let mut a = app();
        let file = temp_attachment("txt", b"not a supported attachment");
        let command = format!("/attach {}", file.display());
        a.set_input_value(&command);
        a.on_key(key(KeyCode::Enter));

        assert!(
            a.composer_attachments().is_empty(),
            "unsupported files must not be staged"
        );
        assert_eq!(
            a.input_value(),
            command,
            "failed attachment keeps the typed prompt text intact"
        );
        assert!(
            matches!(a.transcript.last(), Some(Entry::Error(message)) if message.contains("Attachment error")),
            "a visible attachment error is surfaced"
        );
    }

    #[test]
    fn attach_command_stages_supported_media_in_the_composer() {
        let mut a = app();
        let png = temp_attachment(
            "png",
            &[
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
                b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0,
                0, 0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05,
                0x01, 0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42,
                0x60, 0x82,
            ],
        );
        a.set_input_value(&format!("/attach {}", png.display()));
        a.on_key(key(KeyCode::Enter));

        assert_eq!(a.input_value(), "", "successful attach clears the command");
        assert_eq!(a.composer_attachments().len(), 1, "one attachment staged");
        assert_eq!(a.composer_attachments()[0].media_type, "image/png");
    }

    #[test]
    fn submitting_with_staged_attachments_emits_pending_user_parts() {
        let mut a = app();
        let pdf = temp_attachment("pdf", b"%PDF-1.7\n");
        a.set_input_value(&format!("/attach {}", pdf.display()));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.composer_attachments().len(), 1, "attachment staged");

        a.set_input_value("What is in this file?");
        a.on_key(key(KeyCode::Enter));

        assert_eq!(
            a.take_pending_prompt().as_deref(),
            Some("What is in this file?")
        );
        let parts = a.pending_user_parts_for_turn();
        assert!(
            parts.iter().any(|part| matches!(
                part,
                MessageContentPart::Document { media_type, path }
                    if media_type == "application/pdf" && path.ends_with(".pdf")
            )),
            "submitted turn carries a document content part: {parts:?}"
        );
        assert!(
            a.composer_attachments().is_empty(),
            "attachments move out of the composer once submitted"
        );
    }

    #[test]
    fn attachments_are_visible_in_transcript_and_history_after_turn_completion() {
        let mut a = app();
        let png = temp_attachment(
            "png",
            &[
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
                b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0,
                0, 0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05,
                0x01, 0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42,
                0x60, 0x82,
            ],
        );
        a.set_input_value(&format!("/attach {}", png.display()));
        a.on_key(key(KeyCode::Enter));
        a.set_input_value("Describe the attachment.");
        a.on_key(key(KeyCode::Enter));
        let prompt = a.take_pending_prompt().expect("queued turn prompt");

        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(prompt, rx, flag());
        assert!(
            matches!(
                a.transcript.last(),
                Some(Entry::Assistant(AssistantTurn { .. }))
            ),
            "assistant placeholder is present"
        );
        assert!(
            a.transcript
                .iter()
                .any(|entry| matches!(entry, Entry::User(text) if text.contains("PNG") && text.contains(".png"))),
            "user transcript entry surfaces the attachment reference"
        );

        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "Looks like a tiny PNG.".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        let history = a.history_snapshot();
        assert!(
            history.iter().any(|message| {
                message.role == llm::Role::User
                    && message.parts.iter().any(|part| {
                        matches!(
                            part,
                            MessageContentPart::Image { media_type, path }
                                if media_type == "image/png" && path.ends_with(".png")
                        )
                    })
            }),
            "finalized history keeps the user image part for follow-up turns: {history:?}"
        );
    }

    #[test]
    fn errored_attachment_turn_keeps_prompt_text_but_drops_media_from_history() {
        let mut a = app();
        let png = temp_attachment(
            "png",
            &[
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
                b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0,
                0, 0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05,
                0x01, 0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42,
                0x60, 0x82,
            ],
        );
        a.set_input_value(&format!("/attach {}", png.display()));
        a.on_key(key(KeyCode::Enter));
        a.set_input_value("Please describe this attachment.");
        a.on_key(key(KeyCode::Enter));
        let prompt = a.take_pending_prompt().expect("queued prompt");

        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(prompt, rx, flag());
        tx.send(TurnMessage::SetupError(
            "Unsupported attachment media `application/pdf` for protocol `openai-chat`.".to_owned(),
        ))
        .expect("setup error");
        tx.send(TurnMessage::Finished).expect("finish");
        a.poll_turn();

        let history = a.history_snapshot();
        assert!(
            history.iter().any(|message| {
                message.role == llm::Role::User
                    && message.content == "Please describe this attachment."
                    && message.parts.is_empty()
            }),
            "errored media turns keep prompt text but drop attachments from history: {history:?}"
        );
    }

    // VAL-GAP-CROSS-001 (TUI input half): attaching an image and completing a turn persists a typed
    // File media part (image/png, source "tui_attachment") alongside the answer and usage to the
    // shared store, so the exchange survives a process restart with the media metadata intact and no
    // binary bytes or pasted secrets leaking into the persisted record. The CLI/server export half is
    // locked in crates/cli/tests/gap_cross_multimodal_tui_provider_persistence.rs; the live
    // end-to-end flow is proven black-box with tuistory.
    #[test]
    fn tui_attachment_turn_persists_typed_media_usage_and_answer_across_restart() {
        let path = session_store_path("multimodal-cross");
        let png = temp_attachment(
            "png",
            &[
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
                b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0,
                0, 0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05,
                0x01, 0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42,
                0x60, 0x82,
            ],
        );
        let secret = fake_secret();
        let mut a = app();
        a.set_session_store(Some(path.clone()));

        a.set_input_value(&format!("/attach {}", png.display()));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            a.composer_attachments().len(),
            1,
            "image staged in composer"
        );

        a.set_input_value(&format!(
            "Describe this swatch. Ignore this token: {secret}"
        ));
        a.on_key(key(KeyCode::Enter));
        let prompt = a.take_pending_prompt().expect("queued multimodal prompt");

        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(prompt, rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "The swatch is solid red.".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Event(RunnerEvent::Usage(usage(
            2666,
            1,
            Some(0.0008),
        ))))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        let id = a.session_id.clone().expect("session created");

        // Reopen the shared store as a fresh process would on restart.
        let db = db::Database::open(&path).expect("reopen store");
        let messages = db.messages().list_for_session(&id).expect("messages");
        assert_eq!(messages.len(), 2, "user + assistant persisted");

        let user = &messages[0];
        assert_eq!(user.role, "user");
        let file_part = user
            .parts
            .iter()
            .find_map(|part| match &part.body {
                protocol::MessagePartBody::File {
                    path,
                    media_type,
                    source,
                } => Some((path.clone(), media_type.clone(), source.clone())),
                _ => None,
            })
            .expect("persisted typed file media part");
        assert!(
            file_part.0.ends_with(".png"),
            "media part path references the attachment: {}",
            file_part.0
        );
        assert_eq!(file_part.1.as_deref(), Some("image/png"));
        assert_eq!(
            file_part.2.as_deref(),
            Some("tui_attachment"),
            "the part records its TUI-attachment provenance"
        );

        let assistant = &messages[1];
        assert_eq!(assistant.role, "assistant");
        assert!(
            assistant.content.contains("solid red"),
            "the answer is persisted: {}",
            assistant.content
        );

        // The secret pasted into the prompt is scrubbed from the persisted record (content + parts),
        // and no base64/binary image bytes leak into the persisted text channel.
        let serialized = serde_json::to_string(&messages).expect("serialize messages");
        assert!(
            !serialized.contains(&secret),
            "the pasted secret must be redacted from the persisted record"
        );
        assert!(
            serialized.contains(redaction::PLACEHOLDER),
            "the canonical redaction placeholder marks the scrubbed secret"
        );
        assert!(
            !serialized.contains("base64,"),
            "no base64 image payload leaks into the persisted text channel"
        );

        let totals = db.usage().session_totals(&id).expect("usage totals");
        assert_eq!(totals.input_tokens, 2666);
        assert_eq!(totals.output_tokens, 1);
        assert_eq!(totals.total_tokens, 2667);
        assert_eq!(totals.cost, Some(0.0008));
    }

    /// Run a complete throwaway turn so the app accumulates transcript + history state.
    fn run_turn(a: &mut App, prompt: &str, answer: &str) {
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(prompt.to_owned(), rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: answer.to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();
    }

    #[test]
    fn dispatching_help_opens_the_full_screen_help_view() {
        // VAL-TUI-032 / VAL-VIEW-028: `/help` opens the full-screen command-reference view (not a
        // model turn), titled `Commands & shortcuts` with the `/help` crumb.
        let mut a = app();
        type_str(&mut a, "/help");
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.take_pending_prompt().is_none(),
            "help is not a model turn"
        );
        assert_eq!(
            a.active_view(),
            Some(crate::view::ViewKind::Help),
            "the full-screen help view is open"
        );
        // Esc closes it back to the conversation.
        a.on_key(key(KeyCode::Esc));
        assert!(a.active_view().is_none(), "Esc dismisses the help view");
    }

    #[test]
    fn question_mark_opens_help_on_an_empty_composer() {
        // VAL-FOUND-033: pressing `?` with the composer empty opens the help/command reference.
        let mut a = app();
        a.on_key(key(KeyCode::Char('?')));
        assert_eq!(
            a.active_view(),
            Some(crate::view::ViewKind::Help),
            "`?` on an empty composer opens the help view"
        );
    }

    #[test]
    fn question_mark_is_literal_text_in_a_non_empty_composer() {
        // VAL-FOUND-033: `?` is only the help affordance when the composer is empty; while typing it
        // is an ordinary character (so a user can type `why?`), not a help trigger.
        let mut a = app();
        type_str(&mut a, "why");
        a.on_key(key(KeyCode::Char('?')));
        assert!(a.active_view().is_none(), "`?` in text does not open help");
        assert_eq!(a.input_value(), "why?", "`?` is inserted as literal text");
    }

    #[test]
    fn question_mark_opens_help_from_a_full_screen_view() {
        // VAL-FOUND-033: the `?` affordance behaves identically from a full-screen view — pressing
        // `?` while (e.g.) the status view is open switches to the help reference.
        let mut a = app();
        type_str(&mut a, "/status");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.active_view(), Some(crate::view::ViewKind::Status));
        a.on_key(key(KeyCode::Char('?')));
        assert_eq!(
            a.active_view(),
            Some(crate::view::ViewKind::Help),
            "`?` from a full-screen view opens the help reference"
        );
    }

    #[test]
    fn status_view_permits_row_reflects_the_real_posture() {
        // The `/status` permits row is wired to the real permission posture (via the engine), not a
        // static placeholder: the default build agent ask-gates edit/bash and allows web.
        let mut a = app();
        type_str(&mut a, "/status");
        a.on_key(key(KeyCode::Enter));
        let data = a.status_snapshot().expect("status snapshot after /status");
        assert_eq!(
            data.permits,
            format!(
                "edit ask {sep} bash ask {sep} web allow",
                sep = crate::glyphs::SEPARATOR
            ),
            "permits reflects the real classification"
        );
        assert_ne!(
            data.permits, "edit allow · bash allow-all · web allow",
            "permits is no longer the static placeholder"
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
    fn dispatching_init_opens_the_scan_dialog_then_confirm_queues_a_real_flow_turn() {
        // VAL-VIEW-041/043: `/init` opens the AGENTS.md scan dialog (not a silent write); confirming
        // (Enter) runs the real project-initialization flow that writes AGENTS.md.
        let mut a = app();
        type_str(&mut a, "/init");
        a.on_key(key(KeyCode::Enter));

        // The scan dialog opens (a view, not a silent flow) and no turn is queued yet.
        assert!(a.init_open(), "/init opens the scan dialog");
        assert!(
            a.take_pending_prompt().is_none(),
            "no turn is queued until the dialog is confirmed"
        );
        let dialog = a.init_dialog().expect("init dialog state");
        assert!(!dialog.steps.is_empty(), "the scan log has steps");
        assert!(
            dialog.steps.last().is_some_and(|s| !s.done),
            "the AGENTS.md generation step is the active (pending) step"
        );

        // Confirming the dialog closes it and queues the real init flow turn (AGENTS.md).
        a.on_key(key(KeyCode::Enter));
        assert!(!a.init_open(), "confirming closes the init dialog");
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::Notice(t) if t.contains("/init"))),
            "an /init flow notice is shown on confirm"
        );
        let prompt = a
            .take_pending_prompt()
            .expect("an /init turn is queued on confirm");
        assert!(
            prompt.contains("AGENTS.md"),
            "the init instruction is sent: {prompt}"
        );
        // The transcript shows the concise header, not the verbose instruction.
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(prompt, rx, flag());
        assert!(
            a.transcript
                .iter()
                .any(|e| matches!(e, Entry::User(t) if t == "/init")),
            "the concise /init header is shown as the user message"
        );
    }

    #[test]
    fn dispatching_init_esc_cancels_without_queuing_a_turn() {
        // VAL-VIEW-041: Esc dismisses the init scan dialog without initializing.
        let mut a = app();
        type_str(&mut a, "/init");
        a.on_key(key(KeyCode::Enter));
        assert!(a.init_open());
        a.on_key(key(KeyCode::Esc));
        assert!(!a.init_open(), "Esc closes the init dialog");
        assert!(a.take_pending_prompt().is_none(), "Esc queues no turn");
    }

    #[test]
    fn dispatching_review_opens_the_review_view() {
        // VAL-VIEW-044: `/review` opens the full-screen code-review view (not a silent flow); no
        // turn is queued and the view carries the review data snapshot.
        let mut a = app();
        type_str(&mut a, "/review");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            a.active_view(),
            Some(crate::view::ViewKind::Review),
            "/review opens the review view"
        );
        assert!(
            a.review_snapshot().is_some(),
            "the review data is snapshotted"
        );
        assert!(
            a.take_pending_prompt().is_none(),
            "/review does not queue a turn (it renders as a view)"
        );
        // Esc closes the view back to the conversation.
        a.on_key(key(KeyCode::Esc));
        assert!(a.active_view().is_none(), "Esc closes the review view");
    }

    #[test]
    fn dispatching_compact_opens_the_dialog_then_confirm_queues_a_compaction_turn_that_replaces_history()
     {
        // VAL-VIEW-032/033/034: `/compact` opens the before/after dialog (not a silent flow);
        // confirming (Enter) queues the compaction turn whose summary replaces history.
        let mut a = app();
        run_turn(&mut a, "fact one", "ok");
        run_turn(&mut a, "fact two", "ok");
        assert_eq!(a.history_snapshot().len(), 4, "two turns of history");

        type_str(&mut a, "/compact");
        a.on_key(key(KeyCode::Enter));
        // The dialog opens with the before/after comparison; no turn queued yet.
        assert!(a.compact_open(), "/compact opens the compaction dialog");
        assert!(
            a.take_pending_prompt().is_none(),
            "no compaction turn is queued until confirmed"
        );
        let dialog = a.compact_dialog().expect("compact dialog state");
        assert!(
            dialog.before_tokens.contains("tokens"),
            "before token count shown"
        );
        assert!(dialog.after_tokens.starts_with('~'), "after is an estimate");
        assert!(
            dialog.before_detail.contains("message"),
            "message count shown"
        );

        // Confirming closes the dialog and queues the compaction turn.
        a.on_key(key(KeyCode::Enter));
        assert!(!a.compact_open(), "confirming closes the compaction dialog");
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
        a.begin_turn(prompt, rx, flag());
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
        // VAL-TUI-041 / VAL-VIEW-035/037: `/export` opens the format modal; confirming (Enter)
        // exports the highlighted (default Markdown) format and confirms with a path.
        let mut a = app();
        run_turn(&mut a, "hello", "hi there");
        type_str(&mut a, "/export");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            a.dialog().map(|dialog| dialog.kind),
            Some(DialogKind::Export),
            "/export opens the export format modal (not a silent write)"
        );
        // Confirm the highlighted format; the modal closes and the export is confirmed with a path.
        a.on_key(key(KeyCode::Enter));
        assert!(!a.dialog_open(), "confirming closes the export modal");
        match a.transcript.last() {
            Some(Entry::Notice(text)) => {
                assert!(
                    text.contains("Exported session") && text.contains("Markdown"),
                    "an export confirmation with the format + path: {text}"
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
        a.begin_turn("busy".to_owned(), rx, flag());
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

    fn queue_rows(a: &App) -> Vec<String> {
        a.queue_snapshot().to_vec()
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
            dialog
                .items
                .iter()
                .any(|m| m.starts_with("google/gemini-2.5-flash")),
            "the default model is listed: {:?}",
            dialog.items
        );
        // The active model is pre-highlighted.
        assert!(
            dialog.items[dialog.selected()].starts_with("google/gemini-2.5-flash"),
            "active model pre-highlighted: {}",
            dialog.items[dialog.selected()]
        );
    }

    #[test]
    fn dispatching_agents_opens_the_agent_picker() {
        // VAL-TUI-036: `/agents` opens an agent picker with multiple agents.
        let mut a = app();
        dispatch(&mut a, "agents");
        let dialog = a.dialog().expect("the agent picker opened");
        assert_eq!(dialog.kind, DialogKind::Agents);
        for expected in ["build", "plan", "general", "explore"] {
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
    fn custom_runtime_theme_appears_in_the_theme_picker() {
        frame::style::ThemeColors::clear_custom_themes();
        let mut custom = frame::style::ThemeColors::dark();
        custom.primary = ratatui::style::Color::Rgb(255, 0, 255);
        frame::style::ThemeColors::register_custom_theme("nebula_custom", custom);

        let mut a = app();
        dispatch(&mut a, "themes");
        let dialog = a.dialog().expect("theme picker opened");
        assert!(
            dialog.items.iter().any(|name| name == "nebula_custom"),
            "custom runtime theme appears in picker: {:?}",
            dialog.items
        );
        frame::style::ThemeColors::clear_custom_themes();
    }

    #[test]
    fn theme_picker_lists_the_five_brand_themes_with_descriptors_and_characters() {
        // VAL-PICK-044/045/050: exactly the five real themes in reference order, each with its
        // descriptor and a right-aligned character tag.
        frame::style::ThemeColors::clear_custom_themes();
        let mut a = app();
        dispatch(&mut a, "themes");
        let dialog = a.dialog().expect("theme picker opened");
        assert_eq!(dialog.kind, DialogKind::Themes);
        assert_eq!(
            dialog.items,
            vec!["dark", "light", "vscode-dark", "monokai", "ocean"],
            "the five themes appear in reference order with no extras"
        );
        let rows = a.picker_rows();
        let descriptor = |name: &str| {
            rows.iter()
                .find(|row| row.key == name)
                .and_then(|row| row.desc.clone())
                .unwrap_or_default()
        };
        let character = |name: &str| {
            rows.iter()
                .find(|row| row.key == name)
                .and_then(|row| row.meta.clone())
                .unwrap_or_default()
        };
        assert_eq!(descriptor("dark"), "cortex default");
        assert_eq!(descriptor("vscode-dark"), "familiar");
        assert_eq!(character("dark"), "warm");
        assert_eq!(character("monokai"), "high-contrast");
        assert_eq!(character("ocean"), "calm");
    }

    #[test]
    fn selecting_vscode_dark_applies_the_vscode_chrome() {
        // VAL-PICK-048: committing `vscode-dark` re-renders the UI in the vscode chrome (its
        // display name resolves to the registered `vscode` theme without error).
        let mut a = app();
        dispatch(&mut a, "themes");
        // Move the highlight onto the `vscode-dark` row.
        let idx = a
            .dialog()
            .expect("theme picker")
            .items
            .iter()
            .position(|name| name == "vscode-dark")
            .expect("vscode-dark row present");
        if let Some(dialog) = a.dialog.as_mut() {
            dialog.selected = idx;
        }
        a.sync_theme_preview();
        let confirmed = a.confirm_theme_selection().expect("theme committed");
        assert_eq!(confirmed, "vscode-dark");
        let applied = frame::style::ThemeColors::from_name(a.theme());
        assert_eq!(
            applied.background,
            frame::style::ThemeColors::vscode().background,
            "vscode-dark resolves to the vscode chrome palette"
        );
    }

    /// Seed a parent session with three child subagents (running / queued / done) and load it as the
    /// active session, returning `(app, done_child_id)`.
    fn app_with_seeded_subagents(tag: &str) -> (App, String) {
        let path = session_store_path(tag);
        let parent_id = "ses_parent_subpick";
        let done_id;
        {
            let db = db::Database::open(&path).expect("open seed");
            db.sessions()
                .ensure(parent_id, Some("parent"))
                .expect("parent session");
            let running = db
                .sessions()
                .create_child(
                    parent_id,
                    Some(&format!(
                        "find existing auth middleware (subagent:explore parent:{parent_id})"
                    )),
                )
                .expect("running child");
            db.messages()
                .create(&running.id, "user", "go")
                .expect("running user message");
            db.sessions()
                .create_child(
                    parent_id,
                    Some(&format!(
                        "design token-refresh flow (subagent:plan parent:{parent_id})"
                    )),
                )
                .expect("queued child");
            let done = db
                .sessions()
                .create_child(
                    parent_id,
                    Some(&format!(
                        "summarize redaction audit (subagent:general parent:{parent_id})"
                    )),
                )
                .expect("done child");
            db.messages()
                .create(&done.id, "assistant", "audit complete")
                .expect("done assistant message");
            done_id = done.id.clone();
        }
        let mut a = app();
        a.set_session_store(Some(path));
        a.load_session(parent_id);
        (a, done_id)
    }

    #[test]
    fn dispatching_subagents_opens_the_subagent_picker_with_real_children() {
        // VAL-PICK-051/052: `/subagents` opens the modal titled `Subagents`, listing the live child
        // subagents with their agent names and task descriptions.
        let (mut a, _done) = app_with_seeded_subagents("subpick-open");
        dispatch(&mut a, "subagents");
        let dialog = a.dialog().expect("the subagent picker opened");
        assert_eq!(dialog.kind, DialogKind::Subagents);
        assert_eq!(dialog.title, "Subagents");
        let rows = a.picker_rows();
        for name in ["explore", "plan", "general"] {
            assert!(
                rows.iter().any(|row| row.key == name),
                "subagent {name} listed: {rows:?}"
            );
        }
        let explore = rows
            .iter()
            .find(|row| row.key == "explore")
            .expect("explore row");
        assert_eq!(
            explore.desc.as_deref(),
            Some("find existing auth middleware"),
            "task description shown"
        );
    }

    #[test]
    fn subagent_status_meta_uses_semantic_colors() {
        // VAL-PICK-053/054: running → AMBER, queued → FAINT, done → GREEN; green appears only on
        // done.
        let (mut a, _done) = app_with_seeded_subagents("subpick-colors");
        let theme = frame::style::ThemeColors::from_name(a.theme());
        dispatch(&mut a, "subagents");
        let rows = a.picker_rows().to_vec();
        let row = |name: &str| rows.iter().find(|row| row.key == name).expect("row");
        let running = row("explore");
        assert!(
            running
                .meta
                .as_deref()
                .unwrap_or_default()
                .starts_with("running"),
            "running status word: {:?}",
            running.meta
        );
        assert_eq!(
            running.meta_color,
            Some(theme.secondary),
            "running is AMBER"
        );
        let queued = row("plan");
        assert_eq!(queued.meta.as_deref(), Some("queued"));
        assert_eq!(queued.meta_color, Some(theme.text_muted), "queued is FAINT");
        let done = row("general");
        assert!(
            done.meta.as_deref().unwrap_or_default().starts_with("done"),
            "done status word: {:?}",
            done.meta
        );
        assert_eq!(done.meta_color, Some(theme.success), "done is GREEN");
        // Green appears on the done status only.
        let green_rows = rows
            .iter()
            .filter(|row| row.meta_color == Some(theme.success))
            .count();
        assert_eq!(green_rows, 1, "green is used only for the done status");
    }

    #[test]
    fn subagent_picker_enter_views_the_highlighted_child() {
        // VAL-PICK-056: Enter (`↵ view`) loads the highlighted subagent's session transcript.
        let (mut a, done_id) = app_with_seeded_subagents("subpick-view");
        dispatch(&mut a, "subagents");
        let idx = a
            .picker_rows()
            .iter()
            .position(|row| row.key == "general")
            .expect("done subagent row");
        if let Some(dialog) = a.dialog.as_mut() {
            dialog.selected = idx;
        }
        a.on_key(key(KeyCode::Enter));
        assert!(!a.dialog_open(), "Enter closes the picker");
        assert_eq!(
            a.status.session, done_id,
            "viewing loads the highlighted subagent's session"
        );
    }

    #[test]
    fn subagent_picker_ctrl_c_cancels_running_without_quitting() {
        // VAL-PICK-056: `^C` on a running subagent requests cancel (a notice), and does NOT quit.
        let (mut a, _done) = app_with_seeded_subagents("subpick-cancel");
        dispatch(&mut a, "subagents");
        let idx = a
            .picker_rows()
            .iter()
            .position(|row| row.key == "explore")
            .expect("running subagent row");
        if let Some(dialog) = a.dialog.as_mut() {
            dialog.selected = idx;
        }
        a.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!a.should_quit, "^C in the subagent picker never quits");
        assert!(
            a.transcript
                .iter()
                .any(|entry| matches!(entry, Entry::Notice(text) if text.contains("cancel"))),
            "a cancel request is surfaced"
        );
    }

    #[test]
    fn subagent_picker_esc_closes_without_changing_state() {
        // VAL-PICK-057: esc closes the picker and returns to the conversation unchanged.
        let (mut a, _done) = app_with_seeded_subagents("subpick-esc");
        let before = a.status.session.clone();
        dispatch(&mut a, "subagents");
        a.on_key(key(KeyCode::Esc));
        assert!(!a.dialog_open(), "esc closes the subagent picker");
        assert_eq!(a.status.session, before, "the active session is unchanged");
    }

    #[test]
    fn dispatching_mcp_opens_the_status_view_with_an_empty_state() {
        // VAL-TUI-046 / VAL-VIEW-054: `/mcp` opens the full-screen MCP view; with no servers its
        // body is empty so the view renders its explicit empty state (never a blank panel).
        let mut a = app();
        dispatch(&mut a, "mcp");
        assert_eq!(a.active_view(), Some(crate::view::ViewKind::Mcp));
        assert!(a.dialog().is_none(), "the MCP view is not a modal dialog");
        assert!(
            a.mcp_view_rows().is_empty(),
            "no servers are configured, so the view shows its empty state"
        );
    }

    #[test]
    fn mcp_server_rows_map_states_and_expose_capabilities() {
        // VAL-VIEW-020/021: connected servers carry a `ready` tone + tool count + capability list;
        // a failed server carries an `error` tone with no tool count and its error detail as the
        // exposes text. Resources (by URI) and prompts (by name) join the exposed capabilities.
        let statuses = vec![
            mcp::ServerStatus {
                name: "context7".to_owned(),
                state: mcp::ConnectionState::Connected,
                pid: Some(1),
                error: None,
                tools: Vec::new(),
                resources: Vec::new(),
                prompts: Vec::new(),
            },
            mcp::ServerStatus {
                name: "github".to_owned(),
                state: mcp::ConnectionState::Failed,
                pid: None,
                error: Some("auth failed — run: cortex auth login".to_owned()),
                tools: Vec::new(),
                resources: Vec::new(),
                prompts: Vec::new(),
            },
            mcp::ServerStatus {
                name: "oauth".to_owned(),
                state: mcp::ConnectionState::NeedsAuth,
                pid: None,
                error: Some(
                    "Bearer opaque_challenge=\"challenge-must-not-reach-the-tui\"".to_owned(),
                ),
                tools: Vec::new(),
                resources: Vec::new(),
                prompts: Vec::new(),
            },
        ];
        let tools = vec![
            mcp::McpTool {
                id: "context7_resolve".to_owned(),
                server: "context7".to_owned(),
                tool: "resolve-library-id".to_owned(),
                description: None,
                input_schema: serde_json::json!({"type":"object"}),
            },
            mcp::McpTool {
                id: "context7_query".to_owned(),
                server: "context7".to_owned(),
                tool: "query-docs".to_owned(),
                description: None,
                input_schema: serde_json::json!({"type":"object"}),
            },
        ];
        let resources = vec![mcp::McpResource {
            id: "context7_notes".to_owned(),
            server: "context7".to_owned(),
            uri: "notes://c7".to_owned(),
            name: "notes".to_owned(),
            title: None,
            description: None,
            mime_type: None,
        }];
        let prompts = vec![mcp::McpPrompt {
            id: "context7_greet".to_owned(),
            server: "context7".to_owned(),
            name: "greet".to_owned(),
            title: None,
            description: None,
            arguments: Vec::new(),
        }];

        let rows = mcp_server_rows(&statuses, tools, resources, prompts);
        assert_eq!(rows.len(), 3);

        let ready = &rows[0];
        assert_eq!(ready.name, "context7");
        assert_eq!(ready.status_word, "ready");
        assert_eq!(ready.tone, crate::view::McpStatusTone::Ready);
        assert_eq!(
            ready.tool_count,
            Some(2),
            "connected server counts its tools"
        );
        assert!(
            ready.exposes.contains("resolve-library-id"),
            "tool name in exposes"
        );
        assert!(
            ready.exposes.contains("notes://c7"),
            "resource URI in exposes"
        );
        assert!(ready.exposes.contains("greet"), "prompt name in exposes");

        let failed = &rows[1];
        assert_eq!(failed.status_word, "error");
        assert_eq!(failed.tone, crate::view::McpStatusTone::Error);
        assert_eq!(failed.tool_count, None, "a failed server has no tool count");
        assert!(
            failed.exposes.contains("auth failed"),
            "error detail is the exposes text"
        );

        let needs_auth = &rows[2];
        assert_eq!(needs_auth.status_word, "needs auth");
        assert_eq!(needs_auth.tone, crate::view::McpStatusTone::Degraded);
        assert!(
            needs_auth.exposes.contains("authorization required"),
            "needs_auth keeps a safe actionable detail"
        );
        assert!(
            !needs_auth
                .exposes
                .contains("challenge-must-not-reach-the-tui"),
            "the TUI must never render an OAuth challenge"
        );
    }

    #[test]
    fn dispatching_queue_opens_the_full_screen_queue_view() {
        // VAL-VIEW-015/016: `/queue` opens the full-screen prompt-queue view (not a modal), and the
        // queued prompts are snapshotted as its rows with the head of the queue selected.
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("busy".to_owned(), rx, flag());
        type_str(&mut a, "add unit tests for the buffer cap");
        a.on_key(key(KeyCode::Enter));
        type_str(&mut a, "update CHANGELOG.md for v0.4.3");
        a.on_key(key(KeyCode::Enter));

        dispatch(&mut a, "queue");
        assert_eq!(a.active_view(), Some(crate::view::ViewKind::Queue));
        assert!(a.dialog().is_none(), "the queue view is not a modal dialog");
        assert_eq!(
            a.queue_snapshot().len(),
            2,
            "both queued prompts are snapshotted"
        );
        assert_eq!(
            a.queue_snapshot()[0],
            "add unit tests for the buffer cap",
            "the head of the queue is the first-submitted prompt"
        );
        assert_eq!(
            a.queue_selection(),
            0,
            "the head of the queue is selected on open"
        );
    }

    #[test]
    fn queue_view_arrow_keys_move_and_clamp_the_selection() {
        // VAL-VIEW-018: arrow keys move the highlighted (selection-bar) row within the queue.
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("busy".to_owned(), rx, flag());
        for prompt in ["first queued", "second queued", "third queued"] {
            type_str(&mut a, prompt);
            a.on_key(key(KeyCode::Enter));
        }
        dispatch(&mut a, "queue");
        assert_eq!(a.queue_selection(), 0);

        a.on_key(key(KeyCode::Down));
        assert_eq!(
            a.queue_selection(),
            1,
            "Down moves the selection down one row"
        );
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.queue_selection(), 2, "selection clamps at the last row");
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.queue_selection(), 1, "Up moves the selection back up");

        a.on_key(key(KeyCode::Esc));
        assert!(a.active_view().is_none(), "Esc dismisses the queue view");
    }

    #[test]
    fn queue_view_enter_edits_the_selected_prompt() {
        // misc-queue-inview-actions: `↵` pulls the selected queued prompt back into the composer
        // for editing and closes the view (reusing the shared queue mutation).
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("busy".to_owned(), rx, flag());
        for prompt in ["first queued", "second queued", "third queued"] {
            type_str(&mut a, prompt);
            a.on_key(key(KeyCode::Enter));
        }
        dispatch(&mut a, "queue");
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.queue_selection(), 1);

        a.on_key(key(KeyCode::Enter));
        assert!(a.active_view().is_none(), "editing closes the queue view");
        assert_eq!(
            a.input_value(),
            "second queued",
            "the selected prompt lands back in the composer"
        );
        assert_eq!(
            a.queued_count(),
            2,
            "the edited prompt is pulled out of the queue"
        );
    }

    #[test]
    fn queue_view_ctrl_arrows_reorder_the_selected_prompt() {
        // misc-queue-inview-actions: `Ctrl+Up`/`Ctrl+Down` move the selected queued prompt one slot,
        // keeping the selection bar on it and re-snapshotting the rows.
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("busy".to_owned(), rx, flag());
        for prompt in ["first queued", "second queued", "third queued"] {
            type_str(&mut a, prompt);
            a.on_key(key(KeyCode::Enter));
        }
        dispatch(&mut a, "queue");
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.queue_selection(), 1);

        a.on_key(key_mod(KeyCode::Down, KeyModifiers::CONTROL));
        assert_eq!(a.queue_selection(), 2, "Ctrl+Down follows the moved prompt");
        assert_eq!(
            queue_rows(&a),
            ["first queued", "third queued", "second queued"],
            "the selected prompt moved down one slot"
        );

        a.on_key(key_mod(KeyCode::Up, KeyModifiers::CONTROL));
        assert_eq!(
            a.queue_selection(),
            1,
            "Ctrl+Up follows the moved prompt back"
        );
        assert_eq!(
            queue_rows(&a),
            ["first queued", "second queued", "third queued"],
            "the selected prompt moved back up one slot"
        );

        // Reorder clamps at the ends: Ctrl+Up on the head is a no-op.
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.queue_selection(), 0);
        a.on_key(key_mod(KeyCode::Up, KeyModifiers::CONTROL));
        assert_eq!(a.queue_selection(), 0, "Ctrl+Up on the head is a no-op");
        assert_eq!(
            queue_rows(&a),
            ["first queued", "second queued", "third queued"],
            "the head cannot move up"
        );
    }

    #[test]
    fn queue_view_dd_removes_the_selected_prompt() {
        // misc-queue-inview-actions: the vim-style `dd` chord removes the selected queued prompt and
        // clamps the selection; a lone `d` (or `d` then another key) does not remove anything.
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("busy".to_owned(), rx, flag());
        for prompt in ["first queued", "second queued", "third queued"] {
            type_str(&mut a, prompt);
            a.on_key(key(KeyCode::Enter));
        }
        dispatch(&mut a, "queue");

        // A single `d` arms the chord but does not remove.
        a.on_key(key(KeyCode::Char('d')));
        assert_eq!(a.queued_count(), 3, "a lone `d` does not remove a prompt");
        // A non-`d` key disarms the chord.
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Char('d')));
        assert_eq!(a.queued_count(), 3, "a broken d…x chord does not remove");

        // Select the last row, then `dd` removes it and the selection clamps.
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.queue_selection(), 2);
        a.on_key(key(KeyCode::Char('d')));
        a.on_key(key(KeyCode::Char('d')));
        assert_eq!(a.queued_count(), 2, "`dd` removes the selected prompt");
        assert_eq!(
            queue_rows(&a),
            ["first queued", "second queued"],
            "the last queued prompt was removed"
        );
        assert_eq!(
            a.queue_selection(),
            1,
            "selection clamps to the new last row"
        );
    }

    #[test]
    fn mcp_status_rows_use_the_live_tool_catalogue() {
        let statuses = vec![mcp::ServerStatus {
            name: "api".to_owned(),
            state: mcp::ConnectionState::Connected,
            pid: Some(1234),
            error: None,
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
        }];
        let rows = mcp_status_rows(
            &statuses,
            vec![mcp::McpTool {
                id: "api_extra".to_owned(),
                server: "api".to_owned(),
                tool: "extra".to_owned(),
                description: None,
                input_schema: serde_json::json!({"type":"object"}),
            }],
            Vec::new(),
            Vec::new(),
        );

        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].contains("connected") && rows[0].contains("api_extra"),
            "live tool id is visible in the MCP row: {:?}",
            rows
        );

        let rows = mcp_status_rows(&statuses, Vec::new(), Vec::new(), Vec::new());
        assert!(
            rows[0].contains("tools: (none)") && !rows[0].contains("api_extra"),
            "removed tools disappear from the rendered row: {:?}",
            rows
        );
    }

    #[test]
    fn mcp_status_rows_surface_live_resources_and_prompts() {
        // VAL-GAP-CROSS-010: the TUI `/mcp` view lists MCP resources (by URI) and prompts (by
        // name) so the TUI catalog matches the CLI `cortex mcp list` and the server `/mcp` route.
        let statuses = vec![mcp::ServerStatus {
            name: "demo".to_owned(),
            state: mcp::ConnectionState::Connected,
            pid: Some(4321),
            error: None,
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
        }];
        let rows = mcp_status_rows(
            &statuses,
            Vec::new(),
            vec![mcp::McpResource {
                id: "demo_resource_notes___demo".to_owned(),
                server: "demo".to_owned(),
                uri: "notes://demo".to_owned(),
                name: "demo-notes".to_owned(),
                title: None,
                description: Some("Demo resource".to_owned()),
                mime_type: Some("text/plain".to_owned()),
            }],
            vec![mcp::McpPrompt {
                id: "demo_prompt_greet".to_owned(),
                server: "demo".to_owned(),
                name: "greet".to_owned(),
                title: None,
                description: Some("Greet a caller".to_owned()),
                arguments: Vec::new(),
            }],
        );

        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].contains("resources: notes://demo"),
            "resource uri is visible in the MCP row: {:?}",
            rows
        );
        assert!(
            rows[0].contains("prompts: greet"),
            "prompt name is visible in the MCP row: {:?}",
            rows
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
        let (root, shares_dir) = isolated_share_dir("share-notice");
        let mut a = app();
        a.share_data_dir = Some(shares_dir);
        dispatch(&mut a, "share");
        match a.transcript.last() {
            Some(Entry::Notice(text)) => {
                assert!(text.contains("share"), "a share reference: {text}");
                assert!(text.contains("127.0.0.1"), "a loopback reference: {text}");
            }
            other => panic!("expected a share notice, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pre_persistence_share_then_unshare_revokes_local_reference() {
        let (root, shares_dir) = isolated_share_dir("share-then-unshare");
        let mut a = app();
        a.share_data_dir = Some(shares_dir.clone());
        assert!(
            a.session_id.is_none(),
            "fresh app starts without session id"
        );

        dispatch(&mut a, "share");
        assert!(a.session_id.is_none(), "local share stays pre-persistence");
        assert_eq!(
            share_reference_count(&shares_dir),
            1,
            "share writes one local reference"
        );

        dispatch(&mut a, "unshare");
        assert_eq!(
            share_reference_count(&shares_dir),
            0,
            "unshare removes local pre-persistence references"
        );
        assert!(
            matches!(a.transcript.last(), Some(Entry::Notice(text)) if text.contains("revoked 1 local share reference")),
            "unshare reports one revoked local reference"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dispatching_diff_without_changes_shows_a_clear_notice() {
        let mut a = app();
        dispatch(&mut a, "diff");
        assert!(!a.diff_viewer_open(), "no viewer opens without file diffs");
        assert!(
            matches!(a.transcript.last(), Some(Entry::Notice(text)) if text.contains("No file changes")),
            "a clear no-diff notice is shown"
        );
    }

    #[test]
    fn dispatching_diff_opens_the_viewer_with_file_tree_entries() {
        let before = "alpha\nbeta\ngamma\n";
        let after = "alpha\nBETA\ngamma\n";
        let (rel, path) = temp_relative_diff_file(before, after);

        let mut a = app();
        let diff = crate::diff::compute_file_diff(&rel, Some(before), Some(after)).expect("a diff");
        a.transcript.push(Entry::Diff(diff));

        dispatch(&mut a, "diff");
        assert!(a.diff_viewer_open(), "the diff viewer opens");
        let viewer = a.diff_viewer().expect("open diff viewer");
        assert_eq!(viewer.files.len(), 1, "one file row in the tree");
        assert_eq!(viewer.files[0].path, rel);
        assert_eq!(
            viewer.mode,
            DiffViewMode::Unified,
            "unified is the default mode"
        );
        assert!(
            viewer.is_expanded(0),
            "the first file opens expanded (its hunk body visible)"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn diff_viewer_enter_toggles_the_selected_file_disclosure() {
        // VAL-VIEW-053: activating a file row's disclosure toggles it between expanded (hunks shown)
        // and collapsed (hunks hidden) and back.
        let before = "alpha\nbeta\ngamma\n";
        let after = "alpha\nBETA\ngamma\n";
        let (rel, path) = temp_relative_diff_file(before, after);

        let mut a = app();
        let diff = crate::diff::compute_file_diff(&rel, Some(before), Some(after)).expect("a diff");
        a.transcript.push(Entry::Diff(diff));
        dispatch(&mut a, "diff");
        assert!(a.diff_viewer_open());
        assert!(a.diff_viewer().unwrap().is_expanded(0), "opens expanded");

        a.on_key(key(KeyCode::Enter));
        assert!(
            !a.diff_viewer().unwrap().is_expanded(0),
            "Enter collapses the open file"
        );
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.diff_viewer().unwrap().is_expanded(0),
            "Enter re-expands the file"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn diff_viewer_toggles_between_unified_and_split_modes() {
        let before = "alpha\nbeta\ngamma\n";
        let after = "alpha\nBETA\ngamma\n";
        let (rel, path) = temp_relative_diff_file(before, after);

        let mut a = app();
        let diff = crate::diff::compute_file_diff(&rel, Some(before), Some(after)).expect("a diff");
        a.transcript.push(Entry::Diff(diff));
        dispatch(&mut a, "diff");
        assert_eq!(a.diff_viewer().unwrap().mode, DiffViewMode::Unified);

        a.on_key(key(KeyCode::Char('v')));
        assert_eq!(
            a.diff_viewer().unwrap().mode,
            DiffViewMode::Split,
            "`v` toggles to split mode"
        );
        a.on_key(key(KeyCode::Char('v')));
        assert_eq!(
            a.diff_viewer().unwrap().mode,
            DiffViewMode::Unified,
            "`v` toggles back to unified mode"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn diff_viewer_hunk_revert_changes_only_the_selected_hunk() {
        let before = "line 1\nalpha\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nomega\nline 12\n";
        let after = "line 1\nALPHA\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nOMEGA\nline 12\n";
        let expected_after_revert = "line 1\nALPHA\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nomega\nline 12\n";
        let (rel, path) = temp_relative_diff_file(before, after);

        let mut a = app();
        let diff = crate::diff::compute_file_diff(&rel, Some(before), Some(after)).expect("a diff");
        assert_eq!(diff.hunks.len(), 2, "fixture should produce two hunks");
        a.transcript.push(Entry::Diff(diff));
        dispatch(&mut a, "diff");
        assert!(a.diff_viewer_open());

        a.on_key(key(KeyCode::Tab)); // file tree -> hunks
        a.on_key(key(KeyCode::Down)); // first hunk -> second hunk
        a.on_key(key(KeyCode::Char('r'))); // revert selected hunk

        let disk = std::fs::read_to_string(&path).expect("read reverted file");
        assert_eq!(
            disk, expected_after_revert,
            "only the selected hunk is reverted on disk"
        );

        let viewer = a.diff_viewer().expect("viewer remains open");
        let file = viewer.selected_file().expect("selected file");
        assert_eq!(
            file.hunks.len(),
            1,
            "one hunk remains after targeted revert"
        );

        let _ = std::fs::remove_file(path);
    }

    // VAL-GAP-CROSS-012: a TUI `/diff` viewer hunk revert is reflected CONSISTENTLY by the on-disk
    // bytes AND by the shadow-store net-change surfaces the server `/session/{id}/diff` route and
    // `cortex snapshot diff` read — only the selected hunk is reverted, the unrelated hunk survives
    // everywhere, and the recorded net diff collapses to exactly the remaining change. This locks
    // the cross-flow seam: when a checkpoint store is attached, the revert records a fresh undoable
    // turn (under the session id) so disk and the store's session/snapshot diff never disagree.
    #[test]
    fn hunk_revert_is_consistent_across_disk_and_checkpoint_diff_surfaces() {
        let before = "line 1\nalpha\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nomega\nline 12\n";
        let after = "line 1\nALPHA\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nOMEGA\nline 12\n";
        let expected_after_revert = "line 1\nALPHA\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nomega\nline 12\n";

        let cwd = std::env::current_dir().expect("cwd");
        let unique = format!(
            "{}-{}",
            std::process::id(),
            DIFF_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let rel = format!("target/cortex-tui-revert-xflow-{unique}.txt");
        let absolute = cwd.join(&rel);
        std::fs::create_dir_all(absolute.parent().expect("parent")).expect("create diff dir");
        let shadow = std::env::temp_dir().join(format!("cortex-tui-revert-shadow-{unique}"));
        let store = ShadowStore::open(&cwd, &shadow).expect("open shadow store");
        let session = format!("ses_xflow_{unique}");

        // The "tool that creates multiple hunks": record the original turn (baseline -> two hunks)
        // exactly as the real TUI turn dispatcher does, so the store holds the pre-image.
        std::fs::write(&absolute, before).expect("seed baseline");
        let recorder = store.begin_turn(session.clone());
        recorder.record_pre_image(&absolute);
        std::fs::write(&absolute, after).expect("apply two hunks");
        recorder
            .finalize()
            .expect("finalize original turn")
            .expect("original turn recorded a snapshot");

        let mut a = app();
        a.set_checkpoint_store(Some(store.clone()));
        a.session_id = Some(session.clone());
        let diff = crate::diff::compute_file_diff(&rel, Some(before), Some(after)).expect("a diff");
        assert_eq!(diff.hunks.len(), 2, "fixture should produce two hunks");
        a.transcript.push(Entry::Diff(diff));
        dispatch(&mut a, "diff");
        assert!(a.diff_viewer_open());
        a.on_key(key(KeyCode::Tab)); // file tree -> hunks
        a.on_key(key(KeyCode::Down)); // first hunk -> second (OMEGA) hunk
        a.on_key(key(KeyCode::Char('r'))); // revert only the selected hunk

        // Disk: only the selected (OMEGA) hunk reverted; the unrelated (ALPHA) hunk survives.
        let disk = std::fs::read_to_string(&absolute).expect("read reverted file");
        assert_eq!(
            disk, expected_after_revert,
            "disk reflects only the remaining hunk"
        );

        // Server `/session/{id}/diff` surface (session-scoped net change) agrees: the net diff is
        // the single remaining hunk, with the reverted hunk's text absent.
        let session_diffs = store
            .session_file_diffs(&session)
            .expect("session file diffs");
        assert_eq!(session_diffs.len(), 1, "one file changed net");
        assert_eq!(session_diffs[0].path, rel);
        assert_eq!(session_diffs[0].status, "modified");
        let session_body = &session_diffs[0].diff;
        assert!(
            session_body.contains("ALPHA") && session_body.contains("alpha"),
            "the remaining hunk is present in the session diff: {session_body}"
        );
        assert!(
            !session_body.contains("OMEGA"),
            "the reverted hunk must be absent from the session diff: {session_body}"
        );

        // `cortex snapshot diff` surface (global net change, full unified body) agrees byte-for-byte
        // with the session-scoped diff and reports exactly one added + one removed line.
        let changes = store.snapshot_diff(None, true).expect("snapshot diff");
        assert_eq!(changes.len(), 1, "one file changed net");
        assert_eq!(changes[0].additions, 1, "exactly one remaining addition");
        assert_eq!(changes[0].deletions, 1, "exactly one remaining deletion");
        let full = changes[0].diff.as_deref().unwrap_or_default();
        assert!(
            full.contains("ALPHA") && !full.contains("OMEGA"),
            "the global snapshot diff reflects only the remaining hunk: {full}"
        );

        // The viewer's own recomputed diff also shows the single remaining hunk (UI consistency).
        let viewer = a.diff_viewer().expect("viewer remains open");
        assert_eq!(
            viewer.selected_file().expect("selected file").hunks.len(),
            1,
            "one hunk remains in the viewer after the targeted revert"
        );

        let _ = std::fs::remove_file(&absolute);
        let _ = std::fs::remove_dir_all(&shadow);
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
        let chosen = a.select_model().expect("a changed model is committed");
        assert_ne!(chosen, before, "a different model is committed");
        assert!(
            highlighted.starts_with(&chosen),
            "the highlighted row maps to the chosen model: highlighted={highlighted} chosen={chosen}"
        );
        assert_eq!(a.status.model, chosen, "the active model updates");
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
    fn picker_data_helpers_format_authentic_values() {
        // The rich pickers humanize real catalogue/DB values; these are the exact cell strings.
        assert_eq!(format_context(1_048_576), "1M");
        assert_eq!(format_context(1_000_000), "1M");
        assert_eq!(format_context(400_000), "400K");
        assert_eq!(format_context(200_000), "200K");
        assert_eq!(format_context(163_840), "164K");
        assert_eq!(format_context(0), "—");
        assert_eq!(format_price(0.30), "$0.30");
        assert_eq!(format_price(1.25), "$1.25");
        assert_eq!(format_price(15.0), "$15");
        assert_eq!(format_price(3.0), "$3");
        assert_eq!(format_price(0.0), "free");
        assert_eq!(short_model("google/gemini-2.5-flash"), "gemini-2.5-flash");
        assert_eq!(short_model("noprovider"), "noprovider");
        let now = parse_epoch_secs("2026-01-02 00:00:00").expect("valid timestamp");
        assert_eq!(relative_age("2026-01-01 00:00:00", now), "1d");
        assert_eq!(relative_age("2026-01-01 22:00:00", now), "2h");
        assert_eq!(relative_age("2026-01-01 23:55:00", now), "5m");
        assert_eq!(relative_age("2026-01-01 23:59:30", now), "just now");
        assert_eq!(relative_age("not-a-timestamp", now), "");
    }

    #[test]
    fn session_picker_rows_carry_title_agent_model_and_metric_meta() {
        // VAL-PICK-007: each real session row shows title + `agent · model` + `N msg · <age>`.
        let path = session_store_path("rich-session-rows");
        let sid = {
            let db = db::Database::open(&path).expect("open seed");
            let session = db
                .sessions()
                .create(Some("streaming buffer fix"))
                .expect("session");
            db.messages().create(&session.id, "user", "hi").expect("m1");
            db.messages()
                .create(&session.id, "assistant", "yo")
                .expect("m2");
            session.id
        };
        let mut a = app();
        a.set_session_store(Some(path));
        dispatch(&mut a, "sessions");
        let dialog = a.dialog().expect("session picker open");
        assert_eq!(dialog.kind, DialogKind::Sessions);
        // A real persisted row is pre-highlighted (not the synthetic floor row).
        assert!(
            a.selected_session_id().is_some(),
            "a real session is highlighted"
        );
        let picks = a.build_session_picks();
        let pick = picks
            .iter()
            .find(|pick| pick.id.as_deref() == Some(sid.as_str()))
            .expect("seeded session present");
        assert_eq!(pick.row.key, "streaming buffer fix", "title is the row key");
        let desc = pick.row.desc.as_deref().unwrap_or_default();
        assert!(
            desc.starts_with("build · ") && desc.contains("gemini-2.5-flash"),
            "agent · model descriptor: {desc}"
        );
        let meta = pick.row.meta.as_deref().unwrap_or_default();
        assert!(meta.starts_with("2 msg"), "N msg · age metric: {meta}");
    }

    #[test]
    fn session_search_narrows_then_restores_the_list() {
        // VAL-PICK-008/009: typing into the search row filters by title; clearing restores it.
        let path = session_store_path("session-search");
        {
            let db = db::Database::open(&path).expect("open seed");
            db.sessions().create(Some("alpha buffer")).expect("s1");
            db.sessions().create(Some("beta cards")).expect("s2");
        }
        let mut a = app();
        a.set_session_store(Some(path));
        dispatch(&mut a, "sessions");
        let full = a.dialog().expect("open").items.len();
        assert!(full >= 2, "both seeded sessions listed: {full}");
        type_str(&mut a, "beta");
        let filtered = a.dialog().expect("open").items.clone();
        assert!(
            !filtered.is_empty()
                && filtered
                    .iter()
                    .all(|item| item.to_lowercase().contains("beta")),
            "filtered to matching sessions: {filtered:?}"
        );
        for _ in 0.."beta".len() {
            a.on_key(key(KeyCode::Backspace));
        }
        assert_eq!(
            a.dialog().expect("open").items.len(),
            full,
            "clearing the query restores the full list"
        );
    }

    #[test]
    fn model_picker_rows_are_real_ids_with_ctx_price_and_aligned_keys() {
        // VAL-PICK-015/016/017: real provider/model ids, a fixed key column so descriptions align,
        // and a right-aligned `ctx · price` meta derived from the real catalogue.
        let mut a = app();
        dispatch(&mut a, "models");
        assert!(
            a.picker_key_width().is_some(),
            "fixed key column for alignment"
        );
        let picks = a.build_model_picks();
        assert!(
            picks
                .iter()
                .any(|pick| pick.id == "google/gemini-2.5-flash"),
            "real default id present"
        );
        assert!(
            picks
                .iter()
                .any(|pick| pick.id == "anthropic/claude-opus-4.1"),
            "real anthropic id present"
        );
        let flash = picks
            .iter()
            .find(|pick| pick.id == "google/gemini-2.5-flash")
            .expect("default model row");
        let meta = flash.row.meta.as_deref().unwrap_or_default();
        assert!(
            meta.starts_with("1M · $"),
            "ctx · price meta from real catalogue: {meta}"
        );
        // The descriptor is derived from real catalogue metadata (the default model is flagged
        // `default`), never a hardcoded mockup label.
        let desc = flash.row.desc.as_deref().unwrap_or_default();
        assert!(
            desc.starts_with("default"),
            "the default model's descriptor is derived from real metadata: {desc}"
        );
    }

    #[test]
    fn model_search_narrows_the_list() {
        // VAL-PICK-018: typing filters the model list to fuzzy id/description matches.
        let mut a = app();
        dispatch(&mut a, "models");
        let full = a.dialog().expect("open").items.len();
        type_str(&mut a, "opus");
        let filtered = a.dialog().expect("open").items.clone();
        assert!(
            !filtered.is_empty()
                && filtered
                    .iter()
                    .all(|item| item.to_lowercase().contains("opus")),
            "filtered to matching models: {filtered:?}"
        );
        for _ in 0.."opus".len() {
            a.on_key(key(KeyCode::Backspace));
        }
        assert_eq!(
            a.dialog().expect("open").items.len(),
            full,
            "clearing the query restores the full list"
        );
    }

    #[test]
    fn tab_toggles_model_provider_grouping() {
        // VAL-PICK-067: `⇥` regroups the model list by vendor (headers appear) and toggles back.
        let mut a = app();
        dispatch(&mut a, "models");
        let flat = a.dialog().expect("open").items.clone();
        assert!(
            a.model_rows.iter().all(Option::is_some),
            "flat view has only selectable rows"
        );
        a.on_key(key(KeyCode::Tab));
        let grouped = a.dialog().expect("open").items.clone();
        assert!(
            grouped.len() > flat.len(),
            "grouped view adds vendor header rows: flat={} grouped={}",
            flat.len(),
            grouped.len()
        );
        assert!(
            grouped.iter().any(|item| item == "google"),
            "a vendor header appears: {grouped:?}"
        );
        assert!(
            a.model_rows.iter().any(Option::is_none),
            "grouped view has non-selectable header rows"
        );
        // The highlight stays on a selectable model row, never a header.
        let sel = a.dialog().expect("open").selected();
        assert!(
            a.model_rows[sel].is_some(),
            "highlight is on a selectable row"
        );
        a.on_key(key(KeyCode::Tab));
        assert_eq!(
            a.dialog().expect("open").items.len(),
            flat.len(),
            "toggling back returns to the flat list"
        );
    }

    /// A models.dev-shaped catalogue fixture with two real providers (each carrying dated models)
    /// plus a disallowed Zen provider, used to drive the pure picker projection deterministically.
    fn model_catalogue_fixture() -> providers::Catalogue {
        let json = r#"{
            "openai": {
                "id": "openai",
                "name": "OpenAI",
                "models": {
                    "openai/gpt-5": {"id": "openai/gpt-5", "name": "GPT-5", "reasoning": true, "attachment": true, "limit": {"context": 400000}, "cost": {"input": 1.25}, "release_date": "2025-08-01"},
                    "openai/gpt-4o": {"id": "openai/gpt-4o", "name": "GPT-4o", "reasoning": false, "attachment": true, "limit": {"context": 128000}, "cost": {"input": 2.5}, "release_date": "2024-05-01"}
                }
            },
            "anthropic": {
                "id": "anthropic",
                "name": "Anthropic",
                "models": {
                    "anthropic/claude-opus-4.1": {"id": "anthropic/claude-opus-4.1", "name": "Claude Opus 4.1", "reasoning": true, "attachment": true, "limit": {"context": 200000}, "cost": {"input": 15}, "release_date": "2025-08-05"},
                    "anthropic/claude-sonnet-4": {"id": "anthropic/claude-sonnet-4", "name": "Claude Sonnet 4", "reasoning": true, "attachment": true, "limit": {"context": 1000000}, "cost": {"input": 3}, "release_date": "2025-05-01"}
                }
            },
            "zen": {
                "id": "zen",
                "name": "Zen (disallowed)",
                "models": {
                    "zen/zen-1": {"id": "zen/zen-1", "name": "Zen 1", "limit": {"context": 8000}, "cost": {"input": 0}}
                }
            }
        }"#;
        providers::Catalogue::from_json(json).expect("valid fixture catalogue")
    }

    fn model_pick_pos(picks: &[ModelPick], id: &str) -> usize {
        picks
            .iter()
            .position(|pick| pick.id == id)
            .unwrap_or_else(|| {
                panic!(
                    "pick {id} present in {:?}",
                    picks.iter().map(|pick| &pick.id).collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn model_picks_group_by_vendor_recent_first_not_alphabetical() {
        // VAL-CAT-001/002/003: the picker surfaces multiple provider groups (drawn from the dynamic
        // catalogue), orders each group's models newest-first by release_date (NOT alphabetical),
        // and never lists the disallowed Zen vendor.
        let theme = frame::style::ThemeColors::dark();
        let picks = App::model_picks_from_catalogue(
            &model_catalogue_fixture(),
            None,
            "openai/gpt-5",
            &[],
            theme.text,
        );
        let vendors: BTreeSet<&str> = picks.iter().map(|pick| pick.vendor.as_str()).collect();
        assert!(
            vendors.contains("openai") && vendors.contains("anthropic"),
            "multiple provider groups present: {vendors:?}"
        );
        assert!(
            !vendors.contains("zen") && picks.iter().all(|pick| !pick.id.contains("zen")),
            "the disallowed Zen vendor is excluded everywhere: {vendors:?}"
        );
        // Within the openai group, the newer gpt-5 (2025) sorts above the older gpt-4o (2024) —
        // recency, not alphabetical (alphabetical would rank gpt-4o above gpt-5).
        assert!(
            model_pick_pos(&picks, "openai/gpt-5") < model_pick_pos(&picks, "openai/gpt-4o"),
            "newest openai model leads its group (recent-first)"
        );
        assert!(
            model_pick_pos(&picks, "anthropic/claude-opus-4.1")
                < model_pick_pos(&picks, "anthropic/claude-sonnet-4"),
            "newest anthropic model leads its group (recent-first)"
        );
    }

    #[test]
    fn model_picks_mark_connected_providers_distinctly() {
        // VAL-CAT-007: models served by a connected provider (a stored credential) carry a leading
        // connected marker; not-connected providers' models do not, so the two read distinctly.
        let theme = frame::style::ThemeColors::dark();
        let connected = vec!["openai".to_owned()];
        let picks = App::model_picks_from_catalogue(
            &model_catalogue_fixture(),
            None,
            "openai/gpt-5",
            &connected,
            theme.text,
        );
        let openai = picks
            .iter()
            .find(|pick| pick.id == "openai/gpt-5")
            .expect("openai pick");
        let anthropic = picks
            .iter()
            .find(|pick| pick.id == "anthropic/claude-opus-4.1")
            .expect("anthropic pick");
        assert!(
            openai.row.key.starts_with(crate::glyphs::CONNECTED_MARKER),
            "connected provider model is marked: {:?}",
            openai.row.key
        );
        assert!(
            !anthropic
                .row
                .key
                .starts_with(crate::glyphs::CONNECTED_MARKER),
            "not-connected provider model is unmarked: {:?}",
            anthropic.row.key
        );
        // The full model id survives behind the marker so the row stays searchable/selectable.
        assert!(openai.row.key.contains("openai/gpt-5"));
        assert!(openai.hay.contains("openai/gpt-5") && openai.hay.contains("gpt-5"));
    }

    #[test]
    fn model_picks_meta_is_honest_and_brand_pure() {
        // VAL-CAT-005: each row shows real ctx/price meta in the mono-orange (non-green) meta tone
        // and a capability descriptor derived from real metadata (reasoning/multimodal).
        let theme = frame::style::ThemeColors::dark();
        let picks = App::model_picks_from_catalogue(
            &model_catalogue_fixture(),
            None,
            "openai/gpt-5",
            &[],
            theme.text,
        );
        let opus = picks
            .iter()
            .find(|pick| pick.id == "anthropic/claude-opus-4.1")
            .expect("opus pick");
        assert_eq!(
            opus.row.meta.as_deref(),
            Some("200K · $15"),
            "ctx · price meta from real catalogue"
        );
        assert_eq!(
            opus.row.meta_color,
            Some(theme.text),
            "meta uses the mono-orange text tone (no green data numbers)"
        );
        let desc = opus.row.desc.as_deref().unwrap_or_default();
        assert!(
            desc.contains("reasoning") && desc.contains("multimodal"),
            "honest capability descriptor: {desc}"
        );
    }

    /// A models.dev-shaped fixture carrying a model id ABSENT from the embedded snapshot, used to
    /// prove the background live fetch surfaces the CURRENT catalogue (VAL-CAT-008/010/012).
    fn snapshot_absent_catalogue() -> providers::Catalogue {
        let json = r#"{
            "openrouter": {
                "id": "openrouter",
                "name": "OpenRouter",
                "models": {
                    "fixturevendor/only-in-fixture-v9": {"id": "fixturevendor/only-in-fixture-v9", "name": "Fixture Only v9", "reasoning": true, "limit": {"context": 128000}, "cost": {"input": 1.0}, "release_date": "2099-01-01"}
                }
            }
        }"#;
        providers::Catalogue::from_json(json).expect("valid fixture catalogue")
    }

    const FIXTURE_ONLY_MODEL: &str = "fixturevendor/only-in-fixture-v9";

    #[test]
    fn models_refresh_spawn_gate_is_fail_closed_and_coalesced() {
        // VAL-CAT-010/011: the spawn gate refuses when the network kill-switch is engaged
        // (fail-closed — no socket) and when a refresh is already in flight (coalesced), and permits
        // a refresh only when network is allowed and none is running.
        assert!(
            App::should_spawn_models_refresh(false, false),
            "spawn when the network is allowed and no refresh is in flight"
        );
        assert!(
            !App::should_spawn_models_refresh(true, false),
            "FAIL-CLOSED: never spawn under CORTEX_NO_NET (network disabled)"
        );
        assert!(
            !App::should_spawn_models_refresh(false, true),
            "COALESCE: never stack a second refresh while one is in flight"
        );
        assert!(
            !App::should_spawn_models_refresh(true, true),
            "both guards hold together"
        );
    }

    #[test]
    fn models_refresh_swaps_the_open_picker_in_place() {
        // VAL-CAT-008/010: a background refresh landing WHILE /models is open swaps the fetched
        // catalogue in place — a fixture-only model id (absent from the embedded snapshot) appears
        // with no manual reopen — and the session adopts the fetched catalogue.
        let mut a = app();
        dispatch(&mut a, "models");
        assert!(
            !a.model_all.iter().any(|pick| pick.id == FIXTURE_ONLY_MODEL),
            "precondition: the fixture-only id is not in the offline snapshot"
        );
        a.apply_models_refresh(ModelsRefreshMessage::Refreshed(Box::new(
            snapshot_absent_catalogue(),
        )));
        assert!(
            a.model_all.iter().any(|pick| pick.id == FIXTURE_ONLY_MODEL),
            "the fetched catalogue's fixture-only id appears in the open picker in place"
        );
        assert!(
            a.models_catalogue.is_some(),
            "the session adopts the fetched catalogue for later opens"
        );
        // It renders on the actual picker surface (design-match: the id reaches the buffer).
        let text = buffer_text(&render_buffer(&a, 100, 30));
        assert!(
            text.contains(FIXTURE_ONLY_MODEL),
            "the fixture-only model renders in the open picker:\n{text}"
        );
    }

    #[test]
    fn startup_refresh_is_reflected_on_first_models_open() {
        // VAL-CAT-008: a refresh that lands BEFORE /models is first opened (the startup refresh) is
        // adopted for the session, so the FIRST open surfaces the current catalogue with no manual
        // refresh and no reopen.
        let mut a = app();
        // No picker open yet — mirrors the startup refresh landing on the home screen.
        let (tx, rx) = std::sync::mpsc::channel();
        a.models_refresh_rx = Some(rx);
        tx.send(ModelsRefreshMessage::Refreshed(Box::new(
            snapshot_absent_catalogue(),
        )))
        .unwrap();
        a.poll_models_refresh();
        assert!(
            a.dialog().is_none(),
            "the background refresh does not open a picker on its own"
        );
        dispatch(&mut a, "models");
        assert!(
            a.model_all.iter().any(|pick| pick.id == FIXTURE_ONLY_MODEL),
            "the first /models open reflects the startup-fetched catalogue"
        );
    }

    #[test]
    fn models_refresh_indicator_shows_while_in_flight_then_clears() {
        // VAL-CAT-010: while a background refresh runs the picker shows a subtle "refreshing…"
        // indicator (never an error surface); it clears once the result lands and the list swaps.
        let mut a = app();
        dispatch(&mut a, "models");
        assert!(
            !a.models_refreshing(),
            "a cfg(test) open spawns no real refresh"
        );
        // Simulate an in-flight refresh by injecting the channel the worker would publish on.
        let (tx, rx) = std::sync::mpsc::channel();
        a.models_refresh_rx = Some(rx);
        assert!(
            a.models_refreshing(),
            "the indicator is active while refreshing"
        );
        let text = buffer_text(&render_buffer(&a, 100, 30));
        assert!(
            text.contains("refreshing…"),
            "the subtle refreshing note renders in the picker:\n{text}"
        );
        assert!(!a.has_toasts(), "a refresh never raises a toast");
        // The result lands; the indicator clears and the note disappears.
        tx.send(ModelsRefreshMessage::Refreshed(Box::new(
            snapshot_absent_catalogue(),
        )))
        .unwrap();
        a.poll_models_refresh();
        assert!(
            !a.models_refreshing(),
            "the indicator clears once the refresh lands"
        );
        let text = buffer_text(&render_buffer(&a, 100, 30));
        assert!(
            !text.contains("refreshing…"),
            "the refreshing note is gone after the swap:\n{text}"
        );
    }

    #[test]
    fn models_refresh_failure_degrades_silently_to_offline() {
        // VAL-CAT-011: a failed/again-offline fetch keeps the offline list unchanged with NO toast
        // or error surface (fail-closed degradation to the offline list).
        let mut a = app();
        dispatch(&mut a, "models");
        let before: Vec<String> = a.model_all.iter().map(|pick| pick.id.clone()).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        a.models_refresh_rx = Some(rx);
        tx.send(ModelsRefreshMessage::Failed).unwrap();
        a.poll_models_refresh();
        assert!(
            !a.models_refreshing(),
            "the indicator clears after a failed fetch"
        );
        assert!(
            a.models_catalogue.is_none(),
            "a failed fetch adopts no catalogue"
        );
        let after: Vec<String> = a.model_all.iter().map(|pick| pick.id.clone()).collect();
        assert_eq!(
            before, after,
            "the offline list is unchanged after a failed fetch"
        );
        assert!(!a.has_toasts(), "a failed refresh never raises a toast");
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
    fn agent_picker_rows_are_real_personas_with_desc_and_kind_meta() {
        // VAL-PICK-024/025/026: real agents with descriptions, a right-aligned kind meta colored
        // by role (primary AMBER / subagent FAINT), no search row.
        let mut a = app();
        let theme = frame::style::ThemeColors::from_name(a.theme());
        dispatch(&mut a, "agents");
        let rows = a.picker_rows();
        assert_eq!(rows.len(), 4, "build/plan/general/explore");
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["build", "plan", "general", "explore"]);
        // build is the primary agent (AMBER meta); the rest are subagents (FAINT meta).
        let build = &rows[0];
        assert!(
            build.desc.as_deref().unwrap_or_default().contains("coding"),
            "real description carried: {:?}",
            build.desc
        );
        assert_eq!(build.meta.as_deref(), Some("primary"));
        assert_eq!(build.meta_color, Some(theme.secondary));
        assert_eq!(rows[1].meta.as_deref(), Some("subagent"));
        assert_eq!(rows[1].meta_color, Some(theme.text_muted));
        // No search row: dispatching the agent picker leaves the search query empty and it is not
        // treated as a searchable picker.
        assert!(!a.picker_is_searchable(), "agent picker has no search row");
    }

    #[test]
    fn variant_picker_titled_reasoning_variant_with_descriptions_no_meta() {
        // VAL-PICK-037/038/039: titled `Reasoning variant`, key + description rows, no meta column
        // and no search row.
        let mut a = app();
        dispatch(&mut a, "variant");
        let dialog = a.dialog().expect("the variant picker opened");
        assert_eq!(dialog.kind, DialogKind::Variants);
        assert_eq!(dialog.title, "Reasoning variant");
        let rows = a.picker_rows();
        assert!(!rows.is_empty(), "variants listed");
        for row in rows {
            assert!(
                row.desc.as_deref().is_some_and(|d| !d.is_empty()),
                "each variant row has a description: {:?}",
                row.key
            );
            assert!(row.meta.is_none(), "variant picker has no meta column");
        }
        assert!(
            !a.picker_is_searchable(),
            "variant picker has no search row"
        );
    }

    #[test]
    fn variant_picker_selection_updates_the_active_variant() {
        // VAL-PICK-041: Enter on a highlighted variant applies it and closes the modal.
        let mut a = app();
        assert_eq!(a.status.variant, "default");
        dispatch(&mut a, "variant");
        a.on_key(key(KeyCode::Down)); // default -> fast
        a.on_key(key(KeyCode::Enter));
        assert!(!a.dialog_open(), "confirming closes the variant picker");
        assert_eq!(a.status.variant, "fast", "the active variant updates");
    }

    #[test]
    fn variant_picker_esc_leaves_the_active_variant_unchanged() {
        // VAL-PICK-042: esc dismisses the variant picker without changing the active variant.
        let mut a = app();
        dispatch(&mut a, "variant");
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Esc));
        assert!(!a.dialog_open());
        assert_eq!(
            a.status.variant, "default",
            "esc leaves the variant unchanged"
        );
    }

    #[test]
    fn skill_picker_rows_carry_source_meta() {
        // VAL-PICK-031/032: skills render as key + description with a right-aligned source meta.
        let mut a = app();
        dispatch(&mut a, "skills");
        for row in a.picker_rows() {
            let source = row.meta.as_deref().unwrap_or_default();
            assert!(
                matches!(source, "builtin" | "project" | "plugin"),
                "skill source meta is builtin/project/plugin: {source:?}"
            );
        }
    }

    #[test]
    fn skill_picker_space_toggles_enabled_and_enter_commits_esc_discards() {
        // VAL-PICK-034/035/036: Space toggles the highlighted skill's enabled marker without
        // closing; Enter commits the set; Esc discards uncommitted toggles.
        let mut a = app();
        // Seed a discoverable project skill so the picker is non-empty regardless of the host env.
        a.skill_picks = vec![
            SkillPick {
                name: "alpha".to_owned(),
                desc: "first".to_owned(),
                source: "project".to_owned(),
            },
            SkillPick {
                name: "beta".to_owned(),
                desc: "second".to_owned(),
                source: "project".to_owned(),
            },
        ];
        a.skill_rows = vec!["alpha".to_owned(), "beta".to_owned()];
        a.skill_pending = Some(std::collections::BTreeSet::new());
        a.rebuild_skill_view();
        a.open_dialog(Dialog {
            kind: DialogKind::Skills,
            title: "Skills".to_owned(),
            items: vec!["alpha".to_owned(), "beta".to_owned()],
            selected: 0,
            empty: None,
        });
        // Space enables the highlighted skill: the row gains the `✓` marker and the modal stays open.
        a.on_key(key(KeyCode::Char(' ')));
        assert!(a.dialog_open(), "space does not close the skill picker");
        assert!(
            a.picker_rows()[0].key.starts_with(crate::glyphs::MARK_DONE),
            "enabled skill shows the ✓ marker: {:?}",
            a.picker_rows()[0].key
        );
        // Esc discards the uncommitted toggle: the committed set stays empty.
        a.on_key(key(KeyCode::Esc));
        assert!(
            a.enabled_skills.is_empty(),
            "esc discards uncommitted toggles"
        );
        // Re-open, toggle, and commit with Enter: the committed set now holds the skill.
        a.skill_picks = vec![SkillPick {
            name: "alpha".to_owned(),
            desc: "first".to_owned(),
            source: "project".to_owned(),
        }];
        a.skill_rows = vec!["alpha".to_owned()];
        a.skill_pending = Some(std::collections::BTreeSet::new());
        a.rebuild_skill_view();
        a.open_dialog(Dialog {
            kind: DialogKind::Skills,
            title: "Skills".to_owned(),
            items: vec!["alpha".to_owned()],
            selected: 0,
            empty: None,
        });
        a.on_key(key(KeyCode::Char(' ')));
        a.on_key(key(KeyCode::Enter));
        assert!(!a.dialog_open(), "enter closes the skill picker");
        assert!(
            a.enabled_skills.contains("alpha"),
            "enter commits the toggled skill set: {:?}",
            a.enabled_skills
        );
    }

    #[test]
    fn closing_each_picker_returns_focus_to_the_prompt() {
        // VAL-TUI-076: dismissing any dialog with Esc returns focus to the prompt so typing resumes.
        for command in ["sessions", "models", "agents", "themes"] {
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

        // `/mcp` opens the full-screen view rather than a dialog; Esc likewise returns to the prompt.
        let mut a = app();
        dispatch(&mut a, "mcp");
        assert!(a.active_view_open(), "mcp opened the full-screen view");
        a.on_key(key(KeyCode::Esc));
        assert!(!a.active_view_open(), "mcp view closed on Esc");
        type_str(&mut a, "hello");
        assert_eq!(
            a.input_value(),
            "hello",
            "typing reaches the prompt after closing the mcp view"
        );
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

    #[test]
    fn session_picker_lists_sessions_created_by_external_surfaces() {
        // VAL-CROSS-003/044 (TUI-picker half): sessions that other surfaces persisted to the
        // shared store are listed by `/sessions`, not isolated to their creating surface.
        let path = session_store_path("external-surfaces");
        let (server_id, acp_id) = {
            let db = db::Database::open(&path).expect("open seed");
            let server = db
                .sessions()
                .create(Some("server api marker"))
                .expect("server session");
            let acp = db
                .sessions()
                .create(Some("acp stdio marker"))
                .expect("acp session");
            (server.id, acp.id)
        };

        let mut a = app();
        a.set_session_store(Some(path));
        dispatch(&mut a, "sessions");
        let dialog = a.dialog().expect("the session picker opened");

        // The visible rows show real session titles; the backing entries carry the persisted ids.
        assert!(
            dialog.items.iter().any(|item| item == "server api marker"),
            "server-created session should be present in picker rows: {:?}",
            dialog.items
        );
        assert!(
            dialog.items.iter().any(|item| item == "acp stdio marker"),
            "ACP-created session should be present in picker rows: {:?}",
            dialog.items
        );
        assert!(
            a.session_entries
                .iter()
                .any(|entry| entry.id.as_deref() == Some(server_id.as_str())),
            "server session id is tracked in the backing entries"
        );
        assert!(
            a.session_entries
                .iter()
                .any(|entry| entry.id.as_deref() == Some(acp_id.as_str())),
            "ACP session id is tracked in the backing entries"
        );
    }

    #[test]
    fn attached_tui_polls_server_created_messages_into_the_transcript() {
        // VAL-CROSS-038 in-process seam: a server appends messages to the shared session store, and
        // the attached TUI picks them up without reopening the session picker.
        let path = session_store_path("remote-sync");
        let session_id = {
            let db = db::Database::open(&path).expect("open seed");
            let session = db.sessions().create(Some("remote sync")).expect("session");
            db.messages()
                .create(&session.id, "user", "initial local prompt")
                .expect("initial message");
            session.id
        };

        let mut a = app();
        a.set_session_store(Some(path.clone()));
        a.set_remote_server(Some(
            crate::remote::RemoteServer::from_url("http://127.0.0.1:3200").unwrap(),
        ));
        a.attach_session(&session_id);
        let before = a.transcript.len();

        {
            let db = db::Database::open(&path).expect("reopen seed");
            db.messages()
                .create(&session_id, "user", "server injected prompt")
                .expect("server user");
            db.messages()
                .create(&session_id, "assistant", "server injected answer")
                .expect("server assistant");
        }

        assert!(
            a.poll_remote_session(),
            "new remote messages should be applied"
        );
        assert!(
            a.transcript.len() >= before + 2,
            "the injected user and assistant messages are appended"
        );
        let rendered = a.render_session_markdown();
        assert!(rendered.contains("server injected prompt"), "{rendered}");
        assert!(rendered.contains("server injected answer"), "{rendered}");
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
        a.on_key(key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL));
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
        // The current row (index 0) has no persisted id, so Ctrl+R does not enter rename mode.
        a.on_key(key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL));
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
        a.on_key(key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL));
        for _ in 0.."old title".len() {
            a.on_key(key(KeyCode::Backspace));
        }
        // 'd' would filter outside rename mode; here it must be captured as a character.
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
        let labels = a
            .palette_matches()
            .into_iter()
            .map(|c| c.label)
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label.contains("models")),
            "models matches: {labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label.contains("/help")),
            "unrelated commands hidden: {labels:?}"
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
    fn palette_runs_models_command_and_opens_the_model_picker() {
        // VAL-PICK-063: Enter on a highlighted command dispatches the real command (here `/models`
        // opens the model picker), confirming the palette runs commands rather than inserting text.
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        type_str(&mut a, "models");
        a.on_key(key(KeyCode::Enter));
        assert!(
            !a.palette_open(),
            "the palette closes after running the command"
        );
        let dialog = a.dialog().expect("the model picker opened");
        assert_eq!(dialog.kind, DialogKind::Models);
    }

    #[test]
    fn palette_root_leads_with_primary_commands_and_hides_aliases() {
        // VAL-PICK-060: the palette leads with the reference primary commands (in order) and never
        // shows the `/model` / `/theme` alias duplicates.
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        let labels = a
            .palette_matches()
            .into_iter()
            .map(|m| m.label)
            .collect::<Vec<_>>();
        assert_eq!(
            labels[..5],
            ["/new", "/sessions", "/models", "/agents", "/themes"]
        );
        assert!(
            !labels.iter().any(|l| l == "/model"),
            "no /model alias: {labels:?}"
        );
        assert!(
            !labels.iter().any(|l| l == "/theme"),
            "no /theme alias: {labels:?}"
        );
        // Advanced commands stay reachable through the palette's fuzzy search.
        assert!(
            labels.iter().any(|l| l == "/help"),
            "help still listed: {labels:?}"
        );
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
    fn palette_filter_is_subsequence_fuzzy_not_just_substring() {
        // VAL-PICK-061: the palette filters by subsequence (matching anywhere), consistent with the
        // file picker's fuzzy semantics — a non-contiguous query like `mdl` still surfaces
        // `/models`, which a plain substring `.contains()` would miss.
        let mut a = app();
        a.on_key(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        type_str(&mut a, "mdl");
        let labels = a
            .palette_matches()
            .into_iter()
            .map(|c| c.label)
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label == "/models"),
            "`mdl` fuzzy-matches `/models`: {labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label == "/new"),
            "an unrelated command stays hidden: {labels:?}"
        );
        // Clearing the query restores the full list.
        for _ in 0.."mdl".len() {
            a.on_key(key(KeyCode::Backspace));
        }
        assert_eq!(a.palette_query(), Some(""));
        assert!(
            a.palette_matches().iter().any(|c| c.label == "/new"),
            "clearing restores the full command list"
        );
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
        // VAL-TUI-078: next/prev move the active match (wrapping at the ends). While EDITING, Down/Up
        // navigate as ergonomic extras (Enter is reserved to commit into NAV mode).
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
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.find_status().0, 2, "Down advances again");
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
    fn find_query_accepts_letters_n_and_uppercase_n() {
        // Regression: `n`/`N` must NOT be swallowed by match navigation — every printable letter
        // (including n/N) falls through to the live query so terms like `connection` are typeable.
        let mut a = app();
        a.transcript
            .push(Entry::Notice("open a new connection now".to_owned()));
        a.transcript
            .push(Entry::Notice("Nginx CONNECTION pool".to_owned()));
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for c in "connection".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            a.find().map(|f| f.query.as_str()),
            Some("connection"),
            "the full query (incl. its 'n's) lands in the find input"
        );
        assert_eq!(
            a.find_status().1,
            2,
            "the match count updates for a query containing n/N (case-insensitive)"
        );

        // An uppercase N is equally typeable (never treated as prev-match navigation).
        a.on_key(key(KeyCode::Char('N')));
        assert_eq!(
            a.find().map(|f| f.query.as_str()),
            Some("connectionN"),
            "an uppercase N is typed into the query, not consumed as prev-nav"
        );
    }

    #[test]
    fn find_editing_mode_letters_type_and_arrows_navigate() {
        // In EDITING mode letters (incl. n/N) type into the query and are never consumed as
        // navigation; Down/Up navigate matches as ergonomic extras.
        let mut a = app();
        a.transcript.push(Entry::Notice("nano one".to_owned()));
        a.transcript.push(Entry::Notice("nano two".to_owned()));
        a.transcript.push(Entry::Notice("nano three".to_owned()));
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for c in "nano".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            a.find().map(|f| f.query.as_str()),
            Some("nano"),
            "letters incl. n type into the query, never consumed as navigation"
        );
        assert_eq!(a.find_status(), (0, 3), "three `nano` matches, active at 0");
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.find_status().0, 1, "Down advances to the next match");
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.find_status().0, 0, "Up steps to the previous match");
    }

    #[test]
    fn find_commit_then_n_and_shift_n_navigate_wrapping() {
        // After Enter commits the query, `n` advances and `N` retreats the active match (both
        // wrapping), the cur counter changes, and the query itself is untouched by navigation.
        let mut a = app();
        a.transcript.push(Entry::Notice("apple one".to_owned()));
        a.transcript.push(Entry::Notice("apple two".to_owned()));
        a.transcript.push(Entry::Notice("apple three".to_owned()));
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for c in "apple".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            a.find_status(),
            (0, 3),
            "editing highlights all three matches"
        );
        assert_eq!(
            a.find().map(|f| f.committed),
            Some(false),
            "still EDITING before Enter"
        );
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            a.find().map(|f| f.committed),
            Some(true),
            "Enter commits into NAV mode"
        );
        assert_eq!(
            a.find_status(),
            (0, 3),
            "commit keeps the matches/highlight and the first active match"
        );
        a.on_key(key(KeyCode::Char('n')));
        assert_eq!(a.find_status().0, 1, "n advances to the next match");
        a.on_key(key(KeyCode::Char('n')));
        assert_eq!(a.find_status().0, 2, "n advances again");
        a.on_key(key(KeyCode::Char('n')));
        assert_eq!(a.find_status().0, 0, "n wraps back to the first match");
        a.on_key(key(KeyCode::Char('N')));
        assert_eq!(a.find_status().0, 2, "N wraps to the last match");
        a.on_key(key(KeyCode::Char('N')));
        assert_eq!(a.find_status().0, 1, "N retreats to the previous match");
        assert_eq!(
            a.find().map(|f| f.query.as_str()),
            Some("apple"),
            "n/N navigate in NAV mode instead of typing into the query"
        );
    }

    #[test]
    fn find_nav_printable_char_reenters_editing() {
        // In NAV mode a printable char (not n/N) or Backspace returns to EDITING and refines the
        // query.
        let mut a = app();
        a.transcript.push(Entry::Notice("apple pie".to_owned()));
        a.transcript.push(Entry::Notice("apple tart".to_owned()));
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for c in "apple".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.find().map(|f| f.committed), Some(true), "Enter commits");
        a.on_key(key(KeyCode::Char(' ')));
        a.on_key(key(KeyCode::Char('p')));
        assert_eq!(
            a.find().map(|f| f.committed),
            Some(false),
            "typing a printable char re-enters EDITING"
        );
        assert_eq!(
            a.find().map(|f| f.query.as_str()),
            Some("apple p"),
            "the typed characters refine the query"
        );
        assert_eq!(
            a.find_status().1,
            1,
            "`apple p` now matches only `apple pie`"
        );
        // Backspace also re-enters EDITING and edits the query.
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.find().map(|f| f.committed), Some(true), "re-committed");
        a.on_key(key(KeyCode::Backspace));
        assert_eq!(
            a.find().map(|f| f.committed),
            Some(false),
            "Backspace re-enters EDITING"
        );
        assert_eq!(
            a.find().map(|f| f.query.as_str()),
            Some("apple "),
            "Backspace deletes the last query char"
        );
    }

    #[test]
    fn find_opens_in_editing_mode() {
        // committed defaults to false on open (EDITING mode); see the ui.rs render test for the
        // caret + hint rendering that stays identical in both modes.
        let mut a = app();
        a.transcript
            .push(Entry::Notice("registry schema".to_owned()));
        a.on_key(key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert_eq!(
            a.find().map(|f| f.committed),
            Some(false),
            "committed defaults to false on open"
        );
    }

    #[test]
    fn ctrl_k_and_ctrl_n_navigate_messages() {
        // VAL-TUI-079: message navigation moves the selection between messages.
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::User("second".to_owned()));
        a.transcript.push(Entry::User("third".to_owned()));
        assert_eq!(a.selected_message(), None);
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(
            a.selected_message(),
            Some(2),
            "first selection is the last message"
        );
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(a.selected_message(), Some(1));
        a.on_key(key_mod(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(a.selected_message(), Some(2));
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.selected_message(), None, "Esc clears the selection");
    }

    #[test]
    fn alt_arrows_are_inert_for_message_navigation() {
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::User("second".to_owned()));
        a.on_key(key_mod(KeyCode::Up, KeyModifiers::ALT));
        assert_eq!(a.selected_message(), None);
        a.on_key(key_mod(KeyCode::Down, KeyModifiers::ALT));
        assert_eq!(a.selected_message(), None);
    }

    #[test]
    fn esc_prefixed_csi_chars_do_not_clear_or_leak() {
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::User("second".to_owned()));
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(a.selected_message(), Some(1));
        a.on_key(key(KeyCode::Esc));
        assert_eq!(
            a.selected_message(),
            None,
            "plain Esc still clears selection"
        );
        a.on_key(key(KeyCode::Char('[')));
        a.on_key(key(KeyCode::Char('A')));
        a.on_key(key(KeyCode::Left));
        assert_eq!(
            a.selected_message(),
            Some(1),
            "Esc+[A restores the pre-Esc selection instead of clearing it"
        );
        assert_eq!(
            a.input.value(),
            "",
            "CSI fragment bytes are swallowed and never leak into the prompt"
        );
    }

    #[test]
    fn esc_prefixed_bare_up_is_inert() {
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::User("second".to_owned()));
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(a.selected_message(), Some(1));
        a.on_key(key(KeyCode::Esc));
        a.on_key(key(KeyCode::Up));
        assert_eq!(
            a.selected_message(),
            Some(1),
            "Esc followed by bare Up is treated as inert Alt+Up noise"
        );
        assert_eq!(a.input.value(), "");
    }

    #[test]
    fn esc_prefixed_reversed_csi_chars_do_not_clear_or_leak() {
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::User("second".to_owned()));
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(a.selected_message(), Some(1));
        a.on_key(key(KeyCode::Esc));
        a.on_key(key(KeyCode::Char('A')));
        a.on_key(key(KeyCode::Char('[')));
        a.on_key(key(KeyCode::Left));
        assert_eq!(
            a.selected_message(),
            Some(1),
            "Esc+A[ restores the pre-Esc selection instead of clearing it"
        );
        assert_eq!(
            a.input.value(),
            "",
            "reversed CSI fragment bytes are swallowed and never leak into the prompt"
        );
    }

    #[test]
    fn esc_then_fragment_like_text_is_preserved() {
        let mut a = app();
        a.on_key(key(KeyCode::Esc));
        for c in "A[0;9O;B".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            a.input.value(),
            "A[0;9O;B",
            "Esc must not swallow normal prompt text, even fragment-like chars"
        );
    }

    #[test]
    fn esc_preserves_each_fragment_like_starter_char() {
        for sequence in ["Ax", "Bx", "0", ";", "Ox"] {
            let mut a = app();
            a.on_key(key(KeyCode::Esc));
            for c in sequence.chars() {
                a.on_key(key(KeyCode::Char(c)));
            }
            assert_eq!(
                a.input.value(),
                sequence,
                "sequence `{sequence}` is preserved"
            );
        }
    }

    #[test]
    fn esc_then_bracket_prefixed_text_is_not_discarded() {
        let mut a = app();
        a.on_key(key(KeyCode::Esc));
        a.on_key(key(KeyCode::Char('[')));
        a.on_key(key(KeyCode::Char('x')));
        assert_eq!(
            a.input.value(),
            "[x",
            "incomplete/unrecognized probe candidates are flushed back into the prompt"
        );
    }

    #[test]
    fn esc_then_incomplete_probe_text_survives_submit() {
        let mut a = app();
        a.on_key(key(KeyCode::Esc));
        a.on_key(key(KeyCode::Char('[')));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            a.take_pending_prompt().as_deref(),
            Some("["),
            "buffered probe text is preserved before non-char keys like Enter"
        );
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
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
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
    fn toast_screen_shows_the_chat_breadcrumb() {
        // VAL-MD-027: with one or more toasts active the chrome shows `✦ Cortex Code · chat` over
        // the conversation (the trailing `chat` crumb in AMBER).
        let mut a = app();
        a.push_toast(ToastLevel::Success, "registry schema updated");
        let buf = render_buffer(&a, 92, 30);
        let row0 = (0..92).map(|x| buf[(x, 0)].symbol()).collect::<String>();
        assert!(row0.contains("Cortex Code"), "wordmark on row 0:\n{row0}");
        assert!(
            row0.contains("chat"),
            "trailing `chat` crumb on row 0:\n{row0}"
        );
        // The chrome header is flat (no rounded box corner) — distinct from the MOTD card border.
        assert!(
            !row0.contains('╭'),
            "the toast chrome header row is flat, not the MOTD card border:\n{row0}"
        );
        // The `chat` crumb is AMBER (the active trailing crumb color).
        let theme = frame::style::ThemeColors::from_name(a.theme());
        let chat_start = row0.find("chat").expect("chat crumb present");
        let chat_cell = &buf[(chat_start as u16, 0)];
        assert_eq!(
            chat_cell.fg, theme.secondary,
            "the trailing `chat` crumb is AMBER"
        );
        // With no toasts the plain conversation shows the MOTD card border on row 0 (the flat
        // `✦ Cortex Code · chat` chrome header is NOT drawn), so no `chat` crumb appears.
        let mut plain_app = app();
        plain_app.tick_toasts();
        assert!(!plain_app.has_toasts());
        let plain_buf = render_buffer(&plain_app, 92, 30);
        let plain_row0 = (0..92)
            .map(|x| plain_buf[(x, 0)].symbol())
            .collect::<String>();
        assert!(
            !plain_row0.contains("chat"),
            "no `chat` breadcrumb crumb without toasts:\n{plain_row0}"
        );
        assert!(
            plain_row0.contains('╭'),
            "the plain conversation shows the MOTD card border on row 0:\n{plain_row0}"
        );
    }

    #[test]
    fn toast_band_shows_icon_colored_label_and_bright_message() {
        // VAL-MD-028 / VAL-MD-029: a toast band shows `[icon] Level: message` — icon + label in the
        // level color, message body in BRIGHT — in that order.
        let mut a = app();
        a.set_reduced_motion(true); // static band at its resting column for deterministic columns
        a.push_toast(ToastLevel::Success, "registry schema updated");
        let theme = frame::style::ThemeColors::from_name(a.theme());
        let buf = render_buffer(&a, 92, 30);

        // Find the band's header row (contains the icon + label + message in order).
        let header_row = (0..30)
            .find(|&y| {
                let row = (0..92).map(|x| buf[(x, y)].symbol()).collect::<String>();
                row.contains("[+] Success: registry schema updated")
            })
            .expect("a Success toast header band is rendered");
        let row: String = (0..92).map(|x| buf[(x, header_row)].symbol()).collect();

        // Icon `[+]` and the label `Success:` are the level color (green for success).
        let icon_x = row.find("[+]").expect("icon present") as u16;
        assert_eq!(
            buf[(icon_x, header_row)].fg,
            theme.success,
            "icon is level-colored"
        );
        let label_x = row.find("Success:").expect("label present") as u16;
        assert_eq!(
            buf[(label_x, header_row)].fg,
            theme.success,
            "label is level-colored"
        );

        // The message body is BRIGHT (distinct from the level color).
        let msg_x = row.find("registry").expect("message present") as u16;
        assert_eq!(
            buf[(msg_x, header_row)].fg,
            theme.text_bright,
            "message body is BRIGHT"
        );
        assert_ne!(
            buf[(msg_x, header_row)].fg,
            theme.success,
            "message is not the level color"
        );

        // Every cell of the band header carries an explicit (non-page) background — no black bars.
        assert_ne!(
            buf[(icon_x, header_row)].bg,
            theme.background,
            "band cells carry the panel bg, not the page canvas"
        );
    }

    #[test]
    fn toast_shows_eighths_countdown_bar_with_seconds() {
        // VAL-MD-030: each toast shows an eighths countdown bar tinted to the level color with the
        // remaining seconds right-aligned in FAINT (e.g. `3s`).
        let mut a = app();
        a.set_reduced_motion(true);
        a.push_toast(ToastLevel::Error, "tool 'deploy' not found");
        let theme = frame::style::ThemeColors::from_name(a.theme());
        let buf = render_buffer(&a, 92, 30);

        // The bar row sits directly below the header row.
        let header_row = (0..30)
            .find(|&y| {
                let row = (0..92).map(|x| buf[(x, y)].symbol()).collect::<String>();
                row.contains("[x] Error:")
            })
            .expect("an Error toast header is rendered");
        let bar_row = header_row + 1;
        let row: String = (0..92).map(|x| buf[(x, bar_row)].symbol()).collect();

        // The bar contains eighths block glyphs (the full block `█` at minimum).
        let eighths = ["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];
        assert!(
            eighths.iter().any(|g| row.contains(g)),
            "bar row is composed of eighths blocks:\n{row}"
        );
        // A `Ns` seconds label appears near the right of the band.
        assert!(
            row.contains("3s") || row.contains("2s") || row.contains("1s"),
            "a `Ns` seconds label is shown:\n{row}"
        );
        // A filled bar cell is tinted to the level color (ERR red for an error toast). Search only
        // within the band region (right half) so a mascot `█` on the same row is not sampled.
        let filled_x = (46..92u16)
            .find(|&x| buf[(x, bar_row)].symbol() == "█")
            .expect("a filled bar cell in the band region");
        assert_eq!(
            buf[(filled_x, bar_row)].fg,
            theme.error,
            "the filled bar is the level color"
        );
    }

    #[test]
    fn multiple_toasts_stack_without_overlap() {
        // VAL-MD-034: several active toasts stack as separate bands with uniform 3-row spacing.
        let mut a = app();
        a.set_reduced_motion(true);
        a.push_toast(ToastLevel::Success, "registry schema updated");
        a.push_toast(ToastLevel::Info, "switched model");
        a.push_toast(ToastLevel::Warning, "context at 82% capacity");
        a.push_toast(ToastLevel::Error, "tool not found");
        let buf = render_buffer(&a, 92, 30);

        let header_row_of = |needle: &str| {
            (0..30).find(|&y| {
                (0..92)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .contains(needle)
            })
        };
        let rows: Vec<u16> = ["Success:", "Info:", "Warning:", "Error:"]
            .iter()
            .map(|n| header_row_of(n).unwrap_or_else(|| panic!("band {n} rendered")))
            .collect();
        // The four bands occupy four distinct rows, each 3 rows below the previous (no overlap).
        for pair in rows.windows(2) {
            assert_eq!(
                pair[1] - pair[0],
                3,
                "bands are spaced 3 rows apart: {rows:?}"
            );
        }
    }

    #[test]
    fn no_anim_renders_a_static_toast_with_level_message_and_countdown() {
        // VAL-MD-033 / VAL-FOUND-024: under --no-anim a toast appears statically at its final
        // position (no slide offset) while still showing the level color, message, and countdown.
        let mut anim_off = app();
        anim_off.set_reduced_motion(true);
        anim_off.push_toast(ToastLevel::Info, "switched model");
        let theme = frame::style::ThemeColors::from_name(anim_off.theme());
        let buf = render_buffer(&anim_off, 92, 30);

        // Level + message present.
        let header_row = (0..30)
            .find(|&y| {
                (0..92)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .contains("[i] Info: switched model")
            })
            .expect("static Info toast header rendered under --no-anim");
        // The band is at its resting column (right edge): with band width 44 on a 92-col frame the
        // band starts at col 46, so nothing is drawn left of the transcript region there.
        let row: String = (0..92).map(|x| buf[(x, header_row)].symbol()).collect();
        let icon_x = row.find("[i]").expect("icon present");
        assert!(
            icon_x >= 40,
            "band rests at the right (no slide offset): icon at {icon_x}"
        );
        // Countdown bar present on the next row.
        let bar_row_text: String = (0..92).map(|x| buf[(x, header_row + 1)].symbol()).collect();
        assert!(
            bar_row_text.contains('█') || bar_row_text.contains('▊') || bar_row_text.contains('▋'),
            "countdown bar still rendered under --no-anim:\n{bar_row_text}"
        );
        // Level color still applied to the icon.
        assert_eq!(buf[(icon_x as u16, header_row)].fg, theme.info);
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
        a.on_key(key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL));
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

    // --- undo/redo (VAL-TUI-043/044/085/086/087/088) ---------------------------------------------

    static UNDO_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// A unique temp workspace plus a shadow store rooted at it, for on-disk undo/redo tests.
    fn checkpoint_scratch(tag: &str) -> (PathBuf, ShadowStore) {
        let unique = UNDO_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cortex-tui-undo-{tag}-{}-{unique}",
            std::process::id()
        ));
        let workspace = root.join("ws");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&workspace).expect("create workspace");
        let store = ShadowStore::open(&workspace, &root.join("shadow")).expect("open store");
        (workspace, store)
    }

    /// Simulate a file-affecting turn: capture `file`'s pre-image, write `after`, and finalize the
    /// snapshot — exactly what the background turn worker does around a real mutating turn.
    fn record_change(store: &ShadowStore, workspace: &std::path::Path, file: &str, after: &str) {
        let recorder = store.begin_turn("turn");
        recorder.record_pre_image(&workspace.join(file));
        std::fs::write(workspace.join(file), after).expect("write change");
        recorder.finalize().expect("finalize snapshot");
    }

    fn last_notice(a: &App) -> String {
        a.transcript
            .iter()
            .rev()
            .find_map(|e| match e {
                Entry::Notice(text) => Some(text.clone()),
                _ => None,
            })
            .expect("a notice entry")
    }

    #[test]
    fn restore_report_wording_covers_each_outcome() {
        // VAL-TUI-043/044: a restore confirms the revert/re-apply; the graceful empty case reads
        // clearly; a conflict is reported without claiming success.
        let restored = RestoreOutcome::Restored {
            label: "t".to_owned(),
            paths: vec!["a.txt".to_owned()],
        };
        let (notice, level, _) = restore_report(false, &restored);
        assert!(
            notice.contains("Undid") && notice.contains("reverted") && notice.contains("a.txt")
        );
        assert_eq!(level, ToastLevel::Success);

        let (notice, _, _) = restore_report(true, &restored);
        assert!(notice.contains("Redid") && notice.contains("re-applied"));

        let (notice, level, _) = restore_report(false, &RestoreOutcome::Nothing);
        assert_eq!(notice, "Nothing to undo.");
        assert_eq!(level, ToastLevel::Info);

        let conflict = RestoreOutcome::Conflict {
            label: "t".to_owned(),
            conflicts: vec!["a.txt".to_owned()],
        };
        let (notice, level, _) = restore_report(false, &conflict);
        assert!(notice.contains("Cannot undo") && notice.contains("a.txt"));
        assert_eq!(level, ToastLevel::Error);
    }

    #[test]
    fn undo_restores_then_redo_reapplies_the_file_on_disk() {
        // VAL-TUI-085/086: `/undo` reverts the on-disk file to its pre-change content; `/redo`
        // re-applies the change on disk. Each confirms via a transcript notice.
        let (workspace, store) = checkpoint_scratch("roundtrip");
        std::fs::write(workspace.join("f.txt"), "ORIGINAL\n").unwrap();
        record_change(&store, &workspace, "f.txt", "CHANGED\n");
        assert_eq!(
            std::fs::read_to_string(workspace.join("f.txt")).unwrap(),
            "CHANGED\n"
        );

        let mut a = app();
        a.set_checkpoint_store(Some(store));

        a.undo();
        assert_eq!(
            std::fs::read_to_string(workspace.join("f.txt")).unwrap(),
            "ORIGINAL\n",
            "undo restores the pre-change bytes on disk"
        );
        assert!(
            last_notice(&a).contains("reverted"),
            "undo confirms the revert"
        );

        a.redo();
        assert_eq!(
            std::fs::read_to_string(workspace.join("f.txt")).unwrap(),
            "CHANGED\n",
            "redo re-applies the change on disk"
        );
        assert!(
            last_notice(&a).contains("re-applied"),
            "redo confirms the re-apply"
        );
    }

    #[test]
    fn undo_via_keybind_matches_the_slash_command() {
        // VAL-TUI-087: the undo/redo keybinds (leader + u / r) perform the same on-disk revert and
        // re-apply as the slash commands, sharing one implementation.
        let (workspace, store) = checkpoint_scratch("keybind");
        std::fs::write(workspace.join("f.txt"), "BEFORE\n").unwrap();
        record_change(&store, &workspace, "f.txt", "AFTER\n");

        let mut a = app();
        a.set_checkpoint_store(Some(store));

        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('u')));
        assert_eq!(
            std::fs::read_to_string(workspace.join("f.txt")).unwrap(),
            "BEFORE\n",
            "the undo keybind reverts the file"
        );

        press_leader(&mut a);
        a.on_key(key(KeyCode::Char('r')));
        assert_eq!(
            std::fs::read_to_string(workspace.join("f.txt")).unwrap(),
            "AFTER\n",
            "the redo keybind re-applies the change"
        );
    }

    #[test]
    fn undo_with_nothing_to_revert_is_graceful() {
        // VAL-TUI-088: undo on a fresh session with no checkpointed change reports a clear
        // "nothing to undo" note rather than erroring or changing state.
        let (_workspace, store) = checkpoint_scratch("empty");
        let mut a = app();
        a.set_checkpoint_store(Some(store));

        a.undo();
        assert_eq!(last_notice(&a), "Nothing to undo.");
        assert!(!a.should_quit, "a no-op undo never disrupts the session");
    }

    #[test]
    fn undo_without_a_store_is_a_graceful_no_op() {
        // With no resolvable checkpoint store, undo degrades to the same graceful note.
        let mut a = app();
        a.undo();
        assert_eq!(last_notice(&a), "Nothing to undo.");
    }

    /// Open a fresh store handle at the same shadow directory `root/shadow` a previous handle used,
    /// modelling a SEPARATE surface (a headless `cortex run`, or a relaunched TUI) resolving the one
    /// shared, workspace-keyed location through `ShadowStore::open_default`.
    fn reopen_store(root: &std::path::Path, workspace: &std::path::Path) -> ShadowStore {
        ShadowStore::open(workspace, &root.join("shadow")).expect("reopen shared store")
    }

    // VAL-CROSS-018: a file edited by a separate (headless) surface is reverted by the TUI's `/undo`
    // on the SHARED shadow store. The headless turn records its checkpoint through one store handle;
    // the TUI then opens the same shadow location at launch and undo restores the pre-edit bytes
    // byte-for-byte (the in-process analogue of the black-box headless-run -> TUI-`/undo` flow).
    #[test]
    fn a_separate_surface_edit_is_reverted_by_tui_undo_on_the_shared_store() {
        let (workspace, store_a) = checkpoint_scratch("cross-headless");
        let root = store_a
            .git_dir()
            .parent()
            .expect("shadow parent")
            .to_owned();
        let h0 = b"H0 pre-edit bytes\nsecond line\n";
        std::fs::write(workspace.join("target.txt"), h0).unwrap();

        // The "headless turn" records its checkpoint, edits the file to H1, then exits (handle drop).
        record_change(
            &store_a,
            &workspace,
            "target.txt",
            "H1 edited by the headless turn\n",
        );
        let h1 = std::fs::read(workspace.join("target.txt")).unwrap();
        assert_ne!(h1.as_slice(), h0, "the headless turn changed the bytes");
        drop(store_a);

        // The TUI launches on the same project: it opens the shared store from disk and `/undo`
        // reverts the headless edit byte-for-byte.
        let mut a = app();
        a.set_checkpoint_store(Some(reopen_store(&root, &workspace)));
        a.undo();

        assert_eq!(
            std::fs::read(workspace.join("target.txt"))
                .unwrap()
                .as_slice(),
            h0,
            "the TUI restores the exact pre-headless-edit bytes from the shared store"
        );
        assert!(
            last_notice(&a).contains("reverted"),
            "the TUI reports the revert"
        );
    }

    // VAL-CROSS-020: a checkpoint created in one TUI session survives exiting and relaunching the
    // TUI and remains undoable. The first session's turn records the checkpoint (durable journal on
    // disk); a fresh App opening the same shadow location (a relaunch) reverts the pre-change bytes
    // byte-for-byte through `/undo`.
    #[test]
    fn a_tui_checkpoint_survives_restart_and_remains_undoable() {
        let (workspace, store_1) = checkpoint_scratch("cross-restart");
        let root = store_1
            .git_dir()
            .parent()
            .expect("shadow parent")
            .to_owned();
        let h0 = b"H0 before the TUI change\n";
        std::fs::write(workspace.join("note.txt"), h0).unwrap();

        // TUI session #1: a turn edits note.txt and finalizes its checkpoint, then the session ends.
        {
            let mut session1 = app();
            session1.set_checkpoint_store(Some(reopen_store(&root, &workspace)));
            record_change(&store_1, &workspace, "note.txt", "H1 changed in the TUI\n");
            assert_eq!(
                std::fs::read(workspace.join("note.txt")).unwrap(),
                b"H1 changed in the TUI\n",
                "the TUI turn changed the bytes"
            );
            // Session #1 (its App and the recording handle) is dropped here: the TUI "exits".
            drop(session1);
            drop(store_1);
        }

        // TUI session #2 (relaunch): a brand-new App opens the same shared store from disk and the
        // checkpoint is still undoable, restoring the pre-change bytes byte-for-byte.
        let mut session2 = app();
        session2.set_checkpoint_store(Some(reopen_store(&root, &workspace)));
        assert!(
            session2
                .checkpoints
                .as_ref()
                .expect("store")
                .history()
                .expect("history")
                .can_undo(),
            "the checkpoint persisted across the TUI restart"
        );
        session2.undo();

        assert_eq!(
            std::fs::read(workspace.join("note.txt"))
                .unwrap()
                .as_slice(),
            h0,
            "after restart the TUI restores the exact pre-change bytes"
        );
        assert!(
            last_notice(&session2).contains("reverted"),
            "the relaunched TUI reports the revert"
        );
    }

    /// A unique temp database path bound to an app for session-persistence tests, so a real DB is
    /// exercised without touching any shared location.
    fn session_store_path(tag: &str) -> PathBuf {
        let unique = UNDO_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cortex-tui-session-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create session store dir");
        root.join("cortex.db")
    }

    // VAL-CROSS-002 (persistence half): a turn driven in the TUI persists its user and assistant
    // messages under a real session and surfaces that session id, so `cortex run --session <id>`
    // can resume the exact conversation.
    #[test]
    fn a_tui_turn_persists_its_exchange_and_surfaces_the_session_id() {
        let path = session_store_path("exchange");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        assert_eq!(
            a.status.session, "new",
            "no session id before the first turn"
        );

        run_turn(&mut a, "remember the word marmalade", "noted: marmalade");

        let id = a.session_id.clone().expect("a session was created");
        assert_eq!(
            a.status.session, id,
            "the footer surfaces the new session id"
        );
        assert!(id.starts_with("ses_"), "a real persisted id, got {id}");

        // The exchange is durable and resumable: reopening the store shows both messages in order.
        let db = db::Database::open(&path).expect("reopen store");
        let messages = db.messages().list_for_session(&id).expect("list messages");
        assert_eq!(messages.len(), 2, "user + assistant persisted");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "remember the word marmalade");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "noted: marmalade");
    }

    #[test]
    fn a_tui_turn_persists_usage_for_stats() {
        let path = session_store_path("usage");
        let mut a = app();
        a.set_session_store(Some(path.clone()));

        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("usage turn".to_owned(), rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "usage answer".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Event(RunnerEvent::Usage(usage(
            7,
            11,
            Some(0.0018),
        ))))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        let id = a.session_id.clone().expect("session created");
        let db = db::Database::open(&path).expect("reopen store");
        let totals = db.usage().session_totals(&id).expect("usage totals");
        assert_eq!(totals.input_tokens, 7);
        assert_eq!(totals.output_tokens, 11);
        assert_eq!(totals.total_tokens, 18);
        assert_eq!(totals.cost, Some(0.0018));
    }

    // A second turn appends to the same session rather than creating a new one, so a multi-turn TUI
    // conversation resumes as a single coherent session headless.
    #[test]
    fn successive_tui_turns_append_to_one_session() {
        let path = session_store_path("append");
        let mut a = app();
        a.set_session_store(Some(path.clone()));

        run_turn(&mut a, "first", "one");
        let id = a.session_id.clone().expect("session created");
        run_turn(&mut a, "second", "two");
        assert_eq!(
            a.session_id.as_deref(),
            Some(id.as_str()),
            "the session id is stable across turns"
        );

        let db = db::Database::open(&path).expect("reopen");
        let messages = db.messages().list_for_session(&id).expect("messages");
        assert_eq!(messages.len(), 4, "both turns persisted under one session");
    }

    #[test]
    fn session_picker_lists_subagent_sessions_with_parent_linkage() {
        let path = session_store_path("subagent-picker");
        let parent_id = "parent-session";
        {
            let db = db::Database::open(&path).expect("open seed");
            db.sessions()
                .ensure(parent_id, Some("parent"))
                .expect("parent");
            db.sessions()
                .create_child(
                    parent_id,
                    Some(&format!("subagent:explore parent:{parent_id}")),
                )
                .expect("child");
        }

        let mut a = app();
        a.set_session_store(Some(path));
        dispatch(&mut a, "sessions");
        let items = a.dialog().expect("session picker").items.clone();

        assert!(
            a.session_entries
                .iter()
                .any(|entry| entry.id.as_deref() == Some(parent_id)),
            "parent session is listed: {items:?}"
        );
        assert!(
            items.iter().any(|item| item.contains("subagent:explore")
                && item.contains(&format!("parent:{parent_id}"))),
            "child subagent row includes the parent linkage: {items:?}"
        );
    }

    #[test]
    fn capture_lsp_sidebar_note_parses_grouped_diagnostics_and_counts() {
        // The `render_feedback_block` shape parses into structured per-file groups: each summary
        // line establishes a file (worst-severity dot source) and following detail lines become
        // E/W diagnostic rows; the panel note sums real error/warning counts.
        let mut a = app();
        a.capture_lsp_sidebar_note(
            "edit",
            None,
            "Diagnostics:\n\
             lsp diagnostics for src/buffer.rs: 1 error(s), 1 warning(s), 0 other\n\
             - error [E0433] (rustc) at line 42, character 5-10: cap unused when 0\n\
             - warning at line 18, character 1-3: unused import Read\n\
             lsp diagnostics for src/mod.rs: 0 error(s), 1 warning(s), 0 other\n\
             - warning at line 90, character 2-8: redundant clone()",
        );
        let files = a.sidebar_lsp_files();
        assert_eq!(files.len(), 2, "one group per diagnosed file");
        assert_eq!(
            files[0].file, "buffer.rs",
            "path is reduced to the basename"
        );
        assert_eq!(
            files[0].severity(),
            LspFileSeverity::Error,
            "worst severity = error"
        );
        assert_eq!(files[0].diagnostics.len(), 2);
        assert_eq!(files[0].diagnostics[0].line, 42);
        assert!(files[0].diagnostics[0].is_error);
        assert_eq!(files[0].diagnostics[0].message, "cap unused when 0");
        assert!(!files[0].diagnostics[1].is_error, "second row is a warning");
        assert_eq!(
            files[1].severity(),
            LspFileSeverity::Warning,
            "warning-only file"
        );
        assert_eq!(a.sidebar_lsp_counts(), (1, 2), "note sums real E/W counts");
        a.sidebar_panel = SidebarPanel::Lsp;
        assert_eq!(a.sidebar_note(), format!("1E {} 2W", glyphs::SEPARATOR));
    }

    #[test]
    fn capture_lsp_sidebar_note_marks_zero_diagnostics_as_clean() {
        // A file reported with zero errors and zero warnings becomes a clean group (empty
        // diagnostics), which the panel renders as a green `○ no problems` row.
        let mut a = app();
        a.capture_lsp_sidebar_note(
            "edit",
            None,
            "lsp diagnostics for src/stream_processor.rs: 0 error(s), 0 warning(s), 0 other",
        );
        let files = a.sidebar_lsp_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file, "stream_processor.rs");
        assert!(
            files[0].diagnostics.is_empty(),
            "clean file has no diagnostic rows"
        );
        assert_eq!(files[0].severity(), LspFileSeverity::Clean);
    }

    #[test]
    fn capture_lsp_sidebar_note_upserts_the_latest_report_per_file() {
        // Re-reporting a file replaces its prior group so the panel always reflects the newest
        // diagnostics rather than stacking duplicates.
        let mut a = app();
        a.capture_lsp_sidebar_note(
            "edit",
            None,
            "lsp diagnostics for src/buffer.rs: 2 error(s), 0 warning(s), 0 other\n\
             - error at line 5, character 1-2: first\n\
             - error at line 9, character 1-2: second",
        );
        a.capture_lsp_sidebar_note(
            "edit",
            None,
            "lsp diagnostics for src/buffer.rs: 0 error(s), 0 warning(s), 0 other",
        );
        let files = a.sidebar_lsp_files();
        assert_eq!(files.len(), 1, "the file is upserted, not duplicated");
        assert_eq!(
            files[0].severity(),
            LspFileSeverity::Clean,
            "latest report wins (clean)"
        );
    }

    #[test]
    fn lsp_sidebar_populates_from_a_real_completed_edit_tool_card() {
        // The regression this locks: real edit/write/apply_patch tool cards emit
        // `tools::Diagnostic::render()` lines (NOT the dead `render_feedback_block` shape), so the
        // panel must ingest that shape. Feed REAL `Diagnostic::render()` output (built from the same
        // `tools::Diagnostic` type the mutating tools re-inject) through the full runner flow — a
        // ToolCall opens the card, a ToolResult with the mutating-tool feedback parts completes it —
        // and confirm the sidebar groups the diagnostics under the edited file with the right
        // severities and line numbers.
        let type_error = tools::Diagnostic::with_range("error", 5, 17, 5, 22, "mismatched types")
            .with_metadata(Some("rustc".to_owned()), Some("E0308".to_owned()));
        let unused =
            tools::Diagnostic::with_range("warning", 3, 9, 3, 12, "unused variable: `tmp`")
                .with_metadata(
                    Some("rustc".to_owned()),
                    Some("unused_variables".to_owned()),
                );
        // The exact two-part shape `render_with_feedback` emits: a summary part, then a
        // `Diagnostics:`-headed block of bare `Diagnostic::render()` rows.
        let feedback_block = format!("Diagnostics:\n{}\n{}", type_error.render(), unused.render());

        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("introduce a type error".to_owned(), rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::ToolCall {
            id: "call_edit".to_owned(),
            name: "edit".to_owned(),
            arguments: r#"{"path":"src/broken.rs","oldString":"a","newString":"b"}"#.to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Event(RunnerEvent::ToolResult {
            id: "call_edit".to_owned(),
            name: "edit".to_owned(),
            parts: vec![
                protocol::ToolResultPart::text("edit: replaced 1 occurrence in src/broken.rs"),
                protocol::ToolResultPart::text(&feedback_block),
            ],
            is_error: false,
        }))
        .unwrap();
        a.poll_turn();

        let files = a.sidebar_lsp_files();
        assert_eq!(
            files.len(),
            1,
            "the edited file forms one diagnostics group"
        );
        assert_eq!(
            files[0].file, "broken.rs",
            "attributed to the edit target basename"
        );
        assert_eq!(
            files[0].severity(),
            LspFileSeverity::Error,
            "worst severity = error"
        );
        assert_eq!(
            files[0].diagnostics.len(),
            2,
            "both render() rows are ingested"
        );
        assert_eq!(files[0].diagnostics[0].line, 5);
        assert!(
            files[0].diagnostics[0].is_error,
            "first row is the E0308 error"
        );
        assert_eq!(files[0].diagnostics[0].message, "mismatched types");
        assert_eq!(files[0].diagnostics[1].line, 3);
        assert!(
            !files[0].diagnostics[1].is_error,
            "second row is the unused-variable warning"
        );
        assert_eq!(files[0].diagnostics[1].message, "unused variable: `tmp`");
        assert_eq!(
            a.sidebar_lsp_counts(),
            (1, 1),
            "note counts the real E/W split"
        );
        a.sidebar_panel = SidebarPanel::Lsp;
        assert_eq!(a.sidebar_note(), format!("1E {} 1W", glyphs::SEPARATOR));
    }

    #[test]
    fn lsp_sidebar_ingests_multiple_real_render_diagnostic_files_with_worst_severity_dots() {
        // Two separate mutating-tool results (a warning-only file, then an error file) each carry
        // real `Diagnostic::render()` rows attributed to their own target; the panel keeps both
        // groups with the correct worst-severity dot per file (VAL-SIDE-020/021/023).
        let warn_only = tools::Diagnostic::new("warning", 12, "unused import: `Read`")
            .with_metadata(Some("rustc".to_owned()), Some("unused_imports".to_owned()));
        let hard_error = tools::Diagnostic::new("error", 8, "cannot find value `x` in this scope")
            .with_metadata(Some("rustc".to_owned()), Some("E0425".to_owned()));

        let mut a = app();
        a.capture_lsp_sidebar_note(
            "write",
            Some("src/warn.rs"),
            &format!(
                "write: overwrote src/warn.rs (40 bytes)\nDiagnostics:\n{}",
                warn_only.render()
            ),
        );
        a.capture_lsp_sidebar_note(
            "apply_patch",
            Some("src/broken.rs"),
            &format!(
                "apply_patch: applied 1 change: updated src/broken.rs\nDiagnostics:\n{}",
                hard_error.render()
            ),
        );

        let files = a.sidebar_lsp_files();
        assert_eq!(files.len(), 2, "one group per diagnosed file");
        let warn = files
            .iter()
            .find(|g| g.file == "warn.rs")
            .expect("warn.rs group");
        assert_eq!(
            warn.severity(),
            LspFileSeverity::Warning,
            "warning-only file → amber dot"
        );
        assert_eq!(warn.diagnostics[0].line, 12);
        assert!(!warn.diagnostics[0].is_error);
        let broken = files
            .iter()
            .find(|g| g.file == "broken.rs")
            .expect("broken.rs group");
        assert_eq!(
            broken.severity(),
            LspFileSeverity::Error,
            "error file → red dot"
        );
        assert_eq!(broken.diagnostics[0].line, 8);
        assert!(broken.diagnostics[0].is_error);
    }

    #[test]
    fn lsp_sidebar_ingests_the_cortex_lsp_diagnostics_cli_text_including_clean() {
        // A `bash` tool card running `cortex lsp diagnostics <file>` emits the CLI text shape
        // (`lsp diagnostics for <path>\tstatus=<s>` header + bare `Diagnostic::render()` rows, or
        // `diagnostics=0` when clean). Both the diagnosed and clean forms must ingest, so a clean
        // file renders the green `○ no problems` row (VAL-SIDE-022).
        let err = tools::Diagnostic::with_range("error", 5, 17, 5, 22, "mismatched types")
            .with_metadata(Some("rustc".to_owned()), Some("E0308".to_owned()));
        let mut a = app();
        a.capture_lsp_sidebar_note(
            "bash",
            Some("cortex lsp diagnostics src/broken.rs"),
            &format!(
                "lsp diagnostics for src/broken.rs\tstatus=diagnostics\n\
                 language=rust\nprogram=rust-analyzer\nDiagnostics:\n{}",
                err.render()
            ),
        );
        a.capture_lsp_sidebar_note(
            "bash",
            Some("cortex lsp diagnostics src/clean.rs"),
            "lsp diagnostics for src/clean.rs\tstatus=clean\n\
             language=rust\nprogram=rust-analyzer\ndiagnostics=0",
        );

        let files = a.sidebar_lsp_files();
        assert_eq!(
            files.len(),
            2,
            "both the diagnosed and clean files form groups"
        );
        let broken = files
            .iter()
            .find(|g| g.file == "broken.rs")
            .expect("broken.rs group");
        assert_eq!(broken.severity(), LspFileSeverity::Error);
        assert_eq!(broken.diagnostics[0].line, 5);
        assert!(broken.diagnostics[0].is_error);
        let clean = files
            .iter()
            .find(|g| g.file == "clean.rs")
            .expect("clean.rs group");
        assert_eq!(
            clean.severity(),
            LspFileSeverity::Clean,
            "clean file → green ○ no problems"
        );
        assert!(clean.diagnostics.is_empty());
    }

    #[test]
    fn lsp_sidebar_ignores_unavailable_cli_headers_so_no_false_clean_group_forms() {
        // A `cortex lsp diagnostics` run whose server was unavailable emits `status=unavailable`
        // (+ a `diagnostics=0` line and an `LSP ... unavailable` note). That is NOT a clean result,
        // so it must NOT create a (falsely green) group.
        let mut a = app();
        a.capture_lsp_sidebar_note(
            "bash",
            Some("cortex lsp diagnostics src/clean.rs"),
            "lsp diagnostics for src/clean.rs\tstatus=unavailable\n\
             language=rust\nprogram=rust-analyzer\ndiagnostics=0\n\
             LSP diagnostics unavailable for src/clean.rs: language server stopped responding",
        );
        assert!(
            a.sidebar_lsp_files().is_empty(),
            "an unavailable server forms no group (no false clean row)"
        );
    }

    #[test]
    fn parse_lsp_detail_accepts_both_bare_render_and_legacy_dash_shapes() {
        // The bare `Diagnostic::render()` line (no `- ` prefix, real runtime shape) and the legacy
        // `render_feedback_block` `- ` line both parse; the multi-line-range render form keeps its
        // start line and its colon-bearing message.
        let bare = tools::Diagnostic::with_range("error", 5, 17, 6, 4, "expected `;`, found `}`")
            .with_metadata(Some("rustc".to_owned()), Some("E0308".to_owned()));
        let row = parse_lsp_detail(&bare.render()).expect("bare render row parses");
        assert_eq!(row.line, 5, "anchors on the FIRST `at line <N>`");
        assert!(row.is_error);
        assert_eq!(
            row.message, "expected `;`, found `}`",
            "colon-free message preserved"
        );

        let legacy = parse_lsp_detail("- warning at line 90, character 2-8: redundant clone()")
            .expect("legacy `- ` row still parses");
        assert_eq!(legacy.line, 90);
        assert!(!legacy.is_error);
    }

    #[test]
    fn timeline_view_lists_persisted_turns_on_the_event_rail() {
        // `/timeline` now opens the full-screen Wave-2 view (not a modal dialog); its snapshot lists
        // the session-started milestone plus one event per persisted turn, sourced from the store.
        use crate::view::TimelineDot;
        let path = session_store_path("timeline-view");
        let mut a = app();
        a.set_session_store(Some(path));
        run_turn(&mut a, "first question", "first answer");
        run_turn(&mut a, "second question", "second answer");

        dispatch(&mut a, "timeline");
        assert_eq!(a.active_view(), Some(crate::view::ViewKind::Timeline));
        assert!(a.dialog().is_none(), "the timeline is not a modal dialog");

        let events = a.timeline_snapshot();
        assert!(
            events
                .iter()
                .any(|e| e.who == "session started" && matches!(e.dot, TimelineDot::Filled)),
            "timeline opens with a session-started milestone: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| e.who == "you" && e.text.contains("first question")),
            "timeline lists the user turns: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| e.who == "assistant" && e.text.contains("first answer")),
            "timeline lists the assistant turns: {events:?}"
        );

        // Esc closes the view back to the conversation.
        a.on_key(key(KeyCode::Esc));
        assert!(a.active_view().is_none(), "Esc dismisses the timeline view");
    }

    #[test]
    fn fork_dialog_clones_history_through_selected_timeline_point() {
        let path = session_store_path("fork-dialog");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        run_turn(&mut a, "first question", "first answer");
        run_turn(&mut a, "second question", "second answer");
        let source_id = a.session_id.clone().expect("source session");

        dispatch(&mut a, "fork");
        assert_eq!(
            a.dialog().map(|dialog| dialog.kind),
            Some(DialogKind::Fork),
            "fork opens a timeline-backed fork dialog"
        );

        // default selection is the latest point; move up to assistant #1 (index 1 of 4 rows).
        a.on_key(key(KeyCode::Up));
        a.on_key(key(KeyCode::Up));
        a.on_key(key(KeyCode::Enter));

        let forked_id = a.session_id.clone().expect("forked session");
        assert_ne!(forked_id, source_id, "fork creates a distinct session");

        let db = db::Database::open(&path).expect("reopen store");
        let source_messages = db.messages().list_for_session(&source_id).expect("source");
        let forked_messages = db.messages().list_for_session(&forked_id).expect("forked");
        assert_eq!(source_messages.len(), 4, "seeded full source history");
        assert_eq!(
            forked_messages.len(),
            2,
            "fork keeps history through the selected timeline point only"
        );
        assert_eq!(forked_messages[0].content, "first question");
        assert_eq!(forked_messages[1].content, "first answer");
    }

    #[test]
    fn sidebar_tabs_cover_context_files_lsp_mcp_todo_and_queue() {
        let path = session_store_path("sidebar-panels");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        run_turn(&mut a, "seed", "reply");

        let session_id = a.session_id.clone().expect("session id");
        {
            let db = db::Database::open(&path).expect("open store");
            db.todos()
                .replace_for_session(
                    &session_id,
                    &[protocol::TodoItem::new(
                        "wire sidebar todo",
                        protocol::TodoStatus::Pending,
                    )],
                )
                .expect("seed todo");
        }
        a.load_session(&session_id);

        a.on_key(key_mod(KeyCode::Char('1'), KeyModifiers::CONTROL));
        assert!(a.sidebar_visible(), "Ctrl+1 shows the sidebar");
        assert_eq!(a.sidebar_panel(), SidebarPanel::Context);

        a.on_key(key_mod(KeyCode::Char('2'), KeyModifiers::CONTROL));
        assert_eq!(a.sidebar_panel(), SidebarPanel::Files);

        a.on_key(key_mod(KeyCode::Char('3'), KeyModifiers::CONTROL));
        assert_eq!(a.sidebar_panel(), SidebarPanel::Lsp);

        a.on_key(key_mod(KeyCode::Char('4'), KeyModifiers::CONTROL));
        assert_eq!(a.sidebar_panel(), SidebarPanel::Mcp);

        a.on_key(key_mod(KeyCode::Char('5'), KeyModifiers::CONTROL));
        assert_eq!(a.sidebar_panel(), SidebarPanel::Todo);
        assert!(
            a.sidebar_rows()
                .iter()
                .any(|row| row.contains("wire sidebar todo")),
            "todo panel surfaces persisted session todos"
        );

        // Queue one prompt while busy, then inspect the queue panel.
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("busy".to_owned(), rx, flag());
        type_str(&mut a, "queued prompt row");
        a.on_key(key(KeyCode::Enter));
        a.on_key(key_mod(KeyCode::Char('7'), KeyModifiers::CONTROL));
        assert_eq!(a.sidebar_panel(), SidebarPanel::Queue);
        assert!(
            a.sidebar_rows()
                .iter()
                .any(|row| row.contains("queued prompt row")),
            "queue panel shows queued prompts"
        );
    }

    #[test]
    fn todo_panel_chrome_shows_progress_note_subtitle_and_three_states() {
        // VAL-SIDE-026/027: selecting the todo panel shows crumb `· todo`, title `Todo`, a
        // right-aligned `<done>/<total>` note, a faint `current task list` subtitle, and the three
        // task states (`✓`/`▶`/`○`, never `▸`) in the body.
        let path = session_store_path("side-todo-panel");
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        run_turn(&mut a, "seed", "reply");
        let session_id = a.session_id.clone().expect("session id");
        {
            let db = db::Database::open(&path).expect("open store");
            db.todos()
                .replace_for_session(
                    &session_id,
                    &[
                        protocol::TodoItem::new(
                            "cap the streaming buffer",
                            protocol::TodoStatus::Completed,
                        ),
                        protocol::TodoItem::new(
                            "name the 64KB constant",
                            protocol::TodoStatus::Completed,
                        ),
                        protocol::TodoItem::new(
                            "write tests for the cap",
                            protocol::TodoStatus::InProgress,
                        ),
                        protocol::TodoItem::new(
                            "update CHANGELOG.md",
                            protocol::TodoStatus::Pending,
                        ),
                        protocol::TodoItem::new(
                            "open a pull request",
                            protocol::TodoStatus::Pending,
                        ),
                    ],
                )
                .expect("seed todos");
        }
        a.load_session(&session_id);
        a.on_key(key_mod(KeyCode::Char('5'), KeyModifiers::CONTROL));
        assert_eq!(a.sidebar_panel(), SidebarPanel::Todo);
        // Drop the transient "Sidebar: …" toast so it doesn't overlay the right-hand panel chrome.
        a.toasts.clear();

        let buf = render_buffer(&a, 92, 30);
        let row0 = (0..92).map(|x| buf[(x, 0)].symbol()).collect::<String>();
        assert!(row0.contains("todo"), "breadcrumb crumb is `todo`:\n{row0}");
        let screen = buffer_text(&buf);
        assert!(screen.contains("Todo"), "sidebar title `Todo`");
        assert!(
            screen.contains("2/5"),
            "right-aligned `<done>/<total>` progress note"
        );
        assert!(screen.contains("current task list"), "faint subtitle");
        assert!(
            screen.contains("write tests for the cap"),
            "in-progress task renders"
        );
        assert!(screen.contains('✓'), "completed `✓` marker renders");
        assert!(screen.contains('▶'), "in-progress `▶` marker renders");
        assert!(screen.contains('○'), "pending `○` marker renders");
        assert!(
            !screen.contains('▸') && !screen.contains('▾'),
            "no thin triangles"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn subagents_panel_chrome_shows_count_note_subtitle_and_status_rows() {
        // VAL-SIDE-028/029: selecting the subagents panel shows crumb `· subagents`, title
        // `Subagents`, a right-aligned count note, a faint `background tasks` subtitle, and rows
        // with an `◆` marker + name + right-aligned status + a task description line.
        let path = session_store_path("side-subagents-panel");
        let parent_id = "ses_parent_side";
        {
            let db = db::Database::open(&path).expect("open seed");
            db.sessions()
                .ensure(parent_id, Some("parent"))
                .expect("parent session");
            // A running child (a user message but no assistant reply yet).
            let running = db
                .sessions()
                .create_child(
                    parent_id,
                    Some(&format!(
                        "find auth middleware (subagent:explore parent:{parent_id})"
                    )),
                )
                .expect("running child");
            db.messages()
                .create(&running.id, "user", "find auth middleware")
                .expect("running user message");
            // A done child (an assistant reply present).
            let done = db
                .sessions()
                .create_child(
                    parent_id,
                    Some(&format!(
                        "summarize redaction audit (subagent:general parent:{parent_id})"
                    )),
                )
                .expect("done child");
            db.messages()
                .create(&done.id, "assistant", "summary ready")
                .expect("done assistant message");
        }
        let mut a = app();
        a.set_session_store(Some(path.clone()));
        a.load_session(parent_id);
        a.on_key(key_mod(KeyCode::Char('6'), KeyModifiers::CONTROL));
        assert_eq!(a.sidebar_panel(), SidebarPanel::Subagents);
        a.toasts.clear();

        let buf = render_buffer(&a, 92, 30);
        let row0 = (0..92).map(|x| buf[(x, 0)].symbol()).collect::<String>();
        assert!(
            row0.contains("subagents"),
            "breadcrumb crumb is `subagents`:\n{row0}"
        );
        let screen = buffer_text(&buf);
        assert!(screen.contains("Subagents"), "sidebar title `Subagents`");
        assert!(screen.contains("background tasks"), "faint subtitle");
        assert!(screen.contains('◆'), "the `◆` subagent marker renders");
        assert!(
            screen.contains("explore"),
            "the running subagent name renders"
        );
        assert!(screen.contains("general"), "the done subagent name renders");
        assert!(
            screen.contains("find auth middleware"),
            "the subagent task description renders"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn queue_panel_chrome_shows_count_note_subtitle_and_numbered_rows() {
        // VAL-SIDE-030/031/032: selecting the queue panel shows crumb `· queue`, title `Queue`, a
        // right-aligned count note, a faint `runs after this turn` subtitle, and numbered prompt
        // rows rendered in full (no premature `…`).
        let mut a = app();
        let (_tx, rx) = std::sync::mpsc::channel();
        a.begin_turn("busy".to_owned(), rx, flag());
        for prompt in [
            "add tests for buffer cap",
            "update CHANGELOG.md",
            "run the full test suite",
            "open a PR for the change",
        ] {
            type_str(&mut a, prompt);
            a.on_key(key(KeyCode::Enter));
        }
        a.on_key(key_mod(KeyCode::Char('7'), KeyModifiers::CONTROL));
        assert_eq!(a.sidebar_panel(), SidebarPanel::Queue);
        a.toasts.clear();

        let buf = render_buffer(&a, 92, 30);
        let row0 = (0..92).map(|x| buf[(x, 0)].symbol()).collect::<String>();
        assert!(
            row0.contains("queue"),
            "breadcrumb crumb is `queue`:\n{row0}"
        );
        let screen = buffer_text(&buf);
        assert!(screen.contains("Queue"), "sidebar title `Queue`");
        assert!(screen.contains("runs after this turn"), "faint subtitle");
        assert!(screen.contains('4'), "right-aligned count note (4 queued)");
        assert!(
            screen.contains("add tests for buffer cap"),
            "run-next prompt renders in full"
        );
        assert!(
            screen.contains("open a PR for the change"),
            "later prompt renders in full"
        );
        // These specific prompts fit within the panel interior and are not clipped with `…`.
        let queued = a.sidebar_queue_entries();
        for entry in &queued {
            assert!(
                !entry.contains(glyphs::ELLIPSIS),
                "fitting prompt is verbatim: {entry}"
            );
        }
    }

    // --- Wave 3: context + files panels ------------------------------------------------------

    /// A unique scratch directory for a files-tree test, populated with `spec` relative paths
    /// (a trailing `/` marks a directory to create; otherwise a file with placeholder content).
    fn files_tree_fixture(tag: &str, spec: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "cortex-tui-files-{tag}-{}-{}",
            std::process::id(),
            DIFF_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).expect("create fixture root");
        for entry in spec {
            let path = root.join(entry);
            if entry.ends_with('/') {
                std::fs::create_dir_all(&path).expect("create fixture dir");
            } else {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create fixture parent");
                }
                std::fs::write(&path, b"placeholder\n").expect("write fixture file");
            }
        }
        root
    }

    fn render_buffer(a: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, a)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        buf.content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn context_panel_lists_context_files_with_token_totals() {
        // VAL-SIDE-011/012: the context panel derives its file list + per-file token sizes + the
        // total-size note from real session state (a changed file becomes a context file).
        let cwd = std::env::current_dir().expect("cwd");
        let unique = format!(
            "{}-{}",
            std::process::id(),
            DIFF_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let rel = format!("target/cortex-ctx-{unique}.rs");
        let absolute = cwd.join(&rel);
        std::fs::create_dir_all(absolute.parent().expect("parent")).expect("target dir");
        let content = "a".repeat(8_000);
        std::fs::write(&absolute, &content).expect("seed context file");

        let mut a = app();
        let diff = crate::diff::compute_file_diff(&rel, Some(""), Some(&content)).expect("a diff");
        a.transcript.push(Entry::Diff(diff));

        let files = a.context_files();
        assert_eq!(
            files.len(),
            1,
            "the changed file is the single context file"
        );
        assert!(
            files[0].name.ends_with(".rs") && !files[0].name.contains('/'),
            "row shows the basename: {}",
            files[0].name
        );
        assert_ne!(
            files[0].tokens, "0",
            "per-file token size is a real estimate"
        );
        assert!(a.context_total_tokens() >= 1_000, "total ~ 8000 bytes / 4");
        a.sidebar_visible = true;
        a.sidebar_panel = SidebarPanel::Context;
        assert!(
            a.sidebar_subtitle().contains("1 file in context"),
            "subtitle counts the context file: {}",
            a.sidebar_subtitle()
        );
        std::fs::remove_file(&absolute).ok();
    }

    #[test]
    fn context_panel_renders_marker_counts_and_add_affordance() {
        // VAL-SIDE-011/012/013: `›` ORANGE_D marker + name + DIM token count per file, and a
        // `@ add files to context` affordance with the `@` in ORANGE.
        let cwd = std::env::current_dir().expect("cwd");
        let unique = format!(
            "{}-{}",
            std::process::id(),
            DIFF_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let rel = format!("target/cortex-ctx-render-{unique}.rs");
        let absolute = cwd.join(&rel);
        std::fs::create_dir_all(absolute.parent().expect("parent")).expect("target dir");
        let content = "x".repeat(4_000);
        std::fs::write(&absolute, &content).expect("seed context file");

        let mut a = app();
        let diff = crate::diff::compute_file_diff(&rel, Some(""), Some(&content)).expect("a diff");
        a.transcript.push(Entry::Diff(diff));
        a.sidebar_visible = true;
        a.sidebar_panel = SidebarPanel::Context;

        let theme = frame::style::ThemeColors::from_name(a.theme());
        let buf = render_buffer(&a, 92, 30);
        let text = buffer_text(&buf);
        assert!(text.contains("Context"), "sidebar titled Context");
        assert!(text.contains("1 file in context"), "faint files header");
        assert!(
            text.contains("add files to context"),
            "add-files affordance present"
        );

        // The `›` marker on a file row is the dim-orange (ORANGE_D / border) token.
        let mut prompt_border = false;
        let mut at_orange = false;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                if cell.symbol() == glyphs::PROMPT && cell.fg == theme.border {
                    prompt_border = true;
                }
                if cell.symbol() == "@" && cell.fg == theme.primary {
                    at_orange = true;
                }
            }
        }
        assert!(prompt_border, "context `›` marker is ORANGE_D");
        assert!(at_orange, "`@` add-files glyph is ORANGE");
        std::fs::remove_file(&absolute).ok();
    }

    #[test]
    fn files_tree_builds_disclosure_and_modified_markers() {
        // VAL-SIDE-015/016/018: directories carry `▼`/`▶`, a git-modified file carries `●`, leaves
        // carry no glyph, collapsed dirs hide their children, and names are not prematurely cut.
        let root = files_tree_fixture(
            "build",
            &[
                "crates/",
                "crates/inner.rs",
                "crates/sub/",
                "crates/sub/deep.rs",
                "README.md",
            ],
        );
        let a = app();
        let mut modified = BTreeSet::new();
        modified.insert(root.join("crates/inner.rs"));
        let mut rows = Vec::new();
        a.walk_file_tree(&root, 0, &modified, &mut rows);

        let find = |name: &str| rows.iter().find(|r| r.name == name);
        assert_eq!(
            find("crates/").map(|r| r.kind),
            Some(FileTreeKind::DirExpanded),
            "top-level dir defaults to expanded (▼)"
        );
        assert_eq!(
            find("inner.rs").map(|r| r.kind),
            Some(FileTreeKind::FileModified),
            "the git-modified file carries a ● marker"
        );
        assert_eq!(
            find("README.md").map(|r| r.kind),
            Some(FileTreeKind::File),
            "an unmodified leaf carries no marker"
        );
        assert_eq!(
            find("sub/").map(|r| r.kind),
            Some(FileTreeKind::DirCollapsed),
            "a nested unmodified dir defaults to collapsed (▶)"
        );
        assert!(
            find("deep.rs").is_none(),
            "a collapsed directory hides its children"
        );
        // inner.rs sits one level under crates/ (depth 1), README.md at the root (depth 0).
        assert_eq!(find("inner.rs").map(|r| r.depth), Some(1));
        assert_eq!(find("README.md").map(|r| r.depth), Some(0));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn files_panel_renders_safe_glyphs_and_amber_modified_dot() {
        // VAL-SIDE-014/015/016/018: the files tree renders heavy `▼`/`▶` disclosure (ORANGE_D),
        // an AMBER `●` on modified files, a full long filename (no premature `…`), and never the
        // thin `▾`/`▸` triangles.
        let mut a = app();
        let theme = frame::style::ThemeColors::from_name(a.theme());
        a.sidebar_visible = true;
        a.sidebar_panel = SidebarPanel::Files;
        // Inject a known tree so the rendering is deterministic (independent of the test cwd/git).
        a.files_tree = vec![
            FileTreeRow {
                depth: 0,
                name: "crates/".to_owned(),
                kind: FileTreeKind::DirExpanded,
                path: PathBuf::from("/tmp/crates"),
            },
            FileTreeRow {
                depth: 1,
                name: "engine/".to_owned(),
                kind: FileTreeKind::DirCollapsed,
                path: PathBuf::from("/tmp/crates/engine"),
            },
            FileTreeRow {
                depth: 2,
                name: "stream_processor.rs".to_owned(),
                kind: FileTreeKind::FileModified,
                path: PathBuf::from("/tmp/crates/tui/stream_processor.rs"),
            },
            FileTreeRow {
                depth: 2,
                name: "mod.rs".to_owned(),
                kind: FileTreeKind::File,
                path: PathBuf::from("/tmp/crates/tui/mod.rs"),
            },
        ];

        let buf = render_buffer(&a, 92, 30);
        let text = buffer_text(&buf);
        assert!(text.contains("Files"), "sidebar titled Files");
        assert!(
            text.contains("stream_processor.rs"),
            "a long filename renders in full (no premature truncation)"
        );
        assert!(
            !text.contains('▾') && !text.contains('▸'),
            "no thin disclosure triangles"
        );

        let mut expanded_border = false;
        let mut collapsed_border = false;
        let mut modified_amber = false;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                match cell.symbol() {
                    "▼" if cell.fg == theme.border => expanded_border = true,
                    "▶" if cell.fg == theme.border => collapsed_border = true,
                    "●" if cell.fg == theme.secondary => modified_amber = true,
                    _ => {}
                }
            }
        }
        assert!(expanded_border, "expanded dir uses ▼ in ORANGE_D");
        assert!(collapsed_border, "collapsed dir uses ▶ in ORANGE_D");
        assert!(modified_amber, "modified file uses ● in AMBER");
    }

    #[test]
    fn files_panel_enter_toggles_directory_and_hides_children() {
        // VAL-SIDE-017: with the Files panel open and an empty composer, Enter flips the focused
        // directory's disclosure glyph (▼→▶) and hides/shows its indented children.
        let root = files_tree_fixture("toggle", &["pkg/", "pkg/child.rs", "top.rs"]);
        let mut a = app();
        a.sidebar_visible = true;
        a.sidebar_panel = SidebarPanel::Files;
        a.files_root = root.clone();
        a.rebuild_file_tree();

        let kind_of = |a: &App, name: &str| {
            a.file_tree_rows()
                .iter()
                .find(|r| r.name == name)
                .map(|r| r.kind)
        };
        assert_eq!(
            kind_of(&a, "pkg/"),
            Some(FileTreeKind::DirExpanded),
            "top-level dir starts expanded"
        );
        assert_eq!(
            kind_of(&a, "child.rs"),
            Some(FileTreeKind::File),
            "child visible"
        );

        // Enter toggles the focused directory (cursor at row 0 = pkg/) closed.
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            kind_of(&a, "pkg/"),
            Some(FileTreeKind::DirCollapsed),
            "Enter collapses the directory (▼→▶)"
        );
        assert!(
            kind_of(&a, "child.rs").is_none(),
            "collapsed dir hides its child"
        );

        // Enter again re-expands and reveals the child.
        a.on_key(key(KeyCode::Enter));
        assert_eq!(
            kind_of(&a, "pkg/"),
            Some(FileTreeKind::DirExpanded),
            "Enter re-expands the directory (▶→▼)"
        );
        assert_eq!(
            kind_of(&a, "child.rs"),
            Some(FileTreeKind::File),
            "child visible again"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn files_panel_arrows_move_cursor_and_show_selection_bar() {
        // VAL-SIDE-017 (nav): arrows activate a cursor and draw an inverse selection bar; a fresh
        // panel shows no bar (design-match), matching the reference default.
        let root = files_tree_fixture("nav", &["pkg/", "pkg/child.rs", "top.rs"]);
        let mut a = app();
        a.sidebar_visible = true;
        a.sidebar_panel = SidebarPanel::Files;
        a.files_root = root.clone();
        a.rebuild_file_tree();

        assert!(!a.files_nav_active(), "no selection bar before navigation");
        a.on_key(key(KeyCode::Down));
        assert!(a.files_nav_active(), "Down activates the cursor");
        assert_eq!(a.files_cursor(), 1, "cursor moved to the second row");

        let theme = frame::style::ThemeColors::from_name(a.theme());
        let buf = render_buffer(&a, 92, 30);
        let selected_bar = (0..buf.area.height)
            .any(|y| (0..buf.area.width).any(|x| buf[(x, y)].bg == theme.surface[3]));
        assert!(selected_bar, "the focused row paints an inverse SELBG bar");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn subagent_footer_summary_counts_running_and_done_children() {
        let path = session_store_path("subagent-footer");
        let parent_id = "ses_parent_footer";
        {
            let db = db::Database::open(&path).expect("open seed");
            db.sessions()
                .ensure(parent_id, Some("parent"))
                .expect("parent session");
            let running = db
                .sessions()
                .create_child(
                    parent_id,
                    Some(&format!("subagent:explore parent:{parent_id}")),
                )
                .expect("running child");
            db.messages()
                .create(&running.id, "user", "working")
                .expect("running user message");
            let done = db
                .sessions()
                .create_child(
                    parent_id,
                    Some(&format!("subagent:plan parent:{parent_id}")),
                )
                .expect("done child");
            db.messages()
                .create(&done.id, "assistant", "state: done")
                .expect("done assistant");
        }

        let mut a = app();
        a.set_session_store(Some(path));
        a.load_session(parent_id);
        let (total, running, done) = a.subagent_footer_summary();
        assert_eq!(total, 2);
        assert_eq!(running, 1);
        assert_eq!(done, 1);
    }

    // VAL-GAP-CROSS-008 (TUI leg): a durable background subagent task persisted by the headless flow
    // (child session titled `subagent:<agent> parent:<parent>` with a completed transcript) is shown
    // in the TUI subagent UI after a restart — it surfaces purely from the on-disk session store the
    // TUI shares with `cortex run`, so loading the parent session reveals the child as `done` with
    // its result and parent linkage, no live in-process handle required.
    #[test]
    fn tui_subagent_ui_shows_persisted_background_task_after_restart() {
        let path = session_store_path("subagent-restart");
        let parent_id = "ses_parent_cross008";
        let child_id;
        {
            let db = db::Database::open(&path).expect("open seed");
            db.sessions()
                .ensure(parent_id, Some("parent"))
                .expect("parent session");
            // Mirror exactly what the headless background `task` flow persists: a child session whose
            // title carries subagent/parent linkage, a durable user prompt, and a completed answer.
            let child = db
                .sessions()
                .create_child(
                    parent_id,
                    Some(&format!(
                        "durable background work (subagent:explore parent:{parent_id})"
                    )),
                )
                .expect("child session");
            child_id = child.id.clone();
            db.messages()
                .create(&child.id, "user", "do CHILD_WORK n123")
                .expect("child user message");
            db.messages()
                .create(&child.id, "assistant", "CHILD_ANSWER n123")
                .expect("child assistant message");
        }

        // A fresh TUI app (the "restart") that only knows the shared session store path.
        let mut a = app();
        a.set_session_store(Some(path));
        a.load_session(parent_id);

        let statuses = a.subagent_statuses();
        let row = statuses
            .iter()
            .find(|(id, ..)| id == &child_id)
            .unwrap_or_else(|| panic!("subagent UI must list the persisted child: {statuses:?}"));
        assert_eq!(row.2, "done", "the completed background task shows as done");
        assert!(
            row.1.contains("subagent:explore") && row.1.contains(&format!("parent:{parent_id}")),
            "the subagent row shows the parent linkage: {}",
            row.1
        );
        assert!(
            row.3.contains("CHILD_ANSWER n123"),
            "the subagent row surfaces the durable result detail: {}",
            row.3
        );

        let sidebar = a.sidebar_subagent_rows();
        assert!(
            sidebar.iter().any(|line| line.contains("explore")),
            "the subagents sidebar panel renders the persisted child: {sidebar:?}"
        );

        let (total, running, done) = a.subagent_footer_summary();
        assert_eq!((total, running, done), (1, 0, 1));
    }

    // VAL-CROSS-006 (TUI half): selecting a model for the session via `/models` persists it with the
    // session row, so a headless resume reads it back. Driven through the dialog-confirm path.
    #[test]
    fn selecting_a_model_persists_it_with_the_session() {
        let path = session_store_path("model");
        let mut a = app();
        a.set_session_store(Some(path.clone()));

        a.persist_session_model("anthropic/claude-sonnet-4");
        let id = a.session_id.clone().expect("session created for the model");

        let db = db::Database::open(&path).expect("reopen");
        let session = db.sessions().require(&id).expect("session");
        assert_eq!(
            session.model.as_deref(),
            Some("anthropic/claude-sonnet-4"),
            "the per-session model is persisted"
        );
    }

    // A compaction turn does not persist its internal summarization instruction, so the durable
    // record stays the faithful pre-compaction transcript that a headless resume reads (VAL-CROSS-010).
    #[test]
    fn compaction_does_not_persist_its_internal_prompt() {
        let path = session_store_path("compact");
        let mut a = app();
        a.set_session_store(Some(path.clone()));

        run_turn(&mut a, "remember token ZULU-42", "noted ZULU-42");
        let id = a.session_id.clone().expect("session created");

        // Drive a compaction turn the way `/compact` does (a flagged turn with the summary prompt).
        a.pending_compaction = true;
        let (tx, rx) = std::sync::mpsc::channel();
        a.begin_turn(COMPACTION_PROMPT.to_owned(), rx, flag());
        tx.send(TurnMessage::Event(RunnerEvent::TextDelta {
            text: "Summary: user shared token ZULU-42.".to_owned(),
        }))
        .unwrap();
        tx.send(TurnMessage::Finished).unwrap();
        a.poll_turn();

        // Only the real exchange persisted; the compaction instruction/summary are not recorded, so
        // the durable transcript still contains the original pre-compaction fact verbatim.
        let db = db::Database::open(&path).expect("reopen");
        let messages = db.messages().list_for_session(&id).expect("messages");
        assert_eq!(
            messages.len(),
            2,
            "only the real user+assistant turn persisted"
        );
        assert!(
            messages.iter().all(|m| m.content != COMPACTION_PROMPT),
            "the compaction instruction is never persisted"
        );
        assert!(
            messages.iter().any(|m| m.content.contains("ZULU-42")),
            "the pre-compaction fact survives in the durable record"
        );
    }

    // `/new` detaches from the active session so the next turn starts a fresh one rather than
    // appending to the prior conversation.
    #[test]
    fn new_session_detaches_from_the_active_session() {
        let path = session_store_path("new");
        let mut a = app();
        a.set_session_store(Some(path.clone()));

        run_turn(&mut a, "first", "one");
        let first = a.session_id.clone().expect("first session");

        a.new_session();
        assert_eq!(a.session_id, None, "detached from the prior session");
        assert_eq!(a.status.session, "new", "footer returns to the placeholder");

        run_turn(&mut a, "second", "two");
        let second = a.session_id.clone().expect("second session");
        assert_ne!(first, second, "a fresh session was created");
    }

    // Loading a session from the picker adopts it as active and honors its persisted model, so
    // subsequent turns append to it and the footer matches a headless resume.
    #[test]
    fn loading_a_session_adopts_it_and_honors_its_model() {
        let path = session_store_path("load");
        // Seed a session with a model and a prior message directly in the store.
        let seeded = {
            let db = db::Database::open(&path).expect("open seed");
            let session = db.sessions().create(Some("seed")).expect("create");
            db.sessions()
                .set_model(&session.id, Some("anthropic/claude-sonnet-4"))
                .expect("set model");
            db.messages()
                .create(&session.id, "user", "earlier fact")
                .expect("seed message");
            db.usage()
                .record(&session.id, 12, 34, 46, Some(0.0021))
                .expect("seed usage");
            session.id
        };

        let mut a = app();
        a.set_session_store(Some(path.clone()));
        a.load_session(&seeded);

        assert_eq!(a.session_id.as_deref(), Some(seeded.as_str()), "adopted");
        assert_eq!(
            a.status.model, "anthropic/claude-sonnet-4",
            "the session's model is honored on load"
        );
        assert!(
            a.history_snapshot()
                .iter()
                .any(|m| m.content == "earlier fact"),
            "the prior transcript is loaded into history"
        );
        assert!(
            a.usage.reported,
            "persisted usage is hydrated into the footer"
        );
        assert_eq!(a.usage.input_tokens, 12);
        assert_eq!(a.usage.output_tokens, 34);
        assert_eq!(a.usage.total_tokens, 46);
        assert_eq!(a.usage.cost, Some(0.0021));
    }

    #[test]
    fn loading_a_titled_session_keeps_footer_session_as_canonical_id() {
        let path = session_store_path("load-titled-id");
        let title = "Reply with exactly this token and nothing else: M7_HEADLESS";
        let seeded = {
            let db = db::Database::open(&path).expect("open seed");
            let session = db.sessions().create(Some(title)).expect("create");
            db.messages()
                .create(&session.id, "assistant", "seed answer")
                .expect("seed message");
            session.id
        };

        let mut a = app();
        a.set_session_store(Some(path));
        a.load_session(&seeded);

        assert_eq!(
            a.status.session, seeded,
            "footer/session status must keep the canonical durable id"
        );
        let picks = a.build_session_picks();
        assert!(
            picks.iter().any(|pick| pick.row.key == title),
            "session picker must continue showing the human title: {:?}",
            picks.iter().map(|pick| &pick.row.key).collect::<Vec<_>>()
        );
    }

    #[test]
    fn loading_a_session_renders_persisted_todos() {
        let path = session_store_path("load-todos");
        let seeded = {
            let db = db::Database::open(&path).expect("open seed");
            let session = db.sessions().create(Some("todo seed")).expect("create");
            db.messages()
                .create(&session.id, "assistant", "nonce answer")
                .expect("seed message");
            db.todos()
                .replace_for_session(
                    &session.id,
                    &[
                        protocol::TodoItem::new(
                            "capture message nonce",
                            protocol::TodoStatus::Completed,
                        ),
                        protocol::TodoItem::new(
                            "verify TUI snapshot",
                            protocol::TodoStatus::InProgress,
                        ),
                        protocol::TodoItem::new(
                            "verify server response",
                            protocol::TodoStatus::Pending,
                        ),
                    ],
                )
                .expect("seed todos");
            session.id
        };

        let mut a = app();
        a.set_session_store(Some(path));
        a.load_session(&seeded);

        let rendered = a.render_session_markdown();
        assert!(rendered.contains("nonce answer"), "{rendered}");
        assert!(rendered.contains("Todos"), "{rendered}");
        assert!(
            rendered.contains("1. [completed][medium] capture message nonce"),
            "{rendered}"
        );
        assert!(
            rendered.contains("2. [in_progress][medium] verify TUI snapshot"),
            "{rendered}"
        );
        assert!(
            rendered.contains("3. [pending][medium] verify server response"),
            "{rendered}"
        );
    }

    #[test]
    fn tool_store_context_uses_the_durable_session_and_database() {
        let path = session_store_path("tool-store-context");
        let mut a = app();
        a.set_session_store(Some(path.clone()));

        let (session_id, db_path) = a.tool_store_context_for_turn();

        let session_id = session_id.expect("session id");
        assert!(session_id.starts_with("ses_"), "durable id: {session_id}");
        assert_eq!(db_path.as_deref(), Some(path.as_path()));
        let db = db::Database::open(&path).expect("reopen");
        db.sessions()
            .require(&session_id)
            .expect("context creates the session row");
    }

    // Persistence is gated on a bound store: without one (the unit-test default) a turn never
    // touches a database, so tests stay hermetic.
    #[test]
    fn without_a_store_a_turn_persists_nothing() {
        let mut a = app();
        run_turn(&mut a, "hi", "hello");
        assert_eq!(a.session_id, None, "no session created without a store");
        assert_eq!(a.status.session, "new", "footer unchanged without a store");
    }

    #[test]
    fn idle_remote_abort_marker_is_cleared_without_interrupting_future_turns() {
        let session_id = format!("ses-remote-abort-{}", std::process::id());
        let marker = remote_abort_path(&session_id);
        if let Some(dir) = marker.parent() {
            std::fs::create_dir_all(dir).expect("remote-abort dir");
        }
        std::fs::write(&marker, b"abort\n").expect("seed remote abort marker");

        let mut a = app();
        a.remote_server = Some(
            crate::remote::RemoteServer::from_url("http://127.0.0.1:3200")
                .expect("loopback server url"),
        );
        a.session_id = Some(session_id);

        assert!(
            !a.poll_remote_abort(),
            "idle poll should not report an interrupt"
        );
        assert!(
            !marker.exists(),
            "idle remote poll should clear stale markers to avoid poisoning the next turn"
        );
    }
}
