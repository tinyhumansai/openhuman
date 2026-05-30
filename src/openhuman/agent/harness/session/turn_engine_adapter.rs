//! Engine seams for the stateful `Agent::turn`.
//!
//! These adapt the `Agent` to the shared [`run_turn_engine`] so web/desktop
//! chat runs the same loop as every other entry point, while preserving the
//! Agent's richer state: typed `ConversationMessage` history (with structured
//! tool calls + round-tripped `reasoning_content`), the `ContextManager`
//! reduction chain, KV-cache transcript prefixes, transcript persistence, and
//! the pluggable `ToolDispatcher` (incl. PFormat).
//!
//! * [`AgentToolSource`] owns `Arc`/value clones of the Agent's tool state
//!   (disjoint from the `&mut Agent` the observer holds) and runs each call
//!   through the shared [`run_agent_tool_call`], collecting `ToolCallRecord`s.
//! * [`AgentObserver`] borrows the `Agent` mutably: it runs the context
//!   reduction + re-materializes the engine's `ChatMessage` buffer from the
//!   typed history each iteration, rebuilds the typed history from the engine's
//!   per-iteration callbacks, accumulates usage, and persists the transcript.
//! * [`AgentCheckpoint`] summarizes the turn-so-far into a resumable checkpoint
//!   when the iteration cap is hit (mirrors `summarize_iteration_checkpoint`).

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use super::agent_tool_exec::{run_agent_tool_call, AgentToolExecCtx};
use super::transcript;
use super::turn_checkpoint::MAX_ITER_CHECKPOINT_INSTRUCTION;
use super::types::Agent;
use crate::openhuman::agent::dispatcher::{
    ParsedToolCall as DispatcherParsedToolCall, ToolDispatcher, ToolExecutionResult,
};
use crate::openhuman::agent::harness::engine::{
    CheckpointOutcome, CheckpointStrategy, ProgressReporter, ToolRunResult, ToolSource,
    TurnObserver,
};
use crate::openhuman::agent::harness::parse::ParsedToolCall;
use crate::openhuman::agent::harness::payload_summarizer::PayloadSummarizer;
use crate::openhuman::agent::hooks::ToolCallRecord;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::tool_policy::ToolPolicy;
use crate::openhuman::agent_tool_policy::ToolPolicySession;
use crate::openhuman::context::ReductionOutcome;
use crate::openhuman::inference::model_context::context_window_for_model;
use crate::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ConversationMessage, Provider, ProviderDelta, ToolCall, UsageInfo,
};
use crate::openhuman::tools::{Tool, ToolSpec};

/// Rebuild the persisted `Vec<ToolCall>` for an assistant-with-tools history
/// entry: prefer the provider's native calls, else synthesise from the parsed
/// calls (mirrors `Agent::persisted_tool_calls_for_history`).
fn persisted_tool_calls(
    native: &[ToolCall],
    parsed: &[ParsedToolCall],
    results: &[ToolExecutionResult],
    iteration: usize,
) -> Vec<ToolCall> {
    if !native.is_empty() {
        return native.to_vec();
    }
    // Synthesise from the parsed calls, reusing the *exact* id each result was
    // recorded under (`results[i].tool_call_id`) so the persisted assistant
    // tool-call id matches its `ToolResults` entry — what the next provider
    // turn (and history-fidelity tests) rely on.
    parsed
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let id = results
                .get(idx)
                .and_then(|r| r.tool_call_id.clone())
                .or_else(|| c.id.clone())
                .unwrap_or_else(|| format!("parsed-{}-{}", iteration + 1, idx + 1));
            ToolCall {
                id,
                name: c.name.clone(),
                arguments: c.arguments.to_string(),
            }
        })
        .collect()
}

