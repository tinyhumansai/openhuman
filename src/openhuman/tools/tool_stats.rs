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
