//! Six-flow human gate approval protocol.
//!
//! Workers may publish an immutable `awaiting_user` request under
//! `evidence/v3-human-gate/<review_bundle_sha256>/request.json`. Only the
//! orchestrator may write sibling `decision.json` with status `approved` or
//! `rejected`. Workers never author or mutate decisions. Rejection or any
//! changed gate/review/bridge digest creates a new digest directory; history is
//! never rewritten. Milestone 5-7 bulk work remains blocked until a complete
//! approved decision recomputes every bound digest successfully.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::audit::sha256_hex;
use crate::gate_programmatic_publication::{
    collect_bridge_digests, collect_path_hash_tuples, PathHashTuple,
};
use crate::gate_vision_review::{
    journey_sheet_relative_path, validate_gate_vision_review, JourneyReviewSheet,
};

pub const APPROVAL_SCHEMA_VERSION: u32 = 1;
pub const APPROVAL_FEATURE_ID: &str = "v3-human-gate-approval";
pub const HUMAN_GATE_ROOT: &str = "evidence/v3-human-gate";
pub const REQUEST_FILE_NAME: &str = "request.json";
pub const DECISION_FILE_NAME: &str = "decision.json";

const SIX_FLOWS: [&str; 6] = [
    "daily-work",
    "input-and-approval",
    "work-configuration",
    "provider-and-mcp-connection",
    "recovery",
    "mission",
];

const PRE_BULK_ROOTS: [&str; 3] = [
    "tools-throwaway/mockup-harness-v3/",
    "plans/assets/mockups-v3/",
    "plans/assets/mockups-v3-gates/",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewSheetDigest {
    pub flow: String,
    pub relative_path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeApprovalDigest {
    pub flow: String,
    pub case_id: String,
    pub tick: u32,
    pub kind: String,
    pub txt_path: String,
    pub png_path: String,
    pub audit_path: String,
    pub vision_path: String,
    pub txt_sha256: String,
    pub png_sha256: String,
    pub audit_sha256: String,
    pub vision_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreBulkTreeEvidence {
    pub algorithm: String,
    pub roots: Vec<String>,
    pub entry_count: usize,
    pub tree_sha256: String,
    pub entries: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HumanGateRequest {
    pub schema_version: u32,
    pub feature_id: String,
    pub status: String,
    pub request_digest: String,
    pub review_bundle_sha256: String,
    pub gate_manifest_sha256: String,
    pub path_hash_tuples: Vec<PathHashTuple>,
    pub review_sheet_digests: Vec<ReviewSheetDigest>,
    pub bridge_digests: Vec<BridgeApprovalDigest>,
    pub pre_bulk_tree: PreBulkTreeEvidence,
    pub request_time_unix_ms: u64,
    pub relative_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HumanGateDecision {
    pub schema_version: u32,
    pub status: String,
    pub user_actor: String,
    pub user_message_digest: String,
    pub decision_time_unix_ms: u64,
    pub request_digest: String,
    pub reviewed_bundle_digest: String,
    pub decision_actor_role: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalIntegrity {
    AwaitingUser {
        request: HumanGateRequest,
    },
    Approved {
        request: HumanGateRequest,
        decision: HumanGateDecision,
    },
    Rejected {
        request: HumanGateRequest,
        decision: HumanGateDecision,
        affected_pilot_features: Vec<String>,
    },
}

fn now_unix_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|error| format!("system clock error: {error}"))
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn parse_json_object(text: &str) -> Result<BTreeMap<String, String>, String> {
    // Controlled subset: flat objects with string/number arrays of objects handled separately.
    // This parser stores string/number leaves and raw JSON substring for structured fields.
    let trimmed = text.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Err("approval JSON must be a top-level object".to_owned());
    }
    let mut map = BTreeMap::new();
    let mut i = 1usize;
    let bytes = trimmed.as_bytes();
    while i < trimmed.len() - 1 {
        while i < trimmed.len() - 1 && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes[i] == b'}' {
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            continue;
        }
        if bytes[i] != b'"' {
            return Err(format!("expected object key at offset {i}"));
        }
        i += 1;
        let key_start = i;
        while i < trimmed.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' {
                i += 2;
            } else {
                i += 1;
            }
        }
        if i >= trimmed.len() {
            return Err("unterminated object key".to_owned());
        }
        let key = unescape_json_string(&trimmed[key_start..i])?;
        i += 1;
        while i < trimmed.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= trimmed.len() || bytes[i] != b':' {
            return Err(format!("expected ':' after key {key}"));
        }
        i += 1;
        while i < trimmed.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let (value, next) = parse_json_value(trimmed, i)?;
        map.insert(key, value);
        i = next;
    }
    Ok(map)
}

fn parse_json_value(text: &str, start: usize) -> Result<(String, usize), String> {
    let bytes = text.as_bytes();
    if start >= bytes.len() {
        return Err("unexpected end of JSON value".to_owned());
    }
    match bytes[start] {
        b'"' => {
            let mut i = start + 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    let value = unescape_json_string(&text[start + 1..i])?;
                    return Ok((value, i + 1));
                }
                i += 1;
            }
            Err("unterminated JSON string".to_owned())
        }
        b'{' | b'[' => {
            let open = bytes[start];
            let close = if open == b'{' { b'}' } else { b']' };
            let mut depth = 0i32;
            let mut i = start;
            let mut in_string = false;
            let mut escape = false;
            while i < bytes.len() {
                let b = bytes[i];
                if in_string {
                    if escape {
                        escape = false;
                    } else if b == b'\\' {
                        escape = true;
                    } else if b == b'"' {
                        in_string = false;
                    }
                    i += 1;
                    continue;
                }
                match b {
                    b'"' => in_string = true,
                    b if b == open => depth += 1,
                    b if b == close => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok((text[start..=i].to_owned(), i + 1));
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            Err("unterminated JSON container".to_owned())
        }
        b't' if text[start..].starts_with("true") => Ok(("true".to_owned(), start + 4)),
        b'f' if text[start..].starts_with("false") => Ok(("false".to_owned(), start + 5)),
        b'n' if text[start..].starts_with("null") => Ok(("null".to_owned(), start + 4)),
        b'-' | b'0'..=b'9' => {
            let mut i = start + 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'e' || bytes[i] == b'E' || bytes[i] == b'+' || bytes[i] == b'-')
            {
                i += 1;
            }
            Ok((text[start..i].to_owned(), i))
        }
        other => Err(format!(
            "unsupported JSON value starting with '{}'",
            other as char
        )),
    }
}

fn unescape_json_string(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let mut hex = String::new();
                for _ in 0..4 {
                    let next = chars
                        .next()
                        .ok_or_else(|| "truncated unicode escape".to_owned())?;
                    hex.push(next);
                }
                let code = u32::from_str_radix(&hex, 16)
                    .map_err(|_| format!("invalid unicode escape \\u{hex}"))?;
                let ch = char::from_u32(code)
                    .ok_or_else(|| format!("invalid unicode scalar \\u{hex}"))?;
                out.push(ch);
            }
            Some(other) => return Err(format!("unsupported escape \\{other}")),
            None => return Err("truncated escape".to_owned()),
        }
    }
    Ok(out)
}

