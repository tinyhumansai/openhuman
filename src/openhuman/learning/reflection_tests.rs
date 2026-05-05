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
        // user_reflections are intentionally persisted by
        // `on_turn_complete` (not `store_reflection`) so they share a
        // per-turn dedupe set with the heuristic fast-path. This test
        // therefore only asserts the observation / pattern / preference
        // contracts owned by `store_reflection`; the reflection
        // persistence contract is covered by the dedupe + heuristic
        // tests below.
        user_reflections: vec!["should not be written by store_reflection".into()],
    })
    .await
    .unwrap();

    let keys: Vec<String> = memory_impl.entries.lock().keys().cloned().collect();
    assert!(keys.iter().any(|key| key.starts_with("obs/")));
    assert!(keys.iter().any(|key| key == "pat/pattern_a"));
    assert!(keys.iter().any(|key| key == "pref/pref_a"));
    assert!(
        !keys.iter().any(|key| key.starts_with("ref/")),
        "store_reflection must not persist user_reflections — that path now lives in on_turn_complete so the dedupe set is shared with the heuristic"
    );
}

#[tokio::test]
async fn persist_reflection_writes_to_dedicated_namespace_and_category() {
    let memory_impl = Arc::new(MockMemory::default());
    let memory: Arc<dyn Memory> = memory_impl.clone();
    let hook = ReflectionHook::new(
        reflection_config(),
        Arc::new(Config::default()),
        memory,
        None,
    );

    hook.persist_reflection("I want shorter answers going forward")
        .await
        .unwrap();

    let entries = memory_impl.entries.lock();
    let reflection = entries
        .values()
        .find(|e| e.key.starts_with("ref/"))
        .expect("reflection entry");
    assert_eq!(reflection.namespace.as_deref(), Some(REFLECTIONS_NAMESPACE));
    assert!(matches!(
        reflection.category,
        MemoryCategory::Custom(ref tag) if tag == REFLECTIONS_NAMESPACE
    ));
    assert_eq!(reflection.content, "I want shorter answers going forward");
}

#[tokio::test]
async fn on_turn_complete_dedupes_reflections_across_heuristic_and_llm_paths() {
    use crate::openhuman::providers::Provider;
    use async_trait::async_trait;

    // Stub provider returning a reflection LLM response whose
    // `user_reflections` array repeats the same sentence the heuristic
    // would also lift out of the user message. Only `chat_with_system`
    // needs implementing — `simple_chat` (the call-site used by
    // `ReflectionHook::run_reflection` for the cloud path) has a
    // default trait impl that delegates here.
    struct StubProvider;
    #[async_trait]
    impl Provider for StubProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(r#"{"observations":[],"patterns":[],"user_preferences":[],
                "user_reflections":["Going forward I want concise replies"]}"#
                .into())
        }
    }

    let memory_impl = Arc::new(MockMemory::default());
    let memory: Arc<dyn Memory> = memory_impl.clone();
    let hook = ReflectionHook::new(
        reflection_config(),
        Arc::new(Config::default()),
        memory,
        Some(Arc::new(StubProvider)),
    );

    let turn = TurnContext {
        // Heuristic captures this sentence; the stub LLM also returns
        // the same sentence in `user_reflections`. Without per-turn
        // dedupe both paths would write it.
        user_message: "Going forward I want concise replies.".into(),
        assistant_response: "noted".repeat(120),
        tool_calls: Vec::new(),
        turn_duration_ms: 50,
        session_id: Some("dedupe".into()),
        iteration_count: 1,
    };
    hook.on_turn_complete(&turn).await.unwrap();

    let ref_count = memory_impl
        .entries
        .lock()
        .values()
        .filter(|e| e.key.starts_with("ref/"))
        .count();
    assert_eq!(
        ref_count, 1,
        "reflection captured by both heuristic and LLM paths must be persisted exactly once"
    );
}

#[test]
fn parse_reflection_extracts_user_reflections_field() {
    let raw = r#"{"observations":[],"patterns":[],"user_preferences":[],
        "user_reflections":["I realized I want to focus on Rust this quarter"]}"#;
    let output = ReflectionHook::parse_reflection(raw);
    assert_eq!(
        output.user_reflections,
        vec!["I realized I want to focus on Rust this quarter"]
    );
}

#[test]
fn parse_reflection_defaults_user_reflections_when_absent() {
    let raw = r#"{"observations":["x"],"patterns":[],"user_preferences":[]}"#;
    let output = ReflectionHook::parse_reflection(raw);
    assert!(output.user_reflections.is_empty());
}

#[test]
fn extract_reflection_cues_picks_up_explicit_self_statements() {
    let msg = "I realized I prefer terse answers. Going forward, please skip the disclaimers.";
    let cues = extract_reflection_cues(msg);
    assert_eq!(cues.len(), 2);
    assert!(cues[0].to_ascii_lowercase().contains("i realized"));
    assert!(cues[1].to_ascii_lowercase().contains("going forward"));
}

#[test]
fn extract_reflection_cues_ignores_messages_without_cues() {
    let msg = "What is the weather today? Also, can you summarise this PR?";
    assert!(extract_reflection_cues(msg).is_empty());
}

#[test]
fn extract_reflection_cues_dedupes_identical_sentences() {
    let msg = "Remember that I work in PST. Remember that I work in PST.";
    let cues = extract_reflection_cues(msg);
    assert_eq!(cues.len(), 1);
}

#[tokio::test]
async fn on_turn_complete_persists_heuristic_reflection_even_when_complexity_low() {
    let memory_impl = Arc::new(MockMemory::default());
    let memory: Arc<dyn Memory> = memory_impl.clone();
    // Pin the source to local + threshold high so the LLM path is
    // skipped and we observe ONLY the heuristic capture.
    let mut cfg = reflection_config();
    cfg.min_turn_complexity = 99;
    cfg.reflection_source = ReflectionSource::Local;
    let hook = ReflectionHook::new(cfg, Arc::new(Config::default()), memory, None);

    let turn = TurnContext {
        user_message: "Going forward I want concise replies only.".into(),
        assistant_response: "ok".into(),
        tool_calls: Vec::new(),
        turn_duration_ms: 10,
        session_id: Some("s".into()),
        iteration_count: 1,
    };
    // The LLM path is gated off by complexity, so the call returns Ok
    // even without a provider — only the heuristic should write.
    hook.on_turn_complete(&turn).await.unwrap();

    let keys: Vec<String> = memory_impl.entries.lock().keys().cloned().collect();
    assert!(
        keys.iter().any(|k| k.starts_with("ref/")),
        "heuristic capture should persist a reflection without LLM round-trip"
    );
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
