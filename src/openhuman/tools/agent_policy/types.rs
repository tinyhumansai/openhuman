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
    /// Anything that is not a plain `Allow` — **including `HideFromPrompt`**.
    ///
    /// This is the right question for a *direct* call: a tool the model was
    /// never shown has no business being called by name, and refusing it is a
    /// deliberate safety property (see `host::security_gate`).
    ///
    /// It is the wrong question for `use_skill`. See
    /// [`Self::blocks_execution`].
    pub fn is_denied(&self) -> bool {
        !matches!(self.action, ToolPolicyAction::Allow)
    }

    /// Whether this tool may not be *executed*, as opposed to merely being kept
    /// off the prompt.
    ///
    /// `HideFromPrompt` is not an execution denial. It is the normal, healthy
    /// state of every withheld packed tool — AGENTS.md's disclosure table spells
    /// it out: `Withheld` means "Schemas on the wire: no (reached via
    /// `load_skill` / `use_skill`)", "Registered and callable: **yes**".
    ///
    /// Conflating the two is a live bug with teeth: `use_skill` is the only
    /// route a withheld tool has, and gating it on [`Self::is_denied`] refused
    /// every single one of them — the whole pack mechanism, not an edge case.
    /// An unknown tool still lands here, because `decision_for` defaults absent
    /// names to `Deny`: a tool this session does not have is genuinely not
    /// callable, which is the distinction that keeps owner-only tools out.
    pub fn blocks_execution(&self) -> bool {
        if matches!(
            self.action,
            ToolPolicyAction::Deny | ToolPolicyAction::RequireApproval
        ) {
            return true;
        }
        // The permission ceiling, re-checked here because the classifier cannot
        // have applied it. `build_session_from_refs` tests `explicitly_hidden`
        // BEFORE `exceeds_permission`, so a tool that is both hidden *and* over
        // the ceiling is recorded as `HideFromPrompt` and its permission verdict
        // is never taken. `is_denied()` used to mask that by treating every
        // hidden tool as blocked; a predicate that deliberately admits hidden
        // tools has to carry the ceiling itself or it hands `use_skill` a
        // laundering route into a tool the channel would refuse.
        matches!(
            self.required_permission,
            Some(required) if required > self.allowed_permission
        )
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
