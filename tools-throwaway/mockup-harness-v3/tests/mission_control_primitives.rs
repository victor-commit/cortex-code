//! Unit proof for Mission Control density primitives (Phase A enablement).
//!
//! Synthetic panel layouts (not full screens) bind counter formatting, live
//! status markers, zoned geometry, compressed log rows, and › selection without
//! nested brackets, filled statusline bars, or floating popups — all inside
//! the 120×40 canvas budget.

use mockup_harness_v3::chrome::{
    components::{contains_bracket_key_atom, validate_component, Component, ShortcutByline},
    layout::{StatusLine, CANVAS_COLUMNS, CANVAS_ROWS},
    mission_control::{
        format_fraction, format_page_range, CompressedLog, CompressedLogLine, LiveState,
        MissionLocalKeyStrip, PanelCounter, PanelHeader, SelectionSurface, SelectionSurfaceRow,
        StatusBand, Zone, ZoneBody, ZonedPanel,
    },
    tokens::{validate_glyphs, TextStyle, ACCENT_USE_BUDGET},
};

fn joined_text(component: &impl Component) -> String {
    component
        .lines()
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_linelike_within_canvas(lines: &[impl AsRef<str>]) {
    assert!(
        lines.len() <= CANVAS_ROWS as usize,
        "synthetic panel exceeds canvas rows: {} > {}",
        lines.len(),
        CANVAS_ROWS
    );
    for line in lines {
        let width = line.as_ref().chars().count();
        assert!(
            width <= CANVAS_COLUMNS as usize,
            "line exceeds canvas columns ({width} > {CANVAS_COLUMNS}): {}",
            line.as_ref()
        );
        validate_glyphs(line.as_ref()).expect("synthetic panel uses only allowed glyphs");
        assert!(
            !contains_bracket_key_atom(line.as_ref()),
            "bracket key atom leaked into mission-control primitive: {}",
            line.as_ref()
        );
    }
}

#[test]
fn counter_formatting_matches_mission_control_patterns() {
    assert_eq!(format_fraction(16, 22), "16/22");
    assert_eq!(format_fraction(0, 0), "0/0");
    assert_eq!(format_fraction(1, 3), "1/3");
    assert_eq!(format_page_range(1, 12, 124), "1-12 of 124");
    assert_eq!(format_page_range(13, 24, 124), "13-24 of 124");
    assert_eq!(format_page_range(1, 1, 1), "1-1 of 1");
}

#[test]
fn status_band_renders_live_marker_with_fraction_and_bounded_accent() {
    let band = StatusBand::new(LiveState::Running)
        .with_stage("Preparing release")
        .with_fraction(16, 22)
        .expect("valid fraction");
    validate_component(&band).expect("status band anatomy stays valid");
    assert!(band.anatomy().accent_uses <= ACCENT_USE_BUDGET);
    assert_eq!(band.anatomy().accent_uses, 1, "live marker alone may use accent");

    let text = joined_text(&band);
    assert!(text.contains('●'), "running state uses live marker");
    assert!(text.contains("Running"), "status word stays readable without color");
    assert!(text.contains("Preparing release"));
    assert!(text.contains("16/22"));
    assert!(!text.contains('['), "no nested brackets");
    assert_linelike_within_canvas(&band.lines().iter().map(|l| l.text.as_str()).collect::<Vec<_>>());

    // Waiting / complete markers stay semantic, still no filled statusline.
    let waiting = StatusBand::new(LiveState::Waiting).with_stage("Queued next task");
    assert!(joined_text(&waiting).contains('○'));
    let complete = StatusBand::new(LiveState::Complete).with_fraction(3, 3).unwrap();
    assert!(joined_text(&complete).contains('✓'));
    assert!(joined_text(&complete).contains("3/3"));
}

#[test]
fn panel_header_carries_counters() {
    let fraction = PanelHeader::new("Features").with_counter(PanelCounter::Fraction {
        done: 16,
        total: 22,
    });
    assert_eq!(joined_text(&fraction), "Features · 16/22");

    let page = PanelHeader::new("Progress Log").with_counter(PanelCounter::PageRange {
        start: 1,
        end: 12,
        total: 124,
    });
    assert_eq!(joined_text(&page), "Progress Log · 1-12 of 124");

    let count = PanelHeader::new("Active Worker #18").with_counter(PanelCounter::Label(
        "Time 8m 36s".to_owned(),
    ));
    assert_eq!(joined_text(&count), "Active Worker #18 · Time 8m 36s");

    validate_component(&fraction).unwrap();
    validate_component(&page).unwrap();
    validate_component(&count).unwrap();
}

#[test]
fn selection_surface_uses_chevron_without_nested_brackets() {
    let surface = SelectionSurface::new(
        vec![
            SelectionSurfaceRow::new("Prepare release notes", Some('✓')),
            SelectionSurfaceRow::new("Review findings", Some('●')),
            SelectionSurfaceRow::new("Ship the package", Some('○')),
        ],
        1,
    )
    .expect("selected row required")
    .with_selected_detail("Needs human sign-off before shipping");

    validate_component(&surface).unwrap();
    let lines: Vec<String> = surface.lines().into_iter().map(|l| l.text).collect();
    assert_linelike_within_canvas(&lines);

    assert!(
        lines.iter().any(|line| line.starts_with("› ● Review findings")),
        "selected row uses › plus status glyph, never nested brackets: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.starts_with("  ✓ Prepare")),
        "unselected complete rows keep status glyph without ›: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Needs human sign-off")),
        "detail expands under the selected row only"
    );
    for line in &lines {
        assert!(!contains_bracket_key_atom(line));
        assert!(!line.contains("[enter]"));
        assert!(!line.contains("› ["));
    }
}

