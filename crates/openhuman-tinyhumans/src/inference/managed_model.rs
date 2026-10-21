//! The managed TinyHumans inference model as a host [`ChatModel`] (issue #4727,
//! Motion B), owned by this crate and registered with the core's chat-model
//! registry ([`super::register`]).
//!
//! The managed backend can't be a plain crate `OpenAiModel` preset: it uses a
//! **dynamic** session JWT (fetched per call), emits the `thread_id` extension so
//! the backend groups InferenceLog entries + aligns KV-cache keys, and relies on
//! the `openhuman.usage/billing` response envelope for charged-USD / cached-token
//! accounting. This `ChatModel` bridges all three onto the crate wire client:
//!
//! * **Dynamic JWT**: `invoke`/`stream` resolve the current bearer and build a
//!   fresh crate `OpenAiModel` (Bearer) per call ([`managed_session`](super::managed_session)).
//! * **`thread_id`**: injected into `ModelRequest.provider_options` so the crate
//!   flattens it into the request body as the top-level `thread_id` field.
//! * **Billing envelope**: [`managed_usage`](super::managed_usage) re-projects the
//!   `openhuman.{billing,usage}` envelope into the shape the cost bridge reads.

use std::path::PathBuf;
use std::time::Duration;

use tinyinference_llm::message::Message;
use tinyinference_llm::model::{ChatModel, Modalities, ModelProfile, ModelRequest, ProviderError};
use tinyinference_llm::providers::openai::OpenAiModel;
use tinyinference_llm::providers::ProviderRequestOptions;
use tinyinference_llm::Error as TiError;

use openhuman_embed::__host::security::credentials::{AuthService, APP_SESSION_PROVIDER};

use super::managed_session::managed_bearer;
use crate::backend::url::effective_api_url;
use crate::client::endpoint::is_managed_endpoint_for_api_key;

/// Provider label the wire client stamps on its errors.
pub const PROVIDER_LABEL: &str = "OpenHuman";

/// Where the managed model reads its credentials from.
#[derive(Debug, Clone, Default)]
pub struct ManagedOptions {
    /// Directory holding the credential store (`auth-profiles.json`, API key).
    /// `None` resolves the process default (`~/.openhuman`).
    pub openhuman_dir: Option<PathBuf>,
    /// Whether the credential store is encrypted at rest.
    pub secrets_encrypt: bool,
    /// An auth-profile name to use instead of the default app session.
    pub auth_profile_override: Option<String>,
}

/// connection settings (for JWT + base-URL resolution) and the default model id
/// sent when a request doesn't override it.
pub struct OpenHumanBackendModel {
    options: ManagedOptions,
    api_url: Option<String>,
    pub(super) default_model: String,
    native_tool_calling: bool,
    pub(super) profile: ModelProfile,
    /// Normalized OpenHuman conversation thread. This is deliberately owned by
    /// the managed backend model: BYOK and third-party models must never see
    /// this backend-only extension.
    pub(super) thread_id: Option<String>,
    /// Frozen bearer for inspection-only calls. When set, invocation never
    /// consults mutable credential storage or publishes session state.
    pub(super) inspection_bearer: Option<String>,
}

