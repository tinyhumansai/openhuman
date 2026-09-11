use super::{TaskProfile, TaskRiskLevel, ToolCapability, ToolPolicyAction, ToolPolicyDecision};
use crate::openhuman::tools::{PermissionLevel, Tool};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Builds deterministic per-session policy snapshots from the active agent,
/// channel, configured channel permissions, and available tool registry.
pub struct ToolPolicyEngine;

impl ToolPolicyEngine {
    /// Resolve the policy profile and per-tool decisions for one agent session.
    ///
    /// Empty `channel_permissions` preserves the legacy unrestricted tool
    /// surface. Once any channel policy exists, unknown channels fall back to
    /// read-only.
    pub fn build_session(
        agent_id: impl Into<String>,
        channel: impl Into<String>,
        entrypoint: impl Into<String>,
        channel_permissions: &HashMap<String, String>,
        tools: &[Box<dyn Tool>],
        visible_tool_names: &HashSet<String>,
    ) -> super::ToolPolicySession {
        let tools: Vec<&dyn Tool> = tools.iter().map(|t| t.as_ref()).collect();
        Self::build_session_from_refs(
            agent_id,
            channel,
            entrypoint,
            channel_permissions,
            &tools,
            visible_tool_names,
        )
    }

    /// [`Self::build_session`] over a borrowed tool set that need not be one
    /// contiguous slice.
    ///
    /// A session agent's callable surface is split across two `Arc`s — the
    /// durable registry and the freshly-synthesised delegation set (see
    /// `Agent::synthesized_tools`). Both must be classified, or a delegate tool
    /// the model can see and call would carry no policy decision at all and
    /// fall through the per-channel permission gate.
    pub fn build_session_from_refs(
        agent_id: impl Into<String>,
        channel: impl Into<String>,
        entrypoint: impl Into<String>,
        channel_permissions: &HashMap<String, String>,
        tools: &[&dyn Tool],
        visible_tool_names: &HashSet<String>,
    ) -> super::ToolPolicySession {
        let channel = channel.into();
        let allowed_permission = permission_for_channel(channel_permissions, &channel);
        let profile = TaskProfile {
            agent_id: agent_id.into(),
            channel,
            entrypoint: entrypoint.into(),
            risk_level: TaskRiskLevel::from_allowed_permission(allowed_permission),
            allowed_permission,
        };

        let mut allowed_tool_names = BTreeSet::new();
        let mut blocked_tool_names = BTreeSet::new();
        let mut hidden_tool_names = BTreeSet::new();
        let mut capabilities = Vec::with_capacity(tools.len());
        let mut decisions = HashMap::with_capacity(tools.len());

        for tool in tools {
            let name = tool.name().to_string();
            let required_permission = tool.permission_level();
            let explicitly_hidden =
                !visible_tool_names.is_empty() && !visible_tool_names.contains(&name);
            let exceeds_permission = required_permission > allowed_permission;

            let action = if explicitly_hidden {
                ToolPolicyAction::HideFromPrompt
            } else if exceeds_permission {
                ToolPolicyAction::Deny
            } else {
                ToolPolicyAction::Allow
            };
            log::trace!(
                target: "openhuman::tools::agent_policy",
                "[tool-policy] classified tool name={} required={} allowed={} explicitly_hidden={} exceeds_permission={} action={:?}",
                name,
                required_permission,
                allowed_permission,
                explicitly_hidden,
                exceeds_permission,
                action
            );

            let capability = ToolCapability {
                name: name.clone(),
                required_permission,
            };

            match action {
                ToolPolicyAction::Allow => {
                    allowed_tool_names.insert(name.clone());
                }
                ToolPolicyAction::RequireApproval | ToolPolicyAction::Deny => {
                    blocked_tool_names.insert(name.clone());
                }
                ToolPolicyAction::HideFromPrompt => {
                    hidden_tool_names.insert(name.clone());
                }
            }

            decisions.insert(
                name.clone(),
                ToolPolicyDecision {
                    tool_name: name,
                    action,
                    required_permission: Some(required_permission),
                    allowed_permission,
                },
            );
            capabilities.push(capability);
        }

        super::ToolPolicySession {
            profile,
            capabilities,
            allowed_tool_names,
            blocked_tool_names,
            hidden_tool_names,
            decisions,
        }
    }
}

fn permission_for_channel(
    channel_permissions: &HashMap<String, String>,
    channel: &str,
) -> PermissionLevel {
    if channel_permissions.is_empty() {
        // Empty map means "operator hasn't configured a per-channel
        // policy yet" — keep the legacy unrestricted surface so existing
        // installs (and unit fixtures that don't seed the map) keep
        // working. The hardening lands at the config layer:
        // [`AgentConfig::migrate_channel_permissions_if_legacy`] runs at
        // startup on legacy installs and seeds the map with safe
        // per-channel defaults so the cap actually engages on the very
        // first boot after upgrade. Once any entry exists, unknown
        // channels fall back to ReadOnly (the `None` arm below).
        log::debug!(
            target: "openhuman::tools::agent_policy",
            "[tool-policy] channel permissions empty; preserving legacy unrestricted surface channel={} (config migration seeds per-channel defaults on first boot)",
            channel
        );
        return PermissionLevel::Dangerous;
    }

    match channel_permissions.get(channel) {
        Some(raw) => match parse_permission_level(raw) {
            Some(permission) => {
                log::debug!(
                    target: "openhuman::tools::agent_policy",
                    "[tool-policy] resolved channel permission channel={} raw={} permission={}",
                    channel,
                    raw,
                    permission
                );
                permission
            }
            None => {
                log::debug!(
                    target: "openhuman::tools::agent_policy",
                    "[tool-policy] invalid channel permission; falling back to readonly channel={} raw={}",
                    channel,
                    raw
                );
                PermissionLevel::ReadOnly
            }
        },
        None => {
            log::debug!(
                target: "openhuman::tools::agent_policy",
                "[tool-policy] channel permission missing; falling back to readonly channel={}",
                channel
            );
            PermissionLevel::ReadOnly
        }
    }
}

fn parse_permission_level(value: &str) -> Option<PermissionLevel> {
    let normalized = value.trim().to_ascii_lowercase().replace(['-', '_'], "");
    let parsed = match normalized.as_str() {
        "none" => Some(PermissionLevel::None),
        "readonly" | "read" => Some(PermissionLevel::ReadOnly),
        "write" => Some(PermissionLevel::Write),
        "execute" | "exec" => Some(PermissionLevel::Execute),
        "dangerous" | "danger" => Some(PermissionLevel::Dangerous),
        _ => None,
    };
    if parsed.is_none() {
        log::trace!(
            target: "openhuman::tools::agent_policy",
            "[tool-policy] permission token did not match raw={} normalized={}",
            value,
            normalized
        );
    }
    parsed
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