/// Tool source for `Agent::turn`. Owns clones of the Agent's tool state so it
/// doesn't borrow the `Agent` (which [`AgentObserver`] holds mutably).
pub(super) struct AgentToolSource {
    pub tools: Arc<Vec<Box<dyn Tool>>>,
    pub visible_tool_names: HashSet<String>,
    pub tool_policy_session: ToolPolicySession,
    pub tool_policy: Arc<dyn ToolPolicy>,
    pub payload_summarizer: Option<Arc<dyn PayloadSummarizer>>,
    pub event_session_id: String,
    pub event_channel: String,
    pub agent_definition_id: String,
    pub prefer_markdown: bool,
    pub budget_bytes: usize,
    pub should_send_specs: bool,
    pub advertised_specs: Vec<ToolSpec>,
    /// Collected per-call records, drained by the post-loop epilogue for hooks.
    pub records: Vec<ToolCallRecord>,
}

#[async_trait]
impl ToolSource for AgentToolSource {
    fn request_specs(&self) -> &[ToolSpec] {
        if self.should_send_specs {
            &self.advertised_specs
        } else {
            &[]
        }
    }

    async fn execute_call(
        &mut self,
        call: &ParsedToolCall,
        iteration: usize,
        progress: &dyn ProgressReporter,
        _progress_call_id: &str,
    ) -> ToolRunResult {
        // `run_agent_tool_call` takes the dispatcher's `ParsedToolCall` shape;
        // convert from the engine's internal one.
        let dispatcher_call = DispatcherParsedToolCall {
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            tool_call_id: call.id.clone(),
        };
        let ctx = AgentToolExecCtx {
            tools: &self.tools,
            visible_tool_names: &self.visible_tool_names,
            tool_policy_session: &self.tool_policy_session,
            tool_policy: self.tool_policy.as_ref(),
            payload_summarizer: self.payload_summarizer.as_deref(),
            event_session_id: &self.event_session_id,
            event_channel: &self.event_channel,
            agent_definition_id: &self.agent_definition_id,
            prefer_markdown: self.prefer_markdown,
            budget_bytes: self.budget_bytes,
        };
        let (exec_result, record) =
            run_agent_tool_call(&ctx, progress, &dispatcher_call, iteration).await;
        self.records.push(record);
        ToolRunResult {
            text: exec_result.output,
            success: exec_result.success,
        }
    }
}

/// Turn observer for `Agent::turn`: owns the typed-history rebuild, context
/// management, usage accounting, and transcript persistence.
pub(super) struct AgentObserver<'a> {
    pub agent: &'a mut Agent,
    pub effective_model: String,
    pub cumulative_input: u64,
    pub cumulative_output: u64,
    pub cumulative_cached: u64,
    pub cumulative_charged: f64,
    pub last_turn_usage: Option<transcript::TurnUsage>,
    /// Cached transcript prefix for KV-cache reuse on a resumed session,
    /// consumed on the first iteration.
    pub cached_prefix: Option<Vec<ChatMessage>>,
    /// Tool results buffered during the per-call loop, flushed to typed history
    /// via the dispatcher's `format_results` once the assistant turn lands.
    pub pending_results: Vec<ToolExecutionResult>,
    /// Whether the engine reported a clean final response (so the post-loop
    /// epilogue knows not to push `outcome.text` itself).
    pub did_push_final: bool,
}

impl AgentObserver<'_> {
    fn persist(&mut self) {
        let messages = self
            .agent
            .tool_dispatcher
            .to_provider_messages(&self.agent.history);
        self.agent.persist_session_transcript(
            &messages,
            self.cumulative_input,
            self.cumulative_output,
            self.cumulative_cached,
            self.cumulative_charged,
            self.last_turn_usage.as_ref(),
        );
    }
}

