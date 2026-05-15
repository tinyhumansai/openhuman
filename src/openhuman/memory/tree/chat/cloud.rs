//! Cloud chat provider — routes through the OpenHuman backend's
//! `/openai/v1/chat/completions` surface using the existing
//! [`crate::openhuman::providers::openhuman_backend::OpenHumanBackendProvider`].
//!
//! Used when `memory_tree.llm_backend = "cloud"` (the default). The
//! request shape is the standard OpenAI-compatible chat-completions
//! protocol, with `temperature: 0.0` and a `summarization-v1` (or
//! caller-configured) model.
//!
//! When the configured model is unavailable for the user's organization,
//! the provider automatically falls back through a list of known
//! summarization-capable models before giving up.

use std::path::PathBuf;

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::openhuman::providers::openhuman_backend::OpenHumanBackendProvider;
use crate::openhuman::providers::traits::{ChatMessage, Provider};
use crate::openhuman::providers::ProviderRuntimeOptions;

use super::{ChatPrompt, ChatProvider};

/// Fallback models tried in order when the configured model is unavailable.
const FALLBACK_MODELS: &[&str] = &[
    "summarization-v1",
    "deepseek-ai/DeepSeek-V3-0324",
    "deepseek-ai/DeepSeek-V3",
];

/// Returns true if the error indicates the model is not provisioned for the org.
/// Only matches the explicit "not available for your organization" phrase from
/// the GMI API — generic 404s are NOT treated as model-unavailable to avoid
/// masking unrelated backend failures.
fn is_model_unavailable_error(err: &anyhow::Error) -> bool {
    let msg = format!("{err:?}");
    msg.contains("not available for your organization")
}

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

    /// Try a single model, returning Ok(text) or the error.
    async fn try_model(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> Result<String> {
        self.inner
            .chat_with_history(messages, model, temperature)
            .await
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

        // Try the configured model first.
        match self
            .try_model(&messages, &self.model, prompt.temperature)
            .await
        {
            Ok(text) => {
                log::debug!(
                    "[memory_tree::chat::cloud] response chars={} kind={}",
                    text.len(),
                    prompt.kind
                );
                return Ok(text);
            }
            Err(e) if is_model_unavailable_error(&e) => {
                log::warn!(
                    "[memory_tree::chat::cloud] model={} unavailable, trying fallbacks",
                    self.model
                );
            }
            Err(e) => {
                log::warn!(
                    "[memory_tree::chat::cloud] model={} failed kind={} err={:#}",
                    self.model,
                    prompt.kind,
                    e
                );
                return Err(e).with_context(|| {
                    format!(
                        "cloud chat request kind={} model={} failed",
                        prompt.kind, self.model
                    )
                });
            }
        }

        // Fallback chain — skip the configured model if it appears in the list.
        for &fallback in FALLBACK_MODELS {
            if fallback == self.model {
                continue;
            }
            log::debug!(
                "[memory_tree::chat::cloud] trying fallback model={}",
                fallback
            );
            match self
                .try_model(&messages, fallback, prompt.temperature)
                .await
            {
                Ok(text) => {
                    log::info!(
                        "[memory_tree::chat::cloud] fallback model={} succeeded kind={}",
                        fallback,
                        prompt.kind
                    );
                    return Ok(text);
                }
                Err(e) if is_model_unavailable_error(&e) => {
                    log::debug!(
                        "[memory_tree::chat::cloud] fallback model={} also unavailable",
                        fallback
                    );
                    continue;
                }
                Err(e) => {
                    log::warn!(
                        "[memory_tree::chat::cloud] fallback model={} failed kind={} err={:#}",
                        fallback,
                        prompt.kind,
                        e
                    );
                    return Err(e).with_context(|| {
                        format!(
                            "cloud chat request kind={} fallback model={} failed",
                            prompt.kind, fallback
                        )
                    });
                }
            }
        }

        log::warn!(
            "[memory_tree::chat::cloud] configured model={} and all fallbacks unavailable kind={}",
            self.model,
            prompt.kind
        );
        anyhow::bail!(
            "cloud chat kind={}: configured model '{}' and all fallback models are unavailable \
             for this organization",
            prompt.kind,
            self.model
        )
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

    #[test]
    fn detects_model_unavailable_error() {
        let err = anyhow::anyhow!(
            "OpenHuman API error (404 Not Found): {{\"success\":false,\"error\":\"GMI model \
             'deepseek-ai/DeepSeek-V4-Flash' is not available for your organization.\"}}"
        );
        assert!(is_model_unavailable_error(&err));
    }

    #[test]
    fn non_model_error_not_detected_as_unavailable() {
        let err = anyhow::anyhow!("connection timeout after 30s");
        assert!(!is_model_unavailable_error(&err));
    }

    #[test]
    fn generic_404_with_model_not_treated_as_unavailable() {
        // A generic 404 mentioning "model" should NOT trigger fallback —
        // only the explicit "not available for your organization" phrase should.
        let err =
            anyhow::anyhow!("OpenHuman API error (404 Not Found): model endpoint returned 404");
        assert!(!is_model_unavailable_error(&err));
    }

    #[test]
    fn fallback_list_contains_summarization_v1() {
        assert!(FALLBACK_MODELS.contains(&"summarization-v1"));
    }

    #[test]
    fn fallback_list_not_empty() {
        assert!(!FALLBACK_MODELS.is_empty());
    }
}
