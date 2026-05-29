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

pub const PROVIDER_LABEL: &str = "OpenHuman";

/// Normalize an inbound `model` argument before forwarding to the OpenHuman backend.
///
/// The backend rejects a blank `model` field with
/// `400 {"success":false,"error":"model is required"}` (Sentry **TAURI-RUST-RS**,
/// 163 events / 14d). Empty values reach this layer when a workload routes to
/// `<slug>:` with no model after the colon (see the `[config][migrate]`
/// rewrites in `src/openhuman/config/schema/load.rs:967`) or when an upstream
/// caller passes `model_override: Some("")`.
///
/// Substitute the canonical default tier so the call succeeds instead of
/// failing the wire round-trip. Mirrors the same fallback `make_openhuman_backend`
/// already applies when `default_model` is missing
/// (`src/openhuman/inference/provider/factory.rs:404`), so behavior stays
/// consistent across both entry paths.
fn resolve_model(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        // Debug-tier on purpose: the routing-migration path
        // (`config/schema/load.rs:967`) can hit this on every chat turn for
        // an affected user (~163 events / 14d on Sentry pre-fix). Warn-tier
        // here would just move the noise from Sentry to local log dashboards.
        // Per-process throttling via `Once` was considered — debug is simpler
        // and gives the same diagnostic when needed (set RUST_LOG=debug).
        log::debug!(
            "[providers][openhuman-backend] empty model passed to OpenHuman backend; \
             substituting default `{}` (TAURI-RUST-RS)",
            crate::openhuman::config::MODEL_REASONING_V1
        );
        crate::openhuman::config::MODEL_REASONING_V1.to_string()
    } else {
        trimmed.to_string()
    }
}

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
        // Fail fast when the scheduler-gate signed-out override is set
        // (sidecar saw a 401 from this backend, the user logged out, or
        // boot detected no JWT). Without this guard, every background
        // producer would still race to the network and earn a 401 each
        // time — that is exactly the failure mode that generated
        // 5,414 Sentry events on issue OPENHUMAN-TAURI-1T.
        //
        // Return a sentinel that `is_session_expired_error` matches so
        // any caller that bubbles this up to `jsonrpc.invoke_method`
        // gets the same teardown path as a real backend 401.
        if crate::openhuman::scheduler_gate::is_signed_out() {
            anyhow::bail!(
                "SESSION_EXPIRED: backend session not active — sign in to resume LLM work"
            );
        }
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
        let model = resolve_model(model);
        inner
            .chat_with_system(system_prompt, message, &model, temperature)
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
        let model = resolve_model(model);
        inner.chat_with_history(messages, &model, temperature).await
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let token = self.resolve_bearer()?;
        let inner = self.inner(&token)?;
        let model = resolve_model(model);
        inner.chat(request, &model, temperature).await
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
        // TODO(stream-support): when streaming is enabled here, route
        // `_model` through `resolve_model` before forwarding — same blank
        // model guard as the non-streaming methods (TAURI-RUST-RS).
        stream::once(async move {
            Ok(StreamChunk::error(
                "streaming is not supported for OpenHuman backend provider",
            ))
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TAURI-RUST-RS regression coverage: the OpenHuman backend rejects an
    // empty `model` field with 400 `model is required`. `resolve_model` must
    // intercept blank / whitespace-only values before they hit the wire and
    // substitute the canonical default tier.

    #[test]
    fn resolve_model_substitutes_default_for_empty() {
        assert_eq!(
            resolve_model(""),
            crate::openhuman::config::MODEL_REASONING_V1
        );
    }

    #[test]
    fn resolve_model_substitutes_default_for_whitespace_only() {
        assert_eq!(
            resolve_model("   "),
            crate::openhuman::config::MODEL_REASONING_V1
        );
        assert_eq!(
            resolve_model("\t\n"),
            crate::openhuman::config::MODEL_REASONING_V1
        );
    }

    #[test]
    fn resolve_model_trims_surrounding_whitespace() {
        assert_eq!(resolve_model("  reasoning-v1  "), "reasoning-v1");
    }

    #[test]
    fn resolve_model_preserves_non_empty_value_verbatim() {
        // Non-empty values are passed through unchanged (after trim) — no
        // canonicalisation, no remapping. The backend is authoritative over
        // which model strings it accepts.
        assert_eq!(resolve_model("agentic-v1"), "agentic-v1");
        assert_eq!(resolve_model("hint:reasoning"), "hint:reasoning");
        assert_eq!(resolve_model("some-custom-model"), "some-custom-model");
    }
}
