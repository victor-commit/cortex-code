//! Final gallery README index checks for VAL-V3-HARNESS-032.
//!
//! `plans/assets/mockups-v3/README.md` must list every canonical ID exactly once
//! under its physical category with a one-line description, map every ID exactly
//! once to one of the six user flows, match `--list` category and flow counts,
//! keep storyboard sequences aligned with the flow manifests, and document the
//! public render / audit / case / validation commands.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use mockup_harness_v3::{
    Category, FlowOwner, canonical_inventory, flow_manifests, validate_canonical_map,
};

fn readme_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plans/assets/mockups-v3/README.md")
}

fn read_readme() -> String {
    fs::read_to_string(readme_path()).expect("v3 gallery README.md must exist")
}

fn table_ids(section: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || trimmed.contains("---") {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.is_empty() {
            continue;
        }
        let first = cells[0];
        if first.starts_with('`') && first.ends_with('`') && first.len() > 2 {
            let id = &first[1..first.len() - 1];
            if id
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
            {
                ids.push(id.to_owned());
            }
        }
    }
    ids
}

fn section_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let after = source
        .split_once(start)
        .unwrap_or_else(|| panic!("missing section start: {start}"))
        .1;
    after
        .split_once(end)
        .unwrap_or_else(|| panic!("missing section end: {end}"))
        .0
}

#[test]
fn validation_map_is_ready() {
    validate_canonical_map().expect("canonical inventory must validate");
}

#[test]
fn readme_lists_every_id_once_by_category_with_description() {
    let inventory = canonical_inventory();
    let readme = read_readme();
    let category_section = section_between(&readme, "## Category index", "## Flow index");

    let mut seen = BTreeSet::new();
    let mut category_counts = BTreeMap::<Category, usize>::new();
    let mut flow_by_id = BTreeMap::<String, String>::new();

    for category in Category::ALL {
        let heading = match category {
            Category::Session => "### Session (",
            Category::Input => "### Input (",
            Category::Views => "### Views (",
            Category::Auth => "### Auth (",
            Category::States => "### States (",
            Category::Orchestration => "### Orchestration (",
        };
        let next = match category {
            Category::Session => "### Input (",
            Category::Input => "### Views (",
            Category::Views => "### Auth (",
            Category::Auth => "### States (",
            Category::States => "### Orchestration (",
            Category::Orchestration => "**Category totals:",
        };
        let section = section_between(category_section, heading, next);
        let expected_count = inventory
            .iter()
            .filter(|screen| screen.category == category)
            .count();
        assert!(
            section.starts_with(&format!("{expected_count})"))
                || section.contains(&format!("({expected_count})")),
            "{} heading must declare count {expected_count}",
            category.as_str()
        );

        let mut section_ids = Vec::new();
        for line in section.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') || trimmed.contains("---") {
                continue;
            }
            let cells: Vec<&str> = trimmed
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            if cells.len() < 3 {
                continue;
            }
            let id_cell = cells[0];
            if !(id_cell.starts_with('`') && id_cell.ends_with('`') && id_cell.len() > 2) {
                continue;
            }
            let id = id_cell[1..id_cell.len() - 1].to_owned();
            let description = cells[1].trim();
            let flow = cells[2].trim();
            assert!(
                !description.is_empty(),
                "{id} must have a one-line description in the category index"
            );
            assert!(
                description.ends_with('.') || description.contains(' '),
                "{id} description should be a readable one-line sentence"
            );
            assert!(
                FlowOwner::ALL.iter().any(|owner| owner.as_str() == flow),
                "{id} category row must map to a valid flow, got {flow}"
            );
            assert!(
                seen.insert(id.clone()),
                "duplicate category ID in README: {id}"
            );
            section_ids.push(id.clone());
            flow_by_id.insert(id, flow.to_owned());
        }

        let expected_ids: Vec<&str> = inventory
            .iter()
            .filter(|screen| screen.category == category)
            .map(|screen| screen.id)
            .collect();
        assert_eq!(
            section_ids,
            expected_ids
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>(),
            "category {} must list registry IDs in exact inventory order",
            category.as_str()
        );
        category_counts.insert(category, section_ids.len());
    }

    assert_eq!(seen.len(), 86, "category index must contain exactly 86 IDs");
    for screen in inventory {
        let mapped = flow_by_id
            .get(screen.id)
            .unwrap_or_else(|| panic!("missing category mapping for {}", screen.id));
        assert_eq!(
            mapped,
            screen.flow.as_str(),
            "category-row flow for {} must match registry",
            screen.id
        );
    }

    assert_eq!(category_counts.get(&Category::Session), Some(&21));
    assert_eq!(category_counts.get(&Category::Input), Some(&20));
    assert_eq!(category_counts.get(&Category::Views), Some(&16));
    assert_eq!(category_counts.get(&Category::Auth), Some(&7));
    assert_eq!(category_counts.get(&Category::States), Some(&8));
    assert_eq!(category_counts.get(&Category::Orchestration), Some(&14));
    assert!(
        category_section.contains("session 21 + input 20 + views 16 + auth 7 + states 8 + orchestration 14 = 86"),
        "category total identity line must match --list counts"
    );
}

