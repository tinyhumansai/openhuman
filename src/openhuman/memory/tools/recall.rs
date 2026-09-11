use crate::openhuman::agent::tinyagents::host::agent_memory::DEFAULT_AGENT_MEMORY_NAMESPACE;
use crate::openhuman::memory::api::provider::MemoryRecall;
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write;

/// Let the agent search its own memory.
///
/// Holds no memory handle: it resolves the guarded driver per call, like every
/// other memory tool in this port. That is what lets the session builder stop
/// threading an `Arc<dyn Memory>` through tool construction.
pub struct MemoryRecallTool;

impl MemoryRecallTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for MemoryRecallTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Search memory for relevant facts: stored notes and ingested sources alike. Returns scored \
         results ranked by relevance. Searches the default scope unless a namespace is given."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords or phrase to search for in memory"
                },
                "namespace": {
                    "type": "string",
                    "description": "Namespace to search. Omit for the default scope ('global'); others include 'background', 'autocomplete', 'skill-{id}', or a connector source namespace"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return (default: 5)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let namespace = resolve_namespace(&args)?;
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?
            .trim();
        if query.is_empty() {
            return Err(anyhow::anyhow!("query cannot be empty"));
        }

        #[allow(clippy::cast_possible_truncation)]
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map_or(5, |v| v as usize);

        // Search with the user query only. Prefixing `namespace` into the query
        // string would add a redundant token matching almost every row. Instead,
        // namespace scoping belongs in RecallOpts so the backend restricts the
        // search to the correct namespace column.
        let recall_opts = crate::openhuman::memory::api::recall::OwnedRecallOpts {
            namespace: Some(namespace.to_string()),
            ..Default::default()
        };
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_recall: {e}"))?;
        // `None` scope: the guard intersects it with the ambient per-turn
        // allowlist, so this can only ever be narrowed, never widened.
        match guard.recall(query, limit, &recall_opts, None).await {
            Ok(entries) if entries.is_empty() => Ok(ToolResult::success(
                "No memories found matching that query.",
            )),
            Ok(entries) => {
                let mut output = format!("Found {} memories:\n", entries.len());
                for entry in &entries {
                    let score = entry
                        .score
                        .map_or_else(String::new, |s| format!(" [{s:.0}%]"));
                    let _ = writeln!(
                        output,
                        "- [{}] {}: {}{score}",
                        entry.category, entry.key, entry.content
                    );
                }
                Ok(ToolResult::success(output))
            }
            Err(e) => Ok(ToolResult::error(format!("Memory recall failed: {e}"))),
        }
    }
}

/// The namespace a call searches: the one it names, or the default scope when
/// it names none. An explicit empty string is a caller mistake, not a request
/// for the default — the model had a namespace in mind and lost it.
pub(crate) fn resolve_namespace(args: &serde_json::Value) -> anyhow::Result<&str> {
    // Presence first, then type: the tool path hands the model's arguments
    // over without schema validation, so a `null` or numeric namespace must
    // be refused rather than silently read as "search the default scope".
    let Some(value) = args.get("namespace") else {
        return Ok(DEFAULT_AGENT_MEMORY_NAMESPACE);
    };
    let namespace = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("namespace must be a string"))?
        .trim();
    if namespace.is_empty() {
        return Err(anyhow::anyhow!("namespace cannot be empty"));
    }
    Ok(namespace)
}

#[cfg(test)]
#[path = "recall_tests.rs"]
mod tests;
