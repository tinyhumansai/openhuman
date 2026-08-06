//! OpenHuman helpers and wrappers for native TinyAgents [`ChatModel`] values.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents::harness::message::{AssistantMessage, ContentBlock, MessageDelta};
use tinyagents::harness::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyagents::harness::tool::{ToolCall as TaToolCall, ToolDelta};
use tinyagents::harness::usage::Usage;
use tokio::sync::mpsc::UnboundedSender;

use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::inference::provider::{ChatResponse, ProviderDelta, UsageInfo};

pub(super) type TurnChatModel = Arc<dyn ChatModel<()>>;
pub(super) type TierRoutes = Vec<(String, TurnChatModel)>;
pub(super) type BuiltTurnModels = (TurnChatModel, TierRoutes, TurnChatModel);

/// Convert a crate request into the host's native-role message shape while the
/// remaining bespoke transports still consume `ChatMessage`.
pub(crate) fn native_chat_messages(request: &ModelRequest) -> Vec<ChatMessage> {
    request
        .messages
        .iter()
        .map(crate::openhuman::agent::message_convert::message_to_native_chat_message)
        .collect()
}

/// Build a [`PFormatRegistry`](crate::openhuman::agent::pformat::PFormatRegistry)
/// from the tool schemas advertised on a [`ModelRequest`] (issue #4465).
///
/// The text-mode fallback parse needs each tool's positional parameter layout
/// to reconstruct named JSON arguments from a P-Format `name[a|b]` body. The
/// harness populates `request.tools` when tools are available (schemas are
/// rendered into the prompt for prompt-guided providers, or advertised natively
/// otherwise), so the registry is available in both modes. Tool-less requests
/// skip fallback parsing entirely; this empty registry is therefore consulted
/// only alongside a non-empty advertised tool list.
fn pformat_registry_from_request(
    request: &ModelRequest,
) -> crate::openhuman::agent::pformat::PFormatRegistry {
    request
        .tools
        .iter()
        .map(|t| {
            (
                t.name.clone(),
                crate::openhuman::agent::pformat::PFormatToolParams::from_schema(&t.parameters),
            )
        })
        .collect()
}

/// Translate an openhuman [`ChatResponse`] into a harness [`ModelResponse`]
/// (visible text + tool calls + token usage).
///
/// Native `tool_calls` take precedence; when absent and the request advertised
/// tools, the response text is parsed for prompt-guided (`<tool_call>…` /
/// p-format) calls — matching the legacy dispatcher — so text-mode models drive
/// the tinyagents loop too. Tool-less requests preserve response text verbatim,
/// including literal tool-call examples. When parsing is enabled, visible text
/// is the prose with any parsed tool-call markup stripped.
///
/// `pformat_registry` carries the advertised tools' positional layouts so the
/// text-mode fallback can recover P-Format (`name[a|b]`) calls that ~10 builtin
/// prompts still teach — the migrated parse path had dropped that grammar and
/// silently lost those calls (issue #4465). It is empty for the native-tool
/// path, where `response.tool_calls` is used directly.
///
/// Unknown-tool recovery is handled by `RunPolicy::unknown_tool`, so the model
/// adapter preserves the provider-requested tool name.
fn response_to_model_response(
    response: &ChatResponse,
    pformat_registry: &crate::openhuman::agent::pformat::PFormatRegistry,
    parse_text_tool_calls: bool,
) -> ModelResponse {
    let (visible_text, tool_calls): (String, Vec<TaToolCall>) = if !response.tool_calls.is_empty() {
        let calls = response
            .tool_calls
            .iter()
            .map(|tc| TaToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: serde_json::from_str(&tc.arguments).unwrap_or(serde_json::Value::Null),
                invalid: None,
            })
            .collect();
        (response.text.clone().unwrap_or_default(), calls)
    } else if parse_text_tool_calls {
        let text = response.text.as_deref().unwrap_or_default();
        let (prose, parsed) =
            crate::openhuman::agent::harness::parse_tool_calls_with_pformat(text, pformat_registry);
        if parsed.is_empty() {
            (text.to_string(), Vec::new())
        } else {
            let calls = parsed
                .into_iter()
                .enumerate()
                .map(|(i, p)| TaToolCall {
                    // Prompt-guided calls carry no provider id; synthesize a
                    // stable one so tool results correlate in the harness.
                    id: p.id.unwrap_or_else(|| format!("call_{i}")),
                    name: p.name,
                    arguments: p.arguments,
                    invalid: None,
                })
                .collect();
            (prose, calls)
        }
    } else {
        (response.text.clone().unwrap_or_default(), Vec::new())
    };

    let mut content = Vec::new();
    if !visible_text.is_empty() {
        content.push(ContentBlock::Text(visible_text));
    }
    // Thinking models return `reasoning_content` separately from the visible
    // reply. Preserve it as a typed thinking block so it stays out of
    // `Message::text()` but survives persistence and the next turn's request,
    // where thinking-mode providers require it back.
    if let Some(block) = crate::openhuman::agent::message_convert::reasoning_content_block(
        response.reasoning_content.as_deref(),
    ) {
        content.push(block);
    }
    let usage = response.usage.as_ref().map(|u| {
        // Carry every token breakdown the crate `Usage` can express so a
        // standalone `invoke` is usage-faithful (gap G1): cache reads/writes and
        // reasoning tokens all have crate homes as of tinyagents 1.7. `Usage::new`
        // seeds input/output/total; set the detail fields on top.
        let mut usage = Usage::new(u.input_tokens, u.output_tokens);
        usage.cache_read_tokens = u.cached_input_tokens;
        usage.cache_creation_tokens = u.cache_creation_tokens;
        usage.reasoning_tokens = u.reasoning_tokens;
        usage
    });
    let finish_reason = if tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content,
            tool_calls,
            usage,
        },
        usage,
        finish_reason: Some(finish_reason.to_string()),
        // The crate `Usage` has no field for the provider's **charged USD** or the
        // model's **context window**, but `ModelResponse.raw` is exactly "raw
        // provider metadata preserved for callers who need it" — so stash them
        // there (gap G1). A standalone `invoke` then round-trips the OpenHuman
        // managed backend's charged amount + window via
        // [`usage_info_from_response`], no crate change required. Omitted when the
        // provider reported neither (keeps non-managed responses byte-clean).
        raw: openhuman_usage_meta_raw(response.usage.as_ref()),
        resolved_model: None,
        continue_turn: None,
    }
}

