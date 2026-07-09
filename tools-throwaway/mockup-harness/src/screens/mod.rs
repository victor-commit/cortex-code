//! v2 screen modules - one `pub mod` per mockup screen, mirrored by a
//! registry entry in `main.rs`.
//!
//! The 45 v1 screen sources are parked UNCOMPILED in `../screens_v1_reference/`
//! (and reachable in git history via the baseline commit). They reference the
//! retired v1 chrome API and must not be re-declared here.

// session
pub mod agent_picker;
pub mod attachment_picker;
pub mod chat_long_scroll;
pub mod chat_turn;
pub mod blocked_command;
pub mod command_palette;
pub mod command_palette_submenu;
pub mod composer_multiline;
pub mod diff_edit;
pub mod diff_viewer;
pub mod diff_viewer_staging;
pub mod export_picker;
pub mod file_mention;
pub mod fork_picker;
pub mod help;
pub mod home;
pub mod model_picker;
pub mod model_picker_filtered;
pub mod mcp;
pub mod login;
pub mod onboarding;
pub mod permission_file_edit;
pub mod permission_deny_reason;
pub mod permission_shell;
pub mod reasoning;
pub mod reasoning_collapsed;
pub mod review;
pub mod rich_response;
pub mod queue_manager;
pub mod shell;
pub mod sidebar_context;
pub mod sidebar_files;
pub mod sidebar_lsp;
pub mod sidebar_mcp;
pub mod sidebar_queue;
pub mod sidebar_subagents;
pub mod sidebar_todo;
pub mod slash_menu;
pub mod streaming;
pub mod status;
pub mod skills_picker;
pub mod session_picker;
pub mod subagent_picker;
pub mod variant_picker;
pub mod theme_picker;
pub mod theme_gallery_dark;
pub mod theme_gallery_light;
pub mod timeline;
pub mod turn_error;
pub mod tool_calls;
pub mod tool_failed;
pub mod tool_running;
pub mod tool_success;
pub mod tool_success_expanded;
pub mod tool_task;
pub mod tool_task_expanded;
pub mod compact;
pub mod connect_method;
pub mod connect_apikey;
pub mod connect_connected;
pub mod connect_disconnect;
pub mod connect_oauth;
pub mod connect_provider;
pub mod todo_list;
