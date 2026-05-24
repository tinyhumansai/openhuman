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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use tempfile::TempDir;

    use super::*;
    use crate::openhuman::memory::ToolMemoryPriority;

    fn ensure_memory_client() {
        static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
        let workspace = WORKSPACE.get_or_init(|| {
            let tmp = TempDir::new().expect("tempdir");
            let path = tmp.path().join("workspace");
            std::fs::create_dir_all(&path).expect("workspace dir");
            std::mem::forget(tmp);
            path
        });
        let _ = crate::openhuman::memory::global::init(workspace.clone());
    }

    fn unique_tool_name() -> String {
        format!("tool-memory-{}", uuid::Uuid::new_v4())
    }

    #[tokio::test]
    async fn tool_rule_put_get_list_and_delete_roundtrip() {
        ensure_memory_client();
        let tool_name = unique_tool_name();

        let stored = tool_rule_put(ToolRulePutParams {
            tool_name: tool_name.clone(),
            rule: "Always ask before sending emails".into(),
            priority: None,
            source: None,
            tags: vec!["safety".into()],
            id: Some("   ".into()),
        })
        .await
        .expect("tool rule put")
        .value;

        assert_eq!(stored.tool_name, tool_name);
        assert_eq!(stored.priority, ToolMemoryPriority::Normal);
        assert_eq!(
            stored.source,
            crate::openhuman::memory::ToolMemorySource::Programmatic
        );
        assert_eq!(stored.tags, vec!["safety".to_string()]);
        assert!(
            !stored.id.trim().is_empty(),
            "blank id should be regenerated"
        );

        let fetched = tool_rule_get(ToolRuleRefParams {
            tool_name: stored.tool_name.clone(),
            id: stored.id.clone(),
        })
        .await
        .expect("tool rule get")
        .value
        .expect("stored rule should exist");
        assert_eq!(fetched.rule, "Always ask before sending emails");

        let listed = tool_rule_list(ToolRuleListParams {
            tool_name: stored.tool_name.clone(),
        })
        .await
        .expect("tool rule list")
        .value;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, stored.id);

        let deleted = tool_rule_delete(ToolRuleRefParams {
            tool_name: stored.tool_name.clone(),
            id: stored.id.clone(),
        })
        .await
        .expect("tool rule delete")
        .value;
        assert!(deleted);

        let after = tool_rule_get(ToolRuleRefParams {
            tool_name: stored.tool_name,
            id: stored.id,
        })
        .await
        .expect("tool rule get after delete");
        assert!(after.value.is_none());
    }

    #[tokio::test]
    async fn tool_rules_for_prompt_sorts_by_priority_and_tool_name() {
        ensure_memory_client();
        let primary_tool = unique_tool_name();
        let secondary_tool = unique_tool_name();

        let high = tool_rule_put(ToolRulePutParams {
            tool_name: primary_tool.clone(),
            rule: "Use the dry-run mode first".into(),
            priority: Some(ToolMemoryPriority::High),
            source: None,
            tags: vec![],
            id: None,
        })
        .await
        .expect("put high")
        .value;
        let normal = tool_rule_put(ToolRulePutParams {
            tool_name: secondary_tool.clone(),
            rule: "Log the final command".into(),
            priority: Some(ToolMemoryPriority::Normal),
            source: None,
            tags: vec![],
            id: None,
        })
        .await
        .expect("put normal")
        .value;

        let prompt = tool_rules_for_prompt(ToolRulesForPromptParams {
            tools: vec![secondary_tool.clone(), primary_tool.clone()],
        })
        .await
        .expect("rules for prompt")
        .value;

        assert_eq!(prompt.rules.len(), 1, "only eager rules should be included");
        assert_eq!(prompt.rules[0].id, high.id);
        assert!(prompt.rendered.contains(&primary_tool));
        assert!(prompt.rendered.contains("Use the dry-run mode first"));

        let json_rules = tool_rules_json(ToolRuleListParams {
            tool_name: secondary_tool.clone(),
        })
        .await
        .expect("tool rules json")
        .value;
        assert!(json_rules.is_array(), "tool rules json should be an array");
        assert!(json_rules
            .as_array()
            .expect("array")
            .iter()
            .any(|row| row["rule"] == "Log the final command"));

        let _ = tool_rule_delete(ToolRuleRefParams {
            tool_name: primary_tool,
            id: high.id,
        })
        .await;
        let _ = tool_rule_delete(ToolRuleRefParams {
            tool_name: secondary_tool,
            id: normal.id,
        })
        .await;
    }
}
