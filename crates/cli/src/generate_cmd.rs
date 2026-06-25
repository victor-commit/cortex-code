//! `cortex generate`: author a custom markdown agent with a real model turn.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use errors::{Diagnostic, Exit};
use futures_util::StreamExt;
use llm::{LlmEvent, LlmRequest, Message, Provider};
use redaction::guard_outbound;
use serde::Serialize;
use settings::{ConfigError, ConfigInputs, EffectiveConfig};

use crate::cli::{GenerateArgs, OutputFormat};
use crate::custom_agents;

const OUTBOUND_OPERATION: &str = "agent generation turn";

/// Dispatch the generator command.
pub fn run(args: GenerateArgs) -> Exit {
    if !crate::feature_gate::network_enabled() {
        return crate::feature_gate::network_unavailable("generate");
    }

    if args.description.trim().is_empty() {
        eprintln!("cortex generate: description must not be empty");
        return Exit::Usage;
    }

    if let Err(diag) = guard_outbound(OUTBOUND_OPERATION) {
        eprintln!("cortex generate: {diag}");
        return diag.exit();
    }

    let inputs = match ConfigInputs::from_process(model_overrides(args.model.as_deref())) {
        Ok(inputs) => inputs,
        Err(err) => {
            eprintln!("cortex generate: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };
    let config = match resolve_generate_config(inputs.clone()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("cortex generate: {err}");
            return Exit::Config;
        }
    };
    if let Err(message) = validate_model_id(&config.model_id) {
        eprintln!("cortex generate: {message}");
        return Exit::Failure;
    }
    if let Err(err) = policy::enforce_provider_use(&inputs, &config.provider_id) {
        eprintln!("cortex generate: {err}");
        return if err.is_config() {
            Exit::Config
        } else {
            Exit::Failure
        };
    }

    let provider = match providers::resolve_openrouter_with_options(
        &providers::CredentialInputs::openrouter_from_process(config.api_key),
        config.base_url.as_deref(),
        None,
    ) {
        Ok(provider) => provider,
        Err(diag) => return report_setup_diagnostic(&diag),
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex generate: failed to start the async runtime: {err}");
            return Exit::Failure;
        }
    };

    let answer = match runtime.block_on(generate_agent_text(
        &provider,
        &config.model_id,
        &args.description,
    )) {
        Ok(answer) => answer,
        Err(exit) => return exit,
    };

    let authored = authored_agent_from_answer(&answer, &args.description, args.name.as_deref());
    let dir = args
        .agents_dir
        .as_deref()
        .map(resolve_dir)
        .unwrap_or_else(custom_agents::default_project_agents_dir);
    let path = unique_agent_path(&dir, &authored.name);

    let markdown = custom_agents::render_agent_markdown(
        &authored.name,
        &authored.description,
        "primary",
        Some(&config.model_id),
        &authored.system_prompt,
    );
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "cortex generate: failed to create agent directory {}: {err}",
            dir.display()
        );
        return Exit::Failure;
    }
    if let Err(err) = std::fs::write(&path, markdown) {
        eprintln!(
            "cortex generate: failed to write agent file {}: {err}",
            path.display()
        );
        return Exit::Failure;
    }

    emit_output(args.format.format, &authored, &path, &config.model_id)
}

struct GenerateConfig {
    model_id: String,
    provider_id: String,
    base_url: Option<String>,
    api_key: Option<String>,
}

fn resolve_generate_config(inputs: ConfigInputs) -> Result<GenerateConfig, ConfigError> {
    let effective = settings::load(inputs)?;
    let resolved = providers::resolve_model(Some(effective.model()));
    Ok(GenerateConfig {
        model_id: resolved.model_id,
        provider_id: resolved.provider_id,
        base_url: openrouter_base_url_override(&effective),
        api_key: openrouter_api_key(&effective),
    })
}

async fn generate_agent_text(
    provider: &dyn Provider,
    model_id: &str,
    description: &str,
) -> Result<String, Exit> {
    let system = "You author Cortex Code custom markdown agent definitions. Return only one JSON \
object with keys: name, description, system_prompt. The name must be a short lowercase identifier \
using letters, numbers, '-' or '_'. The system_prompt must be directly usable as a coding agent \
system prompt.";
    let user =
        format!("Create a custom agent for this request:\n\n{description}\n\nReturn only JSON.");
    let request = LlmRequest::new(model_id)
        .message(Message::system(system))
        .message(Message::user(user))
        .temperature(0.2)
        .max_tokens(900);

    let mut stream = match provider.stream(request).await {
        Ok(stream) => stream,
        Err(err) => {
            let diag = err.to_diagnostic();
            eprintln!("cortex generate: {diag}");
            return Err(diag.exit());
        }
    };
    let mut text = String::new();
    let mut error = None;
    while let Some(event) = stream.next().await {
        match event {
            LlmEvent::TextDelta { text: delta } => text.push_str(&delta),
            LlmEvent::Error { message, kind } => {
                error = Some(match kind.as_str() {
                    "transport-unavailable" => Diagnostic::transport_unavailable(message),
                    "net-disabled" => Diagnostic::net_disabled(OUTBOUND_OPERATION),
                    "no-credential" => Diagnostic::no_credential(providers::OPENROUTER_ID),
                    _ => Diagnostic::upstream(message),
                });
            }
            LlmEvent::Done { .. } => break,
            LlmEvent::Reasoning { .. }
            | LlmEvent::ToolCall(_)
            | LlmEvent::ToolResult(_)
            | LlmEvent::Usage(_) => {}
        }
    }
    if let Some(diag) = error {
        eprintln!("cortex generate: {diag}");
        return Err(diag.exit());
    }
    if text.trim().is_empty() {
        eprintln!("cortex generate: provider returned an empty generation");
        return Err(Exit::Failure);
    }
    Ok(text)
}

