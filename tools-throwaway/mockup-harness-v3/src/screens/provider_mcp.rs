//! Flow 4 pilot states for provider connection and capability-led MCP.
//!
//! Provider authentication and MCP management remain separate branches. Provider
//! steps keep the blocked model origin and return there on success or cancel.
//! MCP leads with human capabilities and local recovery actions, never transport
//! internals, and never continues from provider success by adjacency.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, ModelChoiceEvidence,
        RenderRef, SemanticCandidate, SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::ENTRY_CODE_BANNER,
        layout::{ACTION_DOCK_TOP, CANVAS_ROWS, ScreenComposition, StatusLine},
        tokens::{ColorToken, TextStyle},
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Sonnet 4.5";
const DOCK_TOP: u16 = ACTION_DOCK_TOP;
const FLOW4_PROVIDER_TASK: &str = "Connect the model needed for this conversation.";
const FLOW4_PROVIDER_ORIGIN: &str = "Provider connection · blocked model action";
const FLOW4_ONBOARDING_ORIGIN: &str = "Provider connection · onboarding";
const FLOW4_CONVERSATION_ORIGIN: &str = "Provider connection · conversation";
const FLOW4_ONBOARDING_TASK: &str = "Connect a provider for first-run setup.";
const FLOW4_CONVERSATION_TASK: &str = "Connect a provider without losing this conversation.";
const FLOW4_MCP_TASK: &str = "Fix the capabilities this conversation needs.";
const FLOW4_MCP_ORIGIN: &str = "MCP · conversation capabilities";

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderOrigin {
    BlockedModel,
    Onboarding,
    Conversation,
}

impl ProviderOrigin {
    fn from_fixture(fixture_id: &str) -> Self {
        let lower = fixture_id.to_ascii_lowercase();
        if lower.contains("onboarding") {
            Self::Onboarding
        } else if lower.contains("conversation") {
            Self::Conversation
        } else {
            // Gate fixtures and the default production fixture stay blocked-model.
            Self::BlockedModel
        }
    }

    fn visible_label(self) -> &'static str {
        match self {
            Self::BlockedModel => FLOW4_PROVIDER_ORIGIN,
            Self::Onboarding => FLOW4_ONBOARDING_ORIGIN,
            Self::Conversation => FLOW4_CONVERSATION_ORIGIN,
        }
    }

    fn user_task(self) -> &'static str {
        match self {
            Self::BlockedModel => FLOW4_PROVIDER_TASK,
            Self::Onboarding => FLOW4_ONBOARDING_TASK,
            Self::Conversation => FLOW4_CONVERSATION_TASK,
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Self::BlockedModel => "Blocked model action",
            Self::Onboarding => "Onboarding setup",
            Self::Conversation => "Current conversation",
        }
    }

    fn login_cancel_target(self) -> &'static str {
        match self {
            Self::BlockedModel => "model-picker",
            Self::Onboarding => "onboarding",
            Self::Conversation => "chat-turn",
        }
    }

    fn login_cancel_hint(self) -> &'static str {
        match self {
            Self::BlockedModel => "  [esc] Cancel · return to model choices",
            Self::Onboarding => "  [esc] Cancel · return to onboarding",
            Self::Conversation => "  [esc] Cancel · return to conversation",
        }
    }

    fn login_pause_copy(self) -> &'static str {
        match self {
            Self::BlockedModel => {
                "Your blocked model choice and conversation stay paused while you connect."
            }
            Self::Onboarding => {
                "First-run setup stays exactly where you left it while you connect."
            }
            Self::Conversation => {
                "Your conversation draft and transcript stay paused while you connect."
            }
        }
    }

    fn login_scope_copy(self) -> &'static str {
        match self {
            Self::BlockedModel => {
                "This path only unlocks the model provider for the paused choice."
            }
            Self::Onboarding => "This path only unlocks a model provider for first-run setup.",
            Self::Conversation => {
                "This path only unlocks a model provider for this conversation."
            }
        }
    }

    fn blocked_banner(self) -> &'static str {
        match self {
            Self::BlockedModel => "GPT-5.2 Codex is waiting for a provider connection.",
            Self::Onboarding => "First-run setup needs a model provider connection.",
            Self::Conversation => "This conversation needs a model provider connection.",
        }
    }

    fn connect_consequence(self) -> &'static str {
        match self {
            Self::BlockedModel => {
                "Connecting unlocks GPT-5.2 Codex for the paused model choice."
            }
            Self::Onboarding => "Connecting unlocks models for first-run setup.",
            Self::Conversation => {
                "Connecting unlocks models without discarding the saved draft."
            }
        }
    }

    fn return_policy(self, screen_id: &str) -> String {
        if screen_id == "connect-connected" {
            match self {
                Self::BlockedModel => "Return to model choices".to_owned(),
                Self::Onboarding => "Return to onboarding".to_owned(),
                Self::Conversation => "Return to conversation".to_owned(),
            }
        } else {
            match self {
                Self::BlockedModel => "Return to blocked model action".to_owned(),
                Self::Onboarding => "Return to onboarding setup".to_owned(),
                Self::Conversation => "Return to conversation".to_owned(),
            }
        }
    }

    fn origin_subject(self) -> &'static str {
        match self {
            Self::BlockedModel => "blocked-model-origin",
            Self::Onboarding => "onboarding-origin",
            Self::Conversation => "conversation-origin",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderRowState {
    InUse,
    Connected,
    NotConnected,
}

impl ProviderRowState {
    fn label(self) -> &'static str {
        match self {
            Self::InUse => "Connected · in use",
            Self::Connected => "Connected",
            Self::NotConnected => "Not connected",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ProviderCatalogueRow {
    id: &'static str,
    name: &'static str,
    benefit: &'static str,
    state: ProviderRowState,
    methods: &'static [&'static str],
}

const PROVIDER_CATALOGUE: &[ProviderCatalogueRow] = &[
    ProviderCatalogueRow {
        id: "openrouter",
        name: "OpenRouter",
        benefit: "Unified access to many models",
        state: ProviderRowState::InUse,
        methods: &["api-key"],
    },
    ProviderCatalogueRow {
        id: "anthropic",
        name: "Anthropic",
        benefit: "Claude models for everyday coding",
        state: ProviderRowState::Connected,
        methods: &["api-key", "oauth"],
    },
    ProviderCatalogueRow {
        id: "openai",
        name: "OpenAI",
        benefit: "GPT-5.2 Codex and related models",
        state: ProviderRowState::NotConnected,
        methods: &["oauth", "api-key"],
    },
    ProviderCatalogueRow {
        id: "google",
        name: "Google",
        benefit: "Gemini models for broad tasks",
        state: ProviderRowState::NotConnected,
        methods: &["api-key"],
    },
];

fn selected_provider_id(fixture_id: &str) -> &'static str {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("selected-anthropic") || lower.contains("anthropic-selected") {
        "anthropic"
    } else if lower.contains("selected-google") || lower.contains("google-selected") {
        "google"
    } else if lower.contains("selected-openrouter") || lower.contains("openrouter-selected") {
        "openrouter"
    } else if lower.contains("selected-openai") || lower.contains("openai-selected") {
        "openai"
    } else if lower.contains("google") && lower.contains("method") {
        "google"
    } else {
        "openai"
    }
}

fn method_provider_id(fixture_id: &str) -> &'static str {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("google") {
        "google"
    } else if lower.contains("anthropic") {
        "anthropic"
    } else if lower.contains("openrouter") {
        "openrouter"
    } else {
        "openai"
    }
}

fn provider_row(id: &str) -> &'static ProviderCatalogueRow {
    PROVIDER_CATALOGUE
        .iter()
        .find(|row| row.id == id)
        .unwrap_or(&PROVIDER_CATALOGUE[2])
}

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    let surface = match screen_id {
        "login"
        | "connect-provider"
        | "connect-method"
        | "connect-apikey"
        | "connect-oauth"
        | "connect-connected"
        | "connect-disconnect" => Surface::ActionDock,
        "mcp" => Surface::TemporaryDetailView,
        _ => return Err(format!("unknown Flow 4 screen: {screen_id}")),
    };
    ScreenComposition::new(screen_id, surface, statusline.clone()).validate()?;
    let origin = ProviderOrigin::from_fixture(fixture_id);
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "login" => render_login(&mut canvas, screen_id, fixture_id, origin)?,
        "connect-provider" => render_connect_provider(&mut canvas, screen_id, fixture_id, origin)?,
        "connect-method" => render_connect_method(&mut canvas, screen_id, fixture_id, origin)?,
        "connect-apikey" => render_connect_apikey(&mut canvas, screen_id, fixture_id, origin)?,
        "connect-oauth" => {
            render_connect_oauth(&mut canvas, screen_id, fixture_id, origin)?
        }
        "connect-connected" => {
            render_connect_connected(&mut canvas, screen_id, fixture_id, origin)?
        }
        "connect-disconnect" => {
            render_connect_disconnect(&mut canvas, screen_id, fixture_id, origin)?
        }
        "mcp" => render_mcp(&mut canvas, screen_id, fixture_id)?,
        _ => unreachable!(),
    };
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    let selected_id = if screen_id == "connect-method" {
        method_provider_id(fixture_id)
    } else {
        selected_provider_id(fixture_id)
    };
    let selected_row = provider_row(selected_id);
    let method_labels = selected_row.methods;
    let primary_pending = match screen_id {
        "login" => "Choose a provider".to_owned(),
        "connect-provider" => match selected_row.state {
            ProviderRowState::NotConnected => format!("Connect {}", selected_row.name),
            ProviderRowState::Connected | ProviderRowState::InUse => {
                if fixture_id.contains("disconnect") {
                    format!("Disconnect {}", selected_row.name)
                } else {
                    format!("Use {}", selected_row.name)
                }
            }
        },
        "connect-method" => {
            if method_labels.first() == Some(&"api-key") && !method_labels.contains(&"oauth") {
                "Continue with an API key".to_owned()
            } else {
                "Continue with browser sign-in".to_owned()
            }
        }
        "connect-apikey" => match api_key_validation_state(fixture_id) {
            ApiKeyValidation::Valid => "Connect".to_owned(),
            ApiKeyValidation::Checking => "Checking the key".to_owned(),
            ApiKeyValidation::Failure => "Try again".to_owned(),
            ApiKeyValidation::Empty | ApiKeyValidation::Malformed => "Enter an API key".to_owned(),
        },
        "connect-oauth" => match oauth_phase(fixture_id) {
            OAuthPhase::Consent => "Open browser".to_owned(),
            OAuthPhase::Waiting => "Waiting for authorization".to_owned(),
            OAuthPhase::Denied | OAuthPhase::Expired | OAuthPhase::StateMismatch => {
                "Try again".to_owned()
            }
            OAuthPhase::BrowserFailed => "Open browser".to_owned(),
        },
        "connect-connected" => match origin {
            ProviderOrigin::BlockedModel => "Continue".to_owned(),
            ProviderOrigin::Onboarding => "Continue".to_owned(),
            ProviderOrigin::Conversation => "Continue".to_owned(),
        },
        "connect-disconnect" => {
            let target = disconnect_target(fixture_id);
            format!("Disconnect {}", target.name)
        }
        _ => "Continue".to_owned(),
    };

    Ok(AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.into_cells(),
        candidates,
        events,
        transitions,
        work_style: None,
        model_choice: (!matches!(screen_id, "mcp")).then(|| ModelChoiceEvidence {
            applied: MODEL.to_owned(),
            pending: Some("GPT-5.2 Codex".to_owned()),
            origin: origin.short_label().to_owned(),
            pending_action: Some(primary_pending),
            return_policy: origin.return_policy(screen_id),
        }),
        conversation: Some(ConversationIdentity {
            conversation_id: if screen_id == "mcp" {
                "conversation.mcp-capabilities".to_owned()
            } else {
                "conversation.provider-connection".to_owned()
            },
            transcript_id: if screen_id == "mcp" {
                "transcript.mcp-capabilities".to_owned()
            } else {
                "transcript.provider-connection".to_owned()
            },
            session_id: if screen_id == "mcp" {
                "session.mcp-capabilities.1".to_owned()
            } else {
                "session.provider-connection.1".to_owned()
            },
            turn_id: if screen_id == "mcp" {
                "turn.mcp-capabilities.1".to_owned()
            } else {
                "turn.provider-connection.1".to_owned()
            },
            user_task: if screen_id == "mcp" {
                FLOW4_MCP_TASK.to_owned()
            } else {
                origin.user_task().to_owned()
            },
            visible_origin: if screen_id == "mcp" {
                FLOW4_MCP_ORIGIN.to_owned()
            } else {
                origin.visible_label().to_owned()
            },
            submitted_prompt: None,
            paused_activity_id: (screen_id != "mcp")
                .then(|| "pending.model.gpt-5-2-codex".to_owned()),
            later_draft: Some(match origin {
                ProviderOrigin::Onboarding => "Start with a safe model.".to_owned(),
                ProviderOrigin::Conversation => {
                    "Continue the paused conversation.".to_owned()
                }
                ProviderOrigin::BlockedModel => "Continue the paused conversation.".to_owned(),
            }),
        }),
    })
}

fn render_login(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    origin: ProviderOrigin,
) -> Result<ScreenEvidence, String> {
    for (offset, line) in ENTRY_CODE_BANNER.iter().enumerate() {
        canvas.write(2 + offset as u16, 4, line, TextStyle::accent())?;
    }
    let origin_label = origin.visible_label();
    canvas.write(10, 4, origin_label, TextStyle::muted())?;
    canvas.write(12, 4, "Connect a model provider", TextStyle::bright())?;
    canvas.write(
        14,
        4,
        "Code needs a provider before it can use the models in this project.",
        TextStyle::text(),
    )?;
    canvas.write(16, 4, origin.login_pause_copy(), TextStyle::muted())?;
    canvas.write(19, 4, origin.login_scope_copy(), TextStyle::muted())?;

    write_dock_border(canvas)?;
    dock_line(canvas, DOCK_TOP + 1, "Next step", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        match origin {
            ProviderOrigin::BlockedModel => "Choose a provider that can run GPT-5.2 Codex.",
            ProviderOrigin::Onboarding => "Choose a provider for first-run setup.",
            ProviderOrigin::Conversation => "Choose a provider for this conversation.",
        },
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Choose a provider",
        TextStyle::accent(),
    )?;
    dock_line(canvas, DOCK_TOP + 4, origin.login_cancel_hint(), TextStyle::muted())?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Cancel does not attempt a connection.",
        TextStyle::muted(),
    )?;

    let cancel_hint = origin.login_cancel_hint();
    let pairs = [
        semantic_with_refs(
            screen_id,
            fixture_id,
            "entry-brand",
            "code-ascii-banner",
            (2..=7).map(|row| reference(row, 4, 39)).collect(),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "origin",
            origin.origin_subject(),
            reference(
                10,
                4,
                if origin == ProviderOrigin::BlockedModel {
                    48
                } else {
                    4 + origin_label.chars().count() as u16
                },
            ),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "decision",
            "connect-model-provider",
            reference(12, 4, 28),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.login.choose-provider",
            reference(DOCK_TOP + 3, 4, 32),
            "focused",
            Some(action(
                "action.login.choose-provider",
                "Choose a provider",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.login.cancel",
            // Gate and blocked-model cancel labels share the historic 45-column span.
            reference(
                DOCK_TOP + 4,
                4,
                if origin == ProviderOrigin::BlockedModel {
                    45
                } else {
                    4 + cancel_hint.chars().count() as u16
                },
            ),
            "summary",
            Some(action("action.login.cancel", "Cancel", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "statusline",
            "shared-statusline",
            reference(CANVAS_ROWS - 1, 2, 38),
            "summary",
            None,
        ),
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.login.choose-provider",
                "login",
                "connect-provider",
                "direct",
                "the recorded origin opens provider selection without authenticating yet",
            ),
            transition(
                "action.login.cancel",
                "login",
                origin.login_cancel_target(),
                "cancel",
                "cancel restores the recorded origin without starting authentication",
            ),
        ],
    ))
}

fn render_connect_provider(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    origin: ProviderOrigin,
) -> Result<ScreenEvidence, String> {
    let origin_label = origin.visible_label();
    let selected_id = selected_provider_id(fixture_id);
    let selected_idx = PROVIDER_CATALOGUE
        .iter()
        .position(|row| row.id == selected_id)
        .unwrap_or(2);
    let selected = &PROVIDER_CATALOGUE[selected_idx];
    let prefer_disconnect = fixture_id.contains("disconnect");

    canvas.write(3, 4, origin_label, TextStyle::muted())?;
    let banner = origin.blocked_banner();
    canvas.write(4, 4, banner, TextStyle::bright())?;
    canvas.write(6, 4, "Choose a provider", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Each row shows whether a provider is ready, connected, or still needs sign-in.",
        TextStyle::muted(),
    )?;

    for (index, row) in PROVIDER_CATALOGUE.iter().enumerate() {
        let row_y = 11 + index as u16 * 3;
        let marker = if index == selected_idx { "›" } else { " " };
        let style = if index == selected_idx {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        canvas.write(row_y, 4, &format!("{marker} {}", row.name), style)?;
        canvas.write(
            row_y + 1,
            6,
            &format!("{} · {}", row.benefit, row.state.label()),
            TextStyle::muted(),
        )?;
    }

    let (primary_action_id, primary_label, primary_target, primary_kind, primary_path) =
        match selected.state {
            ProviderRowState::NotConnected => (
                "action.connect-provider.connect",
                format!("Connect {}", selected.name),
                "connect-method",
                "direct",
                "the chosen provider exposes its applicable methods",
            ),
            ProviderRowState::Connected | ProviderRowState::InUse if prefer_disconnect => (
                "action.connect-provider.disconnect",
                format!("Disconnect {}", selected.name),
                "connect-disconnect",
                "direct",
                "disconnect confirmation targets only the selected connected provider",
            ),
            ProviderRowState::Connected | ProviderRowState::InUse => (
                "action.connect-provider.use",
                format!("Use {}", selected.name),
                match origin {
                    ProviderOrigin::BlockedModel => "model-picker",
                    ProviderOrigin::Onboarding => "onboarding",
                    ProviderOrigin::Conversation => "chat-turn",
                },
                "direct",
                "using an already-connected provider restores the recorded origin",
            ),
        };

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!(
            "Selected · {} · {}",
            selected.name,
            selected.state.label()
        ),
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        origin.connect_consequence(),
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &format!("› [enter] {primary_label}"),
        TextStyle::accent(),
    )?;
    dock_line(canvas, DOCK_TOP + 4, "  [esc] Back", TextStyle::muted())?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Back keeps provider choice and drafts unchanged.",
        TextStyle::muted(),
    )?;

    // Gate OpenAI blocked-model catalogue uses frozen span ends so existing
    // bridge digests and RenderRef alignment stay byte-stable. Alternate
    // origin/selection fixtures compute character-accurate absolute ends.
    let gate_exact_spans = origin == ProviderOrigin::BlockedModel
        && selected_id == "openai"
        && matches!(selected.state, ProviderRowState::NotConnected);
    let origin_end = if gate_exact_spans {
        48
    } else {
        4 + origin_label.chars().count() as u16
    };
    let banner_end = if gate_exact_spans {
        55
    } else {
        4 + banner.chars().count() as u16
    };
    let historical_row_ends: [u16; 4] = [16, 15, 12, 12];

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            origin.origin_subject(),
            reference(3, 4, origin_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "blocker",
            match origin {
                ProviderOrigin::BlockedModel => "gpt-5-2-codex-waiting",
                ProviderOrigin::Onboarding => "first-run-waiting",
                ProviderOrigin::Conversation => "conversation-waiting",
            },
            reference(4, 4, banner_end),
            "focused",
            None,
        ),
    ];
    for (index, row) in PROVIDER_CATALOGUE.iter().enumerate() {
        let row_y = 11 + index as u16 * 3;
        let detail = if index == selected_idx {
            "focused"
        } else {
            "summary"
        };
        let label_end = if gate_exact_spans {
            historical_row_ends[index]
        } else {
            4 + 2 + row.name.chars().count() as u16
        };
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "provider-row",
            row.id,
            reference(row_y, 4, label_end),
            detail,
            None,
        ));
    }
    let primary_end = if gate_exact_spans {
        28
    } else {
        4 + format!("› [enter] {primary_label}")
            .chars()
            .count() as u16
    };
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "primary-action",
        primary_action_id,
        reference(DOCK_TOP + 3, 4, primary_end),
        "focused",
        Some(action(primary_action_id, &primary_label, false)),
    ));
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "cancel-action",
        "action.connect-provider.back",
        reference(DOCK_TOP + 4, 4, 15),
        "summary",
        Some(action("action.connect-provider.back", "Back", false)),
    ));
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));

    // Gate chain and production login→catalogue fixtures back one step to login.
    // Direct model-picker entry (legacy non-login detours) returns to model-picker.
    let cancel_target = if is_gate_fixture(fixture_id)
        || fixture_id.contains("from-login")
        || fixture_id.contains("auth.connect-provider")
        || matches!(
            origin,
            ProviderOrigin::Onboarding | ProviderOrigin::Conversation
        ) {
        "login"
    } else {
        "model-picker"
    };

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                primary_action_id,
                "connect-provider",
                primary_target,
                primary_kind,
                primary_path,
            ),
            transition(
                "action.connect-provider.back",
                "connect-provider",
                cancel_target,
                "cancel",
                "back restores the screen that opened provider selection without changing credentials",
            ),
        ],
    ))
}

