use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

/// The fixed runtime precedence. The first executable found on `PATH` wins.
pub const PRECEDENCE: [JsRuntime; 3] = [JsRuntime::Bun, JsRuntime::Node, JsRuntime::Deno];

/// A supported external JavaScript runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsRuntime {
    /// Bun (`bun`).
    Bun,
    /// Node.js (`node`).
    Node,
    /// Deno (`deno`).
    Deno,
}

impl JsRuntime {
    /// The command name searched on `PATH`.
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Node => "node",
            Self::Deno => "deno",
        }
    }

    /// A human-readable runtime name for diagnostics.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Bun => "Bun",
            Self::Node => "Node",
            Self::Deno => "Deno",
        }
    }
}

impl fmt::Display for JsRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name())
    }
}

/// The runtime selected for a plugin invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSelection {
    /// Which runtime matched the precedence list.
    pub runtime: JsRuntime,
    /// The exact executable path spawned.
    pub executable: PathBuf,
}

/// Detect a runtime using the process `PATH`.
#[must_use]
pub fn detect_runtime() -> Option<RuntimeSelection> {
    detect_runtime_in_path(std::env::var_os("PATH").as_deref())
}

/// Detect a runtime using a caller-supplied `PATH` value.
#[must_use]
pub fn detect_runtime_in_path(path: Option<&OsStr>) -> Option<RuntimeSelection> {
    let path = path?;
    for runtime in PRECEDENCE {
        if let Some(executable) = find_executable(path, runtime.command()) {
            return Some(RuntimeSelection {
                runtime,
                executable,
            });
        }
    }
    None
}

fn find_executable(path: &OsStr, command: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(command))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    is_executable(&metadata)
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cortex-plugin-detect-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn write_executable(dir: &Path, name: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn uses_bun_node_deno_precedence() {
        let dir = temp_dir("precedence");
        write_executable(&dir, "node");
        write_executable(&dir, "deno");
        let selected = detect_runtime_in_path(Some(dir.as_os_str())).unwrap();
        assert_eq!(selected.runtime, JsRuntime::Node);

        write_executable(&dir, "bun");
        let selected = detect_runtime_in_path(Some(dir.as_os_str())).unwrap();
        assert_eq!(selected.runtime, JsRuntime::Bun);

        std::fs::remove_file(dir.join("bun")).unwrap();
        let selected = detect_runtime_in_path(Some(dir.as_os_str())).unwrap();
        assert_eq!(selected.runtime, JsRuntime::Node);

        std::fs::remove_file(dir.join("node")).unwrap();
        let selected = detect_runtime_in_path(Some(dir.as_os_str())).unwrap();
        assert_eq!(selected.runtime, JsRuntime::Deno);
    }

    #[test]
    fn returns_none_when_no_runtime_is_on_path() {
        let dir = temp_dir("none");
        assert!(detect_runtime_in_path(Some(dir.as_os_str())).is_none());
    }
}
