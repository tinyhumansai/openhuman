//! Post-turn reflection engine.
//!
//! After each qualifying turn, builds a reflection prompt, sends it to the
//! configured LLM (local Ollama or cloud reasoning model), parses structured
//! JSON output, and stores observations in memory.

use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::config::{Config, LearningConfig, ReflectionSource};
use crate::openhuman::memory::{Memory, MemoryCategory};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Structured output expected from the reflection LLM call.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReflectionOutput {
    #[serde(default)]
    pub observations: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub user_preferences: Vec<String>,
}

/// Post-turn hook that reflects on completed turns and stores observations.
pub struct ReflectionHook {
    config: LearningConfig,
    full_config: Arc<Config>,
    memory: Arc<dyn Memory>,
    provider: Option<Arc<dyn crate::openhuman::providers::Provider>>,
    /// Per-session reflection counts for throttling. Key is session_id (or "__global__").
    session_counts: Mutex<HashMap<String, usize>>,
}

impl ReflectionHook {
    pub fn new(
        config: LearningConfig,
        full_config: Arc<Config>,
        memory: Arc<dyn Memory>,
        provider: Option<Arc<dyn crate::openhuman::providers::Provider>>,
    ) -> Self {
        Self {
            config,
            full_config,
            memory,
            provider,
            session_counts: Mutex::new(HashMap::new()),
        }
    }

    fn session_key(ctx: &TurnContext) -> String {
        ctx.session_id
            .clone()
            .unwrap_or_else(|| "__global__".to_string())
    }

    /// Attempt to increment the session counter. Returns true if under the limit.
    fn try_increment(&self, session_key: &str) -> bool {
        let mut counts = self.session_counts.lock();
        let count = counts.entry(session_key.to_string()).or_insert(0);
        if *count >= self.config.max_reflections_per_session {
            log::debug!(
                "[learning] reflection throttled for session {session_key}: {count} >= {}",
                self.config.max_reflections_per_session
            );
            return false;
        }
        *count += 1;
        true
    }

    /// Rollback the session counter (e.g. on reflection failure).
    fn rollback_increment(&self, session_key: &str) {
        let mut counts = self.session_counts.lock();
        if let Some(count) = counts.get_mut(session_key) {
            *count = count.saturating_sub(1);
        }
    }

    /// Check if this turn warrants reflection (complexity check only).
    fn should_reflect(&self, ctx: &TurnContext) -> bool {
        if !self.config.enabled || !self.config.reflection_enabled {
            return false;
        }

        // Check minimum complexity
        let tool_count = ctx.tool_calls.len();
        let response_long = ctx.assistant_response.chars().count() > 500;
        tool_count >= self.config.min_turn_complexity || response_long
    }

    /// Build the reflection prompt from turn context.
    fn build_reflection_prompt(&self, ctx: &TurnContext) -> String {
        let mut prompt = String::from(
            "Analyze this completed agent turn and extract learnings.\n\
             Return a JSON object with these fields:\n\
             - \"observations\": array of strings — what worked, what failed, notable patterns\n\
             - \"patterns\": array of strings — recurring patterns worth remembering\n\
             - \"user_preferences\": array of strings — any user preferences detected\n\n\
             Keep each entry concise (one sentence). Return ONLY valid JSON, no markdown.\n\n",
        );

        prompt.push_str(&format!(
            "## User Message\n{}\n\n",
            truncate(&ctx.user_message, 500)
        ));
        prompt.push_str(&format!(
            "## Assistant Response\n{}\n\n",
            truncate(&ctx.assistant_response, 500)
        ));

        if !ctx.tool_calls.is_empty() {
            prompt.push_str("## Tool Calls\n");
            for tc in &ctx.tool_calls {
                prompt.push_str(&format!(
                    "- {} (success={}, duration={}ms): {}\n",
                    tc.name,
                    tc.success,
                    tc.duration_ms,
                    truncate(&tc.output_summary, 100)
                ));
            }
            prompt.push('\n');
        }

        prompt.push_str(&format!(
            "Turn took {}ms across {} iteration(s).\n",
            ctx.turn_duration_ms, ctx.iteration_count
        ));

        prompt
    }

