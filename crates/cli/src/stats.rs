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
use crate::style::{self, Styler};

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
        // On a TTY the human branch prints an aligned, brand-styled table; off a TTY the raw TSV
        // is emitted byte-identical to the machine baseline (VAL-CLI-017..019).
        OutputFormat::Text => {
            let styler = Styler::for_stdout();
            if styler.is_enabled() {
                emit_text(&render_stats_styled(&report, styler))
            } else {
                emit_text(&render_stats(&report))
            }
        }
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

/// A single breakdown row of the styled TTY table.
struct StatsRow {
    kind: &'static str,
    kind_color: style::Rgb,
    id: String,
    usage: StatsTotals,
}

/// Render the human/TTY view of a `cortex stats` report: a totals block (with the `cost` value in
/// AMBER) followed by an aligned per-entry table (FAINT header + rule; `model`/`session` labels in
/// ORANGE/ORANGE_D, ids in TXT, and in/out/total in TXT with cost in AMBER). No numeric data is
/// ever green (brand purity). This path runs only when the styler is enabled (a real TTY); the raw
/// TSV [`render_stats`] carries the machine bytes.
fn render_stats_styled(report: &StatsReport, styler: Styler) -> String {
    let mut out = String::new();

    // --- Totals block: `label   value`, value TXT except the AMBER cost. ---
    let totals = &report.totals;
    let totals_label_w = "output_tokens".len() + 2;
    let totals_rows: [(&str, String, bool); 4] = [
        ("input_tokens", totals.input_tokens.to_string(), false),
        ("output_tokens", totals.output_tokens.to_string(), false),
        ("total_tokens", totals.total_tokens.to_string(), false),
        ("cost", format!("{:.6}", totals.cost), true),
    ];
    for (label, value, is_cost) in totals_rows {
        out.push_str(&styler.paint(&format!("{label:<totals_label_w$}"), style::DIM));
        let color = if is_cost { style::AMBER } else { style::TXT };
        out.push_str(&styler.paint(&value, color));
        out.push('\n');
    }

    // --- Breakdown rows: models first, then sessions. ---
    let mut rows: Vec<StatsRow> = Vec::new();
    for item in &report.models {
        rows.push(StatsRow {
            kind: "model",
            kind_color: style::ORANGE,
            id: item.model.clone(),
            usage: item.usage,
        });
    }
    for item in &report.sessions {
        rows.push(StatsRow {
            kind: "session",
            kind_color: style::ORANGE_D,
            id: item.session_id.clone(),
            usage: item.usage,
        });
    }
    if rows.is_empty() {
        return out;
    }

    // Column strings + widths (numeric columns right-aligned; header names included in the width).
    let in_strs: Vec<String> = rows.iter().map(|r| r.usage.input_tokens.to_string()).collect();
    let out_strs: Vec<String> = rows.iter().map(|r| r.usage.output_tokens.to_string()).collect();
    let total_strs: Vec<String> = rows.iter().map(|r| r.usage.total_tokens.to_string()).collect();
    let cost_strs: Vec<String> = rows.iter().map(|r| format!("{:.6}", r.usage.cost)).collect();

    let kind_w = rows.iter().map(|r| r.kind.len()).max().unwrap_or(0);
    let id_w = rows.iter().map(|r| r.id.chars().count()).max().unwrap_or(0);
    let base_label_w = kind_w + 2 + id_w;
    let header_label = "model / session";
    let label_area_w = base_label_w.max(header_label.len());
    let in_w = col_width("in", &in_strs);
    let out_w = col_width("out", &out_strs);
    let total_w = col_width("total", &total_strs);
    let cost_w = col_width("cost", &cost_strs);

    // Blank separator, then the FAINT header row and a GHOST rule.
    out.push('\n');
    let header = format!(
        "{header_label:<label_area_w$}  {in:>in_w$}  {out:>out_w$}  {total:>total_w$}  {cost:>cost_w$}",
        in = "in",
        out = "out",
        total = "total",
        cost = "cost",
    );
    out.push_str(&styler.paint(&header, style::FAINT));
    out.push('\n');
    let table_w = label_area_w + 2 + in_w + 2 + out_w + 2 + total_w + 2 + cost_w;
    out.push_str(&styler.paint(&style::BOX_HORIZONTAL.repeat(table_w), style::GHOST));
    out.push('\n');

    // Data rows: kind (ORANGE/ORANGE_D) + id (TXT) + aligned in/out/total (TXT) + cost (AMBER).
    for (index, row) in rows.iter().enumerate() {
        out.push_str(&styler.paint(&format!("{:<kind_w$}", row.kind), row.kind_color));
        out.push_str("  ");
        out.push_str(&styler.paint(&format!("{:<id_w$}", row.id), style::TXT));
        let used = kind_w + 2 + id_w;
        if label_area_w > used {
            out.push_str(&" ".repeat(label_area_w - used));
        }
        out.push_str("  ");
        out.push_str(&styler.paint(&format!("{:>in_w$}", in_strs[index]), style::TXT));
        out.push_str("  ");
        out.push_str(&styler.paint(&format!("{:>out_w$}", out_strs[index]), style::TXT));
        out.push_str("  ");
        out.push_str(&styler.paint(&format!("{:>total_w$}", total_strs[index]), style::TXT));
        out.push_str("  ");
        out.push_str(&styler.paint(&format!("{:>cost_w$}", cost_strs[index]), style::AMBER));
        out.push('\n');
    }

    out
}

