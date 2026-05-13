//! Storage layer for [`ToolMemoryRule`]s.
//!
//! Rules are persisted as KV rows in the tool's dedicated namespace
//! (`tool-{tool_name}`). KV storage is preferred over the document /
//! embedding pipeline because:
//!
//! - Tool guidance is short, structured, and benefits from exact key
//!   lookup more than semantic retrieval.
//! - Writes never block on the local embedding model.
//! - Atomicity per rule is sufficient for safety-critical instructions.
//!
//! This module wraps an [`Arc<dyn Memory>`] handle (rather than the
//! concrete [`MemoryClient`]) so unit tests can swap in an in-memory
//! mock following the existing `tool_tracker` pattern.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::types::{tool_memory_namespace, ToolMemoryPriority, ToolMemoryRule, ToolMemorySource};
use crate::openhuman::memory::{Memory, MemoryCategory};

/// Maximum number of rules surfaced into the system prompt at once.
///
/// Keeps the cache-friendly prefix bounded even when callers stash a long
/// list of Critical rules over time. Lower-priority rules are still
/// available via [`ToolMemoryStore::list_rules`].
pub const TOOL_MEMORY_PROMPT_CAP: usize = 30;

/// High-level store for tool-scoped memory rules.
///
/// All methods operate on a single shared [`Arc<dyn Memory>`] backend.
/// Cheap to clone — the backend is reference-counted.
#[derive(Clone)]
pub struct ToolMemoryStore {
    memory: Arc<dyn Memory>,
}

impl ToolMemoryStore {
    /// Build a new store over the given memory backend.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    /// Upsert a rule and return the stored copy (with `updated_at`
    /// refreshed).
    ///
    /// If a rule with the same `(tool_name, id)` already exists, its
    /// `created_at` is preserved. `tool_name` is sourced from the rule
    /// itself to avoid storage/namespace skew.
    pub async fn put_rule(&self, mut rule: ToolMemoryRule) -> Result<ToolMemoryRule, String> {
        if rule.tool_name.trim().is_empty() {
            return Err("tool_name is required".to_string());
        }
        if rule.rule.trim().is_empty() {
            return Err("rule body is required".to_string());
        }
        if rule.id.trim().is_empty() {
            rule.id = ToolMemoryRule::generate_id();
        }

        let namespace = tool_memory_namespace(&rule.tool_name);
        let key = ToolMemoryRule::storage_key(&rule.id);

        // Preserve created_at on upsert.
        if let Some(existing) = self.fetch_rule(&namespace, &key).await? {
            rule.created_at = existing.created_at;
        }
        rule.updated_at = chrono::Utc::now().to_rfc3339();

        let content = serde_json::to_string(&rule).map_err(|e| e.to_string())?;
        self.memory
            .store(
                &namespace,
                &key,
                &content,
                MemoryCategory::Custom("tool_memory".into()),
                None,
            )
            .await
            .map_err(|e| format!("store tool rule: {e:#}"))?;

        log::debug!(
            "[tool-memory] put rule id={} tool={} priority={:?} source={:?}",
            rule.id,
            rule.tool_name,
            rule.priority,
            rule.source
        );

        Ok(rule)
    }

    /// Fetch a single rule by `(tool_name, id)`.
    pub async fn get_rule(
        &self,
        tool_name: &str,
        rule_id: &str,
    ) -> Result<Option<ToolMemoryRule>, String> {
        let namespace = tool_memory_namespace(tool_name);
        let key = ToolMemoryRule::storage_key(rule_id);
        self.fetch_rule(&namespace, &key).await
    }

