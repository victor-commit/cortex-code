//! `cortex upgrade` and `cortex uninstall`: safe, self-describing lifecycle commands.
//!
//! Neither command touches anything outside Cortex's own artifacts. `upgrade` reports status only
//! (a from-source mono-binary is updated by its installer, not by self-mutation) and changes no
//! files. `uninstall` defaults to a dry run that merely lists the Cortex-owned data and config
//! paths; `--yes` deletes exactly those paths and nothing else.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use db::DbPathInputs;
use errors::Exit;
use serde::Serialize;

use crate::cli::{OutputFormat, UninstallArgs, UpgradeArgs};

/// Known Cortex-owned filenames placed directly inside a configured data directory.
const DATA_FILES: &[&str] = &[
    "cortex.db",
    "cortex.db-wal",
    "cortex.db-shm",
    "auth.json",
    "mcp-oauth.json",
];

// ---------------------------------------------------------------------------
// upgrade
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct UpgradeReport {
    #[serde(rename = "type")]
    kind: &'static str,
    current_version: &'static str,
    supported: bool,
    status: &'static str,
    message: &'static str,
}

const UPGRADE_MESSAGE: &str = "self-update is not performed by this build; reinstall through your \
     installer (cargo, git checkout, or package manager) to upgrade. No files were changed.";

pub fn run_upgrade(args: UpgradeArgs) -> Exit {
    let report = UpgradeReport {
        kind: "upgrade",
        current_version: env!("CARGO_PKG_VERSION"),
        supported: false,
        status: "unsupported",
        message: UPGRADE_MESSAGE,
    };
    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json("upgrade", &report),
        OutputFormat::Text => emit_text(&format!(
            "cortex {} is installed.\nself-update: unsupported\n{}\n",
            report.current_version, report.message
        )),
    }
}

// ---------------------------------------------------------------------------
// uninstall
// ---------------------------------------------------------------------------

/// The kind of filesystem entry a target refers to, so removal picks the right operation.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum TargetKind {
    Dir,
    File,
}

struct Target {
    label: &'static str,
    path: PathBuf,
    kind: TargetKind,
}

#[derive(Serialize)]
struct TargetReport {
    label: &'static str,
    path: String,
    kind: TargetKind,
    existed: bool,
    removed: bool,
}

#[derive(Serialize)]
struct UninstallReport {
    #[serde(rename = "type")]
    kind: &'static str,
    dry_run: bool,
    targets: Vec<TargetReport>,
}

pub fn run_uninstall(args: UninstallArgs) -> Exit {
    let targets = cortex_targets();
    let dry_run = !args.yes;

    let mut reports = Vec::with_capacity(targets.len());
    let mut had_error = false;
    for target in &targets {
        let existed = target.path.exists();
        let mut removed = false;
        if existed && !dry_run {
            match remove_target(target) {
                Ok(()) => removed = true,
                Err(err) => {
                    had_error = true;
                    eprintln!(
                        "cortex uninstall: failed to remove {}: {err}",
                        target.path.display()
                    );
                }
            }
        }
        reports.push(TargetReport {
            label: target.label,
            path: target.path.display().to_string(),
            kind: target.kind,
            existed,
            removed,
        });
    }

    let report = UninstallReport {
        kind: "uninstall",
        dry_run,
        targets: reports,
    };

    let emit = match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json("uninstall", &report),
        OutputFormat::Text => emit_text(&render_uninstall_text(&report)),
    };
    if had_error { Exit::Failure } else { emit }
}

fn render_uninstall_text(report: &UninstallReport) -> String {
    let mut out = String::new();
    if report.dry_run {
        out.push_str("dry run (pass --yes to remove). Cortex-owned targets:\n");
    } else {
        out.push_str("removing Cortex-owned targets:\n");
    }
    let mut any = false;
    for target in &report.targets {
        if !target.existed {
            continue;
        }
        any = true;
        let action = if report.dry_run {
            "would remove"
        } else if target.removed {
            "removed"
        } else {
            "failed"
        };
        out.push_str(&format!("{action}: {} ({})\n", target.path, target.label));
    }
    if !any {
        out.push_str("nothing to remove (no Cortex artifacts found).\n");
    }
    out
}

/// Remove a single target: a directory tree for [`TargetKind::Dir`], a single file otherwise.
fn remove_target(target: &Target) -> io::Result<()> {
    match target.kind {
        TargetKind::Dir => std::fs::remove_dir_all(&target.path),
        TargetKind::File => std::fs::remove_file(&target.path),
    }
}

