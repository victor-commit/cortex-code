//! Project attachment picker dock for `/attach`.

use ratatui::{text::Line, widgets::Paragraph, Frame};

use crate::chrome::{self, AnnotatedListRow, StatusLine};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 15, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("compare the latest picker renders in this project", Some("1m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   Choose an image or document to include with this turn."),
            Line::from(""),
            chrome::user_cmd("/attach", None),
        ],
    );

    let rows = [
        AnnotatedListRow {
            name: "000-attachment.svg",
            detail: "",
            meta: "root · 375b · svg".to_owned(),
            meta_color: chrome::MUTED,
            disabled: false,
        },
        AnnotatedListRow {
            name: "plans/assets/mockups-v2/input/model-picker.png",
            detail: "",
            meta: "input · 205.8k · png".to_owned(),
            meta_color: chrome::MUTED,
            disabled: false,
        },
        AnnotatedListRow {
            name: "plans/assets/mockups-v2/input/theme-picker.png",
            detail: "",
            meta: "input · 90.2k · png".to_owned(),
            meta_color: chrome::MUTED,
            disabled: false,
        },
        AnnotatedListRow {
            name: "plans/assets/mockups-v2/session/home.png",
            detail: "",
            meta: "session · 98.5k · png".to_owned(),
            meta_color: chrome::MUTED,
            disabled: false,
        },
        AnnotatedListRow {
            name: "001-invalid.txt",
            detail: "",
            meta: "root · 64b · unsupported".to_owned(),
            meta_color: chrome::FAINT,
            disabled: true,
        },
        AnnotatedListRow {
            name: "README.md",
            detail: "",
            meta: "root · 11.4k · unsupported".to_owned(),
            meta_color: chrome::FAINT,
            disabled: true,
        },
        AnnotatedListRow {
            name: "Cargo.toml",
            detail: "",
            meta: "root · 4.6k · unsupported".to_owned(),
            meta_color: chrome::FAINT,
            disabled: true,
        },
        AnnotatedListRow {
            name: "plans/assets/mockups-v2/DESIGN-SYSTEM.md",
            detail: "",
            meta: "plans · 21.4k · unsupported".to_owned(),
            meta_color: chrome::FAINT,
            disabled: true,
        },
        AnnotatedListRow {
            name: "tools-throwaway/mockup-harness/assets/DejaVuSansMono.ttf",
            detail: "",
            meta: "assets · 343.1k · unsupported".to_owned(),
            meta_color: chrome::FAINT,
            disabled: true,
        },
    ];
    let inner = chrome::card(
        f,
        dock,
        "Attach file (png, jpg, pdf, svg)",
        &[("↑↓", "move"), ("enter", "attach"), ("esc", "close")],
    );
    let mut lines = chrome::annotated_select_list(inner.width as usize, &rows, 0, Some(""));
    lines.push(Line::from(""));
    lines.push(chrome::list_counter(inner.width as usize, "9 project files"));
    f.render_widget(Paragraph::new(lines), inner);
}
