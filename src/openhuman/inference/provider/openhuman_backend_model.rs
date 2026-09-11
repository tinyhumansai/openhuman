//! Crate-native managed OpenHuman backend as a host [`ChatModel`] (issue #4727,
//! Motion B).
//!
//! The managed backend can't be a plain crate `OpenAiModel` preset: it uses a
//! **dynamic** session JWT (fetched per call), emits the `thread_id` extension so
//! the backend groups InferenceLog entries + aligns KV-cache keys, and relies on
//! the `openhuman.usage/billing` response envelope for charged-USD / cached-token
//! accounting. This host `ChatModel` bridges all three onto the crate wire client:
//!
//! * **Dynamic JWT** — [`invoke`](ChatModel::invoke)/[`stream`](ChatModel::stream)
//!   resolve the current bearer and build a fresh crate `OpenAiModel` (Bearer)
//!   per call.
//! * **`thread_id`** — injected into `ModelRequest.provider_options` so the crate
//!   flattens it into the request body as the top-level `thread_id` field (parity
//!   with the host `with_openhuman_thread_id`).
//! * **Billing envelope** — the crate `parse_response` preserves the full response
//!   JSON on `ModelResponse.raw` but has no field for the managed backend's
//!   charged USD, so [`project_managed_usage`] re-projects the
//!   `openhuman.{billing,usage}` envelope into the `openhuman_usage_meta` shape +
//!   crate `Usage` cache tokens the seam's `usage_info_from_response` reads —
//!   without it the crate-native managed path would report `$0` charged.
//!
//! This is the bespoke-provider rewrite that gates deleting `compatible*.rs` (the
//! managed backend was its last non-BYOK consumer).

use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

use tinyinference::message::Message;
use tinyinference::model::{
    ChatModel, Modalities, ModelProfile, ModelRequest, ModelResponse, ModelStream, ProviderError,
};
use tinyinference::providers::openai::OpenAiModel;
use tinyinference::Error as TiError;

use super::ProviderRuntimeOptions;
use crate::api::config::effective_api_url;
use crate::openhuman::agent::tinyagents::thread_context;
use crate::openhuman::security::credentials::{AuthService, APP_SESSION_PROVIDER};

pub const PROVIDER_LABEL: &str = "OpenHuman";

/// The managed OpenHuman backend as a crate [`ChatModel`]. Holds the backend
/// connection settings (for JWT + base-URL resolution) and the default model id
/// sent when a request doesn't override it.
pub struct OpenHumanBackendModel {
    options: ProviderRuntimeOptions,
    api_url: Option<String>,
    default_model: String,
    native_tool_calling: bool,
    profile: ModelProfile,
}