/// The visible width of a numeric column: the wider of its header name and its longest value.
fn col_width(header: &str, values: &[String]) -> usize {
    values
        .iter()
        .map(String::len)
        .max()
        .unwrap_or(0)
        .max(header.len())
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

    fn sample_report() -> StatsReport {
        StatsReport {
            totals: StatsTotals {
                input_tokens: 20418,
                output_tokens: 3175,
                total_tokens: 23593,
                cost: 0.2147,
            },
            models: vec![
                ModelStats {
                    model: "claude-opus-4.8".to_owned(),
                    usage: StatsTotals {
                        input_tokens: 14203,
                        output_tokens: 2104,
                        total_tokens: 16307,
                        cost: 0.1812,
                    },
                },
                ModelStats {
                    model: "claude-haiku-4.5".to_owned(),
                    usage: StatsTotals {
                        input_tokens: 6215,
                        output_tokens: 1071,
                        total_tokens: 7286,
                        cost: 0.0335,
                    },
                },
            ],
            sessions: vec![SessionStats {
                session_id: "ses_7f3a91".to_owned(),
                model: "claude-opus-4.8".to_owned(),
                usage: StatsTotals {
                    input_tokens: 9120,
                    output_tokens: 1442,
                    total_tokens: 10562,
                    cost: 0.1184,
                },
            }],
        }
    }

    fn fg(color: style::Rgb) -> String {
        format!("\x1b[38;2;{};{};{}m", color.0, color.1, color.2)
    }

    fn strip_ansi(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for e in chars.by_ref() {
                    if e == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    // VAL-CLI-017: the TTY view carries the totals block, a `model / session ... cost` header +
    // rule, and per-entry rows whose numeric columns are aligned (equal visible line width).
    #[test]
    fn styled_stats_has_header_rule_and_aligned_columns() {
        let styled = render_stats_styled(&sample_report(), Styler::new(true));
        let plain = strip_ansi(&styled);
        let lines: Vec<&str> = plain.lines().collect();

        assert!(plain.contains("input_tokens"), "totals block present: {plain}");
        let header = lines
            .iter()
            .find(|line| line.contains("model / session"))
            .expect("header row present");
        assert!(header.contains("in") && header.contains("out"));
        assert!(header.contains("total") && header.contains("cost"));

        // The header, rule, and every data row share one visible width (columns line up).
        let header_w = header.chars().count();
        let rule = lines
            .iter()
            .find(|line| !line.is_empty() && line.chars().all(|c| c == '─'))
            .expect("rule row present");
        assert_eq!(rule.chars().count(), header_w, "rule spans the table width");
        for row in lines
            .iter()
            .filter(|line| line.starts_with("model") || line.starts_with("session"))
        {
            assert_eq!(
                row.chars().count(),
                header_w,
                "data row aligns to the table width: {row:?}"
            );
        }
    }

    // VAL-CLI-018: no numeric data is green; only `cost` values are AMBER, in/out/total are TXT.
    #[test]
    fn styled_stats_costs_are_amber_and_no_number_is_green() {
        let styled = render_stats_styled(&sample_report(), Styler::new(true));
        assert!(
            !styled.contains(&fg(style::OK_GREEN)),
            "no numeric data may be rendered green"
        );
        // The totals cost value is AMBER.
        assert!(
            styled.contains(&format!("{}0.214700\x1b[0m", fg(style::AMBER))),
            "totals cost is AMBER: {styled:?}"
        );
        // A per-row token count is TXT (not amber/green).
        assert!(styled.contains(&fg(style::TXT)), "token values use TXT");
        // The `model` label is ORANGE and `session` is ORANGE_D.
        assert!(styled.contains(&fg(style::ORANGE)), "model label ORANGE");
        assert!(styled.contains(&fg(style::ORANGE_D)), "session label ORANGE_D");
    }

    // VAL-CLI-019 (styling side): the styled TTY output differs from the raw TSV, but the raw TSV
    // renderer used for the piped/machine path is untouched and carries zero ANSI.
    #[test]
    fn piped_tsv_renderer_is_unstyled_and_ansi_free() {
        let report = sample_report();
        let tsv = render_stats(&report);
        assert!(!tsv.contains('\x1b'), "TSV path must never emit ANSI");
        assert!(tsv.contains("input_tokens\t20418"), "TSV totals unchanged");
        assert!(
            tsv.contains("model\tclaude-opus-4.8\t14203\t2104\t16307\t0.18120000"),
            "TSV model row keeps the 8-decimal machine format: {tsv}"
        );
    }
}
