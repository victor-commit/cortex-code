//! Red/green probes for monospaced PNG baseline registration
//! (VAL-V3-HARNESS-038, VAL-V3-HARNESS-040).
//!
//! These probes fail on bbox-centering placement and pass once each glyph is
//! placed at pen origin + CELL_BASELINE using fontdue xmin/ymin. They do not
//! pin full gallery SHA golden tables; that is deferred to the full re-render
//! feature.

#![cfg(feature = "png")]

use image::RgbImage;
use mockup_harness_v3::audit::{audit_screen, StyledCell};
use mockup_harness_v3::chrome::tokens::{
    embedded_fonts, TextStyle, CELL_BASELINE, CELL_HEIGHT, CELL_WIDTH, PNG_HEIGHT, PNG_WIDTH,
};
use mockup_harness_v3::scenario::render_audit_png;

const PROBE_ROW: u16 = 5;
const PROBE_COL_START: u16 = 4;
const MIXED: &str = "Agpijy.|";

/// True when a pixel is not the dark canvas background (any invisible alpha
/// bleed still counts as no-ink at threshold 30 of max channel delta).
fn is_ink(px: image::Rgb<u8>, canvas: image::Rgb<u8>) -> bool {
    let dr = (px[0] as i16 - canvas[0] as i16).unsigned_abs();
    let dg = (px[1] as i16 - canvas[1] as i16).unsigned_abs();
    let db = (px[2] as i16 - canvas[2] as i16).unsigned_abs();
    dr.max(dg).max(db) >= 30
}

/// Returns inclusive ink (top, bottom) inside one cell, or None if empty.
fn cell_ink_envelope(
    image: &RgbImage,
    column: u16,
    row: u16,
    canvas: image::Rgb<u8>,
) -> Option<(u32, u32)> {
    let origin_x = column as u32 * CELL_WIDTH;
    let origin_y = row as u32 * CELL_HEIGHT;
    // Allow 1px bleed outside the cell (box-drawing xmin can be -1).
    let x0 = origin_x.saturating_sub(1);
    let x1 = (origin_x + CELL_WIDTH + 1).min(image.width());
    let y0 = origin_y.saturating_sub(1);
    let y1 = (origin_y + CELL_HEIGHT + 1).min(image.height());
    let mut top = None;
    let mut bottom = None;
    for y in y0..y1 {
        for x in x0..x1 {
            if is_ink(*image.get_pixel(x, y), canvas) {
                top = Some(top.map_or(y, |t: u32| t.min(y)));
                bottom = Some(bottom.map_or(y, |b: u32| b.max(y)));
            }
        }
    }
    match (top, bottom) {
        (Some(t), Some(b)) => Some((t, b)),
        _ => None,
    }
}

/// Expected non-descender bottom (absolute image y) for baseline registration.
fn expected_flat_bottom(row: u16) -> u32 {
    // For glyphs with ymin == 0, bitmap bottom = pen_y - 1 = cell_y + BASELINE - 1.
    row as u32 * CELL_HEIGHT + CELL_BASELINE as u32 - 1
}

/// Clears the whole probe row so residual fixture glyphs cannot bleed into
/// envelope scans (narrow punctuation especially). Then places `text` starting
/// at `PROBE_COL_START`.
fn place_probe_string(audit: &mut mockup_harness_v3::audit::AuditDocument, text: &str) {
    for cell in audit.styled_cells.iter_mut() {
        if cell.row == PROBE_ROW {
            *cell = StyledCell {
                row: PROBE_ROW,
                column: cell.column,
                glyph: ' ',
                style: TextStyle::text(),
                absolute_fg: None,
                absolute_bg: None,
            };
        }
    }
    for (offset, glyph) in text.chars().enumerate() {
        let column = PROBE_COL_START + offset as u16;
        let cell = audit
            .styled_cells
            .iter_mut()
            .find(|cell| cell.row == PROBE_ROW && cell.column == column)
            .unwrap_or_else(|| panic!("missing probe cell row={PROBE_ROW} col={column}"));
        *cell = StyledCell {
            row: PROBE_ROW,
            column,
            glyph,
            style: TextStyle::text(),
            absolute_fg: None,
            absolute_bg: None,
        };
    }
}

