//! The dynamic environment block injected into the system-context epoch.
//!
//! Each turn's system context is grounded in a snapshot of the runtime environment (architecture
//! §8, item 2): the current working directory, the host OS/platform, the current date, and the
//! git-repository status. The model reads this block to answer questions like "what is the current
//! working directory?" or "what platform are you on?" correctly for the real environment the agent
//! is running in.
//!
//! Fields are public and the type is constructed directly in tests; [`EnvironmentBlock::from_process`]
//! gathers a live snapshot (reading the process cwd/OS and shelling out to `git`), keeping the
//! subprocess work out of the otherwise-pure rendering path.

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The git state of the working directory captured for the environment block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatus {
    /// The working directory is not inside a git repository.
    NotARepo,
    /// The working directory is inside a git repository.
    Repo {
        /// The current branch name, when resolvable (`None` for a detached HEAD).
        branch: Option<String>,
        /// Whether the working tree has uncommitted changes.
        dirty: bool,
    },
    /// Git status could not be determined (e.g. `git` is not installed).
    Unavailable,
}

impl GitStatus {
    /// A one-line human description used in the rendered block.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            GitStatus::NotARepo => "not a git repository".to_owned(),
            GitStatus::Unavailable => "unavailable".to_owned(),
            GitStatus::Repo { branch, dirty } => {
                let branch = branch.as_deref().unwrap_or("detached HEAD");
                let state = if *dirty {
                    "uncommitted changes"
                } else {
                    "clean working tree"
                };
                format!("branch {branch}, {state}")
            }
        }
    }
}

/// A snapshot of the runtime environment rendered into the system context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentBlock {
    /// The current working directory, as an absolute path string.
    pub cwd: String,
    /// A human-facing platform name (e.g. `Linux`, `macOS`).
    pub platform: String,
    /// The raw OS token from the build target (e.g. `linux`, `macos`).
    pub os: String,
    /// The CPU architecture token (e.g. `x86_64`, `aarch64`).
    pub arch: String,
    /// The current UTC date, formatted `YYYY-MM-DD`.
    pub date: String,
    /// The git status of the working directory.
    pub git: GitStatus,
}

impl EnvironmentBlock {
    /// Gather a live environment snapshot from the current process and working directory.
    ///
    /// The cwd and OS/arch come from the process; the date is the current UTC date; the git status
    /// is probed by running `git` in the cwd (a missing repo or absent `git` is reported, never an
    /// error). If the cwd cannot be read it is rendered as `unknown`.
    #[must_use]
    pub fn from_process() -> Self {
        let cwd = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "unknown".to_owned());
        let git = std::env::current_dir()
            .map(|path| git_status(&path))
            .unwrap_or(GitStatus::Unavailable);
        Self {
            cwd,
            platform: platform_name(std::env::consts::OS),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            date: utc_date_string(SystemTime::now()),
            git,
        }
    }

    /// Render the environment block as the text appended to the base system prompt.
    ///
    /// The cwd and platform lines are stated plainly and prominently so the model can quote the
    /// exact working directory and platform back to the user.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "<environment>\n\
             Current working directory: {cwd}\n\
             Platform: {platform} ({os} {arch})\n\
             Today's date: {date}\n\
             Git: {git}\n\
             </environment>",
            cwd = self.cwd,
            platform = self.platform,
            os = self.os,
            arch = self.arch,
            date = self.date,
            git = self.git.describe(),
        )
    }
}

/// Map a Rust target OS token to a human-facing platform name.
fn platform_name(os: &str) -> String {
    match os {
        "linux" => "Linux".to_owned(),
        "macos" => "macOS".to_owned(),
        "windows" => "Windows".to_owned(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => other.to_owned(),
            }
        }
    }
}

/// Probe the git status of `dir` by shelling out to `git`. Any failure (no `git`, not a repo)
/// resolves to a non-error [`GitStatus`].
fn git_status(dir: &Path) -> GitStatus {
    let inside = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output();
    let inside = match inside {
        Ok(output) if output.status.success() => output,
        Ok(_) => return GitStatus::NotARepo,
        Err(_) => return GitStatus::Unavailable,
    };
    if String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return GitStatus::NotARepo;
    }

    let branch = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|branch| !branch.is_empty() && branch != "HEAD");

    let dirty = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !output.stdout.is_empty())
        .unwrap_or(false);

    GitStatus::Repo { branch, dirty }
}

/// Format the UTC calendar date of `time` as `YYYY-MM-DD`.
///
/// Computed with the civil-from-days algorithm so no date dependency is required; times before the
/// Unix epoch (which cannot occur for `SystemTime::now`) fall back to the epoch date.
#[must_use]
fn utc_date_string(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Convert a count of days since the Unix epoch into a `(year, month, day)` civil date (UTC),
/// using Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_the_first_of_january_1970() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn a_known_timestamp_renders_the_expected_date() {
        // 1_700_000_000 == 2023-11-14T22:13:20Z.
        let time = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(utc_date_string(time), "2023-11-14");
    }

    #[test]
    fn platform_name_capitalizes_linux() {
        assert_eq!(platform_name("linux"), "Linux");
        assert_eq!(platform_name("macos"), "macOS");
    }

    #[test]
    fn render_states_cwd_platform_date_and_git() {
        let block = EnvironmentBlock {
            cwd: "/tmp/cortex-env-1234".to_owned(),
            platform: "Linux".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            date: "2026-06-19".to_owned(),
            git: GitStatus::NotARepo,
        };
        let rendered = block.render();
        assert!(rendered.contains("/tmp/cortex-env-1234"));
        assert!(rendered.contains("Linux"));
        assert!(rendered.contains("2026-06-19"));
        assert!(rendered.contains("not a git repository"));
    }

    #[test]
    fn git_describe_covers_each_state() {
        assert_eq!(GitStatus::NotARepo.describe(), "not a git repository");
        assert_eq!(
            GitStatus::Repo {
                branch: Some("main".to_owned()),
                dirty: false
            }
            .describe(),
            "branch main, clean working tree"
        );
        assert_eq!(
            GitStatus::Repo {
                branch: None,
                dirty: true
            }
            .describe(),
            "branch detached HEAD, uncommitted changes"
        );
    }

    #[test]
    fn from_process_reads_a_plausible_snapshot() {
        let block = EnvironmentBlock::from_process();
        assert!(!block.cwd.is_empty());
        assert!(!block.platform.is_empty());
        assert_eq!(block.os, std::env::consts::OS);
        assert_eq!(block.date.len(), 10, "date is YYYY-MM-DD");
    }
}
