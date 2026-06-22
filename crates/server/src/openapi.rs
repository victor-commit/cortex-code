use axum::Json;

pub(crate) async fn openapi_json() -> Json<serde_json::Value> {
    Json(openapi_document())
}

/// Build the static OpenAPI document for the routes currently wired into the server router.
#[must_use]
pub(crate) fn openapi_document() -> serde_json::Value {
    serde_json::json!({
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
                "ErrorResponse": {
                    "type": "object",
                    "required": ["error"],
                    "properties": {
                        "error": { "type": "string" },
                        "kind": { "type": ["string", "null"] }
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
    })
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
