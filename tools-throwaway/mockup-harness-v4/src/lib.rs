//! Pure Cortex dump harness (mockup-harness-v4).
//!
//! Publishes F1–F3 TXT/PNG into existing product roots:
//! - plans/assets/mockups-v3/
//! - plans/assets/mockups-v3-gates/
//!
//! NO copper Cortex Identity helpers. Chrome law: library/grok-pure-dump-spec.md.

pub mod chrome;
pub mod publish;
pub mod screens;

pub use chrome::tokens::{
    palette, validate_glyphs, ColorToken, Theme, ALLOWED_GLYPHS, GROK_DAY, GROK_NIGHT,
};
pub use publish::{publish_pilot, publish_pilots, PublishResult};
pub use screens::{list_pilots, render_pilot, PilotSpec};
