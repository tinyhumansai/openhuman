//! The **sub-agent turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this module owns the sub-agent
//! folder's graph definition, its available tools, and its summarization step —
//! all thin over the shared tinyagents seam
//! ([`run_turn_via_tinyagents_shared`]).
//!
//! **Graph.** A single agent-loop turn driven by the tinyagents harness: the
//! model is called, requested tools run, and the loop repeats until the model
//! returns without further tool calls or the iteration budget is exhausted. The
//! canonical sub-agent turn path (the legacy `run_inner_loop` / `run_turn_engine`
//! are removed); `run_typed_mode` calls it unconditionally.
//!
//! **Available tools.** The sub-agent reuses the parent's harness tools plus the
//! per-spawn dynamic tools, advertised via [`SharedToolAdapter`] over the shared
//! `Arc<Vec<Box<dyn Tool>>>` tool sets (`[dynamic_tools, parent_tools]` — dynamic
//! first so a shadowing dynamic tool executes, matching advertisement), filtered
//! by `allowed_names`. `ask_user_clarification` is the early-exit tool.
//!
//! **Summarization.** When the sub-agent model's effective context window is
//! known, the shared seam installs the context-window summarization step
//! (`tinyagents::summarize`) ahead of the deterministic front-trim — see
//! [`run_subagent_via_graph`], which resolves the window before dispatch.
//!
//! It mirrors the original seams: child progress deltas (`Subagent*` events incl.
//! thinking), mid-flight steering, the `ask_user_clarification` early-exit pause,
//! and a graceful model-call-cap checkpoint summary
//! (`SubagentCheckpoint::on_max_iter`).

use std::collections::HashSet;
use std::sync::Arc;

use super::usage::AggregatedUsage;
use crate::openhuman::agent::harness::subagent_runner::types::SubagentRunError;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage, Provider};
use crate::openhuman::tinyagents::{run_turn_via_tinyagents_shared, SubagentScope};
use crate::openhuman::tools::{Tool, ToolSpec};

