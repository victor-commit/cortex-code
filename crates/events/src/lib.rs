//! Typed event bus primitives and event definitions backing SSE and in-proc subscribers.
//!
//! The [`EventBus`] is an in-process typed publish/subscribe bus: a single [`EventBus::publish`]
//! delivers a strongly typed [`Event`] to every attached [`Subscriber`] exactly once, and
//! publishing with no subscribers is a clean no-op. Events carry named, typed payload fields
//! (see [`Event`]) and serialize to JSON, so the same definitions back in-process subscribers
//! today and the server's SSE stream in a later milestone.

mod bus;
mod event;

pub use bus::{EventBus, Subscriber};
pub use event::{
    AssistantReasoningDeltaEvent, AssistantTextDeltaEvent, Event, Lifecycle, LifecyclePhase,
    McpLogMessage, McpPromptsChanged, McpResourcesChanged, McpToolsChanged, MessagePartsEvent,
    PermissionAsked, PermissionDecision, PermissionReplied, Surface, ToolCallEvent,
    ToolResultEvent, TurnDoneEvent, TurnErrorEvent, TurnStartedEvent, TurnUsageEvent,
};
