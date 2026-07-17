//! Shared Flow 3 picker selection contract.
//!
//! All nine named configuration pickers use one selection model: stable
//! identities, bottom-dock anatomy, exclusive selected detail, non-color focus
//! and current markers, local keys, exact cancel/apply origins, and a complete
//! filter matrix (or an explicit non-filter declaration for small lists).

use std::collections::BTreeMap;

/// Canonical Flow 3 configuration pickers covered by VAL-V3-F3-005..011/051.
pub const NINE_PICKERS: [&str; 9] = [
    "agent-picker",
    "model-picker",
    "model-picker-filtered",
    "fork-picker",
    "skills-picker",
    "session-picker",
    "subagent-picker",
    "variant-picker",
    "theme-picker",
];

/// Whether a picker exposes the shared filter contract or declares why not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterCapability {
    /// Large/catalog pickers implement query, no-results, clear, apply, cancel.
    Filterable,
    /// Short catalogs declare in copy and audit that filtering is unnecessary.
    NonFilterable,
}

/// Shared selection propaganda for one picker catalog entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PickerContract {
    pub screen_id: &'static str,
    pub title: &'static str,
    pub stable_identity: &'static str,
    pub filter: FilterCapability,
    /// Human reason rendered when `filter == NonFilterable`.
    pub non_filter_reason: Option<&'static str>,
    /// Public cancel scenarios for exact origin restore (may be empty for pure
    /// anatomy fixtures covered by sibling cancel cases).
    pub cancel_case: Option<&'static str>,
    /// Public apply (or nequivalent primary commit) scenario.
    pub apply_case: Option<&'static str>,
    /// Named cases that must exist for a filterable picker.
    pub filter_matrix: &'static [&'static str],
    pub row_family: &'static str,
}

const EMPTY_FILTER: &[&str] = &[];
const MODEL_FILTER_MATRIX: &[&str] = &[
    "model-filter-apply",
    "model-filter-no-match",
    "model-filter-clear",
    "model-filter-cancel",
    "model-filtered-apply",
];
const SESSION_FILTER_MATRIX: &[&str] = &[
    "session-no-matches-filter",
    "session-no-matches",
    "session-resume-latest",
    "session-preview-back",
    "session-picker-cancel",
];

/// Pure Cortex dump surface titles for the nine Flow 3 pickers.
///
/// Hybrid copper titles stay in [`picker_contracts`]; pure gallery paint from
/// harness-v4 uses these short origin/prompt-aligned labels (VAL residual
/// `public_publication_covers_all_nine_picker_txt_artifacts`).
pub fn pure_picker_title(screen_id: &str) -> Option<&'static str> {
    // Titles must match pure-structure slash_dropdown /live-prompt paint from
    // harness-v4 screens/pickers.rs (M3 STRUCT-011 / CROSS-027). Prefer the
    // exact `❯ …` origin line that ships under plans/assets/mockups-v3/input/.
    Some(match screen_id {
        "agent-picker" => "Pick how Cortex should approach this task.",
        "model-picker" => "Switch model for the next reply only when needed.",
        "model-picker-filtered" => "Narrow the model list with a typed prefix.",
        "fork-picker" => "Fork from this turn into a new branch of the chat.",
        "skills-picker" => "Enable skills for this session.",
        "session-picker" => "Resume a previous conversation.",
        "subagent-picker" => "Spawn a task-local helper for this turn.",
        "variant-picker" => "Pick reasoning effort for the next reply.",
        "theme-picker" => "Preview a theme without leaving this origin.",
        _ => return None,
    })
}

