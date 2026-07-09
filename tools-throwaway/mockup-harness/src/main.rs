//! THROWAWAY mockup harness - renders static Cortex Code TUI screens to
//! deterministic terminal frames.
//!
//! NOT product code. NOT a workspace member. This exists only to produce
//! "screenshot" text assets (and optional PNGs) for a design doc. Every screen
//! is rendered with `ratatui::backend::TestBackend` at a FIXED 120x40 size,
//! from static seed data, with NO clock, NO randomness, NO env, NO network -
//! so re-running produces byte-identical `.txt` output.
//!
//! Usage:
//!   mockup-harness --all               render every screen
//!   mockup-harness <screen>            render one screen (see --list)
//!   mockup-harness <category>          render every screen in a category
//!   mockup-harness --list              list screens grouped by category
//!
//! Output: plans/assets/mockups-v2/<category>/<screen>.txt
//!         (+ <category>/<screen>.png with --features png)

use std::path::{Path, PathBuf};

use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

mod chrome;
mod screens;

/// Fixed, deterministic capture size.
const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

type RenderFn = fn(&mut ratatui::Frame);

/// One registry entry: (category, screen-name, render-fn).
type Entry = (&'static str, &'static str, RenderFn);

/// The screen registry, in a fixed order (no HashMap → deterministic).
/// Grouped by category; each category maps to an output subfolder.
///
/// v2 screens register here as they are built. The v1 registry was retired
/// wholesale: its 45 screen sources are parked uncompiled under
/// `src/screens_v1_reference/` and live in git history.
fn registry() -> Vec<Entry> {
    vec![
        ("session", "home", screens::home::render),
        ("session", "onboarding", screens::onboarding::render),
        ("session", "chat-turn", screens::chat_turn::render),
        ("session", "chat-long-scroll", screens::chat_long_scroll::render),
        ("session", "streaming", screens::streaming::render),
        ("session", "reasoning", screens::reasoning::render),
        ("session", "reasoning-collapsed", screens::reasoning_collapsed::render),
        ("session", "rich-response", screens::rich_response::render),
        ("session", "tool-calls", screens::tool_calls::render),
        ("session", "tool-running", screens::tool_running::render),
        ("session", "tool-success", screens::tool_success::render),
        ("session", "tool-success-expanded", screens::tool_success_expanded::render),
        ("session", "tool-failed", screens::tool_failed::render),
        ("session", "tool-task", screens::tool_task::render),
        (
            "session",
            "tool-task-expanded",
            screens::tool_task_expanded::render,
        ),
        ("session", "diff-edit", screens::diff_edit::render),
        ("session", "shell", screens::shell::render),
        (
            "session",
            "blocked-command",
            screens::blocked_command::render,
        ),
        ("session", "turn-error", screens::turn_error::render),
        ("session", "todo-list", screens::todo_list::render),
        ("session", "compact", screens::compact::render),
        ("input", "slash-menu", screens::slash_menu::render),
        ("input", "agent-picker", screens::agent_picker::render),
        ("input", "command-palette", screens::command_palette::render),
        (
            "input",
            "command-palette-submenu",
            screens::command_palette_submenu::render,
        ),
        ("input", "model-picker", screens::model_picker::render),
        (
            "input",
            "attachment-picker",
            screens::attachment_picker::render,
        ),
        (
            "input",
            "model-picker-filtered",
            screens::model_picker_filtered::render,
        ),
        ("input", "file-mention", screens::file_mention::render),
        ("input", "export-picker", screens::export_picker::render),
        ("input", "fork-picker", screens::fork_picker::render),
        ("input", "queue-manager", screens::queue_manager::render),
        (
            "input",
            "composer-multiline",
            screens::composer_multiline::render,
        ),
        ("input", "skills-picker", screens::skills_picker::render),
        ("input", "session-picker", screens::session_picker::render),
        (
            "input",
            "subagent-picker",
            screens::subagent_picker::render,
        ),
        ("input", "variant-picker", screens::variant_picker::render),
        ("input", "theme-picker", screens::theme_picker::render),
        (
            "input",
            "permission-shell",
            screens::permission_shell::render,
        ),
        (
            "input",
            "permission-file-edit",
            screens::permission_file_edit::render,
        ),
        (
            "input",
            "permission-deny-reason",
            screens::permission_deny_reason::render,
        ),
        ("auth", "login", screens::login::render),
        (
            "auth",
            "connect-provider",
            screens::connect_provider::render,
        ),
        ("auth", "connect-method", screens::connect_method::render),
        (
            "auth",
            "connect-apikey",
            screens::connect_apikey::render,
        ),
        (
            "auth",
            "connect-connected",
            screens::connect_connected::render,
        ),
        (
            "auth",
            "connect-disconnect",
            screens::connect_disconnect::render,
        ),
        ("auth", "connect-oauth", screens::connect_oauth::render),
        ("states", "empty", screens::empty::render),
        (
            "states",
            "error-network",
            screens::error_network::render,
        ),
        (
            "states",
            "error-context",
            screens::error_context::render,
        ),
        ("states", "rate-limit", screens::rate_limit::render),
        ("states", "interrupt", screens::interrupt::render),
        ("states", "toasts", screens::toasts::render),
        (
            "states",
            "checkpoint-undo",
            screens::checkpoint_undo::render,
        ),
        ("states", "share", screens::share::render),
        ("views", "status", screens::status::render),
        (
            "views",
            "sidebar-context",
            screens::sidebar_context::render,
        ),
        ("views", "sidebar-files", screens::sidebar_files::render),
        ("views", "sidebar-lsp", screens::sidebar_lsp::render),
        ("views", "sidebar-mcp", screens::sidebar_mcp::render),
        ("views", "sidebar-queue", screens::sidebar_queue::render),
        ("views", "sidebar-todo", screens::sidebar_todo::render),
        (
            "views",
            "sidebar-subagents",
            screens::sidebar_subagents::render,
        ),
        ("views", "timeline", screens::timeline::render),
        ("views", "help", screens::help::render),
        ("views", "mcp", screens::mcp::render),
        ("views", "review", screens::review::render),
        ("views", "diff-viewer", screens::diff_viewer::render),
        (
            "views",
            "diff-viewer-staging",
            screens::diff_viewer_staging::render,
        ),
        (
            "views",
            "theme-gallery-dark",
            screens::theme_gallery_dark::render,
        ),
        (
            "views",
            "theme-gallery-light",
            screens::theme_gallery_light::render,
        ),
        (
            "orchestration",
            "mode-switch",
            screens::mode_switch::render,
        ),
        (
            "orchestration",
            "mode-switch-refused",
            screens::mode_switch_refused::render,
        ),
        (
            "orchestration",
            "workflow-launch",
            screens::workflow_launch::render,
        ),
        (
            "orchestration",
            "workflow-launch-blocked",
            screens::workflow_launch_blocked::render,
        ),
    ]
}

