// SPDX-License-Identifier: MIT
// Provenance: ported source (direct-port-allowed), adapted to this workspace
// Source-Repo: CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-core/src/style.rs
// Target-Path: crates/frame/src/style.rs
// Port-Mode: adapt (subset)
// Adaptation: crate renamed cortex-frame -> frame. Only the color-palette constants consumed by
//   the markdown theme are ported here; the upstream CortexStyle helper struct and the rest of
//   style.rs are out of scope for the markdown-render feature.
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Cortex color palette (subset) used by the markdown renderer's default theme.

use ratatui::style::Color;

// Brand / accent colors.
/// Primary accent color.
pub const CYAN_PRIMARY: Color = Color::Rgb(0, 255, 163); // #00FFA3
/// Light secondary accent.
pub const SKY_BLUE: Color = Color::Rgb(100, 255, 180); // #64FFB4
/// Mid accent used for links and interactive elements.
pub const DEEP_CYAN: Color = Color::Rgb(0, 200, 130); // #00C882

// Background surfaces.
/// Darkest surface.
pub const SURFACE_0: Color = Color::Rgb(13, 27, 42); // #0D1B2A
/// Mid surface.
pub const SURFACE_1: Color = Color::Rgb(27, 40, 56); // #1B2838

// Text colors.
/// Primary text.
pub const TEXT: Color = Color::Rgb(255, 255, 255); // #FFFFFF
/// Dimmed secondary text.
pub const TEXT_DIM: Color = Color::Rgb(130, 154, 177); // #829AB1
/// Muted text for background elements.
pub const TEXT_MUTED: Color = Color::Rgb(72, 101, 129); // #486581
/// Bright text for emphasis.
pub const TEXT_BRIGHT: Color = Color::Rgb(255, 255, 255); // #FFFFFF

// Semantic colors.
/// Success accent.
pub const SUCCESS: Color = Color::Rgb(0, 245, 212); // #00F5D4
/// Informational accent.
pub const INFO: Color = Color::Rgb(72, 202, 228); // #48CAE4
/// Highlight / emphasis accent.
pub const HIGHLIGHT: Color = Color::Rgb(125, 249, 255); // #7DF9FF

// Borders.
/// Normal border color.
pub const BORDER: Color = Color::Rgb(27, 73, 101); // #1B4965