#[async_trait]
impl TurnObserver for AgentObserver<'_> {
    async fn before_dispatch(
        &mut self,
        buf: &mut Vec<ChatMessage>,
        _iteration: usize,
    ) -> Result<()> {
        // Pre-dispatch token-budget trim on the typed history.
        if let Some(context_window) = context_window_for_model(&self.effective_model) {
            super::super::token_budget::trim_conversation_history_to_budget(
                &mut self.agent.history,
                context_window,
            );
        }
        // Global context-management reduction chain.
        let outcome = self
            .agent
            .context
            .reduce_before_call(&mut self.agent.history)
            .await?;
        if let ReductionOutcome::Exhausted {
            utilisation_pct,
            reason,
        } = &outcome
        {
            return Err(anyhow::anyhow!(
                "Context window exhausted ({utilisation_pct}% full): {reason}"
            ));
        }

        // Re-materialize the engine's ChatMessage buffer from the typed
        // history. On the first iteration of a resumed session, splice the
        // byte-identical cached prefix + the new user-message tail for KV-cache
        // reuse; otherwise rebuild from scratch.
        let messages = if let Some(mut cached) = self.cached_prefix.take() {
            let tail = self.agent.tool_dispatcher.to_provider_messages(
                &self.agent.history[self.agent.history.len().saturating_sub(1)..],
            );
            cached.extend(tail);
            cached
        } else {
            self.agent
                .tool_dispatcher
                .to_provider_messages(&self.agent.history)
        };
        *buf = messages;
        // Second-pass trim on the materialized provider messages (mirrors the
        // legacy `Agent::turn`, which trimmed both the typed history and the
        // built `ChatMessage` list).
        if let Some(context_window) = context_window_for_model(&self.effective_model) {
            super::super::token_budget::trim_chat_messages_to_budget(buf, context_window);
        }
        Ok(())
    }

    fn allow_empty_final(&self) -> bool {
        false
    }

    fn record_usage(&mut self, model: &str, usage: &UsageInfo) {
        self.agent.context.record_usage(usage);
        crate::openhuman::cost::record_provider_usage(model, usage);
        self.cumulative_input += usage.input_tokens;
        self.cumulative_output += usage.output_tokens;
        self.cumulative_cached += usage.cached_input_tokens;
        self.cumulative_charged += usage.charged_amount_usd;
        self.last_turn_usage = Some(transcript::TurnUsage {
            model: model.to_string(),
            usage: transcript::MessageUsage {
                input: usage.input_tokens,
                output: usage.output_tokens,
                cached_input: usage.cached_input_tokens,
                cost_usd: usage.charged_amount_usd,
            },
            ts: chrono::Utc::now().to_rfc3339(),
        });
    }

    fn on_assistant(
        &mut self,
        display_text: &str,
        _response_text: &str,
        reasoning_content: Option<&str>,
        native_tool_calls: &[ToolCall],
        parsed_calls: &[ParsedToolCall],
        iteration: usize,
        is_final: bool,
    ) {
        if is_final {
            let mut assistant_msg = ChatMessage::assistant(display_text.to_string());
            if let Some(rc) = reasoning_content {
                assistant_msg.extra_metadata = Some(serde_json::json!({ "reasoning_content": rc }));
            }
            self.agent
                .history
                .push(ConversationMessage::Chat(assistant_msg));
            self.agent.trim_history();
            self.did_push_final = true;
            return;
        }

        // Assistant turn with tool calls. Mirror `Agent::turn` exactly: push the
        // pre-tool narrative text (if any) as a standalone Chat message, then
        // the structured AssistantToolCalls, then the dispatcher-formatted
        // results buffered during the per-call loop.
        if !display_text.is_empty() {
            self.agent
                .history
                .push(ConversationMessage::Chat(ChatMessage::assistant(
                    display_text.to_string(),
                )));
        }
        let tool_calls = persisted_tool_calls(
            native_tool_calls,
            parsed_calls,
            &self.pending_results,
            iteration,
        );
        self.agent
            .history
            .push(ConversationMessage::AssistantToolCalls {
                text: if display_text.is_empty() {
                    None
                } else {
                    Some(display_text.to_string())
                },
                tool_calls,
                reasoning_content: reasoning_content
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string),
            });
        let results = std::mem::take(&mut self.pending_results);
        let formatted = self.agent.tool_dispatcher.format_results(&results);
        self.agent.history.push(formatted);
        self.agent.trim_history();
    }

    fn on_tool_result(
        &mut self,
        call_id: &str,
        tool_name: &str,
        result_text: &str,
        success: bool,
        _iteration: usize,
    ) {
        self.pending_results.push(ToolExecutionResult {
            name: tool_name.to_string(),
            output: result_text.to_string(),
            success,
            tool_call_id: Some(call_id.to_string()),
        });
    }

    fn after_iteration(&mut self, _buf: &[ChatMessage], _iteration: usize) {
        self.persist();
    }
}