/// Convert a native host response into the crate model response shape.
pub(crate) fn native_model_response(response: &ChatResponse) -> ModelResponse {
    response_to_model_response(
        response,
        &crate::openhuman::agent::pformat::PFormatRegistry::default(),
        false,
    )
}

/// Convert a host response while preserving the legacy text-tool fallback for
/// a request that advertised tools. This remains available to migration
/// fixtures that exercise the old XML/P-Format recovery contract without
/// constructing a [`native model adapter`].
pub(crate) fn native_model_response_for_request(
    response: &ChatResponse,
    request: &ModelRequest,
) -> ModelResponse {
    response_to_model_response(
        response,
        &pformat_registry_from_request(request),
        !request.tools.is_empty(),
    )
}

/// Normalize a completed prompt-guided response for a crate-native model.
///
/// TinyAgents owns the generic prompt protocol and XML tool-call grammar. The
/// host keeps a temporary second pass for its legacy P-Format prompts until
/// those prompts are migrated (migration plan WP1/WP4).
pub(crate) fn prompt_guided_text_response(text: String, request: &ModelRequest) -> ModelResponse {
    if request.tools.is_empty() {
        return ModelResponse::assistant(text);
    }

    let response =
        tinyagents::harness::tool::apply_prompt_tool_calls(ModelResponse::assistant(text.clone()));
    if !response.message.tool_calls.is_empty() {
        return response;
    }

    response_to_model_response(
        &ChatResponse {
            text: Some(text),
            ..Default::default()
        },
        &pformat_registry_from_request(request),
        true,
    )
}

/// JSON key under which the model adapter stashes the provider-reported
/// billing/context metadata that the crate [`Usage`] has no field for
/// (gap G1). Consumed by [`usage_info_from_response`].
const OPENHUMAN_USAGE_META_KEY: &str = "openhuman_usage_meta";

/// The two host [`UsageInfo`] fields with no crate [`Usage`] home, ferried
/// through [`ModelResponse::raw`] so a standalone `invoke` stays usage-faithful.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct OpenhumanUsageMeta {
    /// Provider-charged amount in USD (`UsageInfo::charged_amount_usd`).
    #[serde(default)]
    charged_amount_usd: f64,
    /// Model context window in tokens (`UsageInfo::context_window`).
    #[serde(default)]
    context_window: u64,
}

