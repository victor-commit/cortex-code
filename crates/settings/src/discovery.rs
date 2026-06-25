//! Config-file discovery and the file-layer ordering.
//!
//! Files are returned in ascending precedence (lowest first), so a loader can merge them in
//! order with later files winning:
//!
//! 1. **global** — `${XDG_CONFIG_HOME:-$HOME/.config}/cortex/config.toml`
//! 2. **project chain** — every `cortex.toml` / `.cortex/config.toml` from the topmost project
//!    ancestor down to the invocation directory, so a config in a nested subdirectory (closer
//!    to the CWD) overrides the project-root config.
//!
//! The project walk starts at the CWD and climbs toward the filesystem root, but never scans
//! the home directory or above it: `$HOME` is the boundary so the global config location is
//! never double-counted as a project layer.

use std::path::{Path, PathBuf};

/// A bare `cortex.toml` placed directly in a directory.
const PROJECT_CONFIG_FILE: &str = "cortex.toml";
/// The `.cortex/` project config directory and the file inside it.
const PROJECT_CONFIG_DIR: &str = ".cortex";
const CONFIG_FILE: &str = "config.toml";
const JSON_CONFIG_FILE: &str = "cortex.json";
const JSONC_CONFIG_FILE: &str = "cortex.jsonc";
const NESTED_JSON_CONFIG_FILE: &str = "config.json";
const NESTED_JSONC_CONFIG_FILE: &str = "config.jsonc";

/// Candidate config file paths in ascending precedence order (lowest first).
///
/// Paths are returned whether or not they exist on disk; the caller skips absent files.
pub fn config_files(
    home: Option<&Path>,
    xdg_config_home: Option<&Path>,
    cwd: &Path,
) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Some(dir) = global_config_dir(home, xdg_config_home) {
        files.push(dir.join("cortex").join(CONFIG_FILE));
    }

    for dir in project_chain(home, cwd) {
        files.push(dir.join(PROJECT_CONFIG_FILE));
        files.push(dir.join(PROJECT_CONFIG_DIR).join(CONFIG_FILE));
    }

    files
}

/// Candidate JSON/JSONC config file paths in ascending precedence order (lowest first).
///
/// These are not loaded as config layers today (Cortex settings are TOML), but the loader uses
/// this list to produce a clear migration diagnostic instead of silently ignoring these files.
pub fn json_config_files(
    home: Option<&Path>,
    xdg_config_home: Option<&Path>,
    cwd: &Path,
) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Some(dir) = global_config_dir(home, xdg_config_home) {
        let root = dir.join("cortex");
        files.push(root.join(JSON_CONFIG_FILE));
        files.push(root.join(JSONC_CONFIG_FILE));
        files.push(root.join(PROJECT_CONFIG_DIR).join(NESTED_JSON_CONFIG_FILE));
        files.push(root.join(PROJECT_CONFIG_DIR).join(NESTED_JSONC_CONFIG_FILE));
    }

    for dir in project_chain(home, cwd) {
        files.push(dir.join(JSON_CONFIG_FILE));
        files.push(dir.join(JSONC_CONFIG_FILE));
        files.push(dir.join(PROJECT_CONFIG_DIR).join(NESTED_JSON_CONFIG_FILE));
        files.push(dir.join(PROJECT_CONFIG_DIR).join(NESTED_JSONC_CONFIG_FILE));
    }

    files
}

/// The user's global config file (`<config>/cortex/config.toml`), if a config home exists.
///
/// This is the durable destination for interactively-set user preferences (e.g. the TUI theme
/// picker): a value written here is read back by [`config_files`] as the global layer on the next
/// load. Returns `None` only when neither `$XDG_CONFIG_HOME` nor `$HOME` is available.
pub fn global_config_file(home: Option<&Path>, xdg_config_home: Option<&Path>) -> Option<PathBuf> {
    global_config_dir(home, xdg_config_home).map(|dir| dir.join("cortex").join(CONFIG_FILE))
}

/// The root config directory (`$XDG_CONFIG_HOME` or `$HOME/.config`), when available.
pub fn global_config_root(home: Option<&Path>, xdg_config_home: Option<&Path>) -> Option<PathBuf> {
    global_config_dir(home, xdg_config_home)
}

/// The directory holding the global config tree (`<config>/cortex/config.toml`).
fn global_config_dir(home: Option<&Path>, xdg_config_home: Option<&Path>) -> Option<PathBuf> {
    if let Some(xdg) = xdg_config_home {
        if !xdg.as_os_str().is_empty() {
            return Some(xdg.to_path_buf());
        }
    }
    home.map(|home| home.join(".config"))
}

/// Project directories from the topmost project ancestor down to (and including) the CWD.
///
/// Lowest precedence (topmost ancestor) first. The walk stops at `$HOME` (exclusive) so the
/// home directory and its ancestors are never treated as project layers.
fn project_chain(home: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = Some(cwd.to_path_buf());
    while let Some(dir) = current {
        if Some(dir.as_path()) == home {
            break;
        }
        let parent = dir.parent().map(Path::to_path_buf);
        chain.push(dir);
        current = parent;
    }
    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_prefers_xdg_over_home() {
        let dir = global_config_dir(Some(Path::new("/home/u")), Some(Path::new("/xdg")));
        assert_eq!(dir, Some(PathBuf::from("/xdg")));
    }

    #[test]
    fn global_falls_back_to_home_config_when_xdg_absent_or_empty() {
        assert_eq!(
            global_config_dir(Some(Path::new("/home/u")), None),
            Some(PathBuf::from("/home/u/.config"))
        );
        assert_eq!(
            global_config_dir(Some(Path::new("/home/u")), Some(Path::new(""))),
            Some(PathBuf::from("/home/u/.config"))
        );
    }

    #[test]
    fn project_chain_is_topmost_first_and_stops_at_home() {
        let chain = project_chain(Some(Path::new("/home/u")), Path::new("/home/u/proj/sub"));
        assert_eq!(
            chain,
            vec![
                PathBuf::from("/home/u/proj"),
                PathBuf::from("/home/u/proj/sub"),
            ],
            "home and above are excluded; topmost ancestor comes first"
        );
    }

    #[test]
    fn project_chain_is_empty_at_home_root() {
        let chain = project_chain(Some(Path::new("/home/u")), Path::new("/home/u"));
        assert!(chain.is_empty());
    }

    #[test]
    fn config_files_order_is_global_then_project_root_then_nested() {
        let files = config_files(
            Some(Path::new("/home/u")),
            None,
            Path::new("/home/u/proj/sub"),
        );
        assert_eq!(
            files,
            vec![
                PathBuf::from("/home/u/.config/cortex/config.toml"),
                PathBuf::from("/home/u/proj/cortex.toml"),
                PathBuf::from("/home/u/proj/.cortex/config.toml"),
                PathBuf::from("/home/u/proj/sub/cortex.toml"),
                PathBuf::from("/home/u/proj/sub/.cortex/config.toml"),
            ],
        );
    }

    #[test]
    fn json_config_candidates_cover_global_project_and_nested_paths() {
        let files = json_config_files(
            Some(Path::new("/home/u")),
            None,
            Path::new("/home/u/proj/sub"),
        );
        assert!(
            files.contains(&PathBuf::from("/home/u/.config/cortex/cortex.json")),
            "global JSON candidate is present"
        );
        assert!(
            files.contains(&PathBuf::from("/home/u/proj/cortex.jsonc")),
            "project JSONC candidate is present"
        );
        assert!(
            files.contains(&PathBuf::from("/home/u/proj/sub/.cortex/config.json")),
            "nested .cortex JSON candidate is present"
        );
    }
}
