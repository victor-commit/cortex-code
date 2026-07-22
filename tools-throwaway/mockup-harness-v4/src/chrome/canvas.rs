//! 120x40 cell canvas with semantic styling + deterministic PNG raster.

use super::tokens::{
    glyph_rendering, palette, validate_glyphs, ColorToken, GlyphRendering, TextStyle, Theme,
    CANVAS_COLUMNS, CANVAS_ROWS, CELL_BASELINE, CELL_HEIGHT, CELL_WIDTH, PNG_HEIGHT, PNG_WIDTH,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cell {
    pub row: u16,
    pub column: u16,
    pub glyph: char,
    pub style: TextStyle,
    pub absolute_fg: Option<(u8, u8, u8)>,
    pub absolute_bg: Option<(u8, u8, u8)>,
}

#[derive(Clone, Debug)]
pub struct Canvas {
    pub theme: Theme,
    pub cells: Vec<Cell>,
    occupied: [[bool; CANVAS_COLUMNS as usize]; CANVAS_ROWS as usize],
}

impl Canvas {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            cells: Vec::new(),
            occupied: [[false; CANVAS_COLUMNS as usize]; CANVAS_ROWS as usize],
        }
    }

    pub fn put(&mut self, row: u16, column: u16, glyph: char, style: TextStyle) -> Result<(), String> {
        if row >= CANVAS_ROWS || column >= CANVAS_COLUMNS {
            return Err(format!("cell out of bounds: row={row} col={column}"));
        }
        if !glyph.is_control() {
            validate_glyphs(&glyph.to_string())?;
        }
        if self.occupied[row as usize][column as usize] {
            // Last write wins for pure dump pilots (simplifies jewel overlays).
            self.cells
                .retain(|cell| !(cell.row == row && cell.column == column));
        }
        self.occupied[row as usize][column as usize] = true;
        self.cells.push(Cell {
            row,
            column,
            glyph,
            style,
            absolute_fg: None,
            absolute_bg: None,
        });
        Ok(())
    }

    pub fn write(
        &mut self,
        row: u16,
        column: u16,
        text: &str,
        style: TextStyle,
    ) -> Result<(), String> {
        validate_glyphs(text)?;
        for (col, glyph) in (column..).zip(text.chars()) {
            if col >= CANVAS_COLUMNS {
                return Err(format!("text overflows canvas at row {row}: {text}"));
            }
            self.put(row, col, glyph, style)?;
        }
        Ok(())
    }

    pub fn write_centered(&mut self, row: u16, text: &str, style: TextStyle) -> Result<(), String> {
        let width = text.chars().count() as u16;
        let column = if width >= CANVAS_COLUMNS {
            0
        } else {
            (CANVAS_COLUMNS - width) / 2
        };
        self.write(row, column, text, style)
    }

    /// Fill a horizontal strip with a surface wash (user turns etc.).
    pub fn fill_row_bg(
        &mut self,
        row: u16,
        start_col: u16,
        end_col: u16,
        style: TextStyle,
    ) -> Result<(), String> {
        for col in start_col..end_col.min(CANVAS_COLUMNS) {
            if !self.occupied[row as usize][col as usize] {
                self.put(row, col, ' ', style)?;
            } else if let Some(cell) = self
                .cells
                .iter_mut()
                .find(|c| c.row == row && c.column == col)
            {
                cell.style.background = style.background;
            }
        }
        Ok(())
    }

    pub fn to_text(&self) -> String {
        let mut rows = vec![" ".repeat(CANVAS_COLUMNS as usize); CANVAS_ROWS as usize];
        let mut cells = self.cells.clone();
        cells.sort_by_key(|cell| (cell.row, cell.column));
        for cell in cells {
            let row = &mut rows[cell.row as usize];
            let mut chars: Vec<char> = row.chars().collect();
            chars[cell.column as usize] = cell.glyph;
            *row = chars.into_iter().collect();
        }
        let mut out = rows.join("\n");
        out.push('\n');
        out
    }
}

