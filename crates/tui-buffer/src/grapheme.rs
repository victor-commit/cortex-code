// SPDX-License-Identifier: MIT
// Provenance: direct-port (clean-room exception: direct-port-allowed)
// Upstream: CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Repo: https://github.com/CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: cortex-tui-framework/crates/cortex-tui-buffer/src/grapheme.rs
// Target-Path: crates/tui-buffer/src/grapheme.rs
// Port-Mode: direct-copy
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)
// Upstream notices retained where present; product distributed under Apache-2.0; see NOTICE.

//! Grapheme cluster storage and lookup.
//!
//! Handles multi-codepoint grapheme clusters (emoji, combining characters, etc.)
//! efficiently through interning.

use std::collections::HashMap;

/// Unique identifier for a grapheme cluster in the pool.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GraphemeId(pub(crate) u32);

impl GraphemeId {
    /// Returns the raw ID value.
    #[inline]
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// Pool for storing and deduplicating grapheme clusters.
///
/// Extended grapheme clusters (multi-codepoint sequences like emoji with
/// skin tones, or combining character sequences) are stored once and
/// referenced by [`GraphemeId`].
pub struct GraphemePool {
    graphemes: Vec<String>,
    lookup: HashMap<String, GraphemeId>,
}

impl Default for GraphemePool {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphemePool {
    /// Creates a new empty grapheme pool.
    pub fn new() -> Self {
        Self {
            graphemes: Vec::new(),
            lookup: HashMap::new(),
        }
    }

    /// Creates a pool with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            graphemes: Vec::with_capacity(capacity),
            lookup: HashMap::with_capacity(capacity),
        }
    }

    /// Interns a grapheme cluster and returns its ID.
    ///
    /// If the grapheme already exists, returns the existing ID.
    /// Otherwise, stores it and returns a new ID.
    pub fn intern(&mut self, grapheme: &str) -> GraphemeId {
        if let Some(&id) = self.lookup.get(grapheme) {
            return id;
        }

        let id = GraphemeId(self.graphemes.len() as u32);
        self.graphemes.push(grapheme.to_string());
        self.lookup.insert(grapheme.to_string(), id);
        id
    }

    /// Retrieves a grapheme by its ID.
    #[inline]
    pub fn get(&self, id: GraphemeId) -> Option<&str> {
        self.graphemes.get(id.0 as usize).map(|s| s.as_str())
    }

    /// Returns the number of unique graphemes stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.graphemes.len()
    }

    /// Returns whether the pool is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.graphemes.is_empty()
    }

    /// Clears all stored graphemes.
    ///
    /// Note: This invalidates all previously returned GraphemeIds.
    pub fn clear(&mut self) {
        self.graphemes.clear();
        self.lookup.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_and_get() {
        let mut pool = GraphemePool::new();
        let id = pool.intern("👨‍👩‍👧‍👦");
        assert_eq!(pool.get(id), Some("👨‍👩‍👧‍👦"));
    }

    #[test]
    fn test_deduplication() {
        let mut pool = GraphemePool::new();
        let id1 = pool.intern("🎉");
        let id2 = pool.intern("🎉");
        assert_eq!(id1, id2);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_multiple_graphemes() {
        let mut pool = GraphemePool::new();
        let id1 = pool.intern("🎉");
        let id2 = pool.intern("👋🏽");
        let id3 = pool.intern("🏳️‍🌈");

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_eq!(pool.len(), 3);

        assert_eq!(pool.get(id1), Some("🎉"));
        assert_eq!(pool.get(id2), Some("👋🏽"));
        assert_eq!(pool.get(id3), Some("🏳️‍🌈"));
    }
}
