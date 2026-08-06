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

use tinyagents::harness::message::Message;
use tinyagents::harness::model::{
    ChatModel, Modalities, ModelProfile, ModelRequest, ModelResponse, ModelStream, ProviderError,
};
use tinyagents::harness::providers::openai::OpenAiModel;
use tinyagents::{Result as TaResult, TinyAgentsError};

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
        if crate::openhuman::cron::scheduler_gate::is_signed_out() {
            anyhow::bail!(
                "SESSION_EXPIRED: backend session not active — sign in to resume LLM work"
            );
        }
        let auth = AuthService::new(&self.state_dir(), self.options.secrets_encrypt);
        if let Some(token) = auth
            .get_provider_bearer_token(
                APP_SESSION_PROVIDER,
                self.options.auth_profile_override.as_deref(),
            )?
            .filter(|token| !token.trim().is_empty())
        {
            return Ok(token);
        }
        anyhow::bail!("No backend session: store a JWT via auth (app-session)")
    }

    fn base_url(&self) -> String {
        format!(
            "{}/openai/v1",
            effective_api_url(&self.api_url).trim_end_matches('/')
        )
    }

    /// Resolve the current JWT + base URL and build a fresh crate `OpenAiModel`
    /// (Bearer). Rebuilt per call because the session JWT rotates.
    fn build_wire_model(&self) -> TaResult<OpenAiModel> {
        let token = self
            .resolve_bearer()
            .map_err(|e| TinyAgentsError::Model(e.to_string()))?;
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
            Err(TinyAgentsError::Provider(err)) => {
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

/// Publish a `SessionExpired` event when the backend rejects a crate-native
/// model call with `401`/`403` Unauthorized — mirroring the check in
/// [`CrateBackedProvider::invoke`](super::CrateBackedProvider) which the
/// crate-native path bypasses.
fn maybe_publish_session_expired(err: &TinyAgentsError, operation: &str) {
    if let TinyAgentsError::Provider(pe) = err {
        if pe.provider.as_str() == "OpenHuman" && matches!(pe.status, Some(401 | 403)) {
            let reason =
                crate::openhuman::inference::provider::ops::sanitize_api_error(&pe.message);
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::SessionExpired {
                    source: format!(
                        "openhuman_backend_model.{}({})",
                        operation,
                        pe.status.unwrap_or(0)
                    ),
                    reason,
                },
            );
        }
    }
}

#[async_trait]
impl ChatModel<()> for OpenHumanBackendModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(&self, state: &(), request: ModelRequest) -> TaResult<ModelResponse> {
        let model = self.build_wire_model()?;
        let response = match model.invoke(state, with_thread_id(request)).await {
            Ok(response) => response,
            Err(e) => {
                maybe_publish_session_expired(&e, "invoke");
                return Err(e);
            }
        };
        Ok(project_managed_usage(response))
    }

    async fn stream(&self, state: &(), request: ModelRequest) -> TaResult<ModelStream> {
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
                maybe_publish_session_expired(&e, "stream");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::provider::ProviderRuntimeOptions;
    use tinyagents::harness::message::Message;

    fn backend() -> OpenHumanBackendModel {
        OpenHumanBackendModel::new(
            Some("https://api.example.test"),
            &ProviderRuntimeOptions::default(),
            "reasoning-v1",
        )
    }

    #[tokio::test]
    async fn with_thread_id_injects_when_ambient_thread_present() {
        thread_context::with_thread_id("thread-42", async {
            let request = ModelRequest::new(vec![Message::user("hi")]);
            let updated = with_thread_id(request);
            assert_eq!(
                updated.provider_options["thread_id"],
                serde_json::json!("thread-42")
            );
        })
        .await;
    }

    #[test]
    fn with_thread_id_is_noop_without_ambient_thread() {
        // No thread scope active → provider_options stays whatever it was (null).
        let request = ModelRequest::new(vec![Message::user("hi")]);
        let updated = with_thread_id(request);
        assert!(updated.provider_options.get("thread_id").is_none());
    }

    #[test]
    fn managed_model_advertises_tool_and_vision_capabilities() {
        let model = backend();
        let profile = model.profile().expect("managed profile");
        assert!(profile.tool_calling);
        assert!(profile.modalities.image_in);
    }

    #[test]
    fn resolve_model_normalizes_blank_and_trims_non_empty_values() {
        assert_eq!(
            resolve_model(""),
            crate::openhuman::config::MODEL_REASONING_V1
        );
        assert_eq!(
            resolve_model(" \t\n"),
            crate::openhuman::config::MODEL_REASONING_V1
        );
        assert_eq!(resolve_model("  reasoning-v1  "), "reasoning-v1");
        assert_eq!(resolve_model("hint:reasoning"), "hint:reasoning");
    }

    /// The managed `openhuman.{billing,usage}` envelope on `raw` must re-project
    /// into the host `UsageInfo` the cost bridge reads — charged USD, cached
    /// tokens, and context window — exactly as the legacy legacy model-adapter path did.
    #[test]
    fn project_managed_usage_recovers_charged_and_cached() {
        use crate::openhuman::agent::tinyagents::model::usage_info_from_response;
        use tinyagents::harness::message::AssistantMessage;
        use tinyagents::harness::usage::Usage;

        let raw = serde_json::json!({
            "openhuman": {
                "usage": { "cached_input_tokens": 128, "context_window": 200000 },
                "billing": { "charged_amount_usd": 0.0042 }
            }
        });
        let response = ModelResponse {
            message: AssistantMessage {
                id: None,
                content: vec![],
                tool_calls: vec![],
                usage: None,
            },
            usage: Some(Usage {
                input_tokens: 1000,
                output_tokens: 50,
                ..Usage::default()
            }),
            finish_reason: None,
            raw: Some(raw),
            resolved_model: None,
            continue_turn: None,
        };

        let projected = project_managed_usage(response);
        let usage = usage_info_from_response(&projected).expect("usage recovered");
        assert!(
            (usage.charged_amount_usd - 0.0042).abs() < 1e-9,
            "charged={}",
            usage.charged_amount_usd
        );
        assert_eq!(usage.cached_input_tokens, 128, "cached tokens backfilled");
        assert_eq!(usage.context_window, 200_000);
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 50);
    }

    /// A response with no `openhuman` envelope stays untouched — no meta key, no
    /// charged USD — so non-managed/billing-free responses aren't fabricated.
    #[test]
    fn project_managed_usage_is_noop_without_envelope() {
        use crate::openhuman::agent::tinyagents::model::usage_info_from_response;
        use tinyagents::harness::message::AssistantMessage;
        use tinyagents::harness::usage::Usage;

        let response = ModelResponse {
            message: AssistantMessage {
                id: None,
                content: vec![],
                tool_calls: vec![],
                usage: None,
            },
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: 3,
                ..Usage::default()
            }),
            finish_reason: None,
            raw: Some(serde_json::json!({ "id": "resp_1" })),
            resolved_model: None,
            continue_turn: None,
        };

        let projected = project_managed_usage(response);
        // raw keeps only the wire fields — no meta key injected.
        assert!(projected
            .raw
            .as_ref()
            .unwrap()
            .get("openhuman_usage_meta")
            .is_none());
        let usage = usage_info_from_response(&projected).expect("usage present");
        assert_eq!(usage.charged_amount_usd, 0.0);
        assert_eq!(usage.cached_input_tokens, 3, "crate cached count preserved");
    }

    // ── probe_readiness (B45 — flows provider-connectivity author gate) ────

    #[test]
    fn is_provider_not_configured_error_matches_exact_backend_shape() {
        let err = ProviderError {
            provider: "OpenHuman".to_string(),
            model: None,
            status: Some(400),
            code: Some("BAD_REQUEST".to_string()),
            message: "API key not configured for provider".to_string(),
            retryable: false,
            raw: None,
        };
        assert!(is_provider_not_configured_error(&err));
    }

    #[test]
    fn is_provider_not_configured_error_rejects_other_400s() {
        // A 400 that isn't the "no provider configured" class (e.g. a bad
        // request shape) must NOT be classified as provider-not-configured —
        // only the exact backend-confirmed signal should ever reject.
        let err = ProviderError {
            provider: "OpenHuman".to_string(),
            model: None,
            status: Some(400),
            code: Some("BAD_REQUEST".to_string()),
            message: "invalid request: messages must not be empty".to_string(),
            retryable: false,
            raw: None,
        };
        assert!(!is_provider_not_configured_error(&err));
    }

    #[test]
    fn is_provider_not_configured_error_tolerates_not_configured_for_provider_wording_drift() {
        // The `code_is_bad_request` branch still matches the narrower
        // "not configured for provider" substring even when it isn't
        // introduced by the exact "api key" prefix — tolerance for backend
        // message wording drift, not a broadening to any "not configured".
        let err = ProviderError {
            provider: "OpenHuman".to_string(),
            model: None,
            status: Some(400),
            code: Some("BAD_REQUEST".to_string()),
            message: "credentials not configured for provider 'anthropic'".to_string(),
            retryable: false,
            raw: None,
        };
        assert!(is_provider_not_configured_error(&err));
    }

    #[test]
    fn is_provider_not_configured_error_rejects_generic_not_configured_400() {
        // Tightened contract (finding D): a 400 `BAD_REQUEST` whose message
        // contains only the generic word "not configured" — but not the
        // specific "not configured for provider" phrasing — must fail OPEN,
        // not be misclassified as the provider-key signal. Otherwise an
        // unrelated backend validation error ("model X not configured", "this
        // feature is not configured for your account", …) would falsely
        // reject a run/proposal as a provider problem.
        let err = ProviderError {
            provider: "OpenHuman".to_string(),
            model: None,
            status: Some(400),
            code: Some("BAD_REQUEST".to_string()),
            message: "webhook target not configured".to_string(),
            retryable: false,
            raw: None,
        };
        assert!(!is_provider_not_configured_error(&err));
    }

    #[test]
    fn is_provider_not_configured_error_rejects_non_400_status() {
        let err = ProviderError {
            provider: "OpenHuman".to_string(),
            model: None,
            status: Some(401),
            code: None,
            message: "API key not configured for provider".to_string(),
            retryable: false,
            raw: None,
        };
        assert!(!is_provider_not_configured_error(&err));
    }

    fn seed_app_session(dir: &std::path::Path) {
        use crate::openhuman::security::credentials::{
            AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
        };
        AuthService::new(dir, false)
            .store_provider_token(
                APP_SESSION_PROVIDER,
                DEFAULT_AUTH_PROFILE_NAME,
                "test.session.jwt",
                std::collections::HashMap::new(),
                true,
            )
            .expect("seed app-session token");
    }

    fn backend_pointed_at(addr: &str, dir: &std::path::Path) -> OpenHumanBackendModel {
        OpenHumanBackendModel::new(
            Some(&format!("http://{addr}")),
            &ProviderRuntimeOptions {
                openhuman_dir: Some(dir.to_path_buf()),
                secrets_encrypt: false,
                ..ProviderRuntimeOptions::default()
            },
            "reasoning-v1",
        )
    }

    #[derive(Clone)]
    struct StaticChatResponse {
        status: axum::http::StatusCode,
        body: Value,
    }

    async fn static_chat_handler(
        axum::extract::State(s): axum::extract::State<StaticChatResponse>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        (s.status, axum::Json(s.body)).into_response()
    }

    async fn spawn_static_chat_server(status: axum::http::StatusCode, body: Value) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let app = axum::Router::new()
            .route(
                "/openai/v1/chat/completions",
                axum::routing::post(static_chat_handler),
            )
            .with_state(StaticChatResponse { status, body });
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        addr.to_string()
    }

    async fn slow_chat_handler() -> axum::response::Response {
        use axum::response::IntoResponse;
        // Longer than the probe's 5s timeout — the probe must return before
        // this ever resolves.
        tokio::time::sleep(Duration::from_secs(8)).await;
        (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({
                "choices": [{ "message": { "role": "assistant", "content": "pong" } }]
            })),
        )
            .into_response()
    }

    async fn spawn_slow_chat_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let app = axum::Router::new().route(
            "/openai/v1/chat/completions",
            axum::routing::post(slow_chat_handler),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        addr.to_string()
    }

    #[tokio::test]
    async fn probe_readiness_surfaces_api_key_not_configured() {
        let tmp = tempfile::TempDir::new().unwrap();
        seed_app_session(tmp.path());
        let addr = spawn_static_chat_server(
            axum::http::StatusCode::BAD_REQUEST,
            serde_json::json!({
                "success": false,
                "error": "API key not configured for provider",
                "errorCode": "BAD_REQUEST"
            }),
        )
        .await;
        let backend = backend_pointed_at(&addr, tmp.path());

        let err = backend
            .probe_readiness()
            .await
            .expect_err("a confirmed provider-not-configured 400 must reject");
        assert!(
            err.to_ascii_lowercase()
                .contains("api key not configured for provider"),
            "error must surface the backend's own message: {err}"
        );
    }

    #[tokio::test]
    async fn probe_readiness_fails_open_on_timeout_or_5xx() {
        // 5xx sub-case: a transient backend failure must never block authoring.
        let tmp = tempfile::TempDir::new().unwrap();
        seed_app_session(tmp.path());
        let addr = spawn_static_chat_server(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({ "error": "temporarily unavailable" }),
        )
        .await;
        let backend = backend_pointed_at(&addr, tmp.path());
        backend
            .probe_readiness()
            .await
            .expect("a transient 5xx must fail open (Ok)");

        // Timeout sub-case: a hung backend must fail open once the 5s probe
        // timeout fires, without waiting for the slow handler to respond.
        let tmp2 = tempfile::TempDir::new().unwrap();
        seed_app_session(tmp2.path());
        let addr2 = spawn_slow_chat_server().await;
        let backend2 = backend_pointed_at(&addr2, tmp2.path());
        let started = std::time::Instant::now();
        backend2
            .probe_readiness()
            .await
            .expect("a hung backend must fail open (Ok) once the 5s timeout fires");
        assert!(
            started.elapsed() < Duration::from_secs(7),
            "probe must return around the 5s timeout, not wait for the slow handler"
        );
    }
}