fn rasterize_probe(text: &str, screen_id: &str) -> (RgbImage, image::Rgb<u8>) {
    let mut audit = audit_screen(screen_id, "png-baseline-probe")
        .expect("probe fixture must produce a valid audit");
    place_probe_string(&mut audit, text);
    // chat-turn forces synthetic underscore path for '_'; keep that for parity probe.
    audit.screen_id = screen_id.to_owned();
    audit
        .validate()
        .expect("probe glyphs must stay within audit policy");
    let bytes = render_audit_png(&audit).expect("probe row must rasterize");
    let image = image::load_from_memory(&bytes)
        .expect("probe raster must be a valid PNG")
        .to_rgb8();
    assert_eq!(image.dimensions(), (PNG_WIDTH, PNG_HEIGHT));
    let canvas = *image.get_pixel(0, 0);
    (image, canvas)
}

/// HARNESS-038: mixed-height latin (and punctuation) bottoms align with the shared
/// baseline envelope, and non-descenders share bottoms within 1px.
#[test]
fn mixed_height_latin_registers_to_shared_baseline() {
    let (image, canvas) = rasterize_probe(MIXED, "home");
    let cell_y = PROBE_ROW as u32 * CELL_HEIGHT;
    let expected_flat = expected_flat_bottom(PROBE_ROW);

    // Glyphs that sit on the baseline (no descent): A g? No — g/p/j/y descend.
    // Non-descenders in MIXED: A, i, . , |
    // Actually: A (ymin0), g(ymin-4), p(ymin-4), i(ymin0), j(ymin-4), y(ymin-4), .(ymin0), |(ymin-4)
    let non_descenders: &[(usize, char)] = &[(0, 'A'), (3, 'i'), (6, '.')];
    let descenders: &[(usize, char)] = &[(1, 'g'), (2, 'p'), (4, 'j'), (5, 'y'), (7, '|')];

    let mut non_descender_bottoms = Vec::new();
    for &(offset, ch) in non_descenders {
        let col = PROBE_COL_START + offset as u16;
        let (top, bottom) = cell_ink_envelope(&image, col, PROBE_ROW, canvas)
            .unwrap_or_else(|| panic!("glyph '{ch}' produced no ink"));
        // Must not sit mid-cell via the old centering formula.
        let old_center_like = top as i32 >= cell_y as i32 + 6 && bottom as i32 <= cell_y as i32 + 16
            && ch != 'A'; // loose shape — primary check is bottom envelope
        let _ = old_center_like;
        assert!(
            (bottom as i32 - expected_flat as i32).abs() <= 1,
            "non-descender '{ch}' bottom={bottom} expected≈{expected_flat} (CELL_BASELINE={CELL_BASELINE}); \
             bbox-centering would scatter bottoms"
        );
        non_descender_bottoms.push(bottom);
    }

    let min_b = *non_descender_bottoms.iter().min().unwrap();
    let max_b = *non_descender_bottoms.iter().max().unwrap();
    assert!(
        max_b - min_b <= 1,
        "non-descender bottoms must share a ≤1px envelope, got {min_b}..{max_b}"
    );

    // Descenders must extend past the flat baseline and still share bottoms
    // within 1px of each other (font metrics ymin is shared for g/p/y; j/| match).
    let mut descender_bottoms = Vec::new();
    for &(offset, ch) in descenders {
        let col = PROBE_COL_START + offset as u16;
        let (_top, bottom) = cell_ink_envelope(&image, col, PROBE_ROW, canvas)
            .unwrap_or_else(|| panic!("glyph '{ch}' produced no ink"));
        assert!(
            bottom > expected_flat,
            "descender '{ch}' must hang below baseline (bottom={bottom}, flat={expected_flat})"
        );
        descender_bottoms.push((ch, bottom));
    }
    let bottoms: Vec<u32> = descender_bottoms.iter().map(|(_, b)| *b).collect();
    let min_d = *bottoms.iter().min().unwrap();
    let max_d = *bottoms.iter().max().unwrap();
    assert!(
        max_d - min_d <= 1,
        "descender bottoms must share a ≤1px envelope, got {descender_bottoms:?}"
    );

    // Placement must use xmin (not horizontal bbox center). Narrow '.' must ink
    // near the cell left of its advance; its xmin=3 at size 16 is offset from origin.
    {
        let (regular, _) = embedded_fonts().expect("embedded DejaVuSansMono");
        let (metrics, _) = regular.rasterize('.', 16.0);
        let col = PROBE_COL_START + 6;
        let pen_x = col as i32 * CELL_WIDTH as i32;
        let expected_left = pen_x + metrics.xmin;
        // Find left-most ink of '.'
        let origin_x = col as u32 * CELL_WIDTH;
        let origin_y = cell_y;
        let mut ink_left = None;
        for x in origin_x..origin_x + CELL_WIDTH {
            for y in origin_y..origin_y + CELL_HEIGHT {
                if is_ink(*image.get_pixel(x, y), canvas) {
                    ink_left = Some(ink_left.map_or(x, |l: u32| l.min(x)));
                }
            }
        }
        let ink_left = ink_left.expect("dot must leave ink");
        assert!(
            (ink_left as i32 - expected_left).abs() <= 1,
            "'.' left ink must follow metrics.xmin placement (got {ink_left}, expected≈{expected_left}); \
             horizontal centering would place it mid-cell instead"
        );
    }
}

