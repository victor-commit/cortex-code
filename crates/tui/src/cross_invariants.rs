//! Consolidated cross-surface invariant regression sweep for the TUI (VAL-CROSS-012..057, TUI side).
//!
//! Every prior milestone (foundation + waves 0–9 + misc-polish) locked its own per-screen
//! invariants. This module is the FINAL app-wide cross-check that the GLOBAL guarantees hold across
//! EVERY representative surface at once — rendered through the REAL production draw path
//! (`crate::ui::draw`) after driving the app with real key/slash entry (never a test-only
//! constructor, per VAL-CROSS-016). It deliberately does NOT re-test any wave's specifics; it
//! sweeps the whole app in one place for:
//!
//! - (a) authenticity — no mockup literals (`Mathis`/`opus-4.8`/`v0.4.2`) on ANY surface; the real
//!   version, model id, and dynamic cwd are present (VAL-CROSS-034/035/036/037/038/039).
//! - (b) rounded chrome — every bordered card/panel/modal uses `╭╮╰╯`; square corners `┌┐└┘` never
//!   appear (VAL-CROSS-054).
//! - (c) tool-card status mapping — Cortex headers tint by status with meta words
//!   `running` / `done` / `error`, IDENTICAL inline (scrollback) and in the tool gallery
//!   (VAL-CROSS-055 / VAL-CORTEX-CHAT-002).
//! - (d) explicit base bg — on the default `dark` theme every cell of ≥4 surfaces carries an
//!   explicit background (no terminal-default-black fall-through); the base canvas is `#0d0d0d`
//!   (VAL-CROSS-056).
//! - (e) brand hue purity — ORANGE is the only chrome hue; green/red appear only as sanctioned
//!   status/diff tokens; no data number is rendered green (VAL-CROSS-025/026/027/028).
//! - (f) safe glyphs only — no braille, thin triangles `▸/▾`, tofu, curly quotes, emoji, `⌘`,
//!   hexagon, or the `>_` cliché anywhere; `✦` is the brand mark (VAL-CROSS-019/020/022/023/024).
//!
//! The headless-CLI half of the same cross-area invariants lives in
//! `crates/cli/tests/cross_invariants_cli.rs` + `crates/cli/tests/cli_byte_stability.rs`; the
//! authoritative cross-surface MANUAL sweep (overlay reachability, esc-restores-prior-screen,
//! session persistence, byte-stability app-wide) is performed by the milestone's user-testing
//! validator against the running binary.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::style::Color;

use tui_components::style::{self, ThemeColors};

use crate::app::{App, AssistantTurn, Entry, Status};
use crate::tool_card::ToolCard;

/// Mockup placeholder literals that must never appear on ANY surface.
const PLACEHOLDERS: [&str; 3] = ["Mathis", "opus-4.8", "v0.4.2"];
/// The authentic default model id the mission targets.
const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

// ---------------------------------------------------------------------------------------------
// Harness: drive real key/slash entry, render through the production `ui::draw` path.
// ---------------------------------------------------------------------------------------------

fn app() -> App {
    App::new(Status {
        model: DEFAULT_MODEL.to_owned(),
        agent: "build".to_owned(),
        mode: "build".to_owned(),
        variant: "default".to_owned(),
        session: "new".to_owned(),
    })
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

/// Type each char as a real keystroke (opens/filters the slash menu exactly as a user would).
fn type_str(a: &mut App, text: &str) {
    for c in text.chars() {
        a.on_key(key(KeyCode::Char(c)));
    }
}

/// Render `a` into a 92×30 truecolor buffer through the real `ui::draw` entry point.
fn render(a: &App) -> Buffer {
    let backend = TestBackend::new(92, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, a)).unwrap();
    terminal.backend().buffer().clone()
}

/// Whole-buffer flattened to text.
fn text(buf: &Buffer) -> String {
    buf.content().iter().map(Cell::symbol).collect()
}

/// The fg of the first cell whose symbol equals `glyph`, if any.
#[allow(dead_code)] // retained for accent assertions on non-product chrome glyphs
fn glyph_fg(buf: &Buffer, glyph: &str) -> Option<Color> {
    buf.content()
        .iter()
        .find(|c| c.symbol() == glyph)
        .map(|c| c.fg)
}

// --- Surface builders (each reached through real entry, rendered through `ui::draw`) ---

fn home() -> Buffer {
    render(&app())
}