/// Drive a sub-agent turn on the tinyagents harness. Returns
/// `(text, model_calls, AggregatedUsage, early_exit_tool, hit_cap)` — `hit_cap`
/// is `true` when the run stopped at the model-call cap with work still pending
/// (the caller surfaces this as `SubagentRunStatus::Incomplete`, #4096).
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_subagent_via_graph(
    provider: Arc<dyn Provider>,
    model: &str,
    temperature: f64,
    history: &mut Vec<ChatMessage>,
    parent_tools: Arc<Vec<Box<dyn Tool>>>,
    dynamic_tools: Vec<Box<dyn Tool>>,
    specs: Vec<ToolSpec>,
    allowed_names: HashSet<String>,
    max_iterations: usize,
    run_queue: Option<Arc<crate::openhuman::agent::harness::run_queue::RunQueue>>,
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    agent_id: &str,
    task_id: &str,
    extended_policy: bool,
    worker_thread_id: Option<String>,
    workspace_dir: std::path::PathBuf,
    max_output_tokens: u32,
    model_vision: bool,
    // Transcript-persistence provenance: the resolved child transcript stem
    // (`{parent_chain}__{child_session_key}`) and the provider label (the parent
    // turn's event channel) so the child's raw transcript lands in `session_raw`
    // with the right stem + provider/model meta — parity with the removed
    // `SubagentObserver::persist_transcript`.
    transcript_stem: &str,
    provider_label: &str,
    // Progressive-disclosure handoff cache (integrations_agent with a resolved
    // toolkit); `Some` installs the `HandoffMiddleware` that stashes oversized
    // tool results and shares the cache with the `extract_from_result` tool.
    handoff_cache: Option<
        std::sync::Arc<crate::openhuman::agent::harness::subagent_runner::ResultHandoffCache>,
    >,
) -> Result<(String, usize, AggregatedUsage, Option<String>, bool), SubagentRunError> {
    tracing::info!(
        model,
        max_iterations,
        agent_id,
        task_id,
        model_vision,
        observed = on_progress.is_some(),
        "[subagent_runner:graph] routing sub-agent turn through tinyagents harness"
    );
    // `specs` is derived from the registry inside the runner; the tinyagents
    // adapters advertise each tool via its own `spec()`, so it's unused here.
    let _ = &specs;

    // Vision forwarding (parity with the legacy `run_inner_loop`): rehydrate
    // `[IMAGE:…]` placeholders in the sub-agent's history when either the
    // provider advertises vision or the sub-agent model is user-flagged as
    // vision-capable (BYOK/custom). The expanded copy is provider-only — the
    // persisted `history` written back below keeps the original markers.
    let dispatch_history = if (provider.supports_vision() || model_vision)
        && crate::openhuman::agent::multimodal::has_image_placeholders(history)
    {
        crate::openhuman::agent::multimodal::rehydrate_image_placeholders(history)
    } else {
        history.clone()
    };

    // Child-progress attribution: mirror this sub-agent's iterations / tool calls
    // / text + thinking deltas as `Subagent*` events scoped to (`agent_id`,
    // `task_id`) so the parent thread can nest them under the live subagent row.
    // Always set (not gated on `on_progress`): the scope also tells the shared
    // seam this is a sub-agent turn, so the unknown-tool recovery uses the
    // sub-agent wording. With no progress sink the scoped events simply have
    // nowhere to go, which is harmless.
    let subagent_scope = Some(SubagentScope {
        agent_id: agent_id.to_string(),
        task_id: task_id.to_string(),
        extended_policy,
    });

    // Keep a provider handle for the cap-hit summary call (the run consumes the
    // other clone).
    let summary_provider = provider.clone();

    // Resolve the sub-agent model's effective context window so the harness runs
    // the context-window summarization step (issue #4249) on sub-agent turns too.
    // A long-running / resumed sub-agent (worker threads, durable sessions) can
    // accumulate a transcript past its own window; summarize before each model
    // call rather than relying solely on the parent's one-time trim.
    let context_window = provider.effective_context_window(model).await;

    // A sub-agent turn runs *nested inside* the parent agent's turn (parent
    // harness → spawn_subagent tool → here), so the child's full
    // `run_turn_via_tinyagents_shared` future would otherwise sit on the parent's
    // poll stack. Heap-allocate it (as the legacy `run_inner_loop` did) so the
    // parent+child harness drives don't overflow the stack.
    // Capture native-tool support before `provider` is moved: the durable-history
    // append below serializes this turn's typed suffix with the matching dispatcher.
    let native_tools = provider.supports_native_tools();
    let mut outcome = Box::pin(run_turn_via_tinyagents_shared(
        provider,
        model,
        temperature,
        dispatch_history,
        // Dynamic (per-spawn) tools first so a dynamic tool that intentionally
        // shadows a parent-registry tool of the same name is the one that
        // *executes* — matching the advertisement order (`dedup_tool_specs_by_name`
        // lists dynamic specs before parent specs in `runner.rs`). The shared
        // adapter resolves a name by scanning the sets in order, so a
        // parent-first order would run the parent impl for a shadowed name.
        vec![Arc::new(dynamic_tools), parent_tools],
        allowed_names,
        max_iterations,
        // Parent's progress sink — child events ride it, scoped below.
        on_progress,
        subagent_scope,
        // Resolved above — drives the sub-agent context-window summarization step.
        context_window,
        // Mid-flight steering: forward queued steer messages into the run.
        run_queue,
        // Pause + checkpoint when the child asks the user a clarifying question.
        &["ask_user_clarification"],
        // Pause gracefully at the model-call cap so we can summarize a resumable
        // checkpoint (below) instead of erroring — legacy `on_max_iter` parity.
        true,
        // Bound the sub-agent's per-call output at its configured budget.
        Some(max_output_tokens),
        // Context middlewares: cache-align + default tool-result byte cap so a
        // sub-agent's (often large) tool outputs stay bounded in its transcript,
        // plus the progressive-disclosure handoff when a cache is attached.
        {
            let mut mw = crate::openhuman::tinyagents::TurnContextMiddleware::defaults();
            if let Some(cache) = handoff_cache {
                mw.handoff = Some(crate::openhuman::tinyagents::HandoffConfig {
                    cache,
                    agent_id: agent_id.to_string(),
                    task_id: task_id.to_string(),
                });
            }
            mw
        },
        // Sub-agents gate via their own SubagentToolSource policy path, not the
        // session `.tool_policy()`; no enforcement threaded here.
        None,
    ))
    .await
    .map_err(SubagentRunError::Provider)?;

    // Write the final conversation back so the caller can checkpoint / persist.
    // Keep the original (un-expanded) prior turns and append only this turn's typed
    // suffix, serialized with the matching dispatcher so a native tool round
    // persists as the `{content, tool_calls}` / `{tool_call_id, content}` envelope
    // (re-parsed by `convert::chat_message_to_message` next turn) instead of an
    // assistant with no `tool_calls` followed by an orphan `tool` row. Appending
    // the typed `outcome.conversation` (messages-since-last-user) also avoids
    // indexing a post-trim `outcome.history` with the pre-trim length, and the
    // durable `[IMAGE:…]` markers stay put since the prior user turns are untouched.
    use crate::openhuman::agent::dispatcher::ToolDispatcher;
    let suffix = if native_tools {
        crate::openhuman::agent::dispatcher::NativeToolDispatcher
            .to_provider_messages(&outcome.conversation)
    } else {
        crate::openhuman::agent::dispatcher::XmlToolDispatcher
            .to_provider_messages(&outcome.conversation)
    };
    history.extend(suffix);

    let mut usage = AggregatedUsage {
        input_tokens: outcome.input_tokens,
        output_tokens: outcome.output_tokens,
        // Carry the child's cached-prefix tokens + estimated cost (the turn
        // outcome now reports both) so sub-agent spend rolls into the parent
        // instead of being recorded as uncached and $0.
        cached_input_tokens: outcome.cached_input_tokens,
        charged_amount_usd: outcome.charged_amount_usd,
    };

    // Cap hit with work still pending: summarize the run-so-far into a resumable
    // checkpoint (the delegating agent continues from partial progress) rather
    // than surfacing an empty/partial answer — the legacy `SubagentCheckpoint`.
    if outcome.hit_cap {
        use super::super::super::engine::CheckpointStrategy;
        let digest = build_cap_digest(&outcome.conversation);
        let strategy = super::checkpoint::SubagentCheckpoint {
            provider: summary_provider.as_ref(),
            model: model.to_string(),
            temperature,
            agent_id: agent_id.to_string(),
            // The checkpoint summary call's output cap — the standard per-turn
            // budget (the value this field replaced when it was hardcoded).
            max_output_tokens: crate::openhuman::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS,
        };
        match strategy.on_max_iter(&digest, max_iterations).await {
            Ok(co) => {
                if let Some(u) = co.usage {
                    usage.input_tokens += u.input_tokens;
                    usage.output_tokens += u.output_tokens;
                }
                outcome.text = co.text;
            }
            Err(e) => return Err(SubagentRunError::Provider(e)),
        }
    }

    // Persist the sub-agent's raw transcript to `session_raw` (parity with the
    // removed `SubagentObserver::persist_transcript`). The graph runner replaced
    // the observer but only mirrored to the worker thread, so per-child
    // transcripts stopped being written — breaking downstream learning ingestion
    // (`learning/transcript_ingest`, which reads `session_raw/*.jsonl`).
    // On a cap-hit / early-exit, `outcome.text` is the checkpoint (or clarifying
    // question) that stands in for a final assistant turn — append it so the
    // persisted transcript reflects the actual final state, not the pre-checkpoint
    // history. `history` already carries this turn's typed suffix.
    let transcript_history;
    let history_for_transcript: &[ChatMessage] = if (outcome.hit_cap
        || outcome.early_exit_tool.is_some())
        && !outcome.text.trim().is_empty()
    {
        transcript_history = {
            let mut messages = history.clone();
            messages.push(ChatMessage::assistant(outcome.text.clone()));
            messages
        };
        &transcript_history
    } else {
        history.as_slice()
    };
    persist_subagent_transcript(
        &workspace_dir,
        transcript_stem,
        agent_id,
        task_id,
        provider_label,
        model,
        history_for_transcript,
        &usage,
        context_window.unwrap_or(0),
        // Match the dispatcher the history was actually serialized with (text-mode
        // integrations turns write XML), and the real iteration count.
        if native_tools { "native" } else { "xml" },
        outcome.model_calls as u32,
    );

    // Mirror this turn's conversation to the spawn's worker thread (when one is
    // attached), matching the legacy `SubagentObserver`: assistant intents +
    // final answer as `agent` messages, tool results as `user` messages. The
    // initial user prompt was already written when the worker thread was created.
    if let Some(thread_id) = worker_thread_id {
        mirror_worker_thread(
            &workspace_dir,
            &thread_id,
            agent_id,
            task_id,
            &outcome.conversation,
            // On a cap/early-exit, `outcome.text` is the checkpoint/question that
            // replaced (or stands in for) a final assistant turn.
            if outcome.hit_cap || outcome.early_exit_tool.is_some() {
                Some(outcome.text.as_str())
            } else {
                None
            },
        );
    }

    // On an early-exit (`ask_user_clarification`), `outcome.text` is the question
    // and the runner checkpoints + returns AwaitingUser. `None` = ran to a final
    // answer (or a cap-hit checkpoint summary).
    Ok((
        outcome.text,
        outcome.model_calls,
        usage,
        outcome.early_exit_tool,
        outcome.hit_cap,
    ))
}

