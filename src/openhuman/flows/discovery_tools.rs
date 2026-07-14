//! The discovery agent's terminal output tool: [`SuggestWorkflowsTool`]
//! (`suggest_workflows`).
//!
//! The `flow_discovery` agent (the "Flow Scout", see
//! `agent_registry/agents/flow_discovery/`) reasons read-only over the user's
//! memory, threads, people, connected integrations, existing flows, and the
//! web, then ends its run by calling this tool **once** with a batch of concrete
//! workflow suggestions. The tool validates each pitch, stamps a stable
//! content-hash id (so a re-run dedupes identical ideas instead of piling
//! duplicates — see [`super::store::upsert_suggestions`]), persists them to the
//! `flow_suggestions` table, and returns a `workflow_suggestions` payload the
//! Flows page "Suggested for you" section renders.
//!
//! **Not authoring.** A suggestion is a *pitch* carrying a natural-language
//! `build_prompt`, not a validated graph. When the user clicks "Build this",
//! the frontend hands that prompt to the existing `workflow_builder` agent,
//! which turns it into a real graph proposal to review and save. This keeps the
//! discovery agent strictly read-only and reuses the whole authoring pipeline
//! unchanged.
//!
//! **Permission contract:** `permission_level() == PermissionLevel::None` and
//! `external_effect() == false`. The only write is to the agent's own
//! suggestions sink (internal bookkeeping, no user data mutated, no external
//! effect), so the tool passes a `read_only` sandbox — it is the agent's
//! designated way to emit its result, analogous to
//! [`super::tools::ProposeWorkflowTool`] returning a proposal.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::openhuman::config::Config;
use crate::openhuman::flows::store;
use crate::openhuman::flows::types::{FlowSuggestion, SuggestionStatus};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Hard cap on suggestions accepted in one `suggest_workflows` call. Keeps the
/// UI section digestible and bounds the write; the agent is told to return the
/// few highest-value ideas, not an exhaustive list.
const MAX_SUGGESTIONS_PER_CALL: usize = 8;

/// Max characters kept for any single free-text field before truncation, so a
/// pathological over-long pitch can't bloat the stored row or the card.
const MAX_FIELD_CHARS: usize = 2000;

pub struct SuggestWorkflowsTool {
    config: Arc<Config>,
}

impl SuggestWorkflowsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

/// Derives a stable id for a suggestion from the normalized (lowercased,
/// whitespace-collapsed) title, so the same idea proposed across two discovery
/// runs collides on `ON CONFLICT(id)` and refreshes rather than duplicates.
fn suggestion_id(title: &str) -> String {
    let normalized = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    // 16 hex chars (64 bits) is plenty to avoid collisions across a user's
    // handful of suggestions while keeping the id short and log-friendly.
    format!("sug_{}", hex::encode(&digest[..8]))
}

/// Reads an optional string array field into a `Vec<String>`, trimming empties.
fn read_string_array(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(truncate)
                .collect()
        })
        .unwrap_or_default()
}

/// Trims and truncates a free-text field to [`MAX_FIELD_CHARS`] (char-safe).
fn truncate(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= MAX_FIELD_CHARS {
        return s.to_string();
    }
    s.chars().take(MAX_FIELD_CHARS).collect()
}

#[async_trait]
impl Tool for SuggestWorkflowsTool {
    fn name(&self) -> &str {
        "suggest_workflows"
    }