fn out_dir() -> PathBuf {
    // Resolve `plans/assets/mockups-v2/` relative to the repo root. The harness
    // lives at `<root>/tools-throwaway/mockup-harness/`, so walk up two levels
    // from CARGO_MANIFEST_DIR. Falls back to CWD-relative if unset.
    match option_env!("CARGO_MANIFEST_DIR") {
        Some(manifest) => Path::new(manifest)
            .ancestors()
            .nth(2)
            .unwrap_or_else(|| Path::new("."))
            .join("plans/assets/mockups-v2"),
        None => PathBuf::from("plans/assets/mockups-v2"),
    }
}

/// Render one screen into a fixed-size deterministic buffer.
fn render_buffer(render: RenderFn) -> Buffer {
    let backend = TestBackend::new(WIDTH, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("construct TestBackend terminal");
    terminal.draw(render).expect("draw frame");
    terminal.backend().buffer().clone()
}

/// Serialize a buffer to deterministic text: exactly HEIGHT lines, each padded
/// to WIDTH columns by construction (the shell border fills the last column).
/// Trailing whitespace on each row is trimmed so the assets diff cleanly, while
/// the box-drawing frame guarantees full-width lines where it matters.
fn buffer_to_text(buf: &Buffer) -> String {
    let area = *buf.area();
    let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);
    for y in 0..area.height {
        let mut line = String::with_capacity(area.width as usize);
        for x in 0..area.width {
            match buf.cell((x, y)) {
                Some(cell) => line.push_str(cell.symbol()),
                None => line.push(' '),
            }
        }
        // Trim only trailing spaces (deterministic; frame borders are non-space).
        out.push_str(line.trim_end_matches(' '));
        out.push('\n');
    }
    out
}

fn write_screen(cat: &str, name: &str, render: RenderFn, base: &Path) -> std::io::Result<PathBuf> {
    let dir = base.join(cat);
    std::fs::create_dir_all(&dir)?;
    let buf = render_buffer(render);
    let text = buffer_to_text(&buf);
    let txt_path = dir.join(format!("{name}.txt"));
    std::fs::write(&txt_path, &text)?;
    maybe_write_png(name, &buf, &dir);
    Ok(txt_path)
}

#[cfg(feature = "png")]
fn maybe_write_png(name: &str, buf: &Buffer, dir: &Path) {
    match png::render_png(buf) {
        Ok(img) => {
            let p = dir.join(format!("{name}.png"));
            if let Err(e) = img.save(&p) {
                eprintln!("note: png save skipped for {name}: {e}");
            } else {
                println!("  + {}", p.display());
            }
        }
        Err(e) => eprintln!("note: png render skipped for {name}: {e}"),
    }
}

