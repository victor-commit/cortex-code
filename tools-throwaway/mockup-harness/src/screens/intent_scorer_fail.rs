//! Failure variant of the shared intent-scoring scorecard.

use ratatui::Frame;

use crate::screens::intent_scorer_pass;

pub fn render(f: &mut Frame) {
    intent_scorer_pass::render_fail(f);
}