/// Authoritative shared catalog for the nine named pickers.
pub fn picker_contracts() -> BTreeMap<&'static str, PickerContract> {
    let entries = [
        PickerContract {
            screen_id: "agent-picker",
            title: "How Code should work",
            stable_identity: "picker.work-style",
            filter: FilterCapability::NonFilterable,
            non_filter_reason: Some(
                "No filter · three work-style choices fit on one screen",
            ),
            cancel_case: Some("agent-picker-cancel"),
            apply_case: Some("work-style-unchanged"),
            filter_matrix: EMPTY_FILTER,
            row_family: "work-style-row",
        },
        PickerContract {
            screen_id: "model-picker",
            title: "Choose a model",
            stable_identity: "picker.model-selection",
            filter: FilterCapability::Filterable,
            non_filter_reason: None,
            cancel_case: Some("model-picker-cancel"),
            apply_case: Some("model-picker-apply"),
            filter_matrix: MODEL_FILTER_MATRIX,
            row_family: "model-row",
        },
        PickerContract {
            screen_id: "model-picker-filtered",
            title: "Choose a model",
            stable_identity: "picker.model-selection",
            filter: FilterCapability::Filterable,
            non_filter_reason: None,
            cancel_case: Some("model-filter-cancel"),
            apply_case: Some("model-filtered-apply"),
            filter_matrix: MODEL_FILTER_MATRIX,
            row_family: "model-row",
        },
        PickerContract {
            screen_id: "fork-picker",
            title: "Fork conversation",
            stable_identity: "picker.fork-confirmation",
            filter: FilterCapability::NonFilterable,
            non_filter_reason: Some(
                "No filter · source, checkpoint, and destination are one decision",
            ),
            cancel_case: Some("fork-cancel"),
            apply_case: Some("fork-confirm"),
            filter_matrix: EMPTY_FILTER,
            row_family: "fork-section",
        },
        PickerContract {
            screen_id: "skills-picker",
            title: "Choose session skills",
            stable_identity: "picker.skills-selection",
            filter: FilterCapability::NonFilterable,
            non_filter_reason: Some(
                "No filter · four session skills fit on one screen",
            ),
            cancel_case: Some("skills-cancel"),
            apply_case: Some("skills-apply"),
            filter_matrix: EMPTY_FILTER,
            row_family: "skill-row",
        },
        PickerContract {
            screen_id: "session-picker",
            title: "Resume a conversation",
            stable_identity: "picker.saved-conversations",
            filter: FilterCapability::Filterable,
            non_filter_reason: None,
            cancel_case: Some("session-picker-cancel"),
            apply_case: Some("session-resume-latest"),
            filter_matrix: SESSION_FILTER_MATRIX,
            row_family: "session-row",
        },
        PickerContract {
            screen_id: "subagent-picker",
            title: "Delegate this task",
            stable_identity: "picker.subagent-selection",
            filter: FilterCapability::NonFilterable,
            non_filter_reason: Some(
                "No filter · three task helpers fit on one screen",
            ),
            cancel_case: Some("subagent-cancel"),
            apply_case: Some("subagent-apply"),
            filter_matrix: EMPTY_FILTER,
            row_family: "subagent-row",
        },
        PickerContract {
            screen_id: "variant-picker",
            title: "Choose a response variant",
            stable_identity: "picker.variant-selection",
            filter: FilterCapability::NonFilterable,
            non_filter_reason: Some(
                "No filter · three response variants fit on one screen",
            ),
            cancel_case: Some("variant-cancel"),
            apply_case: Some("variant-apply"),
            filter_matrix: EMPTY_FILTER,
            row_family: "variant-row",
        },
        PickerContract {
            screen_id: "theme-picker",
            title: "Choose a theme",
            stable_identity: "picker.theme-selection",
            filter: FilterCapability::NonFilterable,
            non_filter_reason: Some(
                "No filter · four themes fit on one screen",
            ),
            cancel_case: Some("theme-cancel"),
            apply_case: Some("theme-apply"),
            filter_matrix: EMPTY_FILTER,
            row_family: "theme-row",
        },
    ];
    entries
        .into_iter()
        .map(|entry| (entry.screen_id, entry))
        .collect()
}

/// Declared fixture identity used for contract matrix anatomy renders.
pub fn contract_fixture(screen_id: &str) -> &'static str {
    match screen_id {
        "agent-picker" => "flow3-agent-picker",
        "model-picker" => "flow3-model-picker",
        "model-picker-filtered" => "flow3-model-picker-filtered",
        "fork-picker" => "flow3-fork-picker",
        "skills-picker" => "flow3-skills-picker",
        "session-picker" => "flow3-session-picker",
        "subagent-picker" => "flow3-subagent-picker",
        "variant-picker" => "flow3-variant-picker",
        "theme-picker" => "flow3-theme-picker",
        _ => "flow3-picker-contract",
    }
}

/// Human non-filter declaration text for a catalog entry.
pub fn non_filter_declaration(screen_id: &str) -> Option<&'static str> {
    picker_contracts()
        .get(screen_id)
        .and_then(|contract| contract.non_filter_reason)
}

/// Stripped raw-ID tokens forbidden on default picker surfaces.
pub const FORBIDDEN_RAW_ID_SNIPPETS: &[&str] = &[
    "anthropic/",
    "openai/",
    "session.",
    "model-",
    "agent_id",
    "skill_id",
    "mode_switch",
    "ctx ",
    "$0.",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_named_pickers_are_catalogued_exactly_once() {
        let catalog = picker_contracts();
        assert_eq!(catalog.len(), 9);
        for id in NINE_PICKERS {
            assert!(catalog.contains_key(id), "missing catalog entry for {id}");
        }
    }

    #[test]
    fn filterable_pickers_declare_complete_matrix_cases() {
        let catalog = picker_contracts();
        for id in ["model-picker", "model-picker-filtered", "session-picker"] {
            let entry = catalog.get(id).expect("catalog entry");
            assert_eq!(entry.filter, FilterCapability::Filterable);
            assert!(!entry.filter_matrix.is_empty());
            assert!(entry.non_filter_reason.is_none());
        }
    }

    #[test]
    fn small_pickers_declare_non_filterability() {
        let catalog = picker_contracts();
        for id in [
            "agent-picker",
            "skills-picker",
            "subagent-picker",
            "variant-picker",
            "theme-picker",
            "fork-picker",
        ] {
            let entry = catalog.get(id).expect("catalog entry");
            assert_eq!(entry.filter, FilterCapability::NonFilterable);
            assert!(entry.non_filter_reason.is_some());
            assert!(entry.filter_matrix.is_empty());
        }
    }
}
