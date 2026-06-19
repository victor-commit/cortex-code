//! Live OpenRouter smoke test: a resolved OpenRouter credential drives a real streamed turn.
//!
//! This proves the shipped wiring end-to-end with no mocks: the credential is resolved through the
//! real precedence path (env layer here), the OpenRouter base URL is resolved from the catalogue,
//! and the resulting provider opens a real streamed turn against `google/gemini-2.5-flash`. It is
//! gated on `OPENROUTER_API_KEY` and skips cleanly when the key is absent (so the offline gate stays
//! green); mission validation runs it with the key exported:
//! `set -a; . /projects/cortex-code/.env.local; set +a`.

use futures_util::StreamExt;
use llm::{LlmEvent, LlmRequest, Provider};
use providers::{
    CredentialInputs, CredentialSource, OPENROUTER_DEFAULT_MODEL, OPENROUTER_ID,
    resolve_credential, resolve_model, resolve_openrouter,
};

#[tokio::test]
async fn resolved_openrouter_credential_drives_a_real_gemini_turn() {
    let Ok(api_key) = std::env::var("OPENROUTER_API_KEY") else {
        eprintln!("skipping live smoke test: OPENROUTER_API_KEY not set");
        return;
    };
    if api_key.trim().is_empty() {
        eprintln!("skipping live smoke test: OPENROUTER_API_KEY is empty");
        return;
    }

    // Resolve through the real credential store path (env layer wins).
    let inputs = CredentialInputs {
        env_value: Some(api_key),
        ..Default::default()
    };
    let credential = resolve_credential(OPENROUTER_ID, &inputs).expect("credential resolves");
    assert_eq!(credential.source(), CredentialSource::Env);

    // The resolved model is the documented default served by OpenRouter.
    let resolved = resolve_model(None);
    assert_eq!(resolved.provider_id, OPENROUTER_ID);
    assert_eq!(resolved.model_id, OPENROUTER_DEFAULT_MODEL);

    let provider = resolve_openrouter(&inputs).expect("provider builds from resolved credential");

    let request = LlmRequest::new(&resolved.model_id)
        .system("You are a terse assistant. Reply with a single short word.")
        .user("Reply with exactly the word: pong")
        .max_tokens(32);

    let stream = provider.stream(request).await.expect("live stream opens");
    let events: Vec<LlmEvent> = stream.collect().await;

    let text: String = events
        .iter()
        .filter_map(|event| match event {
            LlmEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        !text.trim().is_empty(),
        "assistant text should be non-empty, got events: {events:?}"
    );
    assert!(
        matches!(events.last(), Some(LlmEvent::Done { .. })),
        "stream must end in a terminal Done, got: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, LlmEvent::Error { .. })),
        "healthy turn must not emit an Error event: {events:?}"
    );

    eprintln!("live OpenRouter turn assistant text: {text:?}");
}
