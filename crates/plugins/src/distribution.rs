use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) const LOCKFILE_RELATIVE_PATH: &[&str] = &[".cortex", "plugin-lock.json"];
pub(crate) const PACKAGE_INSTALL_RELATIVE_PATH: &[&str] = &[".cortex", "plugins", "npm"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackageLockEntry {
    pub(crate) spec: String,
    pub(crate) import_spec: String,
    pub(crate) package_name: String,
    pub(crate) version: String,
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

    pub(crate) fn entry(&self, cwd: &Path, spec: &str) -> Option<PackageLockEntry> {
        self.plugins
            .iter()
            .find(|entry| entry.spec == spec)
            .map(|entry| PackageLockEntry {
                spec: entry.spec.clone(),
                import_spec: entry.import_spec.clone(),
                package_name: entry.package_name.clone(),
                version: entry.version.clone(),
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
    Ok(())
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
