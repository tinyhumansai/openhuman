//! `tinyagents` [`ChatModel`] adapter over an openhuman [`Provider`] (issue #4249).
//!
//! Wraps `Arc<dyn Provider>` so the `tinyagents` agent-loop can drive a real
//! openhuman inference backend. On each model call the harness hands us a
//! provider-neutral [`ModelRequest`] (rich messages + advertised tool schemas);
//! we translate it into an openhuman [`ChatRequest`], call `provider.chat`, and
//! translate the [`ChatResponse`] back into a harness [`ModelResponse`] —
//! carrying through text, native tool calls, and token usage.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents::harness::message::{AssistantMessage, ContentBlock, MessageDelta};
use tinyagents::harness::model::{
    ChatModel, Modalities, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyagents::harness::tool::{ToolCall as TaToolCall, ToolDelta};
use tinyagents::harness::usage::Usage;
use tokio::sync::mpsc::UnboundedSender;

use super::abort_guard::AbortOnDrop;
use crate::openhuman::inference::provider::thread_context::{current_thread_id, with_thread_id};
use crate::openhuman::inference::provider::{
    current_route_slot, with_route_slot, ChatMessage, ChatRequest, ChatResponse, Provider,
    ProviderDelta, UsageInfo,
};
use crate::openhuman::tools::ToolSpec;

/// Translate a harness [`ModelRequest`] into openhuman's message list + tool
/// specs (shared by the buffered and streaming paths).
fn build_chat_inputs(
    request: &ModelRequest,
    native_tools: bool,
) -> (Vec<ChatMessage>, Vec<ToolSpec>) {
    // Native-tool providers need assistant tool calls + tool results encoded in
    // the provider's native envelope so a tool round round-trips; prompt-guided
    // providers need tool results folded into a `[Tool results]` user turn.
    let messages = if native_tools {
        request
            .messages
            .iter()
            .map(super::convert::message_to_native_chat_message)
            .collect()
    } else {
        super::convert::messages_to_text_mode_chat(&request.messages)
    };
    let specs = request
        .tools
        .iter()
        .map(|s| ToolSpec {
            name: s.name.clone(),
            description: s.description.clone(),
            parameters: s.parameters.clone(),
        })
        .collect();
    (messages, specs)
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
    if let Some(block) =
        super::convert::reasoning_content_block(response.reasoning_content.as_deref())
    {
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
    }
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
/// exactly as it does from a [`ProviderModel`].
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
/// `Box<dyn Provider>` (`chat` → `UsageInfo`) onto `Arc<dyn ChatModel>`
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
fn forward_delta(tx: &UnboundedSender<ModelStreamItem>, delta: ProviderDelta) {
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

/// A harness chat model backed by an openhuman [`Provider`].
///
/// The application `State` is `()` — openhuman tools and providers carry no
/// harness-visible shared state — so this adapter implements
/// `ChatModel<()>`.
/// Shared slot that preserves the most recent original provider error.
///
/// tinyagents carries errors as `TinyAgentsError::Model(String)`, which would
/// stringify openhuman's typed `anyhow::Error` (e.g. `AgentError::PermissionDenied`
/// / `MaxIterationsExceeded`) and break the downcast the caller relies on for
/// Sentry suppression and `AgentError`-tagged events. The adapter stashes the
/// original error here before returning the stringified one to the harness, so
/// the runner can re-surface the downcastable error after the run fails.
pub(super) type ProviderErrorSlot = Arc<Mutex<Option<anyhow::Error>>>;

pub(super) struct ProviderModel {
    provider: Arc<dyn Provider>,
    model: String,
    temperature: f64,
    max_tokens: Option<u32>,
    /// Preserves the last original provider error for the runner to re-surface.
    error_slot: ProviderErrorSlot,
    /// Capability profile derived from the wrapped provider (issue #4249,
    /// Phase 2): lets the crate validate a request against the model's actual
    /// capabilities (vision, tool calling, streaming, token limits) *before*
    /// a network call, and drives capability-aware registry resolution.
    profile: ModelProfile,
}

/// Builds a `ChatModel<()>` capability over an openhuman [`Provider`], pinned to
/// `model`/`temperature`, ready to register into a `tinyagents`
/// [`CapabilityRegistry`](tinyagents::registry::CapabilityRegistry) — e.g. the
/// `.ragsh` REPL bridge (`crate::openhuman::rhai_workflows`). [`ProviderModel::new`] is
/// `pub(super)`; this is the crate-visible factory for that one bridge use.
pub(crate) fn provider_chat_model(
    provider: Arc<dyn Provider>,
    model: impl Into<String>,
    temperature: f64,
) -> Arc<dyn ChatModel<()>> {
    Arc::new(ProviderModel::new(provider, model, temperature))
}

pub(super) struct MaxTokensModel {
    inner: Arc<dyn ChatModel<()>>,
    max_tokens: u32,
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

impl ProviderModel {
    /// Build a model adapter for `provider`, pinned to `model`/`temperature`.
    ///
    /// The adapter's [`ModelProfile`] is derived from the provider's declared
    /// capabilities at construction: vision → `modalities.image_in`, native
    /// tool calling → `tool_calling`/`parallel_tool_calls` (openhuman's
    /// `ChatResponse` carries multiple tool calls per response), and
    /// `supports_streaming` → `streaming`. `streaming_tool_chunks` stays
    /// `false` — [`ProviderModel::stream`] forwards text deltas only and
    /// reconstructs tool calls from the final response. Token limits are
    /// threaded in by the runner via [`ProviderModel::with_context_window`] /
    /// [`ProviderModel::with_max_tokens`].
    pub(super) fn new(
        provider: Arc<dyn Provider>,
        model: impl Into<String>,
        temperature: f64,
    ) -> Self {
        let model = model.into();
        // Read the canonical accessor methods (not `capabilities()` directly):
        // several providers override `supports_native_tools`/`supports_vision`
        // without overriding the `capabilities()` struct.
        let native_tools = provider.supports_native_tools();
        let profile = ModelProfile {
            provider: Some(
                if provider.is_local_provider_for_model(&model) {
                    "local"
                } else {
                    "remote"
                }
                .to_string(),
            ),
            model: Some(model.clone()),
            modalities: Modalities {
                image_in: provider.supports_vision(),
                ..Modalities::default()
            },
            tool_calling: native_tools,
            parallel_tool_calls: native_tools,
            streaming: provider.supports_streaming(),
            ..ModelProfile::default()
        };
        Self {
            provider,
            model,
            temperature,
            max_tokens: None,
            error_slot: Arc::new(Mutex::new(None)),
            profile,
        }
    }

    /// A handle to the shared error slot (clone before moving `self` into the
    /// harness, so the runner can recover the typed provider error on failure).
    pub(super) fn error_slot(&self) -> ProviderErrorSlot {
        self.error_slot.clone()
    }

    /// Cap the output tokens requested from the provider for every call.
    pub(super) fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self.profile.max_output_tokens = Some(u64::from(max_tokens));
        self
    }

    /// Record the model's effective context window on the profile so the crate
    /// can validate/select on input capacity before dispatch. Metadata only —
    /// history trimming stays with the context middlewares.
    pub(super) fn with_context_window(mut self, window: u64) -> Self {
        self.profile.max_input_tokens = Some(window);
        self
    }

    /// Override the profile's image-input (vision) capability.
    ///
    /// [`ProviderModel::new`] seeds `modalities.image_in` from the provider's
    /// *provider-wide* `supports_vision()`, but a workload-route projection
    /// (issue #4249, Workstream 02.1 — see [`super::routes`]) knows the
    /// per-route vision capability (e.g. the dedicated `vision-v1` tier is
    /// multimodal while `chat-v1` is text-only). This lets the route adapter
    /// record the accurate per-route modality so capability gating can reject a
    /// non-vision route for an image turn before dispatch.
    pub(super) fn with_vision(mut self, image_in: bool) -> Self {
        self.profile.modalities.image_in = image_in;
        self
    }

    /// Override the profile's reasoning/thinking capability. Set by the
    /// workload-route projection ([`super::routes`]) for reasoning-tier routes so
    /// a request that requires reasoning resolves to a reasoning-capable model.
    pub(super) fn with_reasoning(mut self, reasoning: bool) -> Self {
        self.profile.reasoning = reasoning;
        self
    }

    /// Override tool calling for this model without changing the underlying
    /// provider. Text-mode sub-agents use this to preserve injected providers
    /// while omitting native tool schemas from their requests.
    pub(super) fn with_tool_calling(mut self, enabled: bool) -> Self {
        self.profile.tool_calling = enabled;
        self.profile.parallel_tool_calls = enabled;
        self
    }
}

#[async_trait]
impl ChatModel<()> for ProviderModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let native = self.profile.tool_calling;
        let (messages, specs) = build_chat_inputs(&request, native);
        // Honor a per-request temperature when the caller sets one (e.g. one-shot
        // inference callers that reuse a single model across prompts of differing
        // temperature), else fall back to the temperature pinned at construction.
        // The agent-loop seam leaves `request.temperature` `None`, so the pinned
        // value still governs every turn — behaviour-neutral for the harness path.
        let temperature = request.temperature.unwrap_or(self.temperature);
        // Positional layouts for the text-mode P-Format fallback (issue #4465);
        // empty (and thus behaviour-neutral) when no tools are advertised.
        let pformat_registry = pformat_registry_from_request(&request);
        let chat_request = ChatRequest {
            messages: &messages,
            // Only advertise structured tool specs to native providers. Prompt-
            // guided providers (Ollama/LM Studio profiles) get the tool catalogue
            // folded into the transcript instead; sending a `tools`/`tool_choice`
            // payload would defeat the opt-out and get rejected/ignored.
            tools: (native && !specs.is_empty()).then_some(&specs),
            stream: None,
            // Prefer a per-request output cap when the caller set one, else the
            // cap pinned at construction. The agent-loop seam pins via
            // `with_max_tokens` and leaves `request.max_tokens` `None`
            // (openhuman never sets the crate `RunConfig.max_turn_output_tokens`),
            // so the pinned cap still governs every turn.
            max_tokens: request.max_tokens.or(self.max_tokens),
        };

        tracing::debug!(
            model = %self.model,
            messages = messages.len(),
            tools = specs.len(),
            "[tinyagents] provider.chat via harness model adapter"
        );

        let response = match self
            .provider
            .chat(chat_request, &self.model, temperature)
            .await
        {
            Ok(response) => {
                // #4457 (defect B): the error slot preserves the last provider
                // error for the runner to re-surface as the typed turn failure.
                // A call that *succeeds* — including one the provider fallback
                // chain recovered after an inner error — must clear any stale
                // error so a later, unrelated run failure (e.g. the model-call
                // cap) is not misclassified as that recovered provider error.
                if self.error_slot.lock().unwrap().take().is_some() {
                    tracing::debug!(
                        model = %self.model,
                        "[models] provider chat succeeded; cleared stale error_slot — #4457 defect B"
                    );
                }
                response
            }
            Err(e) => {
                // Classify with OpenHuman's product error taxonomy (issue #4249,
                // Workstream 02.2): a permanent config/auth rejection, billing/quota
                // exhaustion, or context-window overflow is mapped to a *non-retryable*
                // `TinyAgentsError::Validation` (crate `is_retryable` → false), while a
                // transient 5xx/429/network blip stays a retryable `Model` error. This
                // is the same `reliable::is_non_retryable` classifier `ReliableProvider`
                // uses, keeping OpenHuman as the single `ProviderError` mapper. With the
                // retry pin at a single attempt the mapping is behavior-neutral today; it
                // stages honest retry semantics for when the crate loop owns retries.
                let non_retryable =
                    crate::openhuman::inference::provider::reliable::is_non_retryable(&e);
                tracing::debug!(
                    model = %self.model,
                    non_retryable,
                    "[models] provider chat failed; classifying error for tinyagents retry/fallback"
                );
                // Preserve the original (downcastable) error for the runner, then
                // hand the harness a stringified copy to stop the loop.
                let msg = format!("openhuman provider chat failed: {e}");
                *self.error_slot.lock().unwrap() = Some(e);
                return Err(if non_retryable {
                    tinyagents::TinyAgentsError::Validation(msg)
                } else {
                    tinyagents::TinyAgentsError::Model(msg)
                });
            }
        };
        // The buffered path is used only for unobserved turns (no progress sink):
        // the seam sets `streaming = on_progress.is_some()`, so any post-hoc
        // reasoning here would have nowhere to go. Observed turns take `stream()`,
        // which forwards reasoning natively. Reasoning still rides the response as
        // a typed thinking block (see `response_to_model_response`) for
        // persistence/replay.
        // Provider usage (charged USD / context window / cache-creation-reasoning)
        // now reaches the event bridge via `UsageCarryMiddleware`, which reads it
        // off the returned `ModelResponse` (G1) — the adapter no longer carries it.
        Ok(response_to_model_response(
            &response,
            &pformat_registry,
            !request.tools.is_empty(),
        ))
    }

    /// Stream the model response, forwarding openhuman's `ProviderDelta` events
    /// as harness [`ModelStreamItem`]s so the agent loop emits live `ModelDelta`
    /// events (which the [`OpenhumanEventBridge`](super::OpenhumanEventBridge)
    /// mirrors onto `AgentProgress` text deltas).
    ///
    /// A streaming-capable provider forwards incremental text to the
    /// per-call delta channel; a non-streaming provider simply returns the
    /// aggregated response, which still arrives as the terminal `Completed`
    /// item. Native tool calls always ride on `Completed`.
    async fn stream(&self, _state: &(), request: ModelRequest) -> tinyagents::Result<ModelStream> {
        let native = self.profile.tool_calling;
        let (messages, specs) = build_chat_inputs(&request, native);
        // Positional layouts for the text-mode P-Format fallback (issue #4465);
        // built here so it can move into the `'static` producer task below.
        let pformat_registry = pformat_registry_from_request(&request);
        let parse_text_tool_calls = !request.tools.is_empty();
        let provider = self.provider.clone();
        let model = self.model.clone();
        // Per-request temperature when set (see `invoke`), else the pinned value;
        // the agent-loop seam leaves it `None`, so streamed turns are unchanged.
        let temperature = request.temperature.unwrap_or(self.temperature);
        // Same precedence for the output cap (see `invoke`).
        let max_tokens = request.max_tokens.or(self.max_tokens);
        let error_slot = self.error_slot.clone();

        let (item_tx, item_rx) = tokio::sync::mpsc::unbounded_channel::<ModelStreamItem>();

        // #4460: the producer below runs in a detached `tokio::spawn`, and
        // `tokio::task_local`s do NOT propagate across a spawn boundary. Capture
        // the two ambient task-locals the provider call depends on *here*, on the
        // caller's task, and re-establish them inside the spawn:
        //   - `thread_id`  → the managed backend's `thread_id` extension
        //     (`compatible_request::outbound_thread_id`) so streamed requests stay
        //     attributed to the right chat / prompt-cache group.
        //   - resolved-route audit slot → so `record_resolved_provider_route`
        //     calls inside `provider.chat` write back to the caller's scope and the
        //     channel audit reports the *resolved* route, not the requested one.
        let thread_id = current_thread_id();
        let route_slot = current_route_slot();
        // Label for the abort-on-drop debug log; the moved-in `model` clone is
        // consumed by the producer body.
        let abort_label = model.clone();
        tracing::debug!(
            model = %model,
            thread_id = thread_id.as_deref().unwrap_or("<none>"),
            route_slot = route_slot.is_some(),
            "[tinyagents] spawning streamed provider producer; re-establishing task-locals across spawn — #4460"
        );

        // Producer: run the provider call while forwarding its incremental
        // deltas, then emit the terminal item. Everything captured is owned, so
        // the task is `'static`.
        let producer = async move {
            let _ = item_tx.send(ModelStreamItem::Started);
            let (delta_tx, mut delta_rx) = tokio::sync::mpsc::channel::<ProviderDelta>(64);
            let chat_fut = async {
                let req = ChatRequest {
                    messages: &messages,
                    // Prompt-guided providers get the tool catalogue in the
                    // transcript, not a structured `tools` payload (see the
                    // buffered path). `native` is captured by the async move.
                    tools: (native && !specs.is_empty()).then_some(&specs),
                    stream: Some(&delta_tx),
                    max_tokens,
                };
                provider.chat(req, &model, temperature).await
            };
            tokio::pin!(chat_fut);

            let mut streamed_thinking = false;
            let response = loop {
                tokio::select! {
                    maybe = delta_rx.recv() => {
                        if let Some(delta) = maybe {
                            streamed_thinking |= matches!(delta, ProviderDelta::ThinkingDelta { .. });
                            forward_delta(&item_tx, delta);
                        }
                    }
                    res = &mut chat_fut => break res,
                }
            };
            // Drain any deltas that landed before the call returned.
            while let Ok(delta) = delta_rx.try_recv() {
                streamed_thinking |= matches!(delta, ProviderDelta::ThinkingDelta { .. });
                forward_delta(&item_tx, delta);
            }

            let terminal = match response {
                Ok(resp) => {
                    // #4457 (defect B): a successful streaming call — including
                    // one recovered by the provider fallback chain — clears any
                    // stale error preserved in the slot so a later unrelated run
                    // failure is not misclassified as that recovered error.
                    if error_slot.lock().unwrap().take().is_some() {
                        tracing::debug!(
                            model = %model,
                            "[models] streaming provider chat succeeded; cleared stale error_slot — #4457 defect B"
                        );
                    }
                    // Fallback for streaming providers that return reasoning only
                    // on the aggregated response (no incremental thinking
                    // deltas): emit it once through the native crate stream so
                    // the bridge handles scope consistently with live reasoning.
                    if !streamed_thinking {
                        if let Some(reasoning) =
                            resp.reasoning_content.as_ref().filter(|r| !r.is_empty())
                        {
                            let _ = item_tx.send(ModelStreamItem::MessageDelta(
                                MessageDelta::reasoning(reasoning.clone()),
                            ));
                        }
                    }
                    // Provider usage rides the `Completed` response's crate `Usage`
                    // + raw (G1); `UsageCarryMiddleware` reads it off the folded
                    // response for the bridge, so the adapter no longer pushes here.
                    ModelStreamItem::Completed(response_to_model_response(
                        &resp,
                        &pformat_registry,
                        parse_text_tool_calls,
                    ))
                }
                Err(e) => {
                    // Streaming failures ride `ModelStreamItem::Failed(String)`, which
                    // carries no retryable flag (the harness treats it as a retryable
                    // `Model` error), so the non-retryable mapping applied on the
                    // buffered path cannot be expressed here — a crate limitation. With
                    // the retry pin at a single attempt this has no effect today; logged
                    // under `[models]` for parity/auditability (issue #4249, 02.2).
                    let non_retryable =
                        crate::openhuman::inference::provider::reliable::is_non_retryable(&e);
                    tracing::debug!(
                        model = %model,
                        non_retryable,
                        "[models] streaming provider chat failed; harness will treat as retryable Model error"
                    );
                    // Preserve the original (downcastable) error for the runner.
                    let msg = format!("openhuman provider chat failed: {e}");
                    *error_slot.lock().unwrap() = Some(e);
                    ModelStreamItem::Failed(msg)
                }
            };
            let _ = item_tx.send(terminal);
        };

        // Re-establish the captured task-locals inside the spawned task (#4460).
        // `with_thread_id` normalizes an absent id to `None`, so it is a no-op
        // when there was no ambient thread; the route slot is only re-scoped when
        // an enclosing `with_resolved_provider_route_scope` supplied one.
        let handle = tokio::spawn(async move {
            let scoped = with_thread_id(thread_id.unwrap_or_default(), producer);
            match route_slot {
                Some(slot) => with_route_slot(slot, scoped).await,
                None => scoped.await,
            }
        });

        // #4460: tie the producer's lifetime to the consumer. Moving the
        // abort-on-drop guard into the stream state means that dropping the
        // stream (the turn future being hard-cancelled via `AbortHandle`, or
        // dropped for any other reason) aborts the in-flight `provider.chat` call
        // instead of letting it run — and bill — to completion in the background.
        let guard = AbortOnDrop::new(handle, abort_label);
        let stream = futures_util::stream::unfold((item_rx, guard), |(mut rx, guard)| async move {
            rx.recv().await.map(|item| (item, (rx, guard)))
        });
        Ok(Box::pin(stream))
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
}

#[cfg(test)]
mod adapter_param_tests {
    //! The adapter honors a per-request temperature / output cap when the caller
    //! sets one (one-shot callers reuse a model across differing prompts), and
    //! otherwise the value pinned at construction (the agent-loop seam path).
    use super::*;
    use tinyagents::harness::message::Message;

    #[derive(Default)]
    struct CaptureProvider {
        seen: Arc<Mutex<Vec<(f64, Option<u32>)>>>,
    }

    #[async_trait]
    impl Provider for CaptureProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            unreachable!("chat() is overridden")
        }

        async fn chat(
            &self,
            request: ChatRequest<'_>,
            _model: &str,
            temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            self.seen
                .lock()
                .unwrap()
                .push((temperature, request.max_tokens));
            Ok(ChatResponse {
                text: Some("ok".to_string()),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn per_request_overrides_win_else_pinned() {
        let seen: Arc<Mutex<Vec<(f64, Option<u32>)>>> = Arc::default();
        let provider: Arc<dyn Provider> = Arc::new(CaptureProvider { seen: seen.clone() });
        let model = ProviderModel::new(provider, "m", 0.7).with_max_tokens(100);

        // Request carries its own temperature + cap → those win.
        model
            .invoke(
                &(),
                ModelRequest::new(vec![Message::user("x")])
                    .with_temperature(0.1)
                    .with_max_tokens(42),
            )
            .await
            .unwrap();
        // Request leaves both unset → pinned construction values apply.
        model
            .invoke(&(), ModelRequest::new(vec![Message::user("x")]))
            .await
            .unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(
            seen[0],
            (0.1, Some(42)),
            "per-request temperature + cap win"
        );
        assert_eq!(seen[1], (0.7, Some(100)), "unset falls back to pinned");
    }
}
