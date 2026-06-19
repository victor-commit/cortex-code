//! A thin wrapper around the `git` command line, addressed at the shadow git directory only.
//!
//! Every invocation passes `--git-dir <shadow>` explicitly and runs with the workspace root as the
//! process working directory, so the project's own `.git` is never read, written, or required. All
//! operations used by the store are object-level (`hash-object`, `cat-file`, `update-index`,
//! `write-tree`, `commit-tree`, `update-ref`), so no checkout of the work tree is ever performed.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::CheckpointError;

/// A handle on the shadow git directory used to run object-level git commands.
#[derive(Clone, Debug)]
pub(crate) struct Git {
    git_dir: PathBuf,
    work_dir: PathBuf,
}

impl Git {
    /// Bind git commands to `git_dir` (the shadow directory) running from `work_dir` (the
    /// workspace root).
    pub(crate) fn new(git_dir: impl Into<PathBuf>, work_dir: impl Into<PathBuf>) -> Self {
        Self {
            git_dir: git_dir.into(),
            work_dir: work_dir.into(),
        }
    }

    /// The shadow git directory these commands address.
    pub(crate) fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Run `git <args>` and return its captured stdout, mapping any failure to a
    /// [`CheckpointError`].
    pub(crate) fn run(&self, args: &[&str]) -> Result<Vec<u8>, CheckpointError> {
        self.run_with(args, &[], None)
    }

    /// Run `git <args>` feeding `stdin` to the process, returning captured stdout.
    pub(crate) fn run_stdin(
        &self,
        args: &[&str],
        stdin: &[u8],
    ) -> Result<Vec<u8>, CheckpointError> {
        self.run_with(args, &[], Some(stdin))
    }

    /// Run `git <args>` with the extra environment `envs` and optional `stdin`, returning captured
    /// stdout. A non-zero exit becomes a [`CheckpointError`] carrying git's stderr.
    pub(crate) fn run_with(
        &self,
        args: &[&str],
        envs: &[(&str, &str)],
        stdin: Option<&[u8]>,
    ) -> Result<Vec<u8>, CheckpointError> {
        let mut command = Command::new("git");
        command
            .arg("--git-dir")
            .arg(&self.git_dir)
            .args(args)
            .current_dir(&self.work_dir)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| CheckpointError::new(format!("git unavailable: {error}")))?;
        if let Some(bytes) = stdin {
            let mut pipe = child
                .stdin
                .take()
                .ok_or_else(|| CheckpointError::new("git stdin unavailable"))?;
            pipe.write_all(bytes)
                .map_err(|error| CheckpointError::new(format!("git stdin: {error}")))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| CheckpointError::new(format!("git wait: {error}")))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(CheckpointError::new(format!(
                "git {} failed: {}",
                args.first().copied().unwrap_or("?"),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }
}
