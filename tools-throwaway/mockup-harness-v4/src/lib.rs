//! Pure Cortex dump harness (mockup-harness-v4).
//!
//! Publishes F1–F7 canonical + 35 gate frames into existing product roots:
//! - plans/assets/mockups-v3/
//! - plans/assets/mockups-v3-gates/
//!
//! NO copper Cortex Identity helpers. Chrome law: library/cortex-pure-dump-spec.md.

pub mod chrome;
pub mod publish;
pub mod screens;

pub use chrome::tokens::{
    palette, validate_glyphs, ColorToken, Theme, ALLOWED_GLYPHS, CORTEX_DAY, CORTEX_NIGHT,
};
pub use publish::{
    publish_gate_frame, publish_gates, publish_pilot, publish_pilots, PublishResult, GATE_FRAMES,
};
pub use screens::{list_pilots, render_pilot, PilotSpec};
