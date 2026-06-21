//! Navigation requests and result rendering for the local LSP client.
//!
//! This module owns typed, lenient parsing for the polymorphic LSP navigation responses used by
//! hover, definition, references, and symbol queries. It deliberately avoids a large `lsp-types`
//! dependency and mirrors the small hand-written protocol model used by diagnostics.

use serde_json::{Value, json};

use crate::diagnostics::{Position, Range};

pub(crate) const HOVER_METHOD: &str = "textDocument/hover";
pub(crate) const DEFINITION_METHOD: &str = "textDocument/definition";
pub(crate) const REFERENCES_METHOD: &str = "textDocument/references";
pub(crate) const DOCUMENT_SYMBOL_METHOD: &str = "textDocument/documentSymbol";
pub(crate) const WORKSPACE_SYMBOL_METHOD: &str = "workspace/symbol";

/// Textual hover content plus the optional range the server reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// Hover text collapsed to one deterministic string.
    pub contents: String,
    /// Optional zero-based LSP range.
    pub range: Option<Range>,
}

impl Hover {
    fn from_value(value: &Value) -> Option<Self> {
        let contents = hover_contents_string(value.get("contents")?);
        if contents.is_empty() {
            return None;
        }
        let range = value.get("range").and_then(Range::from_value);
        Some(Self { contents, range })
    }
}

fn hover_contents_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Object(_) => value
            .get("value")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_default(),
        Value::Array(items) => items
            .iter()
            .map(hover_contents_string)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// A source location resolved by a navigation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Document URI, usually `file:///...`.
    pub uri: String,
    /// Zero-based LSP range in the document.
    pub range: Range,
}

impl Location {
    fn from_any(value: &Value) -> Option<Self> {
        if let Some(uri) = value.get("uri").and_then(Value::as_str) {
            return Some(Self {
                uri: uri.to_owned(),
                range: Range::from_value(value.get("range")?)?,
            });
        }
        if let Some(uri) = value.get("targetUri").and_then(Value::as_str) {
            let range_value = value
                .get("targetSelectionRange")
                .or_else(|| value.get("targetRange"))?;
            return Some(Self {
                uri: uri.to_owned(),
                range: Range::from_value(range_value)?,
            });
        }
        None
    }
}

/// LSP `SymbolKind` (`1..=26`) with stable labels for tool output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    File,
    Module,
    Namespace,
    Package,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    Function,
    Variable,
    Constant,
    StringLiteral,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
    EnumMember,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

impl SymbolKind {
    /// Map an LSP symbol-kind code to a typed kind.
    #[must_use]
    pub const fn from_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(Self::File),
            2 => Some(Self::Module),
            3 => Some(Self::Namespace),
            4 => Some(Self::Package),
            5 => Some(Self::Class),
            6 => Some(Self::Method),
            7 => Some(Self::Property),
            8 => Some(Self::Field),
            9 => Some(Self::Constructor),
            10 => Some(Self::Enum),
            11 => Some(Self::Interface),
            12 => Some(Self::Function),
            13 => Some(Self::Variable),
            14 => Some(Self::Constant),
            15 => Some(Self::StringLiteral),
            16 => Some(Self::Number),
            17 => Some(Self::Boolean),
            18 => Some(Self::Array),
            19 => Some(Self::Object),
            20 => Some(Self::Key),
            21 => Some(Self::Null),
            22 => Some(Self::EnumMember),
            23 => Some(Self::Struct),
            24 => Some(Self::Event),
            25 => Some(Self::Operator),
            26 => Some(Self::TypeParameter),
            _ => None,
        }
    }

    /// Stable lowercase label used in model-readable output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Module => "module",
            Self::Namespace => "namespace",
            Self::Package => "package",
            Self::Class => "class",
            Self::Method => "method",
            Self::Property => "property",
            Self::Field => "field",
            Self::Constructor => "constructor",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Function => "function",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::StringLiteral => "string",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Array => "array",
            Self::Object => "object",
            Self::Key => "key",
            Self::Null => "null",
            Self::EnumMember => "enum-member",
            Self::Struct => "struct",
            Self::Event => "event",
            Self::Operator => "operator",
            Self::TypeParameter => "type-parameter",
        }
    }
}

