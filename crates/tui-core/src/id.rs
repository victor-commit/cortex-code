// SPDX-License-Identifier: MIT
// Provenance: direct-port (clean-room exception: direct-port-allowed)
// Upstream: CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Repo: https://github.com/CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: cortex-tui-framework/crates/cortex-tui-core/src/id.rs
// Target-Path: crates/tui-core/src/id.rs
// Port-Mode: direct-copy
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)
// Upstream notices retained where present; product distributed under Apache-2.0; see NOTICE.

//! Node identifier types for Cortex TUI.

use std::sync::atomic::{AtomicU64, Ordering};

/// A unique identifier for nodes in the UI tree.
///
/// `NodeId` provides a lightweight, copy-able identifier that can be used
/// to reference nodes across the framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u64);

impl NodeId {
    /// Creates a new unique node ID.
    ///
    /// Each call generates a new ID that is guaranteed to be unique
    /// within the lifetime of the program.
    #[must_use]
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Returns the underlying u64 value.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Creates a NodeId from a raw u64 value.
    ///
    /// # Safety
    /// This should only be used for deserialization or testing purposes.
    /// Using arbitrary values may lead to ID collisions.
    #[must_use]
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeId({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unique_ids() {
        let id1 = NodeId::new();
        let id2 = NodeId::new();
        let id3 = NodeId::new();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_from_raw() {
        let id = NodeId::from_raw(42);
        assert_eq!(id.as_u64(), 42);
    }
}
