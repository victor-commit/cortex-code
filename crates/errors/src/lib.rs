//! Shared error/severity types and the canonical diagnostic enum for Cortex Code.

mod diagnostic;
mod exit;

pub use diagnostic::Diagnostic;
pub use exit::Exit;
