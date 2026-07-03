// SPDX-License-Identifier: MIT
// Provenance: direct-port (clean-room exception: direct-port-allowed)
// Upstream: CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Repo: https://github.com/CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-core/src/markdown/renderer/helpers.rs
// Target-Path: crates/frame/src/markdown/renderer/helpers.rs
// Port-Mode: direct-copy
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Helper functions for markdown rendering.

use ahash::AHasher;
use pulldown_cmark::HeadingLevel;
use std::hash::{Hash, Hasher};

/// Convert HeadingLevel to u8.
pub fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Bullet characters by depth level.
const BULLETS: [&str; 4] = ["•", "◦", "▪", "·"];

/// Get bullet character for a depth level.
pub fn get_bullet(depth: usize) -> &'static str {
    BULLETS[depth.min(BULLETS.len() - 1)]
}

/// Hash a string quickly using ahash.
pub fn hash_string(s: &str) -> u64 {
    let mut hasher = AHasher::default();
    s.hash(&mut hasher);
    hasher.finish()
}
