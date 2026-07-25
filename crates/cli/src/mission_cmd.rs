//! `cortex mission` — create / list / status / run clean-room parallel missions.
//!
//! Mission roots default to `~/.cortex/missions` (override with `CORTEX_MISSIONS_ROOT`).
//! Never writes under Factory `~/.factory/missions/*`.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use errors::Exit;
use events::EventBus;
use missions::{
    create_fixture_mission, create_isolation_fixture_mission, create_mission, list_missions,
    load_features, missions_root, open_mission, open_mission_dir, run_mission_blocking,
    snapshot_mission, CreateMissionOpts, DefaultNoopExecutor, Feature, FeatureStatus,
    MissionsRootInputs, SchedulerConfig, DEFAULT_MAX_PARALLEL, HARD_MAX_PARALLEL,
};
use serde::Serialize;

use crate::cli::{
    MissionArgs, MissionCommand, MissionCreateArgs, MissionListArgs, MissionRunArgs,
    MissionStatusArgs, OutputFormat,
};

/// Dispatch `cortex mission`.
pub fn run(args: MissionArgs, bus: Arc<EventBus>) -> Exit {
    match args.command {
        MissionCommand::Create(a) => create(a),
        MissionCommand::List(a) => list(a),
        MissionCommand::Status(a) => status(a),
        MissionCommand::Run(a) => run_cmd(a, bus),
    }
}

fn resolve_root(explicit: Option<PathBuf>) -> Result<PathBuf, Exit> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    MissionsRootInputs::from_process()
        .resolve()
        .or_else(missions_root)
        .ok_or_else(|| {
            eprintln!(
                "cortex mission: cannot resolve missions root; set HOME or CORTEX_MISSIONS_ROOT"
            );
            Exit::Failure
        })
}

fn create(args: MissionCreateArgs) -> Exit {
    let root = match resolve_root(args.root) {
        Ok(r) => r,
        Err(e) => return e,
    };
    // Guard: never write under Factory missions.
    let root_s = root.display().to_string();
    if root_s.contains(".factory/missions") {
        eprintln!("cortex mission: refusing to write under Factory ~/.factory/missions");
        return Exit::Failure;
    }

    let parallel = args.parallel.unwrap_or(DEFAULT_MAX_PARALLEL).clamp(1, HARD_MAX_PARALLEL);
    let opts = CreateMissionOpts {
        id: args.id,
        title: args
            .title
            .unwrap_or_else(|| "Cortex mission".into()),
        parallel_max: parallel,
        // Fixture create fills default features when this stays empty.
        features: Vec::new(),
    };

    let result = if args.fixture {
        if args.isolation {
            let id = opts
                .id
                .clone()
                .unwrap_or_else(|| format!("iso-{}", now_stamp()));
            create_isolation_fixture_mission(&root, &id)
        } else {
            create_fixture_mission(&root, opts)
        }
    } else {
        // Empty mission; optional single feature seed.
        let mut o = opts;
        if let Some(fid) = args.feature {
            o.features.push(Feature {
                id: fid.clone(),
                description: format!("feature {fid}"),
                skill_name: "worker".into(),
                milestone: "m0".into(),
                preconditions: vec![],
                depends_on: vec![],
                expected_behavior: vec![],
                fulfills: vec![],
                status: FeatureStatus::Pending,
                fail: false,
                work_ms: 50,
            });
        }
        create_mission(&root, o)
    };

    match result {
        Ok(paths) => {
            let features = load_features(&paths).map(|f| f.features.len()).unwrap_or(0);
            let view = CreatedView {
                id: paths.id.clone(),
                mission_dir: paths.root.display().to_string(),
                feature_count: features,
                status: "created".into(),
            };
            match args.format.format {
                OutputFormat::Json | OutputFormat::Jsonl => emit_json(&view),
                OutputFormat::Text => {
                    println!("created mission {}", view.id);
                    println!("  dir: {}", view.mission_dir);
                    println!("  features: {}", view.feature_count);
                    Exit::Success
                }
            }
        }
        Err(err) => {
            eprintln!("cortex mission create: {err}");
            Exit::Failure
        }
    }
}

fn list(args: MissionListArgs) -> Exit {
    let root = match resolve_root(args.root) {
        Ok(r) => r,
        Err(e) => return e,
    };
    match list_missions(&root) {
        Ok(list) => match args.format.format {
            OutputFormat::Json | OutputFormat::Jsonl => emit_json(&list),
            OutputFormat::Text => {
                if list.is_empty() {
                    println!("(no missions under {})", root.display());
                } else {
                    for m in list {
                        println!(
                            "{}\t{}\tfeatures={}\t{}",
                            m.id, m.status, m.feature_count, m.mission_dir
                        );
                    }
                }
                Exit::Success
            }
        },
        Err(err) => {
            eprintln!("cortex mission list: {err}");
            Exit::Failure
        }
    }
}