/// Proves production placement is origin+baseline, not legacy bbox centering.
///
/// Hardcoding a single absolute y for '.' is brittle: short punctuation has
/// noisy anti-alias coverage and neighbor-bleed risk under cell envelope scans.
/// Instead compare *dispersions*: mixed-height non-descender bottoms must share
/// a ≤1 px envelope under live rasterization, while the same metrics under
/// bbox-centering must scatter by more than that (and worse than baseline).
#[test]
fn rejects_legacy_bbox_centering_registration() {
    // Spaced non-descenders of different coverage heights (A≈12, w/m≈9, i≈13).
    // Spaces between glyphs keep envelope scans free of horizontal bleed.
    // Period inherits its baseline check from mixed_height (xmin + ≤1 bottom).
    let sample = "A w m i";
    let (image, canvas) = rasterize_probe(sample, "home");
    let expected_flat = expected_flat_bottom(PROBE_ROW);

    let mut baseline_bottoms: Vec<(char, u32)> = Vec::new();
    for (offset, ch) in sample.chars().enumerate() {
        if ch == ' ' {
            continue;
        }
        let col = PROBE_COL_START + offset as u16;
        let (_top, bottom) = cell_ink_envelope(&image, col, PROBE_ROW, canvas)
            .unwrap_or_else(|| panic!("glyph '{ch}' produced no ink"));
        baseline_bottoms.push((ch, bottom));
        assert!(
            (bottom as i32 - expected_flat as i32).abs() <= 1,
            "'{ch}' bottom={bottom} is offline for CELL_BASELINE (expected≈{expected_flat}); \
             production render_audit_png must keep origin+baseline placement"
        );
    }
    let basemin = baseline_bottoms.iter().map(|(_, b)| *b).min().unwrap();
    let basemax = baseline_bottoms.iter().map(|(_, b)| *b).max().unwrap();
    let baseline_dispersion = basemax - basemin;
    assert!(
        baseline_dispersion <= 1,
        "baseline-registered bottoms must share a ≤1px envelope; got {baseline_bottoms:?}"
    );

    // Counterfactual: same metrics under bbox-centering of each glyph bitmap
    // inside the cell. Heights differ (A=12, w=9, m=9, i=13) so centered
    // bottoms scatter; production must remain tighter than that scatter.
    let (regular, _) = embedded_fonts().expect("embedded fonts");
    let mut centered_bottoms: Vec<(char, i32)> = Vec::new();
    for ch in sample.chars().filter(|c| *c != ' ') {
        let (m, _) = regular.rasterize(ch, 16.0);
        let base_y =
            PROBE_ROW as i32 * CELL_HEIGHT as i32 + (CELL_HEIGHT as i32 - m.height as i32) / 2;
        let centered_bottom = base_y + m.height as i32 - 1;
        centered_bottoms.push((ch, centered_bottom));
    }
    let cmin = centered_bottoms.iter().map(|(_, b)| *b).min().unwrap();
    let cmax = centered_bottoms.iter().map(|(_, b)| *b).max().unwrap();
    let centered_dispersion = (cmax - cmin) as u32;
    assert!(
        centered_dispersion > 1,
        "fontdue metrics changed: centered bottoms no longer scatter ({centered_bottoms:?}); \
         re-check probe sensitivity"
    );
    assert!(
        baseline_dispersion < centered_dispersion,
        "origin+baseline must be tighter than bbox-centering; \
         baseline_dispersion={baseline_dispersion} centered_dispersion={centered_dispersion} \
         baseline={baseline_bottoms:?} centered={centered_bottoms:?}"
    );
}