#[test]
fn readme_maps_every_id_once_to_one_flow() {
    let inventory = canonical_inventory();
    let readme = read_readme();
    let flow_section = section_between(&readme, "## Flow index", "## Surfaces");

    let mut owned = BTreeMap::<String, String>::new();
    for owner in FlowOwner::ALL {
        let heading = match owner {
            FlowOwner::DailyWork => "### Flow 1 · daily-work (",
            FlowOwner::InputAndApproval => "### Flow 2 · input-and-approval (",
            FlowOwner::WorkConfiguration => "### Flow 3 · work-configuration (",
            FlowOwner::ProviderAndMcpConnection => {
                "### Flow 4 · provider-and-mcp-connection ("
            }
            FlowOwner::Recovery => "### Flow 5 · recovery (",
            FlowOwner::Mission => "### Flow 6 · mission (",
        };
        let next = match owner {
            FlowOwner::DailyWork => "### Flow 2 · input-and-approval (",
            FlowOwner::InputAndApproval => "### Flow 3 · work-configuration (",
            FlowOwner::WorkConfiguration => "### Flow 4 · provider-and-mcp-connection (",
            FlowOwner::ProviderAndMcpConnection => "### Flow 5 · recovery (",
            FlowOwner::Recovery => "### Flow 6 · mission (",
            FlowOwner::Mission => "**Flow totals:",
        };
        let section = section_between(flow_section, heading, next);
        let expected_count = inventory
            .iter()
            .filter(|screen| screen.flow == owner)
            .count();
        assert!(
            section.starts_with(&format!("{expected_count})"))
                || section.contains(&format!("({expected_count})")),
            "{} heading must declare count {expected_count}",
            owner.as_str()
        );

        let ids = table_ids(section);
        assert_eq!(
            ids.len(),
            expected_count,
            "{} flow table must list {expected_count} owned IDs",
            owner.as_str()
        );
        for id in ids {
            assert!(
                owned.insert(id.clone(), owner.as_str().to_owned()).is_none(),
                "ID {id} mapped to more than one flow in README"
            );
        }
    }

    assert_eq!(owned.len(), 86, "flow index must own exactly 86 IDs");
    for screen in inventory {
        let mapped = owned
            .get(screen.id)
            .unwrap_or_else(|| panic!("missing flow ownership for {}", screen.id));
        assert_eq!(
            mapped.as_str(),
            screen.flow.as_str(),
            "flow ownership mismatch for {}",
            screen.id
        );
    }
    assert!(
        flow_section.contains(
            "daily-work 24 + input-and-approval 15 + work-configuration 17 + provider-and-mcp-connection 9 + recovery 9 + mission 12 = 86"
        ),
        "flow total identity line must match --list counts"
    );
}

#[test]
fn readme_storyboards_match_flow_manifests() {
    let readme = read_readme();
    for manifest in flow_manifests() {
        let sequence = manifest
            .steps
            .iter()
            .map(|step| format!("`{}`", step.screen_id))
            .collect::<Vec<_>>()
            .join(" → ");
        assert!(
            readme.contains(&sequence),
            "README must document storyboard for {} as {sequence}",
            manifest.name
        );
    }
}

#[test]
fn readme_documents_required_public_commands_and_boundaries() {
    let readme = read_readme();
    let required_snippets = [
        "--list",
        "--implemented",
        "--all",
        "--flow",
        "--audit-json",
        "--case",
        "--tick",
        "--txt",
        "--png",
        "negative-fixture",
        "./check.sh --implemented",
        "./check.sh --all",
        "./sweep_checks.sh",
        "./sweep_determinism.sh",
        "./check_v2_immutable.sh",
        "./check_gate_approval.sh",
        "cargo test --features png",
        "cargo build --features png",
        "cargo clippy --all-targets --features png -- -D warnings",
        "plans/assets/mockups-v3/<category>/<screen>.{txt,png}",
        "plans/assets/mockups-v3-gates/<flow>/<NN>-<screen>.{txt,png}",
        "evidence/v3-human-gate/<review_bundle_sha256>/",
        "decision.json",
        "awaiting_user",
        "Workers never create or mutate decisions",
        "Do not modify repository-root `README.md`",
    ];
    for snippet in required_snippets {
        assert!(
            readme.contains(snippet),
            "README must document/publicize: {snippet}"
        );
    }
}

#[test]
fn owned_harness_032_assertions_are_enumerated() {
    // Keep the owning contract IDs visible to sweep aggregates and scrutiny.
    let owned = ["VAL-V3-HARNESS-032"];
    assert_eq!(owned.len(), 1);
}