/// A possibly nested document symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSymbol {
    /// Symbol name.
    pub name: String,
    /// Server-provided detail such as a signature.
    pub detail: Option<String>,
    /// Symbol kind, when known.
    pub kind: Option<SymbolKind>,
    /// Zero-based range enclosing the symbol.
    pub range: Range,
    /// Container name for flat `SymbolInformation` responses.
    pub container: Option<String>,
    /// Nested children for hierarchical `DocumentSymbol` responses.
    pub children: Vec<DocumentSymbol>,
}

impl DocumentSymbol {
    fn from_value(value: &Value) -> Option<Self> {
        let name = value.get("name")?.as_str()?.to_owned();
        let kind = value
            .get("kind")
            .and_then(Value::as_i64)
            .and_then(SymbolKind::from_code);
        let range = match value.get("range") {
            Some(range) => Range::from_value(range)?,
            None => Range::from_value(value.get("location")?.get("range")?)?,
        };
        let detail = value
            .get("detail")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let container = value
            .get("containerName")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Self::from_value).collect())
            .unwrap_or_default();
        Some(Self {
            name,
            detail,
            kind,
            range,
            container,
            children,
        })
    }
}

/// A flat workspace symbol with its location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolInformation {
    /// Symbol name.
    pub name: String,
    /// Symbol kind, when known.
    pub kind: Option<SymbolKind>,
    /// Document URI where the symbol lives.
    pub uri: String,
    /// Zero-based range. Some `WorkspaceSymbol` responses omit it; those default to zero.
    pub range: Range,
    /// Optional enclosing container.
    pub container: Option<String>,
}

impl SymbolInformation {
    fn from_value(value: &Value) -> Option<Self> {
        let name = value.get("name")?.as_str()?.to_owned();
        let kind = value
            .get("kind")
            .and_then(Value::as_i64)
            .and_then(SymbolKind::from_code);
        let location = value.get("location")?;
        let uri = location.get("uri")?.as_str()?.to_owned();
        let range = location
            .get("range")
            .and_then(Range::from_value)
            .unwrap_or_default();
        let container = value
            .get("containerName")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        Some(Self {
            name,
            kind,
            uri,
            range,
            container,
        })
    }
}

/// Parse a `textDocument/hover` response.
#[must_use]
pub fn hover_from_value(value: &Value) -> Option<Hover> {
    Hover::from_value(value)
}

/// Normalize a `definition` or `references` response into zero or more locations.
#[must_use]
pub fn locations_from_value(value: &Value) -> Vec<Location> {
    match value {
        Value::Array(items) => items.iter().filter_map(Location::from_any).collect(),
        Value::Null => Vec::new(),
        other => Location::from_any(other).into_iter().collect(),
    }
}

/// Parse `textDocument/documentSymbol`, accepting hierarchical and flat server shapes.
#[must_use]
pub fn document_symbols_from_value(value: &Value) -> Vec<DocumentSymbol> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(DocumentSymbol::from_value)
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `workspace/symbol`, accepting normal `SymbolInformation` records and partial workspace
/// symbols with a `location.uri`.
#[must_use]
pub fn workspace_symbols_from_value(value: &Value) -> Vec<SymbolInformation> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(SymbolInformation::from_value)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn text_document_position_params(uri: &str, position: Position) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "position": { "line": position.line, "character": position.character },
    })
}

pub(crate) fn references_params(uri: &str, position: Position, include_declaration: bool) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "position": { "line": position.line, "character": position.character },
        "context": { "includeDeclaration": include_declaration },
    })
}

pub(crate) fn document_symbol_params(uri: &str) -> Value {
    json!({ "textDocument": { "uri": uri } })
}

pub(crate) fn workspace_symbol_params(query: &str) -> Value {
    json!({ "query": query })
}

/// Render hover content for tool output.
#[must_use]
pub fn hover_context_block(hover: Option<&Hover>) -> String {
    match hover {
        None => "No hover information.".to_owned(),
        Some(hover) => {
            let mut out = String::from("Hover");
            if let Some(range) = &hover.range {
                out.push_str(&format!(
                    " (line {}, character {})",
                    one_based(range.start.line),
                    one_based(range.start.character)
                ));
            }
            out.push_str(":\n");
            out.push_str(&hover.contents);
            if !hover.contents.ends_with('\n') {
                out.push('\n');
            }
            out
        }
    }
}

/// Render locations for tool output using 1-based line/character positions.
#[must_use]
pub fn locations_context_block(locations: &[Location]) -> String {
    if locations.is_empty() {
        return "No locations.".to_owned();
    }
    let mut out = format!("{} location(s):\n", locations.len());
    for location in locations {
        out.push_str(&format!(
            "  - {}:line {}:character {}\n",
            location.uri,
            one_based(location.range.start.line),
            one_based(location.range.start.character)
        ));
    }
    out
}