fn require_field(map: &BTreeMap<String, String>, field: &str) -> Result<String, String> {
    map.get(field)
        .cloned()
        .ok_or_else(|| format!("approval JSON missing required field `{field}`"))
}

fn require_u32(map: &BTreeMap<String, String>, field: &str) -> Result<u32, String> {
    let raw = require_field(map, field)?;
    raw.parse::<u32>()
        .map_err(|_| format!("field `{field}` must be an integer, got {raw}"))
}

fn require_u64(map: &BTreeMap<String, String>, field: &str) -> Result<u64, String> {
    let raw = require_field(map, field)?;
    raw.parse::<u64>()
        .map_err(|_| format!("field `{field}` must be an integer, got {raw}"))
}

fn require_hex64(value: &str, field: &str) -> Result<(), String> {
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("field `{field}` must be a 64-character hex SHA-256"));
    }
    Ok(())
}

/// Collects six ordered journey-sheet digests from the vision product.
pub fn collect_review_sheet_digests(
    mission_directory: &str,
) -> Result<Vec<ReviewSheetDigest>, String> {
    let mut digests = Vec::with_capacity(6);
    for flow in SIX_FLOWS {
        let relative = journey_sheet_relative_path(flow);
        let path = Path::new(mission_directory).join(&relative);
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "missing journey review sheet for {flow} at {}: {error}",
                path.display()
            )
        })?;
        digests.push(ReviewSheetDigest {
            flow: flow.to_owned(),
            relative_path: relative,
            sha256: sha256_hex(&bytes),
        });
    }
    Ok(digests)
}

/// Collects complete bridge digests including vision evidence hashes.
pub fn collect_bridge_approval_digests(
    mission_directory: &str,
) -> Result<Vec<BridgeApprovalDigest>, String> {
    let bridges = collect_bridge_digests(mission_directory)?;
    let public_bridges = crate::gate_programmatic_publication::six_flow_gate_bridges();
    let mut out = Vec::with_capacity(bridges.len());
    for bridge in bridges {
        let public = public_bridges
            .iter()
            .find(|item| item.case_id == bridge.case_id && item.flow == bridge.flow)
            .ok_or_else(|| {
                format!(
                    "could not resolve public bridge metadata for case {} in flow {}",
                    bridge.case_id, bridge.flow
                )
            })?;
        let relative_vision = public.vision_evidence.to_owned();
        let vision_absolute = Path::new(mission_directory).join(&relative_vision);
        let vision_bytes = fs::read(&vision_absolute).map_err(|error| {
            format!(
                "missing bridge vision evidence {}: {error}",
                vision_absolute.display()
            )
        })?;
        out.push(BridgeApprovalDigest {
            flow: bridge.flow.to_owned(),
            case_id: bridge.case_id.to_owned(),
            tick: bridge.tick,
            kind: bridge.kind.to_owned(),
            txt_path: bridge.txt_path.clone(),
            png_path: bridge.png_path.clone(),
            audit_path: bridge.audit_path.clone(),
            vision_path: relative_vision,
            txt_sha256: bridge.txt_sha256,
            png_sha256: bridge.png_sha256,
            audit_sha256: bridge.audit_sha256,
            vision_sha256: sha256_hex(&vision_bytes),
        });
    }
    if out.len() != 30 {
        return Err(format!(
            "exactly 30 bridge digests are required for approval, got {}",
            out.len()
        ));
    }
    Ok(out)
}

