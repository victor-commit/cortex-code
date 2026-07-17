//! Public pure-Cortex dump CLI (F1–F7 auth…orchestration + 35 gate frames).
//!
//! Commands:
//!   --list                          list implemented IDs
//!   --publish-pilots                publish all 86 IDs into mockups-v3
//!   --publish <id>                  publish one id
//!   --publish-batch <name>          publish one batch
//!   --publish-gates                 publish all 35 gate frames into mockups-v3-gates

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        print_usage();
        return ExitCode::SUCCESS;
    }
    if args == ["--list"] {
        for pilot in mockup_harness_v4::list_pilots() {
            println!(
                "{}\t{}\timplemented\tf1-f7\tpure-cortex-shell",
                pilot.category, pilot.id
            );
        }
        return ExitCode::SUCCESS;
    }
    if args == ["--publish-pilots"] {
        return match mockup_harness_v4::publish_pilots() {
            Ok(results) => {
                for result in results {
                    println!(
                        "published {}/{} txt={} png={}",
                        result.category,
                        result.id,
                        result.txt_sha256,
                        result.png_sha256.as_deref().unwrap_or("-")
                    );
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("publish-pilots failed: {error}");
                ExitCode::from(3)
            }
        };
    }
    if args == ["--publish-gates"] {
        return match mockup_harness_v4::publish_gates() {
            Ok(results) => {
                println!("published {} gate frames", results.len());
                for result in results {
                    println!(
                        "gate {} txt={} png={}",
                        result.category,
                        result.txt_sha256,
                        result.png_sha256.as_deref().unwrap_or("-")
                    );
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("publish-gates failed: {error}");
                ExitCode::from(3)
            }
        };
    }
    if args.len() == 2 && args[0] == "--publish" {
        let id = &args[1];
        let Some(spec) = mockup_harness_v4::list_pilots()
            .iter()
            .find(|p| p.id == id.as_str())
        else {
            eprintln!("unknown pilot id: {id}");
            return ExitCode::from(2);
        };
        return match mockup_harness_v4::publish_pilot(spec) {
            Ok(result) => {
                println!(
                    "published {}/{} txt={} png={}",
                    result.category,
                    result.id,
                    result.txt_sha256,
                    result.png_sha256.as_deref().unwrap_or("-")
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("publish failed: {error}");
                ExitCode::from(3)
            }
        };
    }

    if args.len() == 2 && args[0] == "--publish-batch" {
        let batch = args[1].as_str();
        let filter: Box<dyn Fn(&str) -> bool> = match batch {
            "auth" => Box::new(|id| {
                matches!(
                    id,
                    "login"
                        | "connect-provider"
                        | "connect-method"
                        | "connect-oauth"
                        | "connect-apikey"
                        | "connect-connected"
                        | "connect-disconnect"
                )
            }),
            "conversation" => Box::new(|id| {
                matches!(
                    id,
                    "home"
                        | "onboarding"
                        | "chat-turn"
                        | "chat-long-scroll"
                        | "streaming"
                        | "reasoning"
                        | "reasoning-collapsed"
                        | "rich-response"
                        | "empty"
                        | "toasts"
                )
            }),
            "tools" => Box::new(|id| {
                matches!(
                    id,
                    "tool-calls"
                        | "tool-running"
                        | "tool-success"
                        | "tool-success-expanded"
                        | "tool-failed"
                        | "tool-task"
                        | "tool-task-expanded"
                        | "shell"
                        | "todo-list"
                        | "compact"
                        | "diff-edit"
                )
            }),
            "pickers" | "f4" => Box::new(|id| {
                matches!(
                    id,
                    "slash-menu"
                        | "command-palette"
                        | "command-palette-submenu"
                        | "model-picker"
                        | "model-picker-filtered"
                        | "agent-picker"
                        | "skills-picker"
                        | "session-picker"
                        | "variant-picker"
                        | "fork-picker"
                        | "theme-picker"
                        | "export-picker"
                        | "attachment-picker"
                        | "subagent-picker"
                        | "file-mention"
                        | "queue-manager"
                        | "composer-multiline"
                )
            }),
            "permissions" | "f5" => Box::new(|id| {
                matches!(
                    id,
                    "permission-shell"
                        | "permission-file-edit"
                        | "permission-deny-reason"
                        | "blocked-command"
                        | "turn-error"
                        | "error-network"
                        | "error-context"
                        | "rate-limit"
                        | "interrupt"
                        | "checkpoint-undo"
                        | "share"
                )
            }),
            "views" | "f6" => Box::new(|id| {
                matches!(
                    id,
                    "status"
                        | "timeline"
                        | "mcp"
                        | "diff-viewer"
                        | "diff-viewer-staging"
                        | "help"
                        | "review"
                        | "theme-gallery-dark"
                        | "theme-gallery-light"
                        | "sidebar-context"
                        | "sidebar-files"
                        | "sidebar-lsp"
                        | "sidebar-todo"
                        | "sidebar-subagents"
                        | "sidebar-mcp"
                        | "sidebar-queue"
                )
            }),
            "orchestration" | "f7" => Box::new(|id| {
                matches!(
                    id,
                    "mode-switch"
                        | "mode-switch-refused"
                        | "workflow-launch"
                        | "workflow-launch-blocked"
                        | "workflow-monitor"
                        | "workflow-monitor-failure"
                        | "team-board"
                        | "reviewer-output"
                        | "reviewer-approved"
                        | "verification-ladder"
                        | "intent-scorer-pass"
                        | "intent-scorer-fail"
                        | "concurrency-metrics"
                        | "tool-envelope"
                        | "onboarding"
                )
            }),
            "f4-f6" => Box::new(|id| {
                matches!(
                    id,
                    "slash-menu"
                        | "command-palette"
                        | "command-palette-submenu"
                        | "model-picker"
                        | "model-picker-filtered"
                        | "agent-picker"
                        | "skills-picker"
                        | "session-picker"
                        | "variant-picker"
                        | "fork-picker"
                        | "theme-picker"
                        | "export-picker"
                        | "attachment-picker"
                        | "subagent-picker"
                        | "file-mention"
                        | "queue-manager"
                        | "composer-multiline"
                        | "permission-shell"
                        | "permission-file-edit"
                        | "permission-deny-reason"
                        | "blocked-command"
                        | "turn-error"
                        | "error-network"
                        | "error-context"
                        | "rate-limit"
                        | "interrupt"
                        | "checkpoint-undo"
                        | "share"
                        | "status"
                        | "timeline"
                        | "mcp"
                        | "diff-viewer"
                        | "diff-viewer-staging"
                        | "help"
                        | "review"
                        | "theme-gallery-dark"
                        | "theme-gallery-light"
                        | "sidebar-context"
                        | "sidebar-files"
                        | "sidebar-lsp"
                        | "sidebar-todo"
                        | "sidebar-subagents"
                        | "sidebar-mcp"
                        | "sidebar-queue"
                )
            }),
            other => {
                eprintln!(
                    "unknown batch: {other} (auth|conversation|tools|pickers|permissions|views|orchestration|f4-f6|f7)"
                );
                return ExitCode::from(2);
            }
        };
        let mut ok = true;
        for spec in mockup_harness_v4::list_pilots() {
            if !filter(spec.id) {
                continue;
            }
            match mockup_harness_v4::publish_pilot(spec) {
                Ok(result) => println!(
                    "published {}/{} txt={} png={}",
                    result.category,
                    result.id,
                    result.txt_sha256,
                    result.png_sha256.as_deref().unwrap_or("-")
                ),
                Err(error) => {
                    eprintln!("publish {id} failed: {error}", id = spec.id);
                    ok = false;
                }
            }
        }
        return if ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(3)
        };
    }

    eprintln!("usage error");
    print_usage();
    ExitCode::from(2)
}

fn print_usage() {
    eprintln!(
        "mockup-harness-v4 (pure Cortex dump F1-F7 + gates)\n\
         --list\n\
         --publish-pilots\n\
         --publish-gates\n\
         --publish <id>\n\
         --publish-batch <auth|conversation|tools|pickers|permissions|views|orchestration|f4-f6|f7>\n"
    );
}