/// Build the `ModelResponse.raw` value carrying charged-USD + context-window
/// metadata, or `None` when the provider reported neither (so responses from
/// providers that don't surface billing stay `raw: None`).
fn openhuman_usage_meta_raw(usage: Option<&UsageInfo>) -> Option<serde_json::Value> {
    let u = usage?;
    if u.charged_amount_usd <= 0.0 && u.context_window == 0 {
        return None;
    }
    let meta = OpenhumanUsageMeta {
        charged_amount_usd: u.charged_amount_usd,
        context_window: u.context_window,
    };
    Some(serde_json::json!({ OPENHUMAN_USAGE_META_KEY: meta }))
}

/// Merge the host billing/context metadata the crate [`Usage`] cannot carry into
/// a crate [`ModelResponse::raw`] under [`OPENHUMAN_USAGE_META_KEY`], so
/// [`usage_info_from_response`] recovers the charged-USD + context window from a
/// crate-native model (e.g. [`OpenHumanBackendModel`](crate::openhuman::inference::provider::OpenHumanBackendModel))
/// exactly as it does from a [`native model adapter`].
///
/// The crate `OpenAiModel` leaves the managed backend's `openhuman.{billing,usage}`
/// envelope only on the raw wire JSON — it has no field for charged USD — so the
/// crate-native managed path would otherwise report `$0` charged and fall back to
/// the catalog estimate (issue #4249, Phase 3 usage-parity). This is the symmetric
/// writer for [`usage_info_from_response`]'s reader.
///
/// No-op when both values are zero (keeps billing-free responses `raw`-clean);
/// otherwise inserts the meta key into the existing raw object (preserving the
/// wire JSON) or creates a fresh object.
pub(crate) fn merge_openhuman_usage_meta(
    raw: Option<serde_json::Value>,
    charged_amount_usd: f64,
    context_window: u64,
) -> Option<serde_json::Value> {
    if charged_amount_usd <= 0.0 && context_window == 0 {
        return raw;
    }
    let meta = match serde_json::to_value(OpenhumanUsageMeta {
        charged_amount_usd,
        context_window,
    }) {
        Ok(v) => v,
        Err(_) => return raw,
    };
    match raw {
        Some(serde_json::Value::Object(mut obj)) => {
            obj.insert(OPENHUMAN_USAGE_META_KEY.to_string(), meta);
            Some(serde_json::Value::Object(obj))
        }
        // A non-object (or absent) raw can't hold the key alongside wire fields —
        // stash the meta on its own so the reader still recovers it.
        _ => Some(serde_json::json!({ OPENHUMAN_USAGE_META_KEY: meta })),
    }
}

/// Reconstruct a host [`UsageInfo`] from a crate [`ModelResponse`], recovering
/// the provider-charged USD + context window the adapter stashed in
/// [`ModelResponse::raw`] (gap G1). Returns `None` when the response carried no
/// usage at all.
///
/// This is the seam one-shot inference callers use once they move off
/// the legacy chat response onto `Arc<dyn ChatModel>`
/// (`invoke` → `ModelResponse`): the full host usage record — real token
/// counts *and* backend-charged USD — survives the crossing.
pub(crate) fn usage_info_from_response(response: &ModelResponse) -> Option<UsageInfo> {
    let usage = response.usage.as_ref()?;
    let meta = response
        .raw
        .as_ref()
        .and_then(|v| v.get(OPENHUMAN_USAGE_META_KEY))
        .and_then(|v| serde_json::from_value::<OpenhumanUsageMeta>(v.clone()).ok())
        .unwrap_or_default();
    Some(UsageInfo {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        context_window: meta.context_window,
        cached_input_tokens: usage.cache_read_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        charged_amount_usd: meta.charged_amount_usd,
    })
}

