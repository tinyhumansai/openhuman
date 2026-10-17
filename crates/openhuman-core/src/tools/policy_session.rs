//! Host entry point for building a tool-policy snapshot.
//!
//! The classification engine lives in `tinytools::ToolPolicyEngine`; this file
//! keeps the host-side trace logging that used to sit inside it, so a policy
//! snapshot stays greppable under `[tool-policy]`.

use std::collections::{BTreeSet, HashMap, HashSet};

use tinytools::{Tool, ToolPolicyEngine, ToolPolicySession};

const LOG_TARGET: &str = "openhuman::tools::agent_policy";

/// Build a [`ToolPolicySession`] over borrowed tools and trace the outcome.
pub(crate) fn build_policy_session(
    agent_id: &str,
    channel: &str,
    entrypoint: &str,
    channel_permissions: &HashMap<String, String>,
    tools: &[&dyn Tool],
    visible_tool_names: &HashSet<String>,
) -> ToolPolicySession {
    log::debug!(
        target: LOG_TARGET,
        "[tool-policy] build_session start agent={} channel={} entrypoint={} tools={} visible={} channel_permissions={}",
        agent_id,
        channel,
        entrypoint,
        tools.len(),
        visible_tool_names.len(),
        channel_permissions.len()
    );
    if channel_permissions.is_empty() {
        log::debug!(
            target: LOG_TARGET,
            "[tool-policy] channel permissions empty; engine preserves the legacy unrestricted surface channel={channel}"
        );
    } else if !channel_permissions.contains_key(channel) {
        log::debug!(
            target: LOG_TARGET,
            "[tool-policy] channel permission missing; engine falls back to readonly channel={channel}"
        );
    }
    let session = ToolPolicyEngine::build_session_from_refs(
        agent_id,
        channel,
        entrypoint,
        channel_permissions,
        tools,
        visible_tool_names,
    );
    log::debug!(
        target: LOG_TARGET,
        "[tool-policy] build_session done agent={} channel={} allowed={} blocked={} hidden={} permission={}",
        agent_id,
        channel,
        session.allowed_tool_names.len(),
        session.blocked_tool_names.len(),
        session.hidden_tool_names.len(),
        session.profile.allowed_permission
    );
    if log::log_enabled!(target: LOG_TARGET, log::Level::Trace) {
        let blocked: BTreeSet<&String> = session.blocked_tool_names.iter().collect();
        log::trace!(
            target: LOG_TARGET,
            "[tool-policy] blocked tools agent={agent_id} channel={channel} names={blocked:?}"
        );
    }
    session
}

#[cfg(test)]
#[path = "policy_session_tests.rs"]
mod tests;
