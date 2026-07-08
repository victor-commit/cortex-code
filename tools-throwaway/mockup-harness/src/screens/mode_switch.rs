//! Screen 1 — the mode picker framed as a live CLI turn: the user runs
//! `/mode agent` to request a promotion, the agent streams the mode roster, the
//! per-mode tool-grant matrix (marking the deltas this switch unlocks) and the
//! permission flow, then asks to confirm the switch. The composer awaits.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{
    self, c, dot, m, t, ACCENT, DIM, FAINT, GHOST, INFO, MUTED, OK, TITLE, TXT, WARN,
};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "/mode agent",
        "confirm to switch to Agent Mode \u{2014} or type a mode name to pick another\u{2026}",
        &[
            ("enter", "confirm switch"),
            ("\u{2191}\u{2193}", "pick mode"),
            ("?", "grants"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/mode agent", "14:24:02"));
    lines.push(Line::from(""));

    // --- promotion summary ---
    lines.push(chrome::agent_head(
        "Mode switch \u{2014} Edit \u{2192} Agent \u{00b7} promotion requested",
    ));
    lines.push(Line::from(vec![
        m("   current  "),
        t("Edit Mode \u{00b7} 9 tools"),
        m("      requested  "),
        c("Agent Mode \u{00b7} 11 tools", TITLE),
        m("      delta  "),
        c("+2 grants", ACCENT),
    ]));
    lines.push(Line::from(vec![
        m("   policy   "),
        c("auto-grant within allow-rules", TITLE),
        m("  \u{00b7}  scoped to this session  \u{00b7}  revertible with "),
        t("/mode edit"),
    ]));
    lines.push(Line::from(""));

    // --- mode roster ---
    lines.push(chrome::agent_head(
        "Modes \u{2014} the runtime grants your tools per mode",
    ));
    lines.push(mode_row("Plan Mode", 8, "read-only inspection", false));
    lines.push(mode_row("Edit Mode", 9, "read + write + run", false));
    lines.push(mode_row(
        "Agent Mode",
        11,
        "read + write + run + subagents",
        true,
    ));
    lines.push(mode_row("Chat Mode", 6, "interactive Q&A", false));
    lines.push(mode_row("Headless", 10, "non-interactive single-shot", false));
    lines.push(Line::from(""));

    // --- tool-grant matrix ---
    lines.push(chrome::agent_head(
        "Tool grants by mode   (\u{0394} = unlocked by this switch)",
    ));
    lines.push(matrix_head());
    lines.push(matrix_rule());
    let rows: [(&str, [&str; 5], bool, &str); 10] = [
        ("Find files", ["yes", "yes", "yes", "yes", "yes"], false, "by name or glob"),
        ("Search code", ["yes", "yes", "yes", "yes", "yes"], false, "regex across the tree"),
        ("Read file", ["yes", "yes", "yes", "yes", "yes"], false, "a specific line range"),
        ("Run shell", ["ro", "gated", "gated", "\u{2014}", "gated"], false, "execute commands"),
        ("Edit file", ["\u{2014}", "yes", "yes", "\u{2014}", "yes"], false, "change a line range"),
        ("Write file", ["\u{2014}", "yes", "yes", "\u{2014}", "yes"], false, "create or overwrite"),
        ("Web lookup", ["gated", "gated", "gated", "gated", "gated"], false, "fetch external docs"),
        ("Ask user", ["yes", "yes", "yes", "yes", "\u{2014}"], false, "prompt for input"),
        ("Plan note", ["yes", "\u{2014}", "yes", "\u{2014}", "yes"], true, "record plan steps"),
        (
            "Spawn subagent",
            ["\u{2014}", "\u{2014}", "yes", "\u{2014}", "yes"],
            true,
            "launch scoped agents",
        ),
    ];
    for (tool, vals, delta, purpose) in rows {
        lines.push(matrix_row(tool, vals, delta, purpose));
    }
    lines.push(Line::from(""));

    // --- permission flow + legend ---
    lines.push(chrome::agent_head("Promotion & permission flow"));
    lines.push(Line::from(vec![
        m("   "),
        step("1"),
        t("request "),
        m("\u{2192}  "),
        step("2"),
        t("policy check "),
        m("\u{2192}  "),
        step("3"),
        t("grant / gate "),
        m("\u{2192}  "),
        step("4"),
        t("scoped grant"),
    ]));
    lines.push(Line::from(vec![
        m("   legend   "),
        c("yes", OK),
        m(" granted   "),
        c("gated", WARN),
        m(" needs confirmation   "),
        c("ro", INFO),
        m(" read-only   "),
        c("\u{2014}", FAINT),
        m(" not granted   "),
        c("\u{0394}", ACCENT),
        m(" new here"),
    ]));
    lines.push(Line::from(""));

    // --- pending confirmation (hugs the composer) ---
    lines.push(chrome::agent_head(
        "Grant Plan note + Spawn subagent for this session? Confirm to switch to Agent Mode.",
    ));

    chrome::transcript(f, body, lines);
}

/// A roster row: `‹marker› Name   N tools   description   ● selected`. The layout
/// contract (marker/name/tools/description column widths) must stay in lockstep
/// with the matrix header spacing above, so the widths are pinned here.
fn mode_row(name: &str, tools: u8, grant: &str, active: bool) -> Line<'static> {
    let marker = if active {
        Span::styled(
            "   \u{203a} ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("   \u{25cb} ", Style::default().fg(FAINT))
    };
    let name_span = if active {
        Span::styled(
            format!("{name:<11}"),
            Style::default().fg(TITLE).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!("{name:<11}"), Style::default().fg(TXT))
    };
    let tools_col = Span::styled(
        format!("{tools:>2} tools   "),
        Style::default().fg(if active { ACCENT } else { FAINT }),
    );
    let mut spans = vec![Span::raw("  "), marker, name_span, tools_col];
    if active {
        spans.push(Span::styled(
            format!("{grant:<32}"),
            Style::default().fg(TXT),
        ));
        spans.push(dot(OK));
        spans.push(c(" selected", OK));
    } else {
        spans.push(Span::styled(grant.to_string(), Style::default().fg(DIM)));
    }
    Line::from(spans)
}

fn matrix_head() -> Line<'static> {
    let cols = ["Plan", "Edit", "Agent", "Chat", "Head"];
    let mut head = vec![Span::styled(
        format!("   {:<16}", "tool"),
        Style::default().fg(MUTED),
    )];
    for col in cols {
        head.push(Span::styled(format!("{col:^8}"), Style::default().fg(MUTED)));
    }
    head.push(Span::styled(format!("{:^5}", "\u{0394}"), Style::default().fg(MUTED)));
    head.push(Span::styled("  purpose", Style::default().fg(MUTED)));
    Line::from(head)
}

fn matrix_rule() -> Line<'static> {
    Line::from(Span::styled(
        format!("   {}", "\u{2500}".repeat(90)),
        Style::default().fg(GHOST),
    ))
}

fn matrix_row(tool: &str, vals: [&str; 5], delta: bool, purpose: &str) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("   {tool:<16}"),
        Style::default().fg(if delta { TITLE } else { TXT }),
    )];
    for v in vals {
        spans.push(cell(v));
    }
    if delta {
        spans.push(Span::styled(
            format!("{:^5}", "\u{0394}"),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw("     "));
    }
    spans.push(Span::styled(format!("  {purpose}"), Style::default().fg(DIM)));
    Line::from(spans)
}

fn cell(v: &str) -> Span<'static> {
    let color = match v {
        "yes" => OK,
        "gated" => WARN,
        "ro" => INFO,
        "\u{2014}" => FAINT,
        _ => MUTED,
    };
    Span::styled(format!("{v:^8}"), Style::default().fg(color))
}

fn step(n: &str) -> Span<'static> {
    Span::styled(
        format!("{n} "),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )
}
