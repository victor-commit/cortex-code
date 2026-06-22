use axum::Json;

pub(crate) async fn openapi_json() -> Json<serde_json::Value> {
    Json(openapi_document())
}

/// Build the static OpenAPI document for the routes currently wired into the server router.
#[must_use]
pub(crate) fn openapi_document() -> serde_json::Value {
    let mut document = serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Cortex Code Local Server API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Loopback-only HTTP/SSE surface for local Cortex Code sessions."
        },
        "servers": [
            {
                "url": "http://127.0.0.1:{port}",
                "variables": {
                    "port": {
                        "default": "3200",
                        "description": "A free port in the reserved 3200-3299 range."
                    }
                }
            }
        ],
        "paths": {
            "/health": {
                "get": {
                    "operationId": "getHealth",
                    "summary": "Check server health.",
                    "responses": {
                        "200": json_response("Server is healthy.", "HealthResponse")
                    }
                }
            },
            "/openapi.json": {
                "get": {
                    "operationId": "getOpenApiDocument",
                    "summary": "Fetch this OpenAPI document.",
                    "responses": {
                        "200": {
                            "description": "OpenAPI document.",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "required": ["openapi", "info", "paths"]
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/event": {
                "get": {
                    "operationId": "streamEvents",
                    "summary": "Subscribe to typed server events as Server-Sent Events.",
                    "responses": {
                        "200": {
                            "description": "SSE stream of JSON event payloads.",
                            "content": {
                                "text/event-stream": {
                                    "schema": {
                                        "type": "string",
                                        "description": "SSE frames with JSON data payloads."
                                    }
                                }
                            }
                        }
                    }
                },
                "post": {
                    "operationId": "publishSurfaceEvent",
                    "summary": "Publish a typed event from an attached local surface onto the SSE stream.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "required": ["kind"],
                                    "description": "A typed Cortex event payload."
                                }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Event publication acknowledgement.", "EventPublishResponse"),
                        "400": json_response("Malformed event payload.", "ErrorResponse")
                    }
                }
            },
            "/session": {
                "get": {
                    "operationId": "listSessions",
                    "summary": "List persisted sessions.",
                    "responses": {
                        "200": {
                            "description": "Session list.",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "array",
                                        "items": { "$ref": "#/components/schemas/Session" }
                                    }
                                }
                            }
                        }
                    }
                },
                "post": {
                    "operationId": "createSession",
                    "summary": "Create a persisted session.",
                    "requestBody": {
                        "required": false,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/CreateSessionRequest" }
                            }
                        }
                    },
                    "responses": {
                        "201": json_response("Created session.", "Session"),
                        "400": json_response("Malformed session creation request.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "getSession",
                    "summary": "Fetch one persisted session.",
                    "responses": {
                        "200": json_response("Session found.", "Session"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                },
                "delete": {
                    "operationId": "deleteSession",
                    "summary": "Delete one persisted session.",
                    "responses": {
                        "204": { "description": "Session deleted." },
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/message": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "listSessionMessages",
                    "summary": "List messages and cumulative usage for one session.",
                    "responses": {
                        "200": json_response("Message list.", "MessageListResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                },
                "post": {
                    "operationId": "postSessionMessage",
                    "summary": "Append a prompt and run a real assistant turn.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/MessageRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Assistant turn result.", "PromptResponse"),
                        "400": json_response("Prompt body was empty or invalid.", "ErrorResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse"),
                        "503": json_response("Network is disabled or service is unavailable.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/prompt": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "getSessionPromptDraft",
                    "summary": "Read the pending remote prompt buffer without submitting it.",
                    "responses": {
                        "200": json_response("Pending prompt draft.", "PromptDraftResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/prompt/append": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "appendSessionPromptDraft",
                    "summary": "Append text to the pending remote prompt buffer without starting a turn.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/MessageRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Updated prompt draft.", "PromptDraftResponse"),
                        "400": json_response("Prompt append body was empty or invalid.", "ErrorResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/prompt/submit": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "submitSessionPromptDraft",
                    "summary": "Submit the pending remote prompt buffer as a real assistant turn.",
                    "responses": {
                        "200": json_response("Assistant turn result.", "PromptResponse"),
                        "400": json_response("Prompt buffer was empty.", "ErrorResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse"),
                        "503": json_response("Network is disabled or service is unavailable.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/prompt/clear": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "clearSessionPromptDraft",
                    "summary": "Discard the pending remote prompt buffer without starting a turn.",
                    "responses": {
                        "200": json_response("Prompt draft cleared.", "PromptDraftResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/command": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "executeSessionCommand",
                    "summary": "Run a verifiable shell command against the session context.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/CommandRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Command result.", "CommandResponse"),
                        "400": json_response("Command body was empty or invalid.", "ErrorResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/abort": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "abortSessionTurn",
                    "summary": "Request cancellation for an in-flight turn.",
                    "responses": {
                        "200": json_response("Abort request status.", "AbortResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/share": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "shareSessionLocal",
                    "summary": "Create a loopback-only local share for one session.",
                    "responses": {
                        "200": json_response("Local share reference.", "ShareResponse"),
                        "400": json_response("Host header was not loopback or port was outside 3200-3299.", "ErrorResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/unshare": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "unshareSessionLocal",
                    "summary": "Revoke all local shares for one session.",
                    "responses": {
                        "200": json_response("Local share revocation status.", "UnshareResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/share/{share_id}": {
                "parameters": [share_id_parameter()],
                "get": {
                    "operationId": "getLocalShare",
                    "summary": "Render a shared session over the loopback server.",
                    "responses": {
                        "200": {
                            "description": "HTML rendering of the shared session.",
                            "content": {
                                "text/html": {
                                    "schema": { "type": "string" }
                                }
                            }
                        },
                        "404": json_response("Share was not found or was revoked.", "ErrorResponse")
                    }
                }
            }
        },
        "components": {
            "schemas": {
                "AbortResponse": {
                    "type": "object",
                    "required": ["status", "active"],
                    "properties": {
                        "status": { "type": "string", "const": "abort_requested" },
                        "active": { "type": "boolean" }
                    }
                },
                "CreateSessionRequest": {
                    "type": "object",
                    "properties": {
                        "title": { "type": ["string", "null"] }
                    },
                    "additionalProperties": false
                },
                "CommandRequest": {
                    "type": "object",
                    "required": ["command"],
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute in the server's session working directory."
                        }
                    },
                    "additionalProperties": false
                },
                "CommandResponse": {
                    "type": "object",
                    "required": ["status", "command", "output", "is_error"],
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["completed", "error"]
                        },
                        "command": { "type": "string" },
                        "output": { "type": "string" },
                        "is_error": { "type": "boolean" }
                    }
                },
                "ErrorResponse": {
                    "type": "object",
                    "required": ["error"],
                    "properties": {
                        "error": { "type": "string" },
                        "kind": { "type": ["string", "null"] }
                    }
                },
                "EventPublishResponse": {
                    "type": "object",
                    "required": ["status", "kind"],
                    "properties": {
                        "status": { "type": "string", "const": "published" },
                        "kind": { "type": "string" }
                    }
                },
                "HealthResponse": {
                    "type": "object",
                    "required": ["status"],
                    "properties": {
                        "status": { "type": "string", "const": "ok" }
                    }
                },
                "Message": {
                    "type": "object",
                    "required": ["id", "session_id", "role", "content", "created_at"],
                    "properties": {
                        "id": { "type": "string" },
                        "session_id": { "type": "string" },
                        "role": { "type": "string" },
                        "content": { "type": "string" },
                        "created_at": { "type": "string" }
                    }
                },
                "MessageListResponse": {
                    "type": "object",
                    "required": ["session", "messages", "usage"],
                    "properties": {
                        "session": { "$ref": "#/components/schemas/Session" },
                        "messages": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/Message" }
                        },
                        "usage": { "$ref": "#/components/schemas/UsageTotals" }
                    }
                },
                "MessagePart": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string", "default": "text" },
                        "text": { "type": "string" }
                    }
                },
                "MessageRequest": {
                    "type": "object",
                    "properties": {
                        "model": {
                            "type": "string",
                            "description": "Optional per-turn model id in fully-qualified provider/model form, for example google/gemini-2.5-flash. When omitted, the session model is used, then the server default."
                        },
                        "text": { "type": "string" },
                        "content": { "type": "string" },
                        "message": { "type": "string" },
                        "prompt": { "type": "string" },
                        "parts": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/MessagePart" }
                        }
                    }
                },
                "PromptResponse": {
                    "type": "object",
                    "required": ["status", "model"],
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["completed", "aborted"]
                        },
                        "model": { "type": "string" },
                        "role": { "type": "string" },
                        "content": { "type": "string" },
                        "message": { "$ref": "#/components/schemas/Message" },
                        "usage": { "$ref": "#/components/schemas/Usage" },
                        "finish_reason": { "type": "string" }
                    }
                },
                "PromptDraftResponse": {
                    "type": "object",
                    "required": ["status", "text"],
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["draft", "updated", "cleared"]
                        },
                        "text": { "type": "string" }
                    }
                },
                "Session": {
                    "type": "object",
                    "required": ["id", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string" },
                        "title": { "type": ["string", "null"] },
                        "created_at": { "type": "string" },
                        "updated_at": { "type": "string" },
                        "model": { "type": ["string", "null"] }
                    }
                },
                "ShareResponse": {
                    "type": "object",
                    "required": ["status", "share_id", "url"],
                    "properties": {
                        "status": { "type": "string", "const": "shared" },
                        "share_id": {
                            "type": "string",
                            "description": "Ephemeral local share identifier."
                        },
                        "url": {
                            "type": "string",
                            "description": "Loopback-only URL for the shared session."
                        }
                    }
                },
                "UnshareResponse": {
                    "type": "object",
                    "required": ["status", "revoked"],
                    "properties": {
                        "status": { "type": "string", "const": "unshared" },
                        "revoked": {
                            "type": "boolean",
                            "description": "Whether at least one active local share was revoked."
                        }
                    }
                },
                "Usage": {
                    "type": "object",
                    "required": ["input_tokens", "output_tokens", "total_tokens"],
                    "properties": {
                        "input_tokens": { "type": "integer", "minimum": 0 },
                        "output_tokens": { "type": "integer", "minimum": 0 },
                        "total_tokens": { "type": "integer", "minimum": 0 },
                        "cost": { "type": ["number", "null"], "minimum": 0 }
                    }
                },
                "UsageTotals": {
                    "type": "object",
                    "required": ["input_tokens", "output_tokens", "total_tokens"],
                    "properties": {
                        "input_tokens": { "type": "integer", "minimum": 0 },
                        "output_tokens": { "type": "integer", "minimum": 0 },
                        "total_tokens": { "type": "integer", "minimum": 0 },
                        "cost": { "type": ["number", "null"], "minimum": 0 }
                    }
                }
            }
        }
    });
    add_auth_documentation(&mut document);
    document
}

const PROTECTED_OPERATIONS: &[(&str, &[&str])] = &[
    ("/event", &["get", "post"]),
    ("/session", &["get", "post"]),
    ("/session/{id}", &["get", "delete"]),
    ("/session/{id}/message", &["get", "post"]),
    ("/session/{id}/prompt", &["get"]),
    ("/session/{id}/prompt/append", &["post"]),
    ("/session/{id}/prompt/submit", &["post"]),
    ("/session/{id}/prompt/clear", &["post"]),
    ("/session/{id}/command", &["post"]),
    ("/session/{id}/abort", &["post"]),
    ("/session/{id}/share", &["post"]),
    ("/session/{id}/unshare", &["post"]),
    ("/share/{share_id}", &["get"]),
];

fn add_auth_documentation(document: &mut serde_json::Value) {
    if let Some(components) = document
        .get_mut("components")
        .and_then(|value| value.as_object_mut())
    {
        components.insert(
            "securitySchemes".to_owned(),
            serde_json::json!({
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "opaque",
                    "description": "When `cortex serve --token <TOKEN>` is configured, protected routes require `Authorization: Bearer <TOKEN>`."
                }
            }),
        );
    }

    for (path, methods) in PROTECTED_OPERATIONS {
        let Some(path_item) = document
            .get_mut("paths")
            .and_then(|value| value.as_object_mut())
            .and_then(|paths| paths.get_mut(*path))
            .and_then(|value| value.as_object_mut())
        else {
            continue;
        };
        for method in *methods {
            let Some(operation) = path_item
                .get_mut(*method)
                .and_then(|value| value.as_object_mut())
            else {
                continue;
            };
            operation
                .entry("security".to_owned())
                .or_insert_with(|| serde_json::json!([{ "bearerAuth": [] }]));
            let responses = operation
                .entry("responses".to_owned())
                .or_insert_with(|| serde_json::json!({}));
            if let Some(responses) = responses.as_object_mut() {
                responses
                    .entry("401".to_owned())
                    .or_insert_with(auth_response);
            }
        }
    }
}

fn auth_response() -> serde_json::Value {
    json_response(
        "Authentication is required when the server is started with --token.",
        "ErrorResponse",
    )
}

fn json_response(description: &str, schema: &str) -> serde_json::Value {
    serde_json::json!({
        "description": description,
        "content": {
            "application/json": {
                "schema": { "$ref": format!("#/components/schemas/{schema}") }
            }
        }
    })
}

fn session_id_parameter() -> serde_json::Value {
    serde_json::json!({
        "name": "id",
        "in": "path",
        "required": true,
        "schema": { "type": "string" },
        "description": "Opaque session id."
    })
}

fn share_id_parameter() -> serde_json::Value {
    serde_json::json!({
        "name": "share_id",
        "in": "path",
        "required": true,
        "schema": { "type": "string" },
        "description": "Ephemeral local share id."
    })
}