#[test]
fn compressed_log_supports_relative_ages_and_pagination() {
    let log = CompressedLog::new(vec![
        CompressedLogLine::new("Execute · cargo test --features png").with_age("8h ago"),
        CompressedLogLine::new("Read src/chrome/components.rs (142 lines)").with_age("7h ago"),
        CompressedLogLine::new("Edit src/chrome/mission_control.rs").with_age("4m ago"),
    ])
    .with_page(1, 3, 124);

    validate_component(&log).unwrap();
    let lines: Vec<String> = log.lines().into_iter().map(|l| l.text).collect();
    assert_linelike_within_canvas(&lines);
    assert!(lines[0].contains("Progress") || lines[0].contains("1-3 of 124"));
    assert!(lines.iter().any(|l| l.contains("8h ago")));
    assert!(lines.iter().any(|l| l.contains("4m ago")));
    assert!(lines.iter().any(|l| l.contains("Execute")));
}

#[test]
fn zoned_panel_exposes_active_work_and_checklist_log_geometry() {
    let panel = ZonedPanel::new(vec![
        Zone::new(
            PanelHeader::new("Active Feature"),
            ZoneBody::Lines(vec![
                "Prepare the release safely and review the result.".to_owned(),
                "Stage · Running".to_owned(),
            ]),
        ),
        Zone::new(
            PanelHeader::new("Features").with_counter(PanelCounter::Fraction {
                done: 16,
                total: 22,
            }),
            ZoneBody::Selection(
                SelectionSurface::new(
                    vec![
                        SelectionSurfaceRow::new("Chrome language kill-switch", Some('✓')),
                        SelectionSurfaceRow::new("Mission Control primitives", Some('●')),
                        SelectionSurfaceRow::new("Phase-A composer rewrite", Some('○')),
                    ],
                    1,
                )
                .unwrap(),
            ),
        ),
        Zone::new(
            PanelHeader::new("Progress Log").with_counter(PanelCounter::PageRange {
                start: 1,
                end: 12,
                total: 124,
            }),
            ZoneBody::Log(
                CompressedLog::new(vec![
                    CompressedLogLine::new("Execute · init isolation snapshot").with_age("2m ago"),
                    CompressedLogLine::new("Read design-redesign-reference-analysis.md")
                        .with_age("1m ago"),
                ]),
            ),
        ),
    ]);

    validate_component(&panel).unwrap();
    let lines: Vec<String> = panel.lines().into_iter().map(|l| l.text).collect();
    assert_linelike_within_canvas(&lines);

    // Zone geometry: headers present, counter-bearing, zones sequential.
    let header_idxs: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            if line.starts_with("Active Feature")
                || line.starts_with("Features ·")
                || line.starts_with("Progress Log ·")
            {
                Some(idx)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        header_idxs.len(),
        3,
        "three zone headers expected: {lines:?}"
    );
    assert!(header_idxs[0] < header_idxs[1] && header_idxs[1] < header_idxs[2]);
    assert!(lines.iter().any(|l| l.contains("16/22")));
    assert!(lines.iter().any(|l| l.contains("1-12 of 124")));
    assert!(lines.iter().any(|l| l.contains("› ● Mission Control")));
}

