//! Typed diagnostics model plus a per-file index for `publishDiagnostics` notifications.

use std::collections::BTreeMap;

use serde_json::Value;

/// The server-to-client diagnostics notification method.
pub const PUBLISH_DIAGNOSTICS_METHOD: &str = "textDocument/publishDiagnostics";

/// LSP `DiagnosticSeverity` (`1 = Error`, `2 = Warning`, `3 = Information`, `4 = Hint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    /// Error severity.
    Error,
    /// Warning severity.
    Warning,
    /// Information severity.
    Information,
    /// Hint severity.
    Hint,
}

impl DiagnosticSeverity {
    /// Map an LSP severity code to a typed severity.
    #[must_use]
    pub const fn from_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(Self::Error),
            2 => Some(Self::Warning),
            3 => Some(Self::Information),
            4 => Some(Self::Hint),
            _ => None,
        }
    }

    /// Stable lowercase label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "information",
            Self::Hint => "hint",
        }
    }
}

/// A zero-based LSP position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    /// Zero-based line.
    pub line: u32,
    /// Zero-based UTF-16 character offset.
    pub character: u32,
}

impl Position {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            line: u32::try_from(value.get("line")?.as_u64()?).ok()?,
            character: u32::try_from(value.get("character")?.as_u64()?).ok()?,
        })
    }
}

/// An inclusive-start, exclusive-end LSP range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    /// Start position.
    pub start: Position,
    /// End position.
    pub end: Position,
}

impl Range {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            start: Position::from_value(value.get("start")?)?,
            end: Position::from_value(value.get("end")?)?,
        })
    }
}

/// One LSP diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Source range.
    pub range: Range,
    /// Optional severity.
    pub severity: Option<DiagnosticSeverity>,
    /// Optional code, normalized to a string.
    pub code: Option<String>,
    /// Optional source, for example `rustc` or `Ruff`.
    pub source: Option<String>,
    /// Human-readable message.
    pub message: String,
}

impl Diagnostic {
    fn from_value(value: &Value) -> Option<Self> {
        let range = Range::from_value(value.get("range")?)?;
        let severity = value
            .get("severity")
            .and_then(Value::as_i64)
            .and_then(DiagnosticSeverity::from_code);
        let code = match value.get("code") {
            Some(Value::String(code)) => Some(code.clone()),
            Some(Value::Number(code)) => Some(code.to_string()),
            _ => None,
        };
        Some(Self {
            range,
            severity,
            code,
            source: value
                .get("source")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            message: value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        })
    }

    /// Stable severity label, defaulting to `note` when the server omitted one.
    #[must_use]
    pub fn severity_label(&self) -> &'static str {
        self.severity.map_or("note", DiagnosticSeverity::label)
    }
}

/// Parsed params for `textDocument/publishDiagnostics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishDiagnosticsParams {
    /// Document URI.
    pub uri: String,
    /// Optional document version.
    pub version: Option<i64>,
    /// Full diagnostic set for that URI.
    pub diagnostics: Vec<Diagnostic>,
}

impl PublishDiagnosticsParams {
    /// Parse notification params. Malformed individual diagnostics are skipped.
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            uri: value.get("uri")?.as_str()?.to_owned(),
            version: value.get("version").and_then(Value::as_i64),
            diagnostics: value
                .get("diagnostics")?
                .as_array()?
                .iter()
                .filter_map(Diagnostic::from_value)
                .collect(),
        })
    }
}

/// Per-file replacement index of latest diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticStore {
    by_file: BTreeMap<String, Vec<Diagnostic>>,
}

impl DiagnosticStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply publishDiagnostics replacement semantics for one URI.
    pub fn apply(&mut self, params: PublishDiagnosticsParams) {
        if params.diagnostics.is_empty() {
            self.by_file.remove(&params.uri);
        } else {
            self.by_file.insert(params.uri, params.diagnostics);
        }
    }

    /// Diagnostics currently known for `uri`.
    #[must_use]
    pub fn for_file(&self, uri: &str) -> &[Diagnostic] {
        self.by_file.get(uri).map_or(&[], Vec::as_slice)
    }

    /// Total diagnostic count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_file.values().map(Vec::len).sum()
    }

    /// Whether there are no diagnostics.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_file.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_diagnostic_severity_range_code_source() {
        let params = PublishDiagnosticsParams::from_value(&json!({
            "uri":"file:///a.py",
            "version":1,
            "diagnostics":[{
                "range":{"start":{"line":2,"character":4},"end":{"line":2,"character":9}},
                "severity":2,
                "code":"F401",
                "source":"Ruff",
                "message":"unused import"
            }]
        }))
        .expect("params parse");
        assert_eq!(params.uri, "file:///a.py");
        assert_eq!(
            params.diagnostics[0].severity,
            Some(DiagnosticSeverity::Warning)
        );
        assert_eq!(params.diagnostics[0].severity_label(), "warning");
        assert_eq!(params.diagnostics[0].range.start.character, 4);
        assert_eq!(params.diagnostics[0].source.as_deref(), Some("Ruff"));
    }

    #[test]
    fn publish_replaces_and_empty_clears() {
        let mut store = DiagnosticStore::new();
        store.apply(
            PublishDiagnosticsParams::from_value(&json!({
                "uri":"file:///a.rs",
                "diagnostics":[{
                    "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},
                    "severity":1,
                    "message":"bad"
                }]
            }))
            .unwrap(),
        );
        assert_eq!(store.len(), 1);
        store.apply(
            PublishDiagnosticsParams::from_value(&json!({
                "uri":"file:///a.rs",
                "diagnostics":[]
            }))
            .unwrap(),
        );
        assert!(store.is_empty());
    }
}
