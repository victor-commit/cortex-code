//! Mission root path resolution (`~/.cortex/missions` by default).

use std::path::PathBuf;

/// Override for the missions root directory.
pub const ENV_MISSIONS_ROOT: &str = "CORTEX_MISSIONS_ROOT";

/// Subdirectory under `$HOME` when no override is set.
const DEFAULT_HOME_SUBDIR: &str = ".cortex/missions";

/// Inputs that determine the missions root (testable; not ambient).
#[derive(Debug, Clone, Default)]
pub struct MissionsRootInputs {
    /// `CORTEX_MISSIONS_ROOT` when set and non-empty.
    pub missions_root: Option<PathBuf>,
    /// User home directory.
    pub home: Option<PathBuf>,
}

impl MissionsRootInputs {
    /// Populate from the process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            missions_root: non_empty_var(ENV_MISSIONS_ROOT),
            home: non_empty_var("HOME"),
        }
    }

    /// Resolve the missions root directory.
    ///
    /// Precedence: `CORTEX_MISSIONS_ROOT` → `$HOME/.cortex/missions`.
    #[must_use]
    pub fn resolve(&self) -> Option<PathBuf> {
        if let Some(root) = self.missions_root.as_ref() {
            return Some(root.clone());
        }
        self.home
            .as_ref()
            .map(|home| home.join(DEFAULT_HOME_SUBDIR))
    }
}

/// Resolve missions root from the process environment.
#[must_use]
pub fn missions_root() -> Option<PathBuf> {
    MissionsRootInputs::from_process().resolve()
}

fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn override_wins_over_home() {
        let inputs = MissionsRootInputs {
            missions_root: Some(PathBuf::from("/tmp/cortex-missions-test")),
            home: Some(PathBuf::from("/home/user")),
        };
        assert_eq!(
            inputs.resolve().as_deref(),
            Some(Path::new("/tmp/cortex-missions-test"))
        );
    }

    #[test]
    fn default_is_dot_cortex_missions() {
        let inputs = MissionsRootInputs {
            missions_root: None,
            home: Some(PathBuf::from("/home/user")),
        };
        assert_eq!(
            inputs.resolve().as_deref(),
            Some(Path::new("/home/user/.cortex/missions"))
        );
    }

    #[test]
    fn missing_home_and_override_yields_none() {
        let inputs = MissionsRootInputs {
            missions_root: None,
            home: None,
        };
        assert!(inputs.resolve().is_none());
    }
}
