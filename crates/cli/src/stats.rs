//! The `cortex stats` subcommand: a local read of persisted token usage and cost.
//!
//! The command opens the same SQLite database as `cortex run`, `cortex session`, and `cortex serve`
//! and derives totals from the recorded per-session usage rows. It never calls the model or opens
//! a network socket, so it is safe to run as a post-concurrency integrity probe.

use std::io::{self, Write};

use db::{Database, DbError, DbPathInputs, Session, UsageTotals};
use errors::Exit;
use serde::Serialize;

use crate::cli::{OutputFormat, StatsArgs};

/// Dispatch a `cortex stats` invocation.
pub fn run(args: StatsArgs) -> Exit {
    let db = match open_database() {
        Ok(db) => db,
        Err(exit) => return exit,
    };
    let report = match build_report(&db) {
        Ok(report) => report,
        Err(err) => return db_error(&err),
    };
    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&report),
        OutputFormat::Text => emit_text(&render_stats(&report)),
    }
}

fn open_database() -> Result<Database, Exit> {
    let inputs = DbPathInputs::from_process();
    let Some(path) = inputs.resolve() else {
        eprintln!(
            "cortex stats: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Err(Exit::Failure);
    };
    Database::open(&path).map_err(|err| db_error(&err))
}

fn build_report(db: &Database) -> Result<StatsReport, DbError> {
    let sessions = db.sessions().list()?;
    let mut breakdown = Vec::with_capacity(sessions.len());
    let mut totals = UsageTotals {
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cost: None,
    };
    for session in sessions {
        let usage = db.usage().session_totals(&session.id)?;
        totals.input_tokens += usage.input_tokens;
        totals.output_tokens += usage.output_tokens;
        totals.total_tokens += usage.total_tokens;
        if let Some(cost) = usage.cost {
            totals.cost = Some(totals.cost.unwrap_or(0.0) + cost);
        }
        breakdown.push(SessionStats { session, usage });
    }
    Ok(StatsReport {
        totals,
        sessions: breakdown,
    })
}

#[derive(Debug, Serialize)]
struct StatsReport {
    totals: UsageTotals,
    sessions: Vec<SessionStats>,
}

#[derive(Debug, Serialize)]
struct SessionStats {
    session: Session,
    usage: UsageTotals,
}

fn render_stats(report: &StatsReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "input_tokens\t{}\noutput_tokens\t{}\ntotal_tokens\t{}\n",
        report.totals.input_tokens, report.totals.output_tokens, report.totals.total_tokens
    ));
    if let Some(cost) = report.totals.cost {
        out.push_str(&format!("cost\t{cost:.8}\n"));
    } else {
        out.push_str("cost\t0\n");
    }
    for item in &report.sessions {
        if item.usage.total_tokens == 0 {
            continue;
        }
        out.push_str(&format!(
            "session\t{}\t{}\t{}\n",
            item.session.id, item.usage.total_tokens, item.usage.output_tokens
        ));
    }
    out
}

fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex stats: failed to serialize output: {err}");
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

fn db_error(err: &DbError) -> Exit {
    eprintln!("cortex stats: {err}");
    Exit::Failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> Session {
        Session {
            id: id.to_owned(),
            title: None,
            created_at: "2026-01-01 00:00:00".to_owned(),
            updated_at: "2026-01-01 00:00:00".to_owned(),
            model: None,
        }
    }

    #[test]
    fn text_stats_include_totals_and_nonzero_sessions() {
        let report = StatsReport {
            totals: UsageTotals {
                input_tokens: 5,
                output_tokens: 7,
                total_tokens: 12,
                cost: Some(0.25),
            },
            sessions: vec![SessionStats {
                session: session("ses_a"),
                usage: UsageTotals {
                    input_tokens: 5,
                    output_tokens: 7,
                    total_tokens: 12,
                    cost: Some(0.25),
                },
            }],
        };

        let rendered = render_stats(&report);
        assert!(rendered.contains("input_tokens\t5"), "{rendered}");
        assert!(rendered.contains("cost\t0.25000000"), "{rendered}");
        assert!(rendered.contains("session\tses_a\t12\t7"), "{rendered}");
    }
}