/// Rasterize a pure-Cortex canvas to deterministic PNG bytes (feature `png`).
#[cfg(feature = "png")]
pub fn render_png(canvas: &Canvas) -> Result<Vec<u8>, String> {
    use image::{DynamicImage, ImageFormat, Rgb as Pixel, RgbImage};

    let palette = palette(canvas.theme);
    let canvas_color = palette.color(ColorToken::Canvas);
    let mut image = RgbImage::from_pixel(
        PNG_WIDTH,
        PNG_HEIGHT,
        Pixel([canvas_color.red, canvas_color.green, canvas_color.blue]),
    );

    for cell in &canvas.cells {
        let fill = if let Some((r, g, b)) = cell.absolute_bg {
            Some(Pixel([r, g, b]))
        } else if let Some(token) = cell.style.background {
            if token == ColorToken::Canvas {
                None
            } else {
                let rgb = palette.color(token);
                Some(Pixel([rgb.red, rgb.green, rgb.blue]))
            }
        } else {
            None
        };
        if let Some(pixel) = fill {
            let cell_x = cell.column as u32 * CELL_WIDTH;
            let cell_y = cell.row as u32 * CELL_HEIGHT;
            for y in 0..CELL_HEIGHT {
                for x in 0..CELL_WIDTH {
                    image.put_pixel(cell_x + x, cell_y + y, pixel);
                }
            }
        }
    }

    let (regular, bold) = super::tokens::embedded_fonts()?;
    for cell in &canvas.cells {
        if cell.glyph == ' ' {
            continue;
        }
        let token_fg = palette.color(cell.style.foreground);
        let foreground = if let Some((r, g, b)) = cell.absolute_fg {
            super::tokens::Rgb {
                red: r,
                green: g,
                blue: b,
            }
        } else {
            token_fg
        };
        let cell_x = cell.column as u32 * CELL_WIDTH;
        let cell_y = cell.row as u32 * CELL_HEIGHT;
        let fg_pixel = Pixel([foreground.red, foreground.green, foreground.blue]);

        match glyph_rendering(cell.glyph) {
            Some(GlyphRendering::SyntheticElbow) => {
                draw_synthetic_elbow(&mut image, cell_x, cell_y, fg_pixel);
                continue;
            }
            Some(GlyphRendering::SyntheticPromptArrow) => {
                draw_synthetic_prompt_arrow(&mut image, cell_x, cell_y, fg_pixel);
                continue;
            }
            Some(GlyphRendering::SyntheticDiamond) => {
                draw_synthetic_diamond(&mut image, cell_x, cell_y, fg_pixel, cell.glyph == '◇');
                continue;
            }
            Some(GlyphRendering::SyntheticBlock) => {
                draw_synthetic_block(&mut image, cell_x, cell_y, fg_pixel, cell.glyph);
                continue;
            }
            Some(GlyphRendering::SyntheticBrailleDot) => {
                // Prefer emb font when present; fall back to center dot.
                let font = if cell.style.bold { &bold } else { &regular };
                if font.lookup_glyph_index(cell.glyph) != 0 {
                    blit_glyph(
                        &mut image,
                        font,
                        cell.glyph,
                        cell.column,
                        cell.row,
                        foreground,
                    );
                } else {
                    draw_braille_dot_fallback(&mut image, cell_x, cell_y, fg_pixel, cell.glyph);
                }
                continue;
            }
            Some(GlyphRendering::EmbeddedFont) | None => {}
        }

        let font = if cell.style.bold { &bold } else { &regular };
        blit_glyph(
            &mut image,
            font,
            cell.glyph,
            cell.column,
            cell.row,
            foreground,
        );
    }

    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(image)
        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|error| format!("could not encode pure-Cortex PNG: {error}"))?;
    Ok(bytes)
}

