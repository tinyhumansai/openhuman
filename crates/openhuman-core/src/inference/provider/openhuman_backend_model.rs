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

use tinyinference_llm::message::Message;
use tinyinference_llm::model::{
    ChatModel, Modalities, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
    ProviderError,
};
use tinyinference_llm::providers::openai::OpenAiModel;
use tinyinference_llm::providers::ProviderRequestOptions;
use tinyinference_llm::Error as TiError;

use super::ProviderRuntimeOptions;
use crate::security::credentials::{AuthService, APP_SESSION_PROVIDER};

pub const PROVIDER_LABEL: &str = "OpenHuman";

fn is_loopback_host(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    }
}

/// Whether `endpoint` is safe to carry the TinyHumans API key as a bearer.
///
/// `https://` always qualifies; plain `http://` only for loopback, matching
/// `openhuman_embed::turn::is_safe_endpoint_for_bearer`'s allowance for local
/// testing against a dev server. Anything else — a plaintext non-loopback
/// endpoint — would put the key on the wire in the clear (CWE-319), so
/// [`OpenHumanBackendModel::resolve_bearer`] refuses before it gets there.
/// Deliberately narrow to the managed-key bearer path: `normalize_api_base_url`
/// itself must stay permissive, because a library host's BYOK/local `api_url`
/// can legitimately be plain HTTP.
pub(crate) fn is_safe_endpoint_for_managed_bearer(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    if url.scheme() == "https" {
        return true;
    }
    if url.scheme() != "http" {
        return false;
    }
    is_loopback_host(&url)
}

/// API keys belong to TinyHumans. A loopback endpoint is permitted for local
/// development; an arbitrary TLS host is not a trusted key recipient.
pub(crate) fn is_managed_endpoint_for_api_key(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    if !is_safe_endpoint_for_managed_bearer(endpoint) {
        return false;
    }
    matches!(
        url.host_str(),
        Some("api.tinyhumans.ai" | "staging-api.tinyhumans.ai")
    ) && url.scheme() == "https"
        || is_loopback_host(&url)
}

/// The managed OpenHuman backend as a crate [`ChatModel`]. Holds the backend
/// connection settings (for JWT + base-URL resolution) and the default model id
/// sent when a request doesn't override it.
pub struct OpenHumanBackendModel {
    options: ProviderRuntimeOptions,
    api_url: Option<String>,
    default_model: String,
    native_tool_calling: bool,
    profile: ModelProfile,
    /// Normalized OpenHuman conversation thread. This is deliberately owned by
    /// the managed backend model: BYOK and third-party models must never see
    /// this backend-only extension.
    thread_id: Option<String>,
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
            thread_id: None,
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
        use crate::security::credentials::session_support::classify_session_token;