fn render_connect_method(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    origin: ProviderOrigin,
) -> Result<ScreenEvidence, String> {
    let origin_label = origin.visible_label();
    let provider = provider_row(method_provider_id(fixture_id));
    let prefer_api_key = fixture_id.contains("api-key") || fixture_id.contains("apikey");
    let methods: Vec<(&'static str, &'static str, &'static str, &'static str)> = provider
        .methods
        .iter()
        .map(|method| match *method {
            "oauth" => (
                "oauth",
                "Browser sign-in",
                "authorize in your browser, then return here.",
                "connect-oauth",
            ),
            "api-key" => (
                "api-key",
                "Use an API key",
                "paste a key stored only on this machine.",
                "connect-apikey",
            ),
            other => (other, other, "supported sign-in method.", "connect-method"),
        })
        .collect();
    let selected_idx = if prefer_api_key {
        methods
            .iter()
            .position(|(id, _, _, _)| *id == "api-key")
            .unwrap_or(0)
    } else {
        0
    };
    let selected = methods[selected_idx];
    let title = format!("Sign in to {}", provider.name);

    canvas.write(3, 4, origin_label, TextStyle::muted())?;
    canvas.write(4, 4, &title, TextStyle::bright())?;
    canvas.write(
        6,
        4,
        "Only the methods this provider supports are shown.",
        TextStyle::muted(),
    )?;

    for (index, (id, label, consequence, _)) in methods.iter().enumerate() {
        let _ = id;
        let row = 10 + index as u16 * 4;
        let marker = if index == selected_idx { "›" } else { " " };
        let style = if index == selected_idx {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        canvas.write(row, 4, &format!("{marker} {label}"), style)?;
        canvas.write(row + 1, 6, consequence, TextStyle::muted())?;
    }

    let (primary_action_id, primary_label, primary_target) = match selected.0 {
        "oauth" => (
            "action.connect-method.oauth",
            "Continue with browser sign-in",
            "connect-oauth",
        ),
        "api-key" => (
            "action.connect-method.api-key",
            "Continue with an API key",
            "connect-apikey",
        ),
        _ => (
            "action.connect-method.continue",
            "Continue",
            selected.3,
        ),
    };
    let dock_consequence = match selected.0 {
        "oauth" => {
            "You keep control of the browser steps. Your keystroke never leaves this device."
        }
        "api-key" => "The key stays on this machine and is never echoed in full.",
        _ => "Continue with the selected method.",
    };

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!("Selected · {}", selected.1),
        TextStyle::bright(),
    )?;
    dock_line(canvas, DOCK_TOP + 2, dock_consequence, TextStyle::text())?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &format!("› [enter] {primary_label}"),
        TextStyle::accent(),
    )?;
    dock_line(canvas, DOCK_TOP + 4, "  [esc] Back", TextStyle::muted())?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        &format!("Back keeps the {} provider choice.", provider.name),
        TextStyle::muted(),
    )?;

    // Gate OpenAI browser-sign-in spans stay frozen (origin 48, provider 21,
    // browser 20, api-key 17, primary 42) for bridge digests.
    let method_gate_exact = origin == ProviderOrigin::BlockedModel
        && provider.id == "openai"
        && selected.0 == "oauth"
        && methods.len() == 2;
    let origin_end = if method_gate_exact {
        48
    } else {
        4 + origin_label.chars().count() as u16
    };
    let provider_end = if method_gate_exact {
        21
    } else {
        4 + title.chars().count() as u16
    };
    let historical_method_ends: &[( &str, u16)] = &[("oauth", 20), ("api-key", 17)];

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            origin.origin_subject(),
            reference(3, 4, origin_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider",
            provider.id,
            reference(4, 4, provider_end),
            "focused",
            None,
        ),
    ];
    for (index, (id, label, _, _)) in methods.iter().enumerate() {
        let row = 10 + index as u16 * 4;
        let detail = if index == selected_idx {
            "focused"
        } else {
            "summary"
        };
        let subject = if *id == "oauth" {
            "browser-sign-in"
        } else {
            *id
        };
        let label_end = if method_gate_exact {
            historical_method_ends
                .iter()
                .find(|(key, _)| *key == *id)
                .map(|(_, end)| *end)
                .unwrap_or(4 + 2 + label.chars().count() as u16)
        } else {
            4 + 2 + label.chars().count() as u16
        };
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "method-row",
            subject,
            reference(row, 4, label_end),
            detail,
            None,
        ));
    }
    let primary_end = if method_gate_exact {
        42
    } else {
        4 + format!("› [enter] {primary_label}")
            .chars()
            .count() as u16
    };
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "primary-action",
        primary_action_id,
        reference(DOCK_TOP + 3, 4, primary_end),
        "focused",
        Some(action(primary_action_id, primary_label, false)),
    ));
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "cancel-action",
        "action.connect-method.back",
        reference(DOCK_TOP + 4, 4, 15),
        "summary",
        Some(action("action.connect-method.back", "Back", false)),
    ));
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                primary_action_id,
                "connect-method",
                primary_target,
                "direct",
                "the provider branch opens the selected sign-in method",
            ),
            transition(
                "action.connect-method.back",
                "connect-method",
                "connect-provider",
                "cancel",
                &format!(
                    "back preserves the chosen {} provider without discarding the origin",
                    provider.name
                ),
            ),
        ],
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApiKeyValidation {
    Empty,
    Malformed,
    Checking,
    Valid,
    Failure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CredentialSource {
    Local,
    Environment,
    Managed,
}

impl CredentialSource {
    fn subject(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Environment => "environment",
            Self::Managed => "managed",
        }
    }

    fn handling_note(self) -> &'static str {
        match self {
            Self::Local => {
                "Saved in removable local storage on this machine · never echoed."
            }
            Self::Environment => {
                "Read from a non-removable environment variable · never echoed."
            }
            Self::Managed => {
                "Supplied by managed organization credentials · not removable here · never echoed."
            }
        }
    }

    fn dock_note(self) -> &'static str {
        match self {
            Self::Local => "Local key stays on this device and can be removed later.",
            Self::Environment => "Environment credentials stay set by the host; cannot remove here.",
            Self::Managed => "Managed credentials stay under organization control; cannot remove here.",
        }
    }
}

fn api_key_validation_state(fixture_id: &str) -> ApiKeyValidation {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("malformed") || lower.contains("invalid") {
        ApiKeyValidation::Malformed
    } else if lower.contains("empty") {
        ApiKeyValidation::Empty
    } else if lower.contains("checking") || lower.contains("validating") {
        ApiKeyValidation::Checking
    } else if lower.contains("failure")
        || lower.contains("failed")
        || lower.contains("rejected")
    {
        ApiKeyValidation::Failure
    } else {
        // Default published and valid fixtures are ready-to-connect.
        ApiKeyValidation::Valid
    }
}

fn api_key_credential_source(fixture_id: &str) -> CredentialSource {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("environment") || lower.contains("env-var") || lower.contains("env.")
    {
        CredentialSource::Environment
    } else if lower.contains("managed") || lower.contains("organization") {
        CredentialSource::Managed
    } else {
        // Default, local, valid, empty, malformed, checking, failure: local removable entry.
        CredentialSource::Local
    }
}

fn api_key_provider_id(fixture_id: &str) -> &'static str {
    method_provider_id(fixture_id)
}

/// Masked face: non-secret prefix + mask run + short tail. Full secret never rendered.
fn api_key_masked_field(state: ApiKeyValidation) -> &'static str {
    match state {
        ApiKeyValidation::Empty => "›                                              █",
        // Malformed is short / incomplete; still fully masked.
        ApiKeyValidation::Malformed => "sk- ········                                   █",
        ApiKeyValidation::Checking
        | ApiKeyValidation::Valid
        | ApiKeyValidation::Failure => "sk- ························ 4f2a           █",
    }
}

fn api_key_validation_copy(state: ApiKeyValidation) -> &'static str {
    match state {
        ApiKeyValidation::Empty => "Paste or type an API key to continue.",
        ApiKeyValidation::Malformed => "That entry looks incomplete. Check the key and try again.",
        ApiKeyValidation::Checking => "● Checking the key… not connected yet.",
        ApiKeyValidation::Valid => "● Looks valid · ready to connect.",
        ApiKeyValidation::Failure => {
            "Could not connect with this key. Try again with a different key."
        }
    }
}

fn render_connect_apikey(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    origin: ProviderOrigin,
) -> Result<ScreenEvidence, String> {
    let origin_label = origin.visible_label();
    let provider = provider_row(api_key_provider_id(fixture_id));
    let source = api_key_credential_source(fixture_id);
    let state = api_key_validation_state(fixture_id);
    let title = format!("Add an API key for {}", provider.name);
    let masked = api_key_masked_field(state);
    let validation = api_key_validation_copy(state);
    let handling = source.handling_note();

    canvas.write(3, 4, origin_label, TextStyle::muted())?;
    canvas.write(4, 4, &title, TextStyle::bright())?;
    canvas.write(
        6,
        4,
        "The key is never shown in full. Only this device validates it.",
        TextStyle::muted(),
    )?;
    canvas.write(
        8,
        4,
        &format!("Provider · {} · locked for this connection", provider.name),
        TextStyle::text(),
    )?;

    canvas.write(11, 4, "API key", TextStyle::muted())?;
    canvas.write(12, 4, masked, TextStyle::bright())?;
    let validation_style = match state {
        ApiKeyValidation::Valid => TextStyle {
            foreground: ColorToken::Ok,
            background: None,
            bold: false,
        },
        ApiKeyValidation::Failure | ApiKeyValidation::Malformed => TextStyle {
            foreground: ColorToken::Bad,
            background: None,
            bold: false,
        },
        ApiKeyValidation::Checking | ApiKeyValidation::Empty => TextStyle::text(),
    };
    canvas.write(14, 4, validation, validation_style)?;
    canvas.write(16, 4, handling, TextStyle::muted())?;
    canvas.write(
        18,
        4,
        "Cancel discards only what you typed. Provider and method stay selected.",
        TextStyle::muted(),
    )?;

    // Interactive dock primary: Connect only when valid-looking; Try again on
    // failure. Empty/malformed restate the requirement (not a silent Connect).
    // Checking is a non-final automatic wait with zero primary (cancel only).
    let primary: Option<(&str, &str, &str, &str, &str)> = match state {
        ApiKeyValidation::Valid => Some((
            "Connect",
            "action.connect-apikey.connect",
            "connect-connected",
            "direct",
            "valid-looking key advances to the connected confirmation after provider check",
        )),
        ApiKeyValidation::Failure => Some((
            "Try again",
            "action.connect-apikey.retry",
            "connect-apikey",
            "direct",
            "one recovery clears the failure and returns to local re-entry",
        )),
        ApiKeyValidation::Empty => Some((
            "Enter an API key",
            "action.connect-apikey.need-key",
            "connect-apikey",
            "direct",
            "empty input cannot submit a connection",
        )),
        ApiKeyValidation::Malformed => Some((
            "Fix the key",
            "action.connect-apikey.fix-key",
            "connect-apikey",
            "direct",
            "malformed input cannot submit a connection",
        )),
        ApiKeyValidation::Checking => None,
    };

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!("API key · {}", provider.name),
        TextStyle::bright(),
    )?;
    dock_line(canvas, DOCK_TOP + 2, source.dock_note(), TextStyle::text())?;
    let primary_row = match primary {
        Some((label, _, _, _, _)) => format!("› [enter] {label}"),
        None => "  Waiting · no connect action yet".to_owned(),
    };
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &primary_row,
        if matches!(
            state,
            ApiKeyValidation::Valid | ApiKeyValidation::Failure
        ) {
            TextStyle::accent()
        } else {
            TextStyle::muted()
        },
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Cancel · discard key input",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Cancel returns to method choice without saving the key.",
        TextStyle::muted(),
    )?;

    let primary_end = 4 + primary_row.chars().count() as u16;
    let masked_end = 4 + masked.chars().count() as u16;
    let validation_end = 4 + validation.chars().count() as u16;
    let handling_end = 4 + handling.chars().count() as u16;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            origin.origin_subject(),
            reference(3, 4, 4 + origin_label.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider",
            provider.id,
            reference(4, 4, 4 + title.chars().count() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "masked-field",
            "api-key-input",
            reference(12, 4, masked_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "validation-state",
            match state {
                ApiKeyValidation::Empty => "empty",
                ApiKeyValidation::Malformed => "malformed",
                ApiKeyValidation::Checking => "checking",
                ApiKeyValidation::Valid => "looks-valid",
                ApiKeyValidation::Failure => "failure",
            },
            reference(14, 4, validation_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "credential-source",
            source.subject(),
            reference(16, 4, handling_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.connect-apikey.cancel",
            reference(DOCK_TOP + 4, 4, 38),
            "summary",
            Some(action(
                "action.connect-apikey.cancel",
                "Cancel · discard key input",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "statusline",
            "shared-statusline",
            reference(CANVAS_ROWS - 1, 2, 38),
            "summary",
            None,
        ),
    ];
    if let Some((label, action_id, _, _, _)) = primary {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            action_id,
            reference(DOCK_TOP + 3, 4, primary_end),
            "focused",
            Some(action(action_id, label, false)),
        ));
    } else {
        // Automatic wait: live state is owner; no primary action.
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "live-state",
            "checking-key",
            reference(DOCK_TOP + 3, 4, primary_end),
            "focused",
            None,
        ));
    }

    let mut transitions = vec![transition(
        "action.connect-apikey.cancel",
        "connect-apikey",
        "connect-method",
        "cancel",
        "cancel discards only the secret input and restores method choice",
    )];
    if let Some((_, action_id, target, kind, path)) = primary {
        transitions.insert(
            0,
            transition(action_id, "connect-apikey", target, kind, path),
        );
    }

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

const OAUTH_USER_CODE: &str = "WDJB-MJHT";
const OAUTH_DEVICE_URL: &str = "https://auth.openai.com/device";
const OAUTH_LOOPBACK_URL: &str = "https://auth.openai.com/authorize";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OAuthFlowKind {
    Device,
    Loopback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OAuthPhase {
    Consent,
    Waiting,
    Denied,
    Expired,
    BrowserFailed,
    StateMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OAuthWaitingTick {
    Early,
    Later,
}

fn oauth_provider_id(fixture_id: &str) -> &'static str {
    method_provider_id(fixture_id)
}

fn oauth_flow_kind(fixture_id: &str) -> OAuthFlowKind {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("loopback") || lower.contains("redirect") || lower.contains("pkce") {
        OAuthFlowKind::Loopback
    } else {
        // Default published, gate, device, failure, and consent fixtures use device auth.
        OAuthFlowKind::Device
    }
}

fn oauth_phase(fixture_id: &str) -> OAuthPhase {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("consent")
        || lower.contains("before-launch")
        || lower.contains("hostile-scope")
        || lower.contains("secret-scope")
        || lower.contains("sanitized-scope")
    {
        OAuthPhase::Consent
    } else if lower.contains("denied") || lower.contains("access-denied") {
        OAuthPhase::Denied
    } else if lower.contains("expired") || lower.contains("timed-out") {
        OAuthPhase::Expired
    } else if lower.contains("browser-failed")
        || lower.contains("open-browser-failed")
        || lower.contains("browser_failed")
    {
        OAuthPhase::BrowserFailed
    } else if lower.contains("state-mismatch") || lower.contains("state_mismatch") {
        OAuthPhase::StateMismatch
    } else {
        OAuthPhase::Waiting
    }
}

fn oauth_waiting_tick(fixture_id: &str) -> OAuthWaitingTick {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("later") || lower.contains("tick-2") || lower.contains("tick.2") {
        OAuthWaitingTick::Later
    } else {
        OAuthWaitingTick::Early
    }
}

fn oauth_scope_line(fixture_id: &str) -> &'static str {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("hostile-scope") {
        // Hostile control sequences are stripped; readable scope remains.
        "Code will request: read models · run completions"
    } else if lower.contains("secret-scope") {
        // Secret-like scope values are redacted; only non-secret names may show.
        "Code will request: read models · [redacted]"
    } else {
        "Code will request: read models · run completions"
    }
}

