//! RPC handlers for the tool-scoped memory layer (see
//! [`crate::openhuman::memory::tool_memory`]).
//!
//! All handlers go through [`active_memory_client`] so they hit the
//! same `UnifiedMemory` backend the rest of the memory RPCs use, and
//! the namespace they touch is exactly `tool-{tool_name}` — never
//! `global` or `tool_effectiveness`.

use serde::Deserialize;
use serde_json::Value;

use crate::openhuman::memory::tool_memory::{
    ToolMemoryPriority, ToolMemoryRule, ToolMemorySource, ToolMemoryStore,
};
use crate::rpc::RpcOutcome;

use super::helpers::active_memory_client;

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

async fn open_store() -> Result<ToolMemoryStore, String> {
    let client = active_memory_client().await?;
    Ok(ToolMemoryStore::new(client.memory_handle()))
}

/// Upsert a tool-scoped memory rule.
pub async fn tool_rule_put(
    params: ToolRulePutParams,
) -> Result<RpcOutcome<ToolMemoryRule>, String> {
    log::debug!("[tool-memory] rpc tool_rule_put tool={}", params.tool_name);
    let store = open_store().await?;
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
    let stored = store.put_rule(rule).await?;
    Ok(RpcOutcome::single_log(stored, "tool memory rule stored"))
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
    let store = open_store().await?;
    let rule = store.get_rule(&params.tool_name, &params.id).await?;
    Ok(RpcOutcome::single_log(rule, "tool memory rule fetched"))
}

/// List every tool-scoped rule for a tool.
pub async fn tool_rule_list(
    params: ToolRuleListParams,
) -> Result<RpcOutcome<Vec<ToolMemoryRule>>, String> {
    log::debug!("[tool-memory] rpc tool_rule_list tool={}", params.tool_name);
    let store = open_store().await?;
    let rules = store.list_rules(&params.tool_name).await?;
    Ok(RpcOutcome::single_log(rules, "tool memory rules listed"))
}

/// Delete a tool-scoped rule by id.
pub async fn tool_rule_delete(params: ToolRuleRefParams) -> Result<RpcOutcome<bool>, String> {
    log::debug!(
        "[tool-memory] rpc tool_rule_delete tool={} id={}",
        params.tool_name,
        params.id
    );
    let store = open_store().await?;
    let deleted = store.delete_rule(&params.tool_name, &params.id).await?;
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
    let store = open_store().await?;
    let grouped = store.rules_for_prompt(&params.tools).await?;
    let mut flat: Vec<ToolMemoryRule> = grouped.into_values().flatten().collect();
    flat.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.tool_name.cmp(&b.tool_name))
            .then_with(|| a.rule.cmp(&b.rule))
    });
    let rendered = crate::openhuman::memory::tool_memory::render_tool_memory_rules(&flat);
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
    let store = open_store().await?;
    let value = store.list_rules_json(&params.tool_name).await?;
    Ok(RpcOutcome::single_log(value, "tool memory rules json"))
}