impl OpenHumanBackendModel {
    pub fn new(
        api_url: Option<&str>,
        options: &ProviderRuntimeOptions,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            options: options.clone(),
            api_url: api_url
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(ToOwned::to_owned),
            default_model: resolve_model(&default_model.into()),
            native_tool_calling: true,
            profile: ModelProfile {
                provider: Some("managed".to_string()),
                modalities: Modalities {
                    image_in: true,
                    ..Modalities::default()
                },
                tool_calling: true,
                parallel_tool_calls: true,
                streaming: true,
                streaming_tool_chunks: true,
                ..ModelProfile::default()
            },
        }
    }

    pub fn with_default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = resolve_model(&model.into());
        self
    }

    /// Force prompt-guided tool calling for toolsets that exceed the managed
    /// backend's native grammar ceiling.
    pub fn with_native_tool_calling(mut self, enabled: bool) -> Self {
        self.native_tool_calling = enabled;
        self.profile.tool_calling = enabled;
        self.profile.parallel_tool_calls = enabled;
        self.profile.streaming_tool_chunks = enabled;
        self
    }

    fn state_dir(&self) -> PathBuf {
        self.options.openhuman_dir.clone().unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|dirs| dirs.home_dir().join(".openhuman"))
                .unwrap_or_else(|| PathBuf::from(".openhuman"))
        })
    }

    fn resolve_bearer(&self) -> anyhow::Result<String> {
        use crate::openhuman::security::credentials::session_support::{
            classify_session_token, SessionTokenCheck,
        };

        if crate::openhuman::cron::scheduler_gate::is_signed_out() {
            anyhow::bail!(
                "SESSION_EXPIRED: backend session not active — sign in to resume LLM work"
            );
        }
        let auth = AuthService::new(&self.state_dir(), self.options.secrets_encrypt);
        let profile = auth.get_profile(
            APP_SESSION_PROVIDER,
            self.options.auth_profile_override.as_deref(),
        )?;

        // #5503: precheck the recorded JWT `exp` BEFORE building a request, the
        // same way `require_live_session_token` guards the backend REST callers.
        // Managed inference used to fire a doomed request on an expired-but-
        // stored token and let the 401 come back — but an expired session can
        // also surface upstream as a misleading "model unavailable", which is a
        // core symptom of #5503 (all tiers "die" over a long session). Failing
        // fast as `session_expired` routes the user to re-auth instead. Offline
        // / local sessions (`is_local_session_token`) and `exp`-less tokens
        // carry no recorded expiry, so `classify_session_token` returns `Live`
        // for them — their behaviour is unchanged and the post-call 401 net
        // still covers a server-side revocation.
        match classify_session_token(profile.as_ref(), chrono::Utc::now()) {
            SessionTokenCheck::Live(token) => Ok(token),
            SessionTokenCheck::Expired => {
                maybe_publish_local_session_expiry();
                anyhow::bail!(
                    "SESSION_EXPIRED: backend session token expired locally — re-authentication required"
                )
            }
            SessionTokenCheck::Absent => {
                anyhow::bail!("No backend session: store a JWT via auth (app-session)")
            }
        }
    }

    fn base_url(&self) -> String {
        format!(
            "{}/openai/v1",
            effective_api_url(&self.api_url).trim_end_matches('/')
        )
    }

    /// Resolve the current JWT + base URL and build a fresh crate `OpenAiModel`
    /// (Bearer). Rebuilt per call because the session JWT rotates.
    fn build_wire_model(&self) -> tinyinference::Result<OpenAiModel> {
        let token = self
            .resolve_bearer()
            .map_err(|e| TiError::Model(e.to_string()))?;
        let base_url = self.base_url();
        // The hosted API is chat-completions only (no `/v1/responses`); auth is a
        // plain bearer JWT. The tier/model rides `request.model`, which the backend
        // resolves — the baked default only applies when a request omits it.
        Ok(
            OpenAiModel::compatible_provider(PROVIDER_LABEL, token, base_url, &self.default_model)
                .with_native_tool_calling(self.native_tool_calling),
        )
    }

    /// Probe whether the managed backend account actually has a working
    /// inference provider configured, cheaply and without inflating usage
    /// (issue B45 — flows provider-connectivity author gate).
    ///
    /// [`build_wire_model`](Self::build_wire_model) only resolves the session
    /// JWT and builds the request client — it says nothing about whether the
    /// account has a provider API key configured server-side. That only
    /// surfaces on a real completion attempt, as an HTTP 400
    /// `{"success":false,"error":"API key not configured for provider","errorCode":"BAD_REQUEST"}`.
    /// Previously the first time a flows author found this out was mid-run,
    /// deep inside a tinyflows `agent` node. This probe moves that discovery
    /// to author time by issuing one minimal completion (`"ping"`,
    /// `max_tokens: 1`) and classifying the result.
    ///
    /// Fails OPEN on everything except a definitive client-configuration
    /// error: a 5-second timeout, a transport failure, a 5xx, or any other
    /// non-matching provider error all return `Ok(())` so a flaky backend or
    /// slow network never blocks authoring. Only a backend-confirmed "no
    /// provider configured for this account" response returns `Err` —
    /// carrying the backend's own error string so the author sees exactly
    /// what run time would have shown them.
    pub async fn probe_readiness(&self) -> Result<(), String> {
        log::debug!(
            "[flows][inference-probe] entering probe_readiness model={}",
            self.default_model
        );

        let model = match self.build_wire_model() {
            Ok(model) => model,
            Err(e) => {
                // The flows readiness gate's Layer 1 (sign-in / session
                // checks) is responsible for catching a genuinely
                // absent/expired session before this ever runs — a
                // construction failure reaching here is a race, not a
                // provider-configuration problem, so fail open rather than
                // duplicate or contradict that gate's message.
                log::debug!(
                    "[flows][inference-probe] wire model construction failed, failing open: {e}"
                );
                return Ok(());
            }
        };

        let request = ModelRequest::new(vec![Message::user("ping")]).with_max_tokens(1);

        let outcome =
            match tokio::time::timeout(Duration::from_secs(5), model.invoke(&(), request)).await {
                Ok(result) => result,
                Err(_) => {
                    log::debug!(
                        "[flows][inference-probe] model={} timed out after 5s, failing open",
                        self.default_model
                    );
                    return Ok(());
                }
            };

        match outcome {
            Ok(_) => {
                log::debug!(
                    "[flows][inference-probe] model={} probe completion succeeded — provider ready",
                    self.default_model
                );
                Ok(())
            }
            Err(TiError::Provider(err)) => {
                if is_provider_not_configured_error(&err) {
                    log::warn!(
                        "[flows][inference-probe] model={} backend reports no provider configured: {}",
                        self.default_model,
                        err.message
                    );
                    Err(err.message.clone())
                } else if err.status.is_some_and(|status| status >= 500) {
                    log::debug!(
                        "[flows][inference-probe] model={} backend {:?}, failing open: {}",
                        self.default_model,
                        err.status,
                        err.message
                    );
                    Ok(())
                } else {
                    // Any other structured provider failure (401, 429, a
                    // malformed request, …) is not the definitive "provider
                    // not configured" signal this probe exists to catch —
                    // fail open rather than risk a false-positive
                    // author-time block.
                    log::debug!(
                        "[flows][inference-probe] model={} non-definitive provider error {:?}, \
                         failing open: {}",
                        self.default_model,
                        err.status,
                        err.message
                    );
                    Ok(())
                }
            }
            Err(e) => {
                log::debug!(
                    "[flows][inference-probe] model={} transport/model error, failing open: {e}",
                    self.default_model
                );
                Ok(())
            }
        }
    }
}