impl OpenHumanBackendModel {
    pub fn new(
        api_url: Option<&str>,
        options: &ManagedOptions,
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
            thread_id: None,
            inspection_bearer: None,
        }
    }

    /// Attach the explicit run thread used by OpenHuman's managed inference
    /// endpoint. Blank values mean no backend thread rather than an empty wire
    /// field.
    pub fn with_thread_id(mut self, thread_id: Option<impl AsRef<str>>) -> Self {
        self.thread_id = thread_id.and_then(|thread_id| {
            let thread_id = thread_id.as_ref().trim();
            (!thread_id.is_empty()).then(|| thread_id.to_owned())
        });
        self
    }

    /// Pin a credential from the read-only inspection snapshot for subsequent
    /// calls. This model must not be used as a runtime model.
    pub fn with_inspection_bearer(mut self, bearer: String) -> Self {
        self.inspection_bearer = Some(bearer);
        self
    }

    pub(super) fn is_inspection_safe(&self) -> bool {
        self.inspection_bearer.is_some()
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

    pub(super) fn state_dir(&self) -> PathBuf {
        self.options.openhuman_dir.clone().unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|dirs| dirs.home_dir().join(".openhuman"))
                .unwrap_or_else(|| PathBuf::from(".openhuman"))
        })
    }

    pub(super) fn resolve_bearer(&self) -> anyhow::Result<String> {
        if let Some(bearer) = self.inspection_bearer.as_ref() {
            return Ok(bearer.clone());
        }
        use openhuman_embed::__host::security::credentials::session_support::classify_session_token;

        // A stored API key (library runtime) is the bearer outright: the
        // OpenAI-compatible managed endpoint accepts it as `Bearer <key>`,
        // and there is no session — so no `exp` and no signed-out state — to
        // consult.
        if let Some(key) = openhuman_embed::__host::security::credentials::api_key::get_api_key_in(
            &self.state_dir(),
            self.options.secrets_encrypt,
        )? {
            // Refuse to send the key over a plaintext channel it could leak
            // from. Scoped to this managed-key path only — `base_url()`
            // comes from the backend transport, which a library host can point
            // at anything (a BYOK/local endpoint legitimately runs over
            // plain HTTP on loopback), so this cannot tighten
            // `normalize_api_base_url` itself without breaking those.
            let endpoint = self.base_url()?;
            if !is_managed_endpoint_for_api_key(&endpoint) {
                anyhow::bail!(
                    "refusing to send the TinyHumans API key to an unmanaged or insecure \
                     endpoint: {endpoint} — use the managed backend or loopback for local testing"
                );
            }
            log::debug!(
                "[providers][openhuman-backend] authenticating managed inference with api-key"
            );
            return Ok(key);
        }
        if openhuman_embed::__host::platform::llm_gate::is_signed_out() {
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
        // fast as `session_expired` routes the user to re-auth instead.
        managed_bearer(classify_session_token(profile.as_ref(), chrono::Utc::now()))
    }

    /// The managed OpenAI-compatible endpoint, from the configured `api_url`
    /// override or this crate's backend URL defaults.
    pub(super) fn base_url(&self) -> anyhow::Result<String> {
        let base = effective_api_url(&self.api_url);
        Ok(format!("{}/openai/v1", base.trim_end_matches('/')))
    }

    /// Resolve the current JWT + base URL and build a fresh crate `OpenAiModel`
    /// (Bearer). Rebuilt per call because the session JWT rotates, but every
    /// build shares one pooled HTTP client ([`managed_inference_http_client`]),
    /// so consecutive calls reuse a warm TLS connection to the backend.
    pub(super) fn build_wire_model(&self) -> tinyinference_llm::Result<OpenAiModel> {
        let token = self
            .resolve_bearer()
            .map_err(|e| TiError::Model(e.to_string()))?;
        let base_url = self.base_url().map_err(|e| TiError::Model(e.to_string()))?;
        // The hosted API is chat-completions only (no `/v1/responses`); auth is a
        // plain bearer JWT. The tier/model rides `request.model`, which the backend
        // resolves — the baked default only applies when a request omits it.
        Ok(
            OpenAiModel::compatible_provider(PROVIDER_LABEL, token, base_url, &self.default_model)
                .with_native_tool_calling(self.native_tool_calling)
                .with_request_options(ProviderRequestOptions {
                    http: Some(managed_inference_http_client()),
                    ..ProviderRequestOptions::default()
                }),
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
            openhuman_embed::__host::config::MODEL_MANAGED_DEFAULT
        );
        openhuman_embed::__host::config::MODEL_MANAGED_DEFAULT.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Connect timeout for the shared managed-inference client; matches the
/// adapter's own default.
const MANAGED_INFERENCE_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long an idle pooled connection is kept. Below the AWS ALB's default 60s
/// idle timeout in front of the backend, so the pool never hands out a
/// connection the load balancer has already closed.
const MANAGED_INFERENCE_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(50);

/// The one HTTP client every managed-inference call in the process shares.
///
/// `OpenAiModel` otherwise builds its own `reqwest::Client` per instance, and
/// [`OpenHumanBackendModel::build_wire_model`] builds an instance per model
/// call, so each call paid a fresh TCP + TLS handshake to the backend (about
/// 150-300 ms measured to `api.tinyhumans.ai`) before its first token. A
/// `reqwest::Client` is a cheap handle over a shared connection pool, so
/// cloning it keeps connections warm across calls, turns and threads.
fn managed_inference_http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            tracing::debug!(
                idle_timeout_secs = MANAGED_INFERENCE_POOL_IDLE_TIMEOUT.as_secs(),
                "[inference][managed] building shared pooled HTTP client"
            );
            reqwest::Client::builder()
                .connect_timeout(MANAGED_INFERENCE_CONNECT_TIMEOUT)
                .pool_idle_timeout(MANAGED_INFERENCE_POOL_IDLE_TIMEOUT)
                .tcp_keepalive(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        error = %error,
                        "[inference][managed] pooled client build failed; using reqwest defaults"
                    );
                    reqwest::Client::new()
                })
        })
        .clone()
}

#[cfg(test)]
#[path = "managed_model_tests.rs"]
mod tests;
