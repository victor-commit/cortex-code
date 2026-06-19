//! Offline cassette tests: deterministic SSE -> `LlmEvent` normalization with no network.
//!
//! These feed recorded OpenAI-compatible Chat SSE byte streams (fixtures live in `tests/fixtures/`,
//! tests only) through the EXACT same code path the live transport uses — `sse_data_frames`
//! (eventsource framing + idle timeout) then `normalize_chat_sse` — and assert the resulting ordered
//! `LlmEvent` sequence. Bytes are delivered in small, frame-splitting chunks to prove the parser
//! buffers across chunk boundaries.

use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use llm::{
    ByteStream, FinishReason, FrameStream, LlmError, LlmEvent, normalize_chat_sse, sse_data_frames,
};

/// Load a fixture and present it as a `ByteStream` split into `chunk_size`-byte pieces so frames
/// straddle chunk boundaries (the realistic streaming case).
fn cassette_stream(name: &str, chunk_size: usize) -> ByteStream {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("read fixture {path}: {err}"));
    let chunks: Vec<Result<Bytes, LlmError>> = bytes
        .chunks(chunk_size)
        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
        .collect();
    Box::pin(futures_util::stream::iter(chunks))
}

async fn normalize_cassette(name: &str, chunk_size: usize) -> Vec<LlmEvent> {
    let bytes = cassette_stream(name, chunk_size);
    let frames: FrameStream = Box::pin(sse_data_frames(bytes, Duration::from_secs(5)));
    normalize_chat_sse(frames).collect().await
}

#[tokio::test]
async fn text_cassette_normalizes_to_ordered_events_ending_in_done() {
    let events = normalize_cassette("chat_text.sse", 7).await;

    // Concatenated TextDelta payloads reconstruct the assistant message.
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            LlmEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello, world!");

    // Usage is reported and precedes the single terminal Done.
    let usage = events
        .iter()
        .find_map(|e| match e {
            LlmEvent::Usage(usage) => Some(usage),
            _ => None,
        })
        .expect("a usage event");
    assert_eq!(usage.input_tokens, 9);
    assert_eq!(usage.output_tokens, 3);
    assert_eq!(usage.total_tokens, 12);
    assert_eq!(usage.cost, Some(0.0000056));

    assert_eq!(
        events.last(),
        Some(&LlmEvent::Done {
            reason: FinishReason::Stop
        })
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, LlmEvent::Done { .. }))
            .count(),
        1,
        "exactly one terminal Done"
    );
}

#[tokio::test]
async fn text_cassette_is_deterministic_across_chunkings() {
    // The same recorded stream must normalize identically regardless of how the bytes are chunked.
    let one_byte = normalize_cassette("chat_text.sse", 1).await;
    let whole = normalize_cassette("chat_text.sse", usize::MAX).await;
    assert_eq!(one_byte, whole);
}

#[tokio::test]
async fn tool_call_cassette_assembles_complete_arguments() {
    let events = normalize_cassette("chat_tool_call.sse", 5).await;

    let tool_call = events
        .iter()
        .find_map(|e| match e {
            LlmEvent::ToolCall(call) => Some(call),
            _ => None,
        })
        .expect("an assembled tool call");
    assert_eq!(tool_call.id, "call_abc");
    assert_eq!(tool_call.name, "get_weather");
    // The fragments `{"location":`, ` "Paris"}` assemble into one complete JSON arguments string.
    assert_eq!(tool_call.arguments, r#"{"location": "Paris"}"#);
    // And the assembled arguments are valid JSON.
    let parsed: serde_json::Value = serde_json::from_str(&tool_call.arguments).expect("valid json");
    assert_eq!(parsed["location"], "Paris");

    assert_eq!(
        events.last(),
        Some(&LlmEvent::Done {
            reason: FinishReason::ToolCalls
        })
    );
}