#[cfg(feature = "png")]
fn blit_glyph(
    image: &mut image::RgbImage,
    font: &fontdue::Font,
    glyph: char,
    column: u16,
    row: u16,
    foreground: super::tokens::Rgb,
) {
    use image::Rgb as Pixel;
    let (metrics, bitmap) = font.rasterize(glyph, 16.0);
    let pen_x = column as i32 * CELL_WIDTH as i32;
    let pen_y = row as i32 * CELL_HEIGHT as i32 + CELL_BASELINE;
    let base_x = pen_x + metrics.xmin;
    let base_y = pen_y - metrics.ymin - metrics.height as i32;
    for y in 0..metrics.height {
        for x in 0..metrics.width {
            let alpha = bitmap[y * metrics.width + x] as u16;
            if alpha == 0 {
                continue;
            }
            let px = base_x + x as i32;
            let py = base_y + y as i32;
            if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
                continue;
            }
            let background = image.get_pixel(px as u32, py as u32).0;
            let blend = |fg: u8, bg: u8| ((fg as u16 * alpha + bg as u16 * (255 - alpha)) / 255) as u8;
            image.put_pixel(
                px as u32,
                py as u32,
                Pixel([
                    blend(foreground.red, background[0]),
                    blend(foreground.green, background[1]),
                    blend(foreground.blue, background[2]),
                ]),
            );
        }
    }
}

#[cfg(feature = "png")]
fn draw_synthetic_elbow(
    image: &mut image::RgbImage,
    cell_x: u32,
    cell_y: u32,
    foreground: image::Rgb<u8>,
) {
    let put = |image: &mut image::RgbImage, x: u32, y: u32| {
        if x < image.width() && y < image.height() {
            image.put_pixel(x, y, foreground);
        }
    };
    let baseline = CELL_BASELINE as u32;
    let stem_left = cell_x + 3;
    let stem_right = cell_x + 4;
    let stem_top = cell_y + 3;
    let arm_top = cell_y + baseline - 2;
    let arm_bottom = cell_y + baseline - 1;
    let arm_right = cell_x + 9;
    for y in stem_top..=arm_bottom {
        for x in stem_left..=stem_right {
            put(image, x, y);
        }
    }
    for y in arm_top..=arm_bottom {
        for x in stem_left..=arm_right {
            put(image, x, y);
        }
    }
}

/// Draw a right-chevron `❯` when the emb font lacks Dingbats coverage.
#[cfg(feature = "png")]
fn draw_synthetic_prompt_arrow(
    image: &mut image::RgbImage,
    cell_x: u32,
    cell_y: u32,
    foreground: image::Rgb<u8>,
) {
    let put = |image: &mut image::RgbImage, x: u32, y: u32| {
        if x < image.width() && y < image.height() {
            image.put_pixel(x, y, foreground);
        }
    };
    // Two-pixel thick chevron pointing right, sitting on latin mid/baseline.
    let mid = cell_y + 10;
    let points = [
        (2u32, mid.saturating_sub(6)),
        (3, mid.saturating_sub(5)),
        (4, mid.saturating_sub(4)),
        (5, mid.saturating_sub(3)),
        (6, mid.saturating_sub(2)),
        (7, mid.saturating_sub(1)),
        (8, mid),
        (7, mid + 1),
        (6, mid + 2),
        (5, mid + 3),
        (4, mid + 4),
        (3, mid + 5),
        (2, mid + 6),
    ];
    for (dx, y) in points {
        put(image, cell_x + dx, y);
        put(image, cell_x + dx + 1, y);
    }
}

#[cfg(feature = "png")]
fn draw_synthetic_diamond(
    image: &mut image::RgbImage,
    cell_x: u32,
    cell_y: u32,
    foreground: image::Rgb<u8>,
    hollow: bool,
) {
    let put = |image: &mut image::RgbImage, x: u32, y: u32| {
        if x < image.width() && y < image.height() {
            image.put_pixel(x, y, foreground);
        }
    };
    let cx = cell_x + 4;
    let cy = cell_y + 10;
    for dy in 0i32..=4 {
        let half = 4 - dy;
        for dx in -half..=half {
            let x = (cx as i32 + dx) as u32;
            let y_up = (cy as i32 - dy) as u32;
            let y_dn = (cy as i32 + dy) as u32;
            let on_edge = dx.abs() == half || dy == 4;
            if !hollow || on_edge {
                put(image, x, y_up);
                put(image, x, y_dn);
            }
        }
    }
}

