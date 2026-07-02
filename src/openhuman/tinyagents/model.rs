//! `tinyagents` [`ChatModel`] adapter over an openhuman [`Provider`] (issue #4249).
//!
//! Wraps `Arc<dyn Provider>` so the `tinyagents` agent-loop can drive a real
//! openhuman inference backend. On each model call the harness hands us a
//! provider-neutral [`ModelRequest`] (rich messages + advertised tool schemas);
//! we translate it into an openhuman [`ChatRequest`], call `provider.chat`, and
//! translate the [`ChatResponse`] back into a harness [`ModelResponse`] —
//! carrying through text, native tool calls, and token usage.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents::harness::message::{AssistantMessage, ContentBlock, MessageDelta};
use tinyagents::harness::model::{
    ChatModel, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyagents::harness::tool::ToolCall as TaToolCall;
use tinyagents::harness::usage::Usage;
use tokio::sync::mpsc::{Sender, UnboundedSender};

use super::observability::{IterationCursor, SubagentScope};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ChatResponse, Provider, ProviderDelta,
};
use crate::openhuman::tools::ToolSpec;

/// Out-of-band forwarder for the streaming progress events that don't round-trip
/// through tinyagents: model reasoning (thinking) deltas and tool-call **argument**
/// deltas.
///
/// tinyagents' streaming `MessageDelta` carries only assembled visible text — no
/// reasoning channel, and the model adapter assembles tool calls itself rather
/// than streaming their argument fragments through the harness — so the
/// [`OpenhumanEventBridge`](super::OpenhumanEventBridge) can't mirror either. The
/// model adapter is the only seam that sees the provider's
/// [`ProviderDelta::ThinkingDelta`] / [`ProviderDelta::ToolCallArgsDelta`], so it
/// forwards them straight onto the progress sink here, sharing the bridge's
/// [`IterationCursor`] so each delta is attributed to the right model call.
/// Parent runs emit the top-level variants; child runs emit the `Subagent`
/// counterpart for thinking (tool-arg deltas have no child variant, so they ride
/// the top-level event).
#[derive(Clone)]
pub struct ThinkingForwarder {
    sink: Sender<AgentProgress>,
    scope: Option<SubagentScope>,
    cursor: IterationCursor,
    /// call_id → tool_name, learned from `ToolCallStart`, so an args delta can
    /// carry the tool name the UI labels it with.
    tool_names: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

impl ThinkingForwarder {
    pub fn new(
        sink: Sender<AgentProgress>,
        scope: Option<SubagentScope>,
        cursor: IterationCursor,
    ) -> Self {
        Self {
            sink,
            scope,
            cursor,
            tool_names: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Best-effort, non-blocking emit of one reasoning chunk (drops on a full
    /// channel, matching the streaming text path).
    fn emit(&self, delta: String) {
        if delta.is_empty() {
            return;
        }
        let iteration = self.cursor.load(Ordering::SeqCst);
        let progress = match &self.scope {
            None => AgentProgress::ThinkingDelta { delta, iteration },
            Some(s) => AgentProgress::SubagentThinkingDelta {
                agent_id: s.agent_id.clone(),
                task_id: s.task_id.clone(),
                delta,
                iteration,
            },
        };
        let _ = self.sink.try_send(progress);
    }

    /// Record the tool name a streaming tool call starts with, and emit the
    /// start marker — an empty-delta `ToolCallArgsDelta` — so consumers see the
    /// call begin before its arguments arrive (matching the legacy
    /// `ProviderDelta::ToolCallStart` mapping).
    fn note_tool_call(&self, call_id: String, tool_name: String) {
        self.tool_names
            .lock()
            .unwrap()
            .insert(call_id.clone(), tool_name.clone());
        let _ = self.sink.try_send(AgentProgress::ToolCallArgsDelta {
            call_id,
            tool_name,
            delta: String::new(),
            iteration: self.cursor.load(Ordering::SeqCst),
        });
    }

    /// Emit one tool-call argument fragment as `ToolCallArgsDelta` so the UI can
    /// show the model composing the call before it executes.
    fn emit_tool_args(&self, call_id: String, delta: String) {
        if delta.is_empty() {
            return;
        }
        let tool_name = self
            .tool_names
            .lock()
            .unwrap()
            .get(&call_id)
            .cloned()
            .unwrap_or_default();
        let _ = self.sink.try_send(AgentProgress::ToolCallArgsDelta {
            call_id,
            tool_name,
            delta,
            iteration: self.cursor.load(Ordering::SeqCst),
        });
    }
}

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
    // The unknown-tool sentinel is a recovery adapter, never a real capability —
    // its contract (see `tools::UNKNOWN_TOOL_SENTINEL`) is that it is never
    // advertised to the model. Filter it out of the advertised specs so it never
    // leaks into the provider's tool list.
    let specs = request
        .tools
        .iter()
        .filter(|s| s.name != super::tools::UNKNOWN_TOOL_SENTINEL)
        .map(|s| ToolSpec {
            name: s.name.clone(),
            description: s.description.clone(),
            parameters: s.parameters.clone(),
        })
        .collect();
    (messages, specs)
}

/// Translate an openhuman [`ChatResponse`] into a harness [`ModelResponse`]
/// (visible text + tool calls + token usage).
///
/// Native `tool_calls` take precedence; when absent, the response text is parsed
/// for prompt-guided (`<tool_call>…` / p-format) calls — matching the legacy
/// dispatcher — so text-mode models drive the tinyagents loop too. The visible
/// text is the prose with any tool-call markup stripped.
///
/// Rewriting a hallucinated/unadvertised tool call onto the recovery sentinel
/// now happens at the tool boundary in
/// [`UnknownToolRewriteMiddleware`](super::middleware) (`before_tool`), not here.
fn response_to_model_response(response: &ChatResponse) -> ModelResponse {
    let (visible_text, tool_calls): (String, Vec<TaToolCall>) = if !response.tool_calls.is_empty() {
        let calls = response
            .tool_calls
            .iter()
            .map(|tc| TaToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: serde_json::from_str(&tc.arguments).unwrap_or(serde_json::Value::Null),
            })
            .collect();
        (response.text.clone().unwrap_or_default(), calls)
    } else if let Some(text) = response.text.as_deref() {
        let (prose, parsed) = crate::openhuman::agent::harness::parse_tool_calls(text);
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
                })
                .collect();
            (prose, calls)
        }
    } else {
        (String::new(), Vec::new())
    };

    let mut content = Vec::new();
    if !visible_text.is_empty() {
        content.push(ContentBlock::Text(visible_text));
    }
    // Thinking models return `reasoning_content` separately from the visible
    // reply; tinyagents' `AssistantMessage` has no reasoning channel, so stash it
    // on a provider-extension content block. It stays out of `Message::text()`
    // (which only concatenates `Text` blocks) but survives into persistence and
    // the next turn's request — where thinking-mode providers require it back.
    if let Some(block) =
        super::convert::reasoning_content_block(response.reasoning_content.as_deref())
    {
        content.push(block);
    }
    let usage = response.usage.as_ref().map(|u| {
        // Carry the provider's cached-prefix input count through the crate
        // `Usage` (it has a `cache_read_tokens` field) so downstream cost
        // accounting can price it at the cached rate. `Usage::new` seeds
        // input/output/total; set the cache field on top. (`charged_amount_usd`
        // has no crate home; the event bridge estimates cost from token counts.)
        let mut usage = Usage::new(u.input_tokens, u.output_tokens);
        usage.cache_read_tokens = u.cached_input_tokens;
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
        raw: None,
        resolved_model: None,
    }
}