        // A stored API key (library runtime) is the bearer outright: the
        // OpenAI-compatible managed endpoint accepts it as `Bearer <key>`,
        // and there is no session — so no `exp` and no signed-out state — to
        // consult.
        if let Some(key) = crate::security::credentials::api_key::get_api_key_in(
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
        if crate::platform::llm_gate::is_signed_out() {
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

    /// The managed OpenAI-compatible endpoint, from the installed backend
    /// transport. Without one there is no managed backend to reach.
    fn base_url(&self) -> anyhow::Result<String> {
        let base = crate::backend::inference_base_url(&self.api_url).map_err(|_| {
            anyhow::anyhow!(
                "{} managed inference needs a backend transport",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            )
        })?;
        Ok(format!("{}/openai/v1", base.trim_end_matches('/')))
    }

    /// Resolve the current JWT + base URL and build a fresh crate `OpenAiModel`
    /// (Bearer). Rebuilt per call because the session JWT rotates, but every
    /// build shares one pooled HTTP client ([`managed_inference_http_client`]),
    /// so consecutive calls reuse a warm TLS connection to the backend.
    fn build_wire_model(&self) -> tinyinference_llm::Result<OpenAiModel> {
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
            crate::config::MODEL_MANAGED_DEFAULT
        );
        crate::config::MODEL_MANAGED_DEFAULT.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Request-metadata key a caller sets (to `true`) to ask for no reasoning on
/// a call — see [`without_reasoning`].
const REASONING_OFF_METADATA_KEY: &str = "openhuman_reasoning_off";

/// Mark `request` as not needing the model to reason first. Provider-neutral:
/// `ModelRequest.metadata` never reaches the wire, so a BYOK or local provider
/// ignores it. The managed backend model turns it into `reasoning: { enabled:
/// false }` ([`apply_reasoning_hint`]), which the backend forwards to OpenRouter
/// only. For short structured helper calls (follow-up suggestions) reasoning
/// adds seconds and more tokens than the answer itself, for no quality gain.
pub(crate) fn without_reasoning(mut request: ModelRequest) -> ModelRequest {
    if !request.metadata.is_object() {
        request.metadata = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = request.metadata.as_object_mut() {
        map.insert(REASONING_OFF_METADATA_KEY.to_string(), Value::Bool(true));
    }
    request
}

/// Translate the request's reasoning choice into the managed backend's wire
/// field: the OpenRouter-style `reasoning` object, which the backend forwards
/// upstream.
///
/// Sources, highest first: a `reasoning` object the caller already put in
/// provider options (left alone); the [`without_reasoning`] hint, which marks
/// one helper call specifically; the request's provider-neutral
/// `ModelRequest::reasoning` (the user's thinking level, from the harness
/// `RunPolicy::default_reasoning`). The
/// neutral field is consumed here, so the OpenAI-compatible transport does not
/// also emit a top-level `reasoning_effort` for the same choice.
fn apply_reasoning_hint(mut request: ModelRequest) -> ModelRequest {
    if request.provider_options.get("reasoning").is_some() {
        return request;
    }
    let wants_off = request
        .metadata
        .get(REASONING_OFF_METADATA_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let neutral = request.reasoning.take();
    let wire = if wants_off {
        Some(serde_json::json!({ "enabled": false }))
    } else {
        neutral.as_ref().and_then(managed_reasoning_wire)
    };
    let Some(wire) = wire else {
        return request;
    };
    let mut options = request.provider_options.clone();
    if !options.is_object() {
        options = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = options.as_object_mut() {
        map.insert("reasoning".to_string(), wire.clone());
    }
    log::debug!("[inference][managed] reasoning for this call: {wire}");
    request.with_provider_options(options)
}

/// The OpenRouter `reasoning` object for a provider-neutral config: `none`
/// disables reasoning, an explicit budget becomes `max_tokens`, and any other
/// effort is sent by name. An empty config sends nothing.
fn managed_reasoning_wire(reasoning: &tinyinference_llm::model::ReasoningConfig) -> Option<Value> {
    use tinyinference_llm::model::ReasoningEffort;
    if reasoning.effort == Some(ReasoningEffort::None) {
        return Some(serde_json::json!({ "enabled": false }));
    }
    if let Some(budget) = reasoning.budget_tokens {
        return Some(serde_json::json!({ "max_tokens": budget }));
    }
    reasoning
        .effort
        .map(|effort| serde_json::json!({ "effort": effort.as_str() }))
}

/// Inject this managed model's explicitly owned thread into provider options.
/// This backend-only wire extension is never inferred from ambient state.
fn with_thread_id(request: ModelRequest, thread_id: Option<&str>) -> ModelRequest {
    let Some(thread_id) = thread_id else {
        return request;
    };
    let mut options = request.provider_options.clone();
    if !options.is_object() {
        options = Value::Object(serde_json::Map::new());
    }
    if let Some(map) = options.as_object_mut() {
        map.insert(
            "thread_id".to_string(),
            Value::String(thread_id.to_string()),
        );
    }
    request.with_provider_options(options)
}

/// The bearer a classified app-session token yields for managed inference.
///
/// Separate from [`OpenHumanBackendModel::resolve_bearer`] so the decision is
/// testable without an on-disk auth profile.
///
/// `exp`-less tokens carry no recorded expiry, so `classify_session_token`
/// reports them `Live`; the offline local session is one of those, and it
/// authenticates no TinyHumans account. Sending it anyway earned a backend
/// `401 "Invalid token"`, which published `SessionExpired` and told a user who
/// was signed in locally that their session had expired (#6932). Refusing here
/// mirrors the arm `resolve_backend_credential` already applies to every
/// backend REST caller, and keeps the local credential intact.
fn managed_bearer(
    check: crate::security::credentials::session_support::SessionTokenCheck,
) -> anyhow::Result<String> {
    use crate::security::credentials::session_support::{
        is_local_session_token, SessionTokenCheck, LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE,
    };

    match check {
        SessionTokenCheck::Live(token) if is_local_session_token(&token) => {
            log::debug!(
                "[providers][openhuman-backend] refusing managed inference for the offline local session"
            );
            anyhow::bail!(LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE)
        }
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

/// Publish a `SessionExpired` event when the local `exp` precheck in
/// [`resolve_bearer`](OpenHumanBackendModel::resolve_bearer) rejects an expired
/// managed session token before a request is ever sent — mirroring
/// [`require_live_session_token`](crate::security::credentials::session_support::require_live_session_token)'s
/// pre-flight publish so the credentials subscriber clears state and the UI
/// re-auths exactly as it would on a real backend 401. Deduped via the
/// scheduler gate so N parallel managed turns in one tick don't emit N events.
fn maybe_publish_local_session_expiry() {
    if crate::platform::llm_gate::is_signed_out() {
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
        maybe_publish_provider_session_expired(pe, operation);
    }
}

fn maybe_publish_provider_session_expired(pe: &ProviderError, operation: &str) {
    if pe.provider.as_str() == "OpenHuman" && matches!(pe.status, Some(401 | 403)) {
        let reason = tinyinference_core::sanitize::sanitize_api_error(&pe.message);
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
        TiError::Provider(pe) => log_managed_provider_error(pe, operation),
        other => {
            log::warn!(
                "[providers][openhuman-backend] managed {operation} failed (non-provider error): {}",
                tinyinference_core::sanitize::sanitize_api_error(&other.to_string()),
            );
        }
    }
}

/// Logs a structured provider failure; the detail is secret-scrubbed and
/// truncated because a provider error can echo request content.
fn log_managed_provider_error(pe: &ProviderError, operation: &str) {
    log::warn!(
        "[providers][openhuman-backend] managed {operation} failed: status={:?} code={:?} provider={} retryable={} detail={}",
        pe.status,
        pe.code,
        pe.provider,
        pe.retryable,
        tinyinference_core::sanitize::sanitize_api_error(&pe.message),
    );
}

/// Gives a failure reported inside a stream the same handling as a failed
/// `stream()` call: logged, and an expired session starts re-authentication.
fn observe_in_band_failure(item: &ModelStreamItem) {
    if let ModelStreamItem::ProviderFailed(pe) = item {
        log_managed_provider_error(pe, "stream (in-band)");
        maybe_publish_provider_session_expired(pe, "stream");
    }
}

#[path = "openhuman_backend_model_calls.rs"]
mod calls;

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

#[path = "openhuman_backend_model_usage.rs"]
mod usage;
use usage::project_managed_usage;

#[cfg(test)]
#[path = "openhuman_backend_model_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "openhuman_backend_model_usage_tests.rs"]
mod usage_tests;