/// Max-iteration checkpoint for `Agent::turn`: summarize the turn's tool digest
/// into a resumable checkpoint (streaming text deltas through the progress
/// sink), with a deterministic fallback.
pub(super) struct AgentCheckpoint {
    pub provider: Arc<dyn Provider>,
    pub dispatcher: Arc<dyn ToolDispatcher>,
    pub model: String,
    pub temperature: f64,
    pub on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    pub user_message: String,
    pub max_iterations: usize,
}

#[async_trait]
impl CheckpointStrategy for AgentCheckpoint {
    async fn on_max_iter(&self, digest: &str, max_iterations: usize) -> Result<CheckpointOutcome> {
        let deterministic = format!(
            "I reached the tool-call limit for this turn ({max_iterations} steps), so I paused here.\n\n\
             **Done so far:**\n{digest}\n\
             **Next steps:** I'll continue from here — just reply (e.g. \"continue\") and I'll pick up \
             where I left off."
        );
        let mut messages = vec![ChatMessage::user(format!(
            "You were working on this user request:\n{}\n\nHere are the tool calls you made this turn \
             and their results — compile your checkpoint from these:\n{}",
            self.user_message, digest
        ))];
        messages.push(ChatMessage::user(MAX_ITER_CHECKPOINT_INSTRUCTION));

        let checkpoint_iteration = (self.max_iterations + 1) as u32;
        // Stream the checkpoint prose as text deltas (tools disabled).
        let (delta_tx_opt, delta_forwarder) = if self.on_progress.is_some() {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(128);
            let progress_tx = self.on_progress.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let Some(ref sink) = progress_tx else {
                        continue;
                    };
                    if let ProviderDelta::TextDelta { delta } = event {
                        if sink
                            .send(AgentProgress::TextDelta {
                                delta,
                                iteration: checkpoint_iteration,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            });
            (Some(tx), Some(forwarder))
        } else {
            (None, None)
        };

        let result = self
            .provider
            .chat(
                ChatRequest {
                    messages: &messages,
                    tools: None,
                    stream: delta_tx_opt.as_ref(),
                },
                &self.model,
                self.temperature,
            )
            .await;
        drop(delta_tx_opt);
        if let Some(handle) = delta_forwarder {
            let _ = handle.await;
        }

        match result {
            Ok(resp) => {
                let usage = resp.usage.clone();
                // Strip any stray tool-call markup; keep only prose.
                let (text, calls) = self.dispatcher.parse_response(&resp);
                let checkpoint = if !text.trim().is_empty() {
                    text
                } else if calls.is_empty() {
                    resp.text.unwrap_or_default()
                } else {
                    String::new()
                };
                let text = if checkpoint.trim().is_empty() {
                    deterministic
                } else {
                    checkpoint
                };
                Ok(CheckpointOutcome { text, usage })
            }
            Err(e) => {
                log::warn!("[agent_loop] checkpoint summary call failed: {e:#}");
                Ok(CheckpointOutcome {
                    text: deterministic,
                    usage: None,
                })
            }
        }
    }
}
