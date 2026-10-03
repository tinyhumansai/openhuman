//! Agent tools giving a running flow a private, tag-scoped slice of memory,
//! plus the flow-memory helpers every flow memory caller shares.
//!
//! Memory v2 has no namespaces: a flow's items are the items carrying its
//! [`flow_tag`] (`flow:<flow_id>`). Every flow write also carries
//! [`FLOWS_TAG`] (`flows`), which is what the read-only cross-flow
//! `scope: "flows"` recall filters on. A keyed write additionally carries
//! [`flow_key_tag`] (`flow:<flow_id>:key:<key>`) so it can be replaced and
//! forgotten by key.
//!
//! Motivating use case: a newsletter-digest flow that runs on a schedule
//! needs to remember which items it already sent so it doesn't re-send them
//! on the next run.
//!
//! **Security invariant (non-negotiable):** there is no code path here by
//! which a flow can write an item tagged for another flow. Every write
//! derives its tags from the run's trusted `TrustedAutomation { Workflow }`
//! origin ([`trusted_flow_id`]), never from a model-supplied `flow_id`
//! argument; outside a trusted workflow run the write is refused outright.
//! Flow writes are stored as `source.kind = agent` learnings with the flow
//! tags, so they never pose as user-authored conversation or documents.
//! [`FlowMemoryRecallTool`] only ever reads with a flow tag filter
//! (`flow:<id>` or `flows`), so it never reaches the user's own memory.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tinymemory::{
    Citation, ForgetTarget, LearningKind, MemoryMeta, MetaFilter, SourceKind, SourceRef,
};
use tinytools::{PermissionLevel, Tool, ToolResult};

use crate::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use crate::config::Config;
use crate::memory::types::{LearnParams, RecallParams};
use crate::memory::{MemoryError, MemoryResult};
use crate::security::policy::ToolOperation;
use crate::security::SecurityPolicy;

/// Prefix of a flow's own tag (see [`flow_tag`]).
pub const FLOW_TAG_PREFIX: &str = "flow:";

/// Tag carried by every flow-written item; the cross-flow (`scope: "flows"`)
/// read filter.
pub const FLOWS_TAG: &str = "flows";

/// Infix between a flow's tag and a key in [`flow_key_tag`].
const FLOW_KEY_INFIX: &str = ":key:";

/// Returns the flow id the *run itself* is scoped under, when the current
/// agent turn is executing inside a saved-flow run
/// (`AgentTurnOrigin::TrustedAutomation { job_id, source: Workflow { .. } }` —
/// see `flows::ops::workflow_origin`). `job_id` on that variant IS the
/// running flow's id.
///
/// **Security invariant:** this is the ONLY trustworthy source of "which flow
/// is calling". A `flow_id` tool argument is model-supplied and can be forged
/// by a prompt-injected caller. Writes refuse when this is `None`; the
/// read-only recall falls back to the argument (cross-flow reads are already
/// open by design, so an argument grants no new read privilege).
fn trusted_flow_id() -> Option<String> {
    match turn_origin::current() {
        Some(AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: TrustedAutomationSource::Workflow { .. },
        }) => Some(job_id),
        _ => None,
    }
}

/// The tag marking an item as `flow_id`'s own.
///
/// **Security invariant:** the only constructor of flow tags. Every caller —
/// the agent tools below, the tinyflows `memory` node adapter, the post-run
/// digest subscriber and `flows_delete` — passes a trusted flow id.
#[must_use]
pub fn flow_tag(flow_id: &str) -> String {
    format!("{FLOW_TAG_PREFIX}{flow_id}")
}

/// The tag marking an item as `flow_id`'s value for `key`.
#[must_use]
pub fn flow_key_tag(flow_id: &str, key: &str) -> String {
    format!("{FLOW_TAG_PREFIX}{flow_id}{FLOW_KEY_INFIX}{key}")
}

