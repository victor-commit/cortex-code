//! Session lifecycle and configuration JSON-RPC handlers for the ACP surface.
//!
//! These methods sit on [`AcpService`](crate::AcpService) and operate over the same durable SQLite
//! store as `cortex run`/`cortex serve`, so a session created here is listable, loadable, and
//! forkable across process restarts. Authentication injects a provider credential into the in-memory
//! override map consulted when a turn resolves its provider, so an ACP client can authenticate and
//! then prompt without a separate CLI auth step.

use std::io::Write;
use std::path::PathBuf;

use db::Database;
use protocol::MessagePart as RichMessagePart;
use serde_json::{Value, json};

use crate::{AcpService, JsonRpcWriter};

/// The built-in agent modes the ACP surface exposes, in stable order.
const MODES: [(&str, &str); 4] = [
    ("build", "Build"),
    ("plan", "Plan"),
    ("general", "General"),
    ("explore", "Explore"),
];

fn session_id_from_params(params: &Value) -> Option<String> {
    params
        .get("sessionId")
        .or_else(|| params.get("session_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn current_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn session_mode_state(current_mode: &str) -> Value {
    let available_modes = MODES
        .iter()
        .map(|(id, name)| json!({ "id": id, "name": name }))
        .collect::<Vec<_>>();
    json!({
        "currentModeId": current_mode,
        "availableModes": available_modes,
    })
}

impl AcpService {
    fn send_session_update<W: Write>(
        &self,
        writer: &JsonRpcWriter<W>,
        session_id: &str,
        update: Value,
    ) {
        let _ = writer.send_notification(
            "session/update",
            json!({ "sessionId": session_id, "update": update }),
        );
    }

    /// `authenticate`: store a provider credential supplied by the client so later prompts resolve
    /// without a separate CLI auth step. The key is never echoed back or logged.
    pub(crate) fn authenticate<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let method_id = params
            .get("methodId")
            .or_else(|| params.get("method_id"))
            .and_then(Value::as_str)
            .unwrap_or("api-key")
            .to_owned();
        let meta = params.get("_meta");
        let api_key = params
            .get("apiKey")
            .or_else(|| params.get("api_key"))
            .or_else(|| meta.and_then(|meta| meta.get("apiKey")))
            .or_else(|| meta.and_then(|meta| meta.get("api_key")))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let provider = params
            .get("provider")
            .or_else(|| meta.and_then(|meta| meta.get("provider")))
            .and_then(Value::as_str)
            .unwrap_or("openrouter")
            .to_owned();

        let Some(api_key) = api_key.filter(|key| !key.trim().is_empty()) else {
            return writer.send_error(
                id,
                -32602,
                format!(
                    "authenticate ({method_id}) requires an apiKey in params or _meta for provider {provider}"
                ),
            );
        };
        self.state
            .credentials
            .lock()
            .expect("credentials mutex")
            .insert(provider, api_key);
        writer.send_result(id, json!({}))
    }

    /// `session/load`: re-activate an existing session and replay its transcript as `session/update`
    /// notifications so the client reconstructs the conversation state.
    pub(crate) fn load_session<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let Some(session_id) = session_id_from_params(&params) else {
            return writer.send_error(id, -32602, "session/load params must include sessionId");
        };
        let db = match Database::open(&self.state.db_path) {
            Ok(db) => db,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        if let Err(err) = db.sessions().require(&session_id) {
            return writer.send_error(id, -32000, err.to_string());
        }
        let messages = match db.messages().list_for_session(&session_id) {
            Ok(messages) => messages,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        for message in &messages {
            let text = message_text(message);
            if text.trim().is_empty() {
                continue;
            }
            let kind = match message.role.as_str() {
                "user" => "user_message_chunk",
                _ => "agent_message_chunk",
            };
            self.send_session_update(
                &writer,
                &session_id,
                json!({
                    "sessionUpdate": kind,
                    "content": { "type": "text", "text": text },
                    "messageId": message.id,
                }),
            );
        }
        self.state
            .update_session_runtime(&session_id, |runtime| runtime.open = true);
        let mode = self
            .state
            .session_runtime(&session_id)
            .mode
            .unwrap_or_else(|| "build".to_owned());
        writer.send_result(id, json!({ "modes": session_mode_state(&mode) }))
    }

    /// `session/list`: enumerate durable sessions newest-first.
    pub(crate) fn list_sessions<W: Write>(
        &self,
        id: Value,
        _params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let db = match Database::open(&self.state.db_path) {
            Ok(db) => db,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        let sessions = match db.sessions().list() {
            Ok(sessions) => sessions,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        let cwd = current_cwd();
        let sessions = sessions
            .into_iter()
            .map(|session| {
                let mut entry = json!({
                    "sessionId": session.id,
                    "cwd": cwd.to_string_lossy(),
                    "updatedAt": session.updated_at,
                });
                if let Some(title) = session.title {
                    entry["title"] = json!(title);
                }
                entry
            })
            .collect::<Vec<_>>();
        writer.send_result(id, json!({ "sessions": sessions }))
    }

    /// `session/resume`: re-activate an existing session for prompting without replaying transcript.
    pub(crate) fn resume_session<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let Some(session_id) = session_id_from_params(&params) else {
            return writer.send_error(id, -32602, "session/resume params must include sessionId");
        };
        let db = match Database::open(&self.state.db_path) {
            Ok(db) => db,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        if let Err(err) = db.sessions().require(&session_id) {
            return writer.send_error(id, -32000, err.to_string());
        }
        self.state
            .update_session_runtime(&session_id, |runtime| runtime.open = true);
        let mode = self
            .state
            .session_runtime(&session_id)
            .mode
            .unwrap_or_else(|| "build".to_owned());
        writer.send_result(
            id,
            json!({ "sessionId": session_id, "modes": session_mode_state(&mode) }),
        )
    }

    /// `session/close`: cancel any in-flight turn and mark the session closed in this connection.
    pub(crate) fn close_session<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let Some(session_id) = session_id_from_params(&params) else {
            return writer.send_error(id, -32602, "session/close params must include sessionId");
        };
        let _ = self.state.active_turns.abort(&session_id);
        self.state
            .update_session_runtime(&session_id, |runtime| runtime.open = false);
        writer.send_result(id, json!({}))
    }

    /// `session/fork`: create a child session and copy the parent's transcript into it.
    pub(crate) fn fork_session<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let Some(session_id) = session_id_from_params(&params) else {
            return writer.send_error(id, -32602, "session/fork params must include sessionId");
        };
        let db = match Database::open(&self.state.db_path) {
            Ok(db) => db,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        let parent = match db.sessions().require(&session_id) {
            Ok(parent) => parent,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        let child = match db
            .sessions()
            .create_child(&parent.id, parent.title.as_deref())
        {
            Ok(child) => child,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        if let Some(model) = parent.model.as_deref()
            && let Err(err) = db.sessions().set_model(&child.id, Some(model))
        {
            return writer.send_error(id, -32000, err.to_string());
        }
        let messages = match db.messages().list_for_session(&session_id) {
            Ok(messages) => messages,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        for message in &messages {
            if let Err(err) = db.messages().create_with_parts(
                &child.id,
                &message.role,
                &message.content,
                &clone_parts(&message.parts),
            ) {
                return writer.send_error(id, -32000, err.to_string());
            }
        }
        writer.send_result(id, json!({ "sessionId": child.id }))
    }

    /// `session/delete`: remove a session and its messages from the durable store.
    pub(crate) fn delete_session<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let Some(session_id) = session_id_from_params(&params) else {
            return writer.send_error(id, -32602, "session/delete params must include sessionId");
        };
        let db = match Database::open(&self.state.db_path) {
            Ok(db) => db,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        if let Err(err) = db.sessions().delete(&session_id) {
            return writer.send_error(id, -32000, err.to_string());
        }
        self.state
            .sessions
            .lock()
            .expect("sessions mutex")
            .remove(&session_id);
        writer.send_result(id, json!({}))
    }

    /// `session/set_mode`: switch the active agent persona for later prompts and emit a
    /// `current_mode_update` notification.
    pub(crate) fn set_session_mode<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let Some(session_id) = session_id_from_params(&params) else {
            return writer.send_error(id, -32602, "session/set_mode params must include sessionId");
        };
        let Some(mode_id) = params
            .get("modeId")
            .or_else(|| params.get("mode_id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return writer.send_error(id, -32602, "session/set_mode params must include modeId");
        };
        if engine::Agent::from_name(&mode_id).is_err() {
            return writer.send_error(
                id,
                -32602,
                format!("unknown mode '{mode_id}': expected one of build, plan, general, explore"),
            );
        }
        self.state
            .update_session_runtime(&session_id, |runtime| runtime.mode = Some(mode_id.clone()));
        self.send_session_update(
            &writer,
            &session_id,
            json!({ "sessionUpdate": "current_mode_update", "currentModeId": mode_id }),
        );
        writer.send_result(id, json!({}))
    }

    /// `session/set_model`: persist the per-session model so later prompts use it, and emit a
    /// `config_option_update` naming the new model.
    pub(crate) fn set_session_model<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let Some(session_id) = session_id_from_params(&params) else {
            return writer.send_error(
                id,
                -32602,
                "session/set_model params must include sessionId",
            );
        };
        let Some(model) = params
            .get("model")
            .or_else(|| params.get("modelId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return writer.send_error(id, -32602, "session/set_model params must include model");
        };
        self.apply_model(id, &session_id, &model, writer)
    }

    /// `session/set_config_option`: apply a session configuration option (currently `model` and
    /// `mode`) and emit the matching update notification.
    pub(crate) fn set_session_config_option<W: Write>(
        &self,
        id: Value,
        params: Value,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        let Some(session_id) = session_id_from_params(&params) else {
            return writer.send_error(
                id,
                -32602,
                "session/set_config_option params must include sessionId",
            );
        };
        let Some(config_id) = params
            .get("configId")
            .or_else(|| params.get("config_id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return writer.send_error(
                id,
                -32602,
                "session/set_config_option params must include configId",
            );
        };
        let value = params
            .get("value")
            .map(|value| match value.get("value") {
                Some(inner) => inner.clone(),
                None => value.clone(),
            })
            .and_then(|value| value.as_str().map(str::to_owned));
        let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
            return writer.send_error(
                id,
                -32602,
                "session/set_config_option params must include a string value",
            );
        };
        match config_id.as_str() {
            "model" => self.apply_model(id, &session_id, &value, writer),
            "mode" => {
                if engine::Agent::from_name(&value).is_err() {
                    return writer.send_error(
                        id,
                        -32602,
                        format!(
                            "unknown mode '{value}': expected one of build, plan, general, explore"
                        ),
                    );
                }
                self.state.update_session_runtime(&session_id, |runtime| {
                    runtime.mode = Some(value.clone());
                });
                self.send_session_update(
                    &writer,
                    &session_id,
                    json!({ "sessionUpdate": "current_mode_update", "currentModeId": value }),
                );
                writer.send_result(id, json!({ "configOptions": [] }))
            }
            other => writer.send_error(
                id,
                -32602,
                format!("unknown configId '{other}': expected 'model' or 'mode'"),
            ),
        }
    }

    fn apply_model<W: Write>(
        &self,
        id: Value,
        session_id: &str,
        model: &str,
        writer: JsonRpcWriter<W>,
    ) -> std::io::Result<()> {
        if let Err(message) = providers::validate_model_id(model) {
            return writer.send_error(id, -32602, message);
        }
        let db = match Database::open(&self.state.db_path) {
            Ok(db) => db,
            Err(err) => return writer.send_error(id, -32000, err.to_string()),
        };
        if let Err(err) = db.sessions().set_model(session_id, Some(model)) {
            return writer.send_error(id, -32000, err.to_string());
        }
        self.state
            .update_session_runtime(session_id, |runtime| runtime.model = Some(model.to_owned()));
        let config_options = json!([
            {
                "id": "model",
                "name": "Model",
                "type": "select",
                "value": model,
                "options": [ { "value": model, "name": model } ],
            }
        ]);
        self.send_session_update(
            &writer,
            session_id,
            json!({ "sessionUpdate": "config_option_update", "configOptions": config_options }),
        );
        writer.send_result(id, json!({ "configOptions": config_options }))
    }
}

fn message_text(message: &db::Message) -> String {
    if !message.content.trim().is_empty() {
        return message.content.clone();
    }
    message
        .parts
        .iter()
        .filter_map(|part| match &part.body {
            protocol::MessagePartBody::Text { text }
            | protocol::MessagePartBody::Reasoning { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn clone_parts(parts: &[RichMessagePart]) -> Vec<RichMessagePart> {
    parts
        .iter()
        .map(|part| {
            let mut copy = part.clone();
            // Clear the stored part id so the child session mints fresh ids instead of colliding
            // with the parent's (the part-id column is unique).
            copy.id = String::new();
            copy.created_at = None;
            copy
        })
        .collect()
}
