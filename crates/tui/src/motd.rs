//! The home / MOTD welcome card and its meta panels, rendered as the top block of the scrollback
//! buffer (not a separate full-screen empty state).
//!
//! On a fresh launch the transcript is empty, so [`push_home`] emits the welcome card followed by
//! two bordered meta panels at the top of the same buffer the conversation stacks into. The card
//! carries the neutral `✦ Cortex · v<version>` shell wordmark in its top border (VAL-CORTEX-UX-002:
//! never the copper `Cortex Code` plate), the brain mascot block-art, a real/dynamic greeting, and
//! a tip/caps row; the meta panels show the real working directory, organization, plan/provider,
//! and the active model id. Everything draws through theme tokens and the safe glyph set, and every
//! value is sourced from the real running binary (no `Mathis` / `opus-4.8` / `v0.4.2` mockup
//! placeholders).

use std::path::{Path, PathBuf};

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use tui_components::style::ThemeColors;

use crate::brand;
use crate::glyphs;

/// Left/right margin (columns) the card and panels are inset from the transcript edges.
const MARGIN: usize = 2;
/// Minimum transcript width at which the full bordered card is drawn; below this a compact
/// fallback keeps the home legible without breaking the frame.
const MIN_CARD_WIDTH: usize = 56;
/// Gap (columns) between the two meta panels.
const PANEL_GAP: usize = 2;
/// Label-field width inside the left (Directory/Org) meta panel.
const LEFT_LABEL_WIDTH: usize = 11;
/// Label-field width inside the right (Plan/Model) meta panel.
const RIGHT_LABEL_WIDTH: usize = 7;

/// The product version shown in the card's title border, e.g. `v0.1.0` — the real binary version.
#[must_use]
pub fn version_string() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// The real OS user for the greeting (never the mockup `Mathis`).
#[must_use]
pub fn current_user() -> String {
    std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("USERNAME").ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "user".to_owned())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty())
}

/// The real working directory, home-relative (`~/…`) when it lives under `$HOME`.
#[must_use]
pub fn current_dir_display() -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    home_relative(&cwd, home_dir().as_deref())
}

/// Render `cwd` home-relative (`~`, `~/rest`) when it lives under `home`, otherwise the absolute
/// path. Pure (no environment access) so it is unit-testable without mutating process state.
fn home_relative(cwd: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home {
        if cwd == home {
            return "~".to_owned();
        }
        if let Ok(rest) = cwd.strip_prefix(home) {
            return format!("~/{}", rest.display());
        }
    }
    cwd.display().to_string()
}

/// The organization label, derived from the real user (never the mockup `Mathis's Organization`).
#[must_use]
pub fn org_label(user: &str) -> String {
    format!("{user}'s Organization")
}

/// The plan/provider label: the default build routes `vendor/model` ids through the OpenRouter
/// gateway, while a bare id implies a local/direct provider. Real runtime context, not a mockup
/// subscription string.
#[must_use]
pub fn provider_label(model: &str) -> String {
    if model.contains('/') {
        "OpenRouter".to_owned()
    } else {
        "Local".to_owned()
    }
}

/// The greeting first line, e.g. `Welcome back, root`.
#[must_use]
pub fn greeting(user: &str) -> String {
    format!("Welcome back, {user}")
}

/// The composer placeholder shown on the home screen (distinct from the conversation placeholder).
pub const HOME_PLACEHOLDER: &str = "what should we build today…";

/// Push the MOTD welcome card as the top block of the scrollback buffer, followed by a separator
/// row. The card is always emitted (it is the persistent top of the single scroll buffer); the home
/// meta panels (Directory/Org, Plan/Model) are emitted only on the pristine home (`show_meta`), so
/// once the user starts typing or a conversation exists the card alone leads the buffer.
pub fn push_home(
    lines: &mut Vec<Line<'static>>,
    model: &str,
    width: usize,
    theme: &ThemeColors,
    show_meta: bool,
) {
    if width < MIN_CARD_WIDTH {
        push_compact_home(lines, model, theme, show_meta);
        return;
    }
    push_card(lines, theme, width);
    lines.push(Line::from(""));
    if show_meta {
        push_meta_panels(lines, model, width, theme);
    }
}

