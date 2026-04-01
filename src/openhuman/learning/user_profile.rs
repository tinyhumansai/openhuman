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
}