/// The key a flow item was written under, recovered from its tags.
#[must_use]
pub fn flow_key_of(meta: &MemoryMeta) -> Option<&str> {
    meta.tags.iter().find_map(|tag| {
        tag.strip_prefix(FLOW_TAG_PREFIX)
            .and_then(|rest| rest.split_once(FLOW_KEY_INFIX))
            .map(|(_, key)| key)
    })
}

/// The filter matching every item `flow_id` wrote.
#[must_use]
pub fn flow_filter(flow_id: &str) -> MetaFilter {
    MetaFilter {
        tags_any: vec![flow_tag(flow_id)],
        ..MetaFilter::default()
    }
}

/// The filter matching every flow-written item, of any flow.
#[must_use]
pub fn cross_flow_filter() -> MetaFilter {
    MetaFilter {
        tags_any: vec![FLOWS_TAG.to_string()],
        ..MetaFilter::default()
    }
}

/// Host metadata for an item `flow_id` writes: `source.kind = agent` with the
/// flow tag as its source id, tagged [`flow_tag`], [`FLOWS_TAG`] and `extra`.
#[must_use]
pub fn flow_meta(flow_id: &str, extra: &[String]) -> MemoryMeta {
    let tag = flow_tag(flow_id);
    let mut tags = vec![tag.clone(), FLOWS_TAG.to_string()];
    tags.extend(extra.iter().cloned());
    MemoryMeta {
        source: SourceRef {
            kind: SourceKind::Agent,
            id: Some(tag),
        },
        tags,
        ..MemoryMeta::default()
    }
}

