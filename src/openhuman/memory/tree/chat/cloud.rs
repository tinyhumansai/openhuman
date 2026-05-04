//! Cloud chat provider — routes through the OpenHuman backend's
//! `/openai/v1/chat/completions` surface using the existing
//! [`crate::openhuman::providers::openhuman_backend::OpenHumanBackendProvider`].
//!
//! Used when `memory_tree.llm_backend = "cloud"` (the default). The
//! request shape is the standard OpenAI-compatible chat-completions
//! protocol, with `temperature: 0.0` and a `summarization-v1` (or
//! caller-configured) model.

use std::path::PathBuf;

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::openhuman::providers::openhuman_backend::OpenHumanBackendProvider;
use crate::openhuman::providers::traits::{ChatMessage, Provider};
use crate::openhuman::providers::ProviderRuntimeOptions;

use super::{ChatPrompt, ChatProvider};

/// Cloud-routed chat provider. Holds an [`OpenHumanBackendProvider`] and
/// forwards each [`ChatProvider::chat_for_json`] call through its
/// `chat_with_history` method.
pub struct CloudChatProvider {
    inner: OpenHumanBackendProvider,
    model: String,
    /// Cached display name `"cloud:<model>"` for logs.
    display: String,
}

impl CloudChatProvider {
    /// Build a new cloud provider against `api_url` (or the default
    /// `effective_api_url` when `None`) for `model`. The provider does NOT
    /// resolve the bearer token at construction — it does so per request,
    /// matching the existing `OpenHumanBackendProvider` contract. That way
    /// a session refresh between memory-tree calls is picked up
    /// transparently.
    ///
    /// `openhuman_dir` is the directory containing `auth-profiles.json` (i.e.
    /// the parent of `config.config_path`). Without it the inner provider
    /// would fall back to `~/.openhuman` and fail with "No backend session"
    /// on workspaces not located at the home default.
    pub fn new(
        api_url: Option<String>,
        model: String,
        openhuman_dir: Option<PathBuf>,
        secrets_encrypt: bool,
    ) -> Self {
        let opts = ProviderRuntimeOptions {
            openhuman_dir,
            secrets_encrypt,
            ..ProviderRuntimeOptions::default()
        };
        let inner = OpenHumanBackendProvider::new(api_url.as_deref(), &opts);
        let display = format!("cloud:{model}");
        Self {
            inner,
            model,
            display,
        }
    }
}

#[async_trait]
impl ChatProvider for CloudChatProvider {
    fn name(&self) -> &str {
        &self.display
    }

    async fn chat_for_json(&self, prompt: &ChatPrompt) -> Result<String> {
        log::debug!(
            "[memory_tree::chat::cloud] kind={} model={} sys_chars={} user_chars={}",
            prompt.kind,
            self.model,
            prompt.system.len(),
            prompt.user.len()
        );

        let messages = vec![
            ChatMessage::system(prompt.system.clone()),
            ChatMessage::user(prompt.user.clone()),
        ];

        let text = self
            .inner
            .chat_with_history(&messages, &self.model, prompt.temperature)
            .await
            .with_context(|| {
                format!(
                    "cloud chat request kind={} model={} failed",
                    prompt.kind, self.model
                )
            })?;

        log::debug!(
            "[memory_tree::chat::cloud] response chars={} kind={}",
            text.len(),
            prompt.kind
        );
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_includes_model() {
        let p = CloudChatProvider::new(None, "summarization-v1".into(), None, true);
        assert_eq!(p.name(), "cloud:summarization-v1");
    }

    #[test]
    fn name_changes_with_model() {
        let p = CloudChatProvider::new(None, "claude-haiku-4.5".into(), None, true);
        assert!(p.name().contains("claude-haiku-4.5"));
    }
}
