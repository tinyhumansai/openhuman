//! `save_preference` — explicit two-lane user-preference capture.
//!
//! Splits a free-form preference into one of two relevance scopes:
//!
//! - **`general`** → applies to *every* reply (tone, language, identity,
//!   standing habits). Stored in [`USER_PREF_GENERAL_NAMESPACE`] and injected
//!   into the system prompt at thread start (Lane A).
//! - **`situational`** → only relevant when its topic comes up. Stored in
//!   [`USER_PREF_SITUATIONAL_NAMESPACE`] and recalled per-turn by semantic
//!   similarity to the user's message (Lane B).
//!
//! `topic` is a snake_case slug used as the storage key, so re-saving the same
//! topic overwrites the prior value (no duplicates — `ON CONFLICT REPLACE`). A
//! topic lives in exactly one scope: writing it under one namespace clears any
//! prior copy in the other so a re-categorised preference can't linger in both
//! lanes.
//!
//! Unlike the inference pipeline (`user_profile` facets), these are written
//! verbatim and immediately — they bypass the stability detector entirely.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::memory::{Memory, MemoryCategory};
use crate::openhuman::memory_store::safety;
use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

// Namespace constants live in `memory::preferences` so the write path (here),
// the system-prompt builder (Lane A), and per-turn recall (Lane B) all share a
// single definition.
pub use crate::openhuman::memory::preferences::{
    USER_PREF_GENERAL_NAMESPACE, USER_PREF_SITUATIONAL_NAMESPACE,
};

/// Relevance scope chosen by the model when saving a preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefScope {
    /// Applies to every reply regardless of topic.
    General,
    /// Only relevant when its topic relates to the current message.
    Situational,
}

impl PrefScope {
    /// Parse the `category` argument (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "general" => Some(Self::General),
            "situational" => Some(Self::Situational),
            _ => None,
        }
    }

    /// Storage namespace for this scope.
    pub fn namespace(self) -> &'static str {
        match self {
            Self::General => USER_PREF_GENERAL_NAMESPACE,
            Self::Situational => USER_PREF_SITUATIONAL_NAMESPACE,
        }
    }

    /// The opposite scope's namespace — cleared on write so a topic lives in
    /// exactly one lane.
    pub fn other_namespace(self) -> &'static str {
        match self {
            Self::General => USER_PREF_SITUATIONAL_NAMESPACE,
            Self::Situational => USER_PREF_GENERAL_NAMESPACE,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Situational => "situational",
        }
    }
}

/// Agent tool that saves an explicit user preference into the two-lane store.
pub struct SavePreferenceTool {
    memory: Arc<dyn Memory>,
    security: Arc<SecurityPolicy>,
}

impl SavePreferenceTool {
    pub fn new(memory: Arc<dyn Memory>, security: Arc<SecurityPolicy>) -> Self {
        Self { memory, security }
    }
}

#[async_trait]
impl Tool for SavePreferenceTool {
    fn name(&self) -> &str {
        "save_preference"
    }

    fn description(&self) -> &str {
        "Save a user preference so it shapes future replies. Call this when the user states or \
         asks to remember a preference. Choose `category`:\n\
         - \"general\": applies to EVERY reply regardless of topic — tone, language, identity, \
           standing habits (e.g. \"reply in British English\", \"be terse\", \"I'm in IST\", \
           \"I'm vegetarian\"). Present in every conversation.\n\
         - \"situational\": only relevant when its topic comes up (e.g. \"when writing Rust prefer \
           X\", \"be formal in emails to my manager\", \"my AWS account is Y\"). Surfaced only when \
           the user's message relates to it.\n\
         `topic` is a short snake_case slug (e.g. reply_language, email_tone_boss, cuisine); \
         re-saving the same topic overwrites the previous value — no duplicates are created."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["topic", "value", "category"],
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "Short snake_case slug naming what this preference is about, e.g. \
                                    reply_language, verbosity, cuisine, email_tone_boss. Lowercase \
                                    letters, digits, and underscores only. Re-saving the same topic \
                                    replaces the previous value."
                },
                "value": {
                    "type": "string",
                    "description": "The preference in plain language, e.g. \"Reply in British English \
                                    spelling and idiom.\""
                },
                "category": {
                    "type": "string",
                    "enum": ["general", "situational"],
                    "description": "general = applies to every reply; situational = only when the \
                                    topic is relevant to the current message."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        tracing::debug!(
            "[tool][save_preference] invoked: topic={:?} category={:?} value_len={}",
            args.get("topic").and_then(|v| v.as_str()),
            args.get("category").and_then(|v| v.as_str()),
            args.get("value")
                .and_then(|v| v.as_str())
                .map_or(0, str::len),
        );

        // Security gate — Write-level autonomy, mirroring remember_preference.
        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "save_preference")
        {
            tracing::warn!("[tool][save_preference] security gate rejected: {error}");
            return Ok(ToolResult::error(error));
        }