/// Forgets every item matching `filter` (which must be non-empty). Memory
/// off forgets nothing.
pub async fn forget_matching(config: &Config, filter: MetaFilter) -> MemoryResult<usize> {
    if filter.is_empty() {
        return Err(MemoryError::invalid(
            "refusing to forget with an empty filter",
        ));
    }
    let bound = match crate::memory::engine::resolve(config).engine() {
        Ok(bound) => bound,
        Err(MemoryError::Off(_)) => {
            tracing::debug!("[flows:memory] forget skipped: memory is off");
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    let report = bound.engine.forget(ForgetTarget::Filter(filter)).await?;
    tracing::debug!(
        engine = %bound.id,
        forgotten = report.forgotten,
        "[flows:memory] forgot items by tag filter"
    );
    Ok(report.forgotten)
}

/// Stores `content` as `flow_id`'s value for `key`, replacing an earlier
/// value under the same key. Returns the stored item's id.
pub async fn remember_keyed(
    config: &Config,
    flow_id: &str,
    key: &str,
    content: &str,
    kind: LearningKind,
) -> MemoryResult<String> {
    // Fail with MEMORY_OFF before touching anything.
    crate::memory::engine::resolve(config).engine()?;
    let key_tag = flow_key_tag(flow_id, key);
    let replaced = forget_matching(
        config,
        MetaFilter {
            tags_any: vec![key_tag.clone()],
            ..MetaFilter::default()
        },
    )
    .await?;
    let view = crate::memory::ops::learn(
        config,
        LearnParams {
            text: content.to_string(),
            kind: Some(kind),
            confidence: None,
            meta: None,
        },
        Some(flow_meta(flow_id, &[key_tag])),
    )
    .await?;
    tracing::debug!(
        flow_id = %flow_id,
        key_chars = key.chars().count(),
        content_chars = content.chars().count(),
        replaced,
        "[flows:memory] keyed flow memory stored"
    );
    Ok(view.id)
}

/// Maps the tool's `category` argument onto a learning kind.
fn learning_kind_for(category: Option<&str>) -> LearningKind {
    match category.map(|c| c.trim().to_ascii_lowercase()) {
        None => LearningKind::Fact,
        Some(c) => match c.as_str() {
            "" | "fact" | "core" => LearningKind::Fact,
            "preference" => LearningKind::Preference,
            "procedure" => LearningKind::Procedure,
            "correction" => LearningKind::Correction,
            _ => LearningKind::Other,
        },
    }
}

/// Read-only recall over a flow's own items, or (with `scope: "flows"`)
/// across every flow's items — never the user's own memory.
pub struct FlowMemoryRecallTool;

impl FlowMemoryRecallTool {
    /// Holds no memory handle — config and engine are resolved per call.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for FlowMemoryRecallTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Renders a recall answer and its citations, each attributed to its flow
/// key when it has one.
fn render_recall(answer: &str, citations: &[Citation]) -> String {
    if citations.is_empty() {
        return "No flow memories found matching that query.".to_string();
    }
    use std::fmt::Write;
    let mut output = format!("{answer}\n\nSources ({}):\n", citations.len());
    for citation in citations {
        let key = flow_key_of(&citation.meta).unwrap_or("-");
        let _ = writeln!(output, "- [{}] [{key}] {}", citation.id.0, citation.snippet);
    }
    output
}

#[async_trait]
impl Tool for FlowMemoryRecallTool {
    fn name(&self) -> &str {
        "flow_memory_recall"
    }

    fn description(&self) -> &str {
        "Search a flow's own private memory for relevant facts — e.g. so a scheduled \
         digest flow can check what it already sent before, to avoid duplicates. `scope: \"flow\"` \
         (the default) searches only the calling flow's own memory. `scope: \"flows\"` searches \
         read-only across every flow's memory (useful when related flows should dedupe \
         against each other). This tool never reads the user's personal memory — only memory \
         flows have written about their own runs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords or phrase to search for"
                },
                "flow_id": {
                    "type": "string",
                    "description": "The calling flow's id. Inside a running flow this is informational \
                     only: the active flow's own id (from the run's trusted origin) is authoritative and \
                     any value supplied here is ignored. Required only when this tool is invoked outside \
                     a flow run (e.g. from a chat agent)."
                },
                "scope": {
                    "type": "string",
                    "enum": ["flow", "flows"],
                    "description": "\"flow\" (default) searches only this flow's own memory; \"flows\" searches read-only across every flow's memory."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return (default: 5)"
                }
            },
            "required": ["query", "flow_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.trim(),
            None => return Ok(ToolResult::error("Missing 'query' parameter".to_string())),
        };
        if query.is_empty() {
            return Ok(ToolResult::error("query cannot be empty".to_string()));
        }
        let flow_id_arg = args.get("flow_id").and_then(|v| v.as_str()).map(str::trim);

        // SECURITY: inside a running flow the trusted origin wins over the
        // model-supplied `flow_id` argument.
        let flow_id = match trusted_flow_id() {
            Some(trusted_id) => {
                tracing::debug!(
                    target: "flows",
                    flow_id = %trusted_id,
                    "[flows:memory] flow_memory_recall: flow id from the trusted Workflow origin"
                );
                trusted_id
            }
            None => match flow_id_arg {
                None => return Ok(ToolResult::error("Missing 'flow_id' parameter".to_string())),
                Some("") => return Ok(ToolResult::error("flow_id cannot be empty".to_string())),
                Some(arg) => arg.to_string(),
            },
        };
        let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("flow");
        let filter = match scope {
            "flow" => flow_filter(&flow_id),
            "flows" => cross_flow_filter(),
            other => {
                return Ok(ToolResult::error(format!(
                    "Unknown scope '{other}': expected 'flow' or 'flows'"
                )))
            }
        };
        #[allow(clippy::cast_possible_truncation)]
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map_or(5, |v| v as usize);

        let config = match crate::config::rpc::load_config_with_timeout().await {
            Ok(config) => config,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "Flow memory recall failed: {error}"
                )))
            }
        };
        tracing::debug!(
            target: "flows",
            scope,
            query_chars = query.chars().count(),
            limit,
            "[flows:memory] flow_memory_recall: querying"
        );
        match crate::memory::ops::recall(
            &config,
            RecallParams {
                question: query.to_string(),
                filter: Some(filter),
                limit: Some(limit),
            },
        )
        .await
        {
            Ok(view) => {
                tracing::debug!(
                    target: "flows",
                    scope,
                    citations = view.citations.len(),
                    "[flows:memory] flow_memory_recall: answered"
                );
                Ok(ToolResult::success(render_recall(
                    &view.answer,
                    &view.citations,
                )))
            }
            Err(error) => {
                tracing::debug!(
                    target: "flows",
                    code = error.code(),
                    "[flows:memory] flow_memory_recall: failed"
                );
                Ok(ToolResult::error(format!(
                    "Flow memory recall failed: {error}"
                )))
            }
        }
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Write access to a flow's own memory — and *only* its own. See the module
/// doc for the security invariant this tool exists to preserve.
pub struct FlowMemoryRememberTool {
    security: Arc<SecurityPolicy>,
}

