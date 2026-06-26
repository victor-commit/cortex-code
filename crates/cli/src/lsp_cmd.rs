//! `cortex lsp` command surface.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use errors::Exit;
use serde::Serialize;

use crate::cli::{LspArgs, LspCommand, LspDiagnosticsArgs, OutputFormat};

/// Dispatch `cortex lsp` subcommands.
pub fn run(args: LspArgs) -> Exit {
    match args.command {
        LspCommand::Diagnostics(args) => diagnostics(args),
        LspCommand::Status(args) => status(args.format.format),
    }
}

#[derive(Serialize)]
struct DiagnosticsReport {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    program: Option<String>,
    status: String,
    diagnostics: Vec<DiagnosticRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Serialize)]
struct DiagnosticRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    severity: String,
    line: u32,
    character: u32,
    end_line: u32,
    end_character: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    message: String,
}

#[derive(Serialize)]
struct StatusRow {
    language: String,
    program: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    install: Option<String>,
}

fn diagnostics(args: LspDiagnosticsArgs) -> Exit {
    let format = args.format.format;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let input_path = PathBuf::from(&args.file);
    let absolute = absolute_path(&cwd, &input_path);
    if !absolute.is_file() {
        eprintln!(
            "cortex lsp diagnostics: {} is not a readable file",
            input_path.display()
        );
        return Exit::Failure;
    }

    let config = match crate::lsp_config::load_from_process() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("cortex lsp diagnostics: {err}");
            return Exit::Config;
        }
    };
    let server = config.resolve_for_path(&absolute).cloned();
    let mut manager = lsp::FeedbackManager::new(&cwd, config);
    let report = if args.workspace {
        workspace_diagnostics_report(
            &args.file,
            server.as_ref(),
            manager.collect_workspace(&input_path),
        )
    } else {
        diagnostics_report(&args.file, server.as_ref(), manager.collect(&input_path))
    };
    emit_diagnostics_report(format, &report)
}

fn diagnostics_report(
    shown_path: &str,
    server: Option<&lsp::LspServerConfig>,
    outcome: lsp::FeedbackOutcome,
) -> DiagnosticsReport {
    diagnostics_report_with_no_net(shown_path, server, outcome, redaction::network_disabled())
}

fn diagnostics_report_with_no_net(
    shown_path: &str,
    server: Option<&lsp::LspServerConfig>,
    outcome: lsp::FeedbackOutcome,
    no_net: bool,
) -> DiagnosticsReport {
    let (status, diagnostics, note) = match outcome {
        lsp::FeedbackOutcome::Diagnostics(diagnostics) => {
            let status = if diagnostics.is_empty() {
                "clean"
            } else {
                "diagnostics"
            };
            (
                status.to_owned(),
                diagnostics
                    .into_iter()
                    .map(|diagnostic| to_diagnostic_row(None, diagnostic))
                    .collect(),
                None,
            )
        }
        lsp::FeedbackOutcome::NoServer => (
            "no_server".to_owned(),
            Vec::new(),
            Some(no_server_note(shown_path, server, no_net)),
        ),
        lsp::FeedbackOutcome::Unavailable => (
            "unavailable".to_owned(),
            Vec::new(),
            Some(format!(
                "LSP diagnostics unavailable for {shown_path}: language server stopped responding or failed to start"
            )),
        ),
    };

    DiagnosticsReport {
        path: shown_path.to_owned(),
        language: server.map(|server| server.language_id().to_owned()),
        program: server.map(|server| server.program().to_owned()),
        status,
        diagnostics,
        note,
    }
}

fn workspace_diagnostics_report(
    shown_path: &str,
    server: Option<&lsp::LspServerConfig>,
    outcome: lsp::WorkspaceDiagnosticsOutcome,
) -> DiagnosticsReport {
    let (status, diagnostics, note) = match outcome {
        lsp::WorkspaceDiagnosticsOutcome::Diagnostics(files) => {
            let diagnostics = files
                .into_iter()
                .flat_map(|file| {
                    file.diagnostics.into_iter().map(move |diagnostic| {
                        to_diagnostic_row(Some(file.path.clone()), diagnostic)
                    })
                })
                .collect::<Vec<_>>();
            let status = if diagnostics.is_empty() {
                "clean"
            } else {
                "diagnostics"
            };
            (status.to_owned(), diagnostics, None)
        }
        lsp::WorkspaceDiagnosticsOutcome::NoServer => (
            "no_server".to_owned(),
            Vec::new(),
            Some(no_server_note(
                shown_path,
                server,
                redaction::network_disabled(),
            )),
        ),
        lsp::WorkspaceDiagnosticsOutcome::Unsupported => (
            "unsupported".to_owned(),
            Vec::new(),
            Some(format!(
                "LSP workspace diagnostics unavailable for {shown_path}: this language server does not advertise workspace pull diagnostics"
            )),
        ),
        lsp::WorkspaceDiagnosticsOutcome::Unavailable => (
            "unavailable".to_owned(),
            Vec::new(),
            Some(format!(
                "LSP workspace diagnostics unavailable for {shown_path}: language server stopped responding or failed to start"
            )),
        ),
    };

    DiagnosticsReport {
        path: shown_path.to_owned(),
        language: server.map(|server| server.language_id().to_owned()),
        program: server.map(|server| server.program().to_owned()),
        status,
        diagnostics,
        note,
    }
}

fn to_diagnostic_row(path: Option<String>, diagnostic: lsp::FileDiagnostic) -> DiagnosticRow {
    DiagnosticRow {
        path,
        severity: diagnostic.severity.to_owned(),
        line: diagnostic.line,
        character: diagnostic.character,
        end_line: diagnostic.end_line,
        end_character: diagnostic.end_character,
        source: diagnostic.source,
        code: diagnostic.code,
        message: diagnostic.message,
    }
}