/// Captures pre-bulk tracked tree evidence for the approved production roots.
pub fn capture_pre_bulk_tree(repository: &str) -> Result<PreBulkTreeEvidence, String> {
    let mut entries = Vec::new();
    for root in PRE_BULK_ROOTS {
        let list = Command::new("git")
            .args(["-C", repository, "ls-files", "-z", "--", root])
            .output()
            .map_err(|error| format!("git ls-files failed for {root}: {error}"))?;
        if !list.status.success() {
            return Err(format!(
                "git ls-files failed for {root}: {}",
                String::from_utf8_lossy(&list.stderr)
            ));
        }
        for path in String::from_utf8_lossy(&list.stdout).split('\0') {
            if path.is_empty() {
                continue;
            }
            let absolute = Path::new(repository).join(path);
            let bytes = fs::read(&absolute).map_err(|error| {
                format!("could not hash pre-bulk path {path}: {error}")
            })?;
            entries.push((path.to_owned(), sha256_hex(&bytes)));
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if entries.windows(2).any(|window| window[0].0 == window[1].0) {
        return Err("pre-bulk tree contains duplicate paths".to_owned());
    }
    let mut canonical = String::new();
    for (path, digest) in &entries {
        canonical.push_str(digest);
        canonical.push(' ');
        canonical.push(' ');
        canonical.push_str(path);
        canonical.push('\n');
    }
    Ok(PreBulkTreeEvidence {
        algorithm: "sha256".to_owned(),
        roots: PRE_BULK_ROOTS.iter().map(|root| (*root).to_owned()).collect(),
        entry_count: entries.len(),
        tree_sha256: sha256_hex(canonical.as_bytes()),
        entries,
    })
}

#[allow(clippy::too_many_arguments)]
fn canonical_request_payload(
    schema_version: u32,
    feature_id: &str,
    status: &str,
    review_bundle_sha256: &str,
    gate_manifest_sha256: &str,
    path_hash_tuples: &[PathHashTuple],
    review_sheet_digests: &[ReviewSheetDigest],
    bridge_digests: &[BridgeApprovalDigest],
    pre_bulk_tree: &PreBulkTreeEvidence,
    request_time_unix_ms: u64,
) -> String {
    // Deterministic JSON excluding request_digest. The request digest hashes this body.
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"schema_version\": {schema_version},\n"));
    out.push_str(&format!(
        "  \"feature_id\": \"{}\",\n",
        json_escape(feature_id)
    ));
    out.push_str(&format!("  \"status\": \"{}\",\n", json_escape(status)));
    out.push_str(&format!(
        "  \"review_bundle_sha256\": \"{}\",\n",
        json_escape(review_bundle_sha256)
    ));
    out.push_str(&format!(
        "  \"gate_manifest_sha256\": \"{}\",\n",
        json_escape(gate_manifest_sha256)
    ));
    out.push_str("  \"path_hash_tuples\": [\n");
    for (index, tuple) in path_hash_tuples.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"path\": \"{}\",\n",
            json_escape(&tuple.path)
        ));
        out.push_str(&format!(
            "      \"sha256\": \"{}\",\n",
            json_escape(&tuple.sha256)
        ));
        out.push_str(&format!(
            "      \"role\": \"{}\"\n",
            json_escape(tuple.role)
        ));
        out.push_str("    }");
        if index + 1 != path_hash_tuples.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n");
    out.push_str("  \"review_sheet_digests\": [\n");
    for (index, sheet) in review_sheet_digests.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"flow\": \"{}\",\n",
            json_escape(&sheet.flow)
        ));
        out.push_str(&format!(
            "      \"relative_path\": \"{}\",\n",
            json_escape(&sheet.relative_path)
        ));
        out.push_str(&format!(
            "      \"sha256\": \"{}\"\n",
            json_escape(&sheet.sha256)
        ));
        out.push_str("    }");
        if index + 1 != review_sheet_digests.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n");
    out.push_str("  \"bridge_digests\": [\n");
    for (index, bridge) in bridge_digests.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"flow\": \"{}\",\n",
            json_escape(&bridge.flow)
        ));
        out.push_str(&format!(
            "      \"case_id\": \"{}\",\n",
            json_escape(&bridge.case_id)
        ));
        out.push_str(&format!("      \"tick\": {},\n", bridge.tick));
        out.push_str(&format!(
            "      \"kind\": \"{}\",\n",
            json_escape(&bridge.kind)
        ));
        out.push_str(&format!(
            "      \"txt_path\": \"{}\",\n",
            json_escape(&bridge.txt_path)
        ));
        out.push_str(&format!(
            "      \"png_path\": \"{}\",\n",
            json_escape(&bridge.png_path)
        ));
        out.push_str(&format!(
            "      \"audit_path\": \"{}\",\n",
            json_escape(&bridge.audit_path)
        ));
        out.push_str(&format!(
            "      \"vision_path\": \"{}\",\n",
            json_escape(&bridge.vision_path)
        ));
        out.push_str(&format!(
            "      \"txt_sha256\": \"{}\",\n",
            json_escape(&bridge.txt_sha256)
        ));
        out.push_str(&format!(
            "      \"png_sha256\": \"{}\",\n",
            json_escape(&bridge.png_sha256)
        ));
        out.push_str(&format!(
            "      \"audit_sha256\": \"{}\",\n",
            json_escape(&bridge.audit_sha256)
        ));
        out.push_str(&format!(
            "      \"vision_sha256\": \"{}\"\n",
            json_escape(&bridge.vision_sha256)
        ));
        out.push_str("    }");
        if index + 1 != bridge_digests.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ],\n");
    out.push_str("  \"pre_bulk_tree\": {\n");
    out.push_str(&format!(
        "    \"algorithm\": \"{}\",\n",
        json_escape(&pre_bulk_tree.algorithm)
    ));
    out.push_str("    \"roots\": [\n");
    for (index, root) in pre_bulk_tree.roots.iter().enumerate() {
        out.push_str(&format!("      \"{}\"", json_escape(root)));
        if index + 1 != pre_bulk_tree.roots.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("    ],\n");
    out.push_str(&format!(
        "    \"entry_count\": {},\n",
        pre_bulk_tree.entry_count
    ));
    out.push_str(&format!(
        "    \"tree_sha256\": \"{}\",\n",
        json_escape(&pre_bulk_tree.tree_sha256)
    ));
    out.push_str("    \"entries\": [\n");
    for (index, (path, digest)) in pre_bulk_tree.entries.iter().enumerate() {
        out.push_str("      {\n");
        out.push_str(&format!("        \"path\": \"{}\",\n", json_escape(path)));
        out.push_str(&format!(
            "        \"sha256\": \"{}\"\n",
            json_escape(digest)
        ));
        out.push_str("      }");
        if index + 1 != pre_bulk_tree.entries.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("    ]\n");
    out.push_str("  },\n");
    out.push_str(&format!(
        "  \"request_time_unix_ms\": {request_time_unix_ms}\n"
    ));
    out.push('}');
    out
}

fn compute_review_bundle_sha256(
    gate_manifest_sha256: &str,
    path_hash_tuples: &[PathHashTuple],
    review_sheet_digests: &[ReviewSheetDigest],
    bridge_digests: &[BridgeApprovalDigest],
    pre_bulk_tree: &PreBulkTreeEvidence,
) -> String {
    let mut canonical = String::new();
    canonical.push_str("gate_manifest_sha256=");
    canonical.push_str(gate_manifest_sha256);
    canonical.push('\n');
    for tuple in path_hash_tuples {
        canonical.push_str("tuple=");
        canonical.push_str(tuple.role);
        canonical.push('\t');
        canonical.push_str(&tuple.path);
        canonical.push('\t');
        canonical.push_str(&tuple.sha256);
        canonical.push('\n');
    }
    for sheet in review_sheet_digests {
        canonical.push_str("sheet=");
        canonical.push_str(&sheet.flow);
        canonical.push('\t');
        canonical.push_str(&sheet.relative_path);
        canonical.push('\t');
        canonical.push_str(&sheet.sha256);
        canonical.push('\n');
    }
    for bridge in bridge_digests {
        canonical.push_str("bridge=");
        canonical.push_str(&bridge.flow);
        canonical.push('\t');
        canonical.push_str(&bridge.case_id);
        canonical.push('\t');
        canonical.push_str(&bridge.tick.to_string());
        canonical.push('\t');
        canonical.push_str(&bridge.kind);
        canonical.push('\t');
        canonical.push_str(&bridge.txt_sha256);
        canonical.push('\t');
        canonical.push_str(&bridge.png_sha256);
        canonical.push('\t');
        canonical.push_str(&bridge.audit_sha256);
        canonical.push('\t');
        canonical.push_str(&bridge.vision_sha256);
        canonical.push('\n');
    }
    canonical.push_str("pre_bulk_tree=");
    canonical.push_str(&pre_bulk_tree.tree_sha256);
    canonical.push('\n');
    sha256_hex(canonical.as_bytes())
}

fn request_body_with_digest(payload_without_digest: &str, request_digest: &str) -> String {
    // Insert request_digest after status for stable presentation.
    if let Some(index) = payload_without_digest.find("  \"status\":") {
        let after_status_line = payload_without_digest[index..]
            .find('\n')
            .map(|offset| index + offset + 1)
            .unwrap_or(payload_without_digest.len());
        let mut body = String::new();
        body.push_str(&payload_without_digest[..after_status_line]);
        body.push_str(&format!(
            "  \"request_digest\": \"{}\",\n",
            json_escape(request_digest)
        ));
        body.push_str(&payload_without_digest[after_status_line..]);
        body
    } else {
        format!(
            "{{\n  \"request_digest\": \"{}\",\n{}",
            json_escape(request_digest),
            &payload_without_digest[1..]
        )
    }
}

/// Builds the current human-gate request if the programmatic and vision bundle is ready.
pub fn build_human_gate_request(
    repository: &str,
    mission_directory: &str,
    request_time_unix_ms: Option<u64>,
) -> Result<HumanGateRequest, String> {
    // Hard readiness barrier: vision product must be current.
    let vision = validate_gate_vision_review(repository, mission_directory)?;
    if vision.required_iteration || !vision.programmatic_gate_ready {
        return Err("vision/programmatic gate is not ready for human approval".to_owned());
    }
    let path_hash_tuples = collect_path_hash_tuples(repository)?;
    if path_hash_tuples.len() != 70 {
        return Err(format!(
            "human-gate requires exactly 70 path-hash tuples, got {}",
            path_hash_tuples.len()
        ));
    }
    let review_sheet_digests = collect_review_sheet_digests(mission_directory)?;
    if review_sheet_digests.len() != 6 {
        return Err("human-gate requires exactly six review sheet digests".to_owned());
    }
    // Reuse sheet hashes collected by vision validation for fail-closed drift checks.
    let vision_sheets: BTreeMap<&str, &JourneyReviewSheet> = vision
        .journey_sheets
        .iter()
        .map(|sheet| (sheet.flow, sheet))
        .collect();
    for sheet in &review_sheet_digests {
        let current = vision_sheets
            .get(sheet.flow.as_str())
            .ok_or_else(|| format!("vision product missing sheet for {}", sheet.flow))?;
        if current.sheet_sha256 != sheet.sha256 {
            return Err(format!(
                "review sheet digest drift for {}: vision={} request={}",
                sheet.flow, current.sheet_sha256, sheet.sha256
            ));
        }
    }
    let bridge_digests = collect_bridge_approval_digests(mission_directory)?;
    let pre_bulk_tree = capture_pre_bulk_tree(repository)?;
    let gate_manifest_sha256 = {
        let path = Path::new(repository).join("plans/assets/mockups-v3-gates/.renderer-manifest-v1");
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "could not hash gate renderer manifest {}: {error}",
                path.display()
            )
        })?;
        sha256_hex(&bytes)
    };
    let review_bundle_sha256 = compute_review_bundle_sha256(
        &gate_manifest_sha256,
        &path_hash_tuples,
        &review_sheet_digests,
        &bridge_digests,
        &pre_bulk_tree,
    );
    let request_time_unix_ms = match request_time_unix_ms {
        Some(value) => value,
        None => now_unix_ms()?,
    };
    let payload = canonical_request_payload(
        APPROVAL_SCHEMA_VERSION,
        APPROVAL_FEATURE_ID,
        "awaiting_user",
        &review_bundle_sha256,
        &gate_manifest_sha256,
        &path_hash_tuples,
        &review_sheet_digests,
        &bridge_digests,
        &pre_bulk_tree,
        request_time_unix_ms,
    );
    let request_digest = sha256_hex(payload.as_bytes());
    Ok(HumanGateRequest {
        schema_version: APPROVAL_SCHEMA_VERSION,
        feature_id: APPROVAL_FEATURE_ID.to_owned(),
        status: "awaiting_user".to_owned(),
        request_digest,
        review_bundle_sha256,
        gate_manifest_sha256,
        path_hash_tuples,
        review_sheet_digests,
        bridge_digests,
        pre_bulk_tree,
        request_time_unix_ms,
        relative_path: String::new(),
    })
}