#[cfg(not(feature = "png"))]
fn maybe_write_png(_name: &str, _buf: &Buffer, _dir: &Path) {
    // PNG is optional/soft-fail. `.txt` is the source of truth. Rebuild with
    // `--features png` to also emit rasterized frames.
}

fn print_list() {
    let reg = registry();
    println!("Available screens ({}):", reg.len());
    let mut current = "";
    for (cat, name, _) in &reg {
        if *cat != current {
            println!("\n  [{cat}]");
            current = cat;
        }
        println!("    {name}");
    }
    println!("\nUse `--all` for every screen, or pass a category name to render a whole group.");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reg = registry();
    let dir = out_dir();

    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!(
            "mockup-harness - throwaway Cortex Code TUI mockup renderer\n\n\
             USAGE:\n  mockup-harness --all\n  mockup-harness <screen>\n  \
             mockup-harness <category>\n  mockup-harness --list\n"
        );
        print_list();
        std::process::exit(if args.is_empty() { 2 } else { 0 });
    }

    if args.iter().any(|a| a == "--list") {
        print_list();
        return;
    }

    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("error: cannot create output dir {}: {e}", dir.display());
        std::process::exit(1);
    }

    let render_all = args.iter().any(|a| a == "--all");
    let selected: Vec<Entry> = if render_all {
        reg.clone()
    } else {
        // Render screens matched by exact screen-name OR by category name
        // (in registry order for determinism).
        let wanted: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        let sel: Vec<Entry> = reg
            .iter()
            .filter(|(cat, name, _)| {
                wanted.iter().any(|w| w.as_str() == *name || w.as_str() == *cat)
            })
            .cloned()
            .collect();
        if sel.is_empty() {
            eprintln!("error: no known screen or category in {args:?}");
            print_list();
            std::process::exit(2);
        }
        sel
    };

    println!("Rendering {} screen(s) to {}", selected.len(), dir.display());
    let mut written = Vec::new();
    for (cat, name, f) in selected {
        match write_screen(cat, name, f, &dir) {
            Ok(p) => {
                println!("  + {}", p.display());
                written.push(p);
            }
            Err(e) => {
                eprintln!("error: failed to write {cat}/{name}: {e}");
                std::process::exit(1);
            }
        }
    }
    println!("Done. {} file(s) written.", written.len());
}

#[cfg(feature = "png")]
mod png {
    //! Real TrueType rasterizer. Each terminal cell is drawn as its own glyph
    //! from embedded DejaVu Sans Mono (regular + bold), coloured with the cell's
    //! actual fg/bg from the ratatui buffer. `.txt` remains canonical; the PNG
    //! is a faithful visual render of the same frame.
    use fontdue::{Font, FontSettings};
    use image::{ImageBuffer, Rgb, RgbImage};
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    const PX: f32 = 17.0;
    const CW: u32 = 10;
    const CH: u32 = 21;
    const BASELINE: i32 = 16;

    const DEFAULT_BG: [u8; 3] = [13, 13, 13];
    const DEFAULT_FG: [u8; 3] = [200, 205, 214];

    const FONT_REGULAR: &[u8] = include_bytes!("../assets/DejaVuSansMono.ttf");
    const FONT_BOLD: &[u8] = include_bytes!("../assets/DejaVuSansMono-Bold.ttf");

    struct Fonts {
        regular: Font,
        bold: Font,
    }

    fn load_fonts() -> Result<Fonts, String> {
        let regular = Font::from_bytes(FONT_REGULAR, FontSettings::default())
            .map_err(|e| format!("load regular font: {e}"))?;
        let bold = Font::from_bytes(FONT_BOLD, FontSettings::default())
            .map_err(|e| format!("load bold font: {e}"))?;
        Ok(Fonts { regular, bold })
    }

    pub fn render_png(buf: &Buffer) -> Result<RgbImage, String> {
        let fonts = load_fonts()?;
        let area = *buf.area();
        let w = area.width as u32 * CW;
        let h = area.height as u32 * CH;
        let mut img: RgbImage = ImageBuffer::from_pixel(w, h, Rgb(DEFAULT_BG));

        for y in 0..area.height {
            for x in 0..area.width {
                let Some(cell) = buf.cell((x, y)) else { continue };
                let ox = x as u32 * CW;
                let oy = y as u32 * CH;
                let bg = color_to_rgb(cell.bg, DEFAULT_BG);
                fill_cell(&mut img, ox, oy, bg);

                let sym = cell.symbol();
                let Some(ch) = sym.chars().next() else { continue };
                if ch == ' ' || ch == '\u{a0}' {
                    continue;
                }
                let fg = color_to_rgb(cell.fg, DEFAULT_FG);
                if ch == '\u{23bf}' {
                    // U+23BF (the result elbow) is absent from DejaVu Sans
                    // Mono, so fontdue would draw the .notdef box; synthesize
                    // the light vertical-and-bottom-right stroke instead.
                    draw_elbow(&mut img, ox, oy, fg);
                    continue;
                }
                let bold = cell.modifier.contains(Modifier::BOLD);
                let font = if bold { &fonts.bold } else { &fonts.regular };
                draw_glyph(&mut img, ox, oy, ch, font, fg, bg);
            }
        }
        Ok(img)
    }

