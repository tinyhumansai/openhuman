//! Agent tools giving a running flow a private, sandboxed memory namespace
//! (`flow_<flow_id>` — see [`flow_namespace`]).
//!
//! Motivating use case: a newsletter-digest flow that runs on a schedule
//! needs to remember which items it already sent so it doesn't re-send them
//! on the next run. Without a durable, flow-scoped place to note "already
//! sent: <item id>", the agent node inside the flow has no way to dedupe
//! across runs other than re-deriving state from the target service itself
//! (which is often lossy or rate-limited).
//!
//! **Security invariant (non-negotiable):** there is no code path here by
//! which a flow can write to — or even name — a namespace other than its
//! own. [`FlowMemoryRememberTool`] derives the namespace internally via
//! [`flow_namespace`] from the caller-supplied `flow_id`; there is no
//! `namespace` parameter a caller could override. Every write is tainted
//! [`MemoryTaint::ExternalSync`] (automation output, not user-authored
//! fact), matching the same taint sync pipelines use for third-party
//! content, so the subconscious gate treats it exactly as conservatively.
//! [`FlowMemoryRecallTool`]'s `scope: "flows"` is read-only cross-flow
//! visibility — it can never be used to write outside a flow's own
//! namespace either.
//!
//! **T-M2 fix:** [`FlowMemoryRememberTool`] only resolves the flow id from
//! the run's own trusted `TrustedAutomation { Workflow }` turn origin (see
//! [`trusted_flow_id`]) — it never trusts a model-supplied `flow_id` arg.
//! Outside a trusted workflow run (every chat/orchestrator turn) the write
//! is refused outright, not routed to whatever `flow_id` the caller named.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, MemoryTaint, RecallOpts};
use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Returns the flow id the *run itself* is scoped under, when the current
/// agent turn is executing inside a saved-flow run
/// (`AgentTurnOrigin::TrustedAutomation { job_id, source: Workflow { .. } }` —
/// see `flows::ops::workflow_origin`, scoped around every `flows_run` /
/// `flows_resume`). `job_id` on that variant IS the running flow's id.
///
/// **Security invariant:** this is the ONLY trustworthy source of "which flow
/// is calling". A `flow_id` value handed in as an ordinary tool argument is
/// model-supplied and can be forged by a prompt-injected caller (or another
/// agent invoking the tool directly) to name a DIFFERENT flow's namespace.
/// When this returns `Some`, callers MUST use it — and ignore any
/// caller-supplied `flow_id` arg — for the `scope: "flow"` / write case.
///
/// **T-M2 fix:** callers running outside a flow run (e.g. a chat/orchestrator
/// turn with the tool wired in some other context) get `None` here. For the
/// write path ([`FlowMemoryRememberTool`]) that used to fall back to trusting
/// the model-supplied `flow_id` arg — which let a prompt-injected chat turn
/// poison an unrelated flow's private dedup namespace, since the tool has no
/// `external_effect` and therefore never parks for approval. `None` now means
/// [`FlowMemoryRememberTool`] REFUSES the write outright; there is no
/// legitimate chat-side use case for writing another flow's namespace, and
/// refusing is the only fail-closed option that doesn't require inventing an
/// ownership proof. [`FlowMemoryRecallTool`] is unaffected — it stays
/// read-only and its `scope: "flows"` already exposes every flow's namespace
/// by design, so an arg-supplied `flow_id` grants no new read privilege.
fn trusted_flow_id() -> Option<String> {
    match turn_origin::current() {
        Some(AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: TrustedAutomationSource::Workflow { .. },
        }) => Some(job_id),
        _ => None,
    }
}