fn render_connect_oauth(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    origin: ProviderOrigin,
) -> Result<ScreenEvidence, String> {
    let origin_label = origin.visible_label();
    let provider = provider_row(oauth_provider_id(fixture_id));
    let flow = oauth_flow_kind(fixture_id);
    let phase = oauth_phase(fixture_id);
    let tick = oauth_waiting_tick(fixture_id);
    let title = format!("Authorize {}", provider.name);
    let scope_line = oauth_scope_line(fixture_id);

    canvas.write(3, 4, origin_label, TextStyle::muted())?;
    canvas.write(4, 4, &title, TextStyle::bright())?;

    let flow_label = match flow {
        OAuthFlowKind::Device => "Device authorization · one-time browser code",
        OAuthFlowKind::Loopback => "Local browser return · no code to type",
    };
    canvas.write(6, 4, flow_label, TextStyle::muted())?;

    // Body content depends on phase; secrets (device_code, tokens, state, verifier)
    // and callback query strings never render.
    let (status_line, status_detail, status_style) = match phase {
        OAuthPhase::Consent => (
            "Review what Code will request before the browser opens.",
            scope_line,
            TextStyle::text(),
        ),
        OAuthPhase::Waiting => {
            let liveness = match (flow, tick) {
                (OAuthFlowKind::Device, OAuthWaitingTick::Early) => {
                    "Elapsed 0m 40s · code expires in 14m"
                }
                (OAuthFlowKind::Device, OAuthWaitingTick::Later) => {
                    "Elapsed 3m 20s · code expires in 11m"
                }
                (OAuthFlowKind::Loopback, OAuthWaitingTick::Early) => {
                    "Elapsed 0m 25s · local return still open"
                }
                (OAuthFlowKind::Loopback, OAuthWaitingTick::Later) => {
                    "Elapsed 2m 10s · local return still open"
                }
            };
            ("● Waiting for authorization", liveness, TextStyle::bright())
        }
        OAuthPhase::Denied => (
            "Authorization was denied in the browser.",
            "Nothing was saved. You can try again with the same provider.",
            TextStyle {
                foreground: ColorToken::Bad,
                background: None,
                bold: false,
            },
        ),
        OAuthPhase::Expired => (
            "This sign-in attempt expired.",
            "The one-time code is no longer valid. Start a new browser sign-in.",
            TextStyle {
                foreground: ColorToken::Bad,
                background: None,
                bold: false,
            },
        ),
        OAuthPhase::BrowserFailed => (
            "Could not open the browser on this machine.",
            "Open the sign-in page yourself, or try again when a browser is available.",
            TextStyle {
                foreground: ColorToken::Bad,
                background: None,
                bold: false,
            },
        ),
        OAuthPhase::StateMismatch => (
            "The browser reply did not match this sign-in attempt.",
            "Nothing was connected. Start a fresh browser sign-in to continue.",
            TextStyle {
                foreground: ColorToken::Bad,
                background: None,
                bold: false,
            },
        ),
    };

    // Actionable browser steps: device shows URL + user_code; loopback shows open page only.
    match (flow, phase) {
        (OAuthFlowKind::Device, OAuthPhase::Consent) => {
            canvas.write(
                8,
                4,
                "1  Confirm consent, then open the provider sign-in page",
                TextStyle::text(),
            )?;
            canvas.write(9, 7, OAUTH_DEVICE_URL, TextStyle::bright())?;
            canvas.write(
                11,
                4,
                "2  You will enter a one-time code on that page",
                TextStyle::text(),
            )?;
            canvas.write(13, 4, status_line, status_style)?;
            canvas.write(14, 6, status_detail, TextStyle::muted())?;
            canvas.write(
                16,
                4,
                "The provider keeps the secret device exchange. Code never shows it.",
                TextStyle::muted(),
            )?;
        }
        (OAuthFlowKind::Device, _) => {
            canvas.write(
                8,
                4,
                "1  Open this sign-in page in your browser",
                TextStyle::text(),
            )?;
            canvas.write(9, 7, OAUTH_DEVICE_URL, TextStyle::bright())?;
            canvas.write(11, 4, "2  Enter this one-time code", TextStyle::text())?;
            canvas.write(12, 7, OAUTH_USER_CODE, TextStyle::bright())?;
            canvas.write(
                13,
                7,
                "Copy this code · it is not a secret access token",
                TextStyle::muted(),
            )?;
            canvas.write(15, 4, status_line, status_style)?;
            canvas.write(16, 6, status_detail, TextStyle::muted())?;
            if matches!(phase, OAuthPhase::Waiting) {
                canvas.write(
                    18,
                    4,
                    "Code is watching for a successful browser approval.",
                    TextStyle::muted(),
                )?;
            }
        }
        (OAuthFlowKind::Loopback, OAuthPhase::Consent) => {
            canvas.write(
                8,
                4,
                "1  Confirm consent, then open this sign-in page",
                TextStyle::text(),
            )?;
            canvas.write(9, 7, OAUTH_LOOPBACK_URL, TextStyle::bright())?;
            canvas.write(
                11,
                4,
                "2  Finish in the browser · Code waits for a local return",
                TextStyle::text(),
            )?;
            canvas.write(13, 4, status_line, status_style)?;
            canvas.write(14, 6, status_detail, TextStyle::muted())?;
            canvas.write(
                16,
                4,
                "No one-time code to type. No callback details are shown here.",
                TextStyle::muted(),
            )?;
        }
        (OAuthFlowKind::Loopback, _) => {
            canvas.write(
                8,
                4,
                "1  Open this sign-in page in your browser",
                TextStyle::text(),
            )?;
            canvas.write(9, 7, OAUTH_LOOPBACK_URL, TextStyle::bright())?;
            canvas.write(
                11,
                4,
                "2  Finish authorizing · Code receives a local return",
                TextStyle::text(),
            )?;
            canvas.write(
                12,
                7,
                "No device code · no callback query is displayed",
                TextStyle::muted(),
            )?;
            canvas.write(14, 4, status_line, status_style)?;
            canvas.write(15, 6, status_detail, TextStyle::muted())?;
            if matches!(phase, OAuthPhase::Waiting) {
                canvas.write(
                    17,
                    4,
                    "A local return listener remains active until success or cancel.",
                    TextStyle::muted(),
                )?;
            }
        }
    }

    // Primary dock action: zero primary while waiting; one recovery/launch otherwise.
    let primary: Option<(&str, &str, &str, &str, &str)> = match phase {
        OAuthPhase::Consent => Some((
            "Open browser",
            "action.connect-oauth.open-browser",
            "connect-oauth",
            "direct",
            "consent is acknowledged before browser launch opens the live wait",
        )),
        OAuthPhase::Waiting => None,
        OAuthPhase::Denied | OAuthPhase::Expired | OAuthPhase::StateMismatch => Some((
            "Try again",
            "action.connect-oauth.retry",
            "connect-oauth",
            "direct",
            "one recovery starts a fresh browser sign-in for the same provider",
        )),
        OAuthPhase::BrowserFailed => Some((
            "Open browser",
            "action.connect-oauth.open-browser",
            "connect-oauth",
            "direct",
            "one recovery reopens or hand-links the sign-in page",
        )),
    };

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!("Browser sign-in · {}", provider.name),
        TextStyle::bright(),
    )?;
    let dock_note = match phase {
        OAuthPhase::Waiting => "Waiting · reopen browser stays secondary · cancel stops cleanly",
        OAuthPhase::Consent => "Consent first · nothing is stored until authorization succeeds",
        OAuthPhase::Denied | OAuthPhase::Expired | OAuthPhase::StateMismatch => {
            "One recovery restart · cancel returns to method choice"
        }
        OAuthPhase::BrowserFailed => "Open browser again · cancel returns without connecting",
    };
    dock_line(canvas, DOCK_TOP + 2, dock_note, TextStyle::text())?;
    let primary_row = match primary {
        Some((label, _, _, _, _)) => format!("› [enter] {label}"),
        None => {
            // Secondary reopen remains visible but is not the primary owner during wait.
            "  [r] Reopen browser · secondary only".to_owned()
        }
    };
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &primary_row,
        if primary.is_some() {
            TextStyle::accent()
        } else {
            TextStyle::muted()
        },
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Cancel sign-in",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Cancel invalidates this attempt and releases any local return.",
        TextStyle::muted(),
    )?;

    let status_row = match (flow, phase) {
        (OAuthFlowKind::Device, OAuthPhase::Consent) => 13u16,
        (OAuthFlowKind::Device, _) => 15,
        (OAuthFlowKind::Loopback, OAuthPhase::Consent) => 13,
        (OAuthFlowKind::Loopback, _) => 14,
    };
    let user_code_row = 12u16;
    let primary_end = 4 + primary_row.chars().count() as u16;
    let origin_end = 4 + origin_label.chars().count() as u16;
    let title_end = 4 + title.chars().count() as u16;
    let flow_end = 4 + flow_label.chars().count() as u16;
    let status_end = 4 + status_line.chars().count() as u16;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            origin.origin_subject(),
            reference(3, 4, origin_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider",
            provider.id,
            reference(4, 4, title_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "oauth-flow",
            match flow {
                OAuthFlowKind::Device => "device-authorization",
                OAuthFlowKind::Loopback => "loopback-redirect",
            },
            reference(6, 4, flow_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.connect-oauth.cancel",
            reference(DOCK_TOP + 4, 4, 28),
            "summary",
            Some(action(
                "action.connect-oauth.cancel",
                "Cancel sign-in",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "statusline",
            "shared-statusline",
            reference(CANVAS_ROWS - 1, 2, 38),
            "summary",
            None,
        ),
    ];

    match phase {
        OAuthPhase::Waiting => {
            let live_subject = match tick {
                OAuthWaitingTick::Early => "waiting-for-authorization.early",
                OAuthWaitingTick::Later => "waiting-for-authorization.later",
            };
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "live-state",
                live_subject,
                reference(status_row, 4, status_end),
                "focused",
                // Automatic success edge for ordered finish cases. Not a
                // primary-action family, so waiting keeps zero primaries.
                Some(action(
                    "action.connect-oauth.finish",
                    "Browser authorization succeeded",
                    true,
                )),
            ));
            // Secondary reopen only (not a primary-action family during wait).
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "action.connect-oauth.reopen-browser",
                reference(DOCK_TOP + 3, 4, primary_end),
                "summary",
                Some(action(
                    "action.connect-oauth.reopen-browser",
                    "Reopen browser",
                    false,
                )),
            ));
        }
        OAuthPhase::Consent | OAuthPhase::Denied | OAuthPhase::Expired
        | OAuthPhase::BrowserFailed | OAuthPhase::StateMismatch => {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "oauth-status",
                match phase {
                    OAuthPhase::Consent => "consent-before-launch",
                    OAuthPhase::Denied => "denied",
                    OAuthPhase::Expired => "expired",
                    OAuthPhase::BrowserFailed => "browser-failed",
                    OAuthPhase::StateMismatch => "state-mismatch",
                    OAuthPhase::Waiting => unreachable!(),
                },
                reference(status_row, 4, status_end),
                "focused",
                None,
            ));
        }
    }

    if matches!(flow, OAuthFlowKind::Device)
        && !matches!(phase, OAuthPhase::Consent)
    {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "one-time-code",
            "user-code",
            reference(user_code_row, 7, 7 + OAUTH_USER_CODE.chars().count() as u16),
            "focused",
            None,
        ));
    }

    if let Some((label, action_id, _, _, _)) = primary {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            action_id,
            reference(DOCK_TOP + 3, 4, primary_end),
            "focused",
            Some(action(action_id, label, false)),
        ));
    }

    if phase == OAuthPhase::Consent
        || fixture_id.to_ascii_lowercase().contains("hostile-scope")
        || fixture_id.to_ascii_lowercase().contains("secret-scope")
        || fixture_id.to_ascii_lowercase().contains("sanitized-scope")
    {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "scope-consent",
            "sanitized-scopes",
            reference(14, 6, 6 + scope_line.chars().count() as u16),
            "summary",
            None,
        ));
    }

    let mut transitions = vec![transition(
        "action.connect-oauth.cancel",
        "connect-oauth",
        "connect-method",
        "cancel",
        "cancel invalidates the waiting sign-in attempt, releases listeners, and restores method choice",
    )];
    if let Some((_, action_id, target, kind, path)) = primary {
        transitions.insert(
            0,
            transition(action_id, "connect-oauth", target, kind, path),
        );
    } else {
        // Public finish remains an ordered success edge for gate bridges; it is
        // an automatic non-primary owner, so the live waiting canvas still has
        // zero primary-action family events.
        transitions.insert(
            0,
            transition(
                "action.connect-oauth.finish",
                "connect-oauth",
                "connect-connected",
                "direct",
                "successful authentication records the connected provider",
            ),
        );
        transitions.push(transition(
            "action.connect-oauth.reopen-browser",
            "connect-oauth",
            "connect-oauth",
            "direct",
            "reopen keeps the same pending attempt and remains secondary during wait",
        ));
    }

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthMethod {
    BrowserSignIn,
    ApiKey,
}

impl AuthMethod {
    fn label(self) -> &'static str {
        match self {
            Self::BrowserSignIn => "Browser sign-in",
            Self::ApiKey => "API key",
        }
    }

    fn lower(self) -> &'static str {
        match self {
            Self::BrowserSignIn => "browser sign-in",
            Self::ApiKey => "API key",
        }
    }
}

fn connected_auth_method(fixture_id: &str) -> AuthMethod {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("api-key")
        || lower.contains("apikey")
        || lower.contains("api_key")
        || (lower.contains("key") && !lower.contains("oauth") && !lower.contains("browser"))
    {
        AuthMethod::ApiKey
    } else {
        // Gate and default production connected fixtures follow OAuth finish.
        AuthMethod::BrowserSignIn
    }
}

fn connected_provider_id(fixture_id: &str) -> &'static str {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("anthropic") {
        "anthropic"
    } else if lower.contains("google") {
        "google"
    } else if lower.contains("openrouter") {
        "openrouter"
    } else {
        "openai"
    }
}

fn connected_benefit_line(provider_id: &str, origin: ProviderOrigin) -> &'static str {
    match (provider_id, origin) {
        ("openai", _) => "GPT-5.2 Codex can use this provider now.",
        ("anthropic", _) => "Claude models can use this provider now.",
        ("google", _) => "Gemini models can use this provider now.",
        ("openrouter", _) => "OpenRouter models can use this provider now.",
        _ => "Models from this provider are ready now.",
    }
}

fn connected_return_target(origin: ProviderOrigin) -> &'static str {
    match origin {
        ProviderOrigin::BlockedModel => "model-picker",
        ProviderOrigin::Onboarding => "onboarding",
        ProviderOrigin::Conversation => "chat-turn",
    }
}

fn connected_primary_label(origin: ProviderOrigin) -> &'static str {
    // VAL-V3-F4-020 wants one Done/Continue. Destination is still origin-exact.
    match origin {
        ProviderOrigin::BlockedModel => "Continue",
        ProviderOrigin::Onboarding => "Continue",
        ProviderOrigin::Conversation => "Continue",
    }
}

fn connected_return_hint(origin: ProviderOrigin) -> &'static str {
    match origin {
        ProviderOrigin::BlockedModel => {
            "Continue resumes the blocked model action exactly once."
        }
        ProviderOrigin::Onboarding => "Continue resumes first-run setup exactly once.",
        ProviderOrigin::Conversation => {
            "Continue resumes the paused conversation exactly once."
        }
    }
}