/// Compact fallback for very narrow terminals: the wordmark, mascot, and (on the pristine home) a
/// one-line model summary, drawn without the bordered frame so nothing overflows.
fn push_compact_home(
    lines: &mut Vec<Line<'static>>,
    model: &str,
    theme: &ThemeColors,
    show_meta: bool,
) {
    let mut title = brand::wordmark(theme);
    title
        .spans
        .push(Span::styled("  ·  ", Style::default().fg(theme.ghost)));
    title.spans.push(Span::styled(
        version_string(),
        Style::default().fg(theme.text_muted),
    ));
    lines.push(title);
    lines.push(Line::from(""));
    lines.extend(brand::mascot_lines(theme));
    lines.push(Line::from(""));
    let user = current_user();
    lines.push(Line::from(Span::styled(
        greeting(&user),
        Style::default()
            .fg(theme.text_bright)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "Your agentic coding terminal.",
        Style::default().fg(theme.text_dim),
    )));
    if show_meta {
        lines.push(Line::from(vec![
            Span::styled("Model ", Style::default().fg(theme.text_muted)),
            Span::styled(model.to_owned(), Style::default().fg(theme.text_bright)),
        ]));
    }
}

/// Push the 7-row welcome card: titled top border, blank, mascot rows with greeting/tip, bottom
/// border.
fn push_card(lines: &mut Vec<Line<'static>>, theme: &ThemeColors, width: usize) {
    lines.push(card_top(theme, width));
    lines.push(card_interior(theme, width, &[]));

    let user = current_user();
    let primary = Style::default().fg(theme.primary);
    let mascot = brand::MASCOT_LINES;

    // Row +2: mascot row 0 + greeting.
    lines.push(card_interior(
        theme,
        width,
        &[
            (2, vec![Span::styled(mascot[0].to_string(), primary)]),
            (
                15,
                vec![Span::styled(
                    greeting(&user),
                    Style::default()
                        .fg(theme.text_bright)
                        .add_modifier(Modifier::BOLD),
                )],
            ),
        ],
    ));
    // Row +3: mascot row 1 + subtitle.
    lines.push(card_interior(
        theme,
        width,
        &[
            (2, vec![Span::styled(mascot[1].to_string(), primary)]),
            (
                15,
                vec![Span::styled(
                    "Your agentic coding terminal.".to_owned(),
                    Style::default().fg(theme.text_dim),
                )],
            ),
        ],
    ));
    // Row +4: mascot row 2 only.
    lines.push(card_interior(
        theme,
        width,
        &[(2, vec![Span::styled(mascot[2].to_string(), primary)])],
    ));
    // Row +5: mascot row 3 + tip/caps row.
    lines.push(card_interior(
        theme,
        width,
        &[
            (2, vec![Span::styled(mascot[3].to_string(), primary)]),
            (15, tip_spans(theme)),
        ],
    ));

    lines.push(card_bottom(theme, width));
}

/// The tip/caps row: command caps in ORANGE, trailing labels in FAINT, spaced apart.
fn tip_spans(theme: &ThemeColors) -> Vec<Span<'static>> {
    let cap = Style::default().fg(theme.primary);
    let label = Style::default().fg(theme.text_muted);
    vec![
        Span::styled("/help", cap),
        Span::styled(" commands", label),
        Span::raw("   "),
        Span::styled(glyphs::KEY_TAB, cap),
        Span::styled(" autocomplete", label),
        Span::raw("   "),
        Span::styled("esc", cap),
        Span::styled(" cancel", label),
        Span::raw("   "),
        Span::styled("@", cap),
        Span::styled(" files", label),
    ]
}

