use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const LOCKFILE_RELATIVE_PATH: &[&str] = &[".cortex", "plugin-lock.json"];
pub(crate) const PACKAGE_INSTALL_RELATIVE_PATH: &[&str] = &[".cortex", "plugins", "npm"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackageLockEntry {
    pub(crate) spec: String,
    pub(crate) import_spec: String,
    pub(crate) package_name: String,
    pub(crate) version: String,
    pub(crate) integrity: String,
    pub(crate) module_path: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct PackageLockfile {
    schema_version: u32,
    #[serde(default)]
    plugins: Vec<PackageLockEntryWire>,
}

impl PackageLockfile {
    pub(crate) fn load(cwd: &Path) -> Result<Self, PackageLockError> {
        let path = lockfile_path(cwd);
        if !path.exists() {
            return Ok(Self {
                schema_version: 1,
                plugins: Vec::new(),
            });
        }
        let text = std::fs::read_to_string(&path).map_err(|source| PackageLockError::Io {
            action: "read plugin lockfile",
            path: path.clone(),
            source,
        })?;
        let lockfile: Self =
            serde_json::from_str(&text).map_err(|source| PackageLockError::Json {
                action: "parse plugin lockfile",
                path: path.clone(),
                source,
            })?;
        if lockfile.schema_version != 1 {
            return Err(PackageLockError::InvalidSchema {
                path,
                schema_version: lockfile.schema_version,
            });
        }
        Ok(lockfile)
    }

    /// Project the lockfile into the public, redaction-safe summary used by `cortex plugin list`.
    pub(crate) fn locked_plugins(&self) -> Vec<LockedPlugin> {
        self.plugins
            .iter()
            .map(|entry| LockedPlugin {
                spec: entry.spec.clone(),
                package_name: entry.package_name.clone(),
                version: entry.version.clone(),
                integrity: entry.integrity.clone(),
            })
            .collect()
    }

    /// Remove the lock entry for `spec`, returning whether an entry was present.
    pub(crate) fn remove(&mut self, spec: &str) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|entry| entry.spec != spec);
        self.plugins.len() != before
    }

    pub(crate) fn entry(&self, cwd: &Path, spec: &str) -> Option<PackageLockEntry> {
        self.plugins
            .iter()
            .find(|entry| entry.spec == spec)
            .map(|entry| PackageLockEntry {
                spec: entry.spec.clone(),
                import_spec: entry.import_spec.clone(),
                package_name: entry.package_name.clone(),
                version: entry.version.clone(),
                integrity: entry.integrity.clone(),
                module_path: restore_path(cwd, &entry.module_path),
            })
    }

    pub(crate) fn upsert(
        &mut self,
        cwd: &Path,
        entry: PackageLockEntry,
    ) -> Result<(), PackageLockError> {
        self.schema_version = 1;
        let wire = PackageLockEntryWire {
            spec: entry.spec,
            import_spec: entry.import_spec,
            package_name: entry.package_name,
            version: entry.version,
            integrity: entry.integrity,
            module_path: store_path(cwd, &entry.module_path),
        };
        if let Some(existing) = self
            .plugins
            .iter_mut()
            .find(|existing| existing.spec == wire.spec)
        {
            *existing = wire;
        } else {
            self.plugins.push(wire);
            self.plugins
                .sort_by(|left, right| left.spec.cmp(&right.spec));
        }
        self.save(cwd)
    }

    fn save(&self, cwd: &Path) -> Result<(), PackageLockError> {
        let path = lockfile_path(cwd);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| PackageLockError::Io {
                action: "create plugin lockfile directory",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut text =
            serde_json::to_string_pretty(self).map_err(|source| PackageLockError::Json {
                action: "serialize plugin lockfile",
                path: path.clone(),
                source,
            })?;
        text.push('\n');
        std::fs::write(&path, text).map_err(|source| PackageLockError::Io {
            action: "write plugin lockfile",
            path,
            source,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PackageLockEntryWire {
    spec: String,
    import_spec: String,
    package_name: String,
    version: String,
    #[serde(default)]
    integrity: String,
    module_path: String,
}

#[derive(Debug)]
pub(crate) enum PackageLockError {
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        action: &'static str,
        path: PathBuf,
        source: serde_json::Error,
    },
    InvalidSchema {
        path: PathBuf,
        schema_version: u32,
    },
}

impl std::fmt::Display for PackageLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                action,
                path,
                source,
            } => write!(
                formatter,
                "failed to {action} at {}: {source}",
                path.display()
            ),
            Self::Json {
                action,
                path,
                source,
            } => write!(
                formatter,
                "failed to {action} at {}: {source}",
                path.display()
            ),
            Self::InvalidSchema {
                path,
                schema_version,
            } => write!(
                formatter,
                "plugin lockfile {} has unsupported schema_version {schema_version}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PackageLockError {}

/// A redaction-safe summary of one pinned package plugin from the project lockfile.
///
/// This is the public projection consumed by `cortex plugin list`; it carries only the
/// reproducibility metadata (spec, resolved package name, pinned version, integrity digest) and
/// never any module bytes or secret-bearing values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LockedPlugin {
    pub spec: String,
    pub package_name: String,
    pub version: String,
    pub integrity: String,
}

/// List the package plugins pinned in the project lockfile under `cwd`.
///
/// Returns an empty list when no lockfile exists. The result is sorted by spec (the lockfile keeps
/// its entries sorted), so the listing is deterministic.
pub fn locked_plugins(cwd: &Path) -> Result<Vec<LockedPlugin>, String> {
    let lockfile = PackageLockfile::load(cwd).map_err(|err| err.to_string())?;
    Ok(lockfile.locked_plugins())
}

/// Remove the lock entry for `spec` from the project lockfile under `cwd`.
///
/// Returns `Ok(true)` when an entry was present and removed (the lockfile is rewritten), `Ok(false)`
/// when no matching entry existed (the lockfile is left untouched, and never created).
pub fn remove_locked_plugin(cwd: &Path, spec: &str) -> Result<bool, String> {
    let mut lockfile = PackageLockfile::load(cwd).map_err(|err| err.to_string())?;
    if !lockfile.remove(spec) {
        return Ok(false);
    }
    lockfile.save(cwd).map_err(|err| err.to_string())?;
    Ok(true)
}

/// The project-relative path of the plugin lockfile (for diagnostics), for example
/// `.cortex/plugin-lock.json`.
#[must_use]
pub fn lockfile_relative_path() -> String {
    LOCKFILE_RELATIVE_PATH.join("/")
}

pub(crate) fn lockfile_path(cwd: &Path) -> PathBuf {
    LOCKFILE_RELATIVE_PATH
        .iter()
        .fold(cwd.to_path_buf(), |path, segment| path.join(segment))
}

pub(crate) fn package_install_dir(cwd: &Path) -> PathBuf {
    PACKAGE_INSTALL_RELATIVE_PATH
        .iter()
        .fold(cwd.to_path_buf(), |path, segment| path.join(segment))
}

pub(crate) fn package_import_spec(spec: &str, cwd: &Path) -> Result<String, String> {
    let stripped = spec.strip_prefix("npm:").unwrap_or(spec);
    if let Some(path) = stripped.strip_prefix("file:") {
        return package_name_from_file_spec(path, cwd);
    }
    if stripped.starts_with('@') {
        let mut parts = stripped.split('/');
        let scope = parts.next().unwrap_or_default();
        let package = parts.next().unwrap_or_default();
        let package = package
            .rsplit_once('@')
            .map_or(package, |(name, _version)| name);
        return Ok(format!("{scope}/{package}"));
    }
    let first = stripped.split('/').next().unwrap_or(stripped);
    let name = first
        .rsplit_once('@')
        .map_or(first, |(name, _version)| name);
    if name.is_empty() {
        return Err(format!(
            "plugin package spec `{spec}` does not include a package name"
        ));
    }
    Ok(name.to_owned())
}

pub(crate) fn normalize_install_spec(spec: &str, cwd: &Path) -> String {
    let stripped = spec.strip_prefix("npm:").unwrap_or(spec);
    if let Some(path) = stripped.strip_prefix("file:") {
        let path = resolve_path(cwd, path);
        return format!("file:{}", path.display());
    }
    stripped.to_owned()
}

pub(crate) fn locked_entry_is_valid(entry: &PackageLockEntry) -> Result<(), String> {
    if !entry.module_path.is_file() {
        return Err(format!(
            "locked plugin package `{}` points at missing module {}",
            entry.spec,
            entry.module_path.display()
        ));
    }
    let Some((_, version)) = nearest_package_json(&entry.module_path) else {
        return Err(format!(
            "locked plugin package `{}` has no readable package.json near {}",
            entry.spec,
            entry.module_path.display()
        ));
    };
    if version != entry.version {
        return Err(format!(
            "plugin package `{}` is pinned to version {} but {} reports version {}; remove {} to re-resolve",
            entry.spec,
            entry.version,
            entry.module_path.display(),
            version,
            LOCKFILE_RELATIVE_PATH.join("/")
        ));
    }
    if !entry.integrity.trim().is_empty() {
        let actual_integrity = module_integrity(&entry.module_path).map_err(|message| {
            format!(
                "plugin package `{}` integrity check failed for {}: {message}",
                entry.spec,
                entry.module_path.display()
            )
        })?;
        if entry.integrity != actual_integrity {
            return Err(format!(
                "plugin package `{}` integrity mismatch for {} (expected {}, got {}); remove {} to re-resolve",
                entry.spec,
                entry.module_path.display(),
                entry.integrity,
                actual_integrity,
                LOCKFILE_RELATIVE_PATH.join("/")
            ));
        }
    }
    Ok(())
}

pub(crate) fn module_integrity(module_path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(module_path).map_err(|source| source.to_string())?;
    let digest = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(format!("sha256:{hex}"))
}

fn nearest_package_json(module_path: &Path) -> Option<(String, String)> {
    let mut current = module_path.parent();
    while let Some(dir) = current {
        let package_json = dir.join("package.json");
        if let Ok(text) = std::fs::read_to_string(&package_json)
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            let version = value
                .get("version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            if !version.is_empty() {
                return Some((name, version));
            }
        }
        current = dir.parent();
    }
    None
}

fn package_name_from_file_spec(path: &str, cwd: &Path) -> Result<String, String> {
    let root = resolve_path(cwd, path);
    let package_json = root.join("package.json");
    let text = std::fs::read_to_string(&package_json).map_err(|source| {
        format!(
            "plugin package spec `file:{path}` could not read {}: {source}",
            package_json.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|source| {
        format!(
            "plugin package spec `file:{path}` has invalid package.json at {}: {source}",
            package_json.display()
        )
    })?;
    value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "plugin package spec `file:{path}` package.json at {} is missing a name",
                package_json.display()
            )
        })
}

fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn store_path(cwd: &Path, path: &Path) -> String {
    path.strip_prefix(cwd)
        .map(|relative| relative.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

fn restore_path(cwd: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_import_spec_strips_versions_and_protocols() {
        let cwd = Path::new("/tmp/cortex");
        assert_eq!(
            package_import_spec("demo-plugin", cwd).unwrap(),
            "demo-plugin"
        );
        assert_eq!(
            package_import_spec("demo-plugin@1.2.3", cwd).unwrap(),
            "demo-plugin"
        );
        assert_eq!(
            package_import_spec("@scope/demo@1.2.3", cwd).unwrap(),
            "@scope/demo"
        );
        assert_eq!(
            package_import_spec("npm:@scope/demo@1.2.3", cwd).unwrap(),
            "@scope/demo"
        );
    }
}
