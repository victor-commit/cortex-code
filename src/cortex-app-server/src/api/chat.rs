//! Chat completions endpoint (OpenAI-compatible).

use std::sync::Arc;

use axum::{Json, extract::State};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

use super::types::{ChatCompletionRequest, ChatCompletionResponse};

/// Handle chat completions by calling the actual LLM provider.
/// This HTTP surface is kept for compatibility, but no app-server provider is wired.
pub async fn chat_completions(
    State(_state): State<Arc<AppState>>,
    Json(_req): Json<ChatCompletionRequest>,
) -> AppResult<Json<ChatCompletionResponse>> {
    Err(AppError::NotImplemented(
        "Chat completions are unsupported in cortex-app-server because no provider runtime is wired; use the TUI for LLM interactions".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ChatMessage;
    use crate::config::ServerConfig;

    #[tokio::test]
    async fn test_chat_completions_returns_not_implemented() {
        let state = Arc::new(AppState::new(ServerConfig::default()).await.unwrap());
        let request = ChatCompletionRequest {
            model: "cortex".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            tools: vec![],
        };

        let error = chat_completions(State(state), Json(request))
            .await
            .expect_err("chat completions should be unsupported");

        assert!(matches!(error, AppError::NotImplemented(_)));
        assert_eq!(error.status_code(), axum::http::StatusCode::NOT_IMPLEMENTED);
        assert_eq!(error.error_code(), "not_implemented");
        assert!(
            error
                .to_string()
                .contains("Chat completions are unsupported")
        );
    }
}
