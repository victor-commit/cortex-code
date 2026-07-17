//! Compatibility wrapper: chat-turn lives under conversation for F2 batching.

use crate::chrome::canvas::Canvas;

pub fn render() -> Result<Canvas, String> {
    super::conversation::render("chat-turn")
}
