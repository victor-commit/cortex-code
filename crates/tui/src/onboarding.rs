//! The first-run onboarding welcome screen (`onboard_welcome.png`).
//!
//! On a pristine launch — a fresh `HOME`/`XDG_*` with no user config and no stored provider
//! credentials — the TUI boots into this screen instead of the normal home/MOTD conversation. It
//! shows the shared `✦ Cortex Code · welcome` chrome, a centered rounded `Cortex CLI v<version>`
//! card (the authentic binary version), the brain mascot beside a `Welcome!` heading + subtitle,
//! three getting-started tips, and a first-run footer, over the persistent 3-zone status bar.
//! Pressing Enter advances out of onboarding into the normal app surface.
//!
//! Everything draws through theme tokens (mono-orange on near-black by default) and the safe glyph
//! set, and the version is the real `CARGO_PKG_VERSION` (never the `v0.4.2` mockup placeholder).

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use tui_components::style::ThemeColors;

use crate::app::App;
use crate::brand;
use crate::glyphs;
use crate::motd;
use crate::ui::{cells_to_line, place_segments};
use crate::view;

/// The breadcrumb crumb shown after `✦ Cortex Code ·` on the onboarding screen.
const WELCOME_CRUMB: &str = "welcome";
/// Width (columns) of the centered welcome card.
const CARD_W: u16 = 58;
/// Height (rows) of the centered welcome card.
const CARD_H: u16 = 13;
/// Interior column the mascot block-art is drawn at (inside the card border).
const MASCOT_COL: usize = 2;
/// Interior column the greeting/subtitle/tip text is drawn at (right of the mascot).
const TEXT_COL: usize = 15;
/// Interior column the tip bullet is drawn at.
const BULLET_COL: usize = 2;
/// Interior column the tip text is drawn at (right of the bullet).
const TIP_TEXT_COL: usize = 4;

/// The three getting-started tips shown inside the welcome card (sending `/help` for commands,
/// using Tab for autocomplete, and `@` to add files to context).
const TIPS: [&str; 3] = [
    "Send /help for commands",
    "Use Tab for autocomplete",
    "@ to add files to context",
];

/// Whether the current process is a pristine first run: no user (global) config file exists and no
/// provider credential is stored. Reads the process environment so a fresh throwaway `HOME`/`XDG_*`
/// (with no config and no credentials) boots into onboarding, while a configured environment does
/// not.
#[must_use]
pub fn is_first_run() -> bool {
    !has_global_config() && !has_stored_credentials()
}

/// The welcome card title, e.g. `Cortex CLI v0.1.0` — the real binary version.
#[must_use]
fn card_title() -> String {
    format!("Cortex CLI {}", motd::version_string())
}

/// Whether a user-level global config file exists at
/// `${XDG_CONFIG_HOME:-$HOME/.config}/cortex/config.toml`.
fn has_global_config() -> bool {
    global_config_path(env_path("XDG_CONFIG_HOME").as_deref(), env_path("HOME").as_deref())
        .is_some_and(|path| path.exists())
}

/// Resolve the global config file path from the config-home inputs, mirroring the settings loader's
/// global layer location. Pure (no environment access) so it is unit-testable.
fn global_config_path(xdg_config_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let base = match xdg_config_home {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => home?.join(".config"),
    };
    Some(base.join("cortex").join("config.toml"))
}

/// Whether the provider credential store holds at least one stored credential.
fn has_stored_credentials() -> bool {
    match providers::AuthPathInputs::from_process().resolve() {
        Some(path) => providers::list_stored_providers(&path)
            .map(|providers| !providers.is_empty())
            .unwrap_or(false),
        None => false,
    }
}

