//! The thin shell-out to the `rg` (ripgrep) binary that backs the non-mutating `glob` and `grep`
//! tools.
//!
//! Both search tools delegate to the real ripgrep program (architecture §3.4; ripgrep is a build
//! precondition) rather than re-implementing globbing/regex search, so their results match an
//! independent `rg` cross-check exactly. The single subtlety this module centralizes is ripgrep's
//! exit-code contract: `0` = matches found, `1` = a clean "no matches" (NOT an error), `2` = a real
//! error. A clean no-match therefore returns an empty string, so the tools can surface an explicit
//! empty result rather than an error or a hang.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// A failure to run ripgrep, distinct from a clean no-match (which is `Ok("")`).
pub(crate) enum RgError {
    /// The `rg` binary is not installed / not on `PATH`.
    NotInstalled,
    /// ripgrep ran but reported a real error (exit code 2), or could not be spawned.
    Failed(String),
}

/// Run `rg` with `args` in `root`, returning its stdout.
///
/// Returns `Ok(stdout)` for a successful search (exit 0) and `Ok(String::new())` for a clean
/// no-match (exit 1), so callers distinguish "found nothing" from "errored". A missing search root,
/// a real ripgrep error (exit 2), or a missing binary yields [`RgError`].
pub(crate) fn run<I, S>(root: &Path, args: I) -> Result<String, RgError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if !root.exists() {
        return Err(RgError::Failed(format!(
            "search root does not exist: {}",
            root.display()
        )));
    }

    let output = Command::new("rg")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                RgError::NotInstalled
            } else {
                RgError::Failed(err.to_string())
            }
        })?;

    match output.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        // ripgrep exits 1 when it ran fine but matched nothing: a clean empty result, not an error.
        Some(1) => Ok(String::new()),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            Err(RgError::Failed(if detail.is_empty() {
                "ripgrep failed".to_owned()
            } else {
                detail.to_owned()
            }))
        }
    }
}
