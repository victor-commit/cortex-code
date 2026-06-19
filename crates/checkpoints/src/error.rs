//! The single error type the shadow checkpoint store returns.
//!
//! Every fallible operation in this crate resolves to a [`CheckpointError`]. It carries a
//! human-readable detail plus a stable diagnostic code for logs. A failure never blocks a tool
//! mutation: the caller (the CLI bridge) records the failure and degrades undo for the affected
//! turn rather than aborting the agent's work.

use std::error::Error;
use std::fmt;

/// A failure interacting with the shadow checkpoint store (its git directory, its object database,
/// or its on-disk journal).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointError {
    detail: String,
}

impl CheckpointError {
    /// Build an error carrying `detail`.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    /// The stable diagnostic code for logs.
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        "checkpoint.shadow_store.unavailable"
    }

    /// The human-readable failure detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "shadow checkpoint store unavailable: {}",
            self.detail
        )
    }
}

impl Error for CheckpointError {}
