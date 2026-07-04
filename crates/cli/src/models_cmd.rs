//! `cortex models`: list built-in and plugin-registered models.
//!
//! The plain listing is fully offline: it reads the on-disk cache written by a prior `--refresh`
//! when present, otherwise the binary's embedded models.dev snapshot — never the network. With
//! `--refresh` it performs a real models.dev fetch behind the `CORTEX_NO_NET` outbound guard,
//! curates the result to the recognizable model set, caches it, and lists it; a fetch failure
//! degrades cleanly (clear diagnostic, non-zero exit) without corrupting the cached catalogue.

use errors::Exit;
use serde::Serialize;
use settings::ConfigInputs;
use std::collections::BTreeMap;
use std::io::{self, Write};

use crate::cli::{ModelsArgs, OutputFormat};
use crate::plugin_bridge::{ModelCostDoc, ModelDoc, PluginState, ProviderDoc};
use crate::style::{self, Styler};

pub fn run(args: ModelsArgs, pure_plugins: bool) -> Exit {
    if args.refresh && !crate::feature_gate::network_enabled() {
        return crate::feature_gate::network_unavailable("models --refresh");
    }

    let listing = match collect_providers(pure_plugins, args.refresh, "models") {
        Ok(listing) => listing,
        Err(exit) => return exit,
    };
    emit(
        args.format.format,
        &listing.providers,
        &listing.default_model,
        &listing.provider_report,
    )
}

/// The provider catalogue projected for `cortex models` / `cortex provider list`.
///
/// Carries the per-provider docs (after provider-config enable/disable and policy-deny filtering),
/// the resolved default model id, and the redacted provider-config report.
pub(crate) struct ProviderListing {
    pub(crate) providers: Vec<ProviderDoc>,
    pub(crate) default_model: String,
    pub(crate) provider_report: providers::ProviderConfigReport,
}

/// Build the provider listing shared by `cortex models` and `cortex provider list`.
///
/// `refresh` triggers a real models.dev fetch (the caller is responsible for the network feature
/// gate); when false the catalogue is read entirely offline. `command` is the subcommand label used
/// in diagnostics so errors read as `cortex <command>: ...`.
pub(crate) fn collect_providers(
    pure_plugins: bool,
    refresh: bool,
    command: &str,
) -> Result<ProviderListing, Exit> {
    let state = match PluginState::from_process_config(pure_plugins) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("cortex {command}: {err}");
            return Err(Exit::Config);
        }
    };

    let config_inputs = match ConfigInputs::from_process(BTreeMap::new()) {
        Ok(inputs) => inputs,
        Err(err) => {
            eprintln!("cortex {command}: cannot determine configuration context: {err}");
            return Err(Exit::Failure);
        }
    };
    let effective = match settings::load(config_inputs.clone()) {
        Ok(effective) => effective,
        Err(err) => {
            eprintln!("cortex {command}: {err}");
            return Err(Exit::Config);
        }
    };
    let provider_config = match providers::ProviderConfigSet::from_table(effective.table()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("cortex {command}: {err}");
            return Err(Exit::Config);
        }
    };
    let provider_report = provider_config.redacted_report();

    let (builtin_catalogue, builtin_source) = if refresh {
        refresh_builtin_catalogue()?
    } else {
        offline_builtin_catalogue()
    };
    let merged_catalogue = provider_config.merged_catalogue(&builtin_catalogue);
    let default_model =
        match provider_config.resolve_model(&merged_catalogue, Some(effective.model())) {
            Ok(resolved) => resolved.model_id,
            Err(err) => {
                eprintln!("cortex {command}: {err}");
                return Err(Exit::Config);
            }
        };

    let mut providers = docs_from_catalogue(
        &merged_catalogue,
        &builtin_catalogue,
        builtin_source,
        &default_model,
        &provider_report.providers,
    );
    let mut plugin_providers = state.providers();
    for provider in &mut plugin_providers {
        provider.config = provider_report.providers.get(&provider.id).cloned();
    }
    providers.extend(plugin_providers);
    let providers = providers
        .into_iter()
        .filter(|provider| provider_config.provider_enabled(&provider.id))
        .collect::<Vec<_>>();
    let providers = match filter_denied_providers(&config_inputs, &providers) {
        Ok(providers) => providers,
        Err(err) => {
            eprintln!("cortex {command}: {err}");
            return Err(if err.is_config() {
                Exit::Config
            } else {
                Exit::Failure
            });
        }
    };

    Ok(ProviderListing {
        providers,
        default_model,
        provider_report,
    })
}

