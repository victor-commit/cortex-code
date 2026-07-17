//! Pure Cortex pilot screens. Copper brand helpers are forbidden in new paints.

pub mod chat_turn;
pub mod login;

use crate::chrome::canvas::Canvas;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PilotSpec {
    pub id: &'static str,
    pub category: &'static str,
}

pub const PILOT_SCREENS: [PilotSpec; 2] = [
    PilotSpec {
        id: "login",
        category: "auth",
    },
    PilotSpec {
        id: "chat-turn",
        category: "session",
    },
];

pub fn render_pilot(id: &str) -> Result<Canvas, String> {
    match id {
        "login" => login::render(),
        "chat-turn" => chat_turn::render(),
        other => Err(format!("unknown pure-Cortex pilot id: {other}")),
    }
}

pub fn list_pilots() -> &'static [PilotSpec] {
    &PILOT_SCREENS
}
