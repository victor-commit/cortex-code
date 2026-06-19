//! Resolution of the on-disk database location.
//!
//! The database lives under a per-user data directory by default and is relocatable through two
//! documented, operational environment variables (resolved directly, never as config keys):
//!
//! - `CORTEX_DB_PATH` — an explicit path to the database file (highest precedence).
//! - `CORTEX_DATA_DIR` — the data directory; the database is `<CORTEX_DATA_DIR>/cortex.db`.
//!
//! With neither set, the default is `${XDG_DATA_HOME:-$HOME/.local/share}/cortex/cortex.db`.

use std::path::{Path, PathBuf};

/// The database file name placed inside the resolved data directory.
pub const DB_FILE_NAME: &str = "cortex.db";

/// The data subdirectory under the user's data home.
const DATA_SUBDIR: &str = "cortex";

/// The explicit database-path override.
pub const ENV_DB_PATH: &str = "CORTEX_DB_PATH";

/// The data-directory override.
pub const ENV_DATA_DIR: &str = "CORTEX_DATA_DIR";

/// Inputs that determine the database path, captured explicitly so resolution is deterministic
/// and unit-testable rather than reading the process environment ambiently.
#[derive(Debug, Clone, Default)]
pub struct DbPathInputs {
    /// `CORTEX_DB_PATH`, when set and non-empty: an explicit path to the database file.
    pub db_path: Option<PathBuf>,
    /// `CORTEX_DATA_DIR`, when set and non-empty: the directory to place the database in.
    pub data_dir: Option<PathBuf>,
    /// `XDG_DATA_HOME`, when set and non-empty: the base data home for the default location.
    pub xdg_data_home: Option<PathBuf>,
    /// The user's `HOME`, used for the default `${HOME}/.local/share` data home.
    pub home: Option<PathBuf>,
}

impl DbPathInputs {
    /// Populate the inputs from the current process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            db_path: non_empty_var(ENV_DB_PATH),
            data_dir: non_empty_var(ENV_DATA_DIR),
            xdg_data_home: non_empty_var("XDG_DATA_HOME"),
            home: non_empty_var("HOME"),
        }
    }

    /// Resolve the database file path according to the documented precedence.
    ///
    /// Returns `None` only when no override is set and `HOME`/`XDG_DATA_HOME` are both absent,
    /// so the default location cannot be determined.
    #[must_use]
    pub fn resolve(&self) -> Option<PathBuf> {
        if let Some(db_path) = self.db_path.as_ref() {
            return Some(db_path.clone());
        }
        if let Some(data_dir) = self.data_dir.as_ref() {
            return Some(data_dir.join(DB_FILE_NAME));
        }
        self.default_data_home()
            .map(|home| home.join(DATA_SUBDIR).join(DB_FILE_NAME))
    }

    /// The base data home for the default location: `XDG_DATA_HOME` if set, else
    /// `${HOME}/.local/share`.
    fn default_data_home(&self) -> Option<PathBuf> {
        if let Some(xdg) = self.xdg_data_home.as_ref() {
            return Some(xdg.clone());
        }
        self.home
            .as_ref()
            .map(|home| home.join(".local").join("share"))
    }
}

/// Read an environment variable, treating an unset or empty value as absent.
fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The human-readable default location string for diagnostics, given a `HOME`.
#[must_use]
pub fn default_location_hint(home: Option<&Path>) -> String {
    match home {
        Some(home) => home
            .join(".local")
            .join("share")
            .join(DATA_SUBDIR)
            .join(DB_FILE_NAME)
            .display()
            .to_string(),
        None => format!("${{XDG_DATA_HOME:-$HOME/.local/share}}/{DATA_SUBDIR}/{DB_FILE_NAME}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_db_path_wins_over_everything() {
        let inputs = DbPathInputs {
            db_path: Some(PathBuf::from("/explicit/my.db")),
            data_dir: Some(PathBuf::from("/data")),
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
        };
        assert_eq!(inputs.resolve(), Some(PathBuf::from("/explicit/my.db")));
    }

    #[test]
    fn data_dir_override_places_db_inside_it() {
        let inputs = DbPathInputs {
            db_path: None,
            data_dir: Some(PathBuf::from("/data")),
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
        };
        assert_eq!(inputs.resolve(), Some(PathBuf::from("/data/cortex.db")));
    }

    #[test]
    fn default_prefers_xdg_data_home() {
        let inputs = DbPathInputs {
            db_path: None,
            data_dir: None,
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
        };
        assert_eq!(
            inputs.resolve(),
            Some(PathBuf::from("/xdg/cortex/cortex.db"))
        );
    }

    #[test]
    fn default_falls_back_to_home_local_share() {
        let inputs = DbPathInputs {
            db_path: None,
            data_dir: None,
            xdg_data_home: None,
            home: Some(PathBuf::from("/home/u")),
        };
        assert_eq!(
            inputs.resolve(),
            Some(PathBuf::from("/home/u/.local/share/cortex/cortex.db"))
        );
    }

    #[test]
    fn unresolvable_without_home_or_xdg() {
        let inputs = DbPathInputs::default();
        assert_eq!(inputs.resolve(), None);
    }
}
