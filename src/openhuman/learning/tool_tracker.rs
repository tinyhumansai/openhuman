//! Tool effectiveness tracking hook.
//!
//! For each tool call in a completed turn, updates running tallies of
//! total calls, successes, failures, and average duration. Stored in the
//! `tool_effectiveness` memory category keyed by `tool/{name}`.

use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::config::LearningConfig;
use crate::openhuman::memory::{Memory, MemoryCategory};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Per-tool effectiveness stats stored in memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStats {
    pub total_calls: u64,
    pub successes: u64,
    pub failures: u64,
    pub avg_duration_ms: f64,
    #[serde(default)]
    pub common_error_patterns: Vec<String>,
}

impl Default for ToolStats {
    fn default() -> Self {
        Self {
            total_calls: 0,
            successes: 0,
            failures: 0,
            avg_duration_ms: 0.0,
            common_error_patterns: Vec::new(),
        }
    }
}

impl ToolStats {
    /// Update stats with a new tool call outcome.
    pub fn record_call(&mut self, success: bool, duration_ms: u64, error_snippet: Option<&str>) {
        self.total_calls += 1;
        if success {
            self.successes += 1;
        } else {
            self.failures += 1;
            if let Some(err) = error_snippet {
                let pattern = err.chars().take(80).collect::<String>();
                if !self.common_error_patterns.contains(&pattern) {
                    self.common_error_patterns.push(pattern);
                    // Keep only recent error patterns
                    if self.common_error_patterns.len() > 5 {
                        self.common_error_patterns.remove(0);
                    }
                }
            }
        }
        // Running average
        let prev_total = self.total_calls - 1;
        self.avg_duration_ms = (self.avg_duration_ms * prev_total as f64 + duration_ms as f64)
            / self.total_calls as f64;
    }

    /// Format stats for display.
    pub fn summary(&self) -> String {
        let success_rate = if self.total_calls > 0 {
            (self.successes as f64 / self.total_calls as f64) * 100.0
        } else {
            0.0
        };
        format!(
            "calls={} success_rate={:.0}% avg_duration={:.0}ms failures={}",
            self.total_calls, success_rate, self.avg_duration_ms, self.failures
        )
    }
}

/// Post-turn hook that tracks tool effectiveness.
pub struct ToolTrackerHook {
    config: LearningConfig,
    memory: Arc<dyn Memory>,
    /// Per-tool lock to serialize read-modify-write cycles.
    tool_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl ToolTrackerHook {
    pub fn new(config: LearningConfig, memory: Arc<dyn Memory>) -> Self {
        Self {
            config,
            memory,
            tool_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Get or create a per-tool lock.
    async fn tool_lock(&self, tool_name: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.tool_locks.lock().await;
        locks
            .entry(tool_name.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Atomically load, update, and save stats for a single tool under a lock.
    async fn update_stats(
        &self,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
        error_summary: Option<&str>,
    ) -> anyhow::Result<()> {
        let lock = self.tool_lock(tool_name).await;
        let _guard = lock.lock().await;

        let key = format!("tool/{tool_name}");
        let mut stats: ToolStats = match self.memory.get(&key).await {
            Ok(Some(entry)) => serde_json::from_str(&entry.content).unwrap_or_default(),
            _ => ToolStats::default(),
        };

        stats.record_call(success, duration_ms, error_summary);

        let content = serde_json::to_string(&stats)?;
        self.memory
            .store(
                &key,
                &content,
                MemoryCategory::Custom("tool_effectiveness".into()),
                None,
            )
            .await?;

        log::debug!(
            "[learning] tool stats updated: {tool_name} — {}",
            stats.summary()
        );
        Ok(())
    }
}

#[async_trait]
impl PostTurnHook for ToolTrackerHook {
    fn name(&self) -> &str {
        "tool_tracker"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        if !self.config.enabled || !self.config.tool_tracking_enabled {
            return Ok(());
        }

        if ctx.tool_calls.is_empty() {
            return Ok(());
        }

        for tc in &ctx.tool_calls {
            let error_summary = if !tc.success {
                Some(tc.output_summary.as_str())
            } else {
                None
            };

            if let Err(e) = self
                .update_stats(&tc.name, tc.success, tc.duration_ms, error_summary)
                .await
            {
                log::warn!(
                    "[learning] failed to update tool stats for {}: {e:#}",
                    tc.name
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_stats_record_call_updates_correctly() {
        let mut stats = ToolStats::default();
        stats.record_call(true, 100, None);
        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.successes, 1);
        assert_eq!(stats.failures, 0);
        assert_eq!(stats.avg_duration_ms, 100.0);

        stats.record_call(false, 200, Some("timeout error"));
        assert_eq!(stats.total_calls, 2);
        assert_eq!(stats.successes, 1);
        assert_eq!(stats.failures, 1);
        assert_eq!(stats.avg_duration_ms, 150.0);
        assert_eq!(stats.common_error_patterns.len(), 1);
    }

    #[test]
    fn tool_stats_summary_formats_correctly() {
        let mut stats = ToolStats::default();
        stats.record_call(true, 50, None);
        stats.record_call(true, 150, None);
        stats.record_call(false, 300, Some("err"));
        let summary = stats.summary();
        assert!(summary.contains("calls=3"));
        assert!(summary.contains("failures=1"));
    }
}
