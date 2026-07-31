use crate::openhuman::memory::Memory;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write;
use std::sync::Arc;

/// Let the agent search its own memory
pub struct MemoryRecallTool {
    memory: Arc<dyn Memory>,
}

impl MemoryRecallTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Search memory for relevant facts in a namespace. Returns scored results ranked by relevance."
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
                    "description": "Namespace to search (e.g. 'global', 'background', 'autocomplete', or 'skill-{id}'). OMIT THIS to search everywhere — 'global' plus connected-app memories like 'skill-gmail'. Only name one when you specifically want to exclude the others."
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
        // An omitted namespace means "search everywhere". Requiring one made
        // every recall a guess the model usually got wrong: it reads `global`
        // first in the schema, picks it, and connector memories — where synced
        // mail actually lives — are structurally unreachable.
        let namespace = args
            .get("namespace")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|namespace| !namespace.is_empty());
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
        let entries = match namespace {
            Some(namespace) => {
                let recall_opts = crate::openhuman::memory::RecallOpts {
                    namespace: Some(namespace),
                    ..crate::openhuman::memory::RecallOpts::default()
                };
                match self.memory.recall(query, limit, recall_opts).await {
                    Ok(entries) => entries,
                    Err(e) => return Ok(ToolResult::error(format!("Memory recall failed: {e}"))),
                }
            }
            None => {
                // The schema says omitting the namespace searches everywhere,
                // so it has to search everywhere — the per-turn cap belongs to
                // the automatic context path, not to a call the model made.
                crate::openhuman::memory::auto_recall::recall_every_namespace(
                    self.memory.as_ref(),
                    query,
                    limit,
                )
                .await
            }
        };

        if entries.is_empty() {
            return Ok(ToolResult::success(
                "No memories found matching that query.",
            ));
        }
        let mut output = format!("Found {} memories:\n", entries.len());
        for entry in &entries {
            // Percent, not the raw 0–1 score: rendering 0.68 as "[1%]" told the
            // model every hit was worthless and pushed it to re-search live
            // sources it had just been handed the answer from.
            let score = entry
                .score
                .map_or_else(String::new, |s| format!(" [{:.0}%]", s * 100.0));
            // Name the namespace when it isn't the default one, so a
            // search-everywhere result says which connector it came from.
            let origin = entry
                .namespace
                .as_deref()
                .filter(|namespace| {
                    *namespace != crate::openhuman::memory::store::types::GLOBAL_NAMESPACE
                })
                .map_or_else(String::new, |namespace| format!(" ({namespace})"));
            // Condense to the query-relevant chunks, capped: recall returns
            // whole documents, and one large document (a synced thread, or an
            // old subagent envelope) would otherwise fill the entire result and
            // bury every other hit.
            let content = crate::openhuman::memory::recall_shaping::condense_recall_content(
                query,
                &entry.content,
            );
            let _ = writeln!(
                output,
                "- [{}]{origin} {}: {content}{score}",
                entry.category, entry.key
            );
        }
        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::embeddings::NoopEmbedding;
    use crate::openhuman::memory::store::UnifiedMemory;
    use crate::openhuman::memory::MemoryCategory;
    use tempfile::TempDir;

    fn seeded_mem() -> (TempDir, Arc<dyn Memory>) {
        let tmp = TempDir::new().unwrap();
        let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        (tmp, Arc::new(mem))
    }

    #[tokio::test]
    async fn recall_empty() {
        let (_tmp, mem) = seeded_mem();
        let tool = MemoryRecallTool::new(mem);
        let result = tool
            .execute(json!({"namespace": "global", "query": "anything"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("No memories found"));
    }

    #[tokio::test]
    async fn recall_finds_match() {
        let (_tmp, mem) = seeded_mem();
        mem.store(
            "global",
            "lang",
            "User prefers Rust",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();
        mem.store(
            "global",
            "tz",
            "Timezone is EST",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();

        let tool = MemoryRecallTool::new(mem);
        let result = tool
            .execute(json!({"namespace": "global", "query": "Rust"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("Rust"));
        assert!(result.output().contains("Found 1"));
    }

    #[tokio::test]
    async fn recall_respects_limit() {
        let (_tmp, mem) = seeded_mem();
        for i in 0..10 {
            mem.store(
                "global",
                &format!("k{i}"),
                &format!("Rust fact {i}"),
                MemoryCategory::Core,
                None,
            )
            .await
            .unwrap();
        }

        let tool = MemoryRecallTool::new(mem);
        let result = tool
            .execute(json!({"namespace": "global", "query": "Rust", "limit": 3}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("Found 3"));
    }

    #[tokio::test]
    async fn recall_missing_query() {
        let (_tmp, mem) = seeded_mem();
        let tool = MemoryRecallTool::new(mem);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[test]
    fn name_and_schema() {
        let (_tmp, mem) = seeded_mem();
        let tool = MemoryRecallTool::new(mem);
        assert_eq!(tool.name(), "memory_recall");
        assert!(tool.parameters_schema()["properties"]["query"].is_object());
    }

    #[test]
    fn namespace_is_optional_so_the_model_is_not_forced_to_guess_one() {
        // Requiring a namespace made every recall a guess: the model reads
        // `global` first, picks it, and connector memories are unreachable.
        let (_tmp, mem) = seeded_mem();
        let tool = MemoryRecallTool::new(mem);
        let required = tool.parameters_schema()["required"].clone();
        assert_eq!(required, json!(["query"]));
    }

    #[tokio::test]
    async fn omitting_the_namespace_searches_connector_memories_too() {
        let (_tmp, mem) = seeded_mem();
        mem.store(
            "skill-gmail",
            "gmail:1",
            "Boulder visit confirmed with the University of Colorado",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();

        let tool = MemoryRecallTool::new(mem);
        let result = tool
            .execute(json!({"query": "University of Colorado"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        let output = result.output();
        assert!(
            output.contains("Boulder visit"),
            "a connector memory must be reachable without naming its namespace: {output}"
        );
        // …and the result says where it came from, so the model can tell a
        // connector hit from a chat one.
        assert!(output.contains("(skill-gmail)"), "{output}");
    }

    #[tokio::test]
    async fn scores_render_as_real_percentages() {
        let (_tmp, mem) = seeded_mem();
        mem.store(
            "global",
            "lang",
            "User prefers Rust",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();

        let tool = MemoryRecallTool::new(mem);
        let output = tool
            .execute(json!({"namespace": "global", "query": "Rust"}))
            .await
            .unwrap()
            .output();

        // A 0–1 score printed with a `%` sign made every hit read as "[0%]" or
        // "[1%]" — the model saw worthless results and went back to the live
        // source it had just been given the answer from.
        assert!(
            !output.contains("[0%]") && !output.contains("[1%]"),
            "a matching hit must not render as 0–1%: {output}"
        );
        assert!(output.contains('%'), "the score is still shown: {output}");
    }
}