    /// List every rule registered for a tool, sorted by priority (high
    /// first) and then `updated_at` descending.
    pub async fn list_rules(&self, tool_name: &str) -> Result<Vec<ToolMemoryRule>, String> {
        let namespace = tool_memory_namespace(tool_name);
        let entries = self
            .memory
            .list(Some(&namespace), None, None)
            .await
            .map_err(|e| format!("list tool rules: {e:#}"))?;

        let mut rules: Vec<ToolMemoryRule> = entries
            .into_iter()
            .filter(|entry| entry.key.starts_with("rule/"))
            .filter_map(
                |entry| match serde_json::from_str::<ToolMemoryRule>(&entry.content) {
                    Ok(rule) => Some(rule),
                    Err(err) => {
                        log::warn!(
                            "[tool-memory] skipping malformed rule key={} tool={tool_name}: {err}",
                            entry.key
                        );
                        None
                    }
                },
            )
            .collect();

        rules.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });

        Ok(rules)
    }

    /// Delete a rule. Returns `true` if the rule existed.
    pub async fn delete_rule(&self, tool_name: &str, rule_id: &str) -> Result<bool, String> {
        let namespace = tool_memory_namespace(tool_name);
        let key = ToolMemoryRule::storage_key(rule_id);
        self.memory
            .forget(&namespace, &key)
            .await
            .map_err(|e| format!("forget tool rule: {e:#}"))
    }

    /// Returns the set of rules whose [`ToolMemoryPriority`] indicates
    /// they must be eagerly surfaced (Critical + High), grouped by tool
    /// name. Result is bounded by [`TOOL_MEMORY_PROMPT_CAP`] entries
    /// total — Critical rules are always preferred over High when the
    /// cap is reached.
    ///
    /// `tools` constrains which tool namespaces to inspect; passing an
    /// empty slice scans every known tool namespace via
    /// [`Memory::namespace_summaries`].
    pub async fn rules_for_prompt(
        &self,
        tools: &[String],
    ) -> Result<HashMap<String, Vec<ToolMemoryRule>>, String> {
        let tool_names = if tools.is_empty() {
            self.list_tool_names().await?
        } else {
            tools
                .iter()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        };

        let mut collected: Vec<ToolMemoryRule> = Vec::new();
        for tool in &tool_names {
            let rules = self.list_rules(tool).await?;
            collected.extend(rules.into_iter().filter(|r| r.priority.is_eager()));
        }

        // Critical first, then High; within a priority, freshest first.
        collected.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        collected.truncate(TOOL_MEMORY_PROMPT_CAP);

        let mut out: HashMap<String, Vec<ToolMemoryRule>> = HashMap::new();
        for rule in collected {
            out.entry(rule.tool_name.clone()).or_default().push(rule);
        }
        Ok(out)
    }

    /// Enumerate every tool that has at least one stored rule, by
    /// inspecting namespace summaries and keeping only the `tool-…`
    /// prefixed ones.
    pub async fn list_tool_names(&self) -> Result<Vec<String>, String> {
        let summaries = self
            .memory
            .namespace_summaries()
            .await
            .map_err(|e| format!("list tool namespaces: {e:#}"))?;
        let mut out = Vec::new();
        for summary in summaries {
            if let Some(tool) = summary.namespace.strip_prefix("tool-") {
                // Exclude empty names and the sentinel used for unscoped
                // edicts captured before any tool call ran — those rules are
                // not permanently associated with a real tool and must not be
                // injected into prompt filtering for arbitrary sessions.
                if !tool.is_empty() && tool != "__unscoped__" {
                    out.push(tool.to_string());
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Convenience constructor: build a rule from caller-supplied fields
    /// and persist it. Returns the stored rule.
    pub async fn record(
        &self,
        tool_name: &str,
        rule_body: &str,
        priority: ToolMemoryPriority,
        source: ToolMemorySource,
        tags: Vec<String>,
    ) -> Result<ToolMemoryRule, String> {
        let mut rule = ToolMemoryRule::new(tool_name, rule_body, priority, source);
        rule.tags = tags;
        self.put_rule(rule).await
    }

    /// Render rules for a single tool into a JSON value suitable for
    /// passing through the RPC envelope. Sorted by priority desc.
    pub async fn list_rules_json(&self, tool_name: &str) -> Result<Value, String> {
        let rules = self.list_rules(tool_name).await?;
        serde_json::to_value(rules).map_err(|e| e.to_string())
    }

    async fn fetch_rule(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<ToolMemoryRule>, String> {
        let entry = self
            .memory
            .get(namespace, key)
            .await
            .map_err(|e| format!("get tool rule: {e:#}"))?;
        match entry {
            Some(entry) => match serde_json::from_str::<ToolMemoryRule>(&entry.content) {
                Ok(rule) => Ok(Some(rule)),
                Err(err) => {
                    log::warn!("[tool-memory] malformed rule entry in {namespace}/{key}: {err}");
                    Ok(None)
                }
            },
            None => Ok(None),
        }
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