fn connected_dock_preserve(origin: ProviderOrigin) -> &'static str {
    match origin {
        ProviderOrigin::BlockedModel => {
            "Draft and pending GPT-5.2 Codex stay exactly as left."
        }
        ProviderOrigin::Onboarding => "First-run draft and setup steps stay exactly as left.",
        ProviderOrigin::Conversation => {
            "Conversation draft and transcript stay exactly as left."
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisconnectSourcePlan {
    /// Only the named local removable credential exists.
    OnlyRemovableLocal,
    /// Local removable + environment remaining after confirm.
    RemovablePlusEnvironment,
    /// Local removable is the last effective source and active model is impacted.
    LastSourceActiveModel,
}

fn disconnect_source_plan(fixture_id: &str) -> DisconnectSourcePlan {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("last-source") || lower.contains("last_source") || lower.contains("only-source")
    {
        DisconnectSourcePlan::LastSourceActiveModel
    } else if lower.contains("multi-source")
        || lower.contains("multi_source")
        || lower.contains("environment")
        || lower.contains("env-remaining")
    {
        DisconnectSourcePlan::RemovablePlusEnvironment
    } else {
        DisconnectSourcePlan::OnlyRemovableLocal
    }
}

fn disconnect_target(fixture_id: &str) -> &'static ProviderCatalogueRow {
    let lower = fixture_id.to_ascii_lowercase();
    let id = if lower.contains("openai") {
        "openai"
    } else if lower.contains("google") {
        "google"
    } else if lower.contains("openrouter") {
        "openrouter"
    } else {
        // Default disconnect target is the catalogue Connected provider (Anthropic).
        "anthropic"
    };
    provider_row(id)
}

fn render_connect_connected(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    origin: ProviderOrigin,
) -> Result<ScreenEvidence, String> {
    let origin_label = origin.visible_label();
    let task = origin.user_task();
    let provider = provider_row(connected_provider_id(fixture_id));
    let method = connected_auth_method(fixture_id);
    let benefit = connected_benefit_line(provider.id, origin);
    let primary_label = connected_primary_label(origin);
    let return_target = connected_return_target(origin);
    let return_hint = connected_return_hint(origin);
    let dock_preserve = connected_dock_preserve(origin);

    let success_line = format!("{} is connected", provider.name);
    let method_line = format!(
        "Method · {} · ready on this device",
        method.label()
    );
    let success_subject = format!("{}-connected", provider.id);
    let outcome_subject = match provider.id {
        "openai" => "gpt-5-2-codex-unlocked",
        "anthropic" => "claude-unlocked",
        "google" => "gemini-unlocked",
        _ => "provider-unlocked",
    };
    let task_subject = match origin {
        ProviderOrigin::BlockedModel => "blocked-model-task",
        ProviderOrigin::Onboarding => "onboarding-task",
        ProviderOrigin::Conversation => "conversation-task",
    };
    let resume_line = match origin {
        ProviderOrigin::BlockedModel => {
            "Return to the blocked model action to finish applying GPT-5.2 Codex."
        }
        ProviderOrigin::Onboarding => {
            "Return to first-run setup to continue with available models."
        }
        ProviderOrigin::Conversation => {
            "Return to the conversation with this provider ready."
        }
    };
    // Keep the historic gate phrasing "Return to model choices" visible as the
    // destination consequence while the primary action itself is Continue.
    let destination_line = match origin {
        ProviderOrigin::BlockedModel => "Return to model choices",
        ProviderOrigin::Onboarding => "Return to onboarding",
        ProviderOrigin::Conversation => "Return to conversation",
    };
    let primary_row = format!("› [enter] {primary_label}");

    canvas.write(3, 4, origin_label, TextStyle::muted())?;
    canvas.write(4, 4, task, TextStyle::muted())?;
    canvas.write(6, 4, &success_line, TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "✓ Connected",
        TextStyle {
            foreground: ColorToken::Ok,
            background: None,
            bold: false,
        },
    )?;
    canvas.write(10, 4, &method_line, TextStyle::text())?;
    canvas.write(12, 4, benefit, TextStyle::text())?;
    canvas.write(14, 4, resume_line, TextStyle::muted())?;
    // In-frame ordered-storyboard honesty: capability work is never the live return.
    canvas.write(
        16,
        4,
        "Capability setup stays on its own ordered path, not this return.",
        TextStyle::muted(),
    )?;
    canvas.write(
        17,
        4,
        "The ordered storyboard later opens that separate capability branch.",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "Continue is safe to repeat · it cannot create another connection.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(canvas, DOCK_TOP + 1, "Provider ready", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        &format!("{dock_preserve} · never into MCP."),
        TextStyle::text(),
    )?;
    dock_line(canvas, DOCK_TOP + 3, &primary_row, TextStyle::accent())?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        &format!("  [esc] Close · {destination_line}"),
        TextStyle::muted(),
    )?;
    dock_line(canvas, DOCK_TOP + 5, return_hint, TextStyle::muted())?;

    let origin_end = if origin == ProviderOrigin::BlockedModel {
        48
    } else {
        4 + origin_label.chars().count() as u16
    };
    let task_end = 4 + task.chars().count() as u16;
    let success_end = 4 + success_line.chars().count() as u16;
    let method_end = 4 + method_line.chars().count() as u16;
    let benefit_end = 4 + benefit.chars().count() as u16;
    let primary_end = 4 + primary_row.chars().count() as u16;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            origin.origin_subject(),
            reference(3, 4, origin_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "task",
            task_subject,
            reference(4, 4, task_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "success",
            success_subject.as_str(),
            reference(6, 4, success_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "method",
            method.lower(),
            reference(10, 4, method_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "outcome",
            outcome_subject,
            reference(12, 4, benefit_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "bridge-note",
            "ordered-capability-branch",
            reference(16, 4, 66),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.connect-connected.return",
            reference(DOCK_TOP + 3, 4, primary_end),
            "focused",
            Some(action(
                "action.connect-connected.return",
                primary_label,
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.connect-connected.close",
            reference(DOCK_TOP + 4, 4, 16),
            "summary",
            Some(action(
                "action.connect-connected.close",
                "Close",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "statusline",
            "shared-statusline",
            reference(CANVAS_ROWS - 1, 2, 38),
            "summary",
            None,
        ),
    ];
    let mut transitions = vec![
        transition(
            "action.connect-connected.return",
            "connect-connected",
            return_target,
            "direct",
            "provider success returns to its recorded origin before any MCP action",
        ),
        transition(
            "action.connect-connected.close",
            "connect-connected",
            return_target,
            "cancel",
            "closing succeeds the same way and never routes into MCP",
        ),
    ];
    // Named ordered-storyboard detour (not a live direct edge) for gate publication honesty.
    if is_gate_fixture(fixture_id) {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "bridge-note",
            "action.storyboard.open-mcp-after-provider",
            reference(17, 4, 68),
            "summary",
            Some(action(
                "action.storyboard.open-mcp-after-provider",
                "Open MCP connections separately",
                false,
            )),
        ));
        transitions.push(transition(
            "action.storyboard.open-mcp-after-provider",
            "connect-connected",
            "mcp",
            "illustrative",
            "provider success returns to its blocked model origin before an explicit MCP action",
        ));
    }
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn render_connect_disconnect(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    origin: ProviderOrigin,
) -> Result<ScreenEvidence, String> {
    let origin_label = origin.visible_label();
    let target = disconnect_target(fixture_id);
    let plan = disconnect_source_plan(fixture_id);
    let question = format!("Disconnect {}?", target.name);
    let primary_label = format!("Disconnect {}", target.name);
    let primary_row = format!("› [enter] {primary_label}");

    let (removable_line, remaining_line, availability_line, impact_line) = match plan {
        DisconnectSourcePlan::OnlyRemovableLocal => (
            format!(
                "This will remove the removable local key for {}.",
                target.name
            ),
            "No other effective credential source for this provider remains."
                .to_owned(),
            format!(
                "After disconnect, {} will show as not connected.",
                target.name
            ),
            "Other providers and MCP connections stay unchanged.".to_owned(),
        ),
        DisconnectSourcePlan::RemovablePlusEnvironment => (
            format!(
                "This will remove the removable local key for {}.",
                target.name
            ),
            "Environment credentials remain · this provider stays available."
                .to_owned(),
            format!(
                "{} stays available from the environment credential source.",
                target.name
            ),
            "Active model is unaffected · no silent model switch.".to_owned(),
        ),
        DisconnectSourcePlan::LastSourceActiveModel => (
            format!(
                "This will remove the removable local key for {}.",
                target.name
            ),
            "No other effective credential source remains for this provider."
                .to_owned(),
            format!(
                "Models that need {} will become unavailable.",
                target.name
            ),
            "Choose a replacement model or cancel · no automatic model change."
                .to_owned(),
        ),
    };

    canvas.write(3, 4, origin_label, TextStyle::muted())?;
    canvas.write(4, 4, "Confirm disconnect", TextStyle::bright())?;
    canvas.write(6, 4, &question, TextStyle::bright())?;
    canvas.write(
        8,
        4,
        &format!(
            "{} is still connected until you confirm.",
            target.name
        ),
        TextStyle::text(),
    )?;
    canvas.write(10, 4, &removable_line, TextStyle::text())?;
    canvas.write(12, 4, &remaining_line, TextStyle::muted())?;
    canvas.write(14, 4, &availability_line, TextStyle::muted())?;
    canvas.write(16, 4, &impact_line, TextStyle::text())?;
    canvas.write(
        18,
        4,
        "Cancel keeps the credential and returns to the provider list.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!("Selected · {} · Connected", target.name),
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Disconnect removes only this named target and source.",
        TextStyle::text(),
    )?;
    dock_line(canvas, DOCK_TOP + 3, &primary_row, TextStyle::accent())?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Cancel · keep connected",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Credential remains until you confirm disconnect.",
        TextStyle::muted(),
    )?;

    let origin_end = if origin == ProviderOrigin::BlockedModel {
        48
    } else {
        4 + origin_label.chars().count() as u16
    };
    let question_end = 4 + question.chars().count() as u16;
    let removable_end = 4 + removable_line.chars().count() as u16;
    let remaining_end = 4 + remaining_line.chars().count() as u16;
    let impact_end = 4 + impact_line.chars().count() as u16;
    let primary_end = 4 + primary_row.chars().count() as u16;
    let still_end = 4
        + format!(
            "{} is still connected until you confirm.",
            target.name
        )
        .chars()
        .count() as u16;

    let target_subject = format!("provider.{}", target.id);
    let remaining_subject = match plan {
        DisconnectSourcePlan::RemovablePlusEnvironment => "environment-remaining",
        DisconnectSourcePlan::OnlyRemovableLocal
        | DisconnectSourcePlan::LastSourceActiveModel => "no-remaining-source",
    };
    let impact_subject = match plan {
        DisconnectSourcePlan::LastSourceActiveModel => "replacement-or-cancel-required",
        _ => "no-silent-switch",
    };

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            origin.origin_subject(),
            reference(3, 4, origin_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "target",
            target_subject.as_str(),
            reference(6, 4, question_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pre-confirm-state",
            "credential-still-present",
            reference(8, 4, still_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "consequence",
            "removable-local-credential",
            reference(10, 4, removable_end),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "remaining-sources",
            remaining_subject,
            reference(12, 4, remaining_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "model-impact",
            impact_subject,
            reference(16, 4, impact_end),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.connect-disconnect.confirm",
            reference(DOCK_TOP + 3, 4, primary_end),
            "focused",
            Some(action(
                "action.connect-disconnect.confirm",
                primary_label.as_str(),
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.connect-disconnect.cancel",
            reference(DOCK_TOP + 4, 4, 35),
            "summary",
            Some(action(
                "action.connect-disconnect.cancel",
                "Cancel",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "statusline",
            "shared-statusline",
            reference(CANVAS_ROWS - 1, 2, 38),
            "summary",
            None,
        ),
    ];

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.connect-disconnect.confirm",
                "connect-disconnect",
                "connect-provider",
                "direct",
                "confirm removes only the selected removable credential and recomputes availability",
            ),
            transition(
                "action.connect-disconnect.cancel",
                "connect-disconnect",
                "connect-provider",
                "cancel",
                "cancel keeps the selected provider connected and restores the catalogue",
            ),
        ],
    ))
}

// ── MCP capability catalogue, ordering, and recovery fixtures ────────────

/// Priority order for exception-first MCP listing (VAL-V3-F4-030).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum CapPriority {
    PolicyBlocked = 0,
    SignInNeeded = 1,
    SetupNeeded = 2,
    Failed = 3,
    /// Handshake succeeded but zero policy-permitted usable capabilities.
    ZeroUsable = 4,
    Retrying = 5,
    Disabled = 6,
    Ready = 7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CapState {
    PolicyBlocked,
    SignInNeeded,
    SetupNeeded,
    Failed,
    /// Successful handshake with zero policy-permitted usable capabilities.
    /// Must never render as Ready (VAL-V3-F4-057).
    ZeroUsable,
    Retrying,
    Disabled,
    Ready,
}

impl CapState {
    fn priority(self) -> CapPriority {
        match self {
            Self::PolicyBlocked => CapPriority::PolicyBlocked,
            Self::SignInNeeded => CapPriority::SignInNeeded,
            Self::SetupNeeded => CapPriority::SetupNeeded,
            Self::Failed => CapPriority::Failed,
            Self::ZeroUsable => CapPriority::ZeroUsable,
            Self::Retrying => CapPriority::Retrying,
            Self::Disabled => CapPriority::Disabled,
            Self::Ready => CapPriority::Ready,
        }
    }

    fn label(self) -> &'static str {
        match self {
            // VAL-V3-F4-052: policy-blocked capability says exactly this phrase.
            Self::PolicyBlocked => "Blocked by policy",
            Self::SignInNeeded => "Sign in needed",
            Self::SetupNeeded => "Needs setup",
            Self::Failed => "Failed",
            Self::ZeroUsable => "No usable capability",
            Self::Retrying => "Retrying",
            Self::Disabled => "Disabled",
            Self::Ready => "Ready",
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::PolicyBlocked | Self::Failed | Self::ZeroUsable => "✗",
            Self::SignInNeeded | Self::SetupNeeded => "●",
            Self::Retrying => "●",
            Self::Disabled => "○",
            Self::Ready => "○",
        }
    }

    fn primary_action(self) -> Option<&'static str> {
        match self {
            Self::PolicyBlocked => Some("Review policy"),
            Self::SignInNeeded => Some("Sign in"),
            Self::SetupNeeded => Some("Setup"),
            Self::Failed => Some("Retry"),
            Self::ZeroUsable => Some("Review policy"),
            Self::Retrying => None,
            Self::Disabled => Some("Enable"),
            Self::Ready => Some("Details"),
        }
    }

    /// Ready only means handshake succeeded and at least one usable capability
    /// remains after policy. Zero usable stays an attention state.
    fn is_healthy_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Per-connection negotiation counts, derived from stable capability identities
/// (VAL-V3-F4-057). `negotiated = allowed + blocked + unavailable`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NegotiationCounts {
    negotiated: u32,
    allowed: u32,
    blocked: u32,
    unavailable: u32,
}

impl NegotiationCounts {
    fn reconcile(self) -> bool {
        self.negotiated == self.allowed.saturating_add(self.blocked).saturating_add(self.unavailable)
    }

    fn summary_line(self) -> String {
        format!(
            "Negotiated {n} · Allowed {a} · Blocked {b} · Unavailable {u}",
            n = self.negotiated,
            a = self.allowed,
            b = self.blocked,
            u = self.unavailable
        )
    }
}

/// Derive readiness from handshake + allowed usable count.
/// Ready is impossible when allowed == 0, even after a successful handshake.
fn ready_state_for_negotiation(handshake_ok: bool, allowed_usable: u32) -> CapState {
    if !handshake_ok {
        CapState::Failed
    } else if allowed_usable == 0 {
        CapState::ZeroUsable
    } else {
        CapState::Ready
    }
}

/// Sanitize untrusted MCP labels (capability, publisher, tool, resource, prompt,
/// error, argument, response). Rejects controls, bidi, zero-width, tabs, and
/// layout hazards while keeping a stable readable identity (VAL-V3-F4-058).
fn sanitize_mcp_label(raw: &str) -> String {
    // Secret-like fragments never render; replaced with a redaction placeholder.
    let lower = raw.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("bearer ")
        || lower.contains("api_key")
        || lower.contains("authorization:")
        || lower.contains("secret=")
        || lower.contains("token=")
    {
        return "[redacted]".to_owned();
    }

    let mut out = String::with_capacity(raw.len());
    let mut previous_space = false;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let code = ch as u32;
        // Drop entire CSI / escape sequences such as ESC[31m that remain after
        // stripping the control introducer alone (would leave "[31m").
        if ch == '\u{001b}' {
            i += 1;
            if i < chars.len() && chars[i] == '[' {
                i += 1;
                while i < chars.len() {
                    let c = chars[i];
                    i += 1;
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            previous_space = false;
            continue;
        }
        // Drop C0 controls, DEL, C1 controls, tabs, and line breaks.
        if code < 0x20 || code == 0x7F || (0x80..=0x9F).contains(&code) {
            if !previous_space {
                out.push(' ');
                previous_space = true;
            }
            i += 1;
            continue;
        }
        // Bidi controls, zero-width, BOM, soft hyphen, annotation controls.
        let bidi_or_zw = matches!(
            code,
            0x200B | 0x200C | 0x200D | 0x200E | 0x200F
                | 0x2028 | 0x2029 | 0x202A | 0x202B | 0x202C | 0x202D | 0x202E
                | 0x2060 | 0x2066 | 0x2067 | 0x2068 | 0x2069
                | 0xFEFF | 0x00AD
        );
        if bidi_or_zw {
            i += 1;
            continue;
        }
        // Combining marks that create width/overstrike hazards: drop.
        if (0x0300..=0x036F).contains(&code)
            || (0x1AB0..=0x1AFF).contains(&code)
            || (0x1DC0..=0x1DFF).contains(&code)
            || (0x20D0..=0x20FF).contains(&code)
            || (0xFE20..=0xFE2F).contains(&code)
        {
            i += 1;
            continue;
        }
        // Collapse whitespace (including non-breaking spaces) so layout stays stable.
        if ch.is_whitespace() || ch == '\u{00A0}' || ch == '\u{3000}' {
            if !previous_space && !out.is_empty() {
                out.push(' ');
                previous_space = true;
            }
            i += 1;
            continue;
        }
        // Replace broad ambiguous-width / private-use / surrogate hazards.
        if (0xD800..=0xDFFF).contains(&code)
            || (0xE000..=0xF8FF).contains(&code)
            || (0xF0000..=0xFFFFD).contains(&code)
            || (0x100000..=0x10FFFD).contains(&code)
        {
            if !previous_space {
                out.push('·');
                previous_space = true;
            }
            i += 1;
            continue;
        }
        // Residual CSI-style sequences without the ESC introducer (e.g. "[31m").
        if ch == '['
            && i + 1 < chars.len()
            && chars[i + 1].is_ascii_digit()
        {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == ';') {
                j += 1;
            }
            if j < chars.len() && chars[j].is_ascii_alphabetic() {
                i = j + 1;
                continue;
            }
        }
        out.push(ch);
        previous_space = false;
        i += 1;
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        "sanitized label".to_owned()
    } else {
        // Bound width so hostile overflow cannot break the 120-col canvas.
        truncate_capability(trimmed, 64)
    }
}

#[derive(Clone, Copy, Debug)]
struct CapRow {
    id: &'static str,
    name: &'static str,
    state: CapState,
    cause: &'static str,
    publisher: &'static str,
    #[allow(dead_code)]
    access: &'static str,
    /// When present, Ready visualisation is gated by these negotiated counts.
    negotiation: Option<NegotiationCounts>,
}

impl CapRow {
    fn display_name(self) -> String {
        sanitize_mcp_label(self.name)
    }

    fn display_publisher(self) -> String {
        sanitize_mcp_label(self.publisher)
    }

    fn display_cause(self) -> String {
        sanitize_mcp_label(self.cause)
    }

    #[allow(dead_code)]
    fn display_access(self) -> String {
        sanitize_mcp_label(self.access)
    }

    /// Enforce VAL-V3-F4-057: Ready only when negotiation allows a usable capability.
    fn resolved_state(self) -> CapState {
        match self.negotiation {
            Some(counts) => {
                // Defensive: when counts exist, Ready requires allow > 0 and reconciliation.
                if self.state == CapState::Ready {
                    if counts.allowed == 0 || !counts.reconcile() {
                        CapState::ZeroUsable
                    } else {
                        CapState::Ready
                    }
                } else if self.state == CapState::ZeroUsable {
                    CapState::ZeroUsable
                } else {
                    self.state
                }
            }
            None => {
                // Without explicit negotiation evidence, treat Ready as already proven usable.
                self.state
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpViewKind {
    /// Pilot gate storyboard layout (byte-stable wording for approved frames).
    GatePilot,
    Empty,
    MixedList,
    SetupFocus,
    AuthNeeded,
    AuthReady,
    FailedRow,
    RetryRunning,
    RetryReady,
    RetryFailed,
    ProgressLive,
    ProgressSuccess,
    ProgressFailed,
    ProgressCancelled,
    FocusedDetail,
    Diagnostics,
    /// Handshake / negotiation fixtures (VAL-V3-F4-057).
    Handshake,
    /// Hostile label list and detail fixtures (VAL-V3-F4-058).
    HostileLabels,
    /// Focused policy review / confirmation (VAL-V3-F4-052).
    PolicyReview,
    PolicyChangeConfirm,
    /// Distinct disable / sign-out / remove management (VAL-V3-F4-053).
    ManageRemoval,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProgressOperation {
    Setup,
    Authentication,
    Reconnect,
    Invocation,
}

impl ProgressOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Authentication => "Authentication",
            Self::Reconnect => "Reconnect",
            Self::Invocation => "Invocation",
        }
    }

    fn cancel_action_id(self) -> &'static str {
        match self {
            Self::Setup => "action.mcp.cancel-setup",
            Self::Authentication => "action.mcp.cancel-auth",
            Self::Reconnect => "action.mcp.cancel-reconnect",
            Self::Invocation => "action.mcp.cancel-invocation",
        }
    }

    fn cancel_label(self) -> &'static str {
        match self {
            Self::Setup => "Cancel setup",
            Self::Authentication => "Cancel sign-in",
            Self::Reconnect => "Cancel reconnect",
            Self::Invocation => "Cancel invocation",
        }
    }

    fn cancel_consequence(self) -> &'static str {
        match self {
            Self::Setup => "Cancel rolls back provisional setup without adding a connection.",
            Self::Authentication => "Cancel stops authentication and leaves the capability unsigned-in.",
            Self::Reconnect => "Cancel restores the pre-reconnect state for this capability only.",
            Self::Invocation => "Cancel stops only this invocation and keeps the connection ready.",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ProgressInfo {
    operation: ProgressOperation,
    owner_id: &'static str,
    owner_name: &'static str,
    phase: &'static str,
    percent: u8,
    elapsed: &'static str,
}

#[derive(Clone, Debug)]
struct McpView {
    kind: McpViewKind,
    rows: Vec<CapRow>,
    selected_id: &'static str,
    bridge_note: bool,
    header_title: &'static str,
    header_task: &'static str,
    intro_line: &'static str,
    support_line: &'static str,
    /// Focused setup/auth body replaces the row list when present.
    focus_body: Option<&'static [&'static str]>,
    /// Optional multi-section body for focused detail / diagnostics.
    detail_sections: Option<&'static [(&'static str, &'static [&'static str])]>,
    progress: Option<ProgressInfo>,
    primary_label: &'static str,
    primary_action_id: &'static str,
    secondary_label: &'static str,
    secondary_action_id: &'static str,
    diagnostics_closed: bool,
    /// Tertiary cancel/back label shown for progress cancel or detail close.
    tertiary_label: Option<&'static str>,
    tertiary_action_id: Option<&'static str>,
}

fn mcp_view(fixture_id: &str) -> McpView {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("empty") {
        return McpView {
            kind: McpViewKind::Empty,
            rows: Vec::new(),
            selected_id: "setup-connection",
            bridge_note: false,
            header_title: FLOW4_MCP_ORIGIN,
            header_task: "Connect external tools",
            intro_line: "Connect external tools so Code can use capabilities outside this project.",
            support_line: "This path stays on capabilities and never opens model provider connection.",
            focus_body: None,
            detail_sections: None,
            progress: None,
            primary_label: "Setup connection",
            primary_action_id: "action.mcp.setup-connection",
            secondary_label: "Open diagnostics",
            secondary_action_id: "action.mcp.diagnostics",
            diagnostics_closed: true,
            tertiary_label: None,
            tertiary_action_id: None,
        };
    }
    if lower.contains("setup-browser") || lower.contains("setup_focus") {
        return McpView {
            kind: McpViewKind::SetupFocus,
            rows: vec![
                CapRow {
                    id: "browser-automation",
                    name: "Browser automation",
                    state: CapState::SetupNeeded,
                    cause: "Missing local tools needed for page checks.",
                    publisher: "Browser Tools",
                    access: "Read pages and capture screenshots",
                negotiation: None,
                },
            ],
            selected_id: "browser-automation",
            bridge_note: false,
            header_title: FLOW4_MCP_ORIGIN,
            header_task: "Finish setup for Browser automation",
            intro_line: "Guided setup for one capability. Cancel rolls back provisional state.",
            support_line: "Return stays on the MCP origin. Advanced setup is secondary only.",
            focus_body: Some(&[
                "Capability  Browser automation",
                "Publisher   Browser Tools",
                "Requested   Read pages and capture screenshots",
                "Scope       This project only",
            ]),
            detail_sections: None,
            progress: None,
            primary_label: "Connect",
            primary_action_id: "action.mcp.setup-connect",
            secondary_label: "Advanced setup",
            secondary_action_id: "action.mcp.advanced-setup",
            diagnostics_closed: true,
            tertiary_label: None,
            tertiary_action_id: None,
        };
    }
    if lower.contains("auth-docs-ready") {
        return McpView {
            kind: McpViewKind::AuthReady,
            rows: sorted_caps(&[
                CapRow {
                    id: "browser-automation",
                    name: "Browser automation",
                    state: CapState::SetupNeeded,
                    cause: "Missing local tools needed for page checks.",
                    publisher: "Browser Tools",
                    access: "Read pages and capture screenshots",
                negotiation: None,
                },
                CapRow {
                    id: "project-docs",
                    name: "Project docs",
                    state: CapState::Ready,
                    cause: "Workspace library is authorized.",
                    publisher: "Docs Workspace",
                    access: "Search project documentation",
                negotiation: None,
                },
                CapRow {
                    id: "code-search",
                    name: "Code search",
                    state: CapState::Ready,
                    cause: "Index is current.",
                    publisher: "Code Index",
                    access: "Search local symbols",
                negotiation: None,
                },
            ]),
            selected_id: "project-docs",
            bridge_note: false,
            header_title: FLOW4_MCP_ORIGIN,
            header_task: FLOW4_MCP_TASK,
            intro_line: "Project docs is ready after sign-in. Attention items still lead.",
            support_line: "Connection details and raw tools stay under diagnostics.",
            focus_body: None,
            detail_sections: None,
            progress: None,
            primary_label: "Details",
            primary_action_id: "action.mcp.details",
            secondary_label: "Open diagnostics",
            secondary_action_id: "action.mcp.diagnostics",
            diagnostics_closed: true,
            tertiary_label: None,
            tertiary_action_id: None,
        };
    }
    if lower.contains("auth-docs") {
        return McpView {
            kind: McpViewKind::AuthNeeded,
            rows: sorted_caps(&[
                CapRow {
                    id: "project-docs",
                    name: "Project docs",
                    state: CapState::SignInNeeded,
                    cause: "This workspace library needs approval before use.",
                    publisher: "Docs Workspace",
                    access: "Search project documentation",
                negotiation: None,
                },
                CapRow {
                    id: "code-search",
                    name: "Code search",
                    state: CapState::Ready,
                    cause: "Index is current.",
                    publisher: "Code Index",
                    access: "Search local symbols",
                negotiation: None,
                },
            ]),
            selected_id: "project-docs",
            bridge_note: false,
            header_title: FLOW4_MCP_ORIGIN,
            header_task: "Authorize Project docs for this conversation",
            intro_line: "One capability needs sign-in. Authorization stays inside the MCP branch.",
            support_line: "Sign in stays on this capability and never exposes tokens by default.",
            focus_body: None,
            detail_sections: None,
            progress: None,
            primary_label: "Sign in",
            primary_action_id: "action.mcp.sign-in",
            secondary_label: "Open diagnostics",
            secondary_action_id: "action.mcp.diagnostics",
            diagnostics_closed: true,
            tertiary_label: None,
            tertiary_action_id: None,
        };
    }
    if lower.contains("retry-ready") {
        return McpView {
            kind: McpViewKind::RetryReady,
            rows: sorted_caps(&[
                CapRow {
                    id: "browser-automation",
                    name: "Browser automation",
                    state: CapState::SetupNeeded,
                    cause: "Missing local tools needed for page checks.",
                    publisher: "Browser Tools",
                    access: "Read pages and capture screenshots",
                negotiation: None,
                },
                CapRow {
                    id: "issue-tracker",
                    name: "Issue tracker",
                    state: CapState::Ready,
                    cause: "Connection restored after retry.",
                    publisher: "Tracker Cloud",
                    access: "Read issues and comments",
                negotiation: None,
                },
                CapRow {
                    id: "code-search",
                    name: "Code search",
                    state: CapState::Ready,
                    cause: "Index is current.",
                    publisher: "Code Index",
                    access: "Search local symbols",
                negotiation: None,
                },
            ]),
            selected_id: "issue-tracker",
            bridge_note: false,
            header_title: FLOW4_MCP_ORIGIN,
            header_task: FLOW4_MCP_TASK,
            intro_line: "Retry restored Issue tracker. Sibling capabilities stayed unchanged.",
            support_line: "Connection details and raw tools stay under diagnostics.",
            focus_body: None,
            detail_sections: None,
            progress: None,
            primary_label: "Details",
            primary_action_id: "action.mcp.details",
            secondary_label: "Open diagnostics",
            secondary_action_id: "action.mcp.diagnostics",
            diagnostics_closed: true,
            tertiary_label: None,
            tertiary_action_id: None,
        };
    }
    if lower.contains("retry-failed") {
        return McpView {
            kind: McpViewKind::RetryFailed,
            rows: sorted_caps(&[
                CapRow {
                    id: "issue-tracker",
                    name: "Issue tracker",
                    state: CapState::Failed,
                    cause: "Still unavailable after retry. The service refused the request.",
                    publisher: "Tracker Cloud",
                    access: "Read issues and comments",
                negotiation: None,
                },
                CapRow {
                    id: "code-search",
                    name: "Code search",
                    state: CapState::Ready,
                    cause: "Index is current.",
                    publisher: "Code Index",
                    access: "Search local symbols",
                negotiation: None,
                },
            ]),
            selected_id: "issue-tracker",
            bridge_note: false,
            header_title: FLOW4_MCP_ORIGIN,
            header_task: FLOW4_MCP_TASK,
            intro_line: "Retry finished with a new plain-English failure on Issue tracker only.",
            support_line: "Connection details and raw tools stay under diagnostics.",
            focus_body: None,
            detail_sections: None,
            progress: None,
            primary_label: "Retry",
            primary_action_id: "action.mcp.retry",
            secondary_label: "Open diagnostics",
            secondary_action_id: "action.mcp.diagnostics",
            diagnostics_closed: true,
            tertiary_label: None,
            tertiary_action_id: None,
        };
    }
    if lower.contains("retry-running") || lower.contains("retrying") {
        return McpView {
            kind: McpViewKind::RetryRunning,
            rows: sorted_caps(&[
                CapRow {
                    id: "issue-tracker",
                    name: "Issue tracker",
                    state: CapState::Retrying,
                    cause: "Retrying the failed connection for this capability only.",
                    publisher: "Tracker Cloud",
                    access: "Read issues and comments",
                negotiation: None,
                },
                CapRow {
                    id: "code-search",
                    name: "Code search",
                    state: CapState::Ready,
                    cause: "Index is current.",
                    publisher: "Code Index",
                    access: "Search local symbols",
                negotiation: None,
                },
            ]),
            selected_id: "issue-tracker",
            bridge_note: false,
            header_title: FLOW4_MCP_ORIGIN,
            header_task: FLOW4_MCP_TASK,
            intro_line: "Only Issue tracker is retrying. Sibling capabilities stay put.",
            support_line: "Connection details and raw tools stay under diagnostics.",
            focus_body: None,
            detail_sections: None,
            progress: None,
            primary_label: "Wait for retry",
            primary_action_id: "action.mcp.wait-retry",
            secondary_label: "Open diagnostics",
            secondary_action_id: "action.mcp.diagnostics",
            diagnostics_closed: true,
            tertiary_label: None,
            tertiary_action_id: None,
        };
    }
    if lower.contains("failed-tracker") || lower.contains("failed_row") {
        return McpView {
            kind: McpViewKind::FailedRow,
            rows: sorted_caps(&[
                CapRow {
                    id: "issue-tracker",
                    name: "Issue tracker",
                    state: CapState::Failed,
                    cause: "Could not reach the issue service for this project.",
                    publisher: "Tracker Cloud",
                    access: "Read issues and comments",
                negotiation: None,
                },
                CapRow {
                    id: "code-search",
                    name: "Code search",
                    state: CapState::Ready,
                    cause: "Index is current.",
                    publisher: "Code Index",
                    access: "Search local symbols",
                negotiation: None,
                },
            ]),
            selected_id: "issue-tracker",
            bridge_note: false,
            header_title: FLOW4_MCP_ORIGIN,
            header_task: FLOW4_MCP_TASK,
            intro_line: "One capability failed. Retry acts only on the selected row.",
            support_line: "Connection details and raw tools stay under diagnostics.",
            focus_body: None,
            detail_sections: None,
            progress: None,
            primary_label: "Retry",
            primary_action_id: "action.mcp.retry",
            secondary_label: "Open diagnostics",
            secondary_action_id: "action.mcp.diagnostics",
            diagnostics_closed: true,
            tertiary_label: None,
            tertiary_action_id: None,
        };
    }

    // Progress / detail / diagnostics fixtures (VAL-V3-F4-035..040, 056).
    if lower.contains("progress-invocation")
        || lower.contains("progress-setup")
        || lower.contains("progress-auth")
        || lower.contains("progress-reconnect")
    {
        return progress_live_view(&lower);
    }
    if lower.contains("progress-success") {
        return progress_success_view();
    }
    if lower.contains("progress-failed") {
        return progress_failed_view();
    }
    if lower.contains("progress-cancelled") {
        return progress_cancelled_view();
    }
    if lower.contains("diagnostics-failed") {
        return diagnostics_failed_view();
    }
    if lower.contains("diagnostics-auth") {
        return diagnostics_auth_view();
    }
    if lower.contains("diagnostics-hostile") {
        return diagnostics_hostile_view();
    }
    if lower.contains("diagnostics-docs") || lower.contains("diagnostics_open") {
        return diagnostics_docs_view();
    }
    if lower.contains("detail-docs") || lower.contains("detail_docs") || lower.contains("focused-detail") {
        return focused_detail_docs_view();
    }

    // Handshake / usable negotiation (VAL-V3-F4-057).
    if lower.contains("handshake-success") || lower.contains("handshake_success") {
        return handshake_success_view();
    }
    if lower.contains("handshake-failed") || lower.contains("handshake_failed") {
        return handshake_failed_view();
    }
    if lower.contains("handshake-zero")
        || lower.contains("zero-usable")
        || lower.contains("zero_usable")
        || lower.contains("zero-capability")
        || lower.contains("zero_capability")
    {
        return handshake_zero_usable_view();
    }
    if lower.contains("handshake-progress") || lower.contains("handshake_progress") {
        // Intermediate pre-terminal handshake tick uses failed placeholder until complete.
        return handshake_failed_view();
    }
    // Hostile label sanitization (VAL-V3-F4-058).
    if lower.contains("hostile-labels")
        || lower.contains("hostile_labels")
        || lower.contains("hostile-label-list")
    {
        return hostile_labels_list_view();
    }
    if lower.contains("hostile-detail") || lower.contains("hostile_detail") {
        return hostile_labels_detail_view();
    }

    // Policy review / confirmation + removal management (VAL-V3-F4-052, 053).
    if lower.contains("policy-change-confirm")
        || lower.contains("policy_change_confirm")
        || lower.contains("policy-confirm")
    {
        return policy_change_confirm_view();
    }
    if lower.contains("policy-review")
        || lower.contains("policy_review")
        || lower.contains("policy-detail")
        || lower.contains("policy-blocked")
        || lower.contains("policy_blocked")
    {
        // Focused policy state when reviewing; list policy-blocked otherwise.
        if lower.contains("review") || lower.contains("detail") {
            return policy_review_view();
        }
        return policy_blocked_list_view();
    }
    if lower.contains("manage-removal")
        || lower.contains("manage_removal")
        || lower.contains("removal")
        || lower.contains("sign-out-manage")
    {
        return manage_removal_view();
    }

    if lower.contains("mixed") || lower.contains("views.mcp") {
        return mixed_mcp_view();
    }
    // Gate pilot + default production publication: keep approved pilot layout.
    if is_gate_fixture(fixture_id)
        || lower.contains("gate.provider-and-mcp.mcp")
        || lower == "canonical.mcp"
        || lower.is_empty()
        || lower.starts_with("case.")
        || lower.contains("provider-and-mcp")
    {
        return gate_pilot_mcp_view();
    }
    mixed_mcp_view()
}

fn gate_pilot_mcp_view() -> McpView {
    McpView {
        kind: McpViewKind::GatePilot,
        rows: sorted_caps(&[
            CapRow {
                id: "browser-automation",
                name: "Browser automation",
                state: CapState::SetupNeeded,
                cause: "Missing local tools needed for page checks.",
                publisher: "Browser Tools",
                access: "Read pages and capture screenshots",
            negotiation: None,
            },
            CapRow {
                id: "project-docs",
                name: "Project docs",
                state: CapState::SignInNeeded,
                cause: "This workspace library needs approval before use.",
                publisher: "Docs Workspace",
                access: "Search project documentation",
            negotiation: None,
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
            negotiation: None,
            },
        ]),
        selected_id: "browser-automation",
        bridge_note: true,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: FLOW4_MCP_TASK,
        intro_line: "Not a continuation of the blocked-model provider connection.",
        support_line: "Capabilities that need attention come first. Healthy ones stay collapsed.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        // Gate pilot wording retained for approved storyboard hash stability.
        primary_label: "Fix setup",
        primary_action_id: "action.mcp.fix-setup",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn policy_blocked_list_view() -> McpView {
    let mut view = mixed_mcp_view();
    view.selected_id = "deploy-hooks";
    view.intro_line = "Deploy hooks is blocked by policy. Review policy is the only recovery.";
    view.primary_label = "Review policy";
    view.primary_action_id = "action.mcp.review-policy";
    view.secondary_label = "Open diagnostics";
    view.secondary_action_id = "action.mcp.diagnostics";
    view
}

fn policy_review_view() -> McpView {
    McpView {
        kind: McpViewKind::PolicyReview,
        rows: sorted_caps(&[CapRow {
            id: "deploy-hooks",
            name: "Deploy hooks",
            state: CapState::PolicyBlocked,
            cause: "Blocked by policy",
            publisher: "Deploy Safe",
            access: "Propose deploy hooks",
            negotiation: None,
        }]),
        selected_id: "deploy-hooks",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Review the workspace policy for Deploy hooks",
        intro_line: "Blocked by policy · authentication cannot override this restriction.",
        support_line: "Retry, Sign in, Setup, and invocation stay disabled until policy changes.",
        focus_body: None,
        detail_sections: Some(&[
            (
                "Policy scope",
                &[
                    "Applies to Deploy hooks for this project only.",
                    "Remote deploy actions are refused by workspace policy.",
                ],
            ),
            (
                "Allowed next step",
                &[
                    "Review policy opens a focused decision for this row.",
                    "Any change states its scope and needs explicit confirmation.",
                ],
            ),
        ]),
        progress: None,
        primary_label: "Change policy",
        primary_action_id: "action.mcp.policy-change",
        secondary_label: "Cancel",
        secondary_action_id: "action.mcp.policy-cancel",
        diagnostics_closed: true,
        tertiary_label: Some("Keep blocked"),
        tertiary_action_id: Some("action.mcp.policy-keep-blocked"),
    }
}

fn policy_change_confirm_view() -> McpView {
    McpView {
        kind: McpViewKind::PolicyChangeConfirm,
        rows: sorted_caps(&[CapRow {
            id: "deploy-hooks",
            name: "Deploy hooks",
            state: CapState::PolicyBlocked,
            cause: "Blocked by policy",
            publisher: "Deploy Safe",
            access: "Propose deploy hooks",
            negotiation: None,
        }]),
        selected_id: "deploy-hooks",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Confirm the policy change for Deploy hooks",
        intro_line: "Confirm before policy changes. Cancel keeps the blocked row unchanged.",
        support_line: "Scope: this project · consequence: Deploy hooks may run after approval.",
        focus_body: None,
        detail_sections: Some(&[(
            "Change consequence",
            &[
                "Scope: Deploy hooks on this project only.",
                "Consequence: remote deploy actions become available.",
                "Cancel returns to the same selected MCP row with policy unchanged.",
            ],
        )]),
        progress: None,
        primary_label: "Confirm change",
        primary_action_id: "action.mcp.policy-change-confirm",
        secondary_label: "Cancel",
        secondary_action_id: "action.mcp.policy-change-cancel",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn manage_removal_view() -> McpView {
    McpView {
        kind: McpViewKind::ManageRemoval,
        rows: sorted_caps(&[
            CapRow {
                id: "project-docs",
                name: "Project docs",
                state: CapState::Ready,
                cause: "User connection with removable authorization.",
                publisher: "Docs Workspace",
                access: "Search project documentation",
                negotiation: None,
            },
            CapRow {
                id: "enterprise-knowledge",
                name: "Enterprise knowledge",
                state: CapState::Ready,
                cause: "Managed by the project · cannot be removed here.",
                publisher: "Knowledge Hub",
                access: "Search architecture notes",
                negotiation: None,
            },
        ]),
        selected_id: "project-docs",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Manage connection actions for Project docs",
        intro_line: "Disable, Sign out, and Remove connection have distinct effects.",
        support_line: "Managed project configuration is identified and not removable here.",
        focus_body: None,
        detail_sections: Some(&[
            (
                "Disable",
                &[
                    "Keeps configuration while preventing invocation.",
                    "You can enable the same connection later without re-setup.",
                ],
            ),
            (
                "Sign out",
                &[
                    "Removes only removable authorization for this connection.",
                    "Configuration remains so you can sign in again.",
                ],
            ),
            (
                "Remove connection",
                &[
                    "Deletes only user-removable connection state for Project docs.",
                    "Managed: Enterprise knowledge is managed by the project and cannot be removed.",
                ],
            ),
        ]),
        progress: None,
        primary_label: "Disable",
        primary_action_id: "action.mcp.disable",
        secondary_label: "Sign out",
        secondary_action_id: "action.mcp.sign-out",
        diagnostics_closed: true,
        tertiary_label: Some("Remove connection"),
        tertiary_action_id: Some("action.mcp.remove-connection"),
    }
}

fn mixed_mcp_view() -> McpView {
    McpView {
        kind: McpViewKind::MixedList,
        rows: sorted_caps(&[
            CapRow {
                id: "deploy-hooks",
                name: "Deploy hooks",
                state: CapState::PolicyBlocked,
                cause: "Workspace policy blocks remote deploy actions.",
                publisher: "Deploy Safe",
                access: "Propose deploy hooks",
            negotiation: None,
            },
            CapRow {
                id: "project-docs",
                name: "Project docs",
                state: CapState::SignInNeeded,
                cause: "This workspace library needs approval before use.",
                publisher: "Docs Workspace",
                access: "Search project documentation",
            negotiation: None,
            },
            CapRow {
                id: "browser-automation",
                name: "Browser automation",
                state: CapState::SetupNeeded,
                cause: "Missing local tools needed for page checks.",
                publisher: "Browser Tools",
                access: "Read pages and capture screenshots",
            negotiation: None,
            },
            CapRow {
                id: "issue-tracker",
                name: "Issue tracker",
                state: CapState::Failed,
                cause: "Could not reach the issue service for this project.",
                publisher: "Tracker Cloud",
                access: "Read issues and comments",
            negotiation: None,
            },
            CapRow {
                id: "metric-export",
                name: "Metric export",
                state: CapState::Disabled,
                cause: "Paused for this project.",
                publisher: "Metrics Lab",
                access: "Export local metric snapshots",
            negotiation: None,
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
            negotiation: None,
            },
            CapRow {
                id: "enterprise-knowledge",
                name: "Enterprise knowledge base for architecture decisions that span many teams",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Knowledge Hub",
                access: "Search architecture notes",
            negotiation: None,
            },
        ]),
        selected_id: "deploy-hooks",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: FLOW4_MCP_TASK,
        intro_line: "MCP capabilities. Attention first; healthy rows stay collapsed.",
        support_line: "Connection details and raw tools stay under diagnostics.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        primary_label: "Review policy",
        primary_action_id: "action.mcp.review-policy",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

/// Successful handshake with at least one policy-permitted capability → Ready.
fn handshake_success_view() -> McpView {
    // Per-connection counts: row-level allowed totals match ready rows after
    // aggregation so the summary and collapsed Ready block stay consistent.
    let docs_counts = NegotiationCounts {
        negotiated: 2,
        allowed: 1,
        blocked: 1,
        unavailable: 0,
    };
    let search_counts = NegotiationCounts {
        negotiated: 1,
        allowed: 1,
        blocked: 0,
        unavailable: 0,
    };
    debug_assert!(docs_counts.reconcile() && search_counts.reconcile());
    McpView {
        kind: McpViewKind::Handshake,
        rows: sorted_caps(&[
            CapRow {
                id: "project-docs",
                name: "Project docs",
                state: ready_state_for_negotiation(true, docs_counts.allowed),
                cause: "Handshake complete with usable tools permitted by policy.",
                publisher: "Docs Workspace",
                access: "Search project documentation",
                negotiation: Some(docs_counts),
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
                negotiation: Some(search_counts),
            },
        ]),
        selected_id: "project-docs",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Handshake completed with usable capabilities",
        intro_line: "Ready requires a successful handshake and at least one usable capability.",
        support_line: "Summary counts reconcile negotiated, allowed, blocked, and unavailable.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        primary_label: "Details",
        primary_action_id: "action.mcp.details",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

/// Handshake succeeded, but every negotiated capability is policy-blocked → attention.
fn handshake_zero_usable_view() -> McpView {
    let counts = NegotiationCounts {
        negotiated: 2,
        allowed: 0,
        blocked: 2,
        unavailable: 0,
    };
    debug_assert!(counts.reconcile());
    // Even if a fixture mistakenly sets Ready, resolved_state demotes it.
    let demoted = ready_state_for_negotiation(true, counts.allowed);
    debug_assert_eq!(demoted, CapState::ZeroUsable);
    McpView {
        kind: McpViewKind::Handshake,
        rows: sorted_caps(&[CapRow {
            id: "restricted-repo",
            name: "Restricted repository tools",
            // Intentionally seed Ready and force demotion via negotiation.
            state: CapState::Ready,
            cause: "Handshake succeeded, but policy permits zero usable capabilities.",
            publisher: "Repo Gate",
            access: "Policy blocked all negotiated tools",
            negotiation: Some(counts),
        }]),
        selected_id: "restricted-repo",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Handshake succeeded without usable capabilities",
        intro_line: "A successful handshake with zero permitted capabilities is not Ready.",
        support_line: "Review policy is the only primary action. Counts reconcile from stable identities.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        primary_label: "Review policy",
        primary_action_id: "action.mcp.review-policy",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

/// Failed handshake → Failed attention row, never Ready.
fn handshake_failed_view() -> McpView {
    let counts = NegotiationCounts {
        negotiated: 0,
        allowed: 0,
        blocked: 0,
        unavailable: 0,
    };
    McpView {
        kind: McpViewKind::Handshake,
        rows: sorted_caps(&[CapRow {
            id: "docs-bridge",
            name: "Docs bridge",
            state: ready_state_for_negotiation(false, 0),
            cause: "Handshake failed before any capability could be negotiated.",
            publisher: "Docs Bridge",
            access: "No negotiated capabilities",
            negotiation: Some(counts),
        }]),
        selected_id: "docs-bridge",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Handshake failed for Docs bridge",
        intro_line: "Failed handshake never reports Ready. Retry stays row-scoped.",
        support_line: "Summary counts stay at zero until a successful handshake completes.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        primary_label: "Retry",
        primary_action_id: "action.mcp.retry",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

/// Hostile MCP labels on the list surface. Sanitized for display.
fn hostile_labels_list_view() -> McpView {
    McpView {
        kind: McpViewKind::HostileLabels,
        rows: sorted_caps(&[
            CapRow {
                id: "hostile-cap",
                // Control sequences, bidi mark, zero-width, tab, overly long tail.
                name: "Docs\u{0007}\u{001b}[31m\t\u{202E}reverse\u{200B}tool\u{0301}ssssssssssssssssssssssssssssssssssssssssssssssssss",
                state: CapState::Failed,
                cause: "Publisher replied with hostile control sequences in every label.",
                publisher: "Pub\u{0000}lisher\u{202A}Bidi\u{200D}Mark\nNew",
                access: "tool: open\u{001b}file · resource: file://\u{200B}secret · prompt: pwn",
                negotiation: Some(NegotiationCounts {
                    negotiated: 3,
                    allowed: 0,
                    blocked: 1,
                    unavailable: 2,
                }),
            },
            CapRow {
                id: "secret-cap",
                name: "Vault tools sk-live-secret-token",
                state: CapState::SignInNeeded,
                cause: "Authorization plan for api_key=supersecret must never render.",
                publisher: "Secret\tHouse Bearer aquicktoken",
                access: "token=abc authorization: yes",
                negotiation: None,
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
                negotiation: Some(NegotiationCounts {
                    negotiated: 1,
                    allowed: 1,
                    blocked: 0,
                    unavailable: 0,
                }),
            },
        ]),
        selected_id: "hostile-cap",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Sanitized capability labels stay readable",
        intro_line: "Untrusted MCP labels are sanitized. Secrets and controls never render.",
        support_line: "Layout stays 120 columns. Provenance for technical labels lives under diagnostics.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        primary_label: "Retry",
        primary_action_id: "action.mcp.retry",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

/// Focused detail with sanitized hostile capability/publisher/tool/resource/
/// prompt/error/argument/response labels and provenance disclosure.
fn hostile_labels_detail_view() -> McpView {
    McpView {
        // Reuse focused-detail action anatomy so Back restores the list row.
        kind: McpViewKind::FocusedDetail,
        rows: sorted_caps(&[CapRow {
            id: "hostile-cap",
            name: "Docs\u{0007}tool\u{200B}name",
            state: CapState::Failed,
            cause: "Error: connection\u{001b}[31m refused for path sk-live-secret-token",
            publisher: "Pub\u{202E}lisher",
            access: "Open sanitized tools only",
            negotiation: Some(NegotiationCounts {
                negotiated: 4,
                allowed: 0,
                blocked: 1,
                unavailable: 3,
            }),
        }]),
        selected_id: "hostile-cap",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Capability detail · sanitized labels",
        intro_line: "Untrusted labels stay sanitized. Provenance is disclosed without secrets.",
        support_line: "Back restores the selected sanitized row identity.",
        focus_body: None,
        detail_sections: Some(&[
            (
                "What you can do",
                &[
                    "Retry this capability after labels are cleaned",
                    "Inspect sanitized diagnostics only when needed",
                ],
            ),
            (
                "Sanitized labels",
                &[
                    "Capability   Docs tool name (sanitized)",
                    "Publisher    Publisher (sanitized, provenance: server name)",
                    "Tool         open file (sanitized)",
                    "Resource     sanitized://resource",
                    "Prompt       sanitized prompt label",
                    "Error        connection refused (sanitized · redacted secrets)",
                    "Argument     sanitized object · redacted secrets",
                    "Response     sanitized · redacted secrets",
                ],
            ),
            (
                "Provenance",
                &[
                    "Source       untrusted MCP server advertisement",
                    "Policy       controls stripped · secrets redacted · width bounded",
                    "Missing      Not available",
                ],
            ),
        ]),
        progress: None,
        primary_label: "Back to list",
        primary_action_id: "action.mcp.details-back",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn progress_tick_metrics(lower: &str) -> (u8, &'static str, &'static str) {
    // Deterministic elapsed and percent for three ticks (0 < 1 < 2).
    if lower.contains("tick-2") || lower.contains("tick.2") || lower.contains("later") {
        (72, "0:18", "Writing response")
    } else if lower.contains("tick-1") || lower.contains("tick.1") || lower.contains("mid") {
        (41, "0:09", "Receiving results")
    } else {
        (12, "0:03", "Opening request")
    }
}

fn progress_live_view(lower: &str) -> McpView {
    let (operation, owner_id, owner_name, sibling_ready) = if lower.contains("progress-setup") {
        (
            ProgressOperation::Setup,
            "browser-automation",
            "Browser automation",
            false,
        )
    } else if lower.contains("progress-auth") {
        (
            ProgressOperation::Authentication,
            "project-docs",
            "Project docs",
            true,
        )
    } else if lower.contains("progress-reconnect") {
        (
            ProgressOperation::Reconnect,
            "issue-tracker",
            "Issue tracker",
            true,
        )
    } else {
        (
            ProgressOperation::Invocation,
            "issue-tracker",
            "Issue tracker",
            true,
        )
    };
    let (percent, elapsed, phase) = progress_tick_metrics(lower);
    let mut rows = vec![CapRow {
        id: owner_id,
        name: owner_name,
        state: CapState::Retrying,
        cause: "In progress - wait or cancel this operation.",
        publisher: "Capability",
        access: "Active operation",
    negotiation: None,
    }];
    if sibling_ready {
        rows.push(CapRow {
            id: "code-search",
            name: "Code search",
            state: CapState::Ready,
            cause: "Index is current.",
            publisher: "Code Index",
            access: "Search local symbols",
        negotiation: None,
        });
    }
    McpView {
        kind: McpViewKind::ProgressLive,
        rows: sorted_caps(&rows),
        selected_id: owner_id,
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: match operation {
            ProgressOperation::Setup => "Finish setup for Browser automation",
            ProgressOperation::Authentication => "Authorize Project docs for this conversation",
            ProgressOperation::Reconnect => "Reconnect Issue tracker",
            ProgressOperation::Invocation => "Read open issues with Issue tracker",
        },
        intro_line: "One capability owns this live work. Sibling rows stay unchanged.",
        support_line: operation.cancel_consequence(),
        focus_body: None,
        detail_sections: None,
        progress: Some(ProgressInfo {
            operation,
            owner_id,
            owner_name,
            phase,
            percent,
            elapsed,
        }),
        primary_label: "Wait",
        primary_action_id: "action.mcp.wait-progress",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: Some(operation.cancel_label()),
        tertiary_action_id: Some(operation.cancel_action_id()),
    }
}

fn progress_success_view() -> McpView {
    McpView {
        kind: McpViewKind::ProgressSuccess,
        rows: sorted_caps(&[
            CapRow {
                id: "issue-tracker",
                name: "Issue tracker",
                state: CapState::Ready,
                cause: "Invocation finished. Connection stayed ready.",
                publisher: "Tracker Cloud",
                access: "Read issues and comments",
            negotiation: None,
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
            negotiation: None,
            },
        ]),
        selected_id: "issue-tracker",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: FLOW4_MCP_TASK,
        intro_line: "Issue tracker finished and collapsed to one healthy line.",
        support_line: "Connection details and raw tools stay under diagnostics.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        primary_label: "Details",
        primary_action_id: "action.mcp.details",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn progress_failed_view() -> McpView {
    McpView {
        kind: McpViewKind::ProgressFailed,
        rows: sorted_caps(&[
            CapRow {
                id: "issue-tracker",
                name: "Issue tracker",
                state: CapState::Failed,
                cause: "Could not finish the request. The service closed the stream.",
                publisher: "Tracker Cloud",
                access: "Read issues and comments",
            negotiation: None,
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
            negotiation: None,
            },
        ]),
        selected_id: "issue-tracker",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: FLOW4_MCP_TASK,
        intro_line: "Invocation ended with a plain-English failure on Issue tracker only.",
        support_line: "Connection details and raw tools stay under diagnostics.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        primary_label: "Retry",
        primary_action_id: "action.mcp.retry",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn progress_cancelled_view() -> McpView {
    McpView {
        kind: McpViewKind::ProgressCancelled,
        rows: sorted_caps(&[
            CapRow {
                id: "issue-tracker",
                name: "Issue tracker",
                state: CapState::Ready,
                cause: "Invocation cancelled. Connection remains ready.",
                publisher: "Tracker Cloud",
                access: "Read issues and comments",
            negotiation: None,
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
            negotiation: None,
            },
        ]),
        selected_id: "issue-tracker",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: FLOW4_MCP_TASK,
        intro_line: "Cancel stopped only the active invocation. Connection stayed ready.",
        support_line: "Connection details and raw tools stay under diagnostics.",
        focus_body: None,
        detail_sections: None,
        progress: None,
        primary_label: "Details",
        primary_action_id: "action.mcp.details",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn focused_detail_docs_view() -> McpView {
    McpView {
        kind: McpViewKind::FocusedDetail,
        rows: sorted_caps(&[CapRow {
            id: "project-docs",
            name: "Project docs",
            state: CapState::Ready,
            cause: "Workspace library is authorized.",
            publisher: "Docs Workspace",
            access: "Search project documentation",
        negotiation: None,
        }]),
        selected_id: "project-docs",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Project docs",
        intro_line: "Focused capability detail. Raw tools stay under diagnostics.",
        support_line: "Back restores the selected list row and scroll position.",
        focus_body: None,
        detail_sections: Some(&[
            (
                "What you can do",
                &[
                    "Search project documentation by plain question",
                    "Open a matching document for reading",
                    "Summarize a section you already located",
                ],
            ),
            (
                "Recent activity",
                &[
                    "Opened architecture overview · successful",
                    "Searched auth notes · successful",
                    "Listed section titles · successful",
                ],
            ),
            (
                "Enabled tools · by purpose",
                &[
                    "Search documents - find project answers",
                    "Open document - read a selected page",
                    "List sections - scan document structure",
                ],
            ),
        ]),
        progress: None,
        primary_label: "Back to list",
        primary_action_id: "action.mcp.details-back",
        secondary_label: "Open diagnostics",
        secondary_action_id: "action.mcp.diagnostics",
        diagnostics_closed: true,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn diagnostics_docs_view() -> McpView {
    McpView {
        kind: McpViewKind::Diagnostics,
        rows: sorted_caps(&[CapRow {
            id: "project-docs",
            name: "Project docs",
            state: CapState::Ready,
            cause: "Workspace library is authorized.",
            publisher: "Docs Workspace",
            access: "Search project documentation",
        negotiation: None,
        }]),
        selected_id: "project-docs",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Project docs · diagnostics",
        intro_line: "Advanced technical record. Secrets stay redacted.",
        support_line: "Close diagnostics returns to the same focused detail.",
        focus_body: None,
        detail_sections: Some(&[
            (
                "Capability state",
                &[
                    "State          Ready",
                    "Publisher      Docs Workspace",
                    "Access         Search project documentation",
                ],
            ),
            (
                "Technical record",
                &[
                    "Transport      stream (sanitized label)",
                    "Endpoint       redacted project path",
                    "Timeout        30 seconds",
                    "Config source  project managed file",
                    "Scopes         read:docs (sanitized, untrusted label)",
                    "Raw tools      docs_search_query · docs_open_page",
                    "Resources      docs://workspace/index",
                    "Prompts        docs_summarize_section",
                    "Schema         tools/list payload · sanitized",
                    "Arguments      {q: \"auth\"} · sanitized",
                    "Responses      last rank list · redacted tokens",
                    "Secret fields  [redacted]",
                    "Missing field  Not available",
                ],
            ),
        ]),
        progress: None,
        primary_label: "Back to detail",
        primary_action_id: "action.mcp.diagnostics-close",
        secondary_label: "Collapse evidence",
        secondary_action_id: "action.mcp.diagnostics-collapse",
        diagnostics_closed: false,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn diagnostics_hostile_view() -> McpView {
    McpView {
        kind: McpViewKind::Diagnostics,
        rows: sorted_caps(&[CapRow {
            id: "project-docs",
            name: "Project docs",
            state: CapState::Ready,
            cause: "Workspace library is authorized.",
            publisher: "Docs Workspace",
            access: "Search project documentation",
        negotiation: None,
        }]),
        selected_id: "project-docs",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Project docs · diagnostics",
        intro_line: "Hostile labels and secret scopes are sanitized with provenance.",
        support_line: "Close diagnostics returns to the same focused detail.",
        focus_body: None,
        detail_sections: Some(&[
            (
                "Capability state",
                &["State          Ready", "Publisher      Docs Workspace (sanitized)"],
            ),
            (
                "Technical record",
                &[
                    "Transport      stream (sanitized label)",
                    "Endpoint       redacted project path",
                    "Timeout        30 seconds",
                    "Config source  project managed file",
                    "Scopes         untrusted scope name · sanitized · provenance disclosed",
                    "Raw tools      docs_search_query",
                    "Resources      sanitized://resource",
                    "Prompts        sanitized prompt label",
                    "Schema         Not available",
                    "Arguments      sanitized object · redacted secrets",
                    "Responses      sanitized · redacted secrets",
                    "Secret fields  [redacted]",
                    "Missing field  Not available",
                ],
            ),
        ]),
        progress: None,
        primary_label: "Back to detail",
        primary_action_id: "action.mcp.diagnostics-close",
        secondary_label: "Collapse evidence",
        secondary_action_id: "action.mcp.diagnostics-collapse",
        diagnostics_closed: false,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn diagnostics_failed_view() -> McpView {
    McpView {
        kind: McpViewKind::Diagnostics,
        rows: sorted_caps(&[
            CapRow {
                id: "issue-tracker",
                name: "Issue tracker",
                state: CapState::Failed,
                cause: "Could not reach the issue service for this project.",
                publisher: "Tracker Cloud",
                access: "Read issues and comments",
            negotiation: None,
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
            negotiation: None,
            },
        ]),
        selected_id: "issue-tracker",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Issue tracker · diagnostics",
        intro_line: "Readable cause and Retry stay primary. Technical evidence is secondary.",
        support_line: "Collapse returns to the same failed row and recovery action.",
        focus_body: None,
        detail_sections: Some(&[
            (
                "Recovery",
                &[
                    "Could not reach the issue service for this project.",
                    "Retry acts only on Issue tracker.",
                ],
            ),
            (
                "Technical record",
                &[
                    "Transport      stream (sanitized label)",
                    "Endpoint       redacted project path",
                    "Timeout        20 seconds",
                    "Config source  project managed file",
                    "Scopes         Not available",
                    "Raw tools      issues_list · issues_get",
                    "Resources      Not available",
                    "Prompts        Not available",
                    "Schema         tools/list payload · sanitized",
                    "Arguments      sanitized · redacted secrets",
                    "Responses      connection refused · sanitized",
                    "Secret fields  [redacted]",
                ],
            ),
        ]),
        progress: None,
        primary_label: "Retry",
        primary_action_id: "action.mcp.retry",
        secondary_label: "Collapse evidence",
        secondary_action_id: "action.mcp.diagnostics-collapse",
        diagnostics_closed: false,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn diagnostics_auth_view() -> McpView {
    McpView {
        kind: McpViewKind::Diagnostics,
        rows: sorted_caps(&[
            CapRow {
                id: "project-docs",
                name: "Project docs",
                state: CapState::SignInNeeded,
                cause: "This workspace library needs approval before use.",
                publisher: "Docs Workspace",
                access: "Search project documentation",
            negotiation: None,
            },
            CapRow {
                id: "code-search",
                name: "Code search",
                state: CapState::Ready,
                cause: "Index is current.",
                publisher: "Code Index",
                access: "Search local symbols",
            negotiation: None,
            },
        ]),
        selected_id: "project-docs",
        bridge_note: false,
        header_title: FLOW4_MCP_ORIGIN,
        header_task: "Project docs · diagnostics",
        intro_line: "Readable cause and Sign in stay primary. Technical evidence is secondary.",
        support_line: "Collapse returns to the same auth-needed row and recovery action.",
        focus_body: None,
        detail_sections: Some(&[
            (
                "Recovery",
                &[
                    "This workspace library needs approval before use.",
                    "Sign in stays on Project docs and never exposes tokens by default.",
                ],
            ),
            (
                "Technical record",
                &[
                    "Transport      stream (sanitized label)",
                    "Endpoint       redacted project path",
                    "Timeout        30 seconds",
                    "Config source  project managed file",
                    "Scopes         read:docs (sanitized, untrusted label)",
                    "Raw tools      docs_search_query",
                    "Resources      docs://workspace/index",
                    "Prompts        Not available",
                    "Schema         Not available",
                    "Arguments      Not available",
                    "Responses      unauthorized · sanitized",
                    "Secret fields  [redacted]",
                ],
            ),
        ]),
        progress: None,
        primary_label: "Sign in",
        primary_action_id: "action.mcp.sign-in",
        secondary_label: "Collapse evidence",
        secondary_action_id: "action.mcp.diagnostics-collapse",
        diagnostics_closed: false,
        tertiary_label: None,
        tertiary_action_id: None,
    }
}

fn sorted_caps(rows: &[CapRow]) -> Vec<CapRow> {
    let mut out = rows.to_vec();
    out.sort_by(|a, b| {
        a.state
            .priority()
            .cmp(&b.state.priority())
            .then_with(|| a.id.cmp(b.id))
    });
    out
}

fn render_mcp(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let view = mcp_view(fixture_id);
    if view.kind == McpViewKind::GatePilot {
        return render_mcp_gate_pilot(canvas, screen_id, fixture_id);
    }

    let mut pairs: Vec<(SemanticCandidate, SemanticEvent)> = Vec::new();
    let mut transitions: Vec<TransitionTrace> = Vec::new();

    canvas.write(2, 4, view.header_title, TextStyle::muted())?;
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "origin",
        "mcp-origin",
        reference(2, 4, view.header_title.len() as u16),
        "summary",
        None,
    ));

    canvas.write(3, 4, view.header_task, TextStyle::bright())?;
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "task",
        "mcp-task",
        reference(3, 4, view.header_task.len() as u16),
        "focused",
        None,
    ));

    let mut row_cursor = 5u16;
    if view.bridge_note {
        canvas.write(
            row_cursor,
            4,
            "Illustrative detour · separate capability branch",
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "bridge-note",
            "illustrative-detour",
            reference(row_cursor, 4, 50),
            "summary",
            None,
        ));
        row_cursor += 2;
    }

    if view.kind == McpViewKind::Empty {
        // Task line already carries "Connect external tools"; body explains MCP once.
        canvas.write(row_cursor, 4, "No capabilities configured yet", TextStyle::bright())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "surface",
            "mcp-empty",
            reference(row_cursor, 4, 32),
            "summary",
            None,
        ));
        row_cursor += 2;
        canvas.write(row_cursor, 4, view.intro_line, TextStyle::text())?;
        row_cursor += 1;
        canvas.write(
            row_cursor,
            4,
            "Use tools, resources, and prompts from outside this project when you need them.",
            TextStyle::muted(),
        )?;
        row_cursor += 2;
        canvas.write(row_cursor, 4, view.support_line, TextStyle::muted())?;
    } else if let Some(progress) = view.progress {
        canvas.write(row_cursor, 4, "Live MCP work", TextStyle::bright())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "surface",
            "mcp-progress",
            reference(row_cursor, 4, 14),
            "summary",
            None,
        ));
        row_cursor += 2;
        canvas.write(row_cursor, 4, view.intro_line, TextStyle::muted())?;
        row_cursor += 2;

        let owner_line = format!(
            "› ● {} · {} in progress",
            progress.owner_name,
            progress.operation.label()
        );
        canvas.write(row_cursor, 4, &owner_line, TextStyle::accent())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "progress-owner",
            progress.owner_id,
            reference(row_cursor, 4, owner_line.chars().count().min(80) as u16),
            "focused",
            None,
        ));
        row_cursor += 1;

        let op_line = format!(
            "Operation  {} · Phase  {}",
            progress.operation.label(),
            progress.phase
        );
        canvas.write(row_cursor, 6, &op_line, TextStyle::bright())?;
        row_cursor += 1;

        let filled = (progress.percent as usize * 20) / 100;
        let empty = 20usize.saturating_sub(filled);
        let bar = format!(
            "Progress   {}{}  {}%",
            "█".repeat(filled),
            "░".repeat(empty),
            progress.percent
        );
        canvas.write(row_cursor, 6, &bar, TextStyle::text())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "progress-metric",
            progress.owner_id,
            reference(row_cursor, 6, bar.chars().count().min(40) as u16),
            "focused",
            None,
        ));
        row_cursor += 1;

        let elapsed_line = format!("Elapsed    {}", progress.elapsed);
        canvas.write(row_cursor, 6, &elapsed_line, TextStyle::text())?;
        row_cursor += 1;

        canvas.write(
            row_cursor,
            6,
            "You can wait, or Cancel this operation below.",
            TextStyle::muted(),
        )?;
        row_cursor += 2;
        canvas.write(row_cursor, 4, view.support_line, TextStyle::muted())?;
        row_cursor += 2;

        // Sibling ready rows stay collapsed and non-owner.
        for cap in view.rows.iter().filter(|r| r.id != progress.owner_id) {
            if row_cursor >= 28 {
                break;
            }
            let state = cap.resolved_state();
            let line = format!("  ○ {} · {}", cap.display_name(), state.label());
            canvas.write(row_cursor, 4, &line, TextStyle::muted())?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "capability-row",
                cap.id,
                reference(row_cursor, 4, line.chars().count().min(80) as u16),
                "summary",
                None,
            ));
            row_cursor += 1;
        }
    } else if let Some(sections) = view.detail_sections {
        let heading = if view.kind == McpViewKind::Diagnostics {
            "Diagnostics"
        } else {
            "Capability detail"
        };
        canvas.write(row_cursor, 4, heading, TextStyle::bright())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "surface",
            if view.kind == McpViewKind::Diagnostics {
                "mcp-diagnostics"
            } else {
                "mcp-focused-detail"
            },
            reference(row_cursor, 4, heading.len() as u16),
            "summary",
            None,
        ));
        row_cursor += 1;
        canvas.write(row_cursor, 4, view.intro_line, TextStyle::muted())?;
        row_cursor += 2;

        // Capability name is the focused heading context (always sanitized).
        if let Some(cap) = view.rows.iter().find(|r| r.id == view.selected_id) {
            let state = cap.resolved_state();
            let title = format!("{} · {}", cap.display_name(), state.label());
            canvas.write(row_cursor, 4, &title, TextStyle::bright())?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "capability-row",
                cap.id,
                reference(row_cursor, 4, title.chars().count().min(80) as u16),
                "focused",
                None,
            ));
            // Identity-stable publisher string for audit and cross-surface checks.
            let publisher = format!("Publisher · {}", cap.display_publisher());
            if row_cursor + 1 < 28 {
                canvas.write(row_cursor + 1, 4, &publisher, TextStyle::muted())?;
            }
            row_cursor += 2;
        }

        for (section_title, lines) in sections {
            if row_cursor >= 28 {
                break;
            }
            canvas.write(row_cursor, 4, section_title, TextStyle::bright())?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "detail-section",
                section_title,
                reference(row_cursor, 4, section_title.len() as u16),
                if view.kind == McpViewKind::Diagnostics {
                    "evidence"
                } else {
                    "summary"
                },
                None,
            ));
            row_cursor += 1;
            for line in *lines {
                if row_cursor >= 28 {
                    break;
                }
                canvas.write(row_cursor, 6, line, TextStyle::text())?;
                row_cursor += 1;
            }
            row_cursor += 1;
        }
        if row_cursor < 29 {
            canvas.write(row_cursor, 4, view.support_line, TextStyle::muted())?;
        }
    } else if let Some(body) = view.focus_body {
        canvas.write(row_cursor, 4, "Guided setup", TextStyle::bright())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "surface",
            "mcp-setup-focus",
            reference(row_cursor, 4, 14),
            "summary",
            None,
        ));
        row_cursor += 2;
        canvas.write(row_cursor, 4, view.intro_line, TextStyle::muted())?;
        row_cursor += 2;
        for line in body {
            canvas.write(row_cursor, 4, line, TextStyle::text())?;
            row_cursor += 1;
        }
        row_cursor += 1;
        canvas.write(row_cursor, 4, view.support_line, TextStyle::muted())?;
        row_cursor += 2;
        canvas.write(
            row_cursor,
            4,
            "Cancel returns to the same selected row without adding a connection.",
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(row_cursor, 4, "MCP capabilities", TextStyle::bright())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "surface",
            "mcp-capabilities",
            reference(row_cursor, 4, 20),
            "summary",
            None,
        ));
        row_cursor += 1;
        canvas.write(row_cursor, 4, view.intro_line, TextStyle::muted())?;
        row_cursor += 1;
        canvas.write(row_cursor, 4, view.support_line, TextStyle::muted())?;
        row_cursor += 2;

        let attention: Vec<&CapRow> = view
            .rows
            .iter()
            .filter(|r| {
                let state = r.resolved_state();
                !state.is_healthy_ready() && state != CapState::Disabled
            })
            .collect();
        let disabled: Vec<&CapRow> = view
            .rows
            .iter()
            .filter(|r| r.resolved_state() == CapState::Disabled)
            .collect();
        let ready: Vec<&CapRow> = view
            .rows
            .iter()
            .filter(|r| r.resolved_state().is_healthy_ready())
            .collect();

        // Aggregate negotiated/allowed/blocked/unavailable from the same row
        // identities so summary counts reconcile (VAL-V3-F4-057).
        let mut negotiated = 0u32;
        let mut allowed = 0u32;
        let mut blocked = 0u32;
        let mut unavailable = 0u32;
        for cap in &view.rows {
            if let Some(counts) = cap.negotiation {
                negotiated = negotiated.saturating_add(counts.negotiated);
                allowed = allowed.saturating_add(counts.allowed);
                blocked = blocked.saturating_add(counts.blocked);
                unavailable = unavailable.saturating_add(counts.unavailable);
            } else {
                // Each listed capability is one negotiated identity when no
                // micro-capability breakdown is provided.
                negotiated = negotiated.saturating_add(1);
                match cap.resolved_state() {
                    CapState::Ready => allowed = allowed.saturating_add(1),
                    CapState::PolicyBlocked | CapState::ZeroUsable => {
                        blocked = blocked.saturating_add(1)
                    }
                    CapState::Failed
                    | CapState::SignInNeeded
                    | CapState::SetupNeeded
                    | CapState::Retrying
                    | CapState::Disabled => unavailable = unavailable.saturating_add(1),
                }
            }
        }
        let count_summary = NegotiationCounts {
            negotiated,
            allowed,
            blocked,
            unavailable,
        };

        // Leave rows 29-34 for footer/actions; stop list content at 28.
        const LIST_FLOOR: u16 = 28;
        if row_cursor < LIST_FLOOR {
            let counts_line = count_summary.summary_line();
            canvas.write(row_cursor, 4, &counts_line, TextStyle::muted())?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "negotiation-counts",
                "mcp-negotiation-counts",
                reference(row_cursor, 4, counts_line.chars().count().min(90) as u16),
                "summary",
                None,
            ));
            // Identity-stable audit authors the counted totals for tests.
            // RenderRef end column must remain exclusive and past the start col.
            let count_end = (4 + counts_line.chars().count() as u16).clamp(8, 118);
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "negotiation-allowed",
                &format!("allowed-{allowed}"),
                reference(row_cursor, 4, count_end),
                "summary",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "negotiation-blocked",
                &format!("blocked-{blocked}"),
                reference(row_cursor, 4, count_end),
                "summary",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "negotiation-unavailable",
                &format!("unavailable-{unavailable}"),
                reference(row_cursor, 4, count_end),
                "summary",
                None,
            ));
            row_cursor += 1;
        }

        for cap in attention {
            if row_cursor + 2 > LIST_FLOOR {
                break;
            }
            let state = cap.resolved_state();
            // Hard safe-guard: never paint Ready when resolved state is attention.
            debug_assert!(
                !state.is_healthy_ready(),
                "attention list must never include Ready rows"
            );
            let selected = cap.id == view.selected_id;
            let prefix = if selected { "› " } else { "  " };
            let name = cap.display_name();
            let title = format!(
                "{prefix}{} {} · {}",
                state.marker(),
                name,
                state.label()
            );
            let style = if selected {
                TextStyle::accent()
            } else {
                TextStyle::text()
            };
            canvas.write(row_cursor, 4, &title, style)?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "capability-row",
                cap.id,
                reference(row_cursor, 4, title.chars().count().min(80) as u16),
                if selected { "focused" } else { "summary" },
                None,
            ));
            row_cursor += 1;
            // Dense attention packing: cause + action on one muted line when possible.
            let cause = cap.display_cause();
            if let Some(action_label) = state.primary_action() {
                let action_bit = if selected {
                    format!("[enter] {action_label}")
                } else {
                    format!("[{action_label}]")
                };
                let detail = if cause.is_empty() {
                    action_bit
                } else {
                    format!("{} · {action_bit}", truncate_capability(&cause, 52))
                };
                canvas.write(
                    row_cursor,
                    6,
                    &detail,
                    if selected {
                        TextStyle::bright()
                    } else {
                        TextStyle::muted()
                    },
                )?;
                row_cursor += 1;
            } else if !cause.is_empty() {
                canvas.write(
                    row_cursor,
                    6,
                    &truncate_capability(&cause, 70),
                    TextStyle::muted(),
                )?;
                row_cursor += 1;
            }
            // Surface per-row negotiation only for zero-usable / handshake attention,
            // so list summary counts stay singular and reconcilable.
            if matches!(cap.resolved_state(), CapState::ZeroUsable) {
                if let Some(counts) = cap.negotiation {
                    if row_cursor < LIST_FLOOR {
                    let line = format!("    {}", counts.summary_line());
                    canvas.write(row_cursor, 6, &line, TextStyle::muted())?;
                    row_cursor += 1;
                    }
                }
            }
        }

        for cap in disabled {
            if row_cursor >= LIST_FLOOR {
                break;
            }
            let state = cap.resolved_state();
            let name = cap.display_name();
            let line = format!("  {} {} · {}", state.marker(), name, state.label());
            canvas.write(row_cursor, 4, &line, TextStyle::muted())?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "capability-row",
                cap.id,
                reference(row_cursor, 4, line.chars().count().min(80) as u16),
                "summary",
                None,
            ));
            row_cursor += 1;
        }

        if !ready.is_empty() && row_cursor < LIST_FLOOR {
            let summary = format!(
                "  ○ {} ready connection{} collapsed",
                ready.len(),
                if ready.len() == 1 { "" } else { "s" }
            );
            canvas.write(row_cursor, 4, &summary, TextStyle::muted())?;
            row_cursor += 1;
            for cap in ready {
                if row_cursor >= LIST_FLOOR {
                    break;
                }
                let state = cap.resolved_state();
                // VAL-V3-F4-057: never label Ready when zero usable capabilities.
                debug_assert!(
                    state.is_healthy_ready(),
                    "Ready summary only lists policy-permitted usable capabilities"
                );
                // Healthy capability occupies exactly one compact line.
                let name = truncate_capability(&cap.display_name(), 48);
                let line = format!("    {name} · {}", state.label());
                canvas.write(row_cursor, 4, &line, TextStyle::text())?;
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "capability-row",
                    if cap.id == "code-search" {
                        "code-search-ready"
                    } else {
                        cap.id
                    },
                    reference(row_cursor, 4, line.chars().count().min(80) as u16),
                    "summary",
                    None,
                ));
                row_cursor += 1;
            }
        }
    }

    // Footer notes sit just above the action region without colliding with rows.
    let footer_row = 29u16;
    if view.diagnostics_closed {
        canvas.write(
            footer_row,
            4,
            "Connection details and raw tools stay under diagnostics.",
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "disclosure",
            "diagnostics_closed",
            reference(footer_row, 4, 56),
            "summary",
            None,
        ));
    } else {
        canvas.write(
            footer_row,
            4,
            "Diagnostics open · technical evidence is secondary to recovery.",
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "disclosure",
            "diagnostics_open",
            reference(footer_row, 4, 58),
            "evidence",
            None,
        ));
    }
    canvas.write(
        30,
        4,
        "Return closes this branch to the conversation, not provider OAuth.",
        TextStyle::muted(),
    )?;

    // Action region.
    let primary_line = format!("› [enter] {}", view.primary_label);
    canvas.write(31, 4, &primary_line, TextStyle::accent())?;
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "primary-action",
        view.primary_action_id,
        reference(31, 4, primary_line.chars().count() as u16),
        "focused",
        Some(action(
            view.primary_action_id,
            view.primary_label,
            false,
        )),
    ));

    if view.kind == McpViewKind::SetupFocus {
        canvas.write(
            32,
            4,
            &format!("  [a] {}", view.secondary_label),
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            view.secondary_action_id,
            reference(32, 4, 22),
            "summary",
            Some(action(
                view.secondary_action_id,
                view.secondary_label,
                false,
            )),
        ));
        canvas.write(33, 4, "  [esc] Cancel", TextStyle::muted())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.mcp.setup-cancel",
            reference(33, 4, 18),
            "summary",
            Some(action("action.mcp.setup-cancel", "Cancel", false)),
        ));
        transitions.push(transition(
            view.primary_action_id,
            "mcp",
            "mcp",
            "direct",
            "guided setup connects the focused capability without provisional leftovers on cancel",
        ));
        transitions.push(transition(
            "action.mcp.setup-cancel",
            "mcp",
            "mcp",
            "cancel",
            "cancel rolls back provisional setup and restores the selected row",
        ));
    } else if view.kind == McpViewKind::ProgressLive {
        canvas.write(
            32,
            4,
            &format!("  [d] {}", view.secondary_label),
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            view.secondary_action_id,
            reference(32, 4, 25),
            "summary",
            Some(action(
                view.secondary_action_id,
                view.secondary_label,
                false,
            )),
        ));
        let cancel_id = view
            .tertiary_action_id
            .unwrap_or("action.mcp.cancel-invocation");
        let cancel_label = view.tertiary_label.unwrap_or("Cancel");
        canvas.write(
            33,
            4,
            &format!("  [esc] {cancel_label}"),
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            cancel_id,
            reference(33, 4, 18 + cancel_label.len() as u16),
            "summary",
            Some(action(cancel_id, cancel_label, false)),
        ));
        transitions.push(transition(
            view.primary_action_id,
            "mcp",
            "mcp",
            "direct",
            "live progress remains owned by one capability while the user waits",
        ));
        transitions.push(transition(
            cancel_id,
            "mcp",
            "mcp",
            "cancel",
            "cancel matches the operation and rolls back only that live work",
        ));
    } else if view.kind == McpViewKind::FocusedDetail {
        canvas.write(
            32,
            4,
            &format!("  [d] {}", view.secondary_label),
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            view.secondary_action_id,
            reference(32, 4, 25),
            "summary",
            Some(action(
                view.secondary_action_id,
                view.secondary_label,
                false,
            )),
        ));
        // Primary already carries Back; avoid a second identical action identity.
        canvas.write(33, 4, "  [esc] Return to conversation", TextStyle::muted())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.mcp.return",
            reference(33, 4, 33),
            "summary",
            Some(action(
                "action.mcp.return",
                "Return to conversation",
                false,
            )),
        ));
        transitions.push(transition(
            view.primary_action_id,
            "mcp",
            "mcp",
            "direct",
            "Back restores the selected capability row and scroll position",
        ));
        transitions.push(transition(
            "action.mcp.diagnostics",
            "mcp",
            "mcp",
            "direct",
            "Open diagnostics discloses technical evidence for the same capability",
        ));
        transitions.push(transition(
            "action.mcp.return",
            "mcp",
            "chat-turn",
            "direct",
            "the capability view closes to its recorded conversation origin",
        ));
    } else if matches!(
        view.kind,
        McpViewKind::PolicyReview | McpViewKind::PolicyChangeConfirm
    ) {
        canvas.write(
            32,
            4,
            &format!("  [c] {}", view.secondary_label),
            TextStyle::muted(),
        )?;
        // Secondary cancel uses a distinct identity from the esc cancel so
        // action bijection stays unique while both route to keep-blocked.
        let secondary_cancel_id = if view.kind == McpViewKind::PolicyReview {
            "action.mcp.policy-cancel"
        } else {
            "action.mcp.policy-change-cancel"
        };
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            secondary_cancel_id,
            reference(32, 4, 18),
            "summary",
            Some(action(secondary_cancel_id, view.secondary_label, false)),
        ));
        canvas.write(33, 4, "  [esc] Keep blocked", TextStyle::muted())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.mcp.policy-keep-blocked",
            reference(33, 4, 24),
                "summary",
            Some(action(
                "action.mcp.policy-keep-blocked",
                "Keep blocked",
                false,
            )),
        ));
        transitions.push(transition(
            view.primary_action_id,
            "mcp",
            "mcp",
            "direct",
            if view.kind == McpViewKind::PolicyReview {
                "Review policy opens a focused decision for the selected blocked row"
            } else {
                "Confirm applies the disclosed policy mutation only for the selected row"
            },
        ));
        transitions.push(transition(
            secondary_cancel_id,
            "mcp",
            "mcp"
            ,
            "cancel",
            "cancel returns to the same selected MCP row with policy and origin unchanged",
        ));
        transitions.push(transition(
            "action.mcp.policy-keep-blocked",
            "mcp",
            "mcp",
            "cancel",
            "keep blocked leaves policy, connection, origin, and selection unchanged",
        ));
    } else if view.kind == McpViewKind::ManageRemoval {
        canvas.write(
            32,
            4,
            &format!("  [s] {}", view.secondary_label),
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            view.secondary_action_id,
            reference(32, 4, 16),
            "summary",
            Some(action(
                view.secondary_action_id,
                view.secondary_label,
                false,
            )),
        ));
        let tertiary_id = view
            .tertiary_action_id
            .unwrap_or("action.mcp.remove-connection");
        let tertiary_label = view.tertiary_label.unwrap_or("Remove connection");
        canvas.write(
            33,
            4,
            &format!("  [r] {tertiary_label}"),
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "tertiary-action",
            tertiary_id,
            reference(33, 4, 10 + tertiary_label.len() as u16),
            "summary",
            Some(action(tertiary_id, tertiary_label, false)),
        ));
        canvas.write(
            34,
            4,
            "  Managed project rows cannot use Remove connection",
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "managed-safeguard",
            "action.mcp.remove-managed-refused",
            reference(34, 4, 54),
            "summary",
            Some(action(
                "action.mcp.remove-managed-refused",
                "Managed cannot remove",
                false,
            )),
        ));
        // Primary enable uses disable on this manage surface.
        // Note: primary line already published action.mcp.disable from primary_action_id.
        transitions.push(transition(
            "action.mcp.disable",
            "mcp",
            "mcp",
            "direct",
            "Disable retains configuration while preventing invocation",
        ));
        transitions.push(transition(
            "action.mcp.sign-out",
            "mcp",
            "mcp",
            "direct",
            "Sign out removes only removable authorization",
        ));
        transitions.push(transition(
            "action.mcp.remove-connection",
            "mcp",
            "mcp",
            "direct",
            "Remove connection deletes only user-removable connection state",
        ));
        transitions.push(transition(
            "action.mcp.remove-managed-refused",
            "mcp",
            "mcp",
            "direct",
            "Managed project configuration is identified and cannot be removed",
        ));
    } else if view.kind == McpViewKind::Diagnostics {
        // Recovery diagnostics keep Retry/Sign in primary; close is secondary only.
        // Healthy diagnostics keep "Back to detail" as primary/close without a
        // duplicate action identity on the cancel row.
        if matches!(view.primary_action_id, "action.mcp.retry" | "action.mcp.sign-in") {
            canvas.write(
                32,
                4,
                &format!("  [d] {}", view.secondary_label),
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                view.secondary_action_id,
                reference(32, 4, 28),
                "summary",
                Some(action(
                    view.secondary_action_id,
                    view.secondary_label,
                    false,
                )),
            ));
            canvas.write(33, 4, "  [esc] Close diagnostics", TextStyle::muted())?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.mcp.diagnostics-close",
                reference(33, 4, 28),
                "summary",
                Some(action(
                    "action.mcp.diagnostics-close",
                    "Close diagnostics",
                    false,
                )),
            ));
            transitions.push(transition(
                view.primary_action_id,
                "mcp",
                "mcp",
                "direct",
                if view.primary_action_id == "action.mcp.retry" {
                    "retry remains the primary recovery action while diagnostics are open"
                } else {
                    "sign-in remains the primary recovery action while diagnostics are open"
                },
            ));
            transitions.push(transition(
                "action.mcp.diagnostics-close",
                "mcp",
                "mcp",
                "cancel",
                "closing diagnostics restores the same recovery row",
            ));
        } else {
            canvas.write(
                32,
                4,
                &format!("  [d] {}", view.secondary_label),
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                view.secondary_action_id,
                reference(32, 4, 28),
                "summary",
                Some(action(
                    view.secondary_action_id,
                    view.secondary_label,
                    false,
                )),
            ));
            canvas.write(33, 4, "  [esc] Return to conversation", TextStyle::muted())?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.mcp.return",
                reference(33, 4, 33),
                "summary",
                Some(action(
                    "action.mcp.return",
                    "Return to conversation",
                    false,
                )),
            ));
            transitions.push(transition(
                view.primary_action_id,
                "mcp",
                "mcp",
                "direct",
                "closing diagnostics restores the same focused capability detail",
            ));
            transitions.push(transition(
                "action.mcp.return",
                "mcp",
                "chat-turn",
                "direct",
                "the capability view closes to its recorded conversation origin",
            ));
        }
    } else {
        canvas.write(
            32,
            4,
            &format!("  [d] {}", view.secondary_label),
            TextStyle::muted(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            view.secondary_action_id,
            reference(32, 4, 25),
            "summary",
            Some(action(
                view.secondary_action_id,
                view.secondary_label,
                false,
            )),
        ));
        canvas.write(33, 4, "  [esc] Return to conversation", TextStyle::muted())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.mcp.return",
            reference(33, 4, 33),
            "summary",
            Some(action(
                "action.mcp.return",
                "Return to conversation",
                false,
            )),
        ));
        transitions.push(transition(
            view.primary_action_id,
            "mcp",
            "mcp",
            "direct",
            match view.kind {
                McpViewKind::Empty => {
                    "setup connection stays on the MCP branch and never opens provider login"
                }
                McpViewKind::AuthNeeded => {
                    "sign-in authorizes only the selected capability inside the MCP branch"
                }
                McpViewKind::FailedRow
                | McpViewKind::RetryFailed
                | McpViewKind::ProgressFailed => {
                    "retry is scoped to the failed capability and leaves siblings unchanged"
                }
                McpViewKind::RetryRunning => {
                    "retry remains owned by the selected capability while it runs"
                }
                McpViewKind::ProgressSuccess | McpViewKind::ProgressCancelled => {
                    "terminal progress states show one healthy or prior usable action"
                }
                McpViewKind::Handshake => {
                    "Ready is allowed only after a usable policy-permitted capability exists"
                }
                McpViewKind::HostileLabels => {
                    "untrusted MCP labels stay sanitized across the selected recovery action"
                }
                _ => "the selected capability action stays row-scoped on the MCP branch",
            },
        ));
        transitions.push(transition(
            "action.mcp.return",
            "mcp",
            "chat-turn",
            "direct",
            "the capability view closes to its recorded conversation origin",
        ));
    }

    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));

    let _ = view.selected_id;

    Ok((
        pairs.iter().map(|(c, _)| c.clone()).collect(),
        pairs.iter().map(|(_, e)| e.clone()).collect(),
        transitions,
    ))
}

