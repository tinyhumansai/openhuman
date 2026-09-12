//! RPC handlers for the tool-scoped memory layer (see
//! [`crate::openhuman::memory::tool_memory`]).
//!
//! All handlers hit the same `UnifiedMemory` backend the rest of the memory
//! RPCs use, and the namespace they touch is exactly `tool-{tool_name}` —
//! never `global` or `tool_effectiveness`.
//!
//! Every handler reaches the module-backed [`MemoryToolMemory`](crate::openhuman::memory::api::provider::MemoryToolMemory)
//! API through [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard),
//! including the host-shaped operations that compose multiple API methods to
//! preserve their historical response values.

use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::openhuman::memory::api::provider::MemoryProvider;

use crate::openhuman::memory::api::tool_memory::{
    ToolMemoryPriority, ToolMemoryRule, ToolMemorySource,
};
use crate::rpc::RpcOutcome;

/// Parameters for `memory_tool_rule_put`.
#[derive(Debug, Deserialize)]
pub struct ToolRulePutParams {
    /// Tool the rule applies to (e.g. `email`, `shell`).
    pub tool_name: String,
    /// Natural-language rule body.
    pub rule: String,
    /// Priority/criticality. Defaults to `normal` when omitted.
    #[serde(default)]
    pub priority: Option<ToolMemoryPriority>,
    /// Provenance — defaults to `programmatic` when omitted.
    #[serde(default)]
    pub source: Option<ToolMemorySource>,
    /// Optional tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional rule id — when supplied, the call upserts in place
    /// rather than creating a new entry.
    #[serde(default)]
    pub id: Option<String>,
}

/// Parameters for `memory_tool_rule_get` / `memory_tool_rule_delete`.
#[derive(Debug, Deserialize)]
pub struct ToolRuleRefParams {
    pub tool_name: String,
    pub id: String,
}

/// Parameters for `memory_tool_rule_list`.
#[derive(Debug, Deserialize)]
pub struct ToolRuleListParams {
    pub tool_name: String,
}

/// Parameters for `memory_tool_rules_for_prompt`.
#[derive(Debug, Deserialize, Default)]
pub struct ToolRulesForPromptParams {
    /// Constrain the result to these tools. Empty (or omitted) scans
    /// every known tool namespace.
    #[serde(default)]
    pub tools: Vec<String>,
}

async fn tool_memory_guard() -> Result<Arc<crate::openhuman::memory::guard::MemoryGuard>, String> {
    super::guard::active_memory_guard().await
}

/// Upsert a tool-scoped memory rule.
pub async fn tool_rule_put(
    params: ToolRulePutParams,
) -> Result<RpcOutcome<ToolMemoryRule>, String> {
    log::debug!("[tool-memory] rpc tool_rule_put tool={}", params.tool_name);
    let mut rule = ToolMemoryRule::new(
        &params.tool_name,
        &params.rule,
        params.priority.unwrap_or_default(),
        params.source.unwrap_or_default(),
    );
    rule.tags = params.tags;
    if let Some(id) = params.id {
        if !id.trim().is_empty() {
            rule.id = id;
        }
    }
    let guard = tool_memory_guard().await?;
    guard
        .as_tool_memory()
        .ok_or_else(|| NO_TOOL_MEMORY.to_string())?
        .put_tool_rule(rule.clone())
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(rule, "tool memory rule stored"))
}

/// Fetch a tool-scoped rule by id.
pub async fn tool_rule_get(
    params: ToolRuleRefParams,
) -> Result<RpcOutcome<Option<ToolMemoryRule>>, String> {
    log::debug!(
        "[tool-memory] rpc tool_rule_get tool={} id={}",
        params.tool_name,
        params.id
    );
    let guard = tool_memory_guard().await?;
    let rule = guard
        .as_tool_memory()
        .ok_or_else(|| NO_TOOL_MEMORY.to_string())?
        .tool_rules(&params.tool_name)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|rule| rule.id == params.id);
    Ok(RpcOutcome::single_log(rule, "tool memory rule fetched"))
}