fn no_server_note(shown_path: &str, server: Option<&lsp::LspServerConfig>, no_net: bool) -> String {
    match server {
        None => format!(
            "LSP diagnostics unavailable for {shown_path}: no language server is configured for this file extension"
        ),
        Some(server) if no_net => format!(
            "LSP diagnostics unavailable for {shown_path}: CORTEX_NO_NET=1 prevents auto-installing a missing language server ({program}); no diagnostics were claimed",
            program = server.program()
        ),
        Some(server) => format!(
            "LSP diagnostics unavailable for {shown_path}: no language server available or auto-install could not provide {program}",
            program = server.program()
        ),
    }
}

fn emit_diagnostics_report(format: OutputFormat, report: &DiagnosticsReport) -> Exit {
    match format {
        OutputFormat::Json | OutputFormat::Jsonl => match serde_json::to_string(report) {
            Ok(mut body) => {
                body.push('\n');
                let _ = io::stdout().write_all(body.as_bytes());
                Exit::Success
            }
            Err(err) => {
                eprintln!("cortex lsp diagnostics: failed to serialize output: {err}");
                Exit::Failure
            }
        },
        OutputFormat::Text => {
            println!(
                "lsp diagnostics for {}\tstatus={}",
                report.path, report.status
            );
            if let Some(language) = &report.language {
                println!("language={language}");
            }
            if let Some(program) = &report.program {
                println!("program={program}");
            }
            if report.diagnostics.is_empty() {
                println!("diagnostics=0");
            } else {
                println!("Diagnostics:");
                for diagnostic in &report.diagnostics {
                    println!("{}", render_diagnostic_row(diagnostic));
                }
            }
            if let Some(note) = &report.note {
                println!("{note}");
            }
            Exit::Success
        }
    }
}

fn render_diagnostic_row(diagnostic: &DiagnosticRow) -> String {
    let code = diagnostic
        .code
        .as_deref()
        .map(|code| format!(" [{code}]"))
        .unwrap_or_default();
    let source = diagnostic
        .source
        .as_deref()
        .map(|source| format!(" ({source})"))
        .unwrap_or_default();
    let location_prefix = diagnostic
        .path
        .as_deref()
        .map(|path| format!("{path}: "))
        .unwrap_or_default();
    if diagnostic.line == diagnostic.end_line {
        format!(
            "{location_prefix}{}{code}{source} at line {}, character {}-{}: {}",
            diagnostic.severity,
            diagnostic.line,
            diagnostic.character,
            diagnostic.end_character,
            diagnostic.message
        )
    } else {
        format!(
            "{location_prefix}{}{code}{source} at line {}, character {} to line {}, character {}: {}",
            diagnostic.severity,
            diagnostic.line,
            diagnostic.character,
            diagnostic.end_line,
            diagnostic.end_character,
            diagnostic.message
        )
    }
}

fn absolute_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn status(format: OutputFormat) -> Exit {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let config = match crate::lsp_config::load_from_process() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("cortex lsp status: {err}");
            return Exit::Config;
        }
    };
    let rows = crate::lsp_bridge::current_status(cwd, config)
        .into_iter()
        .map(|status| {
            let (state, pid) = match status.state {
                lsp::ServerRunState::Running(pid) => ("running".to_owned(), Some(pid)),
                other => (other.label().to_owned(), None),
            };
            StatusRow {
                language: status.language_id,
                program: status.program,
                state,
                pid,
                install: status.install_hint,
            }
        })
        .collect::<Vec<_>>();

    match format {
        OutputFormat::Json | OutputFormat::Jsonl => match serde_json::to_string(&rows) {
            Ok(mut body) => {
                body.push('\n');
                let _ = io::stdout().write_all(body.as_bytes());
                Exit::Success
            }
            Err(err) => {
                eprintln!("cortex lsp status: failed to serialize output: {err}");
                Exit::Failure
            }
        },
        OutputFormat::Text => {
            for row in rows {
                if let Some(pid) = row.pid {
                    println!(
                        "{}\t{}\t{}\tpid={pid}",
                        row.language, row.program, row.state
                    );
                } else {
                    println!("{}\t{}\t{}", row.language, row.program, row.state);
                }
                if let Some(install) = row.install {
                    println!("  install: {install}");
                }
            }
            Exit::Success
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts_server() -> lsp::LspServerConfig {
        lsp::LspServerConfig::new("typescript", "typescript-language-server")
            .with_extension("ts")
            .with_install(lsp::InstallPlan::Npm {
                package: "typescript-language-server".to_owned(),
            })
    }

    #[test]
    fn no_server_report_mentions_no_net_when_switch_is_enabled() {
        let report = diagnostics_report_with_no_net(
            "bad.ts",
            Some(&ts_server()),
            lsp::FeedbackOutcome::NoServer,
            true,
        );

        assert_eq!(report.status, "no_server");
        let note = report.note.expect("no-server note");
        assert!(note.contains("CORTEX_NO_NET=1"), "{note}");
        assert!(note.contains("prevents auto-installing"), "{note}");
        assert!(note.contains("typescript-language-server"), "{note}");
    }

    #[test]
    fn diagnostics_report_marks_empty_diagnostics_as_clean() {
        let report = diagnostics_report(
            "ok.py",
            Some(&lsp::LspServerConfig::new("python", "ruff")),
            lsp::FeedbackOutcome::Diagnostics(Vec::new()),
        );

        assert_eq!(report.status, "clean");
        assert!(report.diagnostics.is_empty());
        assert!(report.note.is_none());
    }
}