/// A non-empty environment variable read as a path.
fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Render the first-run onboarding welcome screen into `area`: the shared `✦ Cortex Code · welcome`
/// chrome (breadcrumb + rule + 3-zone status bar), the centered rounded `Cortex CLI v<version>`
/// card, and the first-run footer beneath it.
pub fn draw_onboarding(frame: &mut Frame, app: &App, area: Rect, theme: &ThemeColors) {
    let body = view::render_chrome(frame, area, app, WELCOME_CRUMB, theme);
    if body.width == 0 || body.height == 0 {
        return;
    }

    // Centered card in the upper-middle of the body, with one blank row under the rule.
    let card_w = CARD_W.min(body.width);
    let card_h = CARD_H.min(body.height.saturating_sub(1));
    if card_w < 4 || card_h < 4 {
        return;
    }
    let card_x = body.x + body.width.saturating_sub(card_w) / 2;
    let card_y = body.y + 1;
    let card_area = Rect {
        x: card_x,
        y: card_y,
        width: card_w,
        height: card_h,
    };

    let inner = view::render_panel(frame, card_area, &card_title(), None, theme);
    let lines = card_interior_lines(inner.width as usize, theme);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.background)),
        inner,
    );

    // First-run footer two rows below the card (a blank gap row, then the two centered lines).
    let footer_y = card_area.y + card_area.height + 1;
    render_footer(frame, body, footer_y, theme);
}

/// Build the welcome card interior: a blank top row, the mascot block-art beside the `Welcome!`
/// heading + subtitle, a gap, then the three tip rows. `width` is the card's inner width.
#[must_use]
fn card_interior_lines(width: usize, theme: &ThemeColors) -> Vec<Line<'static>> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let orange = Style::default().fg(theme.primary).bg(theme.background);
    let bright = Style::default()
        .fg(theme.text_bright)
        .bg(theme.background)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(theme.text_dim).bg(theme.background);
    let bullet_style = Style::default().fg(theme.border).bg(theme.background);

    let mascot = brand::MASCOT_LINES;
    let mut lines = vec![
        // Row 0: blank.
        interior_row(width, base, &[]),
        // Row 1: mascot top + `Welcome!` heading.
        interior_row(
            width,
            base,
            &[
                (MASCOT_COL, vec![(mascot[0].to_owned(), orange)]),
                (TEXT_COL, vec![("Welcome!".to_owned(), bright)]),
            ],
        ),
        // Row 2: mascot eyes + subtitle.
        interior_row(
            width,
            base,
            &[
                (MASCOT_COL, vec![(mascot[1].to_owned(), orange)]),
                (
                    TEXT_COL,
                    vec![("Your AI-powered coding assistant.".to_owned(), dim)],
                ),
            ],
        ),
        // Row 3: mascot lower.
        interior_row(width, base, &[(MASCOT_COL, vec![(mascot[2].to_owned(), orange)])]),
        // Row 4: mascot legs.
        interior_row(width, base, &[(MASCOT_COL, vec![(mascot[3].to_owned(), orange)])]),
        // Row 5: blank gap.
        interior_row(width, base, &[]),
    ];

    // Rows 6..8: the three getting-started tips (`•` ORANGE_D bullet + DIM text).
    for tip in TIPS {
        lines.push(interior_row(
            width,
            base,
            &[
                (BULLET_COL, vec![(glyphs::BULLET.to_owned(), bullet_style)]),
                (TIP_TEXT_COL, vec![(tip.to_owned(), dim)]),
            ],
        ));
    }

    lines
}

/// One card interior row: a `width`-wide cell buffer painted with `base` (so every cell carries the
/// explicit brand background), with each styled segment group placed at its interior column.
fn interior_row(
    width: usize,
    base: Style,
    groups: &[(usize, Vec<(String, Style)>)],
) -> Line<'static> {
    let mut cells = vec![(' ', base); width.max(1)];
    for (col, segments) in groups {
        place_segments(&mut cells, *col, segments);
    }
    cells_to_line(&cells)
}