/// The card's titled top border: `╭─ ✦ Cortex  ·  v0.1.0 ─…─╮`.
fn card_top(theme: &ThemeColors, width: usize) -> Line<'static> {
    let bs = Style::default().fg(theme.card_border);
    let card_w = width.saturating_sub(2 * MARGIN);
    let version = version_string();
    // Title content width: ✦(1) + ' '(1) + wordmark + "  ·  "(5) + version.
    let title_w = 1 + 1 + glyphs_count(brand::WORDMARK_NAME) + 5 + version.chars().count();
    // Inner (between corners) = card_w - 2. Used by "─ "(2) + title + " "(1) + fill.
    let inner = card_w.saturating_sub(2);
    let fill = inner.saturating_sub(2 + title_w + 1);

    let spans = vec![
        Span::raw(" ".repeat(MARGIN)),
        Span::styled(glyphs::BOX_TOP_LEFT, bs),
        Span::styled(format!("{} ", glyphs::BOX_HORIZONTAL), bs),
        Span::styled(
            glyphs::BRAND_STAR,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            brand::WORDMARK_NAME,
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(theme.ghost)),
        Span::styled(version, Style::default().fg(theme.text_muted)),
        Span::raw(" "),
        Span::styled(glyphs::BOX_HORIZONTAL.repeat(fill), bs),
        Span::styled(glyphs::BOX_TOP_RIGHT, bs),
    ];
    Line::from(spans)
}

/// The card's bottom border: `╰─…─╯`.
fn card_bottom(theme: &ThemeColors, width: usize) -> Line<'static> {
    let bs = Style::default().fg(theme.card_border);
    let inner = width.saturating_sub(2 * MARGIN).saturating_sub(2);
    Line::from(vec![
        Span::raw(" ".repeat(MARGIN)),
        Span::styled(glyphs::BOX_BOTTOM_LEFT, bs),
        Span::styled(glyphs::BOX_HORIZONTAL.repeat(inner), bs),
        Span::styled(glyphs::BOX_BOTTOM_RIGHT, bs),
    ])
}