        // Parse category.
        let category = match args.get("category").and_then(|v| v.as_str()) {
            Some(s) => match PrefScope::parse(s) {
                Some(c) => c,
                None => {
                    return Ok(ToolResult::error(format!(
                        "invalid category {s:?}; must be \"general\" or \"situational\""
                    )));
                }
            },
            None => {
                return Ok(ToolResult::error(
                    "missing required argument: category".to_string(),
                ));
            }
        };

        // Parse topic — non-empty snake_case slug (used as the dedup key).
        let topic = match args.get("topic").and_then(|v| v.as_str()) {
            Some(t) => t.trim(),
            None => {
                return Ok(ToolResult::error(
                    "missing required argument: topic".to_string(),
                ));
            }
        };
        if topic.is_empty() {
            return Ok(ToolResult::error("topic cannot be empty".to_string()));
        }
        if !topic
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Ok(ToolResult::error(format!(
                "topic {topic:?} contains invalid characters; use only lowercase letters, digits, \
                 and underscores (snake_case)"
            )));
        }

        // Parse value — free-form, trimmed.
        let value = match args.get("value").and_then(|v| v.as_str()) {
            Some(v) => v.trim(),
            None => {
                return Ok(ToolResult::error(
                    "missing required argument: value".to_string(),
                ));
            }
        };
        if value.is_empty() {
            return Ok(ToolResult::error("value cannot be empty".to_string()));
        }
        // Same secret guard `memory_store` applies — a credential pasted as a
        // "preference" would otherwise be stored verbatim and later surfaced or
        // injected. Reject before any write.
        if safety::has_likely_secret(value) {
            tracing::warn!(
                "[tool][save_preference] rejected secret-like value topic={} value_chars={}",
                topic,
                value.len()
            );
            return Ok(ToolResult::error(
                "Refusing to store content that looks like a secret. Remove credentials or \
                 tokens and try again."
                    .to_string(),
            ));
        }

        let namespace = category.namespace();

        tracing::debug!(
            "[tool][save_preference] storing namespace={} topic={} category={} value_len={}",
            namespace,
            topic,
            category.as_str(),
            value.len()
        );

        match self
            .memory
            .store(namespace, topic, value, MemoryCategory::Core, None)
            .await
        {
            Ok(()) => {
                tracing::info!(
                    "[tool][save_preference] saved namespace={} topic={} category={}",
                    namespace,
                    topic,
                    category.as_str()
                );
                // A topic lives in exactly one scope. Now that the new write has
                // succeeded, clear any prior copy in the other namespace so a
                // re-categorised preference doesn't linger in both lanes. Done
                // *after* the store (not before) so a store failure can never
                // leave the user with neither copy.
                if let Err(e) = self.memory.forget(category.other_namespace(), topic).await {
                    tracing::debug!(
                        "[tool][save_preference] clearing other-scope copy failed (non-fatal) ns={} topic={}: {e}",
                        category.other_namespace(),
                        topic
                    );
                }
                // Surface semantically-related existing preferences so the chat
                // agent (which captured this preference) can spot and resolve a
                // contradiction itself — no separate model call.
                let related = crate::openhuman::memory::preferences::recall_related_preferences(
                    &self.memory,
                    value,
                    topic,
                    4,
                )
                .await;
                let mut msg = format!("Saved {} preference: {topic} = {value}", category.as_str());
                if !related.is_empty() {
                    tracing::info!(
                        "[tool][save_preference] {} related preference(s) surfaced for contradiction check",
                        related.len()
                    );
                    msg.push_str(
                        "\n\nExisting preferences related to this one — check for contradictions:",
                    );
                    for (other_topic, other_value) in &related {
                        msg.push_str(&format!("\n- {other_topic}: {other_value}"));
                    }
                    msg.push_str(
                        "\n\nIf any of these conflicts with what was just saved, resolve it now: \
                         overwrite that topic with save_preference, or remove it with memory_forget. \
                         Otherwise leave them as-is.",
                    );
                }
                Ok(ToolResult::success(msg))
            }
            Err(e) => {
                tracing::error!(
                    "[tool][save_preference] failed to store namespace={} topic={}: {e:#}",
                    namespace,
                    topic
                );
                Ok(ToolResult::error(format!("Failed to save preference: {e}")))
            }
        }
    }
}

#[cfg(test)]
#[path = "save_preference_tests.rs"]
mod tests;