    fn fill_cell(img: &mut RgbImage, ox: u32, oy: u32, rgb: [u8; 3]) {
        for dy in 0..CH {
            for dx in 0..CW {
                let (px, py) = (ox + dx, oy + dy);
                if px < img.width() && py < img.height() {
                    img.put_pixel(px, py, Rgb(rgb));
                }
            }
        }
    }

    // Stroke weight and corner height chosen to sit flush with DejaVu's box
    // glyphs: 2 px strokes match the rendered weight of `│`/`─` at 17 px, and
    // the corner lands just above the text baseline.
    fn draw_elbow(img: &mut RgbImage, ox: u32, oy: u32, fg: [u8; 3]) {
        let put = |img: &mut RgbImage, x: u32, y: u32| {
            if x < img.width() && y < img.height() {
                img.put_pixel(x, y, Rgb(fg));
            }
        };
        let corner_y = oy + 13;
        for y in oy + 1..=corner_y {
            put(img, ox + 4, y);
            put(img, ox + 5, y);
        }
        for x in ox + 4..ox + CW {
            put(img, x, corner_y);
            put(img, x, corner_y + 1);
        }
    }

    // Rasterize the glyph coverage bitmap and alpha-blend fg over the cell bg.
    // fontdue returns metrics in a y-up coordinate space: `ymin` is the offset
    // from the baseline to the bitmap's bottom edge, so the bitmap top sits at
    // (baseline - ymin - height). `xmin` is the left side bearing.
    fn draw_glyph(
        img: &mut RgbImage,
        ox: u32,
        oy: u32,
        ch: char,
        font: &Font,
        fg: [u8; 3],
        bg: [u8; 3],
    ) {
        let (metrics, bitmap) = font.rasterize(ch, PX);
        if metrics.width == 0 || metrics.height == 0 {
            return;
        }
        let gx0 = ox as i32 + metrics.xmin;
        let gy0 = oy as i32 + BASELINE - metrics.ymin - metrics.height as i32;

        for gy in 0..metrics.height {
            for gx in 0..metrics.width {
                let coverage = bitmap[gy * metrics.width + gx];
                if coverage == 0 {
                    continue;
                }
                let px = gx0 + gx as i32;
                let py = gy0 + gy as i32;
                if px < 0 || py < 0 || px as u32 >= img.width() || py as u32 >= img.height() {
                    continue;
                }
                let a = coverage as u16;
                let blend = |f: u8, b: u8| -> u8 {
                    ((f as u16 * a + b as u16 * (255 - a)) / 255) as u8
                };
                img.put_pixel(
                    px as u32,
                    py as u32,
                    Rgb([blend(fg[0], bg[0]), blend(fg[1], bg[1]), blend(fg[2], bg[2])]),
                );
            }
        }
    }

    // Map a ratatui Color onto RGB. The screens use Color::Rgb from the chrome
    // palette plus a handful of named/default colors; unknown indexed colors
    // fall back to the supplied default so the render never panics. Named ANSI
    // colors approximate the v2 chrome tokens.
    fn color_to_rgb(c: Color, default: [u8; 3]) -> [u8; 3] {
        match c {
            Color::Rgb(r, g, b) => [r, g, b],
            Color::Reset => default,
            Color::Black => [0, 0, 0],
            Color::Red => [217, 108, 117],
            Color::Green => [127, 176, 105],
            Color::Yellow => [217, 179, 108],
            Color::Blue => [108, 155, 217],
            Color::Magenta => [186, 130, 200],
            Color::Cyan => [108, 190, 200],
            Color::Gray => [154, 163, 175],
            Color::DarkGray => [76, 85, 102],
            Color::LightRed => [232, 150, 158],
            Color::LightGreen => [166, 204, 148],
            Color::LightYellow => [232, 206, 158],
            Color::LightBlue => [158, 192, 232],
            Color::LightMagenta => [212, 172, 222],
            Color::LightCyan => [162, 216, 224],
            Color::White => [230, 233, 239],
            Color::Indexed(_) => default,
        }
    }
}