/// An interior card row: `│` + positioned span groups (offsets relative to the interior left edge)
/// padded to the interior width + `│`. Span groups are clipped so the right border stays aligned.
fn card_interior(
    theme: &ThemeColors,
    width: usize,
    groups: &[(usize, Vec<Span<'static>>)],
) -> Line<'static> {
    let bs = Style::default().fg(theme.card_border);
    let inner = width.saturating_sub(2 * MARGIN).saturating_sub(2);
    let mut spans = vec![
        Span::raw(" ".repeat(MARGIN)),
        Span::styled(glyphs::BOX_VERTICAL, bs),
    ];
    let mut col = 0usize;
    for (offset, group) in groups {
        if *offset > col {
            let gap = (*offset - col).min(inner.saturating_sub(col));
            if gap > 0 {
                spans.push(Span::raw(" ".repeat(gap)));
                col += gap;
            }
        }
        for span in group {
            if col >= inner {
                break;
            }
            let w = span.content.chars().count();
            if col + w <= inner {
                spans.push(span.clone());
                col += w;
            } else {
                let take = inner - col;
                let clipped: String = span.content.chars().take(take).collect();
                spans.push(Span::styled(clipped, span.style));
                col = inner;
            }
        }
    }
    if inner > col {
        spans.push(Span::raw(" ".repeat(inner - col)));
    }
    spans.push(Span::styled(glyphs::BOX_VERTICAL, bs));
    Line::from(spans)
}

/// Push the two side-by-side meta panels (left: Directory/Org, right: Plan/Model) with real data.
fn push_meta_panels(
    lines: &mut Vec<Line<'static>>,
    model: &str,
    width: usize,
    theme: &ThemeColors,
) {
    let card_w = width.saturating_sub(2 * MARGIN);
    let user = current_user();
    let cwd = current_dir_display();
    let org = org_label(&user);
    let plan = provider_label(model);

    // Size the right panel to fit the longest of its values (the real model id can be long), then
    // give the rest to the left panel.
    let right_value_w = model.chars().count().max(plan.chars().count());
    let right_need = 2 + 1 + RIGHT_LABEL_WIDTH + right_value_w + 1;
    let right_w = right_need.clamp(24, card_w.saturating_sub(24).max(24));
    let left_w = card_w.saturating_sub(PANEL_GAP + right_w).max(24);

    let bright = Style::default().fg(theme.text_bright);
    let amber = Style::default().fg(theme.secondary);

    let left_rows = [
        panel_top(left_w, theme),
        panel_kv(left_w, "Directory", LEFT_LABEL_WIDTH, &cwd, bright, theme),
        panel_kv(left_w, "Org", LEFT_LABEL_WIDTH, &org, bright, theme),
        panel_bottom(left_w, theme),
    ];
    let right_rows = [
        panel_top(right_w, theme),
        panel_kv(right_w, "Plan", RIGHT_LABEL_WIDTH, &plan, amber, theme),
        panel_kv(right_w, "Model", RIGHT_LABEL_WIDTH, model, bright, theme),
        panel_bottom(right_w, theme),
    ];

    for (left, right) in left_rows.into_iter().zip(right_rows) {
        let mut spans = vec![Span::raw(" ".repeat(MARGIN))];
        spans.extend(left.spans);
        spans.push(Span::raw(" ".repeat(PANEL_GAP)));
        spans.extend(right.spans);
        lines.push(Line::from(spans));
    }
}

fn panel_top(w: usize, theme: &ThemeColors) -> Line<'static> {
    let bs = Style::default().fg(theme.card_border);
    Line::from(vec![
        Span::styled(glyphs::BOX_TOP_LEFT, bs),
        Span::styled(glyphs::BOX_HORIZONTAL.repeat(w.saturating_sub(2)), bs),
        Span::styled(glyphs::BOX_TOP_RIGHT, bs),
    ])
}

fn panel_bottom(w: usize, theme: &ThemeColors) -> Line<'static> {
    let bs = Style::default().fg(theme.card_border);
    Line::from(vec![
        Span::styled(glyphs::BOX_BOTTOM_LEFT, bs),
        Span::styled(glyphs::BOX_HORIZONTAL.repeat(w.saturating_sub(2)), bs),
        Span::styled(glyphs::BOX_BOTTOM_RIGHT, bs),
    ])
}

/// A `│ label   value …│` panel row: label FAINT in a fixed field so values align, value in
/// `value_style`, clipped (with an ellipsis) when it would overrun the frame.
fn panel_kv(
    w: usize,
    label: &str,
    label_width: usize,
    value: &str,
    value_style: Style,
    theme: &ThemeColors,
) -> Line<'static> {
    let bs = Style::default().fg(theme.card_border);
    let inner = w.saturating_sub(2);
    // Leading space + label field + value, padded to the interior width.
    let label_field = format!("{label:<label_width$}");
    let used_before_value = 1 + label_field.chars().count();
    let avail = inner.saturating_sub(used_before_value);
    let value_disp = clip(value, avail);
    let value_w = value_disp.chars().count();
    let pad = inner.saturating_sub(used_before_value + value_w);

    Line::from(vec![
        Span::styled(glyphs::BOX_VERTICAL, bs),
        Span::raw(" "),
        Span::styled(label_field, Style::default().fg(theme.text_muted)),
        Span::styled(value_disp, value_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(glyphs::BOX_VERTICAL, bs),
    ])
}

/// Clip `s` to `max` columns, appending an ellipsis when truncated.
fn clip(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str(glyphs::ELLIPSIS);
    out
}