/// The offline builtin catalogue: the cache written by a prior `--refresh` when present and
/// valid, otherwise the embedded snapshot. Either source is read from disk with no network access.
fn offline_builtin_catalogue() -> (providers::Catalogue, &'static str) {
    match providers::load_cached_catalogue() {
        Some(cached) => (cached, "cached"),
        None => (providers::Catalogue::embedded().clone(), "builtin"),
    }
}

/// Refresh the builtin providers from models.dev, fail-closed under `CORTEX_NO_NET`.
///
/// The outbound guard is consulted FIRST, so under the kill-switch this returns the net-disabled
/// exit code with no socket opened. On a successful fetch the curated snapshot is cached (a write
/// failure is non-fatal) and listed; on a fetch failure it returns the mapped failure exit with a
/// clear diagnostic on stderr, leaving any existing cache (and the embedded snapshot) intact.
fn refresh_builtin_catalogue() -> Result<(providers::Catalogue, &'static str), Exit> {
    if let Err(diag) = redaction::guard_outbound("models.dev catalog refresh") {
        eprintln!("cortex models --refresh: {diag}");
        return Err(diag.exit());
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex models --refresh: failed to start the async runtime: {err}");
            return Err(Exit::Failure);
        }
    };

    let url = providers::models_dev_url();
    match runtime.block_on(providers::refresh_openrouter_catalogue(&url)) {
        Ok(catalogue) => {
            if let Err(err) = providers::write_models_cache(&catalogue) {
                // A cache write failure must not fail the refresh: the listing is already in hand.
                eprintln!(
                    "cortex models --refresh: warning: could not cache the refreshed catalog: {err}"
                );
            }
            Ok((catalogue, "models.dev"))
        }
        Err(diag) => {
            eprintln!("cortex models --refresh: {diag}");
            Err(diag.exit())
        }
    }
}

/// Project a catalogue into the `cortex models` document shape, tagging each entry's `source`.
fn docs_from_catalogue(
    catalogue: &providers::Catalogue,
    builtin_catalogue: &providers::Catalogue,
    builtin_source: &'static str,
    default_model: &str,
    provider_reports: &BTreeMap<String, providers::ProviderReport>,
) -> Vec<ProviderDoc> {
    catalogue
        .providers()
        .map(|provider| ProviderDoc {
            id: provider.id.clone(),
            name: provider.name.clone(),
            source: if builtin_catalogue.provider(&provider.id).is_some() {
                builtin_source
            } else {
                "config"
            },
            config: provider_reports.get(&provider.id).cloned(),
            models: provider
                .models
                .values()
                .map(|model| ModelDoc {
                    id: model.id.clone(),
                    name: model.name.clone(),
                    provider: provider.id.clone(),
                    default: model.id == default_model,
                    attachment: model.attachment,
                    tool_call: model.tool_call,
                    reasoning: model.reasoning,
                    cost: ModelCostDoc {
                        input: model.cost.input,
                        output: model.cost.output,
                    },
                    context_length: model.limit.context,
                    max_output_tokens: model.limit.output,
                    source: if builtin_catalogue.model(&provider.id, &model.id).is_some() {
                        builtin_source
                    } else {
                        "config"
                    },
                })
                .collect(),
        })
        .collect()
}

fn filter_denied_providers(
    inputs: &settings::ConfigInputs,
    providers: &[ProviderDoc],
) -> Result<Vec<ProviderDoc>, policy::ProviderPolicyError> {
    providers.iter().try_fold(Vec::new(), |mut acc, provider| {
        if policy::provider_use_action(inputs, &provider.id)? != permissions::Action::Deny {
            acc.push(provider.clone());
        }
        Ok(acc)
    })
}

#[derive(Serialize)]
struct ModelsDocument<'a> {
    default_model: &'a str,
    providers: &'a [ProviderDoc],
    provider_config: &'a providers::ProviderConfigReport,
}

