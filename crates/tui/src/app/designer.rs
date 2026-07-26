//! Designer fixture bridge (`feature = "designer"`).
//!
//! Methods live inside the `app` module so they can set private screen state without
//! making product fields public. Fixture paths never open the session store, spawn
//! models.dev refresh, ambient auth, or live MCP.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use tui_components::toast::ToastLevel;

use super::{
    App, CompactDialog, ContextFileRow, Dialog, DialogKind, DiffViewMode, DiffViewer,
    DiffViewerFocus, Entry, FileTreeKind, FileTreeRow, FindState, InitDialog, InitStep,
    LspDiagnosticRow, LspFileGroup, MentionPicker, Palette, PendingPermission, RenameDialog,
    SidebarPanel, Status,
};
use crate::permission::PermissionAsk;
use crate::view::{
    GitInfo, McpServerRow, McpStatusTone, ReviewData, ReviewFinding, Severity, StatusData,
    TimelineDot, TimelineEvent, ViewKind,
};

impl App {
    /// Hermetic base App for designer paint: cortexnight + reduced motion + temp files root.
    #[must_use]
    pub fn designer_prepare_base(status: Status) -> Self {
        let mut app = Self::new(status);
        app.set_theme("cortexnight".to_owned());
        app.set_reduced_motion(true);
        app.files_root = std::env::temp_dir().join("cortex-designer-hermetic");
        app
    }

    /// Replace the transcript with pure fixture entries (no turn worker).
    pub fn designer_set_transcript(&mut self, entries: Vec<Entry>) {
        self.transcript = entries;
        self.scroll = 0;
    }

    /// Open a picker dialog with injected rows — never calls `open_store` / models refresh.
    pub fn designer_open_dialog(
        &mut self,
        kind: DialogKind,
        title: impl Into<String>,
        items: Vec<String>,
        selected: usize,
    ) {
        let selected = if items.is_empty() {
            0
        } else {
            selected.min(items.len() - 1)
        };
        self.open_dialog(Dialog {
            kind,
            title: title.into(),
            items,
            selected,
            empty: None,
        });
    }

    /// Open a full-screen view and seed minimal snapshot data so bodies are non-empty.
    pub fn designer_open_view(&mut self, kind: ViewKind) {
        match kind {
            ViewKind::Status => {
                self.status_snapshot = Some(designer_status_snapshot(&self.status));
            }
            ViewKind::Timeline => {
                self.timeline_snapshot = vec![
                    TimelineEvent {
                        dot: TimelineDot::Filled,
                        time: "10:00".to_owned(),
                        who: "you".to_owned(),
                        text: "fixture prompt".to_owned(),
                    },
                    TimelineEvent {
                        dot: TimelineDot::Diamond,
                        time: "10:01".to_owned(),
                        who: "assistant".to_owned(),
                        text: "fixture reply".to_owned(),
                    },
                ];
            }
            ViewKind::Queue => {
                self.queue_snapshot = vec![
                    "queued fixture prompt A".to_owned(),
                    "queued fixture prompt B".to_owned(),
                ];
                self.queue_selection = 0;
                self.queue_pending_dd = false;
            }
            ViewKind::Review => {
                self.review_snapshot = Some(ReviewData {
                    branch: "main".to_owned(),
                    files: 2,
                    findings: vec![ReviewFinding {
                        severity: Severity::Medium,
                        location: "src/lib.rs:1".to_owned(),
                        text: "fixture review finding".to_owned(),
                    }],
                });
            }
            ViewKind::Mcp
            | ViewKind::Diff
            | ViewKind::Help
            | ViewKind::MissionControl
            | ViewKind::MissionControlFailure
            | ViewKind::McStub(_) => {}
        }
        self.open_view(kind);
    }

