//! Public pure-Cortex dump CLI (F0 chrome foundation).
//!
//! Commands:
//!   --list                 list pilot IDs
//!   --publish-pilots       render + publish login + chat-turn into mockups-v3
//!   --publish <id>         publish one pilot id

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
                "{}\t{}\timplemented\tpilot\tpure-cortex-shell",
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
    eprintln!("usage error");
    print_usage();
    ExitCode::from(2)
}

fn print_usage() {
    eprintln!(
        "mockup-harness-v4 (pure Cortex dump)\n\
         --list\n\
         --publish-pilots\n\
         --publish <login|chat-turn>\n"
    );
}
