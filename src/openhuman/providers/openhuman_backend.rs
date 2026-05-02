//! Inference via the OpenHuman backend OpenAI-compatible API (`{api_url}/openai/v1/...`) using the app session JWT.
//! Session material is loaded via [`crate::openhuman::credentials`] (see also [`crate::api::jwt`] for shared helpers).

use super::compatible::{AuthStyle, OpenAiCompatibleProvider};
use super::traits::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ProviderCapabilities, StreamChunk,
    StreamOptions, StreamResult,
};
use super::ProviderRuntimeOptions;
use crate::api::config::effective_api_url;
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER};
use async_trait::async_trait;
use futures_util::stream::{self, StreamExt};
use std::path::PathBuf;

const PROVIDER_LABEL: &str = "OpenHuman";

/// Routes chat to `config.api_url` + `/openai` with `Authorization: Bearer` from the `app-session` profile.
pub struct OpenHumanBackendProvider {
    options: ProviderRuntimeOptions,
    api_url: Option<String>,
}

impl OpenHumanBackendProvider {
    pub fn new(api_url: Option<&str>, options: &ProviderRuntimeOptions) -> Self {
        Self {
            options: options.clone(),
            api_url: api_url
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.options.openhuman_dir.clone().unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|d| d.home_dir().join(".openhuman"))
                .unwrap_or_else(|| PathBuf::from(".openhuman"))
        })
    }

    fn resolve_bearer(&self) -> anyhow::Result<String> {
        let auth = AuthService::new(&self.state_dir(), self.options.secrets_encrypt);
        if let Some(t) = auth
            .get_provider_bearer_token(
                APP_SESSION_PROVIDER,
                self.options.auth_profile_override.as_deref(),
            )?
            .filter(|s| !s.trim().is_empty())
        {
            return Ok(t);
        }
        anyhow::bail!("No backend session: store a JWT via auth (app-session)")
    }

    fn base_url(&self) -> anyhow::Result<String> {
        let u = effective_api_url(&self.api_url);
        // Match app `inferenceApi` and onboard model list: `{api}/openai/v1/...`
        Ok(format!("{}/openai/v1", u.trim_end_matches('/')))
    }

    fn inner(&self, token: &str) -> anyhow::Result<OpenAiCompatibleProvider> {
        // Hosted OpenHuman API is chat-completions only; skip /v1/responses fallback so transport
        // errors stay a single clear message (fallback would duplicate the same connection failure).
        // Opt into the `thread_id` extension so the backend can group
        // InferenceLog entries and align KV-cache keys with the same
        // logical chat thread the user sees — third-party providers
        // never see this field (see `with_openhuman_thread_id`).
        Ok(OpenAiCompatibleProvider::new_no_responses_fallback(
            PROVIDER_LABEL,
            &self.base_url()?,
            Some(token),
            AuthStyle::Bearer,
        )
        .with_openhuman_thread_id())
    }
}

#[async_trait]
impl Provider for OpenHumanBackendProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let token = self.resolve_bearer()?;
        let inner = self.inner(&token)?;
        inner
            .chat_with_system(system_prompt, message, model, temperature)
            .await
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let token = self.resolve_bearer()?;
        let inner = self.inner(&token)?;
        inner.chat_with_history(messages, model, temperature).await
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let token = self.resolve_bearer()?;
        let inner = self.inner(&token)?;
        inner.chat(request, model, temperature).await
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        let token = self.resolve_bearer()?;
        let inner = self.inner(&token)?;
        inner.warmup().await
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn stream_chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
        _options: StreamOptions,
    ) -> futures_util::stream::BoxStream<'static, StreamResult<StreamChunk>> {
        stream::once(async move {
            Ok(StreamChunk::error(
                "streaming is not supported for OpenHuman backend provider",
            ))
        })
        .boxed()
    }
}
