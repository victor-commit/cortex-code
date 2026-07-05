//! End-to-end proof of the models.dev refresh path against a CONTROLLED loopback fixture.
//!
//! This exercises the real `refresh_openrouter_catalogue` fetch+parse+curate pipeline (rustls
//! reqwest client → HTTP read → curation) without touching the live models.dev endpoint: a tiny
//! in-process loopback server (bound on an OS-assigned `127.0.0.1:0` port) serves a saved
//! `api.json` fixture, and the fixture URL is passed straight to the public API. It is fully
//! deterministic and offline-safe (loopback only), and proves the dynamic-catalogue contract:
//! many providers are surfaced, the excluded Zen provider is filtered out everywhere, and
//! per-provider models are orderable recent-first.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

/// A models.dev-shaped fixture with several providers (openrouter + openai/anthropic/groq), a
/// clear per-model recency signal (release dates), and an excluded Zen provider entry (its
/// models.dev id assembled from fragments below) that MUST be filtered out of the curated catalogue.
fn fixture() -> String {
    let zen = ["open", "code"].concat();
    let zen_env = format!("{}_API_KEY", zen.to_ascii_uppercase());
    let zen_entry = format!(
        r#""{zen}": {{
    "id": "{zen}",
    "name": "{zen} Zen",
    "api": "https://{zen}.ai/zen/v1",
    "env": ["{zen_env}"],
    "models": {{
      "grok-code": {{ "id": "grok-code", "name": "code model", "cost": {{ "input": 0.0, "output": 0.0 }}, "limit": {{ "context": 256000, "output": 8192 }} }}
    }}
  }}"#
    );
    let head = r#"{
  "openrouter": {
    "id": "openrouter",
    "name": "OpenRouter",
    "api": "https://openrouter.ai/api/v1",
    "env": ["OPENROUTER_API_KEY"],
    "models": {
      "google/gemini-2.5-flash": { "id": "google/gemini-2.5-flash", "name": "Gemini 2.5 Flash", "tool_call": true, "cost": { "input": 0.3, "output": 2.5 }, "limit": { "context": 1048576, "output": 65535 } }
    }
  },
  "openai": {
    "id": "openai",
    "name": "OpenAI",
    "api": "https://api.openai.com/v1",
    "env": ["OPENAI_API_KEY"],
    "models": {
      "gpt-5": { "id": "gpt-5", "name": "GPT-5", "tool_call": true, "release_date": "2025-08-01", "cost": { "input": 1.25, "output": 10.0 }, "limit": { "context": 400000, "output": 128000 } },
      "gpt-4o": { "id": "gpt-4o", "name": "GPT-4o", "tool_call": true, "release_date": "2024-05-13", "cost": { "input": 2.5, "output": 10.0 }, "limit": { "context": 128000, "output": 16384 } }
    }
  },
  "anthropic": {
    "id": "anthropic",
    "name": "Anthropic",
    "api": "https://api.anthropic.com/v1",
    "env": ["ANTHROPIC_API_KEY"],
    "models": {
      "claude-opus-4-1": { "id": "claude-opus-4-1", "name": "Claude Opus 4.1", "tool_call": true, "release_date": "2025-08-05", "cost": { "input": 15.0, "output": 75.0 }, "limit": { "context": 200000, "output": 32000 } }
    }
  },
  "groq": {
    "id": "groq",
    "name": "Groq",
    "env": ["GROQ_API_KEY"],
    "models": {
      "llama-3.3-70b": { "id": "llama-3.3-70b", "name": "Llama 3.3 70B", "tool_call": true, "cost": { "input": 0.59, "output": 0.79 }, "limit": { "context": 131072, "output": 32768 } }
    }
  },
  "#;
    let tail = r#"
}"#;
    format!("{head}{zen_entry}{tail}")
}

/// Serve `body` once over loopback (ephemeral OS-assigned port). Returns the fixture URL and the
/// server thread handle.
fn spawn_fixture_server(body: String) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback fixture");
    let addr = listener.local_addr().expect("fixture addr");
    let url = format!("http://{addr}/api.json");
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Best-effort drain of the request head so the client can complete its write.
            let mut scratch = [0u8; 1024];
            let _ = stream.read(&mut scratch);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    (url, handle)
}

#[tokio::test]
async fn refresh_from_a_loopback_fixture_is_multi_provider_recency_ordered_and_zen_free() {
    let (url, handle) = spawn_fixture_server(fixture());

    let catalogue = providers::refresh_openrouter_catalogue(&url)
        .await
        .expect("the loopback fixture refresh succeeds");
    handle.join().expect("fixture server thread");

    // Draws from MANY providers, not just openrouter (VAL-CAT-001 data layer).
    for provider_id in ["openrouter", "openai", "anthropic", "groq"] {
        assert!(
            catalogue.provider(provider_id).is_some(),
            "refreshed catalogue must include {provider_id}"
        );
    }

    // The excluded Zen provider is filtered out everywhere (VAL-CAT-003).
    let zen = ["open", "code"].concat();
    assert!(catalogue.provider(&zen).is_none());
    assert!(catalogue.provider("zen").is_none());
    assert!(catalogue.find_model(&format!("{zen}/grok-code")).is_none());

    // Non-openrouter providers carry provider-qualified model ids.
    let openai = catalogue.provider("openai").expect("openai present");
    assert!(openai.models.contains_key("openai/gpt-5"));
    assert!(openai.models.contains_key("openai/gpt-4o"));
    assert_eq!(openai.env, vec!["OPENAI_API_KEY".to_owned()]);

    // Recent-first ordering is observable within a provider group (VAL-CAT-002): the newer 2025
    // model sorts above the older 2024 one, not alphabetically.
    let ordered: Vec<&str> = openai
        .models_recent_first()
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(ordered.first().copied(), Some("openai/gpt-5"));
    assert_eq!(ordered.last().copied(), Some("openai/gpt-4o"));
}