/// Whether `err` is the definitive "no inference provider configured for this
/// account" signal the managed backend returns as an HTTP 400 with body
/// `{"success":false,"error":"API key not configured for provider","errorCode":"BAD_REQUEST"}`.
///
/// Deliberately narrow: matches ONLY a 400 whose message contains the specific
/// `"api key not configured for provider"` phrasing, or (as a `BAD_REQUEST`-
/// coded tolerance for message wording drift) the narrower `"not configured
/// for provider"` substring — never a bare `"not configured"`, which an
/// unrelated 400 (a malformed request naming some other unconfigured field,
/// a validation error, …) could also contain. Every other 4xx/5xx/transport
/// failure fails open (see [`OpenHumanBackendModel::probe_readiness`]'s doc).
fn is_provider_not_configured_error(err: &ProviderError) -> bool {
    if err.status != Some(400) {
        return false;
    }
    let message = err.message.to_ascii_lowercase();
    let code_is_bad_request = err
        .code
        .as_deref()
        .is_some_and(|c| c.eq_ignore_ascii_case("BAD_REQUEST"));
    message.contains("api key not configured for provider")
        || (code_is_bad_request && message.contains("not configured for provider"))
}

fn resolve_model(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
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

/// The subset of the managed backend's `openhuman` response envelope the crate
/// `Usage`/`ModelResponse` can't carry — billing + cache tokens — so it can be
/// re-projected for the host cost bridge.
#[derive(Debug, Default, serde::Deserialize)]
struct ManagedEnvelope {
    #[serde(default)]
    usage: Option<ManagedEnvelopeUsage>,
    #[serde(default)]
    billing: Option<ManagedEnvelopeBilling>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ManagedEnvelopeUsage {
    #[serde(default)]
    cached_input_tokens: Option<u64>,
    #[serde(default)]
    context_window: Option<u64>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ManagedEnvelopeBilling {
    #[serde(default)]
    charged_amount_usd: f64,
}

/// Re-project the managed `openhuman.{billing,usage}` envelope — which the crate
/// `OpenAiModel` leaves only on `ModelResponse.raw` — into the metadata the host
/// cost bridge reads: `openhuman_usage_meta` (charged USD + context window) plus a
/// crate `Usage.cache_read_tokens` reconciliation when the crate missed the
/// envelope's cached count. Parity with the legacy model-adapter path's
/// `usage_info_from_response`; without it the crate-native managed turn reports
/// `$0` charged and drops backend-reported cached tokens.
fn project_managed_usage(mut response: ModelResponse) -> ModelResponse {
    let envelope: ManagedEnvelope = response
        .raw
        .as_ref()
        .and_then(|raw| raw.get("openhuman"))
        .and_then(|oh| serde_json::from_value(oh.clone()).ok())
        .unwrap_or_default();

    let charged_amount_usd = envelope
        .billing
        .map(|b| b.charged_amount_usd)
        .unwrap_or(0.0);
    let context_window = envelope
        .usage
        .as_ref()
        .and_then(|u| u.context_window)
        .unwrap_or(0);

    // The `openhuman.usage` cached count is authoritative (the legacy `extract_usage`
    // preferred it over the standard block); backfill it when the crate's standard
    // parse produced none.
    if let (Some(usage), Some(cached)) = (
        response.usage.as_mut(),
        envelope.usage.as_ref().and_then(|u| u.cached_input_tokens),
    ) {
        if usage.cache_read_tokens == 0 {
            usage.cache_read_tokens = cached;
        }
    }

    response.raw = crate::openhuman::agent::tinyagents::model::merge_openhuman_usage_meta(
        response.raw,
        charged_amount_usd,
        context_window,
    );
    response
}

/// Inject the ambient `thread_id` (when set) into the request's
/// `provider_options` so the crate emits it as a top-level `thread_id` body field
/// — parity with the host `with_openhuman_thread_id` extension.
fn with_thread_id(mut request: ModelRequest) -> ModelRequest {
    let Some(thread_id) = thread_context::current_thread_id() else {
        return request;
    };
    let mut options = request.provider_options.clone();
    if !options.is_object() {
        options = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = options.as_object_mut() {
        map.insert("thread_id".to_string(), Value::String(thread_id));
    }
    request = request.with_provider_options(options);
    request
}

/// Publish a `SessionExpired` event when the local `exp` precheck in
/// [`resolve_bearer`](OpenHumanBackendModel::resolve_bearer) rejects an expired
/// managed session token before a request is ever sent — mirroring
/// [`require_live_session_token`](crate::openhuman::security::credentials::session_support::require_live_session_token)'s
/// pre-flight publish so the credentials subscriber clears state and the UI
/// re-auths exactly as it would on a real backend 401. Deduped via the
/// scheduler gate so N parallel managed turns in one tick don't emit N events.
fn maybe_publish_local_session_expiry() {
    if crate::openhuman::cron::scheduler_gate::is_signed_out() {
        return;
    }
    log::warn!(
        "[providers][openhuman-backend] managed session token expired locally — \
         publishing SessionExpired before any inference request"
    );
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SessionExpired {
        source: "openhuman_backend_model.resolve_bearer".to_string(),
        reason: "backend session token expired locally — re-authentication required".to_string(),
    });
}

/// Publish a `SessionExpired` event when the backend rejects a crate-native
/// model call with `401`/`403` Unauthorized — mirroring the check in
/// [`CrateBackedProvider::invoke`](super::CrateBackedProvider) which the
/// crate-native path bypasses.
fn maybe_publish_session_expired(err: &TiError, operation: &str) {
    if let TiError::Provider(pe) = err {
        if pe.provider.as_str() == "OpenHuman" && matches!(pe.status, Some(401 | 403)) {
            let reason =
                crate::openhuman::inference::provider::ops::sanitize_api_error(&pe.message);
            crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SessionExpired {
                source: format!(
                    "openhuman_backend_model.{}({})",
                    operation,
                    pe.status.unwrap_or(0)
                ),
                reason,
            });
        }
    }
}

/// Log the raw upstream failure at the managed inference dispatch boundary
/// (#5503, part d). The managed unavailability path used to surface the true
/// backend cause only after the web-chat error classifier had already collapsed
/// it to a user-facing bucket, so an operator investigating "all tiers died
/// over hours" had no record of what the backend actually returned. This is the
/// one place every managed `invoke`/`stream` failure passes through, so it's
/// where the diagnostic belongs. Structured fields (`status`/`code`/`provider`/
/// `retryable`) are low-cardinality; the message is secret-scrubbed and capped
/// by [`sanitize_api_error`] before it's logged — no tokens, no full PII.
fn log_managed_dispatch_error(err: &TiError, operation: &str) {
    match err {
        TiError::Provider(pe) => {
            log::warn!(
                "[providers][openhuman-backend] managed {operation} failed: status={:?} code={:?} provider={} retryable={} detail={}",
                pe.status,
                pe.code,
                pe.provider,
                pe.retryable,
                crate::openhuman::inference::provider::ops::sanitize_api_error(&pe.message),
            );
        }
        other => {
            log::warn!(
                "[providers][openhuman-backend] managed {operation} failed (non-provider error): {}",
                crate::openhuman::inference::provider::ops::sanitize_api_error(&other.to_string()),
            );
        }
    }
}

#[async_trait]
impl ChatModel<()> for OpenHumanBackendModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        let model = self.build_wire_model()?;
        let response = match model.invoke(state, with_thread_id(request)).await {
            Ok(response) => response,
            Err(e) => {
                log_managed_dispatch_error(&e, "invoke");
                maybe_publish_session_expired(&e, "invoke");
                return Err(e);
            }
        };
        Ok(project_managed_usage(response))
    }

    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        let model = self.build_wire_model()?;
        // NOTE (streaming billing parity): the crate SSE parser sets `raw: None`
        // on the terminal `Completed` response, so the `openhuman.billing` envelope
        // is not available to `project_managed_usage` here — a streaming managed
        // turn's charged USD falls back to the catalog cost estimate (token counts
        // survive via `UsageDelta`). The authoritative charged amount is recovered
        // on the non-streaming `invoke` path above. Restoring it for streaming
        // needs the crate to preserve the final chunk's raw JSON (tracked upstream).
        match model.stream(state, with_thread_id(request)).await {
            Ok(stream) => Ok(stream),
            Err(e) => {
                log_managed_dispatch_error(&e, "stream");
                maybe_publish_session_expired(&e, "stream");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
#[path = "openhuman_backend_model_tests.rs"]
mod tests;
