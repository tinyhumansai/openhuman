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
//!   resolve the current bearer via [`OpenHumanBackendProvider::resolve_bearer`]
//!   and build a fresh crate `OpenAiModel` (Bearer) per call.
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

use tinyagents::harness::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream,
};
use tinyagents::harness::providers::openai::OpenAiModel;
use tinyagents::{Result as TaResult, TinyAgentsError};

use super::openhuman_backend::{OpenHumanBackendProvider, PROVIDER_LABEL};
use super::thread_context;

/// The managed OpenHuman backend as a crate [`ChatModel`]. Holds the backend
/// provider (for JWT + base-URL resolution) and the default model id sent when a
/// request doesn't override it.
pub struct OpenHumanBackendModel {
    backend: OpenHumanBackendProvider,
    default_model: String,
    native_tool_calling: bool,
}

impl OpenHumanBackendModel {
    /// Wrap a resolved [`OpenHumanBackendProvider`] with the default model id.
    pub fn new(backend: OpenHumanBackendProvider, default_model: impl Into<String>) -> Self {
        Self {
            backend,
            default_model: default_model.into(),
            native_tool_calling: true,
        }
    }

    /// Force prompt-guided tool calling for toolsets that exceed the managed
    /// backend's native grammar ceiling.
    pub fn with_native_tool_calling(mut self, enabled: bool) -> Self {
        self.native_tool_calling = enabled;
        self
    }

    /// Resolve the current JWT + base URL and build a fresh crate `OpenAiModel`
    /// (Bearer). Rebuilt per call because the session JWT rotates.
    fn build_wire_model(&self) -> TaResult<OpenAiModel> {
        let token = self
            .backend
            .resolve_bearer()
            .map_err(|e| TinyAgentsError::Model(e.to_string()))?;
        let base_url = self
            .backend
            .base_url()
            .map_err(|e| TinyAgentsError::Model(e.to_string()))?;
        // The hosted API is chat-completions only (no `/v1/responses`); auth is a
        // plain bearer JWT. The tier/model rides `request.model`, which the backend
        // resolves — the baked default only applies when a request omits it.
        Ok(
            OpenAiModel::compatible_provider(PROVIDER_LABEL, token, base_url, &self.default_model)
                .with_native_tool_calling(self.native_tool_calling),
        )
    }
}

/// The subset of the managed backend's `openhuman` response envelope the crate
/// `Usage`/`ModelResponse` can't carry — billing + cache tokens — so it can be
/// re-projected for the host cost bridge. Mirrors the fields the legacy
/// `compatible` provider read via `extract_usage`.
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
/// envelope's cached count. Parity with the `ProviderModel` path's
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

    response.raw = crate::openhuman::tinyagents::model::merge_openhuman_usage_meta(
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

#[async_trait]
impl ChatModel<()> for OpenHumanBackendModel {
    fn profile(&self) -> Option<&ModelProfile> {
        // The managed backend serves every workload tier (the tier rides
        // `request.model`), so it advertises no single static capability profile;
        // vision gating is enforced by the seam's RequiredCapabilitiesMiddleware.
        None
    }

    async fn invoke(&self, state: &(), request: ModelRequest) -> TaResult<ModelResponse> {
        let model = self.build_wire_model()?;
        let response = model.invoke(state, with_thread_id(request)).await?;
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
        model.stream(state, with_thread_id(request)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::provider::ProviderRuntimeOptions;
    use tinyagents::harness::message::Message;

    fn backend() -> OpenHumanBackendModel {
        let provider = OpenHumanBackendProvider::new(
            Some("https://api.example.test"),
            &ProviderRuntimeOptions::default(),
        );
        OpenHumanBackendModel::new(provider, "reasoning-v1")
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
    fn managed_model_has_no_static_profile() {
        assert!(backend().profile().is_none());
    }

    /// The managed `openhuman.{billing,usage}` envelope on `raw` must re-project
    /// into the host `UsageInfo` the cost bridge reads — charged USD, cached
    /// tokens, and context window — exactly as the legacy `ProviderModel` path did.
    #[test]
    fn project_managed_usage_recovers_charged_and_cached() {
        use crate::openhuman::tinyagents::model::usage_info_from_response;
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
        use crate::openhuman::tinyagents::model::usage_info_from_response;
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
}
