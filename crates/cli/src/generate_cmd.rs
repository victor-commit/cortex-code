//! `cortex generate`: author a custom markdown agent with a real model turn.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use errors::{Diagnostic, Exit};
use futures_util::StreamExt;
use llm::{LlmEvent, LlmRequest, Message, Provider};
use redaction::guard_outbound;
use serde::Serialize;
use settings::{ConfigError, ConfigInputs};

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

    let provider = match providers::resolve_runtime_provider_with_options(
        &config.provider_config,
        &config.provider_id,
        &config.model_id,
        None,
    ) {
        Ok(provider) => provider.provider,
        Err(error) => {
            if let Some(config_error) = error.as_config() {
                eprintln!("cortex generate: {config_error}");
                return Exit::Config;
            }
            if let Some(diagnostic) = error.as_diagnostic() {
                return report_setup_diagnostic(diagnostic);
            }
            unreachable!("runtime provider error must be config or diagnostic");
        }
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
        provider.as_ref(),
        &config.model_id,
        &args.description,
        &config.request_params,
    )) {
        Ok(answer) => answer,
        Err(exit) => return exit,
    };

    let authored = authored_agent_from_answer(&answer, &args.description, args.name.as_deref());
    let dir = args
        .agents_dir
        .as_deref()
        .map(|value| resolve_dir(&inputs.cwd, value))
        .unwrap_or_else(|| custom_agents::default_project_agents_dir(&inputs.cwd));
    if let Err(err) = custom_agents::preflight_generation(&inputs, &dir, &authored.name) {
        eprintln!("cortex generate: {err}");
        return Exit::Failure;
    }

    let markdown = custom_agents::render_agent_markdown(
        &authored.name,
        &authored.description,
        "primary",
        Some(&config.model_id),
        &authored.system_prompt,
    );
    let Some(lock_dir) = custom_agents::generation_lock_dir(&inputs) else {
        eprintln!(
            "cortex generate: cannot determine a Cortex state location for generated-agent leases; \
             set HOME, XDG_DATA_HOME, CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Exit::Failure;
    };
    if let Err(err) = std::fs::create_dir_all(&lock_dir) {
        eprintln!(
            "cortex generate: failed to create generation lock directory {}: {err}",
            lock_dir.display()
        );
        return Exit::Failure;
    }

    let _lease = match custom_agents::GenerationLease::acquire(&lock_dir, &authored.name) {
        Ok(lease) => lease,
        Err(err) => {
            eprintln!("cortex generate: {err}");
            return Exit::Failure;
        }
    };
    if let Err(err) = custom_agents::preflight_generation(&inputs, &dir, &authored.name) {
        eprintln!("cortex generate: {err}");
        return Exit::Failure;
    }
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "cortex generate: failed to create agent directory {}: {err}",
            dir.display()
        );
        return Exit::Failure;
    }

    let path = dir.join(format!("{}.md", authored.name));
    if let Err(err) = write_new_agent_file(&path, &markdown) {
        eprintln!(
            "cortex generate: failed to write agent file {}: {err}",
            path.display()
        );
        return Exit::Failure;
    }

    let discovery = custom_agents::discover(&inputs);
    if !custom_agents::generated_agent_is_discoverable(&discovery, &authored.name, &path) {
        let _ = std::fs::remove_file(&path);
        eprintln!(
            "cortex generate: generated agent '{}' was not immediately discoverable; no agent was created",
            authored.name
        );
        return Exit::Failure;
    }

    emit_output(args.format.format, &authored, &path, &config.model_id)
}

struct GenerateConfig {
    model_id: String,
    provider_id: String,
    provider_config: providers::ProviderConfigSet,
    request_params: serde_json::Map<String, serde_json::Value>,
}

fn resolve_generate_config(inputs: ConfigInputs) -> Result<GenerateConfig, ConfigError> {
    let effective = settings::load(inputs)?;
    let provider_config =
        providers::ProviderConfigSet::from_table(effective.table()).map_err(|error| {
            ConfigError::TypeMismatch {
                key: error.key,
                expected: "a valid provider configuration",
                found: error.message,
            }
        })?;
    let resolved = provider_config
        .resolve_model(providers::Catalogue::embedded(), Some(effective.model()))
        .map_err(|error| ConfigError::TypeMismatch {
            key: error.key,
            expected: "a valid provider model mapping",
            found: error.message,
        })?;
    Ok(GenerateConfig {
        request_params: provider_config.provider_params(&resolved.provider_id),
        model_id: resolved.model_id,
        provider_id: resolved.provider_id.clone(),
        provider_config,
    })
}

async fn generate_agent_text(
    provider: &dyn Provider,
    model_id: &str,
    description: &str,
    request_params: &serde_json::Map<String, serde_json::Value>,
) -> Result<String, Exit> {
    let system = "You author Cortex Code custom markdown agent definitions. Return only one JSON \
object with keys: name, description, system_prompt. The name must be a short lowercase identifier \
using letters, numbers, '-' or '_'. The system_prompt must be directly usable as a coding agent \
system prompt.";
    let user =
        format!("Create a custom agent for this request:\n\n{description}\n\nReturn only JSON.");
    let mut request = LlmRequest::new(model_id)
        .message(Message::system(system))
        .message(Message::user(user))
        .temperature(0.2)
        .max_tokens(900);
    request.extra_params = request_params.clone();

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

fn write_new_agent_file(path: &Path, markdown: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let result = file
        .write_all(markdown.as_bytes())
        .and_then(|()| file.sync_all());
    if result.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    result
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

fn resolve_dir(cwd: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
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
