//! The four and only four canonical v3 surface arrangements.

use crate::chrome::tokens::{
    validate_glyphs, ColorToken, TextStyle, CANVAS_COLUMNS as TOKEN_CANVAS_COLUMNS,
    CANVAS_ROWS as TOKEN_CANVAS_ROWS,
};
use crate::Surface;

pub const CANVAS_COLUMNS: u16 = TOKEN_CANVAS_COLUMNS;
pub const CANVAS_ROWS: u16 = TOKEN_CANVAS_ROWS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn bottom(self) -> u16 {
        self.y + self.height
    }

    pub const fn right(self) -> u16 {
        self.x + self.width
    }

    const fn contained_by(self, outer: Self) -> bool {
        self.width > 0
            && self.height > 0
            && self.x >= outer.x
            && self.y >= outer.y
            && self.right() <= outer.right()
            && self.bottom() <= outer.bottom()
    }
}

const CANVAS: Rect = Rect {
    x: 0,
    y: 0,
    width: CANVAS_COLUMNS,
    height: CANVAS_ROWS,
};
const STATUSLINE: Rect = Rect {
    x: 0,
    y: CANVAS_ROWS - 1,
    width: CANVAS_COLUMNS,
    height: 1,
};

/// Fixed geometry for one deliberate surface arrangement. A surface never
/// mixes a dock and composer, and never overlays a detached modal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceLayout {
    surface: Surface,
    transcript: Option<Rect>,
    composer: Option<Rect>,
    action_dock: Option<Rect>,
    inspector: Option<Rect>,
    detail: Option<Rect>,
}

impl SurfaceLayout {
    pub const fn for_surface(surface: Surface) -> Self {
        match surface {
            Surface::ConversationShell => Self {
                surface,
                transcript: Some(Rect {
                    x: 2,
                    y: 1,
                    width: 116,
                    height: 30,
                }),
                composer: Some(Rect {
                    x: 2,
                    y: 32,
                    width: 116,
                    height: 6,
                }),
                action_dock: None,
                inspector: None,
                detail: None,
            },
            Surface::ActionDock => Self {
                surface,
                transcript: Some(Rect {
                    x: 2,
                    y: 1,
                    width: 116,
                    height: 29,
                }),
                composer: None,
                action_dock: Some(Rect {
                    x: 2,
                    y: 31,
                    width: 116,
                    height: 7,
                }),
                inspector: None,
                detail: None,
            },
            Surface::ContextualInspector => Self {
                surface,
                transcript: Some(Rect {
                    x: 2,
                    y: 1,
                    width: 76,
                    height: 30,
                }),
                composer: Some(Rect {
                    x: 2,
                    y: 32,
                    width: 116,
                    height: 6,
                }),
                action_dock: None,
                inspector: Some(Rect {
                    x: 81,
                    y: 1,
                    width: 37,
                    height: 30,
                }),
                detail: None,
            },
            Surface::TemporaryDetailView => Self {
                surface,
                transcript: None,
                composer: None,
                action_dock: None,
                inspector: None,
                detail: Some(Rect {
                    x: 2,
                    y: 1,
                    width: 116,
                    height: 37,
                }),
            },
        }
    }

    pub const fn surface(self) -> Surface {
        self.surface
    }

    pub const fn canvas(self) -> Rect {
        CANVAS
    }

    pub const fn statusline(self) -> Rect {
        STATUSLINE
    }

    pub const fn transcript(self) -> Option<Rect> {
        self.transcript
    }

    pub const fn composer(self) -> Option<Rect> {
        self.composer
    }

    pub const fn action_dock(self) -> Option<Rect> {
        self.action_dock
    }

    pub const fn inspector(self) -> Option<Rect> {
        self.inspector
    }

    pub const fn detail(self) -> Option<Rect> {
        self.detail
    }

    /// Modals are intentionally unrepresentable in the shared v3 layout.
    pub const fn has_floating_modal(self) -> bool {
        false
    }

    /// Proves layout geometry and the one-arrangement surface policy.
    pub fn validate(self) -> Result<(), String> {
        let regions = [
            ("statusline", Some(self.statusline())),
            ("transcript", self.transcript),
            ("composer", self.composer),
            ("action dock", self.action_dock),
            ("inspector", self.inspector),
            ("detail", self.detail),
        ];
        for (name, region) in regions {
            if let Some(region) = region {
                if !region.contained_by(CANVAS) {
                    return Err(format!("{name} exceeds the fixed 120x40 canvas"));
                }
            }
        }

        let valid_shape = match self.surface {
            Surface::ConversationShell => {
                self.transcript.is_some()
                    && self.composer.is_some()
                    && self.action_dock.is_none()
                    && self.inspector.is_none()
                    && self.detail.is_none()
            }
            Surface::ActionDock => {
                self.transcript.is_some()
                    && self.composer.is_none()
                    && self.action_dock.is_some()
                    && self.inspector.is_none()
                    && self.detail.is_none()
            }
            Surface::ContextualInspector => {
                self.transcript.is_some()
                    && self.composer.is_some()
                    && self.action_dock.is_none()
                    && self.inspector.is_some()
                    && self.detail.is_none()
            }
            Surface::TemporaryDetailView => {
                self.transcript.is_none()
                    && self.composer.is_none()
                    && self.action_dock.is_none()
                    && self.inspector.is_none()
                    && self.detail.is_some()
            }
        };
        if !valid_shape {
            return Err(format!(
                "{} must use exactly its one declared surface arrangement",
                self.surface.as_str()
            ));
        }

        if self.statusline() != STATUSLINE {
            return Err("statusline must occupy the final shared canvas row".to_owned());
        }
        if let (Some(composer), Some(dock)) = (self.composer, self.action_dock) {
            if composer.bottom() != dock.bottom() {
                return Err("a dock must replace the composer at the same bottom edge".to_owned());
            }
        }
        Ok(())
    }
}