/// HARNESS-040: synthetic underscore + elbow ink share CELL_BASELINE with latin.
#[test]
fn synthetic_underscore_and_elbow_share_cell_baseline() {
    // Layout on one row: 'A'  '_'  'i'  '⎿' spaced so cell envelopes do not collide.
    // Columns: 4='A', 6='_', 8='i', 10='⎿'
    // Underscore special-path is only forced for screen_id == "chat-turn".
    let mut audit = audit_screen("chat-turn", "canonical.chat-turn")
        .expect("chat-turn fixture must render for underscore path");
    let placements: &[(u16, char)] = &[
        (PROBE_COL_START, 'A'),
        (PROBE_COL_START + 2, '_'),
        (PROBE_COL_START + 4, 'i'),
        (PROBE_COL_START + 6, '⎿'),
    ];
    for &(column, glyph) in placements {
        let cell = audit
            .styled_cells
            .iter_mut()
            .find(|cell| cell.row == PROBE_ROW && cell.column == column)
            .unwrap_or_else(|| panic!("missing cell col={column}"));
        *cell = StyledCell {
            row: PROBE_ROW,
            column,
            glyph,
            style: TextStyle::text(),
            absolute_fg: None,
            absolute_bg: None,
        };
    }
    audit
        .validate()
        .expect("special-glyph probe remains valid audit content");
    let bytes = render_audit_png(&audit).expect("special-glyph probe must rasterize");
    let image = image::load_from_memory(&bytes)
        .expect("valid PNG")
        .to_rgb8();
    let canvas = *image.get_pixel(0, 0);
    let cell_y = PROBE_ROW as u32 * CELL_HEIGHT;
    let flat = expected_flat_bottom(PROBE_ROW);

    // Latin A bottom near shared baseline.
    let (_, a_bottom) = cell_ink_envelope(&image, PROBE_COL_START, PROBE_ROW, canvas)
        .expect("A must have ink");
    assert!((a_bottom as i32 - flat as i32).abs() <= 1);

    // Underscore: one solid row at y = cell_y + BASELINE + 1 (hang under baseline).
    let underscore_y = cell_y + CELL_BASELINE as u32 + 1;
    let us_col = PROBE_COL_START + 2;
    let us_x0 = us_col as u32 * CELL_WIDTH;
    let mut us_ink_rows = Vec::new();
    for y in cell_y..cell_y + CELL_HEIGHT {
        let mut any = false;
        for x in us_x0 + 1..us_x0 + 9 {
            if is_ink(*image.get_pixel(x, y), canvas) {
                any = true;
                break;
            }
        }
        if any {
            us_ink_rows.push(y);
        }
    }
    assert_eq!(
        us_ink_rows,
        vec![underscore_y],
        "synthetic underscore must sit on CELL_BASELINE + 1 (got {us_ink_rows:?})"
    );
    assert!(
        (underscore_y as i32 - flat as i32).abs() <= 2,
        "underscore row must be within 2px of latin non-descender bottom"
    );

    // Elbow arm bottom is baseline - 1 (absolute y = cell_y + BASELINE - 1 = flat).
    let elbow_col = PROBE_COL_START + 6;
    let elbow_x = elbow_col as u32 * CELL_WIDTH;
    let arm_bottom = cell_y + CELL_BASELINE as u32 - 1;
    assert_ne!(
        *image.get_pixel(elbow_x + 6, arm_bottom),
        canvas,
        "elbow horizontal arm must ink at CELL_BASELINE - 1"
    );
    assert_eq!(
        *image.get_pixel(elbow_x + 6, arm_bottom + 1),
        canvas,
        "elbow arm must not sink below the shared baseline foot"
    );
    // Arm top is two pixels thick ending on the baseline.
    assert_ne!(
        *image.get_pixel(elbow_x + 6, arm_bottom - 1),
        canvas,
        "elbow arm is a 2px stroke ending on the baseline"
    );
    assert_eq!(
        arm_bottom, flat,
        "elbow foot must equal the latin flat baseline bottom"
    );
}

/// CELL geometry constants remain the published 10×21 → 1200×840 contract.
#[test]
fn cell_geometry_constants_match_published_canvas() {
    assert_eq!(CELL_WIDTH, 10);
    assert_eq!(CELL_HEIGHT, 21);
    assert_eq!(CELL_BASELINE, 16);
    assert_eq!(PNG_WIDTH, 1200);
    assert_eq!(PNG_HEIGHT, 840);
}