/// Forward one openhuman [`ProviderDelta`]. Visible text, reasoning, and
/// tool-call **argument** fragments all become harness [`ModelStreamItem`]s (so
/// the [`OpenhumanEventBridge`](super::OpenhumanEventBridge) mirrors them as
/// progress deltas from the crate stream alone): text/reasoning as
/// [`MessageDelta`], and each argument fragment as
/// [`ModelStreamItem::ToolCallDelta`] correlated by `call_id`. The tool-call
/// **start** marker now also rides the native stream: with the crate `ToolDelta`
/// carrying an optional `tool_name` (G2), the call-opening delta is a
/// `ToolCallDelta` with the name set and empty content, so the
/// [`OpenhumanEventBridge`](super::OpenhumanEventBridge) records the name and
/// opens the UI timeline row off the crate stream alone — no out-of-band
/// forwarder. The model adapter still assembles the final native tool calls from
/// the `Completed` response (the `StreamAccumulator` treats it as
/// authoritative), so these fragments are progress-only — the UI can show the
/// call being composed.
pub(crate) fn forward_provider_delta(tx: &UnboundedSender<ModelStreamItem>, delta: ProviderDelta) {
    match delta {
        ProviderDelta::TextDelta { delta } => {
            if !delta.is_empty() {
                let _ = tx.send(ModelStreamItem::MessageDelta(MessageDelta::text(delta)));
            }
        }
        ProviderDelta::ThinkingDelta { delta } => {
            if !delta.is_empty() {
                let _ = tx.send(ModelStreamItem::MessageDelta(MessageDelta::reasoning(
                    delta,
                )));
            }
        }
        ProviderDelta::ToolCallStart { call_id, tool_name } => {
            // Call-opening marker: name set, empty content. Rides the native
            // crate stream (G2) so the bridge can label the call before its
            // arguments arrive.
            tracing::trace!(
                call_id = call_id.as_str(),
                tool_name = tool_name.as_str(),
                "[stream] forwarding tool-call start onto crate ToolCallDelta"
            );
            let _ = tx.send(ModelStreamItem::ToolCallDelta(ToolDelta {
                call_id,
                content: String::new(),
                tool_name: Some(tool_name),
            }));
        }
        ProviderDelta::ToolCallArgsDelta { call_id, delta } => {
            if !delta.is_empty() {
                tracing::trace!(
                    call_id = call_id.as_str(),
                    len = delta.len(),
                    "[stream] forwarding tool-arg fragment onto crate ToolCallDelta"
                );
                let _ = tx.send(ModelStreamItem::ToolCallDelta(ToolDelta {
                    call_id,
                    content: delta,
                    tool_name: None,
                }));
            }
        }
    }
}

/// Shared slot that preserves the most recent original provider error.
///
/// tinyagents carries errors as `TinyAgentsError::Model(String)`, which would
/// stringify openhuman's typed `anyhow::Error` (e.g. `AgentError::PermissionDenied`
/// / `MaxIterationsExceeded`) and break the downcast the caller relies on for
/// Sentry suppression and `AgentError`-tagged events. The adapter stashes the
/// original error here before returning the stringified one to the harness, so
/// the runner can re-surface the downcastable error after the run fails.
pub(super) type ModelErrorSlot = Arc<Mutex<Option<anyhow::Error>>>;

pub(super) struct MaxTokensModel {
    inner: Arc<dyn ChatModel<()>>,
    max_tokens: u32,
}

/// A model wrapper that overrides capability metadata without changing wire
/// behavior. Directly injected crate models use this when a turn supplies a
/// more precise context window or explicitly selects prompt-guided tool mode.
pub(super) struct ProfileOverrideModel {
    inner: Arc<dyn ChatModel<()>>,
    profile: ModelProfile,
    request_model: Option<String>,
    request_temperature: Option<f64>,
}

/// Records the concrete provider/model selected by a crate-native turn model.
///
/// TinyAgents' registry records the selected registry key (for example
/// `chat-v1`) on `ModelResponse`, but channel audit events also need the
/// provider and concrete wire model. Each registered route is wrapped with
/// this metadata at construction time. Recording immediately before dispatch
/// means retries are harmless and a successful fallback leaves the last
/// attempted (therefore handling) route in the ambient turn slot.
pub(super) struct RouteRecordingModel {
    inner: Arc<dyn ChatModel<()>>,
    provider: String,
    model: String,
}

impl RouteRecordingModel {
    pub(super) fn new(
        inner: Arc<dyn ChatModel<()>>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            provider: provider.into(),
            model: model.into(),
        }
    }

    fn record_route(&self) {
        super::record_resolved_provider_route(&self.provider, &self.model);
    }
}

#[async_trait]
impl ChatModel<()> for RouteRecordingModel {
    fn profile(&self) -> Option<&ModelProfile> {
        self.inner.profile()
    }