#[derive(Debug, Clone)]
struct AuthoredAgent {
    name: String,
    description: String,
    system_prompt: String,
}

fn authored_agent_from_answer(
    answer: &str,
    requested_description: &str,
    requested_name: Option<&str>,
) -> AuthoredAgent {
    let parsed = extract_json_object(answer).map(|json| {
        let name = json
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(custom_agents::sanitize_identifier);
        let description = json
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let system_prompt = json
            .get("system_prompt")
            .or_else(|| json.get("prompt"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        (name, description, system_prompt)
    });

    let requested_name = requested_name.map(custom_agents::sanitize_identifier);
    let fallback_name = custom_agents::sanitize_identifier(requested_description);
    let (model_name, model_description, model_prompt) = parsed.unwrap_or((None, None, None));
    let name = requested_name
        .or(model_name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(fallback_name);
    let description = model_description.unwrap_or_else(|| requested_description.trim().to_owned());
    let system_prompt = model_prompt.unwrap_or_else(|| {
        format!(
            "You are the {name} custom agent. Follow this role description:\n\n{}\n\nGenerator output:\n{}",
            requested_description.trim(),
            answer.trim()
        )
    });

    AuthoredAgent {
        name,
        description,
        system_prompt,
    }
}

fn extract_json_object(answer: &str) -> Option<serde_json::Value> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&answer[start..=end]).ok()
}

fn unique_agent_path(dir: &Path, name: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{name}.md"));
    if !candidate.exists() {
        return candidate;
    }
    for index in 2..1000 {
        candidate = dir.join(format!("{name}-{index}.md"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{name}-{}.md", std::process::id()))
}

#[derive(Serialize)]
struct GenerateDocument<'a> {
    name: &'a str,
    description: &'a str,
    path: &'a str,
    model: &'a str,
}

fn emit_output(
    format: OutputFormat,
    authored: &AuthoredAgent,
    path: &Path,
    model_id: &str,
) -> Exit {
    let path_string = path.display().to_string();
    let body = match format {
        OutputFormat::Text => format!("generated agent {} at {}\n", authored.name, path.display()),
        OutputFormat::Json | OutputFormat::Jsonl => match serde_json::to_string(&GenerateDocument {
            name: &authored.name,
            description: &authored.description,
            path: &path_string,
            model: model_id,
        }) {
            Ok(mut json) => {
                json.push('\n');
                json
            }
            Err(err) => {
                eprintln!("cortex generate: failed to serialize output: {err}");
                return Exit::Failure;
            }
        },
    };
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

fn resolve_dir(value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn model_overrides(cli_model: Option<&str>) -> BTreeMap<String, String> {
    let mut overrides = BTreeMap::new();
    if let Some(model) = cli_model {
        overrides.insert("model".to_owned(), model.to_owned());
    }
    overrides
}

fn validate_model_id(model_id: &str) -> Result<(), String> {
    providers::validate_model_id(model_id)
}

fn openrouter_base_url_override(effective: &EffectiveConfig) -> Option<String> {
    top_level_string(effective, "openrouter_base_url")
        .or_else(|| provider_openrouter_string(effective, "base_url"))
}

fn openrouter_api_key(effective: &EffectiveConfig) -> Option<String> {
    provider_openrouter_string(effective, "api_key")
        .or_else(|| top_level_string(effective, "api_key"))
}

fn top_level_string(effective: &EffectiveConfig, key: &str) -> Option<String> {
    effective
        .get(key)
        .and_then(|value| value.as_str())
        .and_then(non_empty)
}

fn provider_openrouter_string(effective: &EffectiveConfig, key: &str) -> Option<String> {
    effective
        .get("provider")
        .and_then(|provider| provider.get("openrouter"))
        .and_then(|openrouter| openrouter.get(key))
        .and_then(|value| value.as_str())
        .and_then(non_empty)
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn report_setup_diagnostic(diag: &Diagnostic) -> Exit {
    eprintln!("cortex generate: {diag}");
    if let Diagnostic::NoCredential(provider) = diag {
        if provider == providers::OPENROUTER_ID {
            eprintln!(
                "cortex generate: set the {} environment variable (or store a key in the \
                 credential file or config) to authenticate with OpenRouter",
                providers::OPENROUTER_ENV
            );
        }
    }
    diag.exit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_model_json_from_markdown_wrapped_response() {
        let authored = authored_agent_from_answer(
            "```json\n{\"name\":\"test-writer\",\"description\":\"Writes tests\",\"system_prompt\":\"Write focused tests.\"}\n```",
            "an agent that writes tests",
            None,
        );
        assert_eq!(authored.name, "test-writer");
        assert_eq!(authored.description, "Writes tests");
        assert_eq!(authored.system_prompt, "Write focused tests.");
    }

    #[test]
    fn requested_name_overrides_generated_name() {
        let authored = authored_agent_from_answer(
            "{\"name\":\"other\",\"description\":\"Desc\",\"system_prompt\":\"Prompt\"}",
            "anything",
            Some("Unit Tester!"),
        );
        assert_eq!(authored.name, "unit-tester");
    }

    #[test]
    fn non_json_answer_still_uses_real_answer_in_prompt_body() {
        let authored = authored_agent_from_answer(
            "Use a careful testing persona.",
            "an agent that writes tests",
            None,
        );
        assert_eq!(authored.name, "an-agent-that-writes-tests");
        assert!(
            authored
                .system_prompt
                .contains("Use a careful testing persona.")
        );
    }
}
