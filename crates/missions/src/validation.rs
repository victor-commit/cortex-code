//! Milestone validation gate (programmatic hook + state recording).

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::schema::{MissionState, ValidationAssertionState};
use crate::store::{
    load_mission_state, load_validation_state, save_mission_state, save_validation_state,
    MissionPaths, StoreError,
};

/// Outcome of a validation gate invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationOutcome {
    /// Gate passed; milestone may seal.
    Passed,
    /// Gate failed; seal blocked.
    Failed,
    /// Gate skipped (no commands / dry).
    Skipped,
}

/// Result payload recorded under `validation/<milestone>/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationGateResult {
    /// Milestone id.
    pub milestone: String,
    /// Overall outcome.
    pub outcome: ValidationOutcome,
    /// Wall-clock ms when invoked.
    pub ts_ms: u64,
    /// Command transcripts.
    pub commands: Vec<ValidationCommandResult>,
    /// Human summary.
    pub summary: String,
}

/// One command run by the gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationCommandResult {
    /// Command line.
    pub command: String,
    /// Exit code (or -1 if spawn failed).
    pub exit_code: i32,
    /// Captured stdout+stderr (truncated).
    pub output: String,
}

/// Pluggable validation hook (tests inject fakes; product uses [`ServicesYamlHook`]).
pub trait ValidationHook: Send + Sync {
    /// Run validation for a completed milestone. Returning `Failed` blocks seal.
    fn validate(&self, paths: &MissionPaths, milestone: &str) -> ValidationGateResult;
}

/// No-op hook that always passes (used when no services.yaml commands are configured).
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysPassHook;

impl ValidationHook for AlwaysPassHook {
    fn validate(&self, _paths: &MissionPaths, milestone: &str) -> ValidationGateResult {
        ValidationGateResult {
            milestone: milestone.to_owned(),
            outcome: ValidationOutcome::Passed,
            ts_ms: now_ms(),
            commands: vec![],
            summary: format!("milestone {milestone}: always-pass hook"),
        }
    }
}

/// Hook that fails (tests seal-blocking).
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysFailHook;

impl ValidationHook for AlwaysFailHook {
    fn validate(&self, _paths: &MissionPaths, milestone: &str) -> ValidationGateResult {
        ValidationGateResult {
            milestone: milestone.to_owned(),
            outcome: ValidationOutcome::Failed,
            ts_ms: now_ms(),
            commands: vec![ValidationCommandResult {
                command: "false".into(),
                exit_code: 1,
                output: "forced failure".into(),
            }],
            summary: format!("milestone {milestone}: always-fail hook"),
        }
    }
}

/// Run shell commands from a simple list (fixture services policy).
#[derive(Debug, Clone)]
pub struct CommandListHook {
    /// Commands to run (each is `/bin/sh -c <cmd>`). Empty ⇒ skipped pass.
    pub commands: Vec<String>,
    /// Working directory for commands (mission working directory or mission dir).
    pub cwd: Option<std::path::PathBuf>,
}

impl ValidationHook for CommandListHook {
    fn validate(&self, _paths: &MissionPaths, milestone: &str) -> ValidationGateResult {
        if self.commands.is_empty() {
            return ValidationGateResult {
                milestone: milestone.to_owned(),
                outcome: ValidationOutcome::Skipped,
                ts_ms: now_ms(),
                commands: vec![],
                summary: format!("milestone {milestone}: no validation commands"),
            };
        }
        let mut results = Vec::new();
        let mut all_ok = true;
        for cmd in &self.commands {
            let mut c = Command::new("sh");
            c.arg("-c").arg(cmd);
            if let Some(cwd) = &self.cwd {
                c.current_dir(cwd);
            }
            match c.output() {
                Ok(out) => {
                    let code = out.status.code().unwrap_or(-1);
                    if code != 0 {
                        all_ok = false;
                    }
                    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                    let err = String::from_utf8_lossy(&out.stderr);
                    if !err.is_empty() {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&err);
                    }
                    if text.len() > 4000 {
                        text.truncate(4000);
                        text.push('…');
                    }
                    results.push(ValidationCommandResult {
                        command: cmd.clone(),
                        exit_code: code,
                        output: text,
                    });
                }
                Err(e) => {
                    all_ok = false;
                    results.push(ValidationCommandResult {
                        command: cmd.clone(),
                        exit_code: -1,
                        output: e.to_string(),
                    });
                }
            }
        }
        ValidationGateResult {
            milestone: milestone.to_owned(),
            outcome: if all_ok {
                ValidationOutcome::Passed
            } else {
                ValidationOutcome::Failed
            },
            ts_ms: now_ms(),
            commands: results,
            summary: format!(
                "milestone {milestone}: {}",
                if all_ok { "passed" } else { "failed" }
            ),
        }
    }
}