impl FlowMemoryRememberTool {
    /// Holds no memory handle — config and engine are resolved per call.
    #[must_use]
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for FlowMemoryRememberTool {
    fn name(&self) -> &str {
        "flow_memory_remember"
    }

    fn description(&self) -> &str {
        "Store a fact in THIS flow's own private memory — e.g. so a scheduled digest \
         flow can remember which items it already sent, to avoid re-sending them on the next run. \
         Writing the same `key` again replaces the earlier value. The flow is taken from the run \
         itself; there is no way to target the user's personal memory or another flow's memory \
         from this tool."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "flow_id": {
                    "type": "string",
                    "description": "Informational only: inside a running flow the active flow's own id \
                     (from the run's trusted origin) is authoritative and this value is ignored. This \
                     tool ONLY works inside a workflow run — calling it from chat or any other context \
                     without a trusted run origin is refused, regardless of what is passed here."
                },
                "key": {
                    "type": "string",
                    "description": "Unique key for this memory within the flow's own memory"
                },
                "content": {
                    "type": "string",
                    "description": "The information to remember"
                },
                "category": {
                    "type": "string",
                    "description": "What kind of statement this is: 'fact' (default), 'preference', 'procedure', 'correction', or anything else (stored as 'other')."
                }
            },
            "required": ["flow_id", "key", "content"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let flow_id_arg = args.get("flow_id").and_then(|v| v.as_str());
        let Some(key) = args.get("key").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'key' parameter".to_string()));
        };
        let Some(content) = args.get("content").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'content' parameter".to_string()));
        };
        let kind = learning_kind_for(args.get("category").and_then(|v| v.as_str()));

        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "flow_memory_remember")
        {
            return Ok(ToolResult::error(error));
        }

        // SECURITY (T-M2): the flow comes from the trusted run origin only;
        // outside a workflow run the write is refused.
        let Some(flow_id) = trusted_flow_id() else {
            tracing::warn!(
                target: "flows",
                requested_flow_id_chars = flow_id_arg.map_or(0, str::len),
                "[flows:memory:security] flow_memory_remember refused: no trusted Workflow run origin"
            );
            return Ok(ToolResult::error(
                "flow memory writes are only available inside a workflow run".to_string(),
            ));
        };
        let key = key.trim();
        if key.is_empty() {
            return Ok(ToolResult::error("key cannot be empty".to_string()));
        }
        if crate::security::scrub::has_likely_secret(content) {
            tracing::warn!(
                target: "flows",
                key_chars = key.chars().count(),
                content_chars = content.chars().count(),
                "[flows:memory:safety] flow_memory_remember rejected secret-like content"
            );
            return Ok(ToolResult::error(
                "Refusing to store content that looks like a secret. Remove credentials or tokens and try again.".to_string(),
            ));
        }

        let config = match crate::config::rpc::load_config_with_timeout().await {
            Ok(config) => config,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "Failed to store flow memory: {error}"
                )))
            }
        };
        match remember_keyed(&config, &flow_id, key, content, kind).await {
            Ok(_) => Ok(ToolResult::success(format!("Stored flow memory: {key}"))),
            Err(error) => {
                tracing::debug!(
                    target: "flows",
                    code = error.code(),
                    "[flows:memory] flow_memory_remember: store failed"
                );
                Ok(ToolResult::error(format!(
                    "Failed to store flow memory: {error}"
                )))
            }
        }
    }
}

#[cfg(test)]
#[path = "memory_tools_tests.rs"]
mod tests;
