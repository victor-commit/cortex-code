//! The Cortex Code durable session runner (the agent loop) and its prompt/context assembly.
//!
//! This crate is the one runtime engine (architecture §3.3): it drives a turn from the
//! system-context epoch through streamed provider round-trips to a single coherent final answer.
//! Its pieces are:
//!
//! - [`ModelFamily`] / base prompts — the per-family base system prompt ([`family`]).
//! - [`Agent`] — the selectable agent persona whose fragment shapes the turn ([`agent`]).
//! - [`EnvironmentBlock`] — the dynamic environment snapshot (cwd, OS/platform, date, git status)
//!   injected into the system context ([`environment`]).
//! - [`SystemContext`] — the system-context epoch: base prompt + environment block + persona
//!   ([`context`]).
//! - [`project_history`] — projection of stored history into canonical [`llm::Message`]s
//!   ([`history`]).
//! - [`RunnerEvent`] / [`RunOutcome`] — the single normalized output vocabulary and turn outcome
//!   ([`event`]).
//! - [`SessionRunner`] — the outer/inner loop that executes streamed turns and carries multi-round
//!   turns forward to one final answer ([`runner`]).
//!
//! The runner consumes the `llm` crate's normalized [`llm::LlmEvent`] stream and re-emits a single
//! turn-level event vocabulary ordered so concatenated text deltas equal the final answer, with
//! usage and exactly one terminal `Done` at the end. No mocks live on the shipped path: the same
//! runner code drives a real OpenRouter turn and the offline tests, differing only in the provider
//! implementation behind the [`llm::Provider`] trait.

mod agent;
mod compaction;
mod context;
mod environment;
mod event;
mod family;
mod history;
mod message_parts;
mod runner;
mod stream_lifecycle;
mod tools;

pub use agent::{Agent, UnknownAgent};
pub use compaction::{
    CompactionPolicy, DEFAULT_CONTEXT_LIMIT_TOKENS, DEFAULT_TAIL_MESSAGES, MAX_INPUT_BUDGET_TOKENS,
    estimate_messages_tokens, estimate_tokens, is_context_overflow,
};
pub use context::SystemContext;
pub use environment::{EnvironmentBlock, GitStatus};
pub use event::{CompactionRecord, RunError, RunOutcome, RunnerEvent};
pub use family::ModelFamily;
pub use history::{HistoryMessage, project_history, project_role};
pub use message_parts::build_assistant_message_parts;
pub use runner::{DEFAULT_MAX_STEPS, SessionRunner, Turn};
pub use stream_lifecycle::{
    PartLifecycleEvent, PartLifecycleKind, PartLifecyclePhase, PartLifecycleTracker, agent_part_id,
    compaction_part_id, file_part_id, reasoning_part_id, retry_part_id, step_finish_part_id,
    step_part_id, step_start_part_id, text_part_id, tool_pending_part_id, tool_running_part_id,
    tool_terminal_part_id, usage_part_id,
};
pub use tools::{ToolDispatchResult, ToolDispatcher};
