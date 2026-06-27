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
            },
            "/config": {
                "get": {
                    "operationId": "getConfig",
                    "summary": "Report the effective, secret-redacted configuration.",
                    "responses": {
                        "200": {
                            "description": "Redacted effective configuration table.",
                            "content": {
                                "application/json": {
                                    "schema": { "type": "object", "additionalProperties": true }
                                }
                            }
                        }
                    }
                },
                "patch": {
                    "operationId": "patchConfig",
                    "summary": "Patch the safe top-level config fields and persist them to the project file.",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/ConfigPatchRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "description": "Updated redacted effective configuration table.",
                            "content": {
                                "application/json": {
                                    "schema": { "type": "object", "additionalProperties": true }
                                }
                            }
                        },
                        "400": json_response("Unknown field or invalid value.", "ErrorResponse")
                    }
                }
            },
            "/config/providers": {
                "get": {
                    "operationId": "listConfigProviders",
                    "summary": "List providers and models, matching the cortex models catalog.",
                    "responses": {
                        "200": json_response("Provider and model catalog.", "ProvidersCatalog")
                    }
                }
            },
            "/provider": {
                "get": {
                    "operationId": "listProviders",
                    "summary": "List providers with their resolved authenticated state.",
                    "responses": {
                        "200": json_response("Providers with authenticated state.", "ProviderStateList")
                    }
                }
            },
            "/auth": {
                "get": {
                    "operationId": "listAuth",
                    "summary": "List provider auth methods and currently authenticated providers.",
                    "responses": {
                        "200": json_response("Auth method and authenticated-provider listing.", "AuthListResponse")
                    }
                }
            },
            "/auth/callback": {
                "get": {
                    "operationId": "oauthCallback",
                    "summary": "Complete an OAuth flow: validate state, exchange the code, store a token.",
                    "parameters": [
                        { "name": "code", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "state", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "error", "in": "query", "required": false, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": json_response("Credential stored; provider authenticated.", "AuthStateResponse"),
                        "400": json_response("Missing/expired state or missing code.", "ErrorResponse"),
                        "502": json_response("Token exchange failed upstream.", "ErrorResponse")
                    }
                }
            },
            "/auth/{provider}": {
                "put": {
                    "operationId": "setAuth",
                    "summary": "Store an API-key or pre-obtained OAuth token for a provider.",
                    "parameters": [provider_id_parameter()],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/SetAuthRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Provider authenticated state.", "AuthStateResponse"),
                        "400": json_response("Missing or invalid credential.", "ErrorResponse")
                    }
                },
                "delete": {
                    "operationId": "removeAuth",
                    "summary": "Remove a stored credential (logout).",
                    "parameters": [provider_id_parameter()],
                    "responses": {
                        "200": json_response("Provider authenticated state.", "AuthStateResponse"),
                        "400": json_response("Invalid provider.", "ErrorResponse")
                    }
                }
            },
            "/auth/{provider}/authorize": {
                "post": {
                    "operationId": "oauthAuthorize",
                    "summary": "Begin the OAuth authorization-code flow and return the authorize URL and state.",
                    "parameters": [provider_id_parameter()],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/AuthorizeRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": json_response("Authorization URL and CSRF state challenge.", "AuthorizeResponse"),
                        "400": json_response("Missing required OAuth fields.", "ErrorResponse")
                    }
                }
            },
            "/file": {
                "get": {
                    "operationId": "listFiles",
                    "summary": "List files and directories under a workspace-relative path.",
                    "parameters": [file_path_query(false)],
                    "responses": {
                        "200": json_array_response("Files and directories.", "FileNode"),
                        "400": json_response("Path is not a directory.", "ErrorResponse"),
                        "403": json_response("Path is outside the workspace directory.", "ErrorResponse"),
                        "404": json_response("Path was not found in the workspace.", "ErrorResponse")
                    }
                }
            },
            "/file/content": {
                "get": {
                    "operationId": "readFile",
                    "summary": "Read a workspace file's text or binary/media metadata.",
                    "parameters": [file_path_query(true)],
                    "responses": {
                        "200": json_response("File content.", "FileContent"),
                        "400": json_response("Path is a directory or missing.", "ErrorResponse"),
                        "403": json_response("Path is outside the workspace directory.", "ErrorResponse"),
                        "404": json_response("File was not found in the workspace.", "ErrorResponse")
                    }
                }
            },
            "/file/status": {
                "get": {
                    "operationId": "fileStatus",
                    "summary": "Report the git working-tree status of the workspace.",
                    "responses": {
                        "200": json_array_response("Changed files.", "FileStatus")
                    }
                }
            },
            "/find": {
                "get": {
                    "operationId": "findText",
                    "summary": "Search file contents for a text pattern using ripgrep.",
                    "parameters": [
                        { "name": "pattern", "in": "query", "required": true, "schema": { "type": "string" } },
                        find_limit_query()
                    ],
                    "responses": {
                        "200": json_array_response("Text matches.", "TextMatch"),
                        "400": json_response("Pattern is required.", "ErrorResponse")
                    }
                }
            },
            "/find/file": {
                "get": {
                    "operationId": "findFiles",
                    "summary": "Search for files or directories by name in the workspace.",
                    "parameters": [
                        { "name": "query", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "dirs", "in": "query", "required": false, "schema": { "type": "string", "enum": ["true", "false"] } },
                        { "name": "type", "in": "query", "required": false, "schema": { "type": "string", "enum": ["file", "directory"] } },
                        find_limit_query()
                    ],
                    "responses": {
                        "200": {
                            "description": "Matching file paths.",
                            "content": {
                                "application/json": {
                                    "schema": { "type": "array", "items": { "type": "string" } }
                                }
                            }
                        }
                    }
                }
            },
            "/find/symbol": {
                "get": {
                    "operationId": "findSymbols",
                    "summary": "Search source-symbol definitions across the workspace.",
                    "parameters": [
                        { "name": "query", "in": "query", "required": true, "schema": { "type": "string" } },
                        find_limit_query()
                    ],
                    "responses": {
                        "200": json_array_response("Workspace symbols.", "Symbol"),
                        "400": json_response("Query is required.", "ErrorResponse")
                    }
                }
            },
            "/agent": {
                "get": {
                    "operationId": "listAgents",
                    "summary": "List built-in, custom, and plugin agents.",
                    "responses": {
                        "200": json_array_response("Agent catalog entries.", "AgentEntry")
                    }
                }
            },
            "/model": {
                "get": {
                    "operationId": "listModels",
                    "summary": "List every catalog model, including plugin-registered models.",
                    "responses": {
                        "200": json_response("Model catalog with the default model.", "ModelCatalog")
                    }
                }
            },
            "/command": {
                "get": {
                    "operationId": "listCommands",
                    "summary": "List config-defined custom commands and plugin commands.",
                    "responses": {
                        "200": json_array_response("Command catalog entries.", "CommandEntry")
                    }
                }
            },
            "/skill": {
                "get": {
                    "operationId": "listSkills",
                    "summary": "List discovered skills and plugin skills.",
                    "responses": {
                        "200": json_array_response("Skill catalog entries.", "SkillEntry")
                    }
                }
            },
            "/lsp": {
                "get": {
                    "operationId": "lspStatus",
                    "summary": "Report configured language server status (mirrors `cortex lsp status`).",
                    "responses": {
                        "200": json_array_response("Language server status rows.", "LspStatusRow")
                    }
                }
            },
            "/formatter": {
                "get": {
                    "operationId": "formatterStatus",
                    "summary": "Report formatter status (mirrors `cortex formatter status`).",
                    "responses": {
                        "200": json_array_response("Formatter status rows.", "FormatterStatusRow")
                    }
                }
            },
            "/vcs/diff": {
                "get": {
                    "operationId": "vcsDiff",
                    "summary": "Report the working-tree git diff (read-only).",
                    "parameters": [
                        file_path_query(false),
                        { "name": "staged", "in": "query", "required": false, "schema": { "type": "boolean" }, "description": "Diff against HEAD (staged + unstaged) instead of only unstaged changes." }
                    ],
                    "responses": {
                        "200": json_response("Working-tree diff and per-file stats.", "VcsDiffResponse")
                    }
                }
            },
            "/vcs/apply": {
                "post": {
                    "operationId": "vcsApply",
                    "summary": "Apply a unified diff to the working tree via git apply.",
                    "requestBody": json_request_body("VcsApplyRequest"),
                    "responses": {
                        "200": json_response("Patch applied or validated.", "VcsApplyResponse"),
                        "400": json_response("Missing diff or not a git repository.", "ErrorResponse"),
                        "422": json_response("Patch did not apply cleanly.", "ErrorResponse")
                    }
                }
            },
            "/project": {
                "get": {
                    "operationId": "listProjects",
                    "summary": "List projects known to this server (the active workspace project).",
                    "responses": {
                        "200": json_array_response("Projects.", "Project")
                    }
                }
            },
            "/project/current": {
                "get": {
                    "operationId": "currentProject",
                    "summary": "Describe the active workspace project.",
                    "responses": {
                        "200": json_response("The active project.", "Project")
                    }
                }
            },
            "/workspace": {
                "get": {
                    "operationId": "currentWorkspace",
                    "summary": "Describe the current workspace from git/filesystem state.",
                    "responses": {
                        "200": json_response("The current workspace.", "Workspace")
                    }
                }
            },
            "/worktree": {
                "get": {
                    "operationId": "listWorktrees",
                    "summary": "List git worktrees attached to the workspace repository.",
                    "responses": {
                        "200": json_array_response("Git worktrees.", "Worktree")
                    }
                }
            },
            "/path": {
                "get": {
                    "operationId": "pathInfo",
                    "summary": "Workspace paths summary, or metadata for a specific workspace path.",
                    "parameters": [ file_path_query(false) ],
                    "responses": {
                        "200": json_response("Path summary or metadata.", "PathInfoResponse"),
                        "403": json_response("Path escapes the workspace.", "ErrorResponse"),
                        "404": json_response("Path was not found.", "ErrorResponse")
                    }
                }
            },
            "/pty": {
                "get": {
                    "operationId": "listPtys",
                    "summary": "List live PTY sessions.",
                    "responses": {
                        "200": json_array_response("PTY sessions.", "PtySummary")
                    }
                },
                "post": {
                    "operationId": "createPty",
                    "summary": "Create a PTY running a real child process.",
                    "requestBody": json_request_body("CreatePtyRequest"),
                    "responses": {
                        "201": json_response("Created PTY session.", "PtySummary"),
                        "400": json_response("Could not spawn the requested command.", "ErrorResponse")
                    }
                }
            },
            "/pty/{id}": {
                "get": {
                    "operationId": "getPty",
                    "summary": "Report one PTY session's state.",
                    "parameters": [ pty_id_parameter() ],
                    "responses": {
                        "200": json_response("PTY session state.", "PtySummary"),
                        "404": json_response("No such PTY.", "ErrorResponse")
                    }
                },
                "delete": {
                    "operationId": "closePty",
                    "summary": "Kill the PTY child by its exact handle and drop the session.",
                    "parameters": [ pty_id_parameter() ],
                    "responses": {
                        "200": json_response("PTY closed.", "PtyCloseResponse"),
                        "404": json_response("No such PTY.", "ErrorResponse")
                    }
                }
            },
            "/pty/{id}/input": {
                "post": {
                    "operationId": "ptyInput",
                    "summary": "Write input bytes to the PTY.",
                    "parameters": [ pty_id_parameter() ],
                    "requestBody": json_request_body("PtyInputRequest"),
                    "responses": {
                        "200": json_response("Input written.", "PtyInputResponse"),
                        "404": json_response("No such PTY.", "ErrorResponse")
                    }
                }
            },
            "/pty/{id}/resize": {
                "post": {
                    "operationId": "ptyResize",
                    "summary": "Resize the PTY and update its stored dimensions.",
                    "parameters": [ pty_id_parameter() ],
                    "requestBody": json_request_body("PtyResizeRequest"),
                    "responses": {
                        "200": json_response("PTY resized.", "PtySummary"),
                        "404": json_response("No such PTY.", "ErrorResponse")
                    }
                }
            },
            "/pty/{id}/stream": {
                "get": {
                    "operationId": "ptyStream",
                    "summary": "Stream PTY output as Server-Sent Events.",
                    "parameters": [ pty_id_parameter() ],
                    "responses": {
                        "200": {
                            "description": "SSE stream of base64-encoded PTY output frames.",
                            "content": {
                                "text/event-stream": {
                                    "schema": { "type": "string", "description": "SSE frames: `output` data events and a terminal `exit` event." }
                                }
                            }
                        },
                        "404": json_response("No such PTY.", "ErrorResponse")
                    }
                }
            },
            "/tui/append-prompt": {
                "post": {
                    "operationId": "tuiAppendPrompt",
                    "summary": "Append text to the attached TUI's prompt composer.",
                    "requestBody": json_request_body("AppendPromptRequest"),
                    "responses": {
                        "200": json_response("Control applied by the attached TUI.", "ControlAck"),
                        "400": json_response("Missing text.", "ErrorResponse"),
                        "503": json_response("No attached TUI responded.", "ErrorResponse")
                    }
                }
            },
            "/tui/submit-prompt": {
                "post": {
                    "operationId": "tuiSubmitPrompt",
                    "summary": "Submit the attached TUI's current prompt.",
                    "responses": {
                        "200": json_response("Control applied by the attached TUI.", "ControlAck"),
                        "503": json_response("No attached TUI responded.", "ErrorResponse")
                    }
                }
            },
            "/tui/clear-prompt": {
                "post": {
                    "operationId": "tuiClearPrompt",
                    "summary": "Clear the attached TUI's prompt composer.",
                    "responses": {
                        "200": json_response("Control applied by the attached TUI.", "ControlAck"),
                        "503": json_response("No attached TUI responded.", "ErrorResponse")
                    }
                }
            },
            "/tui/open-models": {
                "post": {
                    "operationId": "tuiOpenModels",
                    "summary": "Open the model picker in the attached TUI.",
                    "responses": {
                        "200": json_response("Control applied by the attached TUI.", "ControlAck"),
                        "503": json_response("No attached TUI responded.", "ErrorResponse")
                    }
                }
            },
            "/tui/execute-command": {
                "post": {
                    "operationId": "tuiExecuteCommand",
                    "summary": "Run a named TUI command in the attached TUI.",
                    "requestBody": json_request_body("ExecuteCommandRequest"),
                    "responses": {
                        "200": json_response("Control applied by the attached TUI.", "ControlAck"),
                        "400": json_response("Missing command.", "ErrorResponse"),
                        "503": json_response("No attached TUI responded.", "ErrorResponse")
                    }
                }
            },
            "/tui/show-toast": {
                "post": {
                    "operationId": "tuiShowToast",
                    "summary": "Show a toast notification in the attached TUI.",
                    "requestBody": json_request_body("ShowToastRequest"),
                    "responses": {
                        "200": json_response("Control applied by the attached TUI.", "ControlAck"),
                        "400": json_response("Missing message.", "ErrorResponse"),
                        "503": json_response("No attached TUI responded.", "ErrorResponse")
                    }
                }
            },
            "/tui/select-session": {
                "post": {
                    "operationId": "tuiSelectSession",
                    "summary": "Switch the attached TUI to a different session.",
                    "requestBody": json_request_body("SelectSessionRequest"),
                    "responses": {
                        "200": json_response("Control applied by the attached TUI.", "ControlAck"),
                        "400": json_response("Missing session id.", "ErrorResponse"),
                        "503": json_response("No attached TUI responded.", "ErrorResponse")
                    }
                }
            },
            "/tui/control/next": {
                "get": {
                    "operationId": "tuiControlNext",
                    "summary": "Attached TUI polls for the next control request.",
                    "responses": {
                        "200": json_response("Next control request.", "ControlRequest"),
                        "204": { "description": "No control request is queued." }
                    }
                }
            },
            "/tui/control/response": {
                "post": {
                    "operationId": "tuiControlResponse",
                    "summary": "Attached TUI reports the outcome of a control request.",
                    "requestBody": json_request_body("ControlResponseRequest"),
                    "responses": {
                        "200": json_response("Outcome recorded.", "ControlResponseAck"),
                        "400": json_response("Missing request id.", "ErrorResponse")
                    }
                }
            }
        },
        "components": {
            "schemas": {
                "FileNode": {
                    "type": "object",
                    "required": ["name", "path", "absolute", "type", "ignored"],
                    "properties": {
                        "name": { "type": "string" },
                        "path": { "type": "string" },
                        "absolute": { "type": "string" },
                        "type": { "type": "string", "enum": ["file", "directory"] },
                        "ignored": { "type": "boolean" }
                    }
                },
                "FileContent": {
                    "type": "object",
                    "required": ["type", "content"],
                    "properties": {
                        "type": { "type": "string", "enum": ["text", "binary"] },
                        "content": { "type": "string" },
                        "encoding": { "type": "string", "enum": ["base64"] },
                        "mimeType": { "type": "string" }
                    }
                },
                "FileStatus": {
                    "type": "object",
                    "required": ["path", "added", "removed", "status"],
                    "properties": {
                        "path": { "type": "string" },
                        "added": { "type": "integer", "minimum": 0 },
                        "removed": { "type": "integer", "minimum": 0 },
                        "status": { "type": "string", "enum": ["added", "deleted", "modified"] }
                    }
                },
                "TextMatch": {
                    "type": "object",
                    "required": ["path", "lines", "line_number", "absolute_offset", "submatches"],
                    "properties": {
                        "path": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" } } },
                        "lines": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" } } },
                        "line_number": { "type": "integer", "minimum": 0 },
                        "absolute_offset": { "type": "integer", "minimum": 0 },
                        "submatches": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["match", "start", "end"],
                                "properties": {
                                    "match": { "type": "object", "required": ["text"], "properties": { "text": { "type": "string" } } },
                                    "start": { "type": "integer", "minimum": 0 },
                                    "end": { "type": "integer", "minimum": 0 }
                                }
                            }
                        }
                    }
                },
                "Symbol": {
                    "type": "object",
                    "required": ["name", "kind", "location"],
                    "properties": {
                        "name": { "type": "string" },
                        "kind": { "type": "integer", "minimum": 0 },
                        "location": {
                            "type": "object",
                            "required": ["uri", "range"],
                            "properties": {
                                "uri": { "type": "string" },
                                "range": {
                                    "type": "object",
                                    "required": ["start", "end"],
                                    "properties": {
                                        "start": { "$ref": "#/components/schemas/SymbolPosition" },
                                        "end": { "$ref": "#/components/schemas/SymbolPosition" }
                                    }
                                }
                            }
                        }
                    }
                },
                "SymbolPosition": {
                    "type": "object",
                    "required": ["line", "character"],
                    "properties": {
                        "line": { "type": "integer", "minimum": 0 },
                        "character": { "type": "integer", "minimum": 0 }
                    }
                },
                "AgentEntry": {
                    "type": "object",
                    "required": ["name", "description", "mode", "source"],
                    "properties": {
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "mode": { "type": "string" },
                        "source": { "type": "string", "enum": ["builtin", "custom", "plugin"] }
                    }
                },
                "CommandEntry": {
                    "type": "object",
                    "required": ["name", "description", "source"],
                    "properties": {
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "source": { "type": "string", "enum": ["config", "plugin"] }
                    }
                },
                "SkillEntry": {
                    "type": "object",
                    "required": ["name", "description", "source"],
                    "properties": {
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "source": { "type": "string", "enum": ["config", "plugin"] }
                    }
                },
                "ModelCatalog": {
                    "type": "object",
                    "required": ["default_model", "models"],
                    "properties": {
                        "default_model": { "type": "string" },
                        "models": { "type": "array", "items": { "type": "object", "additionalProperties": true } }
                    }
                },
                "LspStatusRow": {
                    "type": "object",
                    "required": ["language", "program", "state"],
                    "properties": {
                        "language": { "type": "string" },
                        "program": { "type": "string" },
                        "state": { "type": "string", "enum": ["disabled", "installing", "running", "available", "unavailable", "crashed"] },
                        "pid": { "type": "integer", "minimum": 0 },
                        "install": { "type": "string" }
                    }
                },
                "FormatterStatusRow": {
                    "type": "object",
                    "required": ["name", "formatter", "state", "source", "marker_required"],
                    "properties": {
                        "name": { "type": "string" },
                        "formatter": { "type": "string" },
                        "state": { "type": "string", "enum": ["enabled", "disabled", "missing", "missing_config_marker"] },
                        "source": { "type": "string", "enum": ["default", "config"] },
                        "command": { "type": "array", "items": { "type": "string" } },
                        "env_keys": { "type": "array", "items": { "type": "string" } },
                        "extensions": { "type": "array", "items": { "type": "string" } },
                        "marker_required": { "type": "boolean" },
                        "marker_found": { "type": "boolean" }
                    }
                },
                "VcsFile": {
                    "type": "object",
                    "required": ["path", "added", "removed", "status"],
                    "properties": {
                        "path": { "type": "string" },
                        "added": { "type": "integer", "minimum": 0 },
                        "removed": { "type": "integer", "minimum": 0 },
                        "status": { "type": "string" }
                    }
                },
                "VcsDiffResponse": {
                    "type": "object",
                    "required": ["repository", "diff", "files"],
                    "properties": {
                        "repository": { "type": "boolean" },
                        "diff": { "type": "string" },
                        "files": { "type": "array", "items": { "$ref": "#/components/schemas/VcsFile" } }
                    }
                },
                "VcsApplyRequest": {
                    "type": "object",
                    "required": ["diff"],
                    "properties": {
                        "diff": { "type": "string", "description": "Unified diff to apply via git apply." },
                        "check": { "type": "boolean", "description": "Validate only (git apply --check); never writes to disk." },
                        "reverse": { "type": "boolean", "description": "Apply the patch in reverse (git apply --reverse)." }
                    },
                    "additionalProperties": false
                },
                "VcsApplyResponse": {
                    "type": "object",
                    "required": ["status", "applied", "checked"],
                    "properties": {
                        "status": { "type": "string", "enum": ["applied", "valid"] },
                        "applied": { "type": "boolean" },
                        "checked": { "type": "boolean" }
                    }
                },
                "Project": {
                    "type": "object",
                    "required": ["id", "directory", "worktree"],
                    "properties": {
                        "id": { "type": "string" },
                        "directory": { "type": "string" },
                        "worktree": { "type": "string" },
                        "vcs": { "type": "string", "enum": ["git"] }
                    }
                },
                "Workspace": {
                    "type": "object",
                    "required": ["id", "root", "directory", "worktrees"],
                    "properties": {
                        "id": { "type": "string" },
                        "root": { "type": "string" },
                        "directory": { "type": "string" },
                        "vcs": { "type": "string", "enum": ["git"] },
                        "worktrees": { "type": "integer", "minimum": 0 }
                    }
                },
                "Worktree": {
                    "type": "object",
                    "required": ["path", "bare", "detached", "locked"],
                    "properties": {
                        "path": { "type": "string" },
                        "head": { "type": "string" },
                        "branch": { "type": "string" },
                        "bare": { "type": "boolean" },
                        "detached": { "type": "boolean" },
                        "locked": { "type": "boolean" }
                    }
                },
                "PathInfoResponse": {
                    "type": "object",
                    "description": "Workspace paths summary (directory/worktree/root/config) or path metadata (path/absolute/type/exists).",
                    "additionalProperties": true
                },
                "PtySummary": {
                    "type": "object",
                    "required": ["id", "cols", "rows", "command", "running"],
                    "properties": {
                        "id": { "type": "string" },
                        "pid": { "type": "integer", "minimum": 0 },
                        "cols": { "type": "integer", "minimum": 1 },
                        "rows": { "type": "integer", "minimum": 1 },
                        "command": { "type": "string" },
                        "running": { "type": "boolean" }
                    }
                },
                "CreatePtyRequest": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Program to run; defaults to $SHELL or /bin/sh." },
                        "args": { "type": "array", "items": { "type": "string" } },
                        "cwd": { "type": "string", "description": "Workspace-relative working directory." },
                        "cols": { "type": "integer", "minimum": 1 },
                        "rows": { "type": "integer", "minimum": 1 },
                        "env": { "type": "object", "additionalProperties": { "type": "string" } }
                    },
                    "additionalProperties": false
                },
                "PtyInputRequest": {
                    "type": "object",
                    "required": ["data"],
                    "properties": {
                        "data": { "type": "string" },
                        "base64": { "type": "boolean", "description": "Interpret data as base64 raw bytes instead of UTF-8 text." }
                    },
                    "additionalProperties": false
                },
                "PtyInputResponse": {
                    "type": "object",
                    "required": ["status", "bytes"],
                    "properties": {
                        "status": { "type": "string", "const": "written" },
                        "bytes": { "type": "integer", "minimum": 0 }
                    }
                },
                "PtyResizeRequest": {
                    "type": "object",
                    "required": ["cols", "rows"],
                    "properties": {
                        "cols": { "type": "integer", "minimum": 1 },
                        "rows": { "type": "integer", "minimum": 1 }
                    },
                    "additionalProperties": false
                },
                "PtyCloseResponse": {
                    "type": "object",
                    "required": ["status", "exited"],
                    "properties": {
                        "status": { "type": "string", "const": "closed" },
                        "pid": { "type": "integer", "minimum": 0 },
                        "exited": { "type": "boolean" }
                    }
                },
                "ControlRequest": {
                    "type": "object",
                    "required": ["id", "action", "body"],
                    "properties": {
                        "id": { "type": "string" },
                        "action": { "type": "string" },
                        "body": { "type": "object", "additionalProperties": true }
                    }
                },
                "ControlAck": {
                    "type": "object",
                    "required": ["status", "action"],
                    "properties": {
                        "status": { "type": "string", "enum": ["ok", "error"] },
                        "action": { "type": "string" },
                        "error": { "type": "string" }
                    }
                },
                "ControlResponseRequest": {
                    "type": "object",
                    "required": ["id"],
                    "properties": {
                        "id": { "type": "string" },
                        "success": { "type": "boolean" },
                        "error": { "type": "string" }
                    },
                    "additionalProperties": false
                },
                "ControlResponseAck": {
                    "type": "object",
                    "required": ["status", "matched"],
                    "properties": {
                        "status": { "type": "string", "const": "ok" },
                        "matched": { "type": "boolean" }
                    }
                },
                "AppendPromptRequest": {
                    "type": "object",
                    "required": ["text"],
                    "properties": { "text": { "type": "string" } },
                    "additionalProperties": false
                },
                "ExecuteCommandRequest": {
                    "type": "object",
                    "required": ["command"],
                    "properties": { "command": { "type": "string" } },
                    "additionalProperties": false
                },
                "ShowToastRequest": {
                    "type": "object",
                    "required": ["message"],
                    "properties": {
                        "message": { "type": "string" },
                        "variant": { "type": "string", "enum": ["info", "success", "warning", "error"] },
                        "title": { "type": "string" }
                    },
                    "additionalProperties": false
                },
                "SelectSessionRequest": {
                    "type": "object",
                    "required": ["session_id"],
                    "properties": { "session_id": { "type": "string" } },
                    "additionalProperties": false
                },
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
                },
                "ConfigPatchRequest": {
                    "type": "object",
                    "description": "Safe top-level config fields a client may patch.",
                    "properties": {
                        "model": { "type": "string", "description": "Fully-qualified provider/model id." },
                        "agent": { "type": "string", "description": "Default agent persona." },
                        "max_steps": { "type": "integer", "minimum": 1, "description": "Per-turn step budget." }
                    },
                    "additionalProperties": false
                },
                "ProvidersCatalog": {
                    "type": "object",
                    "required": ["default_model", "providers", "provider_config"],
                    "properties": {
                        "default_model": { "type": "string" },
                        "providers": {
                            "type": "array",
                            "items": { "type": "object", "required": ["id", "name", "models"] }
                        },
                        "provider_config": { "type": "object" }
                    }
                },
                "ProviderStateList": {
                    "type": "object",
                    "required": ["default_model", "providers"],
                    "properties": {
                        "default_model": { "type": "string" },
                        "providers": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/ProviderState" }
                        }
                    }
                },
                "ProviderState": {
                    "type": "object",
                    "required": ["id", "name", "authenticated", "source"],
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "authenticated": { "type": "boolean" },
                        "source": { "type": "string" }
                    }
                },
                "AuthListResponse": {
                    "type": "object",
                    "required": ["authenticated", "methods"],
                    "properties": {
                        "authenticated": { "type": "array", "items": { "type": "string" } },
                        "methods": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["provider", "name", "types", "env", "authenticated"]
                            }
                        }
                    }
                },
                "SetAuthRequest": {
                    "type": "object",
                    "description": "Store an API key or pre-obtained OAuth token for a provider.",
                    "properties": {
                        "type": { "type": "string", "enum": ["api", "oauth"] },
                        "key": { "type": "string", "description": "API key (api) or token (oauth)." },
                        "api_key": { "type": "string" },
                        "access_token": { "type": "string" }
                    }
                },
                "AuthorizeRequest": {
                    "type": "object",
                    "required": ["authorize_endpoint", "token_endpoint", "client_id"],
                    "properties": {
                        "authorize_endpoint": { "type": "string" },
                        "token_endpoint": { "type": "string" },
                        "client_id": { "type": "string" },
                        "client_secret": { "type": "string" },
                        "redirect_uri": { "type": "string" },
                        "scopes": { "type": "array", "items": { "type": "string" } }
                    }
                },
                "AuthorizeResponse": {
                    "type": "object",
                    "required": ["provider", "authorize_url", "state", "redirect_uri"],
                    "properties": {
                        "provider": { "type": "string" },
                        "authorize_url": { "type": "string" },
                        "state": { "type": "string" },
                        "redirect_uri": { "type": "string" }
                    }
                },
                "AuthStateResponse": {
                    "type": "object",
                    "required": ["provider", "authenticated"],
                    "properties": {
                        "provider": { "type": "string" },
                        "authenticated": { "type": "boolean" },
                        "method": { "type": ["string", "null"] }
                    }
                }
            }
        }
    });
    add_auth_documentation(&mut document);
    add_event_taxonomy(&mut document);
    document
}