/// Invoke the validation hook, persist synthesis under `validation/<milestone>/`,
/// update `validation-state.json` + `state.json`, and return the result.
///
/// A `Failed` outcome **blocks seal** (mission state last_validation records failure;
/// callers should not mark the milestone sealed).
pub fn invoke_milestone_validation(
    paths: &MissionPaths,
    milestone: &str,
    hook: &dyn ValidationHook,
) -> Result<ValidationGateResult, StoreError> {
    let result = hook.validate(paths, milestone);
    record_validation_result(paths, &result)?;
    Ok(result)
}

/// Persist a validation result to disk and flip mission state flags.
pub fn record_validation_result(
    paths: &MissionPaths,
    result: &ValidationGateResult,
) -> Result<(), StoreError> {
    let dir = paths.root.join("validation").join(&result.milestone);
    fs::create_dir_all(&dir)?;
    let body = serde_json::to_string_pretty(result)?;
    fs::write(dir.join("scrutiny.json"), body)?;

    // Update validation-state.json assertions for this milestone prefix / any pending.
    let mut vstate = load_validation_state(paths).unwrap_or_default();
    let new_status = match result.outcome {
        ValidationOutcome::Passed | ValidationOutcome::Skipped => "passed",
        ValidationOutcome::Failed => "failed",
    };
    if vstate.assertions.is_empty() {
        vstate.assertions.push(ValidationAssertionState {
            id: format!("VAL-{}-GATE", result.milestone.to_ascii_uppercase()),
            status: new_status.into(),
        });
    } else {
        for a in &mut vstate.assertions {
            if a.status == "pending" {
                a.status = new_status.into();
            }
        }
    }
    save_validation_state(paths, &vstate)?;

    let mut state = load_mission_state(paths).unwrap_or(MissionState {
        id: paths.id.clone(),
        status: "running".into(),
        parallel_max: 3,
        live_workers: Vec::new(),
        validation_invoked: false,
        last_validation: None,
    });
    state.validation_invoked = true;
    state.last_validation = Some(format!(
        "{}:{}",
        result.milestone,
        match result.outcome {
            ValidationOutcome::Passed => "passed",
            ValidationOutcome::Failed => "failed",
            ValidationOutcome::Skipped => "skipped",
        }
    ));
    // Failure blocks seal: keep mission status from becoming a clean sealed complete when failed.
    if result.outcome == ValidationOutcome::Failed && state.status == "completed" {
        state.status = "validation_failed".into();
    }
    save_mission_state(paths, &state)?;
    Ok(())
}

/// True when the last recorded validation failed (seal blocked).
pub fn seal_blocked(paths: &MissionPaths) -> bool {
    load_mission_state(paths)
        .ok()
        .and_then(|s| s.last_validation)
        .is_some_and(|v| v.contains(":failed"))
}

/// Write a marker file under a path (tests).
pub fn write_gate_marker(dir: &Path, name: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join(name), b"invoked")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{create_fixture_mission, CreateMissionOpts};
    use tempfile::tempdir;

    #[test]
    fn validation_gate_records_pass_and_sets_flag() {
        let tmp = tempdir().unwrap();
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("v1".into()),
                title: "val".into(),
                parallel_max: 3,
                features: vec![],
            },
        )
        .unwrap();
        let hook = AlwaysPassHook;
        let result = invoke_milestone_validation(&paths, "m1", &hook).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Passed);
        assert!(paths
            .root
            .join("validation/m1/scrutiny.json")
            .exists());
        let state = load_mission_state(&paths).unwrap();
        assert!(state.validation_invoked);
        assert_eq!(state.last_validation.as_deref(), Some("m1:passed"));
    }

    #[test]
    fn validation_failure_blocks_seal() {
        let tmp = tempdir().unwrap();
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("v2".into()),
                title: "val".into(),
                parallel_max: 3,
                features: vec![],
            },
        )
        .unwrap();
        // Pretend mission completed then validation fails.
        let mut st = load_mission_state(&paths).unwrap();
        st.status = "completed".into();
        save_mission_state(&paths, &st).unwrap();

        let result = invoke_milestone_validation(&paths, "m1", &AlwaysFailHook).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Failed);
        assert!(seal_blocked(&paths));
        let state = load_mission_state(&paths).unwrap();
        assert_eq!(state.status, "validation_failed");
    }

    #[test]
    fn command_list_hook_runs_true() {
        let tmp = tempdir().unwrap();
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("v3".into()),
                title: "val".into(),
                parallel_max: 3,
                features: vec![],
            },
        )
        .unwrap();
        let hook = CommandListHook {
            commands: vec!["true".into()],
            cwd: None,
        };
        let result = invoke_milestone_validation(&paths, "m1", &hook).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Passed);
        assert_eq!(result.commands[0].exit_code, 0);
    }
}