/// Draw the first-run footer beneath the card: a faint `press enter to start   ·   ? for help`
/// action line and, below it, the dim-orange `first run — let's get you set up` line, both centered.
fn render_footer(frame: &mut Frame, body: Rect, y: u16, theme: &ThemeColors) {
    if y.saturating_add(1) >= body.y + body.height {
        return;
    }
    let bg = Style::default().bg(theme.background);
    let action_area = Rect {
        x: body.x,
        y,
        width: body.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(footer_action_line(theme))
            .alignment(Alignment::Center)
            .style(bg),
        action_area,
    );
    let firstrun_area = Rect {
        x: body.x,
        y: y + 1,
        width: body.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(footer_firstrun_line(theme))
            .alignment(Alignment::Center)
            .style(bg),
        firstrun_area,
    );
}

/// The faint action/help footer line: `press enter to start   ·   ? for help` (labels FAINT, the
/// `·` separator GHOST).
#[must_use]
fn footer_action_line(theme: &ThemeColors) -> Line<'static> {
    let faint = Style::default().fg(theme.text_muted).bg(theme.background);
    let ghost = Style::default().fg(theme.ghost).bg(theme.background);
    Line::from(vec![
        Span::styled("press enter to start", faint),
        Span::styled(format!("   {}   ", glyphs::SEPARATOR), ghost),
        Span::styled("? for help", faint),
    ])
}

