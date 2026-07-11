//! Public deterministic audit and named-scenario surface for the v3 harness.

use std::process::ExitCode;

use mockup_harness_v3::{
    audit::audit_screen, canonical_inventory, scenario::replay_case, validate_canonical_map,
};

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.as_slice() == ["--list"] {
        return list_inventory();
    }
    if arguments.as_slice() == ["--audit-json"] {
        return emit_audit_json();
    }
    if let Some((case_id, tick, output)) = parse_case_arguments(&arguments) {
        return emit_case(case_id, tick, output);
    }

    eprintln!(
        "usage: mockup-harness-v3 --list | --audit-json | --case <case-id> --tick <n> (--audit-json | --txt | --png <absolute-temporary-path>)"
    );
    ExitCode::from(2)
}

fn list_inventory() -> ExitCode {
    if let Err(error) = validate_canonical_map() {
        eprintln!("inventory validation failed: {error}");
        return ExitCode::from(3);
    }

    for screen in canonical_inventory() {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            screen.category.as_str(),
            screen.id,
            screen.status.as_str(),
            screen.flow.as_str(),
            screen.surface.as_str(),
        );
    }

    ExitCode::SUCCESS
}

fn emit_audit_json() -> ExitCode {
    match audit_screen("permission-shell", "public-audit") {
        Ok(audit) => {
            println!("{}", audit.to_json());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("audit validation failed: {error}");
            ExitCode::from(3)
        }
    }
}

#[derive(Clone, Copy)]
enum CaseOutput<'a> {
    AuditJson,
    Txt,
    Png(&'a str),
}

fn parse_case_arguments(arguments: &[String]) -> Option<(&str, u32, CaseOutput<'_>)> {
    let [case_flag, case_id, tick_flag, tick, output @ ..] = arguments else {
        return None;
    };
    if case_flag != "--case" || tick_flag != "--tick" {
        return None;
    }
    let tick = tick.parse::<u32>().ok()?;
    match output {
        [flag] if flag == "--audit-json" => Some((case_id, tick, CaseOutput::AuditJson)),
        [flag] if flag == "--txt" => Some((case_id, tick, CaseOutput::Txt)),
        [flag, path] if flag == "--png" => Some((case_id, tick, CaseOutput::Png(path))),
        _ => None,
    }
}

fn emit_case(case_id: &str, tick: u32, output: CaseOutput<'_>) -> ExitCode {
    let replay = match replay_case(case_id, tick) {
        Ok(replay) => replay,
        Err(error) => {
            eprintln!("case usage error: {error}");
            return ExitCode::from(2);
        }
    };

    match output {
        CaseOutput::AuditJson => {
            println!("{}", replay.to_json());
            ExitCode::SUCCESS
        }
        CaseOutput::Txt => {
            print!("{}", replay.audit.render_txt());
            ExitCode::SUCCESS
        }
        CaseOutput::Png(destination) => emit_case_png(destination, &replay),
    }
}

#[cfg(feature = "png")]
fn emit_case_png(destination: &str, replay: &mockup_harness_v3::scenario::CaseReplay) -> ExitCode {
    use std::path::Path;

    if let Err(error) =
        mockup_harness_v3::scenario::validate_png_destination(Path::new(destination))
    {
        eprintln!("case usage error: {error}");
        return ExitCode::from(2);
    }
    match mockup_harness_v3::scenario::write_scenario_png(Path::new(destination), replay) {
        Ok(hash) => {
            println!("{hash}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("scenario PNG failure: {error}");
            ExitCode::from(1)
        }
    }
}

#[cfg(not(feature = "png"))]
fn emit_case_png(
    _destination: &str,
    _replay: &mockup_harness_v3::scenario::CaseReplay,
) -> ExitCode {
    eprintln!("scenario PNG support requires the png feature");
    ExitCode::from(1)
}
