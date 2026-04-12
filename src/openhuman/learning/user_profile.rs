//! User profile learning hook.
//!
//! Extracts user preferences from conversation turns using lightweight regex
//! patterns (e.g. "I prefer...", "always use...", "my timezone is...") and
//! stores them in the `user_profile` memory category.

use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::config::LearningConfig;
use crate::openhuman::memory::{Memory, MemoryCategory};
use async_trait::async_trait;
use std::sync::Arc;

/// Regex-based patterns that signal explicit user preferences.
const PREFERENCE_PATTERNS: &[&str] = &[
    "i prefer ",
    "i always ",
    "always use ",
    "never use ",
    "my timezone ",
    "my language ",
    "i like ",
    "i don't like ",
    "i want ",
    "i need ",
    "please always ",
    "please never ",
    "from now on ",
    "going forward ",
    "my name is ",
    "i am a ",
    "i'm a ",
    "i work ",
    "my role ",
    "my stack ",
];

/// Post-turn hook that extracts user preferences from conversations.
pub struct UserProfileHook {
    config: LearningConfig,
    memory: Arc<dyn Memory>,
}

impl UserProfileHook {
    pub fn new(config: LearningConfig, memory: Arc<dyn Memory>) -> Self {
        Self { config, memory }
    }

    /// Extract preference statements from the user message.
    fn extract_preferences(message: &str) -> Vec<String> {
        let lower = message.to_lowercase();
        let mut found = Vec::new();

        for sentence in message.split(['.', '!', '\n']) {
            let trimmed = sentence.trim();
            if trimmed.is_empty() || trimmed.len() < 10 {
                continue;
            }
            let sentence_lower = trimmed.to_lowercase();
            for pattern in PREFERENCE_PATTERNS {
                if sentence_lower.contains(pattern) {
                    found.push(trimmed.to_string());
                    break;
                }
            }
        }

        // Also check the full message for short, direct preference statements
        if found.is_empty()
            && message.trim().len() >= 15
            && (lower.starts_with("i prefer") || lower.starts_with("always use"))
        {
            found.push(message.trim().to_string());
        }

        // Deduplicate and cap
        found.truncate(5);
        found
    }

    /// Store extracted preferences in memory, deduplicating by slug.
    async fn store_preferences(&self, preferences: &[String]) -> anyhow::Result<()> {
        for pref in preferences {
            let slug = slugify(pref);
            if slug.is_empty() {
                continue;
            }
            let key = format!("pref/{slug}");

            // Check for existing entry to avoid duplicates
            if let Ok(Some(_)) = self.memory.get(&key).await {
                log::debug!("[learning] user preference already stored: {key}");
                continue;
            }

            self.memory
                .store(
                    &key,
                    pref,
                    MemoryCategory::Custom("user_profile".into()),
                    None,
                )
                .await?;
            log::info!("[learning] stored user preference: {key}");
        }
        Ok(())
    }
}

#[async_trait]
impl PostTurnHook for UserProfileHook {
    fn name(&self) -> &str {
        "user_profile"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        if !self.config.enabled || !self.config.user_profile_enabled {
            return Ok(());
        }

        let preferences = Self::extract_preferences(&ctx.user_message);
        if preferences.is_empty() {
            return Ok(());
        }

        log::debug!(
            "[learning] extracted {} preference(s) from user message",
            preferences.len()
        );
        self.store_preferences(&preferences).await
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
    use crate::openhuman::agent::hooks::TurnContext;
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
                    namespace: None,
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
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(self.entries.lock().get(key).cloned())
        }

        async fn list(
            &self,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(self.entries.lock().values().cloned().collect())
        }

        async fn forget(&self, key: &str) -> anyhow::Result<bool> {
            Ok(self.entries.lock().remove(key).is_some())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(self.entries.lock().len())
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[test]
    fn extract_preferences_finds_patterns() {
        let msg = "I prefer Rust over Python. Always use snake_case for variables.";
        let prefs = UserProfileHook::extract_preferences(msg);
        assert_eq!(prefs.len(), 2);
        assert!(prefs[0].contains("prefer"));
        assert!(prefs[1].contains("snake_case"));
    }

    #[test]
    fn extract_preferences_ignores_short_sentences() {
        let msg = "I prefer. OK.";
        let prefs = UserProfileHook::extract_preferences(msg);
        assert!(prefs.is_empty());
    }

    #[test]
    fn extract_preferences_handles_no_matches() {
        let msg = "Can you help me debug this function?";
        let prefs = UserProfileHook::extract_preferences(msg);
        assert!(prefs.is_empty());
    }

    #[test]
    fn extract_preferences_uses_full_message_fallback_and_caps_results() {
        let fallback = UserProfileHook::extract_preferences("I prefer compact diffs in code reviews");
        assert_eq!(fallback, vec!["I prefer compact diffs in code reviews"]);

        let many = UserProfileHook::extract_preferences(
            "I prefer Rust. I always use tests. Please always explain failures. \
             My timezone is PST. My stack is Tauri. Going forward use concise output. \
             Never use nested bullets.",
        );
        assert_eq!(many.len(), 5);
    }

    #[tokio::test]
    async fn store_preferences_skips_duplicates_and_empty_slugs() {
        let memory_impl = Arc::new(MockMemory::default());
        memory_impl
            .store(
                "pref/i_prefer_rust",
                "I prefer Rust",
                MemoryCategory::Custom("user_profile".into()),
                None,
            )
            .await
            .unwrap();
        let memory: Arc<dyn Memory> = memory_impl.clone();
        let hook = UserProfileHook::new(
            LearningConfig {
                enabled: true,
                user_profile_enabled: true,
                ..LearningConfig::default()
            },
            memory,
        );

        hook.store_preferences(&[
            "I prefer Rust".into(),
            "!!!".into(),
            "My timezone is PST".into(),
        ])
        .await
        .unwrap();

        let keys: Vec<String> = memory_impl.entries.lock().keys().cloned().collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"pref/i_prefer_rust".into()));
        assert!(keys.contains(&"pref/my_timezone_is_pst".into()));
    }

    #[tokio::test]
    async fn on_turn_complete_respects_feature_flags_and_stores_preferences() {
        let memory_impl = Arc::new(MockMemory::default());
        let memory: Arc<dyn Memory> = memory_impl.clone();
        let ctx = TurnContext {
            user_message: "My language is English. Please always use concise output.".into(),
            assistant_response: "Noted".into(),
            tool_calls: Vec::new(),
            turn_duration_ms: 10,
            session_id: None,
            iteration_count: 1,
        };

        let disabled = UserProfileHook::new(LearningConfig::default(), memory.clone());
        disabled.on_turn_complete(&ctx).await.unwrap();
        assert!(memory_impl.entries.lock().is_empty());

        let enabled = UserProfileHook::new(
            LearningConfig {
                enabled: true,
                user_profile_enabled: true,
                ..LearningConfig::default()
            },
            memory,
        );
        enabled.on_turn_complete(&ctx).await.unwrap();

        let values: Vec<String> = memory_impl
            .entries
            .lock()
            .values()
            .map(|entry| entry.content.clone())
            .collect();
        assert!(values.iter().any(|value| value.contains("My language is English")));
        assert!(
            values
                .iter()
                .any(|value| value.contains("Please always use concise output"))
        );
    }
}