/// Resolve the Cortex-owned data and configuration paths under the documented precedence, scoped
/// so a removal can only ever touch Cortex's own artifacts:
///
/// - `CORTEX_DB_PATH`: only the explicit database file and its `-wal`/`-shm` siblings.
/// - `CORTEX_DATA_DIR`: only the known Cortex files inside it (the directory itself is left alone,
///   since a user may point it at a shared location).
/// - default: the `cortex` data directory under `${XDG_DATA_HOME:-$HOME/.local/share}`.
/// - config: the `cortex` directory under `${XDG_CONFIG_HOME:-$HOME/.config}`.
/// - `CORTEX_AUTH_FILE`: the explicit auth-store file, when set.
fn cortex_targets() -> Vec<Target> {
    let mut targets = Vec::new();
    let inputs = DbPathInputs::from_process();

    if let Some(db_path) = inputs.db_path.clone() {
        push_db_files(&mut targets, &db_path);
    } else if let Some(data_dir) = inputs.data_dir.clone() {
        for name in DATA_FILES {
            targets.push(Target {
                label: "data file",
                path: data_dir.join(name),
                kind: TargetKind::File,
            });
        }
    } else if let Some(db_path) = inputs.resolve() {
        match db_path.parent() {
            Some(parent) if parent.file_name().is_some_and(|name| name == "cortex") => {
                targets.push(Target {
                    label: "data directory",
                    path: parent.to_path_buf(),
                    kind: TargetKind::Dir,
                });
            }
            _ => push_db_files(&mut targets, &db_path),
        }
    }

    if let Some(auth_file) = non_empty_var("CORTEX_AUTH_FILE") {
        targets.push(Target {
            label: "auth store",
            path: auth_file,
            kind: TargetKind::File,
        });
    }

    if let Some(config_dir) = cortex_config_dir() {
        targets.push(Target {
            label: "config directory",
            path: config_dir,
            kind: TargetKind::Dir,
        });
    }

    targets
}

fn push_db_files(targets: &mut Vec<Target>, db_path: &Path) {
    let display = db_path.display().to_string();
    targets.push(Target {
        label: "database",
        path: db_path.to_path_buf(),
        kind: TargetKind::File,
    });
    targets.push(Target {
        label: "database WAL",
        path: PathBuf::from(format!("{display}-wal")),
        kind: TargetKind::File,
    });
    targets.push(Target {
        label: "database SHM",
        path: PathBuf::from(format!("{display}-shm")),
        kind: TargetKind::File,
    });
}

/// The Cortex config directory: `${XDG_CONFIG_HOME:-$HOME/.config}/cortex`, mirroring the
/// settings crate's global-config discovery.
fn cortex_config_dir() -> Option<PathBuf> {
    if let Some(xdg) = non_empty_var("XDG_CONFIG_HOME") {
        return Some(xdg.join("cortex"));
    }
    non_empty_var("HOME").map(|home| home.join(".config").join("cortex"))
}

fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// shared output helpers
// ---------------------------------------------------------------------------

fn emit_json<T: Serialize>(command: &str, value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex {command}: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_files_target_includes_wal_and_shm_siblings() {
        let mut targets = Vec::new();
        push_db_files(&mut targets, Path::new("/scratch/custom.db"));
        let paths: Vec<String> = targets
            .iter()
            .map(|t| t.path.display().to_string())
            .collect();
        assert!(paths.contains(&"/scratch/custom.db".to_owned()));
        assert!(paths.contains(&"/scratch/custom.db-wal".to_owned()));
        assert!(paths.contains(&"/scratch/custom.db-shm".to_owned()));
    }

    #[test]
    fn uninstall_text_marks_dry_run_targets_as_would_remove() {
        let report = UninstallReport {
            kind: "uninstall",
            dry_run: true,
            targets: vec![TargetReport {
                label: "data directory",
                path: "/home/u/.local/share/cortex".to_owned(),
                kind: TargetKind::Dir,
                existed: true,
                removed: false,
            }],
        };
        let text = render_uninstall_text(&report);
        assert!(text.contains("would remove: /home/u/.local/share/cortex"));
        assert!(!text.contains("removed:"));
    }

    #[test]
    fn upgrade_report_is_unsupported_and_changes_nothing() {
        let report = UpgradeReport {
            kind: "upgrade",
            current_version: env!("CARGO_PKG_VERSION"),
            supported: false,
            status: "unsupported",
            message: UPGRADE_MESSAGE,
        };
        assert!(!report.supported);
        assert_eq!(report.status, "unsupported");
    }
}