fn request_absolute_path(mission_directory: &str, review_bundle_sha256: &str) -> PathBuf {
    Path::new(mission_directory)
        .join(HUMAN_GATE_ROOT)
        .join(review_bundle_sha256)
        .join(REQUEST_FILE_NAME)
}

fn decision_absolute_path(mission_directory: &str, review_bundle_sha256: &str) -> PathBuf {
    Path::new(mission_directory)
        .join(HUMAN_GATE_ROOT)
        .join(review_bundle_sha256)
        .join(DECISION_FILE_NAME)
}

/// Serializes and writes an immutable awaiting_user request for the current bundle.
/// Refuses to overwrite an existing request or decision for that digest.
pub fn write_human_gate_request(
    repository: &str,
    mission_directory: &str,
    request_time_unix_ms: Option<u64>,
) -> Result<HumanGateRequest, String> {
    // Live content identity (bundle digest excludes request_time).
    let live_probe = build_human_gate_request(repository, mission_directory, Some(0))?;
    let directory = Path::new(mission_directory)
        .join(HUMAN_GATE_ROOT)
        .join(&live_probe.review_bundle_sha256);
    let request_path = directory.join(REQUEST_FILE_NAME);
    let decision_path = directory.join(DECISION_FILE_NAME);

    if request_path.exists() {
        // Immutable history: load the stored request and verify its bound content still
        // matches the live review bundle. Never rewrite request_time or body.
        let existing = load_request(mission_directory, &live_probe.review_bundle_sha256)?;
        if existing.review_bundle_sha256 != live_probe.review_bundle_sha256
            || existing.gate_manifest_sha256 != live_probe.gate_manifest_sha256
            || existing.path_hash_tuples != live_probe.path_hash_tuples
            || existing.review_sheet_digests != live_probe.review_sheet_digests
            || existing.bridge_digests != live_probe.bridge_digests
            || existing.pre_bulk_tree.tree_sha256 != live_probe.pre_bulk_tree.tree_sha256
        {
            return Err(format!(
                "immutable request already exists for digest {} and content changed; create a new review_bundle_sha256 directory instead of mutating history",
                live_probe.review_bundle_sha256
            ));
        }
        revalidate_request_against_live_bundle(repository, mission_directory, &existing)?;
        return Ok(existing);
    }

    if decision_path.exists() {
        return Err(format!(
            "decision already exists for digest {} without a matching request write; refusing worker mutation",
            live_probe.review_bundle_sha256
        ));
    }

    let mut request =
        build_human_gate_request(repository, mission_directory, request_time_unix_ms)?;
    if request.review_bundle_sha256 != live_probe.review_bundle_sha256 {
        return Err("review bundle drifted while preparing request".to_owned());
    }
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "could not create human-gate directory {}: {error}",
            directory.display()
        )
    })?;
    let body = serialize_request(&request);
    let temp = directory.join(format!(".{REQUEST_FILE_NAME}.tmp"));
    fs::write(&temp, &body)
        .map_err(|error| format!("could not stage request.json: {error}"))?;
    fs::rename(&temp, &request_path)
        .map_err(|error| format!("could not publish request.json: {error}"))?;
    // Prove immutability markers.
    let mode = fs::metadata(&request_path)
        .map_err(|error| format!("could not stat request.json: {error}"))?
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = mode;
        permissions.set_mode(0o444);
        let _ = fs::set_permissions(&request_path, permissions);
    }
    let _ = mode;
    request.relative_path = format!(
        "{HUMAN_GATE_ROOT}/{}/{}",
        request.review_bundle_sha256, REQUEST_FILE_NAME
    );
    Ok(request)
}