/// Minimal shared status metadata. It is rendered over the canvas rather than
/// as a filled footer strip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusLine {
    text: String,
    style: TextStyle,
}

impl StatusLine {
    pub fn new(path: &str, branch: &str, model: &str) -> Result<Self, String> {
        if [path, branch, model]
            .iter()
            .any(|field| field.chars().any(char::is_control))
        {
            return Err("statusline fields must not contain control characters".to_owned());
        }
        if !path.starts_with("~/") || path.contains('·') || path.trim().is_empty() {
            return Err("statusline path must be a local ~/ path".to_owned());
        }
        if branch.trim().is_empty() || branch.contains('·') {
            return Err("statusline branch must be a plain readable label".to_owned());
        }
        let lower_model = model.to_ascii_lowercase();
        if model.trim().is_empty()
            || model.contains('/')
            || model.contains('·')
            || lower_model.contains("ctx")
            || lower_model.contains("idle")
            || model.contains('$')
        {
            return Err("statusline model must be a human-readable display name".to_owned());
        }
        let text = format!("{path} · {branch} · {model}");
        validate_glyphs(&text)?;
        if text.chars().count() > CANVAS_COLUMNS as usize {
            return Err("statusline must fit one 120-column canvas row".to_owned());
        }
        Ok(Self {
            text,
            style: TextStyle {
                foreground: ColorToken::Muted,
                background: Some(ColorToken::Canvas),
                bold: false,
            },
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn style(&self) -> TextStyle {
        self.style
    }
}

/// Outer editorial horizontal pad for Cortex-inspired 120-col canvas geometry
/// (VAL-V3-DS-035 / F-11). Conversation shells and dominant content leave a
/// deliberate gutter so content is never edge-glued to columns 0 / 119.
pub const OUTER_HPAD: u16 = 2;
/// Dominant content measure after `OUTER_HPAD` gutters on both sides.
pub const OUTER_CONTENT_WIDTH: u16 = CANVAS_COLUMNS - (OUTER_HPAD * 2);

/// Left column of the outer editorial content measure (once-padded).
pub const fn outer_content_x() -> u16 {
    OUTER_HPAD
}

/// Width available between outer gutters.
pub const fn outer_content_width() -> u16 {
    OUTER_CONTENT_WIDTH
}

/// Pads `text` with `OUTER_HPAD` blank columns on each side to a full 120-col row.
///
/// Text wider than `OUTER_CONTENT_WIDTH` fails closed; shorter text is right-padded
/// with spaces so the returned line spans the full canvas width and the right
/// gutter stays empty.
pub fn with_outer_hpad(text: &str) -> Result<String, String> {
    let content_width = OUTER_CONTENT_WIDTH as usize;
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > content_width {
        return Err(format!(
            "content exceeds outer editorial measure ({content_width} columns)"
        ));
    }
    let mut row = String::with_capacity(CANVAS_COLUMNS as usize);
    for _ in 0..OUTER_HPAD {
        row.push(' ');
    }
    for ch in &chars {
        row.push(*ch);
    }
    for _ in chars.len()..content_width {
        row.push(' ');
    }
    for _ in 0..OUTER_HPAD {
        row.push(' ');
    }
    debug_assert_eq!(row.chars().count() as u16, CANVAS_COLUMNS);
    Ok(row)
}

/// Standard bottom action-dock band shared by pilot frames.
/// Matches `SurfaceLayout::for_surface(ActionDock).action_dock()`: y=31 height=7.
pub const ACTION_DOCK_TOP: u16 = 31;
pub const ACTION_DOCK_BOTTOM: u16 = 37;
pub const ACTION_DOCK_INNER_ROWS: u16 = 5;

/// Geometry for the shared bottom action dock chrome (not a floating card).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionDockGeometry {
    pub top: u16,
    pub bottom: u16,
    pub inner_rows: u16,
}

/// Returns the fixed bottom dock band used by shared chrome helpers.
pub const fn action_dock_geometry() -> ActionDockGeometry {
    ActionDockGeometry {
        top: ACTION_DOCK_TOP,
        bottom: ACTION_DOCK_BOTTOM,
        inner_rows: ACTION_DOCK_INNER_ROWS,
    }
}

/// Hard ceiling for nonempty content lines on progressive-disclosure first frames.
/// Higher is allowed only for temporary detail views that deliberately show evidence.
pub const FIRST_FRAME_NONEMPTY_SOFT_CAP: usize = 22;

/// A renderer-facing declaration that binds each composed screen to one of the
/// four central arrangements and the one shared statusline policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenComposition {
    screen_id: String,
    surface: Surface,
    statusline: StatusLine,
}

impl ScreenComposition {
    pub fn new(screen_id: &str, surface: Surface, statusline: StatusLine) -> Self {
        Self {
            screen_id: screen_id.to_owned(),
            surface,
            statusline,
        }
    }

    pub fn surface(&self) -> Surface {
        self.surface
    }

    pub fn statusline(&self) -> &StatusLine {
        &self.statusline
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.screen_id.trim().is_empty() {
            return Err("screen composition needs a canonical screen ID".to_owned());
        }
        SurfaceLayout::for_surface(self.surface).validate()?;
        if self.statusline.style.background != Some(ColorToken::Canvas) {
            return Err("statusline must share the canvas background".to_owned());
        }
        Ok(())
    }
}