fn emit(
    format: OutputFormat,
    providers: &[ProviderDoc],
    default_model: &str,
    provider_config: &providers::ProviderConfigReport,
) -> Exit {
    match format {
        OutputFormat::Text => {
            // On a TTY the human branch prints provider-grouped, brand-styled rows with aligned
            // ctx/in/out columns and a summary footer; off a TTY the historical `Name (id)` +
            // tab-separated rows are emitted byte-identical to the machine baseline
            // (VAL-CLI-024..028).
            let styler = Styler::for_stdout();
            if styler.is_enabled() {
                let body = render_models_styled(providers, default_model, styler);
                emit_text(&body)
            } else {
                emit_plain(providers)
            }
        }
        OutputFormat::Json | OutputFormat::Jsonl => {
            let document = ModelsDocument {
                default_model,
                providers,
                provider_config,
            };
            match serde_json::to_string(&document) {
                Ok(line) => {
                    println!("{line}");
                    Exit::Success
                }
                Err(err) => {
                    eprintln!("cortex models: failed to serialize models: {err}");
                    Exit::Failure
                }
            }
        }
    }
}

/// Emit the machine/piped listing: the historical `Name (id)` provider header followed by one
/// tab-separated `  <id>\t<name>[ [default]]\tctx=…\tin=$…/Mtok\tout=$…/Mtok` row per model. This
/// path carries the pinned baseline bytes and must never change.
fn emit_plain(providers: &[ProviderDoc]) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    for provider in providers {
        if let Err(err) = writeln!(handle, "{} ({})", provider.name, provider.id) {
            return map_stdout_error(err);
        }
        for model in &provider.models {
            let marker = if model.default { " [default]" } else { "" };
            if let Err(err) = writeln!(
                handle,
                "  {}\t{}{}\tctx={}\tin=${:.2}/Mtok\tout=${:.2}/Mtok",
                model.id,
                model.name,
                marker,
                model.context_length,
                model.cost.input,
                model.cost.output
            ) {
                return map_stdout_error(err);
            }
        }
    }
    if let Err(err) = handle.flush() {
        return map_stdout_error(err);
    }
    Exit::Success
}

/// The `[default]` inline marker text.
const DEFAULT_TAG: &str = "[default]";

/// Render the human/TTY view of `cortex models`: provider groups (name ORANGE + `(id)` FAINT,
/// separated by a blank line), each listing model rows with the id in AMBER, the display name in
/// TXT, an inline `[default]` tag in AMBER, and `ctx=`/`in=`/`out=` in fixed, aligned columns (the
/// `ctx=` value right-aligned so an unknown length shows a dash placeholder). A FAINT summary
/// footer closes the listing. No numeric data is ever green (brand purity). Runs only when the
/// styler is enabled (a real TTY); [`emit_plain`] carries the machine bytes.
fn render_models_styled(providers: &[ProviderDoc], default_model: &str, styler: Styler) -> String {
    // Global column widths so every provider group aligns to the same grid.
    let mut id_w = 0usize;
    let mut name_w = 0usize;
    let mut ctx_w = 0usize;
    let mut in_w = 0usize;
    let mut model_count = 0usize;
    for provider in providers {
        for model in &provider.models {
            model_count += 1;
            id_w = id_w.max(model.id.chars().count());
            let mut name_len = model.name.chars().count();
            if model.default {
                name_len += 1 + DEFAULT_TAG.chars().count();
            }
            name_w = name_w.max(name_len);
            ctx_w = ctx_w.max(ctx_value(model.context_length).chars().count());
            in_w = in_w.max(price_cell(model.cost.input).chars().count());
        }
    }

    let mut out = String::new();
    for (index, provider) in providers.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&styler.paint(&provider.name, style::ORANGE));
        out.push(' ');
        out.push_str(&styler.paint(&format!("({})", provider.id), style::FAINT));
        out.push('\n');

        for model in &provider.models {
            out.push_str("  ");
            out.push_str(&styler.paint(&pad(&model.id, id_w), style::AMBER));
            out.push_str("  ");

            // Name column (+ optional inline `[default]`), padded to the global name width.
            let name_visible = model.name.chars().count()
                + if model.default {
                    1 + DEFAULT_TAG.chars().count()
                } else {
                    0
                };
            out.push_str(&styler.paint(&model.name, style::TXT));
            if model.default {
                out.push(' ');
                out.push_str(&styler.paint(DEFAULT_TAG, style::AMBER));
            }
            if name_w > name_visible {
                out.push_str(&" ".repeat(name_w - name_visible));
            }
            out.push_str("  ");

            // ctx= column: `ctx=` label FAINT + right-aligned value TXT (dash for unknown).
            out.push_str(&styler.paint("ctx=", style::FAINT));
            out.push_str(&styler.paint(&rpad_left(&ctx_value(model.context_length), ctx_w), style::TXT));
            out.push_str("  ");

            // in= / out= price columns: label FAINT + value TXT, in= padded so out= aligns.
            out.push_str(&styler.paint("in=", style::FAINT));
            out.push_str(&styler.paint(&pad(&price_cell(model.cost.input), in_w), style::TXT));
            out.push_str("  ");
            out.push_str(&styler.paint("out=", style::FAINT));
            out.push_str(&styler.paint(&price_cell(model.cost.output), style::TXT));
            out.push('\n');
        }
    }

    // Summary footer.
    out.push('\n');
    let footer = format!(
        "{} providers {} {} models {} default {} (--refresh to sync models.dev)",
        providers.len(),
        style::SEPARATOR,
        model_count,
        style::SEPARATOR,
        default_model,
    );
    out.push_str(&styler.paint(&footer, style::FAINT));
    out.push('\n');
    out
}

