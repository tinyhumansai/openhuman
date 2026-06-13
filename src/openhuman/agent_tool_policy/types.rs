use crate::openhuman::tools::PermissionLevel;
use std::collections::{BTreeSet, HashMap, HashSet};

const NO_TOOLS_ALLOWED_SENTINEL: &str = "__openhuman_no_policy_allowed_tools__";

/// Coarse task risk derived from the highest permission level allowed for the
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl TaskRiskLevel {
    pub fn from_allowed_permission(permission: PermissionLevel) -> Self {
        match permission {
            PermissionLevel::None | PermissionLevel::ReadOnly => Self::Low,
            PermissionLevel::Write => Self::Medium,
            PermissionLevel::Execute => Self::High,
            PermissionLevel::Dangerous => Self::Critical,
        }
    }
}

impl std::fmt::Display for TaskRiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

/// Resolved task profile for one agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskProfile {
    pub agent_id: String,
    pub channel: String,
    pub entrypoint: String,
    pub risk_level: TaskRiskLevel,
    pub allowed_permission: PermissionLevel,
}

/// Policy action for a tool in the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicyAction {
    Allow,
    RequireApproval,
    Deny,
    HideFromPrompt,
}

/// Deterministic decision for a specific tool name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyDecision {
    pub tool_name: String,
    pub action: ToolPolicyAction,
    pub required_permission: Option<PermissionLevel>,
    pub allowed_permission: PermissionLevel,
}

impl ToolPolicyDecision {
    pub fn is_denied(&self) -> bool {
        !matches!(self.action, ToolPolicyAction::Allow)
    }
}

/// Tool metadata used by prompt and policy summaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCapability {
    pub name: String,
    pub required_permission: PermissionLevel,
}

/// Immutable policy snapshot attached to an `Agent` session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicySession {
    pub profile: TaskProfile,
    pub capabilities: Vec<ToolCapability>,
    pub allowed_tool_names: BTreeSet<String>,
    pub blocked_tool_names: BTreeSet<String>,
    pub hidden_tool_names: BTreeSet<String>,
    pub decisions: HashMap<String, ToolPolicyDecision>,
}

impl ToolPolicySession {
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        self.allowed_tool_names.contains(tool_name)
    }

    pub fn has_restrictions(&self) -> bool {
        !self.blocked_tool_names.is_empty() || !self.hidden_tool_names.is_empty()
    }

    pub fn restricted_tool_count(&self) -> usize {
        self.blocked_tool_names.len() + self.hidden_tool_names.len()
    }

    pub fn visible_tool_names_for_prompt(&self) -> HashSet<String> {
        if !self.has_restrictions() {
            return HashSet::new();
        }
        let mut names: HashSet<String> = self.allowed_tool_names.iter().cloned().collect();
        if names.is_empty() {
            names.insert(NO_TOOLS_ALLOWED_SENTINEL.to_string());
        }
        names
    }

    pub fn decision_for(&self, tool_name: &str) -> ToolPolicyDecision {
        self.decisions
            .get(tool_name)
            .cloned()
            .unwrap_or_else(|| ToolPolicyDecision {
                tool_name: tool_name.to_string(),
                action: ToolPolicyAction::Deny,
                required_permission: None,
                allowed_permission: self.profile.allowed_permission,
            })
    }
}