    /// Show/hide the sidebar and select a panel with hermetic sample rows (no cwd walk).
    pub fn designer_set_sidebar(&mut self, visible: bool, panel: SidebarPanel) {
        self.sidebar_visible = visible;
        if !visible {
            return;
        }
        self.sidebar_panel = panel;
        match panel {
            SidebarPanel::Files => {
                self.files_cursor = 0;
                self.files_nav_active = false;
                self.files_expanded.clear();
                self.files_collapsed.clear();
                self.files_tree = vec![
                    FileTreeRow {
                        depth: 0,
                        name: "src/".to_owned(),
                        kind: FileTreeKind::DirExpanded,
                        path: PathBuf::from("src"),
                    },
                    FileTreeRow {
                        depth: 1,
                        name: "main.rs".to_owned(),
                        kind: FileTreeKind::FileModified,
                        path: PathBuf::from("src/main.rs"),
                    },
                    FileTreeRow {
                        depth: 1,
                        name: "lib.rs".to_owned(),
                        kind: FileTreeKind::File,
                        path: PathBuf::from("src/lib.rs"),
                    },
                ];
            }
            SidebarPanel::Lsp => {
                self.sidebar_lsp_groups = vec![LspFileGroup {
                    file: "src/app.rs".to_owned(),
                    errors: 1,
                    warnings: 1,
                    diagnostics: vec![
                        LspDiagnosticRow {
                            line: 10,
                            is_error: true,
                            message: "fixture error".to_owned(),
                        },
                        LspDiagnosticRow {
                            line: 20,
                            is_error: false,
                            message: "fixture warning".to_owned(),
                        },
                    ],
                }];
            }
            SidebarPanel::Context
            | SidebarPanel::Mcp
            | SidebarPanel::Todo
            | SidebarPanel::Subagents
            | SidebarPanel::Queue => {
                // Empty-state paint is valid; avoid open_store / live MCP.
            }
        }
    }

    /// Open the command palette with an optional query (matches come from the live catalogue).
    pub fn designer_open_palette(&mut self, query: &str, _matches: Vec<String>) {
        let mut palette = Palette::default();
        palette.query = query.to_owned();
        palette.selected = 0;
        self.palette = Some(palette);
    }

    /// Display-only permission dock — dummy reply channel, no live worker.
    pub fn designer_open_permission_preview(
        &mut self,
        tool: impl Into<String>,
        subject: impl Into<String>,
    ) {
        let (tx, _rx) = mpsc::channel();
        let ask = PermissionAsk::new(tool, subject, tx);
        self.pending_permission = Some(PendingPermission {
            ask,
            selected: 0,
            feedback: String::new(),
        });
    }

    /// Toggle `!` shell-mode chrome without key routing.
    pub fn designer_set_shell_mode(&mut self, on: bool) {
        self.shell_mode = on;
    }

    /// Plan-mode chrome via the product `status.agent == "plan"` seam.
    pub fn designer_set_plan_mode_chrome(&mut self, on: bool) {
        if on {
            self.status.agent = "plan".to_owned();
            if self.status.mode.is_empty() {
                self.status.mode = "read-only".to_owned();
            }
        } else if self.status.agent == "plan" {
            self.status.agent = "build".to_owned();
        }
    }

    /// Open slash autocomplete by seeding a `/` composer query.
    pub fn designer_open_slash(&mut self) {
        self.set_input_value("/");
        self.slash_selected = 0;
    }

    /// Open the `@` mention picker with injected file paths (no FS walk).
    pub fn designer_open_mention(&mut self, files: Vec<String>) {
        let files = if files.is_empty() {
            vec![
                "src/main.rs".to_owned(),
                "src/lib.rs".to_owned(),
                "Cargo.toml".to_owned(),
            ]
        } else {
            files
        };
        self.set_input_value("@");
        self.mention = Some(MentionPicker {
            files,
            selected: 0,
        });
    }

    /// Open find-in-chat with an optional query.
    pub fn designer_open_find(&mut self, query: &str) {
        self.find = Some(FindState {
            query: query.to_owned(),
            active: 0,
            matches: 0,
            committed: false,
        });
        if !query.is_empty() {
            self.recompute_find();
        }
    }

    /// Surface a toast (default error level for overlay-toast-error).
    pub fn designer_open_toast(&mut self, level: ToastLevel, message: impl Into<String>) {
        self.push_toast(level, message);
    }

    /// Open the themes gallery full-screen surface.
    pub fn designer_open_theme_gallery(&mut self) {
        self.open_theme_gallery();
    }