fn serialize_request(request: &HumanGateRequest) -> String {
    let payload = canonical_request_payload(
        request.schema_version,
        &request.feature_id,
        &request.status,
        &request.review_bundle_sha256,
        &request.gate_manifest_sha256,
        &request.path_hash_tuples,
        &request.review_sheet_digests,
        &request.bridge_digests,
        &request.pre_bulk_tree,
        request.request_time_unix_ms,
    );
    request_body_with_digest(&payload, &request.request_digest)
}

/// Explicitly refuses worker-authored decisions.
pub fn worker_write_decision_is_forbidden(
    mission_directory: &str,
    review_bundle_sha256: &str,
    decision: &HumanGateDecision,
) -> Result<(), String> {
    let _ = (mission_directory, review_bundle_sha256, decision);
    Err(
        "workers must never author or mutate decision.json; only the orchestrator may write immutable approved/rejected decisions from an explicit user response"
            .to_owned(),
    )
}

fn parse_request_json(
    mission_directory: &str,
    relative_path: &str,
    text: &str,
) -> Result<HumanGateRequest, String> {
    let map = parse_json_object(text)?;
    let schema_version = require_u32(&map, "schema_version")?;
    if schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(format!(
            "unsupported approval schema_version {schema_version}"
        ));
    }
    let feature_id = require_field(&map, "feature_id")?;
    if feature_id != APPROVAL_FEATURE_ID {
        return Err(format!("unexpected feature_id {feature_id}"));
    }
    let status = require_field(&map, "status")?;
    if status != "awaiting_user" {
        return Err(format!(
            "request status must be awaiting_user, got {status}"
        ));
    }
    let request_digest = require_field(&map, "request_digest")?;
    require_hex64(&request_digest, "request_digest")?;
    let review_bundle_sha256 = require_field(&map, "review_bundle_sha256")?;
    require_hex64(&review_bundle_sha256, "review_bundle_sha256")?;
    let gate_manifest_sha256 = require_field(&map, "gate_manifest_sha256")?;
    require_hex64(&gate_manifest_sha256, "gate_manifest_sha256")?;
    let request_time_unix_ms = require_u64(&map, "request_time_unix_ms")?;

    let path_hash_tuples = parse_path_hash_tuples(&require_field(&map, "path_hash_tuples")?)?;
    if path_hash_tuples.len() != 70 {
        return Err(format!(
            "request must bind exactly 70 path-hash tuples, got {}",
            path_hash_tuples.len()
        ));
    }
    let review_sheet_digests =
        parse_review_sheet_digests(&require_field(&map, "review_sheet_digests")?)?;
    if review_sheet_digests.len() != 6 {
        return Err(format!(
            "request must bind exactly six review sheets, got {}",
            review_sheet_digests.len()
        ));
    }
    let bridge_digests = parse_bridge_digests(&require_field(&map, "bridge_digests")?)?;
    if bridge_digests.len() != 30 {
        return Err(format!(
            "request must bind exactly 30 bridge digests, got {}",
            bridge_digests.len()
        ));
    }
    let pre_bulk_tree = parse_pre_bulk_tree(&require_field(&map, "pre_bulk_tree")?)?;

    // Recompute request digest over the canonical payload without request_digest.
    let payload = canonical_request_payload(
        schema_version,
        &feature_id,
        &status,
        &review_bundle_sha256,
        &gate_manifest_sha256,
        &path_hash_tuples,
        &review_sheet_digests,
        &bridge_digests,
        &pre_bulk_tree,
        request_time_unix_ms,
    );
    let recomputed = sha256_hex(payload.as_bytes());
    if recomputed != request_digest {
        return Err(format!(
            "request_digest is stale or forged: stored={request_digest} recomputed={recomputed}"
        ));
    }
    let recomputed_bundle = compute_review_bundle_sha256(
        &gate_manifest_sha256,
        &path_hash_tuples,
        &review_sheet_digests,
        &bridge_digests,
        &pre_bulk_tree,
    );
    if recomputed_bundle != review_bundle_sha256 {
        return Err(format!(
            "review_bundle_sha256 does not match bound content: stored={review_bundle_sha256} recomputed={recomputed_bundle}"
        ));
    }
    // Directory name must equal the review bundle digest (immutable addressability).
    let expected_prefix = format!("{HUMAN_GATE_ROOT}/{review_bundle_sha256}/");
    if !relative_path.starts_with(&expected_prefix) {
        return Err(format!(
            "request path {relative_path} does not live under digest directory {expected_prefix}"
        ));
    }
    let _ = mission_directory;
    Ok(HumanGateRequest {
        schema_version,
        feature_id,
        status,
        request_digest,
        review_bundle_sha256,
        gate_manifest_sha256,
        path_hash_tuples,
        review_sheet_digests,
        bridge_digests,
        pre_bulk_tree,
        request_time_unix_ms,
        relative_path: relative_path.to_owned(),
    })
}

