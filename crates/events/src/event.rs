//! Typed event definitions carried on the [`EventBus`](crate::EventBus).

use serde::Serialize;

/// The surface that produced an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// The interactive terminal UI.
    Tui,
    /// The headless `cortex run` path.
    Run,
    /// The local loopback server.
    Serve,
    /// The configuration-reporting path.
    Config,
    /// The persisted-session management path (`cortex session`).
    Session,
}

impl Surface {
    /// The stable lowercase name of this surface.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Surface::Tui => "tui",
            Surface::Run => "run",
            Surface::Serve => "serve",
            Surface::Config => "config",
            Surface::Session => "session",
        }
    }
}

/// The phase of a process/surface lifecycle transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    /// A surface is starting up.
    Startup,
    /// A surface is shutting down.
    Shutdown,
}

impl LifecyclePhase {
    /// The stable lowercase name of this phase.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            LifecyclePhase::Startup => "startup",
            LifecyclePhase::Shutdown => "shutdown",
        }
    }
}

/// A process/surface lifecycle event (e.g. a surface starting up).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Lifecycle {
    /// Whether the surface is starting up or shutting down.
    pub phase: LifecyclePhase,
    /// Which surface emitted the transition.
    pub surface: Surface,
    /// The binary version at the time of the transition.
    pub version: String,
}

/// A typed event delivered on the [`EventBus`](crate::EventBus).
///
/// Each variant pairs an event *kind* with a strongly typed payload, so subscribers
/// (in-process today, the server SSE stream later) observe named fields rather than opaque
/// strings. Variants serialize with an internal `kind` tag, making them SSE/JSON ready.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A process/surface lifecycle transition.
    Lifecycle(Lifecycle),
}

impl Event {
    /// The stable wire/topic name of this event's kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Event::Lifecycle(_) => "lifecycle",
        }
    }

    /// Construct a lifecycle event.
    #[must_use]
    pub fn lifecycle(phase: LifecyclePhase, surface: Surface, version: impl Into<String>) -> Self {
        Event::Lifecycle(Lifecycle {
            phase,
            surface,
            version: version.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_stable() {
        let event = Event::lifecycle(LifecyclePhase::Startup, Surface::Run, "0.1.0");
        assert_eq!(event.kind(), "lifecycle");
    }

    #[test]
    fn lifecycle_serializes_with_named_typed_fields() {
        let event = Event::lifecycle(LifecyclePhase::Startup, Surface::Run, "0.1.0");
        let value = serde_json::to_value(&event).expect("serialize");

        assert_eq!(value["kind"], "lifecycle");
        assert_eq!(value["phase"], "startup");
        assert_eq!(value["surface"], "run");
        assert_eq!(value["version"], "0.1.0");
    }

    #[test]
    fn phase_and_surface_names_match_serialization() {
        assert_eq!(LifecyclePhase::Startup.as_str(), "startup");
        assert_eq!(LifecyclePhase::Shutdown.as_str(), "shutdown");
        assert_eq!(Surface::Tui.as_str(), "tui");
        assert_eq!(Surface::Serve.as_str(), "serve");
        assert_eq!(Surface::Config.as_str(), "config");
    }
}