fn glyphs_count(s: &str) -> usize {
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn rendered(lines: &[Line]) -> String {
        lines.iter().map(plain).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn version_is_the_real_binary_version_never_the_mockup() {
        let v = version_string();
        assert_eq!(v, format!("v{}", env!("CARGO_PKG_VERSION")));
        assert_eq!(v, "v0.1.0", "the real workspace version");
        assert_ne!(v, "v0.4.2", "never the mockup version literal");
    }

    #[test]
    fn provider_label_reflects_the_routing() {
        assert_eq!(provider_label("google/gemini-2.5-flash"), "OpenRouter");
        assert_eq!(provider_label("local-model"), "Local");
    }

    #[test]
    fn current_dir_is_home_relative_when_under_home() {
        let home = Path::new("/home/dev");
        assert_eq!(
            home_relative(home, Some(home)),
            "~",
            "home itself collapses to ~"
        );
        assert_eq!(
            home_relative(Path::new("/home/dev/projects/x"), Some(home)),
            "~/projects/x",
            "a path under home is shown relative to ~"
        );
        assert_eq!(
            home_relative(Path::new("/srv/app"), Some(home)),
            "/srv/app",
            "a path outside home stays absolute"
        );
        assert_eq!(
            home_relative(Path::new("/srv/app"), None),
            "/srv/app",
            "with no home the absolute path is used"
        );
    }

    #[test]
    fn home_card_title_has_star_wordmark_and_real_version() {
        let theme = ThemeColors::dark();
        let mut lines = Vec::new();
        push_home(&mut lines, "google/gemini-2.5-flash", 92, &theme, true);
        let text = rendered(&lines);
        assert!(
            text.contains('✦'),
            "brand star in the title border:\n{text}"
        );
        assert!(
            text.contains(brand::WORDMARK_NAME),
            "neutral shell wordmark name:\n{text}"
        );
        assert!(
            text.contains("Cortex"),
            "neutral Cortex shell identity:\n{text}"
        );
        // VAL-CORTEX-UX-002: primary home MOTD must not show the copper `Cortex Code` plate.
        assert!(
            !text.contains("Cortex Code"),
            "bans retired Cortex Code wordmark plate:\n{text}"
        );
        assert!(
            !text.contains("✦ Cortex Code"),
            "bans ✦ Cortex Code MOTD title:\n{text}"
        );
        assert!(text.contains("v0.1.0"), "real version:\n{text}");
        assert!(!text.contains("v0.4.2"), "never the mockup version");
    }

    #[test]
    fn home_card_title_uses_brand_tokens() {
        let theme = ThemeColors::dark();
        let top = card_top(&theme, 92);
        let star = top
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "✦")
            .expect("star span");
        assert_eq!(star.style.fg, Some(theme.primary), "star is brand ORANGE");
        let name = top
            .spans
            .iter()
            .find(|s| s.content.as_ref() == brand::WORDMARK_NAME)
            .expect("name span");
        assert_eq!(name.content.as_ref(), "Cortex");
        assert_eq!(name.style.fg, Some(theme.text_bright), "name is BRIGHT");
        // The frame uses the dim card border, not the brighter active border.
        let corner = top
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "╭")
            .expect("corner span");
        assert_eq!(corner.style.fg, Some(theme.card_border));
    }

    #[test]
    fn home_renders_the_brain_mascot_in_orange() {
        let theme = ThemeColors::dark();
        let mut lines = Vec::new();
        push_home(&mut lines, "google/gemini-2.5-flash", 92, &theme, true);
        let text = rendered(&lines);
        assert!(
            text.contains("██ ▌  ▐ ██"),
            "the mascot block-art renders:\n{text}"
        );
        // A mascot block cell carries the brand accent.
        let block = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains('▐'));
        assert_eq!(
            block.and_then(|s| s.style.fg),
            Some(theme.primary),
            "mascot is the brand accent"
        );
    }

    #[test]
    fn home_greeting_uses_the_real_user_two_lines() {
        let theme = ThemeColors::dark();
        let mut lines = Vec::new();
        push_home(&mut lines, "google/gemini-2.5-flash", 92, &theme, true);
        let text = rendered(&lines);
        let user = current_user();
        assert!(
            text.contains(&format!("Welcome back, {user}")),
            "greeting uses the real user `{user}`:\n{text}"
        );
        assert!(
            text.contains("Your agentic coding terminal."),
            "second greeting line:\n{text}"
        );
        assert!(!text.contains("Mathis"), "never the mockup user literal");
    }

    #[test]
    fn home_tip_row_has_caps_and_labels() {
        let theme = ThemeColors::dark();
        let mut lines = Vec::new();
        push_home(&mut lines, "google/gemini-2.5-flash", 92, &theme, true);
        let text = rendered(&lines);
        for fragment in [
            "/help",
            "commands",
            "⇥",
            "autocomplete",
            "esc",
            "cancel",
            "@",
            "files",
        ] {
            assert!(text.contains(fragment), "tip row has `{fragment}`:\n{text}");
        }
        // Caps are ORANGE; the trailing label is FAINT.
        let tip = tip_spans(&theme);
        let help = tip
            .iter()
            .find(|s| s.content.as_ref() == "/help")
            .expect("/help cap");
        assert_eq!(help.style.fg, Some(theme.primary));
        let commands = tip
            .iter()
            .find(|s| s.content.as_ref() == " commands")
            .expect("commands label");
        assert_eq!(commands.style.fg, Some(theme.text_muted));
    }

    #[test]
    fn home_meta_panels_show_real_dynamic_values() {
        let theme = ThemeColors::dark();
        let mut lines = Vec::new();
        push_home(&mut lines, "google/gemini-2.5-flash", 92, &theme, true);
        let text = rendered(&lines);
        for fragment in ["Directory", "Org", "Plan", "Model"] {
            assert!(text.contains(fragment), "meta label `{fragment}`:\n{text}");
        }
        assert!(
            text.contains("google/gemini-2.5-flash"),
            "the real model id is shown, not `opus-4.8`:\n{text}"
        );
        assert!(!text.contains("opus-4.8"), "never the mockup model literal");
        assert!(
            text.contains(&current_dir_display()),
            "real cwd shown:\n{text}"
        );
    }

    #[test]
    fn card_only_home_omits_the_meta_panels() {
        // With `show_meta = false` (typing / active conversation) the welcome card still leads the
        // buffer, but the Directory/Org/Plan/Model meta panels are not emitted, leaving the region
        // below the card empty for the transcript.
        let theme = ThemeColors::dark();
        let mut lines = Vec::new();
        push_home(&mut lines, "google/gemini-2.5-flash", 92, &theme, false);
        let text = rendered(&lines);
        assert!(
            text.contains(brand::WORDMARK_NAME),
            "card wordmark still present:\n{text}"
        );
        assert!(
            !text.contains("Cortex Code"),
            "VAL-CORTEX-UX-002: no Cortex Code plate without show_meta:\n{text}"
        );
        assert!(
            text.contains("Welcome back,"),
            "card greeting still present:\n{text}"
        );
        for fragment in ["Directory", "Org", "Plan ", "Model "] {
            assert!(
                !text.contains(fragment),
                "meta panel `{fragment}` must be absent without show_meta:\n{text}"
            );
        }
    }

    #[test]
    fn meta_panel_rows_are_border_balanced() {
        // Every panel row begins and ends with a vertical border glyph and the two panels keep a
        // gap between them (no collision).
        let theme = ThemeColors::dark();
        let mut lines = Vec::new();
        push_meta_panels(&mut lines, "google/gemini-2.5-flash", 92, &theme);
        assert_eq!(
            lines.len(),
            4,
            "two 4-row panels rendered as 4 combined rows"
        );
        for line in &lines {
            let text = plain(line);
            assert!(
                text.matches('│').count() >= 4 || text.contains('╭') || text.contains('╰'),
                "panel row has paired borders:\n{text}"
            );
        }
    }
}