    fn description(&self) -> &str {
        "Emit your discovered workflow suggestions for the user. Call this ONCE at the end of \
         your run with the few highest-value automations you found. Each suggestion is a PITCH \
         (not a graph): give a short `title`, a one-sentence `one_liner` of what it does, a \
         `rationale` grounded in what you actually observed about THIS user (a recurring thread, \
         a stated goal in memory, a connected app — never generic advice), and a self-contained \
         `build_prompt` that the workflow-builder agent can turn into a real graph. Ground \
         `suggested_connections` in real connection_ref values you saw via list_flow_connections \
         and `suggested_slugs` in real slugs from search_tool_catalog — NEVER invent either. \
         This tool only records the suggestions for the user to review on the Flows page; it does \
         NOT create, enable, or run any flow. Return 1-8 suggestions, best first."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "suggestions": {
                    "type": "array",
                    "description": "1-8 workflow suggestions, highest-value first.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {
                                "type": "string",
                                "description": "Short, human-friendly title, e.g. \"Auto-file email receipts\"."
                            },
                            "one_liner": {
                                "type": "string",
                                "description": "One sentence describing what the workflow does."
                            },
                            "rationale": {
                                "type": "string",
                                "description": "Why this is suggested to THIS user, grounded in what you observed."
                            },
                            "trigger_hint": {
                                "type": "string",
                                "description": "Likely trigger: \"schedule\" | \"app_event\" | \"manual\" (only these self-fire).",
                                "enum": ["schedule", "app_event", "manual"]
                            },
                            "steps_outline": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Plain-language step outline, one per element."
                            },
                            "suggested_connections": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Real connection_ref values from list_flow_connections. Never invented."
                            },
                            "suggested_slugs": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Real Composio action slugs from search_tool_catalog. Never hallucinated."
                            },
                            "build_prompt": {
                                "type": "string",
                                "description": "Self-contained natural-language brief handed to the workflow-builder on \"Build this\"."
                            },
                            "confidence": {
                                "type": "number",
                                "description": "Your confidence in [0,1] that this is a genuinely useful, buildable automation."
                            }
                        },
                        "required": ["title", "one_liner", "rationale", "build_prompt"]
                    }
                },
                "run_id": {
                    "type": "string",
                    "description": "Optional correlation id for the discovery run that produced these."
                }
            },
            "required": ["suggestions"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Terminal emit sink — see module doc. Only writes to the agent's own
        // suggestions store; no user data mutated, no external effect.
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let items = match args.get("suggestions").and_then(Value::as_array) {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                return Ok(ToolResult::error(
                    "Missing or empty 'suggestions' array. Return 1-8 suggestions, best first."
                        .to_string(),
                ))
            }
        };

        let run_id = args
            .get("run_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let now = chrono::Utc::now().to_rfc3339();
        let mut suggestions: Vec<FlowSuggestion> = Vec::new();

        for (idx, item) in items.iter().take(MAX_SUGGESTIONS_PER_CALL).enumerate() {
            let required = |key: &str| -> Result<String, String> {
                match item.get(key).and_then(Value::as_str).map(str::trim) {
                    Some(s) if !s.is_empty() => Ok(truncate(s)),
                    _ => Err(format!("suggestion #{}: missing '{key}'", idx + 1)),
                }
            };

            let title = match required("title") {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(e)),
            };
            let one_liner = match required("one_liner") {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(e)),
            };
            let rationale = match required("rationale") {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(e)),
            };
            let build_prompt = match required("build_prompt") {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::error(e)),
            };

            let trigger_hint = item
                .get("trigger_hint")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let confidence = item
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);

            suggestions.push(FlowSuggestion {
                id: suggestion_id(&title),
                title,
                one_liner,
                rationale,
                trigger_hint,
                steps_outline: read_string_array(item, "steps_outline"),
                suggested_connections: read_string_array(item, "suggested_connections"),
                suggested_slugs: read_string_array(item, "suggested_slugs"),
                build_prompt,
                confidence,
                status: SuggestionStatus::New,
                created_at: now.clone(),
                source_run_id: run_id.clone(),
            });
        }

        // Dedupe within this single batch on id, keeping the first (highest
        // ranked) occurrence, so two near-identical titles in one call don't
        // fight over the same row.
        suggestions.dedup_by(|a, b| a.id == b.id);
        let mut seen = std::collections::HashSet::new();
        suggestions.retain(|s| seen.insert(s.id.clone()));

        tracing::info!(
            target: "flows",
            count = suggestions.len(),
            run_id = run_id.as_deref().unwrap_or("-"),
            "[flows] suggest_workflows: recording discovered suggestions"
        );

        let written = store::upsert_suggestions(&self.config, &suggestions)
            .map_err(|e| anyhow::anyhow!("failed to persist suggestions: {e}"))?;

        let payload = json!({
            "type": "workflow_suggestions",
            "count": written,
            "suggestions": suggestions,
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?))
    }
}

#[cfg(test)]
#[path = "discovery_tools_tests.rs"]
mod tests;