/// The `ctx=` value string: the context length, or a dash placeholder when it is unknown (`0`).
fn ctx_value(context_length: u64) -> String {
    if context_length == 0 {
        style::EM_DASH.to_owned()
    } else {
        context_length.to_string()
    }
}

/// A `$x.xx/Mtok` price cell.
fn price_cell(price: f64) -> String {
    format!("${price:.2}/Mtok")
}

/// Left-justify `text` (pad trailing spaces) to a minimum visible width.
fn pad(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if width > len {
        format!("{text}{}", " ".repeat(width - len))
    } else {
        text.to_owned()
    }
}

/// Right-justify `text` (pad leading spaces) to a minimum visible width.
fn rpad_left(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if width > len {
        format!("{}{text}", " ".repeat(width - len))
    } else {
        text.to_owned()
    }
}

fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if let Err(err) = handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
    {
        return map_stdout_error(err);
    }
    Exit::Success
}

fn map_stdout_error(err: io::Error) -> Exit {
    if err.kind() == io::ErrorKind::BrokenPipe {
        return Exit::Success;
    }

    eprintln!("cortex models: failed to write output: {err}");
    Exit::Failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(
        id: &str,
        name: &str,
        default: bool,
        context_length: u64,
        input: f64,
        output: f64,
    ) -> ModelDoc {
        ModelDoc {
            id: id.to_owned(),
            name: name.to_owned(),
            provider: "p".to_owned(),
            default,
            attachment: false,
            tool_call: false,
            reasoning: false,
            cost: ModelCostDoc { input, output },
            context_length,
            max_output_tokens: 0,
            source: "builtin",
        }
    }

    fn sample_providers() -> Vec<ProviderDoc> {
        vec![
            ProviderDoc {
                id: "anthropic".to_owned(),
                name: "Anthropic".to_owned(),
                source: "builtin",
                config: None,
                models: vec![
                    model("claude-opus-4.8", "Claude Opus 4.8", true, 200_000, 15.0, 75.0),
                    model("claude-haiku-4.5", "Claude Haiku 4.5", false, 200_000, 1.0, 5.0),
                ],
            },
            ProviderDoc {
                id: "openrouter".to_owned(),
                name: "OpenRouter".to_owned(),
                source: "builtin",
                config: None,
                models: vec![model("auto", "Auto Router", false, 0, 0.0, 0.0)],
            },
        ]
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

    // VAL-CLI-024: provider-grouped rows (name ORANGE + `(id)` FAINT, blank line between groups)
    // with the id AMBER, name TXT, and fixed, aligned ctx/in/out columns across every row.
    #[test]
    fn styled_models_groups_and_aligns_columns() {
        let styled = render_models_styled(&sample_providers(), "claude-opus-4.8", Styler::new(true));
        let plain = strip_ansi(&styled);
        let lines: Vec<&str> = plain.lines().collect();

        assert_eq!(lines[0], "Anthropic (anthropic)", "provider header: {plain}");
        assert!(
            styled.contains(&format!("{}Anthropic\x1b[0m", fg(style::ORANGE))),
            "provider name is ORANGE"
        );
        assert!(
            styled.contains(&format!("{}(anthropic)\x1b[0m", fg(style::FAINT))),
            "provider id is FAINT"
        );

        // A blank line separates the two provider groups.
        assert!(
            plain.contains("\n\nOpenRouter (openrouter)"),
            "blank line between groups: {plain:?}"
        );

        // Every model row's `ctx=` column starts at the same visible column across all groups.
        // (Compare visible char columns, not byte offsets: the em-dash is multi-byte.)
        let col_of = |line: &str, needle: &str| -> usize {
            let byte = line.find(needle).unwrap();
            line[..byte].chars().count()
        };
        let ctx_cols: Vec<usize> = lines
            .iter()
            .filter(|l| l.contains("ctx="))
            .map(|l| col_of(l, "ctx="))
            .collect();
        assert!(ctx_cols.len() >= 3, "three model rows: {plain}");
        assert!(
            ctx_cols.iter().all(|c| *c == ctx_cols[0]),
            "ctx= column is aligned across rows: {plain:?}"
        );
        // in= and out= likewise align.
        for label in ["in=", "out="] {
            let cols: Vec<usize> = lines
                .iter()
                .filter(|l| l.contains(label))
                .map(|l| col_of(l, label))
                .collect();
            assert!(
                cols.iter().all(|c| *c == cols[0]),
                "{label} column is aligned: {plain:?}"
            );
        }

        // The id is AMBER and the display name is TXT.
        assert!(styled.contains(&format!("{}claude-opus-4.8", fg(style::AMBER))));
        assert!(styled.contains(&format!("{}Claude Opus 4.8", fg(style::TXT))));
    }

    // VAL-CLI-025: the default model carries an inline `[default]` (AMBER); an unknown context
    // length renders a dash placeholder aligned in the ctx column.
    #[test]
    fn styled_models_marks_default_and_dashes_unknown_ctx() {
        let styled = render_models_styled(&sample_providers(), "claude-opus-4.8", Styler::new(true));
        let plain = strip_ansi(&styled);

        assert!(plain.contains("Claude Opus 4.8 [default]"), "inline default tag: {plain}");
        assert!(
            styled.contains(&format!("{}[default]\x1b[0m", fg(style::AMBER))),
            "[default] tag is AMBER"
        );
        // The `auto` row shows a dash for unknown ctx, right-aligned in the ctx column.
        let auto_line = plain.lines().find(|l| l.contains("auto")).expect("auto row");
        assert!(auto_line.contains("ctx="), "auto row has a ctx column: {auto_line}");
        assert!(auto_line.contains('—'), "auto row shows a dash: {auto_line}");
        assert!(
            !auto_line.contains("ctx=0"),
            "unknown ctx never prints a literal 0: {auto_line}"
        );
    }

    // VAL-CLI-026: no numeric data is green; coloring is limited to ORANGE/AMBER/FAINT/TXT tokens.
    #[test]
    fn styled_models_uses_no_green() {
        let styled = render_models_styled(&sample_providers(), "claude-opus-4.8", Styler::new(true));
        assert!(
            !styled.contains(&fg(style::OK_GREEN)),
            "no numeric data may be rendered green"
        );
    }

    // VAL-CLI-027: a FAINT footer reports the real provider/model counts and default id.
    #[test]
    fn styled_models_prints_summary_footer() {
        let styled = render_models_styled(&sample_providers(), "claude-opus-4.8", Styler::new(true));
        let plain = strip_ansi(&styled);
        assert!(
            plain.contains("2 providers · 3 models · default claude-opus-4.8 (--refresh to sync models.dev)"),
            "footer names real counts + default: {plain}"
        );
        // The footer line is FAINT.
        assert!(
            styled.contains(&format!("{}2 providers", fg(style::FAINT))),
            "footer is FAINT"
        );
    }

    // VAL-CLI-028 (styling side): the machine/piped renderer keeps the exact historical bytes with
    // zero ANSI, and a disabled styler is never routed through the styled path.
    #[test]
    fn styled_models_disabled_is_ansi_free_passthrough() {
        let styled = render_models_styled(&sample_providers(), "claude-opus-4.8", Styler::new(false));
        assert!(!styled.contains('\x1b'), "disabled styler emits no ANSI: {styled:?}");
    }
}
