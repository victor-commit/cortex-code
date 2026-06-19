use std::collections::BTreeMap;
use std::io::{self, Write};

use errors::Exit;
use settings::{ConfigInputs, load};

use crate::cli::{ConfigArgs, OutputFormat};

/// Execute `cortex config`: load every configuration layer and report the effective result.
///
/// The merge precedence is fixed (defaults < global < project < nested < env < CLI), so the
/// reported values reflect the highest layer present for each key. `--format json` (or `jsonl`,
/// which is a single object on one line) emits a strict, machine-parseable document on stdout;
/// the default text format emits a TOML dump. Diagnostics go to stderr only.
pub fn run(args: ConfigArgs) -> Exit {
    let mut overrides = BTreeMap::new();
    if let Some(model) = args.model {
        overrides.insert("model".to_owned(), model);
    }

    let inputs = match ConfigInputs::from_process(overrides) {
        Ok(inputs) => inputs,
        Err(err) => {
            eprintln!("cortex config: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };

    let effective = match load(inputs) {
        Ok(effective) => effective,
        Err(err) => {
            eprintln!("cortex config: {err}");
            return Exit::Config;
        }
    };

    let mut body = match args.format {
        OutputFormat::Text => effective.to_toml(),
        OutputFormat::Json | OutputFormat::Jsonl => effective.to_json(),
    };
    if !body.ends_with('\n') {
        body.push('\n');
    }

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
