//! Tool that lets the agent query its own tool effectiveness data.

use crate::openhuman::learning::tool_tracker::ToolStats;
use crate::openhuman::memory::{Memory, MemoryCategory};
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use std::sync::Arc;

pub struct ToolStatsTool {
    memory: Arc<dyn Memory>,
}

impl ToolStatsTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for ToolStatsTool {
    fn name(&self) -> &str {
        "tool_stats"
    }

    fn description(&self) -> &str {
        "Query effectiveness statistics for tools you have used. Returns call counts, success rates, average durations, and common error patterns. Optionally filter by tool name."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Optional: filter stats to a specific tool name. Omit to see all tracked tools."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let filter = args
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        log::debug!(
            "[tool_stats] executing query filter={:?}",
            filter.as_deref()
        );

        let entries = self
            .memory
            .list(
                Some("tool_effectiveness"),
                Some(&MemoryCategory::Custom("tool_effectiveness".into())),
                None,
            )
            .await?;

        log::debug!(
            "[tool_stats] found {} tool effectiveness entries",
            entries.len()
        );

        if entries.is_empty() {
            log::debug!("[tool_stats] no entries, returning early");
            return Ok(ToolResult::success(
                "No tool effectiveness data recorded yet.",
            ));
        }

        let mut output = String::from("## Tool Effectiveness Stats\n\n");
        let mut found = false;

        for entry in &entries {
            let tool_name = entry.key.strip_prefix("tool/").unwrap_or(&entry.key);

            if let Some(ref filter_name) = filter {
                if tool_name != filter_name {
                    continue;
                }
            }

            found = true;
            match serde_json::from_str::<ToolStats>(&entry.content) {
                Ok(stats) => {
                    let success_rate = if stats.total_calls > 0 {
                        (stats.successes as f64 / stats.total_calls as f64) * 100.0
                    } else {
                        0.0
                    };
                    output.push_str(&format!("**{}**\n", tool_name));
                    output.push_str(&format!("  Calls: {}\n", stats.total_calls));
                    output.push_str(&format!("  Success rate: {:.0}%\n", success_rate));
                    output.push_str(&format!("  Avg duration: {:.0}ms\n", stats.avg_duration_ms));
                    if stats.failures > 0 {
                        output.push_str(&format!("  Failures: {}\n", stats.failures));
                    }
                    if !stats.common_error_patterns.is_empty() {
                        output.push_str("  Recent errors:\n");
                        for err in &stats.common_error_patterns {
                            output.push_str(&format!("    - {}\n", err));
                        }
                    }
                    output.push('\n');
                }
                Err(_) => {
                    log::warn!(
                        "[tool_stats] failed to parse stats for tool '{}' (content_len={})",
                        tool_name,
                        entry.content.len()
                    );
                    output.push_str(&format!("**{}**: (unparseable stats)\n\n", tool_name));
                }
            }
        }

        if !found {
            if let Some(name) = filter {
                log::debug!("[tool_stats] filter '{name}' matched no entries");
                return Ok(ToolResult::success(format!(
                    "No effectiveness data recorded for tool '{name}'."
                )));
            }
        }

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use serde_json::json;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MockMemory {
        entries: Mutex<HashMap<String, MemoryEntry>>,
    }

    #[async_trait]
    impl Memory for MockMemory {
        fn name(&self) -> &str {
            "mock"
        }
        async fn store(
            &self,
            namespace: &str,
            key: &str,
            content: &str,
            category: MemoryCategory,
            session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            self.entries.lock().insert(
                key.to_string(),
                MemoryEntry {
                    id: key.to_string(),
                    key: key.to_string(),
                    content: content.to_string(),
                    namespace: Some(namespace.to_string()),
                    category,
                    timestamp: "now".into(),
                    session_id: session_id.map(str::to_string),
                    score: None,
                },
            );
            Ok(())
        }
        async fn recall(
            &self,
            _q: &str,
            _l: usize,
            _opts: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(vec![])
        }
        async fn get(&self, _namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(self.entries.lock().get(key).cloned())
        }
        async fn list(
            &self,
            _namespace: Option<&str>,
            _cat: Option<&MemoryCategory>,
            _s: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(self.entries.lock().values().cloned().collect())
        }
        async fn forget(&self, _namespace: &str, key: &str) -> anyhow::Result<bool> {
            Ok(self.entries.lock().remove(key).is_some())
        }
        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
            Ok(vec![])
        }
        async fn count(&self) -> anyhow::Result<usize> {
            Ok(self.entries.lock().len())
        }
        async fn health_check(&self) -> bool {
            true
        }
    }

    fn make_tool() -> ToolStatsTool {
        ToolStatsTool::new(Arc::new(MockMemory::default()))
    }

    #[test]
    fn name_is_correct() {
        assert_eq!(make_tool().name(), "tool_stats");
    }

    #[test]
    fn description_is_non_empty() {
        assert!(!make_tool().description().is_empty());
    }

    #[test]
    fn schema_is_object_type() {
        let schema = make_tool().parameters_schema();
        assert_eq!(schema["type"], "object");
    }

    #[tokio::test]
    async fn returns_no_data_message_when_empty() {
        let result = make_tool().execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("No tool effectiveness data"));
    }

    #[tokio::test]
    async fn returns_stats_for_stored_entry() {
        use crate::openhuman::learning::tool_tracker::ToolStats;
        let mem = Arc::new(MockMemory::default());
        let stats = ToolStats {
            total_calls: 5,
            successes: 4,
            failures: 1,
            avg_duration_ms: 120.0,
            common_error_patterns: vec![],
        };
        mem.store(
            "tool_effectiveness",
            "tool/shell",
            &serde_json::to_string(&stats).unwrap(),
            MemoryCategory::Custom("tool_effectiveness".into()),
            None,
        )
        .await
        .unwrap();
        let tool = ToolStatsTool::new(mem);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        let out = result.output();
        assert!(out.contains("shell"));
        assert!(out.contains("Calls: 5"));
    }

    #[tokio::test]
    async fn filter_by_tool_name_returns_no_data_when_missing() {
        use crate::openhuman::learning::tool_tracker::ToolStats;
        let mem = Arc::new(MockMemory::default());
        let stats = ToolStats {
            total_calls: 1,
            successes: 1,
            failures: 0,
            avg_duration_ms: 50.0,
            common_error_patterns: vec![],
        };
        mem.store(
            "tool_effectiveness",
            "tool/shell",
            &serde_json::to_string(&stats).unwrap(),
            MemoryCategory::Custom("tool_effectiveness".into()),
            None,
        )
        .await
        .unwrap();
        let tool = ToolStatsTool::new(mem);
        let result = tool
            .execute(json!({"tool_name": "file_read"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result
            .output()
            .contains("No effectiveness data recorded for tool 'file_read'"));
    }
}