/// Render document symbols for tool output using 1-based line/character positions.
#[must_use]
pub fn document_symbols_context_block(symbols: &[DocumentSymbol]) -> String {
    if symbols.is_empty() {
        return "No document symbols.".to_owned();
    }
    let mut out = format!("{} document symbol(s):\n", symbols.len());
    push_document_symbols(symbols, 1, &mut out);
    out
}

fn push_document_symbols(symbols: &[DocumentSymbol], depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    for symbol in symbols {
        let kind = symbol.kind.map_or("symbol", SymbolKind::label);
        out.push_str(&format!(
            "{indent}- {kind} {} (line {}, character {})\n",
            symbol.name,
            one_based(symbol.range.start.line),
            one_based(symbol.range.start.character)
        ));
        if !symbol.children.is_empty() {
            push_document_symbols(&symbol.children, depth + 1, out);
        }
    }
}

/// Render workspace symbols for tool output using 1-based line/character positions.
#[must_use]
pub fn workspace_symbols_context_block(symbols: &[SymbolInformation]) -> String {
    if symbols.is_empty() {
        return "No workspace symbols.".to_owned();
    }
    let mut out = format!("{} workspace symbol(s):\n", symbols.len());
    for symbol in symbols {
        let kind = symbol.kind.map_or("symbol", SymbolKind::label);
        let container = symbol
            .container
            .as_deref()
            .map(|name| format!(" in {name}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  - {kind} {}{container} ({}:line {}:character {})\n",
            symbol.name,
            symbol.uri,
            one_based(symbol.range.start.line),
            one_based(symbol.range.start.character)
        ));
    }
    out
}

const fn one_based(zero_based: u32) -> u32 {
    zero_based.saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_and_location_link_shapes_normalize() {
        let value = json!([
            {
                "uri": "file:///a.rs",
                "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}}
            },
            {
                "targetUri": "file:///b.rs",
                "targetRange": {"start": {"line": 9, "character": 0}, "end": {"line": 10, "character": 1}},
                "targetSelectionRange": {"start": {"line": 9, "character": 4}, "end": {"line": 9, "character": 7}}
            }
        ]);
        let locations = locations_from_value(&value);
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[0].uri, "file:///a.rs");
        assert_eq!(locations[1].uri, "file:///b.rs");
        assert_eq!(locations[1].range.start.character, 4);
    }

    #[test]
    fn hover_markup_and_arrays_collapse_to_text() {
        let hover = hover_from_value(&json!({
            "contents": [
                {"language": "rust", "value": "fn add_one(input: i32) -> i32"},
                {"kind": "markdown", "value": "Adds one."}
            ],
            "range": {"start": {"line": 2, "character": 4}, "end": {"line": 2, "character": 11}}
        }))
        .expect("hover parses");
        assert!(hover.contents.contains("fn add_one"));
        assert!(hover.contents.contains("Adds one."));
        assert_eq!(hover.range.unwrap().start.line, 2);
    }

    #[test]
    fn symbol_shapes_parse_and_render_with_kinds() {
        let symbols = document_symbols_from_value(&json!([
            {
                "name": "Widget",
                "kind": 23,
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 4, "character": 1}},
                "children": [
                    {
                        "name": "new",
                        "kind": 6,
                        "range": {"start": {"line": 1, "character": 4}, "end": {"line": 3, "character": 5}}
                    }
                ]
            },
            {
                "name": "make",
                "kind": 12,
                "location": {
                    "uri": "file:///a.rs",
                    "range": {"start": {"line": 7, "character": 0}, "end": {"line": 9, "character": 1}}
                }
            }
        ]));
        assert_eq!(symbols.len(), 2);
        let rendered = document_symbols_context_block(&symbols);
        assert!(rendered.contains("struct Widget (line 1, character 1)"));
        assert!(rendered.contains("method new (line 2, character 5)"));
        assert!(rendered.contains("function make (line 8, character 1)"));

        let workspace = workspace_symbols_from_value(&json!([
            {
                "name": "Widget",
                "kind": 23,
                "location": {"uri": "file:///lib.rs"},
                "containerName": "probe"
            }
        ]));
        let rendered = workspace_symbols_context_block(&workspace);
        assert!(rendered.contains("struct Widget in probe"));
    }
}