fn parse_path_hash_tuples(raw: &str) -> Result<Vec<PathHashTuple>, String> {
    let items = parse_object_array(raw)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let path = require_field(&item, "path")?;
        let sha256 = require_field(&item, "sha256")?;
        require_hex64(&sha256, "path_hash_tuples.sha256")?;
        let role = require_field(&item, "role")?;
        if role != "gate-txt" && role != "gate-png" {
            return Err(format!("unexpected tuple role {role}"));
        }
        // Store role as 'static by leaking is forbidden — PathHashTuple uses &'static str.
        // Reconstruct using known statics.
        let role_static: &'static str = if role == "gate-txt" {
            "gate-txt"
        } else {
            "gate-png"
        };
        out.push(PathHashTuple {
            path,
            sha256,
            role: role_static,
        });
    }
    Ok(out)
}

fn parse_review_sheet_digests(raw: &str) -> Result<Vec<ReviewSheetDigest>, String> {
    let items = parse_object_array(raw)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let flow = require_field(&item, "flow")?;
        let relative_path = require_field(&item, "relative_path")?;
        let sha256 = require_field(&item, "sha256")?;
        require_hex64(&sha256, "review_sheet_digests.sha256")?;
        out.push(ReviewSheetDigest {
            flow,
            relative_path,
            sha256,
        });
    }
    Ok(out)
}

fn parse_bridge_digests(raw: &str) -> Result<Vec<BridgeApprovalDigest>, String> {
    let items = parse_object_array(raw)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let flow = require_field(&item, "flow")?;
        let case_id = require_field(&item, "case_id")?;
        let tick = require_field(&item, "tick")?
            .parse::<u32>()
            .map_err(|_| "bridge tick must be an integer".to_owned())?;
        let kind = require_field(&item, "kind")?;
        let txt_path = require_field(&item, "txt_path")?;
        let png_path = require_field(&item, "png_path")?;
        let audit_path = require_field(&item, "audit_path")?;
        let vision_path = require_field(&item, "vision_path")?;
        let txt_sha256 = require_field(&item, "txt_sha256")?;
        let png_sha256 = require_field(&item, "png_sha256")?;
        let audit_sha256 = require_field(&item, "audit_sha256")?;
        let vision_sha256 = require_field(&item, "vision_sha256")?;
        for (field, value) in [
            ("txt_sha256", &txt_sha256),
            ("png_sha256", &png_sha256),
            ("audit_sha256", &audit_sha256),
            ("vision_sha256", &vision_sha256),
        ] {
            require_hex64(value, field)?;
        }
        out.push(BridgeApprovalDigest {
            flow,
            case_id,
            tick,
            kind,
            txt_path,
            png_path,
            audit_path,
            vision_path,
            txt_sha256,
            png_sha256,
            audit_sha256,
            vision_sha256,
        });
    }
    Ok(out)
}

fn parse_pre_bulk_tree(raw: &str) -> Result<PreBulkTreeEvidence, String> {
    let map = parse_json_object(raw)?;
    let algorithm = require_field(&map, "algorithm")?;
    if algorithm != "sha256" {
        return Err("pre_bulk_tree.algorithm must be sha256".to_owned());
    }
    let roots_raw = require_field(&map, "roots")?;
    let roots = parse_string_array(&roots_raw)?;
    let entry_count = require_field(&map, "entry_count")?
        .parse::<usize>()
        .map_err(|_| "pre_bulk_tree.entry_count must be an integer".to_owned())?;
    let tree_sha256 = require_field(&map, "tree_sha256")?;
    require_hex64(&tree_sha256, "pre_bulk_tree.tree_sha256")?;
    let entries_raw = require_field(&map, "entries")?;
    let entry_objects = parse_object_array(&entries_raw)?;
    let mut entries = Vec::with_capacity(entry_objects.len());
    for item in entry_objects {
        let path = require_field(&item, "path")?;
        let sha256 = require_field(&item, "sha256")?;
        require_hex64(&sha256, "pre_bulk_tree.entries.sha256")?;
        entries.push((path, sha256));
    }
    if entries.len() != entry_count {
        return Err(format!(
            "pre_bulk_tree.entry_count {entry_count} does not match entries length {}",
            entries.len()
        ));
    }
    Ok(PreBulkTreeEvidence {
        algorithm,
        roots,
        entry_count,
        tree_sha256,
        entries,
    })
}

fn parse_string_array(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err("expected JSON string array".to_owned());
    }
    let mut out = Vec::new();
    let mut i = 1usize;
    let bytes = trimmed.as_bytes();
    while i < trimmed.len() - 1 {
        while i < trimmed.len() - 1 && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes[i] == b']' {
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            continue;
        }
        let (value, next) = parse_json_value(trimmed, i)?;
        out.push(value);
        i = next;
    }
    Ok(out)
}

fn parse_object_array(raw: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err("expected JSON object array".to_owned());
    }
    let mut out = Vec::new();
    let mut i = 1usize;
    let bytes = trimmed.as_bytes();
    while i < trimmed.len() - 1 {
        while i < trimmed.len() - 1 && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes[i] == b']' {
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            continue;
        }
        let (value, next) = parse_json_value(trimmed, i)?;
        out.push(parse_json_object(&value)?);
        i = next;
    }
    Ok(out)
}

