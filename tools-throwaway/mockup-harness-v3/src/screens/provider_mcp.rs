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
        layout::{CANVAS_ROWS, ScreenComposition, StatusLine},
        tokens::{ColorToken, TextStyle},
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Sonnet 4.5";
const DOCK_TOP: u16 = 31;
const FLOW4_PROVIDER_TASK: &str = "Connect the model needed for this conversation.";
const FLOW4_PROVIDER_ORIGIN: &str = "Provider connection · blocked model action";
const FLOW4_MCP_TASK: &str = "Fix the capabilities this conversation needs.";
const FLOW4_MCP_ORIGIN: &str = "MCP · conversation capabilities";
const CODE_BANNER: [&str; 6] = [
    " ██████              ██",
    "██                   ██",
    "██        ████    █████   ████",
    "██       ██  ██  ██  ██  ██████",
    "██       ██  ██  ██  ██  ██",
    " ██████   ████    █████   █████",
];

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    let surface = match screen_id {
        "login" | "connect-provider" | "connect-method" | "connect-oauth" | "connect-connected" => {
            Surface::ActionDock
        }
        "mcp" => Surface::TemporaryDetailView,
        _ => return Err(format!("unknown Flow 4 screen: {screen_id}")),
    };
    ScreenComposition::new(screen_id, surface, statusline.clone()).validate()?;
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "login" => render_login(&mut canvas, screen_id, fixture_id)?,
        "connect-provider" => render_connect_provider(&mut canvas, screen_id, fixture_id)?,
        "connect-method" => render_connect_method(&mut canvas, screen_id, fixture_id)?,
        "connect-oauth" => render_connect_oauth(&mut canvas, screen_id, fixture_id)?,
        "connect-connected" => render_connect_connected(&mut canvas, screen_id, fixture_id)?,
        "mcp" => render_mcp(&mut canvas, screen_id, fixture_id)?,
        _ => unreachable!(),
    };
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;
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
            origin: if is_gate_fixture(fixture_id) {
                "Blocked model action".to_owned()
            } else {
                "Current conversation".to_owned()
            },
            pending_action: Some(match screen_id {
                "login" => "Choose a provider".to_owned(),
                "connect-provider" => "Connect OpenAI".to_owned(),
                "connect-method" => "Continue with browser sign-in".to_owned(),
                "connect-oauth" => "I've authorized".to_owned(),
                "connect-connected" => "Return to model choices".to_owned(),
                _ => "Continue".to_owned(),
            }),
            return_policy: if screen_id == "connect-connected" {
                "Return to model choices".to_owned()
            } else {
                "Return to blocked model action".to_owned()
            },
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
                FLOW4_PROVIDER_TASK.to_owned()
            },
            visible_origin: if screen_id == "mcp" {
                FLOW4_MCP_ORIGIN.to_owned()
            } else {
                FLOW4_PROVIDER_ORIGIN.to_owned()
            },
            submitted_prompt: None,
            paused_activity_id: (screen_id != "mcp")
                .then(|| "pending.model.gpt-5-2-codex".to_owned()),
            later_draft: Some("Continue the paused conversation.".to_owned()),
        }),
    })
}

fn render_login(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    for (offset, line) in CODE_BANNER.iter().enumerate() {
        canvas.write(2 + offset as u16, 4, line, TextStyle::accent())?;
    }
    canvas.write(10, 4, FLOW4_PROVIDER_ORIGIN, TextStyle::muted())?;
    canvas.write(12, 4, "Connect a model provider", TextStyle::bright())?;
    canvas.write(
        14,
        4,
        "Code needs a provider before it can use the models in this project.",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        "Your blocked model choice and conversation stay paused while you connect.",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "This path only unlocks the model provider for the paused choice.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(canvas, DOCK_TOP + 1, "Next step", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Choose a provider that can run GPT-5.2 Codex.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Choose a provider",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Cancel · return to model choices",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Cancel does not attempt a connection.",
        TextStyle::muted(),
    )?;

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
            "blocked-model-origin",
            reference(10, 4, 48),
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
            reference(DOCK_TOP + 4, 4, 45),
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
                "the blocked model action opens provider selection",
            ),
            transition(
                "action.login.cancel",
                "login",
                "model-picker",
                "cancel",
                "cancel restores the blocked model origin without starting authentication",
            ),
        ],
    ))
}

