//! Per-project saved-permission store keyed by the canonical workspace path.
//!
//! A thin headless adapter over the shared [`permissions::store`] durable store: it resolves the
//! per-project store file by the SAME workspace key the shadow checkpoint store uses
//! ([`crate::checkpoint::workspace_key`]), so headless `cortex run` and the interactive TUI address
//! ONE store and a decision saved in either surface is honored by the other without re-asking
//! (VAL-CROSS-027/028). The durable format, location precedence, and read/write mechanics live in
//! `permissions::store`; this module only maps a working directory to its workspace key. The path
//! resolution and round-trip behavior are tested there; the live cross-process/cross-surface proofs
//! exercise this adapter end to end.

use std::io;
use std::path::Path;

use permissions::SavedDecision;

/// Load this project's saved decisions from the process-resolved store, or an empty list when no
/// store exists yet or no data location can be determined.
#[must_use]
pub fn load_for(cwd: &Path) -> Vec<SavedDecision> {
    permissions::store::load_for(&crate::checkpoint::workspace_key(cwd))
}

/// Persist this project's saved decisions to the process-resolved store.
///
/// # Errors
/// Returns the underlying [`io::Error`] when the resolved store file cannot be written.
pub fn save_for(cwd: &Path, decisions: &[SavedDecision]) -> io::Result<()> {
    permissions::store::save_for(&crate::checkpoint::workspace_key(cwd), decisions)
}
