//! Live OpenRouter test: a real streamed turn against `google/gemini-2.5-flash`.
//!
//! This exercises the SHIPPED path end-to-end with no mocks: the rustls transport opens a real
//! streamed POST to OpenRouter and the bytes are normalized into `LlmEvent`s. It is gated on
//! `OPENROUTER_API_KEY` and skips cleanly when the key is absent (so the offline gate stays green);
//! mission validation runs it with the key exported:
//! `set -a; . /projects/cortex-code/.env.local; set +a`.

use futures_util::StreamExt;
use llm::{LlmEvent, LlmRequest, OpenAiCompatible, OpenAiConfig, Provider};

#[tokio::test]
async fn openrouter_real_streamed_turn_yields_text_and_done() {
    let Ok(api_key) = std::env::var("OPENROUTER_API_KEY") else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    if api_key.trim().is_empty() {
        eprintln!("skipping live test: OPENROUTER_API_KEY is empty");
        return;
    }

    let provider = OpenAiCompatible::new(
        OpenAiConfig::openrouter(api_key)
            .header("HTTP-Referer", "https://cortex.code")
            .header("X-Title", "Cortex Code"),
    )
    .expect("build provider");

    let request = LlmRequest::new("google/gemini-2.5-flash")
        .system("You are a terse assistant. Reply with a single short word.")
        .user("Reply with exactly the word: pong")
        .max_tokens(32);

    let stream = provider
        .stream(request)
        .await
        .expect("live stream should open");
    let events: Vec<LlmEvent> = stream.collect().await;

    let text: String = events
        .iter()
        .filter_map(|e| match e {
            LlmEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();

    let text_delta_count = events
        .iter()
        .filter(|e| matches!(e, LlmEvent::TextDelta { .. }))
        .count();

    assert!(
        text_delta_count >= 1,
        "expected at least one TextDelta, got events: {events:?}"
    );
    assert!(
        !text.trim().is_empty(),
        "assistant text should be non-empty, got events: {events:?}"
    );

    let last = events.last().expect("at least one event");
    assert!(
        matches!(last, LlmEvent::Done { .. }),
        "stream must end in a terminal Done, got: {last:?}"
    );

    // No normalized error event should appear on a healthy turn.
    assert!(
        !events.iter().any(|e| matches!(e, LlmEvent::Error { .. })),
        "healthy turn must not emit an Error event: {events:?}"
    );

    eprintln!("live turn assistant text: {text:?}");
}