#[test]
fn synthetic_mission_panel_snapshot_fits_120x40_and_avoids_forbidden_chrome() {
    // Builders compose StatusBand + ZonedPanel + optional local key strip.
    // This is not a published screen; it locks the chain for orchestration writers.
    let band = StatusBand::new(LiveState::Running)
        .with_stage("Active work")
        .with_fraction(16, 22)
        .unwrap();
    let panel = ZonedPanel::new(vec![
        Zone::new(
            PanelHeader::new("Active Feature"),
            ZoneBody::Lines(vec![
                "Add Mission Control density primitives for builders.".to_owned(),
                "Milestone · v3-design-redesign-phase-a".to_owned(),
            ]),
        ),
        Zone::new(
            PanelHeader::new("Features").with_counter(PanelCounter::Fraction {
                done: 16,
                total: 22,
            }),
            ZoneBody::Selection(
                SelectionSurface::new(
                    vec![
                        SelectionSurfaceRow::new("Chrome language kill-switch", Some('✓')),
                        SelectionSurfaceRow::new("Mission Control primitives", Some('●')),
                        SelectionSurfaceRow::new("Phase-A composer rewrite", Some('○')),
                        SelectionSurfaceRow::new("Phase-A orchestration rewrite", Some('○')),
                    ],
                    1,
                )
                .unwrap()
                .with_selected_detail("Status band, counters, zones, compressed log, selection"),
            ),
        ),
        Zone::new(
            PanelHeader::new("Progress Log").with_counter(PanelCounter::PageRange {
                start: 1,
                end: 4,
                total: 48,
            }),
            ZoneBody::Log(CompressedLog::new(vec![
                CompressedLogLine::new("Execute · cargo test mission_control_primitives")
                    .with_age("1m ago"),
                CompressedLogLine::new("Read library/design-redesign-reference-analysis.md")
                    .with_age("3m ago"),
                CompressedLogLine::new("Edit src/chrome/mission_control.rs").with_age("now"),
            ])),
        ),
    ]);
    let key_strip = MissionLocalKeyStrip::new(
        ShortcutByline::new(vec![
            mockup_harness_v3::chrome::components::ShortcutHint::new("Enter", "open progress")
                .unwrap(),
            mockup_harness_v3::chrome::components::ShortcutHint::new("i", "intervene").unwrap(),
            mockup_harness_v3::chrome::components::ShortcutHint::new("Esc", "back to chat")
                .unwrap(),
        ])
        .unwrap(),
    );

    let mut snapshot_lines: Vec<String> = Vec::new();
    snapshot_lines.push("Prepare the release safely".to_owned());
    snapshot_lines.extend(band.lines().into_iter().map(|l| l.text));
    snapshot_lines.push(String::new()); // visual zone separator (content, not filled bar)
    snapshot_lines.extend(panel.lines().into_iter().map(|l| l.text));
    snapshot_lines.push(String::new());
    snapshot_lines.extend(key_strip.lines().into_iter().map(|l| l.text));

    // Shared statusline policy: path · branch · model on canvas, never a filled strip.
    let statusline = StatusLine::new("~/code", "main", "Claude Opus 4.8").unwrap();
    assert_eq!(statusline.style().background, Some(mockup_harness_v3::chrome::tokens::ColorToken::Canvas));
    // Full canvas: content rows + trailing spacer(s) + final statusline row ≤ 40.
    while snapshot_lines.len() < (CANVAS_ROWS as usize - 1) {
        snapshot_lines.push(String::new());
    }
    snapshot_lines.push(statusline.text().to_owned());

    assert_eq!(snapshot_lines.len(), CANVAS_ROWS as usize);
    assert_linelike_within_canvas(&snapshot_lines);

    // Snapshot must prove counters + zones.
    let joined = snapshot_lines.join("\n");
    assert!(joined.contains("● Running"));
    assert!(joined.contains("16/22"));
    assert!(joined.contains("Features · 16/22"));
    assert!(joined.contains("Progress Log · 1-4 of 48"));
    assert!(joined.contains("› ● Mission Control primitives"));
    assert!(joined.contains("1m ago") || joined.contains("3m ago"));
    assert!(joined.contains("Enter to open progress · i to intervene · Esc to back to chat"));
    assert!(!joined.contains("[enter]"));
    assert!(!joined.contains("[esc]"));
    // No floating popup anatomy: no dual centered card frames.
    assert!(!joined.contains("╭─") || joined.matches('╭').count() <= 2);

    // TXT artifact line projection (source of truth shape for builders).
    let txt = snapshot_lines
        .iter()
        .map(|line| {
            let mut padded = line.clone();
            while padded.chars().count() < CANVAS_COLUMNS as usize {
                padded.push(' ');
            }
            padded
        })
        .collect::<Vec<_>>();
    assert!(txt.iter().all(|line| line.chars().count() == CANVAS_COLUMNS as usize));
    assert_eq!(txt.len(), 40);

    // Spot-check styles on status band marker use Accent foreground once.
    let band_lines = band.lines();
    assert!(
        band_lines
            .iter()
            .any(|line| line.style.foreground == mockup_harness_v3::chrome::tokens::ColorToken::Accent
                || line.style == TextStyle::accent()),
        "live marker weekend uses restrained ACCENT"
    );
}

#[test]
fn mission_local_key_strip_is_scoped_not_global_chat_footer() {
    // N-9: global always-on chat keybar is banned. Mission temporary detail may
    // carry a local byline confined to the mission surface.
    let strip = MissionLocalKeyStrip::new(
        ShortcutByline::enter_esc("open progress", "back to chat").unwrap(),
    );
    validate_component(&strip).unwrap();
    let text = joined_text(&strip);
    assert_eq!(text, "Enter to open progress · Esc to back to chat");
    assert!(!contains_bracket_key_atom(&text));
    assert!(strip.anatomy().has_local_keys);
    assert!(strip.anatomy().primary_action_count <= 1);
}
