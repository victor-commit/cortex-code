//! Public pure-Cortex dump CLI (F1–F3 auth + conversation + tools).
//!
//! Commands:
//!   --list                 list implemented IDs
//!   --publish-pilots       render + publish all F1–F3 IDs into mockups-v3
//!   --publish <id>         publish one id

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
                "{}\t{}\timplemented\tf1-f3\tpure-cortex-shell",
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

    // Optional filter publish: --publish-batch auth|conversation|tools
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
            other => {
                eprintln!("unknown batch: {other} (auth|conversation|tools)");
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
        "mockup-harness-v4 (pure Cortex dump F1-F3)\n\
         --list\n\
         --publish-pilots\n\
         --publish <id>\n\
         --publish-batch <auth|conversation|tools>\n"
    );
}