/// The dim-orange first-run footer line: `first run — let's get you set up`.
#[must_use]
fn footer_firstrun_line(theme: &ThemeColors) -> Line<'static> {
    Line::from(Span::styled(
        format!("first run {} let's get you set up", glyphs::EM_DASH),
        Style::default().fg(theme.border).bg(theme.background),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Status};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn app() -> App {
        App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            variant: "default".to_owned(),
            session: "new".to_owned(),
        })
    }

    fn buffer_text(buf: &Buffer) -> String {
        buf.content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<Vec<_>>()
            .concat()
    }

    fn render() -> Buffer {
        let a = app();
        let theme = ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_onboarding(frame, &a, frame.area(), &theme))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn first_run_true_only_without_config_and_credentials() {
        // A fresh config-home with no config file and no home resolves to onboarding.
        let tmp = std::env::temp_dir().join(format!("cortex-onb-{}", std::process::id()));
        let cfg = tmp.join("config");
        // No file created yet: the path does not exist.
        assert!(global_config_path(Some(&cfg), None).is_some_and(|p| !p.exists()));

        // Once a config file is written, the global-config check would see it.
        std::fs::create_dir_all(cfg.join("cortex")).unwrap();
        std::fs::write(cfg.join("cortex").join("config.toml"), "theme = \"dark\"\n").unwrap();
        assert!(
            global_config_path(Some(&cfg), None).is_some_and(|p| p.exists()),
            "an existing global config file is detected"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn global_config_path_prefers_xdg_then_home() {
        let xdg = Path::new("/x/cfg");
        let home = Path::new("/home/dev");
        assert_eq!(
            global_config_path(Some(xdg), Some(home)),
            Some(PathBuf::from("/x/cfg/cortex/config.toml")),
            "XDG_CONFIG_HOME wins"
        );
        assert_eq!(
            global_config_path(None, Some(home)),
            Some(PathBuf::from("/home/dev/.config/cortex/config.toml")),
            "falls back to $HOME/.config"
        );
        assert_eq!(global_config_path(None, None), None, "no config home resolvable");
    }

    #[test]
    fn card_title_is_the_authentic_version_never_the_mockup() {
        let title = card_title();
        assert_eq!(title, "Cortex CLI v0.1.0", "authentic binary version");
        assert!(!title.contains("v0.4.2"), "never the mockup version literal");
    }

    #[test]
    fn breadcrumb_reads_cortex_code_welcome() {
        // VAL-ONB-003: the top breadcrumb reads `✦ Cortex Code · welcome`.
        let buf = render();
        let header = row_text(&buf, 0);
        assert!(header.contains('✦'), "brand star on row 0:\n{header}");
        assert!(header.contains("Cortex Code"), "wordmark on row 0:\n{header}");
        assert!(header.contains("welcome"), "welcome crumb on row 0:\n{header}");
    }

    #[test]
    fn renders_rounded_card_with_authentic_version_title() {
        // VAL-ONB-004/005: a rounded card whose title is the authentic `Cortex CLI v0.1.0`.
        let buf = render();
        let text = buffer_text(&buf);
        assert!(text.contains('╭') && text.contains('╮'), "rounded top corners");
        assert!(text.contains('╰') && text.contains('╯'), "rounded bottom corners");
        assert!(text.contains("Cortex CLI v0.1.0"), "authentic card title:\n{text}");
        assert!(!text.contains("v0.4.2"), "never the mockup version");
    }

    #[test]
    fn renders_mascot_welcome_and_subtitle() {
        // VAL-ONB-006/007: the brain mascot block-art, a `Welcome!` heading, and a subtitle.
        let buf = render();
        let text = buffer_text(&buf);
        assert!(text.contains("██ ▌  ▐ ██"), "brain mascot block-art:\n{text}");
        assert!(text.contains("Welcome!"), "welcome heading");
        assert!(
            text.contains("Your AI-powered coding assistant."),
            "subtitle line:\n{text}"
        );
    }

    #[test]
    fn renders_three_bulleted_tips() {
        // VAL-ONB-008: exactly three tips, each prefixed by a `•` bullet.
        let buf = render();
        let text = buffer_text(&buf);
        for tip in TIPS {
            assert!(text.contains(tip), "tip `{tip}` present:\n{text}");
        }
        assert_eq!(text.matches('•').count(), 3, "exactly three bullet markers");
    }

    #[test]
    fn renders_first_run_footer() {
        // VAL-ONB-009: a faint action line and a dim-orange first-run line beneath the card.
        let buf = render();
        let text = buffer_text(&buf);
        assert!(text.contains("press enter to start"), "action line:\n{text}");
        assert!(text.contains("? for help"), "help hint on the action line");
        assert!(
            text.contains("first run") && text.contains("let's get you set up"),
            "first-run footer line:\n{text}"
        );
    }

    #[test]
    fn status_bar_persists_with_real_model() {
        // VAL-ONB-010: the 3-zone status bar remains on the bottom row with the real model id.
        let buf = render();
        let status = row_text(&buf, 29);
        assert!(status.contains("? for help"), "status bar left zone:\n{status}");
        assert!(
            status.contains("allow all commands"),
            "status bar center zone:\n{status}"
        );
        assert!(
            status.contains("google/gemini-2.5-flash"),
            "status bar right zone shows the real model id:\n{status}"
        );
        assert!(!status.contains("opus-4.8"), "never the mockup model id");
    }

    #[test]
    fn card_tokens_are_brand_pure() {
        // VAL-ONB-012: the mascot is ORANGE and the tip bullets are dim-orange (ORANGE_D); no
        // non-orange chrome hue on the card.
        let theme = ThemeColors::from_name("dark");
        let lines = card_interior_lines(56, &theme);
        // A mascot block cell carries the brand accent.
        let mascot_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains('▐'))
            .expect("mascot span");
        assert_eq!(mascot_span.style.fg, Some(theme.primary), "mascot is ORANGE");
        // The bullet is dim-orange.
        let bullet_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content == glyphs::BULLET)
            .expect("bullet span");
        assert_eq!(bullet_span.style.fg, Some(theme.border), "bullet is ORANGE_D");
    }

    #[test]
    fn onboarding_uses_only_safe_glyphs() {
        // VAL-ONB-012: no tofu/forbidden glyphs (braille, thin triangles, `⌘`, curly quotes, `>_`).
        let buf = render();
        let text = buffer_text(&buf);
        assert!(!glyphs::contains_braille(&text), "no braille anywhere");
        for forbidden in ['▸', '▾', '⌘', '⬡', '“', '”', '‘', '’'] {
            assert!(!text.contains(forbidden), "forbidden glyph {forbidden:?}");
        }
        assert!(!text.contains(">_"), "no `>_` cliché");
    }
}