fn truncate_capability(name: &str, max_chars: usize) -> String {
    let count = name.chars().count();
    if count <= max_chars {
        return name.to_owned();
    }
    let take = max_chars.saturating_sub(1);
    let mut out: String = name.chars().take(take).collect();
    out.push('…');
    out
}

/// Byte-stable gate pilot layout (approved Flow 4 storyboard frame).
fn render_mcp_gate_pilot(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, FLOW4_MCP_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, FLOW4_MCP_TASK, TextStyle::bright())?;
    canvas.write(
        5,
        4,
        "Illustrative detour · separate capability branch",
        TextStyle::muted(),
    )?;
    canvas.write(7, 4, "MCP capabilities", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Not a continuation of the blocked-model provider connection.",
        TextStyle::muted(),
    )?;
    canvas.write(
        9,
        4,
        "Capabilities that need attention come first. Healthy ones stay collapsed.",
        TextStyle::muted(),
    )?;

    // Attention-first ordering: setup, sign-in, then health summary.
    canvas.write(12, 4, "› Browser automation · Needs setup", TextStyle::accent())?;
    canvas.write(
        13,
        6,
        "Missing local tools needed for page checks.",
        TextStyle::muted(),
    )?;
    canvas.write(14, 6, "[enter] Fix setup", TextStyle::bright())?;

    canvas.write(17, 4, "  Project docs · Sign in needed", TextStyle::text())?;
    canvas.write(
        18,
        6,
        "This workspace library needs approval before use.",
        TextStyle::muted(),
    )?;
    canvas.write(19, 6, "[enter] Sign in", TextStyle::bright())?;

    canvas.write(22, 4, "  ○ 1 ready connection collapsed", TextStyle::muted())?;
    canvas.write(23, 6, "Code search · Ready", TextStyle::text())?;

    canvas.write(
        26,
        4,
        "Connection details and raw tools stay under diagnostics.",
        TextStyle::muted(),
    )?;
    canvas.write(
        27,
        4,
        "Return closes this branch to the conversation, not provider OAuth.",
        TextStyle::muted(),
    )?;
    canvas.write(30, 4, "› [enter] Fix setup", TextStyle::accent())?;
    canvas.write(31, 4, "  [d] Open diagnostics", TextStyle::muted())?;
    canvas.write(32, 4, "  [esc] Return to conversation", TextStyle::muted())?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "mcp-origin",
            reference(2, 4, FLOW4_MCP_ORIGIN.len() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "task",
            "mcp-task",
            reference(3, 4, FLOW4_MCP_TASK.len() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "bridge-note",
            "illustrative-detour",
            reference(5, 4, 50),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "surface",
            "mcp-capabilities",
            reference(7, 4, 20),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "capability-row",
            "browser-automation",
            reference(12, 4, 38),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "capability-row",
            "project-docs",
            reference(17, 4, 34),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "capability-row",
            "code-search-ready",
            reference(23, 6, 24),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.mcp.fix-setup",
            reference(30, 4, 22),
            "focused",
            Some(action("action.mcp.fix-setup", "Fix setup", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.mcp.diagnostics",
            reference(31, 4, 25),
            "summary",
            Some(action("action.mcp.diagnostics", "Open diagnostics", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.mcp.return",
            reference(32, 4, 33),
            "summary",
            Some(action("action.mcp.return", "Return to conversation", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "statusline",
            "shared-statusline",
            reference(CANVAS_ROWS - 1, 2, 38),
            "summary",
            None,
        ),
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.mcp.fix-setup",
                "mcp",
                "mcp",
                "direct",
                "guided setup stays on the selected capability until it resolves",
            ),
            transition(
                "action.mcp.return",
                "mcp",
                "chat-turn",
                "direct",
                "the capability view closes to its recorded conversation origin",
            ),
        ],
    ))
}

fn is_gate_fixture(fixture_id: &str) -> bool {
    fixture_id.starts_with("gate.provider-and-mcp.")
        || fixture_id.starts_with("flow4.provider-and-mcp.")
}

fn write_dock_border(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(
        DOCK_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    canvas.write(
        DOCK_TOP + 6,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    Ok(())
}

fn dock_line(canvas: &mut Canvas, row: u16, text: &str, style: TextStyle) -> Result<(), String> {
    canvas.write(row, 2, "│", TextStyle::muted())?;
    canvas.write(row, 4, text, style)?;
    canvas.write(row, 117, "│", TextStyle::muted())?;
    Ok(())
}

fn action(action_id: &str, label: &str, automatic: bool) -> ActionEvidence {
    ActionEvidence {
        action_id: action_id.to_owned(),
        label: label.to_owned(),
        enabled: true,
        automatic,
    }
}

fn reference(row: u16, start: u16, end: u16) -> RenderRef {
    RenderRef { row, start, end }
}

fn semantic(
    screen_id: &str,
    fixture_id: &str,
    family: &str,
    subject_id: &str,
    render_ref: RenderRef,
    detail_level: &str,
    action: Option<ActionEvidence>,
) -> (SemanticCandidate, SemanticEvent) {
    semantic_with_refs(
        screen_id,
        fixture_id,
        family,
        subject_id,
        vec![render_ref],
        detail_level,
        action,
    )
}

fn semantic_with_refs(
    screen_id: &str,
    fixture_id: &str,
    family: &str,
    subject_id: &str,
    render_refs: Vec<RenderRef>,
    detail_level: &str,
    action: Option<ActionEvidence>,
) -> (SemanticCandidate, SemanticEvent) {
    let identity = Identity {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        candidate_id: format!("candidate.{family}.{subject_id}"),
        event_id: format!("event.{family}.{subject_id}"),
        family: family.to_owned(),
        subject_id: subject_id.to_owned(),
        render_refs,
        detail_level: detail_level.to_owned(),
    };
    (
        SemanticCandidate {
            identity: identity.clone(),
        },
        SemanticEvent { identity, action },
    )
}

fn transition(
    action_id: &str,
    from_screen_id: &str,
    to_screen_id: &str,
    kind: &str,
    path: &str,
) -> TransitionTrace {
    TransitionTrace {
        action_id: action_id.to_owned(),
        from_screen_id: from_screen_id.to_owned(),
        to_screen_id: to_screen_id.to_owned(),
        kind: kind.to_owned(),
        path: path.to_owned(),
    }
}