    /// Call the configured LLM for reflection.
    async fn run_reflection(&self, prompt: &str) -> anyhow::Result<String> {
        match self.config.reflection_source {
            ReflectionSource::Local => {
                let service = crate::openhuman::local_ai::global(&self.full_config);
                service
                    .prompt(&self.full_config, prompt, Some(512), true)
                    .await
                    .map_err(|e| anyhow::anyhow!("local reflection failed: {e}"))
            }
            ReflectionSource::Cloud => {
                let provider = self.provider.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("no cloud provider configured for reflection")
                })?;
                provider.simple_chat(prompt, "hint:reasoning", 0.3).await
            }
        }
    }

    /// Parse the LLM response into structured reflection output.
    fn parse_reflection(raw: &str) -> ReflectionOutput {
        // Try to extract JSON from the response (may have surrounding text)
        let trimmed = raw.trim();
        let json_str = if let Some(start) = trimmed.find('{') {
            if let Some(end) = trimmed.rfind('}') {
                &trimmed[start..=end]
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        serde_json::from_str(json_str).unwrap_or_else(|_| {
            log::debug!(
                "[learning] could not parse reflection JSON, using raw text as observation"
            );
            ReflectionOutput {
                observations: vec![trimmed.to_string()],
                patterns: Vec::new(),
                user_preferences: Vec::new(),
            }
        })
    }

    /// Store reflection output in memory.
    async fn store_reflection(&self, output: &ReflectionOutput) -> anyhow::Result<()> {
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let hash = &uuid::Uuid::new_v4().to_string()[..8];

        if !output.observations.is_empty() {
            let content = output.observations.join("\n");
            let key = format!("obs/{date}/{hash}");
            self.memory
                .store(
                    "learning_observations",
                    &key,
                    &content,
                    MemoryCategory::Custom("learning_observations".into()),
                    None,
                )
                .await?;
            log::debug!(
                "[learning] stored {} observation(s) at {key}",
                output.observations.len()
            );
        }

        for pattern in &output.patterns {
            let slug = slugify(pattern);
            let key = format!("pat/{slug}");
            self.memory
                .store(
                    "learning_patterns",
                    &key,
                    pattern,
                    MemoryCategory::Custom("learning_patterns".into()),
                    None,
                )
                .await?;
        }

        // User preferences are handled by UserProfileHook, but store raw if present
        for pref in &output.user_preferences {
            let slug = slugify(pref);
            let key = format!("pref/{slug}");
            self.memory
                .store(
                    "user_profile",
                    &key,
                    pref,
                    MemoryCategory::Custom("user_profile".into()),
                    None,
                )
                .await?;
        }

        Ok(())
    }
}

#[async_trait]
impl PostTurnHook for ReflectionHook {
    fn name(&self) -> &str {
        "reflection"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        if !self.should_reflect(ctx) {
            return Ok(());
        }

        let session_key = Self::session_key(ctx);
        if !self.try_increment(&session_key) {
            return Ok(());
        }

        log::debug!("[learning] starting reflection for session={session_key}",);

        let prompt = self.build_reflection_prompt(ctx);
        let result = self.run_reflection(&prompt).await;

        let raw = match result {
            Ok(raw) => raw,
            Err(e) => {
                // Rollback the counter so failures don't consume quota
                self.rollback_increment(&session_key);
                return Err(e);
            }
        };

        let output = Self::parse_reflection(&raw);

        log::info!(
            "[learning] reflection complete: observations={} patterns={} prefs={}",
            output.observations.len(),
            output.patterns.len(),
            output.user_preferences.len()
        );

        if let Err(e) = self.store_reflection(&output).await {
            self.rollback_increment(&session_key);
            return Err(e);
        }

        Ok(())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

fn slugify(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' || c == '_' {
                Some('_')
            } else {
                None
            }
        })
        .take(40)
        .collect()
}

#[cfg(test)]
mod tests {
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
}