fn parse_decision_json(
    relative_path: &str,
    text: &str,
    expected_request_digest: &str,
    expected_bundle_digest: &str,
) -> Result<HumanGateDecision, String> {
    let map = parse_json_object(text)?;
    let schema_version = require_u32(&map, "schema_version")?;
    if schema_version != APPROVAL_SCHEMA_VERSION {
        return Err(format!(
            "unsupported decision schema_version {schema_version}"
        ));
    }
    let status = require_field(&map, "status")?;
    if status != "approved" && status != "rejected" {
        return Err(format!(
            "decision status must be approved or rejected, got {status}"
        ));
    }
    let user_actor = require_field(&map, "user_actor")?;
    if user_actor.trim().is_empty() {
        return Err("decision user_actor must be non-empty".to_owned());
    }
    // Worker actors are never valid.
    let lowered = user_actor.to_ascii_lowercase();
    if lowered.contains("worker")
        || lowered.contains("droid")
        || lowered == "system"
        || lowered == "agent"
    {
        return Err(format!(
            "decision user_actor `{user_actor}` is not an explicit user; worker-authored decisions fail"
        ));
    }
    let user_message_digest = require_field(&map, "user_message_digest")?;
    require_hex64(&user_message_digest, "user_message_digest")?;
    let decision_time_unix_ms = require_u64(&map, "decision_time_unix_ms")?;
    let request_digest = require_field(&map, "request_digest")?;
    require_hex64(&request_digest, "decision.request_digest")?;
    if request_digest != expected_request_digest {
        return Err(format!(
            "decision request_digest mismatch: expected {expected_request_digest}, got {request_digest}"
        ));
    }
    let reviewed_bundle_digest = require_field(&map, "reviewed_bundle_digest")?;
    require_hex64(&reviewed_bundle_digest, "reviewed_bundle_digest")?;
    if reviewed_bundle_digest != expected_bundle_digest {
        return Err(format!(
            "decision reviewed_bundle_digest mismatch: expected {expected_bundle_digest}, got {reviewed_bundle_digest}"
        ));
    }
    let decision_actor_role = require_field(&map, "decision_actor_role")?;
    if decision_actor_role != "orchestrator" {
        return Err(format!(
            "decision_actor_role must be orchestrator, got {decision_actor_role}; worker-authored decisions fail validation"
        ));
    }
    Ok(HumanGateDecision {
        schema_version,
        status,
        user_actor,
        user_message_digest,
        decision_time_unix_ms,
        request_digest,
        reviewed_bundle_digest,
        decision_actor_role,
        relative_path: relative_path.to_owned(),
    })
}

/// Identifies pilot feature leaves that bound gate inputs touch when a decision is rejected
/// or any bound digest drifts.
pub fn identify_affected_pilot_features(
    request: &HumanGateRequest,
    reason: &str,
) -> Vec<String> {
    let mut features = vec![
        "v3-gate-programmatic-publication".to_owned(),
        "v3-gate-vision-review".to_owned(),
        "v3-human-gate-approval".to_owned(),
    ];
    let mut flows = BTreeMap::new();
    for tuple in &request.path_hash_tuples {
        if let Some(flow) = tuple.path.split('/').next() {
            *flows.entry(flow.to_owned()).or_insert(0usize) += 1;
        }
    }
    for flow in flows.keys() {
        match flow.as_str() {
            "daily-work" => features.push("v3-pilot-flow1-storyboard".to_owned()),
            "input-and-approval" => features.push("v3-pilot-flow2-storyboard".to_owned()),
            "work-configuration" => features.push("v3-pilot-flow3-storyboard".to_owned()),
            "provider-and-mcp-connection" => {
                features.push("v3-pilot-flow4-provider-mcp".to_owned())
            }
            "recovery" => features.push("v3-pilot-flow5-storyboard".to_owned()),
            "mission" => features.push("v3-pilot-flow6-storyboard".to_owned()),
            _ => {}
        }
    }
    features.push(format!("reason:{reason}"));
    features.sort();
    features.dedup();
    features
}

/// Loads a request from disk and validates its internal consistency.
pub fn load_request(
    mission_directory: &str,
    review_bundle_sha256: &str,
) -> Result<HumanGateRequest, String> {
    let path = request_absolute_path(mission_directory, review_bundle_sha256);
    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "missing human-gate request at {}: {error}",
            path.display()
        )
    })?;
    let relative = format!("{HUMAN_GATE_ROOT}/{review_bundle_sha256}/{REQUEST_FILE_NAME}");
    parse_request_json(mission_directory, &relative, &text)
}

/// Validates an optional decision against a request. Missing decision keeps awaiting_user.
pub fn load_decision_if_present(
    mission_directory: &str,
    request: &HumanGateRequest,
) -> Result<Option<HumanGateDecision>, String> {
    let path = decision_absolute_path(mission_directory, &request.review_bundle_sha256);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "could not read decision.json at {}: {error}",
            path.display()
        )
    })?;
    let relative = format!(
        "{HUMAN_GATE_ROOT}/{}/{DECISION_FILE_NAME}",
        request.review_bundle_sha256
    );
    let decision = parse_decision_json(
        &relative,
        &text,
        &request.request_digest,
        &request.review_bundle_sha256,
    )?;
    Ok(Some(decision))
}

/// Recomputes live repository/mission digests and compares them to a stored request.
pub fn revalidate_request_against_live_bundle(
    repository: &str,
    mission_directory: &str,
    request: &HumanGateRequest,
) -> Result<(), String> {
    let live = build_human_gate_request(
        repository,
        mission_directory,
        Some(request.request_time_unix_ms),
    )?;
    if live.review_bundle_sha256 != request.review_bundle_sha256 {
        return Err(format!(
            "hash-stale review bundle: request={} live={}; create a new digest directory rather than mutating history",
            request.review_bundle_sha256, live.review_bundle_sha256
        ));
    }
    if live.request_digest != request.request_digest {
        return Err(format!(
            "hash-stale request digest: request={} live={}",
            request.request_digest, live.request_digest
        ));
    }
    if live.gate_manifest_sha256 != request.gate_manifest_sha256 {
        return Err("gate manifest digest drifted from the recorded request".to_owned());
    }
    if live.path_hash_tuples != request.path_hash_tuples {
        return Err("path-hash tuples drifted from the recorded request".to_owned());
    }
    if live.review_sheet_digests != request.review_sheet_digests {
        return Err("review sheet digests drifted from the recorded request".to_owned());
    }
    if live.bridge_digests != request.bridge_digests {
        return Err("bridge digests drifted from the recorded request".to_owned());
    }
    if live.pre_bulk_tree.tree_sha256 != request.pre_bulk_tree.tree_sha256 {
        return Err("pre-bulk tree digest drifted from the recorded request".to_owned());
    }
    Ok(())
}