/// The reason a guarded handler in this file cannot proceed.
///
/// A driver that does not advertise `Capability::ToolMemory` returns `None`
/// from `as_tool_memory()`; the embedded driver always advertises it, so this
/// is reachable only under a null / fallback binding.
///
/// Shared with the `memory_tools_list` / `memory_tools_put` agent tools, which
/// route through the same family.
pub(crate) const NO_TOOL_MEMORY: &str = "memory driver does not support the tool_memory family";

/// List every tool-scoped rule for a tool.
///
/// Routed through [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard).
/// The wire type matches by identity, not conversion:
/// `memory::tool_memory::ToolMemoryRule` **is**
/// `crate::openhuman::memory::api::tool_memory::ToolMemoryRule`.
pub async fn tool_rule_list(
    params: ToolRuleListParams,
) -> Result<RpcOutcome<Vec<ToolMemoryRule>>, String> {
    log::debug!("[tool-memory] rpc tool_rule_list tool={}", params.tool_name);
    let guard = super::guard::active_memory_guard().await?;
    let rules = guard
        .as_tool_memory()
        .ok_or_else(|| NO_TOOL_MEMORY.to_string())?
        .tool_rules(&params.tool_name)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(rules, "tool memory rules listed"))
}

/// Delete a tool-scoped rule by id.
///
/// Routed through [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard)
/// and the shared tool-memory API.
pub async fn tool_rule_delete(params: ToolRuleRefParams) -> Result<RpcOutcome<bool>, String> {
    log::debug!(
        "[tool-memory] rpc tool_rule_delete tool={} id={}",
        params.tool_name,
        params.id
    );
    let guard = super::guard::active_memory_guard().await?;
    let deleted = guard
        .as_tool_memory()
        .ok_or_else(|| NO_TOOL_MEMORY.to_string())?
        .delete_tool_rule(&params.tool_name, &params.id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(deleted, "tool memory rule deleted"))
}

/// Return the rendered prompt block plus the structured rule list for
/// the caller-supplied set of tools. Used by the session builder to
/// pin Critical / High rules into the system prompt.
#[derive(Debug, serde::Serialize)]
pub struct ToolRulesForPromptResult {
    /// Pre-rendered Markdown block, ready for injection.
    pub rendered: String,
    /// Underlying rule snapshot the renderer used.
    pub rules: Vec<ToolMemoryRule>,
}

/// Pre-fetch Critical + High priority rules for prompt injection.
pub async fn tool_rules_for_prompt(
    params: ToolRulesForPromptParams,
) -> Result<RpcOutcome<ToolRulesForPromptResult>, String> {
    log::debug!(
        "[tool-memory] rpc tool_rules_for_prompt tools={:?}",
        params.tools
    );
    let guard = tool_memory_guard().await?;
    let family = guard
        .as_tool_memory()
        .ok_or_else(|| NO_TOOL_MEMORY.to_string())?;
    let mut flat = Vec::new();
    for tool in &params.tools {
        flat.extend(
            family
                .tool_rules(tool)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|rule| rule.priority.is_eager()),
        );
    }
    flat.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.tool_name.cmp(&b.tool_name))
            .then_with(|| a.rule.cmp(&b.rule))
    });
    let rendered = crate::openhuman::memory::tool_memory::prompt::render_tool_memory_rules(&flat);
    Ok(RpcOutcome::single_log(
        ToolRulesForPromptResult {
            rendered,
            rules: flat,
        },
        "tool memory rules prepared for prompt",
    ))
}

/// Render the raw JSON form of a tool's rules, useful for envelope
/// consumers that want the unfiltered list.
pub async fn tool_rules_json(params: ToolRuleListParams) -> Result<RpcOutcome<Value>, String> {
    log::debug!(
        "[tool-memory] rpc tool_rules_json tool={}",
        params.tool_name
    );
    let guard = tool_memory_guard().await?;
    let rules = guard
        .as_tool_memory()
        .ok_or_else(|| NO_TOOL_MEMORY.to_string())?
        .tool_rules(&params.tool_name)
        .await
        .map_err(|e| e.to_string())?;
    let value = serde_json::to_value(rules).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(value, "tool memory rules json"))
}

#[cfg(test)]
#[path = "tool_memory_tests.rs"]
mod tests;
