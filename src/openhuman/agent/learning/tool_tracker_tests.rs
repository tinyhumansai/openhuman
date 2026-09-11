use super::*;
use crate::openhuman::agent::hooks::{ToolCallRecord, TurnContext};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use async_trait::async_trait;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

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
                taint: Default::default(),
            },
        );
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: crate::openhuman::memory::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(self.entries.lock().get(key).cloned())
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(self.entries.lock().values().cloned().collect())
    }

    async fn forget(&self, _namespace: &str, key: &str) -> anyhow::Result<bool> {
        Ok(self.entries.lock().remove(key).is_some())
    }

    async fn namespace_summaries(
        &self,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.entries.lock().len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

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

#[test]
fn tool_stats_keeps_only_recent_unique_error_patterns() {
    let mut stats = ToolStats::default();
    for idx in 0..7 {
        stats.record_call(false, 10, Some(&format!("error pattern {idx}")));
    }
    stats.record_call(false, 10, Some("error pattern 6"));

    assert_eq!(stats.failures, 8);
    assert_eq!(stats.common_error_patterns.len(), 5);
    assert_eq!(
        stats.common_error_patterns.first().unwrap(),
        "error pattern 2"
    );
    assert_eq!(
        stats.common_error_patterns.last().unwrap(),
        "error pattern 6"
    );
}

#[tokio::test]
async fn update_stats_merges_with_existing_memory_entry() {
    let memory_impl = Arc::new(MockMemory::default());
    memory_impl
        .store(
            "tool_effectiveness",
            "tool/shell",
            &serde_json::to_string(&ToolStats {
                total_calls: 2,
                successes: 1,
                failures: 1,
                avg_duration_ms: 50.0,
                common_error_patterns: vec!["timeout".into()],
            })
            .unwrap(),
            MemoryCategory::Custom("tool_effectiveness".into()),
            None,
        )
        .await
        .unwrap();

    let memory: Arc<dyn Memory> = memory_impl.clone();
    let hook = ToolTrackerHook::new(
        LearningConfig {
            enabled: true,
            tool_tracking_enabled: true,
            ..LearningConfig::default()
        },
        memory,
    );

    hook.update_stats("shell", true, 250, None).await.unwrap();

    let stored = memory_impl
        .get("tool_effectiveness", "tool/shell")
        .await
        .unwrap()
        .unwrap();
    let parsed: ToolStats = serde_json::from_str(&stored.content).unwrap();
    assert_eq!(parsed.total_calls, 3);
    assert_eq!(parsed.successes, 2);
    assert_eq!(parsed.failures, 1);
    assert!((parsed.avg_duration_ms - 116.66666666666667).abs() < 0.001);
}

#[tokio::test]
async fn on_turn_complete_skips_when_disabled_or_no_tools() {
    let memory_impl = Arc::new(MockMemory::default());
    let memory: Arc<dyn Memory> = memory_impl.clone();
    let hook = ToolTrackerHook::new(LearningConfig::default(), memory);
    let ctx = TurnContext {
        user_message: "hello".into(),
        assistant_response: "world".into(),
        tool_calls: Vec::new(),
        turn_duration_ms: 10,
        session_id: None,
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    };

    hook.on_turn_complete(&ctx).await.unwrap();
    assert!(memory_impl.entries.lock().is_empty());
}

#[tokio::test]
async fn on_turn_complete_records_each_tool_call() {
    let memory_impl = Arc::new(MockMemory::default());
    let memory: Arc<dyn Memory> = memory_impl.clone();
    let hook = ToolTrackerHook::new(
        LearningConfig {
            enabled: true,
            tool_tracking_enabled: true,
            ..LearningConfig::default()
        },
        memory,
    );
    let ctx = TurnContext {
        user_message: "hello".into(),
        assistant_response: "world".into(),
        tool_calls: vec![
            ToolCallRecord {
                name: "shell".into(),
                arguments: serde_json::json!({}),
                success: true,
                output_summary: "ok".into(),
                duration_ms: 100,
            },
            ToolCallRecord {
                name: "shell".into(),
                arguments: serde_json::json!({}),
                success: false,
                output_summary: "permission denied while writing".into(),
                duration_ms: 200,
            },
        ],
        turn_duration_ms: 20,
        session_id: None,
        agent_id: None,
        entrypoint: None,
        iteration_count: 1,
    };

    hook.on_turn_complete(&ctx).await.unwrap();

    let stored = memory_impl
        .get("tool_effectiveness", "tool/shell")
        .await
        .unwrap()
        .unwrap();
    let parsed: ToolStats = serde_json::from_str(&stored.content).unwrap();
    assert_eq!(parsed.total_calls, 2);
    assert_eq!(parsed.successes, 1);
    assert_eq!(parsed.failures, 1);
    assert_eq!(
        parsed.common_error_patterns,
        vec!["permission denied while writing"]
    );
}