/// Persist a sub-agent turn's raw transcript to `session_raw`, mirroring the
/// removed `SubagentObserver::persist_transcript`: `agent_type:"subagent"`, the
/// `task_id`, and the provider/model + usage carried on the last assistant
/// message so per-thread usage reads price the sub-agent at its own model.
#[allow(clippy::too_many_arguments)]
fn persist_subagent_transcript(
    workspace_dir: &std::path::Path,
    transcript_stem: &str,
    agent_id: &str,
    task_id: &str,
    provider_label: &str,
    model: &str,
    history: &[ChatMessage],
    usage: &AggregatedUsage,
    context_window: u64,
    dispatcher: &str,
    iteration: u32,
) {
    use crate::openhuman::agent::harness::session::transcript;

    let path = match transcript::resolve_keyed_transcript_path(workspace_dir, transcript_stem) {
        Ok(p) => p,
        Err(err) => {
            tracing::debug!(
                agent_id,
                error = %err,
                "[subagent_runner:graph] failed to resolve child transcript path"
            );
            return;
        }
    };
    let now = chrono::Utc::now().to_rfc3339();
    let turn_usage = transcript::TurnUsage {
        provider: provider_label.to_string(),
        model: model.to_string(),
        usage: transcript::MessageUsage {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cached_input: usage.cached_input_tokens,
            context_window,
            cost_usd: usage.charged_amount_usd,
        },
        ts: now.clone(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        iteration,
    };
    let meta = transcript::TranscriptMeta {
        agent_name: agent_id.to_string(),
        agent_id: Some(agent_id.to_string()),
        agent_type: Some("subagent".to_string()),
        dispatcher: dispatcher.into(),
        provider: Some(turn_usage.provider.clone()),
        model: Some(turn_usage.model.clone()),
        created: now.clone(),
        updated: now,
        turn_count: 1,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        charged_amount_usd: usage.charged_amount_usd,
        thread_id: crate::openhuman::inference::provider::thread_context::current_thread_id(),
        task_id: Some(task_id.to_string()),
    };
    if let Err(err) = transcript::write_transcript(&path, history, &meta, Some(&turn_usage)) {
        tracing::debug!(
            agent_id,
            error = %err,
            "[subagent_runner:graph] failed to write child transcript"
        );
    }
}

/// Mirror a sub-agent turn's structured conversation to its worker thread,
/// matching the legacy [`SubagentObserver`]: assistant turns (intents + final)
/// become `agent` messages, tool results become `user` messages. `extra_final`,
/// when set, is appended as a trailing `agent` message (the cap checkpoint or
/// clarifying question, which isn't a plain assistant turn in the transcript).
fn mirror_worker_thread(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    agent_id: &str,
    task_id: &str,
    conversation: &[ConversationMessage],
    extra_final: Option<&str>,
) {
    use crate::openhuman::memory_conversations::{
        append_message, ConversationMessage as StoredMessage,
    };

    let mut append = |content: String, sender: &str| {
        let message = StoredMessage {
            id: format!("{sender}:{}", uuid::Uuid::new_v4()),
            content,
            message_type: "text".to_string(),
            extra_metadata: serde_json::json!({
                "scope": "worker_thread",
                "agent_id": agent_id,
                "task_id": task_id,
            }),
            sender: sender.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(err) = append_message(workspace_dir.to_path_buf(), thread_id, message) {
            tracing::debug!(
                agent_id,
                thread_id,
                error = %err,
                "[subagent_runner:graph] failed to append worker-thread message"
            );
        }
    };

    for msg in conversation {
        match msg {
            ConversationMessage::AssistantToolCalls { text, .. } => {
                if let Some(t) = text.as_deref().filter(|t| !t.trim().is_empty()) {
                    append(t.to_string(), "agent");
                }
            }
            ConversationMessage::ToolResults(results) => {
                for r in results {
                    append(r.content.clone(), "user");
                }
            }
            ConversationMessage::Chat(c) if c.role == "assistant" => {
                if !c.content.trim().is_empty() {
                    append(c.content.clone(), "agent");
                }
            }
            _ => {}
        }
    }

    if let Some(text) = extra_final.filter(|t| !t.trim().is_empty()) {
        append(text.to_string(), "agent");
    }
}

/// Build the `tool → outcome` digest the cap-hit summary call summarizes, in the
/// legacy `- {name} [{ok|failed}]: {output}` format (engine `run_tool_digest`),
/// pairing each tool result back to its call by id. Tool success isn't carried
/// on the converted transcript, so results are reported optimistically as `ok`.
fn build_cap_digest(conversation: &[ConversationMessage]) -> String {
    use std::collections::HashMap;
    use std::fmt::Write as _;

    // call_id -> tool name, from this turn's assistant tool-call rounds.
    let mut names: HashMap<&str, &str> = HashMap::new();
    for msg in conversation {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = msg {
            for call in tool_calls {
                names.insert(call.id.as_str(), call.name.as_str());
            }
        }
    }

    let mut out = String::new();
    for msg in conversation {
        if let ConversationMessage::ToolResults(results) = msg {
            for r in results {
                let name = names
                    .get(r.tool_call_id.as_str())
                    .copied()
                    .unwrap_or("tool");
                let body = crate::openhuman::util::truncate_with_ellipsis(&r.content, 800);
                let _ = writeln!(out, "- {name} [ok]: {body}");
            }
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::provider::{ChatResponse, ToolCall};
    use crate::openhuman::tools::ToolResult;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoTool;
    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echo"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let m = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
            Ok(ToolResult::success(format!("echoed:{m}")))
        }
    }

    struct TwoStepProvider {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl Provider for TwoStepProvider {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            _r: crate::openhuman::inference::provider::ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(ChatResponse {
                    tool_calls: vec![ToolCall {
                        id: "1".to_string(),
                        name: "echo".to_string(),
                        arguments: r#"{"msg":"hi"}"#.to_string(),
                        extra_content: None,
                    }],
                    ..Default::default()
                })
            } else {
                Ok(ChatResponse {
                    text: Some("all done".to_string()),
                    ..Default::default()
                })
            }
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn subagent_runs_through_the_graph_engine_with_real_tools() {
        let provider = Arc::new(TwoStepProvider {
            calls: AtomicUsize::new(0),
        });
        let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(EchoTool)]);
        let mut allowed = HashSet::new();
        allowed.insert("echo".to_string());
        let mut history = vec![ChatMessage::user("please echo hi")];

        let (output, iterations, usage, early_exit, hit_cap) = run_subagent_via_graph(
            provider,
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            allowed,
            10,
            None,
            None,
            "researcher",
            "task-1",
            false,
            None,
            std::env::temp_dir(),
            1024,
            false,
            "root-session__real_tools",
            "mock-channel",
            None,
        )
        .await
        .expect("graph subagent runs");

        assert_eq!(output, "all done");
        assert_eq!(iterations, 2);
        assert!(early_exit.is_none());
        assert!(!hit_cap, "a clean finish should not report a cap hit");
        let _ = usage;
        // History was written back: user + assistant(tool) + tool result + assistant(final).
        assert!(history.len() >= 4);
        assert!(history.iter().any(|m| m.content.contains("echoed:hi")));
    }

    /// A provider that streams visible text + reasoning through the request's
    /// delta sender, exercising the child-progress bridge end to end.
    struct ThinkingStreamProvider;
    #[async_trait]
    impl Provider for ThinkingStreamProvider {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            r: crate::openhuman::inference::provider::ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            use crate::openhuman::inference::provider::ProviderDelta;
            if let Some(tx) = r.stream {
                let _ = tx
                    .send(ProviderDelta::ThinkingDelta {
                        delta: "let me think".into(),
                    })
                    .await;
                for chunk in ["Hel", "lo"] {
                    let _ = tx
                        .send(ProviderDelta::TextDelta {
                            delta: chunk.into(),
                        })
                        .await;
                }
            }
            Ok(ChatResponse {
                text: Some("Hello".to_string()),
                ..Default::default()
            })
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn child_text_and_thinking_deltas_are_scoped_to_the_subagent() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
        let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);
        let mut history = vec![ChatMessage::user("hi")];

        let (output, _iters, _usage, _early, _hit_cap) = run_subagent_via_graph(
            Arc::new(ThinkingStreamProvider),
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            HashSet::new(),
            4,
            None,
            Some(tx),
            "researcher",
            "task-7",
            false,
            None,
            std::env::temp_dir(),
            1024,
            false,
            "root-session__scoped_deltas",
            "mock-channel",
            None,
        )
        .await
        .expect("child-delta subagent runs");

        assert_eq!(output, "Hello");

        let mut text = String::new();
        let mut thinking = String::new();
        let mut saw_iter = false;
        while let Ok(p) = rx.try_recv() {
            match p {
                AgentProgress::SubagentTextDelta { delta, task_id, .. } => {
                    assert_eq!(task_id, "task-7");
                    text.push_str(&delta);
                }
                AgentProgress::SubagentThinkingDelta {
                    delta, agent_id, ..
                } => {
                    assert_eq!(agent_id, "researcher");
                    thinking.push_str(&delta);
                }
                AgentProgress::SubagentIterationStarted { task_id, .. } => {
                    assert_eq!(task_id, "task-7");
                    saw_iter = true;
                }
                // The parent-scoped variants must never appear on a child run.
                AgentProgress::TextDelta { .. }
                | AgentProgress::ThinkingDelta { .. }
                | AgentProgress::IterationStarted { .. } => {
                    panic!("child run emitted a parent-scoped progress event");
                }
                _ => {}
            }
        }
        assert!(saw_iter, "a SubagentIterationStarted should be emitted");
        assert!(
            text.contains("Hello"),
            "child text deltas should reassemble, got {text:?}"
        );
        assert!(
            thinking.contains("let me think"),
            "child thinking deltas should be forwarded, got {thinking:?}"
        );
    }

    /// A tool named like the early-exit tool that echoes its `question` arg.
    struct AskTool;
    #[async_trait]
    impl Tool for AskTool {
        fn name(&self) -> &str {
            "ask_user_clarification"
        }
        fn description(&self) -> &str {
            "ask the user a clarifying question"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"question": {"type": "string"}}})
        }
        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let q = args
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(ToolResult::success(q))
        }
    }

    /// A provider whose first turn calls `ask_user_clarification`; a second turn
    /// would answer, but the early-exit pause should stop the loop before it.
    struct AskThenAnswer {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl Provider for AskThenAnswer {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            _r: crate::openhuman::inference::provider::ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(ChatResponse {
                    tool_calls: vec![ToolCall {
                        id: "ask-1".to_string(),
                        name: "ask_user_clarification".to_string(),
                        arguments: r#"{"question":"which file?"}"#.to_string(),
                        extra_content: None,
                    }],
                    ..Default::default()
                })
            } else {
                Ok(ChatResponse {
                    text: Some("should not be reached".to_string()),
                    ..Default::default()
                })
            }
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn ask_user_clarification_pauses_and_surfaces_the_question() {
        let provider = Arc::new(AskThenAnswer {
            calls: AtomicUsize::new(0),
        });
        let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(AskTool)]);
        let mut allowed = HashSet::new();
        allowed.insert("ask_user_clarification".to_string());
        let mut history = vec![ChatMessage::user("help me")];

        let (output, iterations, _usage, early_exit, _hit_cap) = run_subagent_via_graph(
            provider.clone(),
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            allowed,
            10,
            None,
            None,
            "researcher",
            "task-9",
            false,
            None,
            std::env::temp_dir(),
            1024,
            false,
            "root-session__clarification",
            "mock-channel",
            None,
        )
        .await
        .expect("ask-clarification subagent runs");

        // The loop paused after the tool round: the early-exit tool is surfaced
        // and the question is the returned text — the second model turn never ran.
        assert_eq!(early_exit.as_deref(), Some("ask_user_clarification"));
        assert_eq!(output, "which file?");
        assert_eq!(
            iterations, 1,
            "the loop should pause before a second model call"
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    /// A tool that always succeeds, so the loop keeps going until the cap.
    struct NoopTool;
    #[async_trait]
    impl Tool for NoopTool {
        fn name(&self) -> &str {
            "noop"
        }
        fn description(&self) -> &str {
            "no-op"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _a: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
    }

    /// A provider that never finishes: every tool-enabled turn asks for `noop`.
    /// A request with no tools is the cap-hit summary call — it returns prose.
    struct LoopForeverProvider;
    #[async_trait]
    impl Provider for LoopForeverProvider {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            r: crate::openhuman::inference::provider::ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            if r.tools.is_some() {
                Ok(ChatResponse {
                    tool_calls: vec![ToolCall {
                        id: "n".to_string(),
                        name: "noop".to_string(),
                        arguments: "{}".to_string(),
                        extra_content: None,
                    }],
                    ..Default::default()
                })
            } else {
                // The summary call (tools=None): return a progress checkpoint.
                Ok(ChatResponse {
                    text: Some("progress: explored two leads".to_string()),
                    ..Default::default()
                })
            }
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn cap_hit_summarizes_a_resumable_checkpoint() {
        let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(NoopTool)]);
        let mut allowed = HashSet::new();
        allowed.insert("noop".to_string());
        let mut history = vec![ChatMessage::user("do a big task")];

        let (output, iterations, _usage, early_exit, hit_cap) = run_subagent_via_graph(
            Arc::new(LoopForeverProvider),
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            allowed,
            2,
            None,
            None,
            "researcher",
            "task-cap",
            false,
            None,
            std::env::temp_dir(),
            1024,
            false,
            "root-session__cap_hit",
            "mock-channel",
            None,
        )
        .await
        .expect("cap-hit subagent runs");

        // The loop paused at the 2-call budget and summarized instead of erroring.
        assert!(early_exit.is_none());
        assert!(hit_cap, "reaching the model-call cap should report hit_cap");
        assert_eq!(iterations, 2, "the loop should stop at the model-call cap");
        assert!(
            output.contains("progress: explored two leads"),
            "cap hit should return the summary checkpoint, got {output:?}"
        );
    }
}