fn render_connect_provider(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(3, 4, FLOW4_PROVIDER_ORIGIN, TextStyle::muted())?;
    canvas.write(
        4,
        4,
        "GPT-5.2 Codex is waiting for a provider connection.",
        TextStyle::bright(),
    )?;
    canvas.write(6, 4, "Choose a provider", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Each row shows whether a provider is ready, connected, or still needs sign-in.",
        TextStyle::muted(),
    )?;

    let rows = [
        ("OpenRouter", "Unified access to many models", "Connected · in use"),
        ("Anthropic", "Claude models for everyday coding", "Connected"),
        ("OpenAI", "GPT-5.2 Codex and related models", "Not connected"),
        ("Google", "Gemini models for broad tasks", "Not connected"),
    ];
    let selected = 2usize;
    for (index, (name, benefit, state)) in rows.iter().enumerate() {
        let row = 11 + index as u16 * 3;
        let marker = if index == selected { "›" } else { " " };
        let style = if index == selected {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        canvas.write(row, 4, &format!("{marker} {name}"), style)?;
        canvas.write(row + 1, 6, &format!("{benefit} · {state}"), TextStyle::muted())?;
    }

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Selected · OpenAI · Not connected",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Connecting unlocks GPT-5.2 Codex for the paused model choice.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Connect OpenAI",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Back",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Back keeps provider choice and drafts unchanged.",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "blocked-model-origin",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "blocker",
            "gpt-5-2-codex-waiting",
            reference(4, 4, 55),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider-row",
            "openrouter",
            reference(11, 4, 16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider-row",
            "anthropic",
            reference(14, 4, 15),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider-row",
            "openai",
            reference(17, 4, 12),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider-row",
            "google",
            reference(20, 4, 12),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.connect-provider.connect",
            reference(DOCK_TOP + 3, 4, 28),
            "focused",
            Some(action(
                "action.connect-provider.connect",
                "Connect OpenAI",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.connect-provider.back",
            reference(DOCK_TOP + 4, 4, 15),
            "summary",
            Some(action("action.connect-provider.back", "Back", false)),
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

    let cancel_target = if is_gate_fixture(fixture_id) {
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
                "action.connect-provider.connect",
                "connect-provider",
                "connect-method",
                "direct",
                "the chosen provider exposes its applicable methods",
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
) -> Result<ScreenEvidence, String> {
    canvas.write(3, 4, FLOW4_PROVIDER_ORIGIN, TextStyle::muted())?;
    canvas.write(4, 4, "Sign in to OpenAI", TextStyle::bright())?;
    canvas.write(
        6,
        4,
        "Only the methods this provider supports are shown.",
        TextStyle::muted(),
    )?;

    canvas.write(10, 4, "› Browser sign-in", TextStyle::accent())?;
    canvas.write(
        11,
        6,
        "authorize in your browser, then return here.",
        TextStyle::muted(),
    )?;
    canvas.write(14, 4, "  Use an API key", TextStyle::text())?;
    canvas.write(
        15,
        6,
        "paste a key stored only on this machine.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Selected · Browser sign-in",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "You keep control of the browser steps. Your keystroke never leaves this device.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Continue with browser sign-in",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Back",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Back keeps the OpenAI provider choice.",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "blocked-model-origin",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider",
            "openai",
            reference(4, 4, 21),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "method-row",
            "browser-sign-in",
            reference(10, 4, 20),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "method-row",
            "api-key",
            reference(14, 4, 17),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.connect-method.oauth",
            reference(DOCK_TOP + 3, 4, 42),
            "focused",
            Some(action(
                "action.connect-method.oauth",
                "Continue with browser sign-in",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.connect-method.back",
            reference(DOCK_TOP + 4, 4, 15),
            "summary",
            Some(action("action.connect-method.back", "Back", false)),
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
                "action.connect-method.oauth",
                "connect-method",
                "connect-oauth",
                "direct",
                "the provider branch opens the selected sign-in method",
            ),
            transition(
                "action.connect-method.back",
                "connect-method",
                "connect-provider",
                "cancel",
                "back preserves the chosen OpenAI provider without discarding the origin",
            ),
        ],
    ))
}

fn render_connect_oauth(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(3, 4, FLOW4_PROVIDER_ORIGIN, TextStyle::muted())?;
    canvas.write(4, 4, "Authorize OpenAI", TextStyle::bright())?;
    canvas.write(
        6,
        4,
        "Finish these browser steps, then return to Code.",
        TextStyle::muted(),
    )?;

    canvas.write(9, 4, "1  Open this sign-in page in your browser", TextStyle::text())?;
    canvas.write(10, 7, "https://auth.openai.com/device", TextStyle::bright())?;
    canvas.write(12, 4, "2  Enter this one-time code", TextStyle::text())?;
    canvas.write(13, 7, "WDJB-MJHT", TextStyle::bright())?;
    canvas.write(14, 7, "Code expires in 14m", TextStyle::muted())?;
    canvas.write(17, 4, "● Waiting for authorization", TextStyle::bright())?;
    canvas.write(
        18,
        6,
        "Code is watching for a successful browser approval.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Browser sign-in in progress",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Cancel stops the wait and does not connect the provider.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] I've authorized",
        TextStyle::accent(),
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
        "Nothing is stored until authorization succeeds.",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "blocked-model-origin",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "provider",
            "openai",
            reference(4, 4, 20),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "live-state",
            "waiting-for-authorization",
            reference(17, 4, 31),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "one-time-code",
            "user-code",
            reference(13, 7, 16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.connect-oauth.finish",
            reference(DOCK_TOP + 3, 4, 28),
            "focused",
            Some(action(
                "action.connect-oauth.finish",
                "I've authorized",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.connect-oauth.cancel",
            reference(DOCK_TOP + 4, 4, 26),
            "summary",
            Some(action("action.connect-oauth.cancel", "Cancel sign-in", false)),
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
                "action.connect-oauth.finish",
                "connect-oauth",
                "connect-connected",
                "direct",
                "successful authentication records the connected provider",
            ),
            transition(
                "action.connect-oauth.cancel",
                "connect-oauth",
                "connect-method",
                "cancel",
                "cancel invalidates the waiting sign-in attempt without connecting",
            ),
        ],
    ))
}

fn render_connect_connected(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(3, 4, FLOW4_PROVIDER_ORIGIN, TextStyle::muted())?;
    canvas.write(4, 4, FLOW4_PROVIDER_TASK, TextStyle::muted())?;
    canvas.write(6, 4, "OpenAI is connected", TextStyle::bright())?;
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
    canvas.write(
        10,
        4,
        "GPT-5.2 Codex can use this provider now.",
        TextStyle::text(),
    )?;
    canvas.write(
        12,
        4,
        "Return to the blocked model action to finish applying GPT-5.2 Codex.",
        TextStyle::muted(),
    )?;
    // In-frame ordered-storyboard honesty: capability work is never the live return.
    canvas.write(
        15,
        4,
        "Capability setup stays on its own ordered path, not this return.",
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        "The ordered storyboard later opens that separate capability branch.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Provider ready",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Your conversation draft and pending GPT-5.2 Codex remain exactly as left.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Return to model choices",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Close",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Success returns to the blocked model origin, never into MCP.",
        TextStyle::muted(),
    )?;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "blocked-model-origin",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "task",
            "blocked-model-task",
            reference(4, 4, FLOW4_PROVIDER_TASK.len() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "success",
            "openai-connected",
            reference(6, 4, 23),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "outcome",
            "gpt-5-2-codex-unlocked",
            reference(10, 4, 44),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "bridge-note",
            "ordered-capability-branch",
            reference(15, 4, 66),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.connect-connected.return",
            reference(DOCK_TOP + 3, 4, 38),
            "focused",
            Some(action(
                "action.connect-connected.return",
                "Return to model choices",
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
            "model-picker",
            "direct",
            "provider success returns to its blocked model origin before any MCP action",
        ),
        transition(
            "action.connect-connected.close",
            "connect-connected",
            "model-picker",
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
            reference(16, 4, 68),
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

fn render_mcp(
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