/// Prefix for a flow's private, sandboxed memory namespace (see
/// [`flow_namespace`]).
///
/// **Deviates from the originally specced `"flow:"` (colon) separator —
/// deliberately.** The `Memory` trait's `UnifiedMemory` backend
/// (`src/openhuman/memory/store/`) is internally inconsistent about
/// namespace sanitization: `store_with_taint`/`recall`/`list`/
/// `MemoryClient::clear_namespace` all route through
/// `UnifiedMemory::sanitize_namespace`
/// (`memory_store/namespace_store/init.rs`), which collapses any character
/// outside `[A-Za-z0-9_/-]` — including `:` — to `_` before touching SQLite.
/// But `Memory::forget` (`memory_store/memory_trait.rs`) queries
/// `WHERE namespace = ?1` against the **raw, unsanitized** argument. With a
/// `"flow:"` prefix, `forget("flow:<id>", key)` would therefore silently
/// never match the row `store_with_taint` actually persisted as
/// `"flow_<id>"` — the post-run digest subscriber's retention sweep
/// (`bus::FlowRunDigestSubscriber`) would then never evict old entries, and
/// `namespace_summaries()`-based cross-flow listing (`scope: "flows"` in
/// [`FlowMemoryRecallTool`]) would have to match the sanitized form anyway
/// since `namespace_summaries` reads the persisted (sanitized) column back
/// verbatim. Using `"flow_"` (already a fixed point of `sanitize_namespace`,
/// since flow ids are hyphenated UUIDs — no character in either the prefix
/// or a flow id ever needs sanitizing) makes every `Memory` method agree
/// with every other one on the exact namespace string, with no silent
/// mismatch anywhere. The namespace is still shared-root and
/// profile-independent exactly as specified — only the separator character
/// changed.
///
/// Re-exported from `flows::mod` as `flows::FLOW_MEMORY_NAMESPACE_PREFIX` —
/// see that module for why this lives here rather than in `mod.rs` itself.
pub const FLOW_MEMORY_NAMESPACE_PREFIX: &str = "flow_";

/// Builds a flow's private, profile-independent memory namespace from a
/// `flow_id`.
///
/// **Security invariant:** this is the *only* place in the codebase that may
/// construct this namespace string. Every caller — the `flow_memory_recall`
/// / `flow_memory_remember` agent tools below, the post-run digest
/// subscriber (`bus::FlowRunDigestSubscriber`), and the `flows_delete`
/// cleanup hook (`ops::flows_delete`) — goes through this function with a
/// `flow_id`, never with a caller-supplied raw namespace. A flow can
/// therefore never write to, or be told the name of, any memory namespace
/// but its own.
///
/// Re-exported from `flows::mod` as `flows::flow_namespace`.
pub fn flow_namespace(flow_id: &str) -> String {
    format!("{FLOW_MEMORY_NAMESPACE_PREFIX}{flow_id}")
}

/// The persisted-namespace prefix matching every flow's memory namespace, as
/// [`Memory::namespace_summaries`] returns it.
///
/// This is intentionally the *same* string as [`FLOW_MEMORY_NAMESPACE_PREFIX`]
/// — kept as a separate, explicitly-named constant here so the "match against
/// what recall/list see" intent stays self-evident at each call site,
/// independent of whether the two ever need to diverge in the future.
const FLOW_MEMORY_NAMESPACE_LISTED_PREFIX: &str = FLOW_MEMORY_NAMESPACE_PREFIX;

