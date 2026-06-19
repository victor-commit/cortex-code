//! TEMP back-compat shim. DELETE in Wave 15 (T115). plan: tui-swap-fulllive
//!
//! This is NOT ported upstream code; it is our coexistence bridge, so it carries
//! no provenance header. It republishes the 14 ocean/cyan palette constants that
//! used to live in this crate's old `style.rs`, so the still-alive old TUI
//! (`cortex-cli`) and old `cortex-tui-components` keep compiling unchanged during
//! the TUI swap.
//!
//! The constants are `ratatui::style::Color` (not the foundation `Color`):
//! every consumer feeds them into `ratatui::style::Style::fg/bg` and into
//! `const fn` theme builders, which require ratatui's enum in const context. The
//! foundation `Color` is an RGBA-float struct with no ratatui interop, so using
//! it here would break `cargo build --workspace` and defeat the shim's purpose.
//! `ratatui` is therefore an optional dependency, pulled in only by the
//! `legacy-palette` feature.
//!
//! Removal checklist for T115 (delete ALL of these together):
//!   1. this file (`src/legacy_palette.rs`),
//!   2. the `#[cfg(feature = "legacy-palette")] mod legacy_palette;` in `lib.rs`,
//!   3. the `#[cfg(feature = "legacy-palette")] pub use ...` at the end of
//!      `style.rs`,
//!   4. the `[features] legacy-palette` + optional `ratatui` dep in this crate's
//!      `Cargo.toml`,
//!   5. the `features = ["legacy-palette"]` opt-ins in `crates/cortex-cli` and
//!      `crates/cortex-tui-components` `Cargo.toml`.

use ratatui::style::Color;

/// Primary green accent. (legacy palette)
pub const CYAN_PRIMARY: Color = Color::Rgb(0, 255, 163);

/// Main background, deep void. (legacy palette)
pub const VOID: Color = Color::Rgb(10, 22, 40);

/// Surface level 0, darkest surface. (legacy palette)
pub const SURFACE_0: Color = Color::Rgb(13, 27, 42);

/// Surface level 1, mid surface. (legacy palette)
pub const SURFACE_1: Color = Color::Rgb(27, 40, 56);

/// Surface level 2, light surface. (legacy palette)
pub const SURFACE_2: Color = Color::Rgb(36, 59, 83);

/// Primary text, white. (legacy palette)
pub const TEXT: Color = Color::Rgb(255, 255, 255);

/// Dimmed secondary text. (legacy palette)
pub const TEXT_DIM: Color = Color::Rgb(130, 154, 177);

/// Muted text for background elements. (legacy palette)
pub const TEXT_MUTED: Color = Color::Rgb(72, 101, 129);

/// Success, cyan-green. (legacy palette)
pub const SUCCESS: Color = Color::Rgb(0, 245, 212);

/// Warning, golden amber. (legacy palette)
pub const WARNING: Color = Color::Rgb(255, 200, 87);

/// Error, coral red. (legacy palette)
pub const ERROR: Color = Color::Rgb(255, 107, 107);

/// Info, light blue. (legacy palette)
pub const INFO: Color = Color::Rgb(72, 202, 228);

/// Normal border, subtle navy. (legacy palette)
pub const BORDER: Color = Color::Rgb(27, 73, 101);

/// Focused border, bright green. (legacy palette)
pub const BORDER_FOCUS: Color = Color::Rgb(0, 255, 163);