/// Factory MC blocks: filled/hollow slot cells, mini progress ticks, left selection bar.
#[cfg(feature = "png")]
fn draw_synthetic_block(
    image: &mut image::RgbImage,
    cell_x: u32,
    cell_y: u32,
    foreground: image::Rgb<u8>,
    glyph: char,
) {
    let put = |image: &mut image::RgbImage, x: u32, y: u32| {
        if x < image.width() && y < image.height() {
            image.put_pixel(x, y, foreground);
        }
    };
    let fill_rect = |image: &mut image::RgbImage, x0: u32, y0: u32, x1: u32, y1: u32| {
        for y in y0..=y1 {
            for x in x0..=x1 {
                put(image, x, y);
            }
        }
    };
    let outline = |image: &mut image::RgbImage, x0: u32, y0: u32, x1: u32, y1: u32| {
        for x in x0..=x1 {
            put(image, x, y0);
            put(image, x, y1);
        }
        for y in y0..=y1 {
            put(image, x0, y);
            put(image, x1, y);
        }
    };
    match glyph {
        // Filled slot cell ▮
        '▮' => fill_rect(image, cell_x + 2, cell_y + 4, cell_x + 7, cell_y + 16),
        // Hollow slot cell ▯
        '▯' => outline(image, cell_x + 2, cell_y + 4, cell_x + 7, cell_y + 16),
        // Filled mini tick ▰ (short bar)
        '▰' => fill_rect(image, cell_x + 1, cell_y + 7, cell_x + 8, cell_y + 13),
        // Hollow mini tick ▱
        '▱' => outline(image, cell_x + 1, cell_y + 7, cell_x + 8, cell_y + 13),
        // Left half selection bar ▌
        '▌' => fill_rect(image, cell_x, cell_y + 2, cell_x + 4, cell_y + 18),
        // Right half pane edge ▐
        '▐' => fill_rect(image, cell_x + 5, cell_y + 2, cell_x + 9, cell_y + 18),
        _ => fill_rect(image, cell_x + 2, cell_y + 5, cell_x + 7, cell_y + 15),
    }
}

#[cfg(feature = "png")]
fn draw_braille_dot_fallback(
    image: &mut image::RgbImage,
    cell_x: u32,
    cell_y: u32,
    foreground: image::Rgb<u8>,
    glyph: char,
) {
    // Decode U+28xx 8-dot braille into a 2x4 grid inside the cell.
    let bits = (glyph as u32).saturating_sub(0x2800) as u8;
    let map = [
        (1u32, 3u32, 0u8),  // dot 1
        (1, 7, 1),          // dot 2
        (1, 11, 2),         // dot 3
        (5, 3, 3),          // dot 4
        (5, 7, 4),          // dot 5
        (5, 11, 5),         // dot 6
        (1, 15, 6),         // dot 7
        (5, 15, 7),         // dot 8
    ];
    for (dx, dy, bit) in map {
        if bits & (1 << bit) != 0 {
            let x = cell_x + dx;
            let y = cell_y + dy;
            for oy in 0..2u32 {
                for ox in 0..2u32 {
                    let px = x + ox;
                    let py = y + oy;
                    if px < image.width() && py < image.height() {
                        image.put_pixel(px, py, foreground);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

    #[test]
    fn canvas_dimensions_stable() {
        let mut c = Canvas::new(Theme::CortexNight);
        c.write(0, 0, "hello", TextStyle::plain(ColorToken::Text))
            .unwrap();
        let text = c.to_text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 40);
        assert!(lines.iter().all(|l| l.chars().count() == 120));
        assert!(lines[0].starts_with("hello"));
    }
}
