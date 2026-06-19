//! Verbose/debug observability: install a stderr tracing subscriber and provide the event-bus
//! sinks that make event delivery observable from the CLI.
//!
//! Observability is opt-in so default invocations keep stdout *and* stderr clean. It is enabled
//! by any of: one or more `-v/--verbose` flags, a non-empty `RUST_LOG`, or a truthy
//! `CORTEX_VERBOSE`. When enabled, logs (including each delivered event) are routed to stderr;
//! stdout is never touched.

use std::sync::atomic::{AtomicUsize, Ordering};

use events::{Event, Subscriber};
use tracing_subscriber::EnvFilter;

/// Resolve whether verbose/debug observability is requested and, if so, install a stderr
/// tracing subscriber. Returns `true` when event-bus sinks should be attached.
pub fn init(verbose: u8) -> bool {
    let rust_log = std::env::var("RUST_LOG")
        .ok()
        .filter(|value| !value.is_empty());
    let enabled = verbose > 0 || rust_log.is_some() || cortex_verbose();
    if !enabled {
        return false;
    }

    let filter = match rust_log {
        Some(value) => EnvFilter::new(value),
        None => {
            let level = if verbose >= 2 { "trace" } else { "debug" };
            EnvFilter::new(format!("cortex={level}"))
        }
    };

    // `try_init` so repeated calls within a single test process do not panic on a second install.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true)
        .try_init();

    true
}

/// `CORTEX_VERBOSE` is truthy when set to anything other than empty, `0`, or `false`.
fn cortex_verbose() -> bool {
    std::env::var_os("CORTEX_VERBOSE").is_some_and(|raw| {
        let value = raw.to_string_lossy();
        let value = value.trim();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

/// The debug/verbose log sink: emits one structured stderr line per delivered event, carrying
/// the event kind and its typed payload fields (serialized to JSON), so delivery is observable.
pub struct LogSink;

impl Subscriber for LogSink {
    fn on_event(&self, event: &Event) {
        tracing::debug!(
            target: "cortex::events",
            kind = event.kind(),
            payload = %redacted_payload(event),
            "event delivered to subscriber"
        );
    }
}

/// Serialize an event to JSON for logging, scrubbing API-key-shaped secrets to the canonical
/// [`redaction::PLACEHOLDER`] so a secret carried on the bus never reaches the log sink in
/// cleartext.
fn redacted_payload(event: &Event) -> String {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| "<unserializable>".to_owned());
    redaction::redact(&payload)
}

/// A second in-process subscriber that tallies deliveries, used to demonstrate that a single
/// publish reaches every subscriber exactly once (no loss, no duplication).
#[derive(Default)]
pub struct CounterSink {
    count: AtomicUsize,
}

impl CounterSink {
    /// Create a sink with a zero tally.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of events observed by this subscriber so far.
    #[must_use]
    pub fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// Emit the running tally to the debug log so the second subscriber's view is observable.
    pub fn report(&self) {
        tracing::debug!(
            target: "cortex::events",
            observed = self.count(),
            "event-bus subscriber tally"
        );
    }
}

impl Subscriber for CounterSink {
    fn on_event(&self, _event: &Event) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use events::{LifecyclePhase, Surface};
    use std::sync::Arc;

    #[test]
    fn counter_sink_counts_each_delivery() {
        let counter = CounterSink::new();
        let event = Event::lifecycle(LifecyclePhase::Startup, Surface::Run, "0.1.0");
        assert_eq!(counter.count(), 0);
        counter.on_event(&event);
        counter.on_event(&event);
        assert_eq!(counter.count(), 2);
    }

    #[test]
    fn log_payload_scrubs_a_secret_carried_on_an_event() {
        // Assembled from fragments so no contiguous key-shaped literal lives in source.
        let secret = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(4));
        let event = Event::lifecycle(LifecyclePhase::Startup, Surface::Run, secret.as_str());
        let payload = redacted_payload(&event);
        assert!(
            !payload.contains(&secret),
            "the raw secret must not reach the log payload: {payload}"
        );
        assert!(
            payload.contains(redaction::PLACEHOLDER),
            "the canonical placeholder must mark the redaction: {payload}"
        );
    }

    #[test]
    fn counter_sink_is_usable_as_a_boxed_subscriber() {
        let counter: Arc<CounterSink> = Arc::new(CounterSink::new());
        let dynamic: Arc<dyn Subscriber> = counter.clone();
        dynamic.on_event(&Event::lifecycle(
            LifecyclePhase::Startup,
            Surface::Run,
            "0.1.0",
        ));
        assert_eq!(counter.count(), 1);
    }
}