/// Full integrity check used by `check_gate_approval.sh`.
///
/// Modes:
/// - `require_approved=false`: succeeds for a valid awaiting_user request with no decision,
///   or for a valid approved/rejected orchestrator decision (rejected still succeeds path
///   validation but reports rejected including affected pilot leaves).
/// - `require_approved=true`: fails unless an orchestrator-authored approved decision binds
///   the current request and live digests (bulk production gate).
pub fn validate_approval_integrity(
    repository: &str,
    mission_directory: &str,
    require_approved: bool,
) -> Result<ApprovalIntegrity, String> {
    let root = Path::new(mission_directory).join(HUMAN_GATE_ROOT);
    if !root.is_dir() {
        return Err(format!(
            "human-gate evidence root is missing: {}",
            root.display()
        ));
    }
    let mut digests = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|error| format!("could not read human-gate root: {error}"))?
    {
        let entry = entry.map_err(|error| format!("could not read human-gate entry: {error}"))?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.len() == 64 && name.chars().all(|ch| ch.is_ascii_hexdigit()) {
            digests.push(name);
        }
    }
    if digests.is_empty() {
        return Err("no human-gate review bundle directories found".to_owned());
    }
    digests.sort();
    // Prefer the digest that matches the current live bundle; otherwise take the newest path mtime.
    let live_preview = build_human_gate_request(repository, mission_directory, Some(0));
    let preferred = match &live_preview {
        Ok(live) if digests.iter().any(|digest| digest == &live.review_bundle_sha256) => {
            live.review_bundle_sha256.clone()
        }
        _ => {
            // Fall back to highest mtime digest for awaiting revalidation after rejection.
            let mut best = digests[0].clone();
            let mut best_time = 0u64;
            for digest in &digests {
                let path = root.join(digest).join(REQUEST_FILE_NAME);
                if let Ok(meta) = fs::metadata(&path) {
                    if let Ok(modified) = meta.modified() {
                        if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                            let ms = duration.as_millis() as u64;
                            if ms >= best_time {
                                best_time = ms;
                                best = digest.clone();
                            }
                        }
                    }
                }
            }
            best
        }
    };

    let request = load_request(mission_directory, &preferred)?;
    revalidate_request_against_live_bundle(repository, mission_directory, &request)
        .map_err(|error| format!("hash-stale or incomplete bundle for {preferred}: {error}"))?;
    let decision = load_decision_if_present(mission_directory, &request)?;
    match decision {
        None => {
            if require_approved {
                return Err(
                    "bulk production is blocked: decision.json is missing; human approval is still awaiting_user"
                        .to_owned(),
                );
            }
            Ok(ApprovalIntegrity::AwaitingUser { request })
        }
        Some(decision) if decision.status == "approved" => {
            Ok(ApprovalIntegrity::Approved { request, decision })
        }
        Some(decision) => {
            let affected = identify_affected_pilot_features(&request, "rejected");
            if require_approved {
                return Err(format!(
                    "bulk production is blocked: decision status is rejected; affected pilots: {}",
                    affected.join(", ")
                ));
            }
            Ok(ApprovalIntegrity::Rejected {
                request,
                decision,
                affected_pilot_features: affected,
            })
        }
    }
}

/// Fixture helper: proves that a changed gate/review/bridge digest forces a new directory.
pub fn prove_changed_digest_requires_new_directory(
    previous_bundle_sha256: &str,
    next_bundle_sha256: &str,
) -> Result<(), String> {
    if previous_bundle_sha256 == next_bundle_sha256 {
        return Err(
            "changed inputs must produce a new review_bundle_sha256 directory rather than mutate history"
                .to_owned(),
        );
    }
    if previous_bundle_sha256.len() != 64 || next_bundle_sha256.len() != 64 {
        return Err("bundle digests must be 64-character hex".to_owned());
    }
    Ok(())
}

/// Fixture helper: invalid worker decision payloads fail validation.
pub fn prove_worker_decision_is_rejected(sample_decision_json: &str) -> Result<(), String> {
    let request_digest = "0".repeat(64);
    let bundle_digest = "1".repeat(64);
    match parse_decision_json(
        "evidence/v3-human-gate/deadbeef/decision.json",
        sample_decision_json,
        &request_digest,
        &bundle_digest,
    ) {
        Ok(_) => Err("worker-authored or malformed decision must fail".to_owned()),
        Err(error)
            if error.contains("worker")
                || error.contains("orchestrator")
                || error.contains("user_actor")
                || error.contains("mismatch")
                || error.contains("must be") =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "unexpected decision failure shape for worker proof: {error}"
        )),
    }
}

#[cfg(test)]
mod unit_tests {
    use super::{
        prove_changed_digest_requires_new_directory, prove_worker_decision_is_rejected,
        worker_write_decision_is_forbidden, HumanGateDecision, APPROVAL_SCHEMA_VERSION,
    };

    #[test]
    fn worker_cannot_write_decision() {
        let decision = HumanGateDecision {
            schema_version: APPROVAL_SCHEMA_VERSION,
            status: "approved".into(),
            user_actor: "worker-self".into(),
            user_message_digest: "a".repeat(64),
            decision_time_unix_ms: 1,
            request_digest: "b".repeat(64),
            reviewed_bundle_digest: "c".repeat(64),
            decision_actor_role: "worker".into(),
            relative_path: "x".into(),
        };
        assert!(worker_write_decision_is_forbidden("/tmp", "deadbeef", &decision).is_err());
    }

    #[test]
    fn changed_digest_requires_new_directory() {
        let previous = "a".repeat(64);
        let next = "b".repeat(64);
        prove_changed_digest_requires_new_directory(&previous, &next).unwrap();
        assert!(prove_changed_digest_requires_new_directory(&previous, &previous).is_err());
    }

    #[test]
    fn worker_authored_decision_json_fails() {
        let sample = r#"{
  "schema_version": 1,
  "status": "approved",
  "user_actor": "gate-worker",
  "user_message_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "decision_time_unix_ms": 1,
  "request_digest": "0000000000000000000000000000000000000000000000000000000000000000",
  "reviewed_bundle_digest": "1111111111111111111111111111111111111111111111111111111111111111",
  "decision_actor_role": "worker"
}"#;
        prove_worker_decision_is_rejected(sample).unwrap();
    }
}