    /// Open the `/diff` viewer overlay with injected file diffs (not `ViewKind::Diff`).
    pub fn designer_open_diff_viewer(&mut self, files: Vec<crate::diff::FileDiff>) {
        let files = if files.is_empty() {
            let f0 = crate::diff::compute_file_diff(
                "crates/tui/src/stream_processor.rs",
                Some("keep_a\nremove_me\nkeep_b\n"),
                Some("keep_a\nadd_me\nkeep_b\n"),
            );
            let f1 = crate::diff::compute_file_diff(
                "crates/tui/src/buffer.rs",
                Some("alpha\nbeta\n"),
                Some("ALPHA\nbeta\n"),
            );
            match (f0, f1) {
                (Some(a), Some(b)) => vec![a, b],
                (Some(a), None) => vec![a],
                (None, Some(b)) => vec![b],
                (None, None) => Vec::new(),
            }
        } else {
            files
        };
        if files.is_empty() {
            return;
        }
        let expanded = (0..files.len()).map(|i| i == 0).collect();
        self.diff_viewer = Some(DiffViewer {
            files,
            selected_file: 0,
            selected_hunk: 0,
            mode: DiffViewMode::Unified,
            focus: DiffViewerFocus::Files,
            expanded,
        });
    }

    /// First-run onboarding full-frame (wraps public `set_onboarding`).
    pub fn designer_set_onboarding(&mut self, on: bool) {
        self.set_onboarding(on);
    }

    /// Fake busy/streaming chrome without spawning a turn worker.
    pub fn designer_set_busy(&mut self, on: bool) {
        if on {
            let (_tx, rx) = mpsc::channel();
            self.turn_rx = Some(rx);
            self.spinner_start = Some(Instant::now());
        } else {
            self.turn_rx = None;
            self.spinner_start = None;
            self.cancel_flag = None;
        }
    }

    /// Open rename confirm dialog with pure fixture data (no store).
    pub fn designer_open_rename(&mut self, session_id: impl Into<String>, current_title: Option<String>) {
        self.rename_dialog = Some(RenameDialog {
            session_id: session_id.into(),
            current_title,
            buffer: String::new(),
        });
    }

    /// Open compact confirm dialog with pure fixture numbers.
    pub fn designer_open_compact(&mut self) {
        self.compact_dialog = Some(CompactDialog {
            before_tokens: "42,318 tokens".to_owned(),
            before_detail: "12 messages".to_owned(),
            after_tokens: "~8,400 tokens".to_owned(),
            after_detail: "1 summary + 2 recent".to_owned(),
            keeps: vec![
                "fixture keep: prior prompt".to_owned(),
                "fixture keep: latest ask".to_owned(),
            ],
        });
    }

    /// Open init confirm dialog with pure scan steps.
    pub fn designer_open_init(&mut self) {
        self.init_dialog = Some(InitDialog {
            root: "/tmp/cortex-designer-hermetic".to_owned(),
            steps: vec![
                InitStep {
                    label: "detected Rust workspace (cargo)".to_owned(),
                    done: true,
                },
                InitStep {
                    label: "scan AGENTS.md targets".to_owned(),
                    done: false,
                },
            ],
        });
    }

    /// Inject context-panel sample rows used by some sidebar paints (optional helper).
    pub fn designer_set_context_files(&mut self, files: Vec<ContextFileRow>) {
        // Context rows are derived from transcript attachments / history in product code.
        // For fixtures, push a notice so the panel is not the only empty surface; real
        // context_files() still reads live state — keep this as a no-op seed hook for now.
        let _ = files;
    }

    /// Inject MCP view-oriented sample via a no-network path (rows read live at paint).
    pub fn designer_seed_mcp_rows_hint(&mut self) {
        let _rows: Vec<McpServerRow> = vec![McpServerRow {
            name: "fixture-mcp".to_owned(),
            status_word: "ready".to_owned(),
            tone: McpStatusTone::Ready,
            tool_count: Some(3),
            exposes: "tools".to_owned(),
        }];
        // Product MCP body reads manager/config; empty state is acceptable for paint smoke.
        let _ = _rows;
    }
}

fn designer_status_snapshot(status: &Status) -> StatusData {
    StatusData {
        session: if status.session.is_empty() {
            "designer fixture session".to_owned()
        } else {
            status.session.clone()
        },
        started: "2h ago · 12 messages".to_owned(),
        agent: status.agent.clone(),
        model: status.model.clone(),
        variant: if status.variant.is_empty() {
            "default".to_owned()
        } else {
            status.variant.clone()
        },
        context_used: 42_318,
        context_limit: 200_000,
        tokens: "in 38.2k · out 4.1k · 42.3k total".to_owned(),
        cost: "$0.84 this session".to_owned(),
        permits: "edit allow · bash allow-all · web allow".to_owned(),
        workspace: "/tmp/cortex-designer-hermetic".to_owned(),
        git: Some(GitInfo {
            branch: "main".to_owned(),
            clean: true,
        }),
    }
}