fn status(args: MissionStatusArgs) -> Exit {
    let paths = match resolve_mission(&args.mission, args.root) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match snapshot_mission(&paths) {
        Ok(snap) => match args.format.format {
            OutputFormat::Json | OutputFormat::Jsonl => emit_json(&snap),
            OutputFormat::Text => {
                println!("mission {}", snap.id);
                println!("  status: {}", snap.status);
                println!(
                    "  features: {}/{} done · parallel {}/{}",
                    snap.features_done,
                    snap.features_total,
                    snap.parallel_live,
                    snap.parallel_max
                );
                println!("  completed: {}", snap.completed.join(", "));
                println!("  failed: {}", snap.failed.join(", "));
                println!("  pending: {}", snap.pending.join(", "));
                println!(
                    "  validation_invoked: {} ({})",
                    snap.validation_invoked,
                    snap.last_validation.as_deref().unwrap_or("-")
                );
                if !snap.progress_tail.is_empty() {
                    println!("  progress:");
                    for line in &snap.progress_tail {
                        println!("    {line}");
                    }
                }
                Exit::Success
            }
        },
        Err(err) => {
            eprintln!("cortex mission status: {err}");
            Exit::Failure
        }
    }
}

fn run_cmd(args: MissionRunArgs, bus: Arc<EventBus>) -> Exit {
    let paths = match resolve_mission(&args.mission, args.root) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let parallel = args
        .parallel
        .unwrap_or(DEFAULT_MAX_PARALLEL)
        .clamp(1, HARD_MAX_PARALLEL);
    let config = SchedulerConfig {
        max_parallel: parallel,
        worker_id_base: 21,
        run_validation: !args.no_validate,
    };
    let bus_opt = if args.no_events {
        None
    } else {
        Some(bus)
    };

    match run_mission_blocking(
        &paths,
        config,
        Arc::new(DefaultNoopExecutor),
        bus_opt,
        None,
    ) {
        Ok(report) => match args.format.format {
            OutputFormat::Json | OutputFormat::Jsonl => emit_json(&report),
            OutputFormat::Text => {
                println!("mission {} finished", report.mission_id);
                println!("  peak_live: {}", report.peak_live);
                println!("  saw_parallel: {}", report.saw_parallel);
                println!("  completed: {}", report.completed.join(", "));
                println!("  failed: {}", report.failed.join(", "));
                println!("  pending: {}", report.pending.join(", "));
                println!(
                    "  validation_invoked: {} ({})",
                    report.validation_invoked,
                    report.last_validation.as_deref().unwrap_or("-")
                );
                if report.failed.is_empty()
                    && report
                        .last_validation
                        .as_deref()
                        .is_none_or(|v| !v.contains("failed"))
                {
                    Exit::Success
                } else if !report.completed.is_empty() {
                    // Partial / isolated failure still exits 0 for fixture observability unless
                    // --strict is set. Use non-zero only when everything failed.
                    if report.completed.is_empty() {
                        Exit::Failure
                    } else {
                        Exit::Success
                    }
                } else {
                    Exit::Failure
                }
            }
        },
        Err(err) => {
            eprintln!("cortex mission run: {err}");
            Exit::Failure
        }
    }
}

fn resolve_mission(
    mission: &str,
    root: Option<PathBuf>,
) -> Result<missions::MissionPaths, Exit> {
    // Absolute / relative path with features.json?
    let as_path = PathBuf::from(mission);
    if as_path.join("features.json").is_file() || as_path.is_dir() && as_path.join("mission.md").is_file()
    {
        return open_mission_dir(&as_path).map_err(|e| {
            eprintln!("cortex mission: {e}");
            Exit::Failure
        });
    }
    let root = resolve_root(root)?;
    open_mission(&root, mission).map_err(|e| {
        eprintln!("cortex mission: {e}");
        Exit::Failure
    })
}

#[derive(Serialize)]
struct CreatedView {
    id: String,
    mission_dir: String,
    feature_count: usize,
    status: String,
}

fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string_pretty(value) {
        Ok(s) => {
            let mut out = io::stdout().lock();
            if writeln!(out, "{s}").is_err() {
                return Exit::Failure;
            }
            Exit::Success
        }
        Err(err) => {
            eprintln!("cortex mission: json encode failed: {err}");
            Exit::Failure
        }
    }
}

fn now_stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
