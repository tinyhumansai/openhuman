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
use std::sync::Arc;

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
}

impl ToolTrackerHook {
    pub fn new(config: LearningConfig, memory: Arc<dyn Memory>) -> Self {
        Self { config, memory }
    }

    /// Load existing stats for a tool, or return defaults.
    async fn load_stats(&self, tool_name: &str) -> ToolStats {
        let key = format!("tool/{tool_name}");
        match self.memory.get(&key).await {
            Ok(Some(entry)) => serde_json::from_str(&entry.content).unwrap_or_default(),
            _ => ToolStats::default(),
        }
    }

    /// Save updated stats for a tool.
    async fn save_stats(&self, tool_name: &str, stats: &ToolStats) -> anyhow::Result<()> {
        let key = format!("tool/{tool_name}");
        let content = serde_json::to_string(stats)?;
        self.memory
            .store(
                &key,
                &content,
                MemoryCategory::Custom("tool_effectiveness".into()),
                None,
            )
            .await
    }

    /// Atomically update stats with retry loop to handle concurrent updates.
    async fn update_stats_atomic(
        &self,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
        error_snippet: Option<&str>,
    ) -> anyhow::Result<()> {
        const MAX_RETRIES: usize = 5;
        let mut attempt = 0;

        loop {
            // Load current stats
            let mut stats = self.load_stats(tool_name).await;

            // Apply the update
            stats.record_call(success, duration_ms, error_snippet);

            // Attempt to save
            match self.save_stats(tool_name, &stats).await {
                Ok(_) => {
                    tracing::debug!(
                        tool_name = %tool_name,
                        attempt = attempt + 1,
                        "[learning] tool stats updated: {}",
                        stats.summary()
                    );
                    return Ok(());
                }
                Err(e) if attempt < MAX_RETRIES => {
                    attempt += 1;
                    tracing::debug!(
                        tool_name = %tool_name,
                        attempt = attempt,
                        error = %e,
                        "[learning] retrying tool stats update due to potential race"
                    );
                    // Small backoff
                    tokio::time::sleep(tokio::time::Duration::from_millis(10 * attempt as u64))
                        .await;
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        tool_name = %tool_name,
                        attempts = attempt + 1,
                        error = %e,
                        "[learning] failed to save tool stats after retries"
                    );
                    return Err(e);
                }
            }
        }
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
            let error_snippet = if !tc.success {
                Some(tc.output_snippet.as_str())
            } else {
                None
            };

            // Use atomic update to prevent lost updates from concurrent turns
            if let Err(e) = self
                .update_stats_atomic(&tc.name, tc.success, tc.duration_ms, error_snippet)
                .await
            {
                tracing::warn!(
                    tool_name = %tc.name,
                    error = %e,
                    "[learning] failed to update tool stats"
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