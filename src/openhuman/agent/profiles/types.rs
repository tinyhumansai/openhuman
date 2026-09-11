//! Serde domain types for persistent agent profiles.
//!
//! Profiles let the UI choose a primary agent persona plus runtime defaults
//! (model, temperature, soul, allowed tools/skills/connectors, memory sources)
//! without editing built-in agent TOML.

use serde::{Deserialize, Serialize};

pub const DEFAULT_PROFILE_ID: &str = "default";

/// A user-selectable agent "flavour".
///
/// `None` on any allowlist field (`allowed_tools`, `allowed_skills`,
/// `allowed_mcp_servers`, `composio_integrations`, `memory_sources`) is the
/// "all / unrestricted" sentinel. An empty vec normalises to `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt_suffix: Option<String>,
    /// Tool `name()` allowlist this profile can see. None = all tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub built_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,
    /// Inline SOUL.md content for this personality. Falls back to workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soul_md: Option<String>,
    /// Relative path to a personality-specific SOUL.md file (checked before inline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soul_md_path: Option<String>,
    /// Composio toolkit slugs this personality can access. None = all integrations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composio_integrations: Option<Vec<String>>,
    /// Memory-source entry ids this profile recalls from. None = all sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_sources: Option<Vec<String>>,
    /// Whether this profile recalls prior agent conversations / cross-chat
    /// context. Default true (preserves legacy behaviour).
    #[serde(default = "default_true")]
    pub include_agent_conversations: bool,
    /// Skill / workflow ids this profile can list and run. None = all skills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skills: Option<Vec<String>>,
    /// MCP server names this profile can reach. None = all configured servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_mcp_servers: Option<Vec<String>>,
    /// Auto-assigned memory directory suffix: "" for default, "-1", "-2", etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_dir_suffix: Option<String>,
    /// Whether this profile is the master orchestrator personality.
    #[serde(default)]
    pub is_master: bool,
    /// Display order (lower = shown first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<u32>,
    /// Give this profile its own memory subtree (`memory-<id>`,
    /// `memory_tree-<id>`, `session_raw-<id>`) instead of the shared/global one.
    /// Default false. See [`super::paths::effective_memory_suffix`].
    #[serde(default)]
    pub dedicated_memory: bool,
    /// Give this profile its own working directory under `action_dir`
    /// (`<action_dir>/profiles/<id>`) used as the default cwd for its tool runs.
    /// Default false. See [`super::home::dedicated_workspace_dir`].
    #[serde(default)]
    pub dedicated_workspace: bool,
}

/// serde default for `include_agent_conversations` (true).
pub(crate) fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfilesState {
    pub active_profile_id: String,
    pub profiles: Vec<AgentProfile>,
}

/// A stable, serialized signature of a profile — used as a cache/identity key
/// for prompt construction.
pub fn profile_signature(profile: &AgentProfile) -> String {
    serde_json::to_string(profile).unwrap_or_else(|_| profile.id.clone())
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