#[cfg(all(test, feature = "designer"))]
mod tests {
    use super::*;
    use crate::ui;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn fixture_status() -> Status {
        Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "default".to_owned(),
            variant: "default".to_owned(),
            session: "new".to_owned(),
        }
    }

    fn paint(app: &App) -> String {
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| ui::draw(frame, app))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn designer_prepare_base_paints_empty_home_with_cortex_chrome() {
        // Given: hermetic base App (empty transcript home)
        let app = App::designer_prepare_base(fixture_status());
        assert_eq!(app.theme(), "cortexnight");
        assert!(app.reduced_motion());

        // When: paint at gallery size
        let screen = paint(&app);

        // Evidence-friendly dump (visible with --nocapture)
        let preview: String = screen.chars().take(400).collect();
        eprintln!("designer home paint preview (first 400 chars):\n{preview}");

        // Then: brand / prompt chrome is present
        assert!(
            screen.contains('✦')
                || screen.contains("Cortex")
                || screen.contains('❯')
                || screen.contains('›'),
            "home paint missing Cortex chrome glyphs; got:\n{screen}"
        );
        assert!(!screen.trim().is_empty(), "home paint must be non-empty");
    }

    #[test]
    fn designer_open_dialog_models_paints_without_network() {
        // Given: base + injected models dialog rows
        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_dialog(
            DialogKind::Models,
            "Models",
            vec![
                "google/gemini-2.5-flash".to_owned(),
                "anthropic/claude-sonnet-4".to_owned(),
            ],
            0,
        );
        assert!(app.dialog_open());

        // When
        let screen = paint(&app);

        // Then
        assert!(
            screen.contains("Models") || screen.contains("gemini") || screen.contains("claude"),
            "dialog-models paint missing picker content; got:\n{screen}"
        );
    }

    #[test]
    fn designer_open_view_status_paints_with_seeded_snapshot() {
        // Given
        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_view(ViewKind::Status);
        assert_eq!(app.active_view(), Some(ViewKind::Status));
        assert!(app.status_snapshot().is_some());

        // When
        let screen = paint(&app);

        // Then
        assert!(
            screen.contains("Session")
                || screen.contains("status")
                || screen.contains("gemini")
                || screen.contains("Cortex"),
            "view-status paint missing expected chrome; got:\n{screen}"
        );
    }

    #[test]
    fn designer_set_transcript_and_busy_do_not_panic_on_paint() {
        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_set_transcript(vec![
            Entry::User("hello fixture".to_owned()),
            Entry::Assistant(super::super::AssistantTurn {
                reasoning: String::new(),
                answer: "hi from fixture".to_owned(),
            }),
        ]);
        app.designer_set_busy(true);
        assert!(app.is_busy());
        let screen = paint(&app);
        assert!(!screen.trim().is_empty());
    }

    #[test]
    fn designer_overlays_and_sidebar_paint_smoke() {
        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_palette("help", vec!["Help".to_owned()]);
        assert!(app.palette_open());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_permission_preview("bash", "rm -rf /tmp/fixture");
        assert!(app.permission_open());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_set_sidebar(true, SidebarPanel::Context);
        assert!(app.sidebar_visible());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_theme_gallery();
        assert!(app.theme_gallery_open());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_diff_viewer(Vec::new());
        assert!(app.diff_viewer_open());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_set_shell_mode(true);
        app.designer_set_plan_mode_chrome(true);
        assert!(app.shell_mode());
        assert!(app.plan_mode());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_slash();
        assert!(app.slash_menu_open());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_mention(Vec::new());
        assert!(app.mention_open());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_set_transcript(vec![Entry::User("needle fixture".to_owned())]);
        app.designer_open_find("needle");
        assert!(app.find().is_some());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_open_toast(ToastLevel::Error, "fixture toast error");
        assert!(app.has_toasts());
        let _ = paint(&app);

        let mut app = App::designer_prepare_base(fixture_status());
        app.designer_set_onboarding(true);
        assert!(app.onboarding_active());
        let _ = paint(&app);
    }
}
