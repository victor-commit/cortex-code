//! Live runner test: a real streamed `google/gemini-2.5-flash` turn driven through the durable
//! [`SessionRunner`] against OpenRouter, with no mocks.
//!
//! This is the end-to-end proof for the runner: the credential is resolved through the real
//! `providers` precedence path, the OpenRouter provider streams a real turn, and the runner
//! assembles the normalized event vocabulary. It is gated on `OPENROUTER_API_KEY` and skips cleanly
//! when the key is absent (so the offline gate stays green); mission validation runs it with the
//! key exported: `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage mirrors the fulfilled assertions:
//! - VAL-RUN-002: a "think step by step, then FINAL:" prompt yields a final answer containing
//!   `FINAL:` and a single terminal completion event.
//! - VAL-RUN-007: concatenated text-delta events equal the final answer; usage and a single
//!   terminal `Done` follow the text, with no duplicate terminal event.
//! - VAL-LLM-033: a dynamic environment block naming a unique scratch cwd and the Linux platform
//!   lets the model answer the exact cwd and platform correctly.

use engine::{EnvironmentBlock, GitStatus, RunnerEvent, SessionRunner, SystemContext, Turn};
use llm::Provider;
use providers::{CredentialInputs, resolve_openrouter};

const MODEL: &str = "google/gemini-2.5-flash";

/// Resolve the OpenRouter provider from the env key, or `None` to skip when unset/empty.
fn provider_or_skip() -> Option<impl Provider> {
    let api_key = std::env::var("OPENROUTER_API_KEY").ok()?;
    if api_key.trim().is_empty() {
        eprintln!("skipping live runner test: OPENROUTER_API_KEY is empty");
        return None;
    }
    let inputs = CredentialInputs {
        env_value: Some(api_key),
        ..Default::default()
    };
    Some(resolve_openrouter(&inputs).expect("provider builds from resolved credential"))
}

/// A unique scratch-directory path for the environment block (the dir need not exist on disk; the
/// block is text the model reads).
fn unique_scratch_dir() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("/tmp/cortex-env-{}-{nanos}", std::process::id())
}

fn concat_text(events: &[RunnerEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            RunnerEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn live_multi_step_prompt_reaches_a_single_final_answer() {
    let Some(provider) = provider_or_skip() else {
        eprintln!("skipping live runner test: OPENROUTER_API_KEY not set");
        return;
    };

    let environment = EnvironmentBlock::from_process();
    let system = SystemContext::for_model(MODEL, environment).render();
    let turn = Turn::new(
        MODEL,
        system,
        "Think step by step about why the sky appears blue, then give a final answer on its own \
         line labelled exactly 'FINAL:'.",
    )
    .with_max_tokens(400);

    let mut events = Vec::new();
    let outcome = SessionRunner::default()
        .run(&provider, turn, |event| events.push(event))
        .await;

    assert!(
        outcome.is_success(),
        "live turn must not error: {:?}",
        outcome.error
    );

    // VAL-RUN-002: the turn reaches one coherent final answer containing FINAL:.
    assert!(
        outcome.answer().contains("FINAL:"),
        "final answer must contain FINAL:, got: {:?}",
        outcome.answer()
    );

    // VAL-RUN-007: concatenated text deltas equal the assembled final answer.
    assert_eq!(
        concat_text(&events),
        outcome.answer(),
        "concatenated text deltas must equal the final answer"
    );

    // Exactly one terminal Done, and it is the last event; usage (if any) precedes it.
    let done_positions: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, RunnerEvent::Done { .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        done_positions,
        vec![events.len() - 1],
        "one terminal Done, last"
    );
    if let Some(usage_pos) = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Usage(_)))
    {
        assert!(
            usage_pos < events.len() - 1,
            "usage must arrive before the terminal Done"
        );
    }

    // Real usage was reported by the priced model.
    let usage = outcome.usage.as_ref().expect("usage reported");
    assert!(usage.output_tokens > 0, "output tokens should be positive");

    eprintln!("live FINAL answer: {:?}", outcome.answer());
}

#[tokio::test]
async fn live_environment_block_answers_exact_cwd_and_platform() {
    let Some(provider) = provider_or_skip() else {
        eprintln!("skipping live runner test: OPENROUTER_API_KEY not set");
        return;
    };

    let scratch = unique_scratch_dir();
    let environment = EnvironmentBlock {
        cwd: scratch.clone(),
        platform: "Linux".to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        date: "2026-06-19".to_owned(),
        git: GitStatus::NotARepo,
    };
    let system = SystemContext::for_model(MODEL, environment).render();
    let turn = Turn::new(
        MODEL,
        system,
        "Using only the environment context, reply with exactly one line in this format: \
         'cwd=<current working directory> platform=<platform>'.",
    )
    .with_max_tokens(120);

    let outcome = SessionRunner::default().run(&provider, turn, |_| {}).await;

    assert!(
        outcome.is_success(),
        "live turn must not error: {:?}",
        outcome.error
    );
    let answer = outcome.answer();
    assert!(
        answer.contains(&scratch),
        "answer must state the exact injected cwd {scratch:?}, got: {answer:?}"
    );
    assert!(
        answer.contains("Linux"),
        "answer must state the Linux platform, got: {answer:?}"
    );

    eprintln!("live env answer: {answer:?}");
}
