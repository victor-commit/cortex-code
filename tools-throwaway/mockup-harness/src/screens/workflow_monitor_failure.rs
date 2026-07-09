//! Failure-escalated workflow monitor variant.

use ratatui::Frame;

pub fn render(f: &mut Frame) {
    super::workflow_monitor::render_failure(f);
}
