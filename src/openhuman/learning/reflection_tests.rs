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

fn reflection_config() -> LearningConfig {
    LearningConfig {
        enabled: true,
        reflection_enabled: true,
        reflection_source: ReflectionSource::Cloud,
        max_reflections_per_session: 2,
        min_turn_complexity: 1,
        ..LearningConfig::default()
    }
}

fn reflective_turn() -> TurnContext {
    TurnContext {
        user_message: "Please debug the failing build".into(),
        assistant_response: "I inspected the logs and found the root cause.".repeat(20),
        tool_calls: vec![ToolCallRecord {
            name: "shell".into(),
            arguments: serde_json::json!({"cmd":"cargo test"}),
            success: true,
            output_summary: "tests passed".into(),
            duration_ms: 1200,
        }],
        turn_duration_ms: 2200,
        session_id: Some("session-1".into()),
        iteration_count: 2,
    }
}

#[test]
fn parse_reflection_valid_json() {
    let raw = r#"{"observations":["Tool A was effective"],"patterns":["User prefers concise output"],"user_preferences":["timezone: PST"]}"#;
    let output = ReflectionHook::parse_reflection(raw);
    assert_eq!(output.observations.len(), 1);
    assert_eq!(output.patterns.len(), 1);
    assert_eq!(output.user_preferences.len(), 1);
}

#[test]
fn parse_reflection_with_surrounding_text() {
    let raw = r#"Here is the analysis:
{"observations":["worked well"],"patterns":[],"user_preferences":[]}
That's my assessment."#;
    let output = ReflectionHook::parse_reflection(raw);
    assert_eq!(output.observations, vec!["worked well"]);
}

#[test]
fn parse_reflection_invalid_json_falls_back() {
    let raw = "This is not JSON at all";
    let output = ReflectionHook::parse_reflection(raw);
    assert_eq!(output.observations.len(), 1);
    assert!(output.observations[0].contains("not JSON"));
}

#[test]
fn slugify_produces_clean_keys() {
    assert_eq!(slugify("User prefers Rust"), "user_prefers_rust");
    assert_eq!(slugify("hello-world_test"), "hello_world_test");
}

#[test]
fn should_reflect_requires_learning_and_complexity() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let hook = ReflectionHook::new(
        reflection_config(),
        Arc::new(Config::default()),
        memory,
        None,
    );
    assert!(hook.should_reflect(&reflective_turn()));

    let mut disabled = reflection_config();
    disabled.enabled = false;
    let hook = ReflectionHook::new(
        disabled,
        Arc::new(Config::default()),
        Arc::new(MockMemory::default()),
        None,
    );
    assert!(!hook.should_reflect(&reflective_turn()));

    let mut simple = reflective_turn();
    simple.tool_calls.clear();
    simple.assistant_response = "short".into();
    let hook = ReflectionHook::new(
        reflection_config(),
        Arc::new(Config::default()),
        Arc::new(MockMemory::default()),
        None,
    );
    assert!(!hook.should_reflect(&simple));
}

#[test]
fn build_reflection_prompt_includes_tool_calls_and_truncation() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let hook = ReflectionHook::new(
        reflection_config(),
        Arc::new(Config::default()),
        memory,
        None,
    );
    let mut turn = reflective_turn();
    turn.user_message = "u".repeat(700);
    turn.assistant_response = "a".repeat(700);
    turn.tool_calls[0].output_summary = "x".repeat(200);

    let prompt = hook.build_reflection_prompt(&turn);
    assert!(prompt.contains("## User Message"));
    assert!(prompt.contains("## Assistant Response"));
    assert!(prompt.contains("## Tool Calls"));
    assert!(prompt.contains("shell (success=true, duration=1200ms):"));
    assert!(prompt.contains("Turn took 2200ms across 2 iteration(s)."));
    assert!(prompt.contains(&format!("{}...", "u".repeat(500))));
    assert!(prompt.contains(&format!("{}...", "a".repeat(500))));
    assert!(prompt.contains(&format!("{}...", "x".repeat(100))));
}

#[test]
fn session_key_and_counter_management_work() {
    let hook = ReflectionHook::new(
        reflection_config(),
        Arc::new(Config::default()),
        Arc::new(MockMemory::default()),
        None,
    );

    let global_ctx = TurnContext {
        session_id: None,
        ..reflective_turn()
    };
    assert_eq!(ReflectionHook::session_key(&global_ctx), "__global__");

    assert!(hook.try_increment("s"));
    assert!(hook.try_increment("s"));
    assert!(!hook.try_increment("s"));
    hook.rollback_increment("s");
    assert!(hook.try_increment("s"));
}

#[tokio::test]
async fn store_reflection_persists_all_categories() {
    let memory_impl = Arc::new(MockMemory::default());
    let memory: Arc<dyn Memory> = memory_impl.clone();
    let hook = ReflectionHook::new(
        reflection_config(),
        Arc::new(Config::default()),
        memory,
        None,
    );
    hook.store_reflection(&ReflectionOutput {
        observations: vec!["Observed failure".into()],
        patterns: vec!["Pattern A".into()],
        user_preferences: vec!["Pref A".into()],
    })
    .await
    .unwrap();

    let keys: Vec<String> = memory_impl.entries.lock().keys().cloned().collect();
    assert!(keys.iter().any(|key| key.starts_with("obs/")));
    assert!(keys.iter().any(|key| key == "pat/pattern_a"));
    assert!(keys.iter().any(|key| key == "pref/pref_a"));
}

#[tokio::test]
async fn on_turn_complete_rolls_back_counter_when_reflection_call_fails() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    let hook = ReflectionHook::new(
        reflection_config(),
        Arc::new(Config::default()),
        memory,
        None,
    );
    let turn = reflective_turn();

    let err = hook.on_turn_complete(&turn).await.unwrap_err();
    assert!(err.to_string().contains("no cloud provider configured"));
    assert_eq!(
        hook.session_counts
            .lock()
            .get("session-1")
            .copied()
            .unwrap_or_default(),
        0
    );
}
