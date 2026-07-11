//! Public deterministic audit and named-scenario surface for the v3 harness.

use std::process::ExitCode;

use mockup_harness_v3::{
    canonical_inventory,
    renderer::{
        audit_json, flow_manifest_json, parse_selector, preflight_render, publish, RenderExit,
    },
    scenario::replay_case,
    validate_canonical_map,
};

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.as_slice() == ["--list"] {
        return list_inventory();
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "negative-fixture")
    {
        return emit_negative_fixture(&arguments);
    }
    if let Some((case_id, tick, output)) = parse_case_arguments(&arguments) {
        return emit_case(case_id, tick, output);
    }
    if let Some(audit_arguments) = arguments.strip_prefix(&["--audit-json".to_owned()]) {
        return emit_audit_json(audit_arguments);
    }
    emit_render(&arguments)
}

fn emit_negative_fixture(arguments: &[String]) -> ExitCode {
    let [command, fixture_id] = arguments else {
        eprintln!("negative fixture usage error: expected one fixture ID");
        print_usage();
        return ExitCode::from(2);
    };
    if command != "negative-fixture" {
        eprintln!("negative fixture usage error: expected negative-fixture command");
        print_usage();
        return ExitCode::from(2);
    }
    if !mockup_harness_v3::negative_fixtures::fixture_manifest()
        .iter()
        .any(|fixture| fixture.id == fixture_id)
    {
        eprintln!("negative fixture usage error: unknown fixture ID: {fixture_id}");
        return ExitCode::from(2);
    }
    match mockup_harness_v3::negative_fixtures::run_public_negative_fixture(fixture_id) {
        Ok(()) => {
            eprintln!("negative fixture unexpectedly accepted invalid input: {fixture_id}");
            ExitCode::from(3)
        }
        Err(error) => {
            eprintln!("negative fixture validation failed: {error}");
            ExitCode::from(3)
        }
    }
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

fn emit_audit_json(arguments: &[String]) -> ExitCode {
    let selector = if arguments.is_empty() {
        None
    } else {
        match parse_selector(arguments) {
            Ok(selector) => Some(selector),
            Err(error) => {
                eprintln!("audit usage error: {error}");
                return ExitCode::from(2);
            }
        }
    };
    match audit_json(selector.as_ref()) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("audit validation failed: {error}");
            ExitCode::from(3)
        }
    }
}

fn emit_render(arguments: &[String]) -> ExitCode {
    let selector = match parse_selector(arguments) {
        Ok(selector) => selector,
        Err(error) => {
            eprintln!("render usage error: {error}");
            print_usage();
            return ExitCode::from(2);
        }
    };
    let plan = match preflight_render(&selector) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("render validation failed: {error}");
            return ExitCode::from(3);
        }
    };
    let flow = plan.flow_name().and_then(|name| {
        mockup_harness_v3::flow_manifests()
            .iter()
            .find(|flow| flow.name == name)
    });
    match publish(&plan) {
        Ok(()) => {
            if let Some(flow) = flow {
                println!("{}", flow_manifest_json(flow));
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("render failed: {}", error.message);
            match error.exit {
                RenderExit::Io => ExitCode::from(1),
                RenderExit::Validation => ExitCode::from(3),
                RenderExit::Busy => ExitCode::from(75),
            }
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage: mockup-harness-v3 --list | --implemented | --all | <screen> | <category> | --flow <name> | --audit-json [selector] | --case <case-id> --tick <n> (--audit-json | --txt | --png <absolute-temporary-path>) | negative-fixture <fixture-id>"
    );
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
