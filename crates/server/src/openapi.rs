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
        "x-cortex-compatibility": {
            "decision": "wire-compatible",
            "recorded_backlog_item": "gap-server-wire-compatibility-decision",
            "route_contract": {
                "primary_namespace": "legacy-flat",
                "compatibility_target": "reference-wire",
                "rules": [
                    "OpenAPI is the source of truth for every implemented route and method.",
                    "Compatibility aliases are additive and must not remove existing stable paths.",
                    "New route families should converge on reference-compatible naming and shapes."
                ]
            },
            "schema_contract": {
                "rules": [
                    "Responses use stable object schemas with explicit required fields.",
                    "Identifiers are treated as opaque strings across HTTP, SSE, ACP, and SDK surfaces.",
                    "Schema changes require additive evolution or explicit migration notes."
                ]
            },
            "event_contract": {
                "rules": [
                    "Event names remain stable and machine-consumable.",
                    "SSE payloads carry enough identifiers to correlate with REST resources.",
                    "Legacy and compatibility event vocabularies may coexist while parity work lands."
                ]
            },
            "sdk_contract": {
                "rules": [
                    "Generated SDK clients consume this OpenAPI document as the canonical contract.",
                    "Typed event clients must reconstruct turn state from SSE without polling.",
                    "Wire-compatibility gaps remain explicit until matching route and event coverage ships."
                ]
            }
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
                "patch": {
                    "operationId": "updateSession",
                    "summary": "Update a session's title and/or model selection.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/UpdateSessionRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Updated session.", "Session"),
                        "400": json_response("Update body was empty or invalid.", "ErrorResponse"),
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
            "/session/status": {
                "get": {
                    "operationId": "getSessionStatusMap",
                    "summary": "Report the status of every persisted session.",
                    "responses": {
                        "200": {
                            "description": "Map of session id to status.",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "additionalProperties": { "$ref": "#/components/schemas/SessionStatus" }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "/session/{id}/status": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "getSessionStatus",
                    "summary": "Report whether one session is running, waiting, or idle.",
                    "responses": {
                        "200": json_response("Session status.", "SessionStatus"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/wait": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "waitSession",
                    "summary": "Block until the session is no longer running, then report status.",
                    "responses": {
                        "200": json_response("Final session status.", "SessionStatus"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/context": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "getSessionContext",
                    "summary": "Report the assembled turn context (model/agent/token estimate).",
                    "responses": {
                        "200": json_response("Session context.", "SessionContext"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/children": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "listSessionChildren",
                    "summary": "List the sessions forked from this one.",
                    "responses": {
                        "200": {
                            "description": "Child session list.",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "array",
                                        "items": { "$ref": "#/components/schemas/Session" }
                                    }
                                }
                            }
                        },
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/diff": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "getSessionDiff",
                    "summary": "Report the net file changes produced by this session's turns.",
                    "responses": {
                        "200": {
                            "description": "Per-file diffs.",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "array",
                                        "items": { "$ref": "#/components/schemas/FileDiff" }
                                    }
                                }
                            }
                        },
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/fork": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "forkSession",
                    "summary": "Create a child session inheriting this session's transcript.",
                    "requestBody": {
                        "required": false,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/ForkRequest" }
                            }
                        }
                    },
                    "responses": {
                        "201": json_response("Created child session.", "Session"),
                        "400": json_response("Fork cutoff message was not found.", "ErrorResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/init": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "initSession",
                    "summary": "Analyze the project and create/update an AGENTS.md file.",
                    "responses": {
                        "200": json_response("Init turn result.", "PromptResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse"),
                        "503": json_response("Network is disabled or service is unavailable.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/summarize": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "summarizeSession",
                    "summary": "Produce a concise AI summary of the session.",
                    "responses": {
                        "200": json_response("Summary result.", "SummaryResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse"),
                        "503": json_response("Network is disabled or service is unavailable.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/compact": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "compactSession",
                    "summary": "Compact the session history into an AI summary.",
                    "responses": {
                        "200": json_response("Compaction result.", "SummaryResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse"),
                        "503": json_response("Network is disabled or service is unavailable.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/prompt_async": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "promptSessionAsync",
                    "summary": "Admit a prompt and return immediately; the turn streams over SSE.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/MessageRequest" }
                            }
                        }
                    },
                    "responses": {
                        "202": json_response("Prompt accepted for async execution.", "AsyncPromptResponse"),
                        "400": json_response("Prompt body was empty or invalid.", "ErrorResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/shell": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "shellSession",
                    "summary": "Run a shell command in the session context and persist it.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/CommandRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Shell result.", "ShellResponse"),
                        "400": json_response("Command body was empty or invalid.", "ErrorResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/revert": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "revertSession",
                    "summary": "Undo the session's recent file changes via the checkpoint store.",
                    "requestBody": {
                        "required": false,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/RevertRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Revert result.", "RevertResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse"),
                        "409": json_response("Revert refused due to out-of-band changes.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/unrevert": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "unrevertSession",
                    "summary": "Redo previously reverted file changes and clear reverted state.",
                    "responses": {
                        "200": json_response("Unrevert result.", "RevertResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse"),
                        "409": json_response("Unrevert refused due to out-of-band changes.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/message/{message_id}": {
                "parameters": [session_id_parameter(), message_id_parameter()],
                "get": {
                    "operationId": "getSessionMessage",
                    "summary": "Fetch a single message with its rich parts.",
                    "responses": {
                        "200": json_response("Message.", "Message"),
                        "404": json_response("Message was not found.", "ErrorResponse")
                    }
                },
                "delete": {
                    "operationId": "deleteSessionMessage",
                    "summary": "Delete a single message and its parts.",
                    "responses": {
                        "200": json_response("Message deleted.", "DeleteResponse"),
                        "404": json_response("Message was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/message/{message_id}/part/{part_id}": {
                "parameters": [session_id_parameter(), message_id_parameter(), part_id_parameter()],
                "patch": {
                    "operationId": "updateMessagePart",
                    "summary": "Replace one rich message part.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/RichMessagePart" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Updated part.", "RichMessagePart"),
                        "404": json_response("Message or part was not found.", "ErrorResponse")
                    }
                },
                "delete": {
                    "operationId": "deleteMessagePart",
                    "summary": "Delete one rich message part.",
                    "responses": {
                        "200": json_response("Part deleted.", "DeleteResponse"),
                        "404": json_response("Message or part was not found.", "ErrorResponse")
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
            "/session/{id}/todo": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "listSessionTodos",
                    "summary": "List persisted todo items for one session.",
                    "responses": {
                        "200": json_response("Todo list.", "TodoListResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/todos": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "listSessionTodosAlias",
                    "summary": "Alias for listing persisted todo items for one session.",
                    "responses": {
                        "200": json_response("Todo list.", "TodoListResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
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
            "/permission/pending": {
                "get": {
                    "operationId": "listPendingPermissions",
                    "summary": "List all currently pending permission requests.",
                    "responses": {
                        "200": json_response("Pending permissions.", "PendingPermissionResponse")
                    }
                }
            },
            "/permission/reply": {
                "post": {
                    "operationId": "replyPendingPermission",
                    "summary": "Resolve one pending permission request, optionally cascading to related requests.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/PermissionReplyRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Pending permission resolved.", "PermissionReplyResponse"),
                        "400": json_response("Permission reply body was invalid.", "ErrorResponse"),
                        "404": json_response("Pending permission request was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/permission/pending": {
                "parameters": [session_id_parameter()],
                "get": {
                    "operationId": "listSessionPendingPermissions",
                    "summary": "List pending permission requests for one session.",
                    "responses": {
                        "200": json_response("Pending permissions.", "PendingPermissionResponse"),
                        "404": json_response("Session was not found.", "ErrorResponse")
                    }
                }
            },
            "/session/{id}/permission/reply": {
                "parameters": [session_id_parameter()],
                "post": {
                    "operationId": "replySessionPendingPermission",
                    "summary": "Resolve one pending permission request scoped to a session.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/PermissionReplyRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Pending permission resolved.", "PermissionReplyResponse"),
                        "400": json_response("Permission reply body was invalid.", "ErrorResponse"),
                        "404": json_response("Session or pending permission was not found.", "ErrorResponse")
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
            },
            "/project/discovery": {
                "get": {
                    "operationId": "getProjectDiscovery",
                    "summary": "Discover the current project directory and plugin workspace adapters.",
                    "responses": {
                        "200": json_response("Project discovery metadata.", "ProjectDiscoveryResponse")
                    }
                }
            },
            "/experimental/workspace/adapter": {
                "get": {
                    "operationId": "listWorkspaceAdapterDiscovery",
                    "summary": "List plugin-registered experimental workspace adapters.",
                    "responses": {
                        "200": {
                            "description": "Workspace adapter discovery entries.",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "array",
                                        "items": { "$ref": "#/components/schemas/WorkspaceAdapterDiscovery" }
                                    }
                                }
                            }
                        }
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
                        "parts": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/RichMessagePart" }
                        },
                        "created_at": { "type": "string" }
                    }
                },
                "RichMessagePart": {
                    "type": "object",
                    "required": ["type"],
                    "properties": {
                        "id": { "type": "string" },
                        "created_at": { "type": "string" },
                        "type": { "type": "string" }
                    },
                    "additionalProperties": true
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
                    "required": ["status", "model", "agent", "queued"],
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["completed", "aborted"]
                        },
                        "model": { "type": "string" },
                        "agent": { "type": "string" },
                        "queued": { "type": "boolean" },
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
                "ProjectDiscoveryResponse": {
                    "type": "object",
                    "required": ["directory", "workspace_adapters"],
                    "properties": {
                        "directory": {
                            "type": "string",
                            "description": "Current server workspace directory (redacted when needed)."
                        },
                        "workspace_adapters": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/WorkspaceAdapterDiscovery" }
                        }
                    }
                },
                "PendingPermissionItem": {
                    "type": "object",
                    "required": ["id", "sessionId", "messageId", "tool", "subject", "waitedMs"],
                    "properties": {
                        "id": { "type": "string" },
                        "sessionId": { "type": "string" },
                        "messageId": { "type": "string" },
                        "tool": { "type": "string" },
                        "subject": { "type": "string" },
                        "waitedMs": { "type": "integer", "minimum": 0 }
                    }
                },
                "PendingPermissionResponse": {
                    "type": "object",
                    "required": ["pending"],
                    "properties": {
                        "pending": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/PendingPermissionItem" }
                        }
                    }
                },
                "PermissionReplyRequest": {
                    "type": "object",
                    "required": ["id", "decision"],
                    "properties": {
                        "id": { "type": "string" },
                        "decision": { "type": "string", "enum": ["allow", "deny"] },
                        "always": { "type": "boolean", "default": false },
                        "cascade": { "type": "boolean", "default": false },
                        "feedback": { "type": ["string", "null"] }
                    },
                    "additionalProperties": false
                },
                "PermissionReplyResponse": {
                    "type": "object",
                    "required": ["status", "id", "resolved"],
                    "properties": {
                        "status": { "type": "string", "const": "resolved" },
                        "id": { "type": "string" },
                        "resolved": { "type": "integer", "minimum": 0 }
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
                        "model": { "type": ["string", "null"] },
                        "parent_id": { "type": ["string", "null"] },
                        "reverted_message_id": { "type": ["string", "null"] }
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
                "TodoItem": {
                    "type": "object",
                    "required": ["content", "status", "priority"],
                    "properties": {
                        "content": { "type": "string" },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "in_progress", "completed", "cancelled"]
                        },
                        "priority": {
                            "type": "string",
                            "enum": ["high", "medium", "low"]
                        },
                    }
                },
                "TodoListResponse": {
                    "type": "object",
                    "required": ["session", "todos"],
                    "properties": {
                        "session": { "$ref": "#/components/schemas/Session" },
                        "todos": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/TodoItem" }
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
                },
                "WorkspaceAdapterDiscovery": {
                    "type": "object",
                    "required": ["plugin", "type", "name", "description"],
                    "properties": {
                        "plugin": {
                            "type": "string",
                            "description": "Plugin identity from the configured plugin spec (redacted)."
                        },
                        "type": {
                            "type": "string",
                            "description": "Stable workspace adapter type id."
                        },
                        "name": {
                            "type": "string",
                            "description": "Human-readable adapter name."
                        },
                        "description": {
                            "type": "string",
                            "description": "Adapter description surfaced to discovery clients."
                        }
                    }
                },
                "UpdateSessionRequest": {
                    "type": "object",
                    "properties": {
                        "title": { "type": ["string", "null"] },
                        "model": { "type": ["string", "null"] }
                    }
                },
                "SessionStatus": {
                    "type": "object",
                    "required": ["id", "status", "busy", "reverted", "pendingPermissions"],
                    "properties": {
                        "id": { "type": "string" },
                        "status": {
                            "type": "string",
                            "enum": ["running", "waiting", "idle"]
                        },
                        "busy": { "type": "boolean" },
                        "reverted": { "type": "boolean" },
                        "pendingPermissions": { "type": "integer", "minimum": 0 }
                    }
                },
                "SessionContext": {
                    "type": "object",
                    "required": ["id", "model", "agent", "messageCount", "estimatedTokens", "reverted", "busy"],
                    "properties": {
                        "id": { "type": "string" },
                        "model": { "type": "string" },
                        "agent": { "type": "string" },
                        "messageCount": { "type": "integer", "minimum": 0 },
                        "estimatedTokens": { "type": "integer", "minimum": 0 },
                        "reverted": { "type": "boolean" },
                        "busy": { "type": "boolean" }
                    }
                },
                "FileDiff": {
                    "type": "object",
                    "required": ["path", "status", "diff"],
                    "properties": {
                        "path": { "type": "string" },
                        "status": {
                            "type": "string",
                            "enum": ["added", "modified", "deleted"]
                        },
                        "diff": {
                            "type": "string",
                            "description": "Unified diff of net session changes for this path."
                        }
                    }
                },
                "ForkRequest": {
                    "type": "object",
                    "properties": {
                        "messageId": {
                            "type": ["string", "null"],
                            "description": "Truncate the inherited transcript at this message id."
                        },
                        "title": { "type": ["string", "null"] }
                    }
                },
                "RevertRequest": {
                    "type": "object",
                    "properties": {
                        "messageId": { "type": ["string", "null"] }
                    }
                },
                "RevertResponse": {
                    "type": "object",
                    "required": ["status", "session"],
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["reverted", "unreverted", "nothing"]
                        },
                        "session": { "$ref": "#/components/schemas/Session" },
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "conflicts": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    }
                },
                "SummaryResponse": {
                    "type": "object",
                    "required": ["status", "model", "summary", "message"],
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["summarized", "compacted"]
                        },
                        "model": { "type": "string" },
                        "summary": { "type": "string" },
                        "message": { "$ref": "#/components/schemas/Message" }
                    }
                },
                "AsyncPromptResponse": {
                    "type": "object",
                    "required": ["status", "operationId", "sessionId", "queued"],
                    "properties": {
                        "status": { "type": "string", "const": "accepted" },
                        "operationId": { "type": "string" },
                        "sessionId": { "type": "string" },
                        "queued": { "type": "boolean" }
                    }
                },
                "ShellResponse": {
                    "type": "object",
                    "required": ["status", "command", "output", "isError", "message"],
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["completed", "error"]
                        },
                        "command": { "type": "string" },
                        "output": { "type": "string" },
                        "isError": { "type": "boolean" },
                        "message": { "$ref": "#/components/schemas/Message" }
                    }
                },
                "DeleteResponse": {
                    "type": "object",
                    "required": ["status", "deleted"],
                    "properties": {
                        "status": { "type": "string", "const": "deleted" },
                        "deleted": { "type": "boolean" }
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
    ("/session/status", &["get"]),
    ("/session/{id}", &["get", "patch", "delete"]),
    ("/session/{id}/status", &["get"]),
    ("/session/{id}/wait", &["get"]),
    ("/session/{id}/context", &["get"]),
    ("/session/{id}/children", &["get"]),
    ("/session/{id}/diff", &["get"]),
    ("/session/{id}/fork", &["post"]),
    ("/session/{id}/init", &["post"]),
    ("/session/{id}/summarize", &["post"]),
    ("/session/{id}/compact", &["post"]),
    ("/session/{id}/prompt_async", &["post"]),
    ("/session/{id}/shell", &["post"]),
    ("/session/{id}/revert", &["post"]),
    ("/session/{id}/unrevert", &["post"]),
    ("/session/{id}/message", &["get", "post"]),
    ("/session/{id}/message/{message_id}", &["get", "delete"]),
    (
        "/session/{id}/message/{message_id}/part/{part_id}",
        &["patch", "delete"],
    ),
    ("/session/{id}/todo", &["get"]),
    ("/session/{id}/todos", &["get"]),
    ("/session/{id}/prompt", &["get"]),
    ("/session/{id}/prompt/append", &["post"]),
    ("/session/{id}/prompt/submit", &["post"]),
    ("/session/{id}/prompt/clear", &["post"]),
    ("/session/{id}/command", &["post"]),
    ("/permission/pending", &["get"]),
    ("/permission/reply", &["post"]),
    ("/session/{id}/permission/pending", &["get"]),
    ("/session/{id}/permission/reply", &["post"]),
    ("/session/{id}/abort", &["post"]),
    ("/session/{id}/share", &["post"]),
    ("/session/{id}/unshare", &["post"]),
    ("/share/{share_id}", &["get"]),
    ("/project/discovery", &["get"]),
    ("/experimental/workspace/adapter", &["get"]),
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

fn message_id_parameter() -> serde_json::Value {
    serde_json::json!({
        "name": "message_id",
        "in": "path",
        "required": true,
        "schema": { "type": "string" },
        "description": "Opaque message id."
    })
}

fn part_id_parameter() -> serde_json::Value {
    serde_json::json!({
        "name": "part_id",
        "in": "path",
        "required": true,
        "schema": { "type": "string" },
        "description": "Opaque message part id."
    })
}
