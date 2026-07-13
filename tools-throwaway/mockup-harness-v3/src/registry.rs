//! Stable canonical state inventory for the isolated v3 harness.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Category {
    Session,
    Input,
    Views,
    Auth,
    States,
    Orchestration,
}

impl Category {
    pub const ALL: [Self; 6] = [
        Self::Session,
        Self::Input,
        Self::Views,
        Self::Auth,
        Self::States,
        Self::Orchestration,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Input => "input",
            Self::Views => "views",
            Self::Auth => "auth",
            Self::States => "states",
            Self::Orchestration => "orchestration",
        }
    }

    const fn expected_count(self) -> usize {
        match self {
            Self::Session => 21,
            Self::Input => 20,
            Self::Views => 16,
            Self::Auth => 7,
            Self::States => 8,
            Self::Orchestration => 14,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FlowOwner {
    DailyWork,
    InputAndApproval,
    WorkConfiguration,
    ProviderAndMcpConnection,
    Recovery,
    Mission,
}

impl FlowOwner {
    pub const ALL: [Self; 6] = [
        Self::DailyWork,
        Self::InputAndApproval,
        Self::WorkConfiguration,
        Self::ProviderAndMcpConnection,
        Self::Recovery,
        Self::Mission,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DailyWork => "daily-work",
            Self::InputAndApproval => "input-and-approval",
            Self::WorkConfiguration => "work-configuration",
            Self::ProviderAndMcpConnection => "provider-and-mcp-connection",
            Self::Recovery => "recovery",
            Self::Mission => "mission",
        }
    }

    const fn expected_count(self) -> usize {
        match self {
            Self::DailyWork => 24,
            Self::InputAndApproval => 15,
            Self::WorkConfiguration => 17,
            Self::ProviderAndMcpConnection => 9,
            Self::Recovery => 9,
            Self::Mission => 12,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    ConversationShell,
    ActionDock,
    ContextualInspector,
    TemporaryDetailView,
}

impl Surface {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConversationShell => "conversation-shell",
            Self::ActionDock => "action-dock",
            Self::ContextualInspector => "contextual-inspector",
            Self::TemporaryDetailView => "temporary-detail-view",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImplementationStatus {
    Implemented,
    Unimplemented,
}

impl ImplementationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Unimplemented => "unimplemented",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenSpec {
    pub id: &'static str,
    pub category: Category,
    pub flow: FlowOwner,
    pub surface: Surface,
    pub status: ImplementationStatus,
}

const IMPLEMENTED: ImplementationStatus = ImplementationStatus::Implemented;
const UNIMPLEMENTED: ImplementationStatus = ImplementationStatus::Unimplemented;

const CANONICAL_INVENTORY: [ScreenSpec; 86] = [
    ScreenSpec {
        id: "home",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "onboarding",
        category: Category::Session,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "chat-turn",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "chat-long-scroll",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        // Compact history restore plus earlier/latest long-history navigation.
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "streaming",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "reasoning",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "reasoning-collapsed",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "rich-response",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "tool-calls",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "tool-running",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "tool-success",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "tool-success-expanded",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "tool-failed",
        category: Category::Session,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "tool-task",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "tool-task-expanded",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "diff-edit",
        category: Category::Session,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "shell",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "blocked-command",
        category: Category::Session,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "turn-error",
        category: Category::Session,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "todo-list",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "compact",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "slash-menu",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "agent-picker",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "command-palette",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "command-palette-submenu",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "model-picker",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "attachment-picker",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "model-picker-filtered",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "file-mention",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "export-picker",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "fork-picker",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "queue-manager",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "composer-multiline",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "skills-picker",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "session-picker",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "subagent-picker",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "variant-picker",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "theme-picker",
        category: Category::Input,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "permission-shell",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "permission-file-edit",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "permission-deny-reason",
        category: Category::Input,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "status",
        category: Category::Views,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "timeline",
        category: Category::Views,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::TemporaryDetailView,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "mcp",
        category: Category::Views,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::TemporaryDetailView,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "diff-viewer",
        category: Category::Views,
        flow: FlowOwner::DailyWork,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "diff-viewer-staging",
        category: Category::Views,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::TemporaryDetailView,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "help",
        category: Category::Views,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "review",
        category: Category::Views,
        flow: FlowOwner::DailyWork,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "theme-gallery-dark",
        category: Category::Views,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "theme-gallery-light",
        category: Category::Views,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "sidebar-context",
        category: Category::Views,
        flow: FlowOwner::DailyWork,
        surface: Surface::ContextualInspector,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "sidebar-files",
        category: Category::Views,
        flow: FlowOwner::DailyWork,
        surface: Surface::ContextualInspector,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "sidebar-lsp",
        category: Category::Views,
        flow: FlowOwner::DailyWork,
        surface: Surface::ContextualInspector,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "sidebar-todo",
        category: Category::Views,
        flow: FlowOwner::DailyWork,
        surface: Surface::ContextualInspector,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "sidebar-subagents",
        category: Category::Views,
        flow: FlowOwner::DailyWork,
        surface: Surface::ContextualInspector,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "sidebar-mcp",
        category: Category::Views,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::ContextualInspector,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "sidebar-queue",
        category: Category::Views,
        flow: FlowOwner::InputAndApproval,
        surface: Surface::ContextualInspector,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "login",
        category: Category::Auth,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "connect-provider",
        category: Category::Auth,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "connect-method",
        category: Category::Auth,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "connect-apikey",
        category: Category::Auth,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::ActionDock,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "connect-oauth",
        category: Category::Auth,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "connect-connected",
        category: Category::Auth,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "connect-disconnect",
        category: Category::Auth,
        flow: FlowOwner::ProviderAndMcpConnection,
        surface: Surface::ActionDock,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "empty",
        category: Category::States,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ConversationShell,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "error-network",
        category: Category::States,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "error-context",
        category: Category::States,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "rate-limit",
        category: Category::States,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "interrupt",
        category: Category::States,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "toasts",
        category: Category::States,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "checkpoint-undo",
        category: Category::States,
        flow: FlowOwner::Recovery,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "share",
        category: Category::States,
        flow: FlowOwner::DailyWork,
        surface: Surface::ActionDock,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "mode-switch",
        category: Category::Orchestration,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "mode-switch-refused",
        category: Category::Orchestration,
        flow: FlowOwner::WorkConfiguration,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "workflow-launch",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "workflow-launch-blocked",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "workflow-monitor",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::ConversationShell,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "workflow-monitor-failure",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "team-board",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "reviewer-output",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::TemporaryDetailView,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "reviewer-approved",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::ActionDock,
        status: IMPLEMENTED,
    },
    ScreenSpec {
        id: "verification-ladder",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "intent-scorer-pass",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "intent-scorer-fail",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "concurrency-metrics",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
    ScreenSpec {
        id: "tool-envelope",
        category: Category::Orchestration,
        flow: FlowOwner::Mission,
        surface: Surface::TemporaryDetailView,
        status: UNIMPLEMENTED,
    },
];

/// Returns the stable, ordered 86-state inventory. The order is public CLI order.
pub fn canonical_inventory() -> &'static [ScreenSpec] {
    &CANONICAL_INVENTORY
}

/// Rejects a registry whose IDs, category counts, or journey ownership drift from
/// the canonical v3 contract.
pub fn validate_inventory(inventory: &[ScreenSpec]) -> Result<(), String> {
    let expected_ids = canonical_inventory()
        .iter()
        .map(|screen| screen.id)
        .collect::<BTreeSet<_>>();
    let expected_screens = canonical_inventory()
        .iter()
        .map(|screen| (screen.id, screen))
        .collect::<BTreeMap<_, _>>();
    let mut flow_owners = BTreeMap::<&str, BTreeSet<FlowOwner>>::new();

    for screen in inventory {
        validate_surface_labels(&[screen.surface.as_str()])?;
        validate_alias(screen.id, screen.id)?;
        flow_owners
            .entry(screen.id)
            .or_default()
            .insert(screen.flow);
    }
    for (id, owners) in &flow_owners {
        if owners.len() > 1 {
            return Err(format!("multiple flow owners: {id}"));
        }
    }

    let mut actual_ids = BTreeSet::new();
    for screen in inventory {
        if !actual_ids.insert(screen.id) {
            return Err(format!("duplicate canonical ID: {}", screen.id));
        }
        let Some(expected) = expected_screens.get(screen.id) else {
            return Err(format!("extra canonical ID: {}", screen.id));
        };
        if screen.category != expected.category {
            return Err(format!("category mapping mismatch for {}", screen.id));
        }
        if screen.flow != expected.flow {
            return Err(format!("flow ownership mapping mismatch for {}", screen.id));
        }
        if screen.surface != expected.surface {
            return Err(format!("surface mapping mismatch for {}", screen.id));
        }
    }
    for id in &expected_ids {
        if !actual_ids.contains(id) {
            return Err(format!("missing canonical ID: {id}"));
        }
    }

    for category in Category::ALL {
        let actual_count = inventory
            .iter()
            .filter(|screen| screen.category == category)
            .count();
        if actual_count != category.expected_count() {
            return Err(format!(
                "category count mismatch for {}: expected {}, got {actual_count}",
                category.as_str(),
                category.expected_count(),
            ));
        }
    }
    for flow in FlowOwner::ALL {
        let actual_count = inventory
            .iter()
            .filter(|screen| screen.flow == flow)
            .count();
        if actual_count != flow.expected_count() {
            return Err(format!(
                "flow ownership count mismatch for {}: expected {}, got {actual_count}",
                flow.as_str(),
                flow.expected_count(),
            ));
        }
    }

    Ok(())
}

/// Rejects absent, duplicate, or unsupported names for the four declared
/// surface arrangements before inventory evidence can be accepted.
pub fn validate_surface_labels(labels: &[&str]) -> Result<(), String> {
    if labels.is_empty() {
        return Err("missing surface".to_owned());
    }
    let allowed = [
        "conversation-shell",
        "action-dock",
        "contextual-inspector",
        "temporary-detail-view",
    ];
    let mut seen = BTreeSet::new();
    for label in labels {
        if !allowed.contains(label) {
            return Err(format!("unknown surface: {label}"));
        }
        if !seen.insert(*label) {
            return Err(format!("duplicate surface: {label}"));
        }
    }
    Ok(())
}

/// Proves a public identity did not require case or punctuation rewriting to
/// match its canonical registry representation.
pub fn validate_alias(actual: &str, expected: &str) -> Result<(), String> {
    if actual != expected {
        return Err(format!("alias normalization mismatch: {actual}"));
    }
    Ok(())
}
