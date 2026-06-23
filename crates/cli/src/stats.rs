//! The `cortex stats` subcommand: a local read of persisted token usage and cost.
//!
//! The command opens the same SQLite database as `cortex run`, `cortex session`, and `cortex serve`
//! and derives totals from the recorded per-session usage rows. It never calls the model or opens
//! a network socket, so it is safe to run as a post-concurrency integrity probe.

use std::io::{self, Write};

use std::collections::BTreeMap;

use db::{Database, DbError, DbPathInputs, UsageTotals};
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
    let mut report = StatsReport {
        totals: StatsTotals::default(),
        models: Vec::new(),
        sessions: Vec::new(),
    };
    let mut by_model: BTreeMap<String, StatsTotals> = BTreeMap::new();
    for session in sessions {
        let usage = StatsTotals::from_usage(db.usage().session_totals(&session.id)?);
        report.totals.add(usage);
        if usage.is_zero() {
            continue;
        }
        let model = session.model.unwrap_or_else(|| "unknown".to_owned());
        by_model.entry(model.clone()).or_default().add(usage);
        report.sessions.push(SessionStats {
            session_id: session.id,
            model,
            usage,
        });
    }
    report.models = by_model
        .into_iter()
        .map(|(model, usage)| ModelStats { model, usage })
        .collect();
    Ok(report)
}

#[derive(Debug, Serialize)]
struct StatsReport {
    totals: StatsTotals,
    models: Vec<ModelStats>,
    sessions: Vec<SessionStats>,
}

#[derive(Debug, Serialize)]
struct SessionStats {
    session_id: String,
    model: String,
    usage: StatsTotals,
}

#[derive(Debug, Serialize)]
struct ModelStats {
    model: String,
    usage: StatsTotals,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct StatsTotals {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cost: f64,
}

impl StatsTotals {
    fn from_usage(usage: UsageTotals) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cost: usage.cost.unwrap_or(0.0),
        }
    }

    fn add(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.total_tokens += other.total_tokens;
        self.cost += other.cost;
    }

    fn is_zero(self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.total_tokens == 0
            && self.cost == 0.0
    }
}

fn render_stats(report: &StatsReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "input_tokens\t{}\noutput_tokens\t{}\ntotal_tokens\t{}\n",
        report.totals.input_tokens, report.totals.output_tokens, report.totals.total_tokens
    ));
    out.push_str(&format!("cost\t{:.8}\n", report.totals.cost));
    for item in &report.models {
        out.push_str(&format!(
            "model\t{}\t{}\t{}\t{}\t{:.8}\n",
            item.model,
            item.usage.input_tokens,
            item.usage.output_tokens,
            item.usage.total_tokens,
            item.usage.cost
        ));
    }
    for item in &report.sessions {
        out.push_str(&format!(
            "session\t{}\t{}\t{}\t{}\t{}\t{:.8}\n",
            item.session_id,
            item.model,
            item.usage.input_tokens,
            item.usage.output_tokens,
            item.usage.total_tokens,
            item.usage.cost
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

    #[test]
    fn text_stats_include_totals_and_nonzero_sessions() {
        let report = StatsReport {
            totals: StatsTotals {
                input_tokens: 5,
                output_tokens: 7,
                total_tokens: 12,
                cost: 0.25,
            },
            models: vec![ModelStats {
                model: "google/gemini-2.5-flash".to_owned(),
                usage: StatsTotals {
                    input_tokens: 5,
                    output_tokens: 7,
                    total_tokens: 12,
                    cost: 0.25,
                },
            }],
            sessions: vec![SessionStats {
                session_id: "ses_a".to_owned(),
                model: "google/gemini-2.5-flash".to_owned(),
                usage: StatsTotals {
                    input_tokens: 5,
                    output_tokens: 7,
                    total_tokens: 12,
                    cost: 0.25,
                },
            }],
        };

        let rendered = render_stats(&report);
        assert!(rendered.contains("input_tokens\t5"), "{rendered}");
        assert!(rendered.contains("cost\t0.25000000"), "{rendered}");
        assert!(
            rendered.contains("model\tgoogle/gemini-2.5-flash\t5\t7\t12\t0.25000000"),
            "{rendered}"
        );
        assert!(
            rendered.contains("session\tses_a\tgoogle/gemini-2.5-flash\t5\t7\t12\t0.25000000"),
            "{rendered}"
        );
    }
}