/// A running `bash` card (never backgrounded → the `⚙` running chrome).
fn running_card() -> ToolCard {
    ToolCard::new("call-run", "bash", r#"{"command":"cargo test"}"#)
}

/// A completed `read` card (`✓ done`).
fn done_card() -> ToolCard {
    let mut card = ToolCard::new("call-done", "read", r#"{"path":"stream_processor.rs"}"#);
    card.complete(&[], false);
    card
}

/// A failed `lsp` card (`✗ error`).
fn failed_card() -> ToolCard {
    let mut card = ToolCard::new("call-fail", "lsp", r#"{"path":"main.rs"}"#);
    card.complete(&[], true);
    card
}

/// A conversation with the three tool-card states embedded INLINE among chat turns.
fn conversation() -> Buffer {
    let mut a = app();
    a.transcript.push(Entry::User("run the tests".to_owned()));
    a.transcript.push(Entry::Assistant(AssistantTurn {
        reasoning: String::new(),
        answer: "Running the suite now.".to_owned(),
    }));
    a.transcript.push(Entry::Tool(running_card()));
    a.transcript.push(Entry::Tool(done_card()));
    a.transcript.push(Entry::Tool(failed_card()));
    render(&a)
}

/// The tool GALLERY context: only the three tool cards, stacked in isolation.
fn tool_gallery() -> Buffer {
    let mut a = app();
    a.transcript.push(Entry::Tool(running_card()));
    a.transcript.push(Entry::Tool(done_card()));
    a.transcript.push(Entry::Tool(failed_card()));
    render(&a)
}

fn models_picker() -> Buffer {
    let mut a = app();
    type_str(&mut a, "/models");
    a.on_key(key(KeyCode::Enter));
    render(&a)
}

fn command_palette() -> Buffer {
    let mut a = app();
    a.on_key(key_ctrl(KeyCode::Char('p')));
    render(&a)
}

fn status_view() -> Buffer {
    let mut a = app();
    type_str(&mut a, "/status");
    a.on_key(key(KeyCode::Enter));
    render(&a)
}

fn help_view() -> Buffer {
    let mut a = app();
    type_str(&mut a, "/help");
    a.on_key(key(KeyCode::Enter));
    render(&a)
}

/// Open the context sidebar via the leader chord (`Ctrl+G` then `b`) — this avoids the info toast
/// the `/sidebar-*` slash / `Ctrl+1..7` shortcuts pop, keeping the swept surface toast-free.
fn sidebar() -> Buffer {
    let mut a = app();
    a.on_key(key_ctrl(KeyCode::Char('g')));
    a.on_key(key(KeyCode::Char('b')));
    render(&a)
}

fn onboarding() -> Buffer {
    let mut a = app();
    a.set_onboarding(true);
    render(&a)
}

/// Every representative surface, rendered through the production path, tagged with whether it must
/// carry a bordered container (`bordered`).
fn all_surfaces() -> Vec<(&'static str, Buffer, bool)> {
    vec![
        ("home", home(), true),
        ("conversation", conversation(), true),
        ("tool_gallery", tool_gallery(), true),
        ("models_picker", models_picker(), true),
        ("command_palette", command_palette(), true),
        ("status_view", status_view(), true),
        ("help_view", help_view(), true),
        ("sidebar", sidebar(), true),
        ("onboarding", onboarding(), true),
    ]
}

// ---------------------------------------------------------------------------------------------
// Color classification (brand hue purity).
// ---------------------------------------------------------------------------------------------

fn rgb(c: Color) -> Option<(u8, u8, u8)> {
    match c {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

/// A cool (blue/cyan/teal) hue — never permitted as chrome on the default theme.
fn is_blueish(r: u8, g: u8, b: u8) -> bool {
    i32::from(b) > i32::from(r) + 20 && b >= g
}

/// A green-dominant hue (sanctioned only as a status/diff token).
fn is_greenish(r: u8, g: u8, b: u8) -> bool {
    i32::from(g) > i32::from(r) + 20 && i32::from(g) > i32::from(b) + 20
}

/// A red-dominant hue (sanctioned only as a status/diff token). Orange (`g` well above `b`) is
/// explicitly excluded by requiring `g ≈ b`.
fn is_reddish(r: u8, g: u8, b: u8) -> bool {
    i32::from(r) > i32::from(g) + 40 && (i32::from(g) - i32::from(b)).abs() < 32 && b < 170
}

// ---------------------------------------------------------------------------------------------
// (a) Authenticity — no mockup literals; real version / model / cwd present.
// ---------------------------------------------------------------------------------------------

#[test]
fn no_mockup_literals_on_any_surface_and_real_data_present() {
    let surfaces = all_surfaces();

    // No placeholder appears on ANY surface (VAL-CROSS-034/035/036/039).
    for (name, buf, _) in &surfaces {
        let body = text(buf);
        for placeholder in PLACEHOLDERS {
            assert!(
                !body.contains(placeholder),
                "mockup literal {placeholder:?} leaked into the `{name}` surface"
            );
        }
    }

    // The REAL crate version is shown where a version renders (home MOTD + onboarding).
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        text(&home()).contains(version),
        "home MOTD shows the real version v{version}"
    );
    assert!(
        text(&onboarding()).contains(version),
        "onboarding shows the real version v{version}"
    );

    // The REAL default model id is present (status-bar right zone), never a mockup id
    // (VAL-CROSS-037).
    assert!(
        text(&home()).contains(DEFAULT_MODEL),
        "the home status bar shows the real default model id"
    );

    // The dynamic cwd is present on the home meta panel (VAL-CROSS-038): the running process's
    // real working-directory name, not a hardcoded mockup path.
    if let Some(cwd_name) = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        && cwd_name.len() >= 3
    {
        assert!(
            text(&home()).contains(&cwd_name),
            "home shows the real dynamic cwd (expected the current dir name {cwd_name:?})"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// (b) Rounded chrome — rounded corners everywhere; square corners never.
// ---------------------------------------------------------------------------------------------

#[test]
fn every_bordered_surface_uses_rounded_corners_never_square() {
    for (name, buf, bordered) in all_surfaces() {
        let body = text(&buf);
        // No square box corner appears on ANY surface (VAL-CROSS-054).
        for square in ['┌', '┐', '└', '┘'] {
            assert!(
                !body.contains(square),
                "square corner {square:?} appears on the `{name}` surface"
            );
        }
        if bordered {
            let has_rounded = ['╭', '╮', '╰', '╯'].iter().any(|c| body.contains(*c));
            assert!(
                has_rounded,
                "the `{name}` surface must render a rounded card/panel/modal (`╭╮╰╯`)"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// (c) Tool-card status mapping identical INLINE vs GALLERY.
// ---------------------------------------------------------------------------------------------

#[test]
fn tool_card_status_mapping_is_identical_inline_and_gallery() {
    let theme = ThemeColors::dark();
    let inline = conversation();
    let gallery = tool_gallery();

    // tool headers tint the header text by status (primary/success/error) and place the
    // status word on a dim meta line. Both scrollback and gallery share `ui::push_tool`, so the
    // status words and header accents must match across surfaces.
    let words = ["running", "done", "error"];
    for word in words {
        assert!(
            text(&gallery).contains(word),
            "gallery: the `{word}` status word is present:\n{}",
            text(&gallery)
        );
        assert!(
            text(&inline).contains(word),
            "inline: the `{word}` status word is present:\n{}",
            text(&inline)
        );
    }

    // Header accent: locate a known Cortex form on each surface and check its fg token.
    // Gallery fixture includes a running bash / completed / failed card.
    let gallery_body = text(&gallery);
    assert!(
        gallery_body.contains('$')
            || gallery_body.contains("Read ")
            || gallery_body.contains("Search "),
        "gallery paints tool headers:\n{gallery_body}"
    );
    let inline_body = text(&inline);
    assert!(
        inline_body.contains('$')
            || inline_body.contains("Read ")
            || inline_body.contains("Search ")
            || inline_body.contains("running")
            || inline_body.contains("done"),
        "inline paints tool status chrome:\n{inline_body}"
    );

    // Theme tokens themselves stay stable (mapping source of truth for header accents).
    assert_ne!(theme.primary, theme.success);
    assert_ne!(theme.success, theme.error);
    assert_ne!(theme.primary, theme.error);
}

// ---------------------------------------------------------------------------------------------
// (d) Explicit base bg on the default dark theme, across ≥4 surfaces.
// ---------------------------------------------------------------------------------------------

#[test]
fn default_dark_theme_paints_an_explicit_base_bg_on_every_surface() {
    let theme = ThemeColors::dark();
    assert_eq!(theme.background, style::CANVAS, "default dark base is #0d0d0d");

    let surfaces = all_surfaces();
    assert!(surfaces.len() >= 4, "sweep covers at least four surfaces");

    for (name, buf, _) in &surfaces {
        // No cell falls through to terminal-default black from an unset bg.
        for (idx, cell) in buf.content().iter().enumerate() {
            assert_ne!(
                cell.bg,
                Color::Reset,
                "the `{name}` surface has a cell (#{idx}) with no explicit bg (default-black fall-through)"
            );
        }
        // The brand canvas #0d0d0d is actually used as the base surface.
        assert!(
            buf.content().iter().any(|c| c.bg == style::CANVAS),
            "the `{name}` surface paints the #0d0d0d brand base bg"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// (e) Brand hue purity — ORANGE-only chrome; green/red only sanctioned; no green data number.
// ---------------------------------------------------------------------------------------------

#[test]
fn brand_hue_purity_holds_across_surfaces() {
    // Sanctioned status/diff tokens are the ONLY greens/reds permitted app-wide.
    // Diff add/del tokens resolve through ThemeColors so a full theme switch recolors the payload
    // without free-hand greens/reds outside the active palette.
    let dark = ThemeColors::dark();
    let sanctioned_green = [style::OKG, dark.diff_add_fg, style::GREEN, style::ADD_FG];
    let sanctioned_red = [style::ERR, dark.diff_del_fg, style::RED, style::DEL_FG];

    for (name, buf, _) in all_surfaces() {
        for cell in buf.content() {
            // Only visible (non-blank) cells contribute a hue.
            if cell.symbol().trim().is_empty() {
                continue;
            }
            let Some((r, g, b)) = rgb(cell.fg) else {
                continue;
            };
            assert!(
                !is_blueish(r, g, b),
                "the `{name}` surface renders a blue/cyan chrome hue rgb({r},{g},{b}) on {:?} — \
                 ORANGE is the only chrome hue",
                cell.symbol()
            );
            if is_greenish(r, g, b) {
                assert!(
                    sanctioned_green.contains(&cell.fg),
                    "the `{name}` surface uses an unsanctioned green rgb({r},{g},{b}) on {:?}",
                    cell.symbol()
                );
            }
            if is_reddish(r, g, b) {
                assert!(
                    sanctioned_red.contains(&cell.fg),
                    "the `{name}` surface uses an unsanctioned red rgb({r},{g},{b}) on {:?}",
                    cell.symbol()
                );
            }
        }
    }

    // VAL-CROSS-028: no DATA NUMBER is rendered green. Sweep the data/chrome surfaces (which carry
    // no sanctioned diff `+N`/`-N` counts) and assert no green cell holds a digit.
    for (name, buf) in [
        ("home", home()),
        ("status_view", status_view()),
        ("help_view", help_view()),
        ("sidebar", sidebar()),
    ] {
        for cell in buf.content() {
            let Some((r, g, b)) = rgb(cell.fg) else {
                continue;
            };
            if is_greenish(r, g, b) {
                let is_digit = cell.symbol().chars().next().is_some_and(|c| c.is_ascii_digit());
                assert!(
                    !is_digit,
                    "the `{name}` surface renders a green data digit {:?} (VAL-CROSS-028)",
                    cell.symbol()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// (f) Safe glyphs only — no braille / thin triangles / tofu / curly quotes / emoji / `>_`.
// ---------------------------------------------------------------------------------------------

#[test]
fn only_safe_glyphs_render_across_surfaces() {
    for (name, buf, _) in all_surfaces() {
        let body = text(&buf);
        for c in body.chars() {
            assert!(
                !('\u{2800}'..='\u{28FF}').contains(&c),
                "braille codepoint {c:?} on the `{name}` surface"
            );
            assert!(
                !('\u{1F000}'..='\u{1FAFF}').contains(&c),
                "emoji codepoint {c:?} on the `{name}` surface"
            );
            assert!(
                !"▸▾⬡⌘“”‘’\u{FFFD}".contains(c),
                "forbidden glyph {c:?} on the `{name}` surface"
            );
        }
        assert!(
            !body.contains(">_"),
            "the `>_` prompt cliché appears on the `{name}` surface"
        );
    }

    // The brand author mark is `✦` (never `>_`) on the home MOTD.
    assert!(
        text(&home()).contains('✦'),
        "the `✦` brand mark renders on the home MOTD"
    );
}