    async fn invoke(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelResponse> {
        self.record_route();
        self.inner.invoke(state, request).await
    }

    async fn stream(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelStream> {
        self.record_route();
        self.inner.stream(state, request).await
    }
}

impl ProfileOverrideModel {
    pub(super) fn new(inner: Arc<dyn ChatModel<()>>, profile: ModelProfile) -> Self {
        Self {
            inner,
            profile,
            request_model: None,
            request_temperature: None,
        }
    }

    pub(super) fn with_request_model(mut self, model: impl Into<String>) -> Self {
        self.request_model = Some(model.into());
        self
    }

    pub(super) fn with_request_temperature(mut self, temperature: f64) -> Self {
        self.request_temperature = Some(temperature);
        self
    }

    fn pin_request_options(&self, mut request: ModelRequest) -> ModelRequest {
        if request.model.is_none() {
            request.model.clone_from(&self.request_model);
        }
        if request.temperature.is_none() {
            request.temperature = self.request_temperature;
        }
        request
    }
}

#[async_trait]
impl ChatModel<()> for ProfileOverrideModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelResponse> {
        self.inner
            .invoke(state, self.pin_request_options(request))
            .await
    }

    async fn stream(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelStream> {
        self.inner
            .stream(state, self.pin_request_options(request))
            .await
    }
}

impl MaxTokensModel {
    pub(super) fn new(inner: Arc<dyn ChatModel<()>>, max_tokens: u32) -> Self {
        Self { inner, max_tokens }
    }

    fn cap(&self, mut request: ModelRequest) -> ModelRequest {
        request.max_tokens = Some(
            request
                .max_tokens
                .map_or(self.max_tokens, |current| current.min(self.max_tokens)),
        );
        request
    }
}

#[async_trait]
impl ChatModel<()> for MaxTokensModel {
    fn profile(&self) -> Option<&ModelProfile> {
        self.inner.profile()
    }

    async fn invoke(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelResponse> {
        self.inner.invoke(state, self.cap(request)).await
    }

    async fn stream(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelStream> {
        self.inner.stream(state, self.cap(request)).await
    }
}

#[cfg(test)]
mod route_recording_tests {
    use super::*;
    use crate::openhuman::agent::tinyagents::{
        current_resolved_provider_route, with_resolved_provider_route_scope, ResolvedProviderRoute,
    };

    struct SuccessfulModel;

    #[async_trait]
    impl ChatModel<()> for SuccessfulModel {
        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyagents::Result<ModelResponse> {
            Ok(ModelResponse::assistant("ok"))
        }
    }

    #[tokio::test]
    async fn selected_model_records_concrete_route_and_fallback_overwrites_primary() {
        let primary = RouteRecordingModel::new(Arc::new(SuccessfulModel), "openhuman", "chat-v1");
        let fallback =
            RouteRecordingModel::new(Arc::new(SuccessfulModel), "anthropic", "claude-sonnet-4");

        let observed = with_resolved_provider_route_scope(async {
            primary
                .invoke(&(), ModelRequest::default())
                .await
                .expect("primary dispatch");
            fallback
                .invoke(&(), ModelRequest::default())
                .await
                .expect("fallback dispatch");
            current_resolved_provider_route()
        })
        .await;

        assert_eq!(
            observed,
            Some(ResolvedProviderRoute {
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn streamed_model_records_concrete_route_before_stream_consumption() {
        let model =
            RouteRecordingModel::new(Arc::new(SuccessfulModel), "openhuman", "reasoning-v1");

        let observed = with_resolved_provider_route_scope(async {
            let _stream = model
                .stream(&(), ModelRequest::default())
                .await
                .expect("stream dispatch");
            current_resolved_provider_route()
        })
        .await;

        assert_eq!(
            observed,
            Some(ResolvedProviderRoute {
                provider: "openhuman".to_string(),
                model: "reasoning-v1".to_string(),
            })
        );
    }
}

#[cfg(test)]
mod g1_usage_tests {
    //! Gap G1: a standalone `invoke` must stay usage-faithful — token
    //! breakdowns ride the crate `Usage`, and the two host fields with no crate
    //! home (charged USD + context window) ride `ModelResponse.raw` and
    //! reconstruct exactly via [`usage_info_from_response`].
    use super::*;

    fn empty_registry() -> crate::openhuman::agent::pformat::PFormatRegistry {
        crate::openhuman::agent::pformat::PFormatRegistry::default()
    }

    #[test]
    fn usage_round_trips_charged_usd_and_all_token_breakdowns() {
        let chat = ChatResponse {
            text: Some("hi".to_string()),
            tool_calls: Vec::new(),
            usage: Some(UsageInfo {
                input_tokens: 100,
                output_tokens: 20,
                context_window: 128_000,
                cached_input_tokens: 40,
                cache_creation_tokens: 10,
                reasoning_tokens: 7,
                charged_amount_usd: 0.0123,
            }),
            reasoning_content: None,
        };
        let model_response = response_to_model_response(&chat, &empty_registry(), false);

        // Crate Usage carries every token breakdown natively.
        let usage = model_response.usage.expect("usage present");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_read_tokens, 40);
        assert_eq!(usage.cache_creation_tokens, 10);
        assert_eq!(usage.reasoning_tokens, 7);

        // Charged USD + context window ride raw and reconstruct exactly.
        let recovered = usage_info_from_response(&model_response).expect("usage info");
        assert_eq!(recovered.input_tokens, 100);
        assert_eq!(recovered.output_tokens, 20);
        assert_eq!(recovered.context_window, 128_000);
        assert_eq!(recovered.cached_input_tokens, 40);
        assert_eq!(recovered.cache_creation_tokens, 10);
        assert_eq!(recovered.reasoning_tokens, 7);
        assert!((recovered.charged_amount_usd - 0.0123).abs() < 1e-9);
    }

    #[test]
    fn no_billing_metadata_leaves_raw_clean() {
        let chat = ChatResponse {
            text: Some("hi".to_string()),
            tool_calls: Vec::new(),
            usage: Some(UsageInfo {
                input_tokens: 5,
                output_tokens: 3,
                ..Default::default()
            }),
            reasoning_content: None,
        };
        let model_response = response_to_model_response(&chat, &empty_registry(), false);
        assert!(
            model_response.raw.is_none(),
            "no charged USD / window ⇒ raw stays None"
        );
        let recovered = usage_info_from_response(&model_response).expect("usage info");
        assert_eq!(recovered.charged_amount_usd, 0.0);
        assert_eq!(recovered.context_window, 0);
        assert_eq!(recovered.input_tokens, 5);
    }

    #[test]
    fn no_usage_reconstructs_to_none() {
        let chat = ChatResponse {
            text: Some("hi".to_string()),
            tool_calls: Vec::new(),
            usage: None,
            reasoning_content: None,
        };
        let model_response = response_to_model_response(&chat, &empty_registry(), false);
        assert!(usage_info_from_response(&model_response).is_none());
    }

    #[test]
    fn tool_less_response_preserves_literal_tool_call_markup() {
        let text = r#"Example: <tool_call>{"name":"lookup","arguments":{}}</tool_call>"#;
        let chat = ChatResponse {
            text: Some(text.to_string()),
            ..Default::default()
        };

        let response = response_to_model_response(&chat, &empty_registry(), false);

        assert_eq!(response.text(), text);
        assert!(response.message.tool_calls.is_empty());
    }

    #[test]
    fn tool_enabled_response_still_extracts_tool_call_markup() {
        let chat = ChatResponse {
            text: Some(r#"<tool_call>{"name":"lookup","arguments":{}}</tool_call>"#.to_string()),
            ..Default::default()
        };

        let response = response_to_model_response(&chat, &empty_registry(), true);

        assert_eq!(response.text(), "");
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(response.message.tool_calls[0].name, "lookup");
    }

    fn tool_request() -> ModelRequest {
        ModelRequest {
            tools: vec![tinyagents::harness::tool::ToolSchema::new(
                "lookup",
                "looks up a record",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" },
                        "query": { "type": "string" }
                    }
                }),
            )],
            ..Default::default()
        }
    }

    #[test]
    fn prompt_guided_response_uses_tinyagents_xml_parser() {
        let response = prompt_guided_text_response(
            r#"Checking.<tool_call>{"name":"lookup","arguments":{"id":7}}</tool_call>"#.to_string(),
            &tool_request(),
        );

        assert_eq!(response.text(), "Checking.");
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(response.message.tool_calls[0].id, "call_1");
        assert_eq!(response.message.tool_calls[0].name, "lookup");
        assert_eq!(
            response.message.tool_calls[0].arguments,
            serde_json::json!({"id": 7})
        );
    }

    #[test]
    fn prompt_guided_response_keeps_legacy_pformat_fallback() {
        let response = prompt_guided_text_response(
            "<tool_call>lookup[7|needle]</tool_call>".to_string(),
            &tool_request(),
        );

        assert_eq!(response.text(), "");
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(response.message.tool_calls[0].name, "lookup");
        assert_eq!(
            response.message.tool_calls[0].arguments,
            serde_json::json!({"id": 7, "query": "needle"})
        );
    }
}