/// Read-only recall merged across **every** flow's own `flow_<id>` memory
/// namespace — never the user's personal/global memory, and never any
/// namespace outside the `flow_*` prefix.
///
/// Shared by [`FlowMemoryRecallTool`]'s `scope: "flows"` arm and the
/// tinyflows `memory` node's `scope: "flows"` (`OpenHumanMemory::recall` in
/// `crate::openhuman::flows::tinyflows::memory_adapter`) — both surfaces must see
/// identical cross-flow results, so this is the one place that walks
/// [`Memory::namespace_summaries`] and filters to `flow_*`. A per-namespace
/// recall failure is logged and skipped rather than failing the whole call,
/// so one corrupt/unavailable flow namespace can't blank out every other
/// flow's results.
pub async fn cross_flow_recall(
    memory: &Arc<dyn Memory>,
    query: &str,
    limit: usize,
    min_score: Option<f64>,
) -> anyhow::Result<Vec<MemoryEntry>> {
    let summaries = memory.namespace_summaries().await?;
    let mut merged: Vec<MemoryEntry> = Vec::new();
    for summary in summaries
        .iter()
        .filter(|s| s.namespace.starts_with(FLOW_MEMORY_NAMESPACE_LISTED_PREFIX))
    {
        let opts = RecallOpts {
            namespace: Some(summary.namespace.as_str()),
            min_score,
            ..RecallOpts::default()
        };
        match memory.recall(query, limit, opts).await {
            Ok(entries) => merged.extend(entries),
            Err(e) => {
                log::warn!(
                    "[flows:memory] cross_flow_recall failed for namespace={}: {e}",
                    summary.namespace
                );
            }
        }
    }
    merged.sort_by(|a, b| {
        b.score
            .unwrap_or(0.0)
            .partial_cmp(&a.score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    merged.truncate(limit);
    Ok(merged)
}

/// Read-only recall over a flow's own memory namespace, or (with
/// `scope: "flows"`) across every flow's namespace.
///
/// `scope: "flows"` is intentionally still read-only and still confined to
/// `flow_*` namespaces — it can never see the user's personal/global memory,
/// only other flows' own automation output.
pub struct FlowMemoryRecallTool {
    memory: Arc<dyn Memory>,
}

impl FlowMemoryRecallTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

/// Renders recall hits the same way [`crate::openhuman::memory::tools::recall`]
/// does, with the flow's memory namespace context in each line so a
/// cross-flow `scope: "flows"` result is attributable.
fn render_entries(entries: &[MemoryEntry]) -> String {
    if entries.is_empty() {
        return "No memories found matching that query.".to_string();
    }
    use std::fmt::Write;
    let mut output = format!("Found {} memories:\n", entries.len());
    for entry in entries {
        let score = entry
            .score
            .map_or_else(String::new, |s| format!(" [{s:.0}%]"));
        let namespace = entry.namespace.as_deref().unwrap_or("?");
        let _ = writeln!(
            output,
            "- [{namespace}] [{}] {}: {}{score}",
            entry.category, entry.key, entry.content
        );
    }
    output
}

#[async_trait]
impl Tool for FlowMemoryRecallTool {
    fn name(&self) -> &str {
        "flow_memory_recall"
    }

    fn description(&self) -> &str {
        "Search a flow's own private memory namespace for relevant facts — e.g. so a scheduled \
         digest flow can check what it already sent before, to avoid duplicates. `scope: \"flow\"` \
         (the default) searches only the calling flow's own namespace. `scope: \"flows\"` searches \
         read-only across every flow's private namespace (useful when related flows should dedupe \
         against each other), merged and re-ranked by relevance. This tool never reads the user's \
         personal or global memory — only memory flows have written about their own runs."
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
                    "description": "\"flow\" (default) searches only this flow's own memory namespace; \"flows\" searches read-only across every flow's namespace."
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
        // T-m5: input-validation problems report via `ToolResult::error`
        // uniformly (never `Err(anyhow!)`) — matching every other tool on
        // this belt (recall's own scope-error arms below, remember's
        // flow_id/key checks). An `Err` return surfaces to the model as a
        // hard tool-invocation failure rather than a normal tool result the
        // agent can read and react to in-turn.
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.trim(),
            None => return Ok(ToolResult::error("Missing 'query' parameter".to_string())),
        };
        if query.is_empty() {
            return Ok(ToolResult::error("query cannot be empty".to_string()));
        }

        let flow_id_arg = args.get("flow_id").and_then(|v| v.as_str()).map(str::trim);

        // SECURITY: inside a running flow, the run's own trusted origin is
        // the ONLY authoritative source for "which flow is calling" — never
        // the model-supplied `flow_id` arg. Without this, a prompt-injected
        // caller could pass a different flow's id and read across the
        // sandbox boundary the module doc promises. See `trusted_flow_id`.
        let trusted = trusted_flow_id();
        let flow_id: String = match &trusted {
            Some(trusted_id) => {
                tracing::debug!(
                    target: "flows",
                    flow_id = %trusted_id,
                    "[flows:memory] flow_memory_recall: flow id resolved from the trusted Workflow \
                     run origin (any model-supplied flow_id arg is ignored)"
                );
                trusted_id.clone()
            }
            None => {
                let Some(arg) = flow_id_arg else {
                    return Ok(ToolResult::error("Missing 'flow_id' parameter".to_string()));
                };
                if arg.is_empty() {
                    return Ok(ToolResult::error("flow_id cannot be empty".to_string()));
                }
                arg.to_string()
            }
        };
        let flow_id = flow_id.as_str();
        let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("flow");

        #[allow(clippy::cast_possible_truncation)]
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map_or(5, |v| v as usize);

        match scope {
            "flow" => {
                let namespace = flow_namespace(flow_id);
                let opts = RecallOpts {
                    namespace: Some(namespace.as_str()),
                    ..RecallOpts::default()
                };
                match self.memory.recall(query, limit, opts).await {
                    Ok(entries) => Ok(ToolResult::success(render_entries(&entries))),
                    Err(e) => Ok(ToolResult::error(format!("Flow memory recall failed: {e}"))),
                }
            }
            "flows" => match cross_flow_recall(&self.memory, query, limit, None).await {
                Ok(merged) => Ok(ToolResult::success(render_entries(&merged))),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to list flow memory namespaces: {e}"
                ))),
            },
            other => Ok(ToolResult::error(format!(
                "Unknown scope '{other}': expected 'flow' or 'flows'"
            ))),
        }
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Write access to a flow's own private memory namespace — and *only* its
/// own. See the module doc for the security invariant this tool exists to
/// preserve.
pub struct FlowMemoryRememberTool {
    memory: Arc<dyn Memory>,
    security: Arc<SecurityPolicy>,
}

impl FlowMemoryRememberTool {
    pub fn new(memory: Arc<dyn Memory>, security: Arc<SecurityPolicy>) -> Self {
        Self { memory, security }
    }
}

#[async_trait]
impl Tool for FlowMemoryRememberTool {
    fn name(&self) -> &str {
        "flow_memory_remember"
    }

    fn description(&self) -> &str {
        "Store a fact in THIS flow's own private memory namespace — e.g. so a scheduled digest \
         flow can remember which items it already sent, to avoid re-sending them on the next run. \
         The namespace is derived internally from `flow_id`; there is no way to target the user's \
         personal memory or another flow's namespace from this tool. Stored content is tainted as \
         externally-sourced automation output, never treated as a user-authored fact."
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
                    "description": "Unique key for this memory within the flow's own namespace"
                },
                "content": {
                    "type": "string",
                    "description": "The information to remember"
                },
                "category": {
                    "type": "string",
                    "description": "Memory category: 'core' (permanent), 'daily' (session), 'conversation' (chat), or a custom category name. Defaults to 'core'."
                }
            },
            "required": ["flow_id", "key", "content"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        // T-m5: uniform `ToolResult::error` for input-validation problems —
        // see the matching note on `FlowMemoryRecallTool::execute`.
        let flow_id_arg = args.get("flow_id").and_then(|v| v.as_str());
        let Some(key) = args.get("key").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'key' parameter".to_string()));
        };
        let Some(content) = args.get("content").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'content' parameter".to_string()));
        };

        let category = match args.get("category").and_then(|v| v.as_str()) {
            Some("core") | None => MemoryCategory::Core,
            Some("daily") => MemoryCategory::Daily,
            Some("conversation") => MemoryCategory::Conversation,
            // Route custom categories through `FromStr` so a `custom:<name>`
            // wire value resolves back to `Custom("<name>")` rather than
            // double-prefixing — mirrors `memory_store::MemoryStoreTool`.
            Some(other) => other
                .parse()
                .unwrap_or_else(|_| MemoryCategory::Custom(other.to_string())),
        };

        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "flow_memory_remember")
        {
            return Ok(ToolResult::error(error));
        }

        // SECURITY (T-M2 fix): resolve the namespace-governing flow id from
        // the run's trusted origin ONLY — never from the model-supplied
        // `flow_id` arg. Without this, a prompt-injected caller (or another
        // agent invoking this tool directly) could pass a DIFFERENT flow's
        // id here and poison that flow's private namespace, and — because
        // this tool has no `external_effect` — the write would never park
        // for approval. There is no legitimate chat-side caller of this
        // write path (see `trusted_flow_id`'s doc comment): outside a
        // trusted workflow run, refuse outright rather than trusting an
        // arg that cannot be distinguished from an attacker's.
        let trusted = trusted_flow_id();
        let flow_id: String = match &trusted {
            Some(trusted_id) => {
                tracing::debug!(
                    target: "flows",
                    flow_id = %trusted_id,
                    "[flows:memory] flow_memory_remember: flow id resolved from the trusted Workflow \
                     run origin (any model-supplied flow_id arg is ignored)"
                );
                trusted_id.clone()
            }
            None => {
                // T-M2 supersedes the arg validation that used to live here: the
                // model-supplied `flow_id` is never trusted outside a run, so
                // there is nothing to validate — refuse instead.
                log::warn!(
                    "[flows:memory:security] flow_memory_remember refused: no trusted Workflow run \
                     origin (requested flow_id_chars={})",
                    flow_id_arg.map_or(0, str::len)
                );
                return Ok(ToolResult::error(
                    "flow memory writes are only available inside a workflow run".to_string(),
                ));
            }
        };
        let flow_id = flow_id.as_str();
        let key = key.trim();
        if key.is_empty() {
            return Ok(ToolResult::error("key cannot be empty".to_string()));
        }

        if crate::openhuman::memory::store::safety::has_likely_secret(content) {
            log::warn!(
                "[flows:memory:safety] flow_memory_remember rejected secret-like content flow_id_chars={} key_chars={} content_chars={}",
                flow_id.chars().count(),
                key.chars().count(),
                content.chars().count()
            );
            return Ok(ToolResult::error(
                "Refusing to store content that looks like a secret. Remove credentials or tokens and try again.".to_string(),
            ));
        }

        // SECURITY: the namespace is derived internally from `flow_id` —
        // this tool has no `namespace` parameter, so a flow can only ever
        // write into its own `flow_<id>` sandbox, never user/global memory
        // or another flow's namespace.
        let namespace = flow_namespace(flow_id);
        let display_key = format!("{namespace}/{key}");
        match self
            .memory
            .store_with_taint(
                &namespace,
                key,
                content,
                category,
                None,
                MemoryTaint::ExternalSync,
            )
            .await
        {
            Ok(()) => Ok(ToolResult::success(format!(
                "Stored flow memory: {display_key}"
            ))),
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to store flow memory: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::embeddings::NoopEmbedding;
    use crate::openhuman::memory::store::UnifiedMemory;
    use crate::openhuman::security::AutonomyLevel;
    use tempfile::TempDir;

    fn test_security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy::default())
    }

    fn test_mem() -> (TempDir, Arc<dyn Memory>) {
        let tmp = TempDir::new().unwrap();
        let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        (tmp, Arc::new(mem))
    }

    // ── flow_namespace / FLOW_MEMORY_NAMESPACE_PREFIX ───────────────
    // (relocated from `flows::mod` — see that module's re-export comment)

    #[test]
    fn flow_namespace_uses_the_shared_root_prefix() {
        assert_eq!(flow_namespace("abc-123"), "flow_abc-123");
        assert!(flow_namespace("abc-123").starts_with(FLOW_MEMORY_NAMESPACE_PREFIX));
    }

    #[test]
    fn flow_namespace_is_distinct_per_flow() {
        assert_ne!(flow_namespace("a"), flow_namespace("b"));
    }

    // ── FlowMemoryRecallTool ────────────────────────────────────────

    #[test]
    fn recall_name_and_schema() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRecallTool::new(mem);
        assert_eq!(tool.name(), "flow_memory_recall");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["flow_id"].is_object());
        assert!(schema["properties"]["scope"].is_object());
    }

    #[tokio::test]
    async fn recall_empty_returns_no_results() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRecallTool::new(mem);
        let result = tool
            .execute(json!({"query": "anything", "flow_id": "f1"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("No memories found"));
    }

    #[tokio::test]
    async fn store_then_recall_matches() {
        let (_tmp, mem) = test_mem();
        mem.store_with_taint(
            &flow_namespace("f1"),
            "sent_item_42",
            "Sent newsletter item 42 to subscribers",
            MemoryCategory::Core,
            None,
            MemoryTaint::ExternalSync,
        )
        .await
        .unwrap();

        let tool = FlowMemoryRecallTool::new(mem);
        let result = tool
            .execute(json!({"query": "newsletter item 42", "flow_id": "f1"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("newsletter item 42") || result.output().contains("42"));
        assert!(result.output().contains("Found 1"));
    }

    #[tokio::test]
    async fn scope_flow_isolates_to_own_namespace() {
        let (_tmp, mem) = test_mem();
        mem.store_with_taint(
            &flow_namespace("f1"),
            "k",
            "shared keyword hit",
            MemoryCategory::Core,
            None,
            MemoryTaint::ExternalSync,
        )
        .await
        .unwrap();
        mem.store_with_taint(
            &flow_namespace("f2"),
            "k",
            "shared keyword hit",
            MemoryCategory::Core,
            None,
            MemoryTaint::ExternalSync,
        )
        .await
        .unwrap();

        let tool = FlowMemoryRecallTool::new(mem);
        let result = tool
            .execute(json!({"query": "shared keyword", "flow_id": "f1", "scope": "flow"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        // Only f1's own entry, not f2's.
        assert!(result.output().contains("Found 1"));
    }

    #[tokio::test]
    async fn scope_flows_crosses_namespaces() {
        let (_tmp, mem) = test_mem();
        mem.store_with_taint(
            &flow_namespace("f1"),
            "k",
            "shared keyword hit from f1",
            MemoryCategory::Core,
            None,
            MemoryTaint::ExternalSync,
        )
        .await
        .unwrap();
        mem.store_with_taint(
            &flow_namespace("f2"),
            "k",
            "shared keyword hit from f2",
            MemoryCategory::Core,
            None,
            MemoryTaint::ExternalSync,
        )
        .await
        .unwrap();

        let tool = FlowMemoryRecallTool::new(mem);
        let result = tool
            .execute(json!({"query": "shared keyword", "flow_id": "f1", "scope": "flows"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        // Both f1's and f2's namespaces are visible under scope="flows".
        assert!(result.output().contains("Found 2"));
    }

    // T-m5: a missing/invalid input param reports via `ToolResult::error`
    // (an `Ok(..)` the model can read and react to in-turn), never
    // `Err(anyhow!)` (a hard tool-invocation failure) — matching every other
    // input-validation problem on this belt (see the scope/empty-value
    // tests above, which already used this channel before the fix).
    #[tokio::test]
    async fn recall_missing_query_errs() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRecallTool::new(mem);
        let result = tool.execute(json!({"flow_id": "f1"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Missing 'query'"));
    }

    #[tokio::test]
    async fn recall_missing_flow_id_errs() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRecallTool::new(mem);
        let result = tool.execute(json!({"query": "anything"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Missing 'flow_id'"));
    }

    // ── FlowMemoryRememberTool ──────────────────────────────────────

    #[test]
    fn remember_name_and_schema() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem, test_security());
        assert_eq!(tool.name(), "flow_memory_remember");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["flow_id"].is_object());
        assert!(schema["properties"]["key"].is_object());
        assert!(schema["properties"]["content"].is_object());
        // No `namespace` parameter exists — the security invariant that a
        // flow can never target another namespace.
        assert!(schema["properties"]["namespace"].is_null());
        assert_eq!(tool.permission_level(), PermissionLevel::Write);
    }

    /// Helper: a trusted `TrustedAutomation { Workflow }` origin scoped to
    /// `job_id`, the only source `flow_memory_remember` will act on since the
    /// T-M2 fix.
    fn trusted_workflow_origin(job_id: &str) -> AgentTurnOrigin {
        AgentTurnOrigin::TrustedAutomation {
            job_id: job_id.to_string(),
            source: TrustedAutomationSource::Workflow {
                require_approval: false,
            },
        }
    }

    #[tokio::test]
    async fn remember_stores_with_external_sync_taint() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem.clone(), test_security());
        let result = turn_origin::with_origin(
            trusted_workflow_origin("f1"),
            tool.execute(
                json!({"flow_id": "f1", "key": "sent_item_42", "content": "Sent item 42"}),
            ),
        )
        .await
        .unwrap();
        assert!(!result.is_error);

        let entry = mem
            .get(&flow_namespace("f1"), "sent_item_42")
            .await
            .unwrap()
            .expect("entry should be stored");
        assert_eq!(entry.content, "Sent item 42");
        assert_eq!(entry.taint, MemoryTaint::ExternalSync);
    }

    #[tokio::test]
    async fn remember_writes_only_to_own_flow_namespace() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem.clone(), test_security());
        turn_origin::with_origin(
            trusted_workflow_origin("f1"),
            tool.execute(json!({"flow_id": "f1", "key": "k", "content": "f1 content"})),
        )
        .await
        .unwrap();

        // Never lands in another flow's namespace, the shared "flows" scope
        // namespace, or global/user memory.
        assert!(mem.get(&flow_namespace("f2"), "k").await.unwrap().is_none());
        assert!(mem.get("global", "k").await.unwrap().is_none());
        assert!(mem
            .get("f1", "k") // raw flow_id, not the derived namespace
            .await
            .unwrap()
            .is_none());
    }

    /// SECURITY (T-M2): the primary fix under test — a chat/orchestrator turn
    /// (no trusted `Workflow` run origin) must be refused outright, never
    /// routed to the model-supplied `flow_id`. This is the exact
    /// prompt-injection scenario: an attacker-controlled chat turn calling
    /// `flow_memory_remember` with another flow's id to poison its dedup
    /// memory (e.g. mark an item as already-sent so a digest flow skips it
    /// forever).
    #[tokio::test]
    async fn remember_refuses_outside_a_trusted_workflow_run() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem.clone(), test_security());

        // No `turn_origin::with_origin` wrapper — this call has no trusted
        // Workflow run origin, exactly like every chat/orchestrator turn.
        let result = tool
            .execute(json!({
                "flow_id": "digest-flow-victim",
                "key": "sent_item_42",
                "content": "Sent newsletter item 42 to subscribers"
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result
            .output()
            .contains("only available inside a workflow run"));
        // Nothing was written to the targeted namespace — or anywhere else.
        assert!(mem
            .get(&flow_namespace("digest-flow-victim"), "sent_item_42")
            .await
            .unwrap()
            .is_none());
    }

    /// SECURITY (Fix 1): a running flow's own id — carried by the run's
    /// `TrustedAutomation { Workflow }` origin — is authoritative. A
    /// mismatched, model-supplied `flow_id` arg must be silently ignored,
    /// never allowed to redirect the write into a different flow's
    /// namespace.
    #[tokio::test]
    async fn remember_ignores_mismatched_flow_id_arg_inside_trusted_workflow_run() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem.clone(), test_security());

        let origin = AgentTurnOrigin::TrustedAutomation {
            job_id: "f-real".to_string(),
            source: TrustedAutomationSource::Workflow {
                require_approval: false,
            },
        };
        let result = turn_origin::with_origin(
            origin,
            tool.execute(json!({
                "flow_id": "f-other",
                "key": "sent_item_1",
                "content": "Sent item 1"
            })),
        )
        .await
        .unwrap();
        assert!(!result.is_error);

        // Landed in the trusted run's own namespace...
        assert!(mem
            .get(&flow_namespace("f-real"), "sent_item_1")
            .await
            .unwrap()
            .is_some());
        // ...and NOT the mismatched, model-supplied namespace.
        assert!(mem
            .get(&flow_namespace("f-other"), "sent_item_1")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn remember_blocked_in_readonly_autonomy() {
        let (_tmp, mem) = test_mem();
        let readonly = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = FlowMemoryRememberTool::new(mem.clone(), readonly);
        let result = tool
            .execute(json!({"flow_id": "f1", "key": "k", "content": "blocked"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("read-only mode"));
        assert!(mem.get(&flow_namespace("f1"), "k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remember_rejects_secret_like_content() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem.clone(), test_security());
        let result = turn_origin::with_origin(
            trusted_workflow_origin("f1"),
            tool.execute(json!({
                "flow_id": "f1",
                "key": "api",
                "content": "api_key=sk-123456789012345678901234567890"
            })),
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("looks like a secret"));
        assert!(mem
            .get(&flow_namespace("f1"), "api")
            .await
            .unwrap()
            .is_none());
    }

    /// Outside a trusted run, `flow_id` no longer matters — the T-M2 refusal
    /// fires regardless of whether it was supplied. (Inside a trusted run the
    /// arg is informational only and ignored either way — see
    /// `remember_ignores_mismatched_flow_id_arg_inside_trusted_workflow_run`.)
    #[tokio::test]
    async fn remember_missing_flow_id_outside_trusted_run_is_refused() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem, test_security());
        let result = tool
            .execute(json!({"key": "k", "content": "c"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result
            .output()
            .contains("only available inside a workflow run"));
    }

    // T-m5 (retained through the T-M2 merge): the missing-param checks run
    // BEFORE the trusted-origin resolution, so they are still reachable outside
    // a run and still assert the `ToolResult::error` channel rather than `Err`.
    #[tokio::test]
    async fn remember_missing_key_errs() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem, test_security());
        let result = tool
            .execute(json!({"flow_id": "f1", "content": "c"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Missing 'key'"));
    }

    #[tokio::test]
    async fn remember_missing_content_errs() {
        let (_tmp, mem) = test_mem();
        let tool = FlowMemoryRememberTool::new(mem, test_security());
        let result = tool
            .execute(json!({"flow_id": "f1", "key": "k"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Missing 'content'"));
    }
}