/// Forward one openhuman [`ProviderDelta`]. Visible text becomes a harness
/// [`MessageDelta`] (so the bridge mirrors it as a text delta); reasoning and
/// tool-call **argument** fragments ride the out-of-band [`ThinkingForwarder`]
/// (the harness stream carries neither). The model adapter still assembles the
/// final native tool calls from the `Completed` response — these fragments are
/// progress-only, so the UI can show the call being composed.
fn forward_delta(
    tx: &UnboundedSender<ModelStreamItem>,
    thinking: Option<&ThinkingForwarder>,
    delta: ProviderDelta,
) {
    match delta {
        ProviderDelta::TextDelta { delta } => {
            if !delta.is_empty() {
                // `MessageDelta::text` sets the visible-text fragment and defaults
                // the `reasoning` (new in tinyagents 1.2.0) and `tool_call` fields.
                // Reasoning still rides the out-of-band `ThinkingForwarder` below
                // (see `ThinkingDelta`) rather than the native `reasoning` channel,
                // preserving the existing subagent-scoped thinking UI wiring.
                let _ = tx.send(ModelStreamItem::MessageDelta(MessageDelta::text(delta)));
            }
        }
        ProviderDelta::ThinkingDelta { delta } => {
            if let Some(forwarder) = thinking {
                forwarder.emit(delta);
            }
        }
        ProviderDelta::ToolCallStart { call_id, tool_name } => {
            if let Some(forwarder) = thinking {
                forwarder.note_tool_call(call_id, tool_name);
            }
        }
        ProviderDelta::ToolCallArgsDelta { call_id, delta } => {
            if let Some(forwarder) = thinking {
                forwarder.emit_tool_args(call_id, delta);
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
pub type ProviderErrorSlot = Arc<Mutex<Option<anyhow::Error>>>;

pub struct ProviderModel {
    provider: Arc<dyn Provider>,
    model: String,
    temperature: f64,
    max_tokens: Option<u32>,
    /// When set, the adapter forwards provider reasoning deltas onto the
    /// progress sink (the harness stream has no reasoning channel).
    thinking: Option<ThinkingForwarder>,
    /// Preserves the last original provider error for the runner to re-surface.
    error_slot: ProviderErrorSlot,
}

impl ProviderModel {
    /// Build a model adapter for `provider`, pinned to `model`/`temperature`.
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<String>, temperature: f64) -> Self {
        Self {
            provider,
            model: model.into(),
            temperature,
            max_tokens: None,
            thinking: None,
            error_slot: Arc::new(Mutex::new(None)),
        }
    }

    /// A handle to the shared error slot (clone before moving `self` into the
    /// harness, so the runner can recover the typed provider error on failure).
    pub fn error_slot(&self) -> ProviderErrorSlot {
        self.error_slot.clone()
    }

    /// Cap the output tokens requested from the provider for every call.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Forward provider reasoning / thinking deltas onto a progress sink via
    /// `forwarder` (parent or sub-agent scoped). See [`ThinkingForwarder`].
    pub fn with_thinking(mut self, forwarder: ThinkingForwarder) -> Self {
        self.thinking = Some(forwarder);
        self
    }
}

#[async_trait]
impl ChatModel<()> for ProviderModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let native = self.provider.supports_native_tools();
        let (messages, specs) = build_chat_inputs(&request, native);
        let chat_request = ChatRequest {
            messages: &messages,
            // Only advertise structured tool specs to native providers. Prompt-
            // guided providers (Ollama/LM Studio profiles) get the tool catalogue
            // folded into the transcript instead; sending a `tools`/`tool_choice`
            // payload would defeat the opt-out and get rejected/ignored.
            tools: (native && !specs.is_empty()).then_some(&specs),
            stream: None,
            max_tokens: self.max_tokens,
        };

        tracing::debug!(
            model = %self.model,
            messages = messages.len(),
            tools = specs.len(),
            "[tinyagents] provider.chat via harness model adapter"
        );

        let response = match self
            .provider
            .chat(chat_request, &self.model, self.temperature)
            .await
        {
            Ok(response) => response,
            Err(e) => {
                // Preserve the original (downcastable) error for the runner, then
                // hand the harness a stringified copy to stop the loop.
                let msg = format!("openhuman provider chat failed: {e}");
                *self.error_slot.lock().unwrap() = Some(e);
                return Err(tinyagents::TinyAgentsError::Model(msg));
            }
        };
        // Non-streaming path: surface any reasoning the provider returned as a
        // single post-hoc thinking delta (it had no per-token channel to ride).
        if let Some(forwarder) = &self.thinking {
            if let Some(reasoning) = response
                .reasoning_content
                .as_ref()
                .filter(|r| !r.is_empty())
            {
                forwarder.emit(reasoning.clone());
            }
        }
        Ok(response_to_model_response(&response))
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
        let native = self.provider.supports_native_tools();
        let (messages, specs) = build_chat_inputs(&request, native);
        let provider = self.provider.clone();
        let model = self.model.clone();
        let temperature = self.temperature;
        let max_tokens = self.max_tokens;
        let thinking = self.thinking.clone();
        let error_slot = self.error_slot.clone();

        let (item_tx, item_rx) = tokio::sync::mpsc::unbounded_channel::<ModelStreamItem>();

        // Producer: run the provider call while forwarding its incremental
        // deltas, then emit the terminal item. Everything captured is owned, so
        // the task is `'static`.
        tokio::spawn(async move {
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
                            forward_delta(&item_tx, thinking.as_ref(), delta);
                        }
                    }
                    res = &mut chat_fut => break res,
                }
            };
            // Drain any deltas that landed before the call returned.
            while let Ok(delta) = delta_rx.try_recv() {
                streamed_thinking |= matches!(delta, ProviderDelta::ThinkingDelta { .. });
                forward_delta(&item_tx, thinking.as_ref(), delta);
            }

            let terminal = match response {
                Ok(resp) => {
                    // Fallback for providers that return reasoning only on the
                    // aggregated response (no incremental thinking deltas): emit
                    // it once so child/parent thinking output isn't lost.
                    if !streamed_thinking {
                        if let Some(forwarder) = &thinking {
                            if let Some(reasoning) =
                                resp.reasoning_content.as_ref().filter(|r| !r.is_empty())
                            {
                                forwarder.emit(reasoning.clone());
                            }
                        }
                    }
                    ModelStreamItem::Completed(response_to_model_response(&resp))
                }
                Err(e) => {
                    // Preserve the original (downcastable) error for the runner.
                    let msg = format!("openhuman provider chat failed: {e}");
                    *error_slot.lock().unwrap() = Some(e);
                    ModelStreamItem::Failed(msg)
                }
            };
            let _ = item_tx.send(terminal);
        });

        let stream = futures_util::stream::unfold(item_rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        Ok(Box::pin(stream))
    }
}