/// Publish the full SSE event taxonomy onto the OpenAPI document so a generated SDK can enumerate
/// and decode every event family into a typed variant without special-casing server-only names.
fn add_event_taxonomy(document: &mut serde_json::Value) {
    let kinds: Vec<&'static str> = events::Event::all_kinds().to_vec();
    document["x-event-families"] = serde_json::json!(kinds);
    if let Some(schemas) = document
        .get_mut("components")
        .and_then(|components| components.get_mut("schemas"))
        .and_then(serde_json::Value::as_object_mut)
    {
        schemas.insert(
            "Event".to_owned(),
            serde_json::json!({
                "type": "object",
                "required": ["kind"],
                "description": "A typed Cortex event. The `kind` discriminator selects the family; \
                                payload fields are flat alongside it.",
                "discriminator": { "propertyName": "kind" },
                "properties": {
                    "kind": {
                        "type": "string",
                        "description": "Stable event family name.",
                        "enum": kinds,
                    }
                }
            }),
        );
    }
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
    ("/config", &["get", "patch"]),
    ("/config/providers", &["get"]),
    ("/provider", &["get"]),
    ("/auth", &["get"]),
    ("/auth/{provider}", &["put", "delete"]),
    ("/auth/{provider}/authorize", &["post"]),
    ("/file", &["get"]),
    ("/file/content", &["get"]),
    ("/file/status", &["get"]),
    ("/find", &["get"]),
    ("/find/file", &["get"]),
    ("/find/symbol", &["get"]),
    ("/agent", &["get"]),
    ("/model", &["get"]),
    ("/command", &["get"]),
    ("/skill", &["get"]),
    ("/lsp", &["get"]),
    ("/formatter", &["get"]),
    ("/vcs/diff", &["get"]),
    ("/vcs/apply", &["post"]),
    ("/project", &["get"]),
    ("/project/current", &["get"]),
    ("/workspace", &["get"]),
    ("/worktree", &["get"]),
    ("/path", &["get"]),
    ("/pty", &["get", "post"]),
    ("/pty/{id}", &["get", "delete"]),
    ("/pty/{id}/input", &["post"]),
    ("/pty/{id}/resize", &["post"]),
    ("/pty/{id}/stream", &["get"]),
    ("/tui/append-prompt", &["post"]),
    ("/tui/submit-prompt", &["post"]),
    ("/tui/clear-prompt", &["post"]),
    ("/tui/open-models", &["post"]),
    ("/tui/execute-command", &["post"]),
    ("/tui/show-toast", &["post"]),
    ("/tui/select-session", &["post"]),
    ("/tui/control/next", &["get"]),
    ("/tui/control/response", &["post"]),
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

fn json_array_response(description: &str, schema: &str) -> serde_json::Value {
    serde_json::json!({
        "description": description,
        "content": {
            "application/json": {
                "schema": {
                    "type": "array",
                    "items": { "$ref": format!("#/components/schemas/{schema}") }
                }
            }
        }
    })
}

fn json_request_body(schema: &str) -> serde_json::Value {
    serde_json::json!({
        "required": true,
        "content": {
            "application/json": {
                "schema": { "$ref": format!("#/components/schemas/{schema}") }
            }
        }
    })
}

fn pty_id_parameter() -> serde_json::Value {
    serde_json::json!({
        "name": "id",
        "in": "path",
        "required": true,
        "schema": { "type": "string" },
        "description": "Opaque PTY session id."
    })
}

fn file_path_query(required: bool) -> serde_json::Value {
    serde_json::json!({
        "name": "path",
        "in": "query",
        "required": required,
        "schema": { "type": "string" },
        "description": "Workspace-relative path. Paths outside the workspace are refused."
    })
}

fn find_limit_query() -> serde_json::Value {
    serde_json::json!({
        "name": "limit",
        "in": "query",
        "required": false,
        "schema": { "type": "integer", "minimum": 1 },
        "description": "Maximum number of results to return (bounded by the server)."
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

fn provider_id_parameter() -> serde_json::Value {
    serde_json::json!({
        "name": "provider",
        "in": "path",
        "required": true,
        "schema": { "type": "string" },
        "description": "Provider id (e.g. openrouter)."
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
