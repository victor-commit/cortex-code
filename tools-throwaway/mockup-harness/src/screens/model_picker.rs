//! Model picker dock with provider groups and neutral selection treatment.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, ListRow, StatusLine};

const MODELS: &[ListRow] = &[
    ListRow::Section("Anthropic"),
    ListRow::Item {
        name: "Claude Opus 4.8",
        detail: "slow · best quality · ● current",
        meta: "200K ctx · $15/M",
    },
    ListRow::Item {
        name: "Claude Sonnet 4.6",
        detail: "fast · strong coding",
        meta: "200K ctx · $3/M",
    },
    ListRow::Item {
        name: "Claude Haiku 4.5",
        detail: "fast · low cost",
        meta: "200K ctx · $1/M",
    },
    ListRow::Item {
        name: "Claude Sonnet 4.5",
        detail: "balanced · coding",
        meta: "200K ctx · $3/M",
    },
    ListRow::Item {
        name: "Claude Opus 4.6",
        detail: "slow · deep reasoning",
        meta: "200K ctx · $15/M",
    },
    ListRow::Section("OpenAI"),
    ListRow::Item {
        name: "GPT-5",
        detail: "balanced · reasoning",
        meta: "400K ctx · $5/M",
    },
    ListRow::Item {
        name: "GPT-5 Codex",
        detail: "fast · coding",
        meta: "400K ctx · $4/M",
    },
    ListRow::Item {
        name: "GPT-4.1",
        detail: "fast · reliable",
        meta: "1M ctx · $2/M",
    },
    ListRow::Item {
        name: "OpenAI o3",
        detail: "slow · complex tasks",
        meta: "200K ctx · $10/M",
    },
    ListRow::Item {
        name: "GPT-4.1 mini",
        detail: "fast · low cost",
        meta: "1M ctx · $0.40/M",
    },
    ListRow::Section("Google"),
    ListRow::Item {
        name: "Gemini 2.5 Pro",
        detail: "balanced · long context",
        meta: "1M ctx · $1.25/M",
    },
    ListRow::Item {
        name: "Gemini 2.5 Flash",
        detail: "fast · low cost",
        meta: "1M ctx · $0.30/M",
    },
    ListRow::Item {
        name: "Gemini 2.5 Flash Lite",
        detail: "fastest · low cost",
        meta: "1M ctx · $0.10/M",
    },
    ListRow::Item {
        name: "Gemini 2.0 Flash",
        detail: "fast · multimodal",
        meta: "1M ctx · $0.10/M",
    },
    ListRow::Item {
        name: "Gemini 1.5 Pro",
        detail: "balanced · long context",
        meta: "2M ctx · $1.25/M",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 22, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd(
                "which model should handle this multi-file migration?",
                Some("1m ago"),
            ),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I can use the active model or switch before the next turn."),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Select model",
        &[
            ("↑↓", "move"),
            ("enter", "select"),
            ("esc", "cancel"),
            ("type", "filter"),
        ],
    );
    let lines = chrome::sectioned_select_list(inner.width as usize, MODELS, 1, Some(""));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
