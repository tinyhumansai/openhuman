//! Turn lifecycle: running a single interaction, executing tools, and
//! wiring the context pipeline + sub-agent harness around them.
//!
//! This file owns the "hot path" methods on `Agent`:
//!
//! - [`Agent::turn`] — the big one. Orchestrates system-prompt build,
//!   memory-context injection, the provider loop, tool dispatch, and
//!   the context pipeline (tool-result budget → microcompact →
//!   autocompact signal → session-memory extraction trigger).
//! - [`Agent::execute_tool_call`] / [`Agent::execute_tools`] — the
//!   per-call runners, including the fork-cache `ForkContext` stash
//!   for `spawn_subagent { mode: "fork" }` invocations.
//! - [`Agent::build_parent_execution_context`] /
//!   [`Agent::build_fork_context`] — snapshot helpers for sub-agent
//!   task-locals.
//! - [`Agent::trim_history`], [`Agent::fetch_learned_context`],
//!   [`Agent::build_system_prompt`] — the small helpers `turn()` leans
//!   on every call.
//! - [`Agent::spawn_session_memory_extraction`] — the fire-and-forget
//!   background archivist fork.

use super::transcript;
use super::types::Agent;
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::dispatcher::{ParsedToolCall, ToolExecutionResult};
use crate::openhuman::agent::harness;
use crate::openhuman::agent::hooks::{self, ToolCallRecord, TurnContext};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::context::prompt::{
    LearnedContextData, PromptContext, PromptTool, RenderedPrompt,
};
use crate::openhuman::context::{ReductionOutcome, ARCHIVIST_EXTRACTION_PROMPT};
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::providers::{ChatMessage, ChatRequest, ConversationMessage};
use crate::openhuman::tools::Tool;
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::sync::Arc;

impl Agent {
    /// Executes a single interaction "turn" with the agent.
    ///
    /// This function is the primary driver of the agent's behavior. It manages the
    /// end-to-end lifecycle of a user request:
    ///
    /// 1. **Initialization**: Resumes from a session transcript if this is a new turn
    ///    to preserve KV-cache stability.
    /// 2. **Prompt Construction**: Builds the system prompt (only on the first turn)
    ///    incorporating learned context and tool instructions.
    /// 3. **Context Injection**: Enriches the user message with relevant memories
    ///    fetched via the [`MemoryLoader`].
    /// 4. **Execution Loop**: Enters a loop (up to `max_tool_iterations`) where it:
    ///    - Manages the context window (reduction/summarization).
    ///    - Calls the LLM provider.
    ///    - Parses and executes tool calls.
    ///    - Accumulates results into history.
    /// 5. **Synthesis**: Returns the final assistant response after all tools have
    ///    finished or the iteration budget is exhausted.
    /// 6. **Background Tasks**: Triggers episodic memory indexing and facts
    ///    extraction asynchronously.
    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
        let turn_started = std::time::Instant::now();
        self.emit_progress(AgentProgress::TurnStarted);
        log::info!("[agent] turn started — awaiting user message processing");
        log::info!(
            "[agent_loop] turn start message_chars={} history_len={} max_tool_iterations={}",
            user_message.chars().count(),
            self.history.len(),
            self.config.max_tool_iterations
        );
        // ── Session transcript resume ─────────────────────────────────
        // On a fresh session (empty history), look for a previous
        // transcript to pre-populate the exact provider messages for
        // KV cache prefix reuse.
        if self.history.is_empty() && self.cached_transcript_messages.is_none() {
            self.try_load_session_transcript();
        }

        if self.history.is_empty() {
            // Learned context is only baked into the system prompt on the
            // very first turn — once the history is non-empty we reuse the
            // stored prompt verbatim to preserve the KV-cache prefix the
            // inference backend has already tokenised. Fetching it later
            // would just burn memory-store reads on data we throw away.
            self.fetch_connected_integrations().await;
            let learned = self.fetch_learned_context().await;
            let rendered_prompt = self.build_system_prompt(learned)?;
            log::info!("[agent] system prompt built — initialising conversation history");
            log::info!(
                "[agent_loop] system prompt built chars={}",
                rendered_prompt.text.chars().count()
            );
            log::debug!("[agent_loop] system prompt body:\n{}", rendered_prompt.text);
            self.system_prompt_cache_boundary = rendered_prompt.cache_boundary;
            self.history
                .push(ConversationMessage::Chat(ChatMessage::system(
                    rendered_prompt.text,
                )));
        } else {
            // Deliberately do NOT rebuild the system prompt on subsequent
            // turns. The rendered prompt is the KV-cache prefix the inference
            // backend has already tokenised; replacing its bytes (even
            // cosmetically) forces the backend to re-prefill from scratch.
            //
            // Dynamic turn-to-turn context (memory recall, learned snippets)
            // rides on the user message via `memory_loader.load_context()`
            // — that's where the caller should inject anything that varies
            // between turns.
            log::trace!(
                "[agent_loop] system prompt reused (history_len={}) — KV cache prefix preserved",
                self.history.len()
            );
        }

        if self.auto_save {
            let _ = self
                .memory
                .store("user_msg", user_message, MemoryCategory::Conversation, None)
                .await;
        }

        log::info!("[agent] loading memory context for user message");
        let context = self
            .memory_loader
            .load_context(self.memory.as_ref(), user_message)
            .await
            .unwrap_or_default();

        let enriched = if context.is_empty() {
            log::info!("[agent] no memory context found — using raw user message");
            self.last_memory_context = None;
            user_message.to_string()
        } else {
            log::info!(
                "[agent] memory context loaded — enriching user message context_chars={}",
                context.chars().count()
            );
            self.last_memory_context = Some(context.clone());
            format!("{context}{user_message}")
        };

        self.history
            .push(ConversationMessage::Chat(ChatMessage::user(enriched)));

        // Pin the main agent to its configured model for the lifetime of
        // the session. Per-turn classification used to run here, but it
        // would flip `effective_model` mid-conversation (e.g. reasoning →
        // coding based on a single keyword). Every flip invalidates the
        // backend's KV cache namespace for this session, costing full
        // re-prefill on the very next turn. The main agent's job is to
        // decide *which sub-agent* to spawn — that routing lives in the
        // model prompt, not in the Rust-side classifier. Sub-agents pick
        // their own tier via `ModelSpec::Hint(...)` in their definition.
        let effective_model = self.model_name.clone();
        log::info!(
            "[agent_loop] model pinned model={} (per-turn classification disabled for KV cache stability)",
            effective_model
        );

        // Snapshot the parent's runtime once per turn so any
        // `spawn_subagent` invocation that fires inside this turn can
        // read it via the PARENT_CONTEXT task-local. We override the
        // model field with the post-classification effective model.
        let mut parent_context = self.build_parent_execution_context();
        parent_context.model_name = effective_model.clone();

        // Bump the session-memory turn counter. Used later by
        // `should_extract_session_memory` to decide whether to spawn a
        // background archivist fork at end-of-turn.
        self.context.tick_turn();

        // Collect tool call records across all iterations for post-turn hooks
        let mut all_tool_records: Vec<ToolCallRecord> = Vec::new();

        // Capture the last `Vec<ChatMessage>` sent to the provider so we
        // can persist it as a session transcript after the turn completes.
        let mut last_provider_messages: Option<Vec<ChatMessage>> = None;

        // Accumulate usage stats across iterations for the transcript.
        let mut cumulative_input_tokens: u64 = 0;
        let mut cumulative_output_tokens: u64 = 0;
        let mut cumulative_cached_input_tokens: u64 = 0;
        let mut cumulative_charged_usd: f64 = 0.0;

        let turn_body = async {
            for iteration in 0..self.config.max_tool_iterations {
                self.emit_progress(AgentProgress::IterationStarted {
                    iteration: (iteration + 1) as u32,
                    max_iterations: self.config.max_tool_iterations as u32,
                });
                log::info!(
                    "[agent_loop] iteration start i={} history_len={}",
                    iteration + 1,
                    self.history.len()
                );

                // Global context management: run the reduction chain
                // before every provider hit. Cheap when the guard is
                // healthy; executes the summarizer LLM call
                // internally when the pipeline asks for autocompaction
                // (summarization, microcompact, and the circuit
                // breaker all live inside [`ContextManager`]).
                let outcome = self.context.reduce_before_call(&mut self.history).await?;
                match &outcome {
                    ReductionOutcome::NoOp => {}
                    ReductionOutcome::Microcompacted {
                        envelopes_cleared,
                        entries_cleared,
                        bytes_freed,
                    } => {
                        log::info!(
                            "[agent_loop] context microcompact i={} envelopes={} entries={} bytes_freed={}",
                            iteration + 1,
                            envelopes_cleared,
                            entries_cleared,
                            bytes_freed
                        );
                    }
                    ReductionOutcome::Summarized(stats) => {
                        log::info!(
                            "[agent_loop] context autocompact summarized i={} messages_removed={} approx_tokens_freed={} summary_chars={}",
                            iteration + 1,
                            stats.messages_removed,
                            stats.approx_tokens_freed,
                            stats.summary_chars
                        );
                    }
                    ReductionOutcome::SummarizationFailed {
                        utilisation_pct,
                        reason,
                    } => {
                        log::warn!(
                            "[agent_loop] context summarizer failed i={} utilisation_pct={} reason={}",
                            iteration + 1,
                            utilisation_pct,
                            reason
                        );
                    }
                    ReductionOutcome::NotAttempted { utilisation_pct } => {
                        log::warn!(
                            "[agent_loop] context autocompact disabled in config i={} utilisation_pct={}",
                            iteration + 1,
                            utilisation_pct
                        );
                    }
                    ReductionOutcome::Exhausted {
                        utilisation_pct,
                        reason,
                    } => {
                        log::error!(
                            "[agent_loop] context exhausted i={} utilisation_pct={} reason={}",
                            iteration + 1,
                            utilisation_pct,
                            reason
                        );
                        return Err(anyhow::anyhow!(
                            "Context window exhausted ({utilisation_pct}% full): {reason}"
                        ));
                    }
                }

                // Use cached transcript messages on the first iteration of
                // a resumed session to provide a byte-identical prefix for
                // KV cache reuse. After `.take()` the cache is consumed;
                // subsequent iterations rebuild from history normally.
                let messages = if let Some(mut cached) = self.cached_transcript_messages.take() {
                    // Append only the delta (new user message) from the
                    // end of the current history.
                    let new_tail = self.tool_dispatcher.to_provider_messages(
                        &self.history[self.history.len().saturating_sub(1)..],
                    );
                    cached.extend(new_tail);
                    log::info!(
                        "[transcript] resumed from cached transcript prefix_len={} new_tail={}",
                        cached.len() - 1,
                        1
                    );
                    cached
                } else {
                    self.tool_dispatcher.to_provider_messages(&self.history)
                };
                last_provider_messages = Some(messages.clone());

                log::info!(
                    "[agent] iteration {}/{} — sending request to provider model={}",
                    iteration + 1,
                    self.config.max_tool_iterations,
                    effective_model
                );
                log::info!(
                    "[agent_loop] provider request i={} messages={} send_tool_specs={}",
                    iteration + 1,
                    messages.len(),
                    self.tool_dispatcher.should_send_tool_specs()
                );
                let provider_started = std::time::Instant::now();
                let response = match self
                    .provider
                    .chat(
                        ChatRequest {
                            messages: &messages,
                            tools: if self.tool_dispatcher.should_send_tool_specs() {
                                Some(self.visible_tool_specs.as_slice())
                            } else {
                                None
                            },
                            system_prompt_cache_boundary: self.system_prompt_cache_boundary,
                        },
                        &effective_model,
                        self.temperature,
                    )
                    .await
                {
                    Ok(resp) => {
                        log::info!(
                            "[agent_loop] provider response i={} elapsed_ms={} text_chars={} native_tool_calls={}",
                            iteration + 1,
                            provider_started.elapsed().as_millis(),
                            resp.text.as_ref().map_or(0, |t| t.chars().count()),
                            resp.tool_calls.len()
                        );
                        log::debug!("[agent_loop] provider response: {resp:?}");
                        // Feed the context manager (guard +
                        // session-memory token accounting). No-op when
                        // the provider doesn't return usage.
                        if let Some(ref usage) = resp.usage {
                            self.context.record_usage(usage);
                            cumulative_input_tokens += usage.input_tokens;
                            cumulative_output_tokens += usage.output_tokens;
                            cumulative_cached_input_tokens += usage.cached_input_tokens;
                            cumulative_charged_usd += usage.charged_amount_usd;
                        }
                        resp
                    }
                    Err(err) => return Err(err),
                };

                let (text, calls) = self.tool_dispatcher.parse_response(&response);
                let calls = Self::with_fallback_tool_call_ids(calls, iteration);
                log::info!(
                    "[agent] provider responded — parsed tool_calls={} text_chars={}",
                    calls.len(),
                    text.chars().count()
                );
                log::info!(
                    "[agent_loop] parsed response i={} parsed_text_chars={} parsed_tool_calls={}",
                    iteration + 1,
                    text.chars().count(),
                    calls.len()
                );
                if calls.is_empty() {
                    let final_text = if text.is_empty() {
                        response.text.unwrap_or_default()
                    } else {
                        text
                    };
                    log::info!(
                        "[agent] no tool calls — returning final response after {} iteration(s)",
                        iteration + 1
                    );
                    log::info!(
                        "[agent_loop] final response i={} final_chars={}",
                        iteration + 1,
                        final_text.chars().count()
                    );

                    self.emit_progress(AgentProgress::TurnCompleted {
                        iterations: (iteration + 1) as u32,
                    });

                    self.history
                        .push(ConversationMessage::Chat(ChatMessage::assistant(
                            final_text.clone(),
                        )));
                    self.trim_history();

                    if self.auto_save {
                        let summary = truncate_with_ellipsis(&final_text, 100);
                        let _ = self
                            .memory
                            .store("assistant_resp", &summary, MemoryCategory::Daily, None)
                            .await;
                    }

                    // Session-memory tool-call accounting. The actual
                    // background extraction spawn happens *outside*
                    // `turn_body` so the spawned task can take an owned
                    // parent context without fighting the borrow
                    // checker against `self`. We capture the decision
                    // here and surface it via the manager's session
                    // state — the epilogue (below) reads
                    // `should_extract_session_memory()`.
                    self.context.record_tool_calls(all_tool_records.len());

                    // Fire post-turn hooks (non-blocking)
                    if !self.post_turn_hooks.is_empty() {
                        let ctx = TurnContext {
                            user_message: user_message.to_string(),
                            assistant_response: final_text.clone(),
                            tool_calls: all_tool_records,
                            turn_duration_ms: turn_started.elapsed().as_millis() as u64,
                            session_id: None,
                            iteration_count: iteration + 1,
                        };
                        hooks::fire_hooks(&self.post_turn_hooks, ctx);
                    }

                    return Ok(final_text);
                }

                if !text.is_empty() {
                    log::info!(
                        "[agent_loop] assistant pre-tool text i={} chars={}",
                        iteration + 1,
                        text.chars().count()
                    );
                    // Push the assistant text into history; rendering is
                    // the caller's responsibility (the CLI loop walks
                    // `agent.history()` after each turn, sub-agents and
                    // library consumers get whatever they need through
                    // the returned value / history accessors).
                    self.history
                        .push(ConversationMessage::Chat(ChatMessage::assistant(
                            text.clone(),
                        )));
                }
                let tool_names: Vec<&str> = calls.iter().map(|call| call.name.as_str()).collect();
                log::info!(
                    "[agent] dispatching {} tool(s): {:?}",
                    calls.len(),
                    tool_names
                );
                log::info!(
                    "[agent_loop] executing tools i={} names={:?}",
                    iteration + 1,
                    tool_names
                );
                let persisted_tool_calls =
                    Self::persisted_tool_calls_for_history(&response, &calls, iteration);
                log::info!(
                    "[agent_loop] persisting assistant tool calls i={} persisted_tool_calls={} parsed_tool_calls={}",
                    iteration + 1,
                    persisted_tool_calls.len(),
                    calls.len()
                );
                self.history.push(ConversationMessage::AssistantToolCalls {
                    text: if text.is_empty() {
                        None
                    } else {
                        Some(text.clone())
                    },
                    tool_calls: persisted_tool_calls,
                });

                let (results, records) = self.execute_tools(&calls, iteration).await;
                all_tool_records.extend(records);
                log::info!(
                    "[agent_loop] tool results complete i={} result_count={}",
                    iteration + 1,
                    results.len()
                );
                for r in &results {
                    log::info!(
                        "[agent] tool response name={} success={} output_chars={}",
                        r.name,
                        r.success,
                        r.output.chars().count(),
                    );
                    log::debug!(
                        "[agent] tool response body name={}: {}",
                        r.name,
                        truncate_with_ellipsis(&r.output, 300)
                    );
                }
                log::info!(
                    "[agent] all tools complete for iteration {} — looping back to provider",
                    iteration + 1
                );
                let formatted = self.tool_dispatcher.format_results(&results);
                self.history.push(formatted);
                self.trim_history();
                log::info!(
                    "[agent_loop] iteration end i={} history_len={}",
                    iteration + 1,
                    self.history.len()
                );
            }

            log::warn!(
                "[agent] exceeded max tool iterations ({}) — aborting turn",
                self.config.max_tool_iterations
            );
            log::warn!(
                "[agent_loop] exceeded maximum tool iterations max={}",
                self.config.max_tool_iterations
            );
            anyhow::bail!(
                "Agent exceeded maximum tool iterations ({})",
                self.config.max_tool_iterations
            )
        }; // end of `turn_body` async block

        // Run the turn body inside the parent-execution-context scope so
        // that any `spawn_subagent` tool call fired during the loop can
        // read the parent's provider, tools, model, and workspace via
        // the PARENT_CONTEXT task-local.
        let result = harness::with_parent_context(parent_context, turn_body).await;

        // ── Session transcript persistence ────────────────────────────
        // Persist the exact provider messages so a future session can
        // resume with a byte-identical prefix for KV cache reuse.
        if result.is_ok() {
            if let Some(ref messages) = last_provider_messages {
                self.persist_session_transcript(
                    messages,
                    cumulative_input_tokens,
                    cumulative_output_tokens,
                    cumulative_cached_input_tokens,
                    cumulative_charged_usd,
                );
            }
        }

        // ── Session-memory extraction (stage 5) ───────────────────────
        //
        // If the pipeline's deltas have crossed all three thresholds
        // (token growth, tool calls, turn count), spawn a *background*
        // archivist sub-agent that will distil durable facts into the
        // workspace MEMORY.md file via the `update_memory_md` tool.
        //
        // The spawn is fire-and-forget: the main turn returns the
        // user-visible response immediately, and the archivist runs
        // asynchronously on the `agentic` tier. We optimistically mark
        // the extraction complete right away — if it actually fails,
        // we'll just retry on the next threshold window (a few turns
        // later), which is the right amount of retry behaviour for a
        // librarian task that's idempotent across reruns.
        if result.is_ok() && self.context.should_extract_session_memory() {
            self.spawn_session_memory_extraction();
        }

        result
    }

    // ─────────────────────────────────────────────────────────────────
    // Per-call tool execution
    // ─────────────────────────────────────────────────────────────────

    /// Executes a single tool call and returns the result and execution record.
    ///
    /// This method:
    /// 1. Emits telemetry events for the start of execution.
    /// 2. Handles the special `spawn_subagent` tool with `fork` context.
    /// 3. Validates tool visibility and availability.
    /// 4. Dispatches to the underlying tool implementation.
    /// 5. Applies per-result byte budgets to prevent context window bloat.
    /// 6. Sanitizes and records the outcome for post-turn hooks.
    pub(super) async fn execute_tool_call(
        &self,
        call: &ParsedToolCall,
        iteration: usize,
    ) -> (ToolExecutionResult, ToolCallRecord) {
        let started = std::time::Instant::now();
        publish_global(DomainEvent::ToolExecutionStarted {
            tool_name: call.name.clone(),
            session_id: self.event_session_id().to_string(),
        });
        self.emit_progress(AgentProgress::ToolCallStarted {
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
            iteration: (iteration + 1) as u32,
        });
        log::info!("[agent] executing tool: {}", call.name);
        log::info!("[agent_loop] tool start name={}", call.name);

        // Special-case `spawn_subagent { mode: "fork", … }`: stash a
        // ForkContext task-local so the sub-agent runner can replay the
        // parent's exact rendered prompt + tool schemas + message prefix
        // for backend prefix-cache reuse. The branch is taken before
        // executing the tool so the task-local is visible inside
        // `tool.execute(...)`.
        let fork_context_for_call = if call.name == "spawn_subagent"
            && call
                .arguments
                .get("mode")
                .and_then(|v| v.as_str())
                .map(|s| s.eq_ignore_ascii_case("fork"))
                .unwrap_or(false)
        {
            Some(self.build_fork_context(call))
        } else {
            None
        };

        let (raw_result, success) = if !self.visible_tool_names.is_empty()
            && !self.visible_tool_names.contains(&call.name)
        {
            log::warn!(
                "[agent] blocked tool call '{}' — not in visible tool set",
                call.name
            );
            (
                format!("Tool '{}' is not available to this agent", call.name),
                false,
            )
        } else if let Some(tool) = self.tools.iter().find(|t| t.name() == call.name) {
            let exec = tool.execute(call.arguments.clone());
            let outcome = if let Some(fork_ctx) = fork_context_for_call {
                harness::with_fork_context(fork_ctx, exec).await
            } else {
                exec.await
            };
            match outcome {
                Ok(r) => {
                    if !r.is_error {
                        (r.output(), true)
                    } else {
                        (format!("Error: {}", r.output()), false)
                    }
                }
                Err(e) => (format!("Error executing {}: {e}", call.name), false),
            }
        } else {
            (format!("Unknown tool: {}", call.name), false)
        };

        // Context pipeline stage 1: apply the per-result byte budget
        // *inline* before the result enters history. This is the only
        // cache-safe reduction stage — the truncated body has never
        // been sent to the backend so it creates no cache invalidation.
        // Source the budget from the context manager so it tracks the
        // resolved `context.tool_result_budget_bytes` (including any
        // env/config overrides) rather than the deprecated
        // `agent.tool_result_budget_bytes` field.
        let budget_bytes = self.context.tool_result_budget_bytes();
        let (result, budget_outcome) =
            crate::openhuman::context::apply_tool_result_budget(raw_result, budget_bytes);
        if budget_outcome.truncated {
            log::info!(
                "[agent_loop] tool_result_budget applied name={} original_bytes={} final_bytes={} dropped_bytes={}",
                call.name,
                budget_outcome.original_bytes,
                budget_outcome.final_bytes,
                budget_outcome.original_bytes - budget_outcome.final_bytes
            );
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        publish_global(DomainEvent::ToolExecutionCompleted {
            tool_name: call.name.clone(),
            session_id: self.event_session_id().to_string(),
            success,
            elapsed_ms,
        });
        self.emit_progress(AgentProgress::ToolCallCompleted {
            tool_name: call.name.clone(),
            success,
            output_chars: result.chars().count(),
            elapsed_ms,
            iteration: (iteration + 1) as u32,
        });
        log::info!(
            "[agent] tool completed: {} success={} elapsed_ms={}",
            call.name,
            success,
            elapsed_ms
        );
        log::debug!(
            "[agent] tool output for {}: {}",
            call.name,
            truncate_with_ellipsis(&result, 500)
        );
        log::info!(
            "[agent_loop] tool finish name={} elapsed_ms={} output_chars={} success={}",
            call.name,
            elapsed_ms,
            result.chars().count(),
            success
        );

        let output_summary = hooks::sanitize_tool_output(&result, &call.name, success);

        let record = ToolCallRecord {
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            success,
            output_summary,
            duration_ms: elapsed_ms,
        };

        let exec_result = ToolExecutionResult {
            name: call.name.clone(),
            output: result,
            success,
            tool_call_id: call.tool_call_id.clone(),
        };

        (exec_result, record)
    }

    /// Executes multiple tool calls in sequence.
    ///
    /// Collects results and execution records for all requested tools in a single batch.
    pub(super) async fn execute_tools(
        &self,
        calls: &[ParsedToolCall],
        iteration: usize,
    ) -> (Vec<ToolExecutionResult>, Vec<ToolCallRecord>) {
        let mut results = Vec::with_capacity(calls.len());
        let mut records = Vec::with_capacity(calls.len());
        for call in calls {
            let (exec_result, record) = self.execute_tool_call(call, iteration).await;
            results.push(exec_result);
            records.push(record);
        }
        (results, records)
    }

    // ─────────────────────────────────────────────────────────────────
    // Sub-agent context snapshots
    // ─────────────────────────────────────────────────────────────────

    /// Snapshot the parent's runtime so spawned sub-agents can read
    /// it via the [`harness::PARENT_CONTEXT`] task-local.
    pub(super) fn build_parent_execution_context(&self) -> harness::ParentExecutionContext {
        harness::ParentExecutionContext {
            provider: Arc::clone(&self.provider),
            all_tools: Arc::clone(&self.tools),
            all_tool_specs: Arc::clone(&self.tool_specs),
            model_name: self.model_name.clone(),
            temperature: self.temperature,
            workspace_dir: self.workspace_dir.clone(),
            memory: Arc::clone(&self.memory),
            agent_config: self.config.clone(),
            skills: Arc::new(self.skills.clone()),
            memory_context: self.last_memory_context.clone(),
            session_id: self.event_session_id().to_string(),
            channel: self.event_channel().to_string(),
        }
    }

    /// Build a [`harness::ForkContext`] capturing the parent's
    /// rendered system prompt + tool schemas + message prefix at the
    /// moment a `spawn_subagent { mode: "fork", … }` call fires.
    ///
    /// The system prompt is pulled from `history[0]` (the agent always
    /// stores its rendered system prompt as the first message). The
    /// message prefix is the entire current history rendered through
    /// the dispatcher — the *same* sequence the parent's next call
    /// would send, except the new fork directive replaces the parent's
    /// next continuation.
    pub(super) fn build_fork_context(&self, call: &ParsedToolCall) -> harness::ForkContext {
        let messages = self.tool_dispatcher.to_provider_messages(&self.history);
        let system_prompt: String = messages
            .first()
            .filter(|m| m.role == "system")
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let fork_task_prompt = call
            .arguments
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        harness::ForkContext {
            system_prompt: Arc::new(system_prompt),
            tool_specs: Arc::clone(&self.visible_tool_specs),
            message_prefix: Arc::new(messages),
            cache_boundary: self.system_prompt_cache_boundary,
            fork_task_prompt,
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // History & prompt helpers
    // ─────────────────────────────────────────────────────────────────

    /// Emit a progress event (fire-and-forget) if the sender is set.
    fn emit_progress(&self, event: AgentProgress) {
        if let Some(ref tx) = self.on_progress {
            let _ = tx.try_send(event);
        }
    }

    /// Truncates the conversation history to the configured maximum message count.
    ///
    /// System messages are always preserved. Older non-system messages are
    /// dropped first.
    pub(super) fn trim_history(&mut self) {
        let max = self.config.max_history_messages;
        if self.history.len() <= max {
            return;
        }

        let mut system_messages = Vec::new();
        let mut other_messages = Vec::new();

        for msg in self.history.drain(..) {
            match &msg {
                ConversationMessage::Chat(chat) if chat.role == "system" => {
                    system_messages.push(msg);
                }
                _ => other_messages.push(msg),
            }
        }

        if other_messages.len() > max {
            let drop_count = other_messages.len() - max;
            other_messages.drain(0..drop_count);
        }

        self.history = system_messages;
        self.history.extend(other_messages);
    }

    /// Pre-fetches learned context data from memory (observations, patterns, user profile).
    ///
    /// This is an async, non-blocking operation that populates the context
    /// for the system prompt.
    pub(super) async fn fetch_learned_context(&self) -> LearnedContextData {
        if !self.learning_enabled {
            return LearnedContextData::default();
        }

        let obs_entries = self
            .memory
            .list(
                Some(&MemoryCategory::Custom("learning_observations".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let pat_entries = self
            .memory
            .list(
                Some(&MemoryCategory::Custom("learning_patterns".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let profile_entries = self
            .memory
            .list(Some(&MemoryCategory::Custom("user_profile".into())), None)
            .await
            .unwrap_or_default();

        // Pull every namespace's root-level summary from the tree
        // summarizer. This is the densest user memory we can hand the
        // orchestrator: each root holds up to 20 000 tokens of distilled
        // long-term context. Done synchronously here because the calls
        // are filesystem reads, not provider/network round-trips, and
        // happen exactly once per session (only on the first turn).
        let tree_root_summaries = collect_tree_root_summaries(&self.workspace_dir);

        LearnedContextData {
            observations: obs_entries
                .iter()
                .rev()
                .take(5)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            patterns: pat_entries
                .iter()
                .take(3)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            user_profile: profile_entries
                .iter()
                .take(20)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            tree_root_summaries,
        }
    }

    /// Fetches the user's active Composio connections and populates
    /// `self.connected_integrations` so the system prompt can surface them.
    ///
    /// Best-effort: failures are logged and silently ignored (the prompt
    /// just won't show an integrations section).
    pub(super) async fn fetch_connected_integrations(&mut self) {
        use crate::openhuman::composio::{build_composio_client, providers::toolkit_description};
        use crate::openhuman::config::Config;
        use crate::openhuman::context::prompt::ConnectedIntegration;

        let config = match Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                log::debug!(
                    "[agent] skipping connected integrations fetch: config load failed: {e}"
                );
                return;
            }
        };

        let Some(client) = build_composio_client(&config) else {
            log::debug!(
                "[agent] skipping connected integrations fetch: no composio client (not signed in?)"
            );
            return;
        };

        match client.list_connections().await {
            Ok(resp) => {
                let integrations: Vec<ConnectedIntegration> = resp
                    .connections
                    .iter()
                    .filter(|c| c.status == "ACTIVE" || c.status == "CONNECTED")
                    .map(|c| ConnectedIntegration {
                        toolkit: c.toolkit.clone(),
                        description: toolkit_description(&c.toolkit).to_string(),
                    })
                    .collect();
                log::info!(
                    "[agent] fetched {} connected integrations for prompt",
                    integrations.len()
                );
                for ci in &integrations {
                    log::debug!("[agent] connected integration: {} — {}", ci.toolkit, ci.description);
                }
                self.connected_integrations = integrations;
            }
            Err(e) => {
                log::warn!("[agent] failed to fetch connected integrations: {e}");
            }
        }
    }

    /// Builds the system prompt for the current turn, including tool
    /// instructions and learned context.
    pub(super) fn build_system_prompt(
        &self,
        learned: LearnedContextData,
    ) -> Result<RenderedPrompt> {
        let tools_slice: &[Box<dyn Tool>] = self.tools.as_slice();
        let instructions = self.tool_dispatcher.prompt_instructions(tools_slice);
        // Adapt the owned Box<dyn Tool> slice into the shared PromptTool
        // shape that every prompt-building call-site uses. Temporary vec
        // borrows from `tools_slice` and lives for the duration of the
        // prompt build.
        let prompt_tools = PromptTool::from_tools(tools_slice);
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            tools: &prompt_tools,
            skills: &self.skills,
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &self.visible_tool_names,
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            connected_integrations: &self.connected_integrations,
        };
        // Route through the global context manager so every
        // prompt-building call-site — main agent, sub-agent runner,
        // channel runtimes — shares one builder configuration while
        // still preserving cache-boundary metadata for provider calls.
        self.context.build_system_prompt_with_cache_metadata(&ctx)
    }

    // ─────────────────────────────────────────────────────────────────
    // Session transcript helpers
    // ─────────────────────────────────────────────────────────────────

    /// Try to load a previous session transcript for KV cache resume.
    ///
    /// Best-effort: failures are logged and silently ignored.
    pub(super) fn try_load_session_transcript(&mut self) {
        match transcript::find_latest_transcript(&self.workspace_dir, &self.agent_definition_name) {
            Some(path) => {
                log::info!(
                    "[transcript] found previous transcript path={}",
                    path.display()
                );
                match transcript::read_transcript(&path) {
                    Ok(session) => {
                        if session.messages.is_empty() {
                            log::debug!(
                                "[transcript] previous transcript is empty — skipping resume"
                            );
                            return;
                        }
                        // Restore the cache boundary from the transcript
                        // metadata so the provider request carries the
                        // same offset as the original session.
                        self.system_prompt_cache_boundary = session.meta.cache_boundary;
                        log::info!(
                            "[transcript] loaded {} messages for resume (cache_boundary={:?})",
                            session.messages.len(),
                            session.meta.cache_boundary
                        );
                        self.cached_transcript_messages = Some(session.messages);
                    }
                    Err(err) => {
                        log::warn!(
                            "[transcript] failed to parse previous transcript {}: {err}",
                            path.display()
                        );
                    }
                }
            }
            None => {
                log::debug!(
                    "[transcript] no previous transcript found for agent={}",
                    self.agent_definition_name
                );
            }
        }
    }

    /// Persist the exact provider messages as a session transcript.
    ///
    /// Best-effort: failures are logged and silently ignored. The JSONL
    /// conversation store remains the authoritative persistence layer;
    /// session transcripts are an optimization for KV cache stability.
    pub(super) fn persist_session_transcript(
        &mut self,
        messages: &[ChatMessage],
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
        charged_amount_usd: f64,
    ) {
        // Resolve the transcript path on first write.
        if self.session_transcript_path.is_none() {
            match transcript::resolve_new_transcript_path(
                &self.workspace_dir,
                &self.agent_definition_name,
            ) {
                Ok(path) => {
                    log::info!(
                        "[transcript] new session transcript path={}",
                        path.display()
                    );
                    self.session_transcript_path = Some(path);
                }
                Err(err) => {
                    log::warn!("[transcript] failed to resolve transcript path: {err}");
                    return;
                }
            }
        }

        let path = self.session_transcript_path.as_ref().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        let meta = transcript::TranscriptMeta {
            agent_name: self.agent_definition_name.clone(),
            dispatcher: if self.tool_dispatcher.should_send_tool_specs() {
                "native".into()
            } else {
                "xml".into()
            },
            cache_boundary: self.system_prompt_cache_boundary,
            created: now.clone(),
            updated: now,
            turn_count: self.context.stats().session_memory_current_turn as usize,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            charged_amount_usd,
        };

        if let Err(err) = transcript::write_transcript(path, messages, &meta) {
            log::warn!(
                "[transcript] failed to write transcript {}: {err}",
                path.display()
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // Session-memory extraction (stage 5 of the context pipeline)
    // ─────────────────────────────────────────────────────────────────

    /// Spawn a background archivist sub-agent to extract durable facts
    /// from the recent conversation into `MEMORY.md`. Fire-and-forget.
    ///
    /// Gated by [`context_pipeline::SessionMemoryState::should_extract`]
    /// — see its docs for the threshold invariants. Safe to call from
    /// inside `turn()` after the turn body has settled.
    pub(super) fn spawn_session_memory_extraction(&mut self) {
        let Some(registry) = harness::AgentDefinitionRegistry::global() else {
            log::debug!("[session_memory] registry not initialised — skipping extraction spawn");
            return;
        };
        let Some(definition) = registry.get("archivist").cloned() else {
            log::debug!(
                "[session_memory] archivist definition not found — skipping extraction spawn"
            );
            return;
        };

        // Build a dedicated ParentExecutionContext for the background
        // task. The in-progress turn's context has already been
        // consumed by the `with_parent_context` scope above, so this is
        // a fresh snapshot.
        let parent_ctx = self.build_parent_execution_context();
        let extraction_prompt = ARCHIVIST_EXTRACTION_PROMPT.to_string();

        // Flip the extraction state to "in-progress" so future
        // should_extract checks return false until the archivist
        // finishes. We then hand a shared handle to the spawned task
        // so it can mark the extraction complete (resets deltas) on
        // success, or failed (keeps deltas intact for retry) on error.
        // This replaces the old optimistic `mark_complete` that
        // silently dropped the retry window when extractions failed.
        let stats_snapshot = self.context.stats();
        self.context.mark_session_memory_started();
        let sm_handle = self.context.session_memory_handle();

        log::info!(
            "[session_memory] spawning background archivist extraction (turn={}, tokens={})",
            stats_snapshot.session_memory_current_turn,
            stats_snapshot.session_memory_total_tokens
        );

        tokio::spawn(async move {
            let options = harness::SubagentRunOptions::default();
            let fut = harness::run_subagent(&definition, &extraction_prompt, options);
            let result = harness::with_parent_context(parent_ctx, fut).await;
            match result {
                Ok(outcome) => {
                    tracing::info!(
                        agent_id = %outcome.agent_id,
                        task_id = %outcome.task_id,
                        iterations = outcome.iterations,
                        output_chars = outcome.output.chars().count(),
                        "[session_memory] archivist extraction completed"
                    );
                    if let Ok(mut sm) = sm_handle.lock() {
                        sm.mark_extraction_complete();
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "[session_memory] archivist extraction failed — will retry after next threshold crossing"
                    );
                    // Leave the deltas intact so the next threshold
                    // crossing schedules another attempt. Clearing
                    // `extraction_in_progress` lets the retry
                    // actually fire.
                    if let Ok(mut sm) = sm_handle.lock() {
                        sm.mark_extraction_failed();
                    }
                }
            }
        });
    }
}

/// Wrapper around
/// [`crate::openhuman::tree_summarizer::store::collect_root_summaries_with_caps`]
/// that pins the per-namespace and total caps to the constants exposed
/// from `context::prompt`. The store helper does the actual work — this
/// indirection just keeps the call site readable and the caps in one
/// place where the prompt section is defined.
fn collect_tree_root_summaries(workspace_dir: &std::path::Path) -> Vec<(String, String)> {
    use crate::openhuman::context::prompt::{
        USER_MEMORY_PER_NAMESPACE_MAX_CHARS, USER_MEMORY_TOTAL_MAX_CHARS,
    };
    crate::openhuman::tree_summarizer::store::collect_root_summaries_with_caps(
        workspace_dir,
        USER_MEMORY_PER_NAMESPACE_MAX_CHARS,
        USER_MEMORY_TOTAL_MAX_CHARS,
    )
}

/// Sanitize a learned memory entry before injecting into the system prompt.
/// Strips raw data, limits length, and removes potential secrets.
fn sanitize_learned_entry(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Truncate to a safe length
    let max_len = 200;
    let sanitized: String = trimmed.chars().take(max_len).collect();
    // Strip anything that looks like a secret/token
    if sanitized.contains("Bearer ")
        || sanitized.contains("sk-")
        || sanitized.contains("ghp_")
        || sanitized.contains("-----BEGIN")
    {
        return "[redacted: potential secret]".to_string();
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_bus::{global, init_global, DomainEvent};
    use crate::openhuman::agent::dispatcher::XmlToolDispatcher;
    use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
    use crate::openhuman::agent::memory_loader::MemoryLoader;
    use crate::openhuman::memory::Memory;
    use crate::openhuman::providers::{ChatRequest, ChatResponse, Provider};
    use crate::openhuman::tools::Tool;
    use crate::openhuman::tools::ToolResult;
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::sync::Arc;
    use tokio::sync::Mutex as AsyncMutex;
    use tokio::sync::Notify;
    use tokio::time::{sleep, timeout, Duration};

    struct DummyProvider;

    #[async_trait]
    impl Provider for DummyProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> Result<String> {
            Ok("unused".into())
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> Result<ChatResponse> {
            Ok(ChatResponse {
                text: Some("unused".into()),
                tool_calls: vec![],
                usage: None,
            })
        }
    }

    struct SequenceProvider {
        responses: AsyncMutex<Vec<anyhow::Result<ChatResponse>>>,
        requests: AsyncMutex<Vec<Vec<ChatMessage>>>,
    }

    #[async_trait]
    impl Provider for SequenceProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> Result<String> {
            Ok("unused".into())
        }

        async fn chat(
            &self,
            request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> Result<ChatResponse> {
            self.requests.lock().await.push(request.messages.to_vec());
            self.responses.lock().await.remove(0)
        }
    }

    struct FixedMemoryLoader {
        context: String,
    }

    #[async_trait]
    impl MemoryLoader for FixedMemoryLoader {
        async fn load_context(
            &self,
            _memory: &dyn Memory,
            _user_message: &str,
        ) -> anyhow::Result<String> {
            Ok(self.context.clone())
        }
    }

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
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::success("echo-output"))
        }
    }

    struct LongTool;

    #[async_trait]
    impl Tool for LongTool {
        fn name(&self) -> &str {
            "long"
        }

        fn description(&self) -> &str {
            "long"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::success("x".repeat(800)))
        }
    }

    struct RecordingHook {
        calls: Arc<AsyncMutex<Vec<TurnContext>>>,
        notify: Arc<Notify>,
    }

    #[async_trait]
    impl PostTurnHook for RecordingHook {
        fn name(&self) -> &str {
            "recording"
        }

        async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
            self.calls.lock().await.push(ctx.clone());
            self.notify.notify_waiters();
            Ok(())
        }
    }

    fn make_agent(visible_tool_names: Option<HashSet<String>>) -> Agent {
        let workspace = tempfile::TempDir::new().expect("temp workspace");
        let workspace_path = workspace.path().to_path_buf();
        std::mem::forget(workspace);
        let memory_cfg = crate::openhuman::config::MemoryConfig {
            backend: "none".into(),
            ..crate::openhuman::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
        );

        let mut builder = Agent::builder()
            .provider(Box::new(DummyProvider))
            .tools(vec![Box::new(EchoTool)])
            .memory(mem)
            .tool_dispatcher(Box::new(XmlToolDispatcher))
            .workspace_dir(workspace_path)
            .event_context("turn-test-session", "turn-test-channel")
            .config(crate::openhuman::config::AgentConfig {
                max_history_messages: 3,
                ..crate::openhuman::config::AgentConfig::default()
            });

        if let Some(names) = visible_tool_names {
            builder = builder.visible_tool_names(names);
        }

        builder.build().unwrap()
    }

    fn make_agent_with_builder(
        provider: Arc<dyn Provider>,
        tools: Vec<Box<dyn Tool>>,
        memory_loader: Box<dyn MemoryLoader>,
        post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
        config: crate::openhuman::config::AgentConfig,
        context_config: crate::openhuman::config::ContextConfig,
    ) -> Agent {
        let workspace = tempfile::TempDir::new().expect("temp workspace");
        let workspace_path = workspace.path().to_path_buf();
        std::mem::forget(workspace);
        let memory_cfg = crate::openhuman::config::MemoryConfig {
            backend: "none".into(),
            ..crate::openhuman::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::openhuman::memory::create_memory(&memory_cfg, &workspace_path, None).unwrap(),
        );

        Agent::builder()
            .provider_arc(provider)
            .tools(tools)
            .memory(mem)
            .memory_loader(memory_loader)
            .tool_dispatcher(Box::new(XmlToolDispatcher))
            .post_turn_hooks(post_turn_hooks)
            .config(config)
            .context_config(context_config)
            .workspace_dir(workspace_path)
            .auto_save(true)
            .event_context("turn-test-session", "turn-test-channel")
            .build()
            .unwrap()
    }

    #[test]
    fn trim_history_preserves_system_and_keeps_latest_non_system_entries() {
        let mut agent = make_agent(None);
        agent.history = vec![
            ConversationMessage::Chat(ChatMessage::system("sys")),
            ConversationMessage::Chat(ChatMessage::user("u1")),
            ConversationMessage::Chat(ChatMessage::assistant("a1")),
            ConversationMessage::Chat(ChatMessage::user("u2")),
            ConversationMessage::Chat(ChatMessage::assistant("a2")),
        ];

        agent.trim_history();

        assert_eq!(agent.history.len(), 4);
        assert!(
            matches!(&agent.history[0], ConversationMessage::Chat(msg) if msg.role == "system")
        );
        assert!(agent
            .history
            .iter()
            .all(|msg| !matches!(msg, ConversationMessage::Chat(chat) if chat.content == "u1")));
        assert!(agent
            .history
            .iter()
            .any(|msg| matches!(msg, ConversationMessage::Chat(chat) if chat.content == "a2")));
    }

    #[test]
    fn build_fork_context_uses_visible_specs_and_prompt_argument() {
        let mut visible = HashSet::new();
        visible.insert("echo".to_string());
        let agent = make_agent(Some(visible));
        let call = ParsedToolCall {
            name: "spawn_subagent".into(),
            arguments: serde_json::json!({ "prompt": "fork task" }),
            tool_call_id: None,
        };

        let fork = agent.build_fork_context(&call);
        assert_eq!(fork.fork_task_prompt, "fork task");
        assert_eq!(fork.tool_specs.len(), 1);
        assert_eq!(fork.tool_specs[0].name, "echo");
        assert_eq!(fork.message_prefix.len(), 0);
    }

    #[test]
    fn build_parent_context_and_sanitize_helpers_cover_snapshot_paths() {
        let mut agent = make_agent(None);
        agent.last_memory_context = Some("remember this".into());
        agent.skills = vec![crate::openhuman::skills::Skill {
            name: "demo".into(),
            ..Default::default()
        }];

        let parent = agent.build_parent_execution_context();
        assert_eq!(parent.model_name, agent.model_name);
        assert_eq!(parent.temperature, agent.temperature);
        assert_eq!(parent.memory_context.as_deref(), Some("remember this"));
        assert_eq!(parent.session_id, "turn-test-session");
        assert_eq!(parent.channel, "turn-test-channel");
        assert_eq!(parent.skills.len(), 1);

        assert_eq!(sanitize_learned_entry("   "), "");
        assert_eq!(
            sanitize_learned_entry("Bearer abcdef"),
            "[redacted: potential secret]"
        );
        let long = "x".repeat(500);
        assert_eq!(sanitize_learned_entry(&long).chars().count(), 200);
        assert!(collect_tree_root_summaries(agent.workspace_dir()).is_empty());
    }

    #[tokio::test]
    async fn transcript_roundtrip_work() {
        let mut agent = make_agent(None);

        let messages = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("hello"),
            ChatMessage::assistant("done"),
        ];
        agent.system_prompt_cache_boundary = Some(12);
        agent.persist_session_transcript(&messages, 10, 5, 3, 0.25);
        assert!(agent.session_transcript_path.is_some());

        let loaded = transcript::read_transcript(agent.session_transcript_path.as_ref().unwrap())
            .expect("transcript should be readable");
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.meta.cache_boundary, Some(12));
        assert_eq!(loaded.meta.input_tokens, 10);

        let mut resumed = make_agent(None);
        resumed.workspace_dir = agent.workspace_dir.clone();
        resumed.agent_definition_name = agent.agent_definition_name.clone();
        resumed.try_load_session_transcript();
        assert_eq!(resumed.system_prompt_cache_boundary, Some(12));
        assert_eq!(
            resumed.cached_transcript_messages.as_ref().map(|m| m.len()),
            Some(3)
        );
    }

    #[tokio::test]
    async fn execute_tool_call_blocks_invisible_tool_and_emits_events() {
        let _ = init_global(64);
        let events = Arc::new(AsyncMutex::new(Vec::<DomainEvent>::new()));
        let events_handler = Arc::clone(&events);
        let _handle = global().unwrap().on("turn-events-test", move |event| {
            let events = Arc::clone(&events_handler);
            let cloned = event.clone();
            Box::pin(async move {
                events.lock().await.push(cloned);
            })
        });

        let mut visible = HashSet::new();
        visible.insert("other".to_string());
        let agent = make_agent(Some(visible));
        let call = ParsedToolCall {
            name: "echo".into(),
            arguments: serde_json::json!({}),
            tool_call_id: Some("tc-1".into()),
        };

        let (result, record) = agent.execute_tool_call(&call, 0).await;
        assert!(!result.success);
        assert!(result.output.contains("not available to this agent"));
        assert_eq!(record.name, "echo");
        assert!(!record.success);

        sleep(Duration::from_millis(20)).await;
        let captured = events.lock().await;
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::ToolExecutionStarted { tool_name, session_id }
                if tool_name == "echo" && session_id == "turn-test-session"
        )));
        assert!(captured.iter().any(|event| matches!(
            event,
            DomainEvent::ToolExecutionCompleted {
                tool_name,
                session_id,
                success,
                ..
            } if tool_name == "echo" && session_id == "turn-test-session" && !success
        )));
    }

    #[tokio::test]
    async fn execute_tool_call_reports_unknown_tool() {
        let agent = make_agent(None);
        let call = ParsedToolCall {
            name: "missing".into(),
            arguments: serde_json::json!({}),
            tool_call_id: None,
        };

        let (result, record) = agent.execute_tool_call(&call, 0).await;
        assert!(!result.success);
        assert!(result.output.contains("Unknown tool: missing"));
        assert_eq!(record.name, "missing");
        assert!(!record.success);
    }

    #[tokio::test]
    async fn turn_runs_full_tool_cycle_with_context_and_hooks() {
        let provider_impl = Arc::new(SequenceProvider {
            responses: AsyncMutex::new(vec![
                Ok(ChatResponse {
                    text: Some(
                        "preface <tool_call>{\"name\":\"echo\",\"arguments\":{\"value\":1}}</tool_call>"
                            .into(),
                    ),
                    tool_calls: vec![],
                    usage: None,
                }),
                Ok(ChatResponse {
                    text: Some("final answer".into()),
                    tool_calls: vec![],
                    usage: None,
                }),
            ]),
            requests: AsyncMutex::new(Vec::new()),
        });
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let hook_calls = Arc::new(AsyncMutex::new(Vec::<TurnContext>::new()));
        let hook_notify = Arc::new(Notify::new());
        let hooks: Vec<Arc<dyn PostTurnHook>> = vec![Arc::new(RecordingHook {
            calls: Arc::clone(&hook_calls),
            notify: Arc::clone(&hook_notify),
        })];

        let mut agent = make_agent_with_builder(
            provider,
            vec![Box::new(EchoTool)],
            Box::new(FixedMemoryLoader {
                context: "[Injected]\n".into(),
            }),
            hooks,
            crate::openhuman::config::AgentConfig {
                max_tool_iterations: 3,
                max_history_messages: 10,
                ..crate::openhuman::config::AgentConfig::default()
            },
            crate::openhuman::config::ContextConfig::default(),
        );

        let response = agent
            .turn("hello world")
            .await
            .expect("turn should succeed");
        assert_eq!(response, "final answer");
        assert!(agent.last_memory_context.as_deref() == Some("[Injected]\n"));
        assert!(agent.history.iter().any(|message| matches!(
            message,
            ConversationMessage::AssistantToolCalls { text, tool_calls }
                if text.as_deref().is_some_and(|value| value.contains("preface")) && tool_calls.len() == 1
        )));
        assert!(agent.history.iter().any(|message| matches!(
            message,
            ConversationMessage::Chat(chat) if chat.role == "assistant" && chat.content == "final answer"
        )));

        timeout(Duration::from_secs(1), async {
            loop {
                if !hook_calls.lock().await.is_empty() {
                    break;
                }
                hook_notify.notified().await;
            }
        })
        .await
        .expect("hook should fire");

        let recorded_hooks = hook_calls.lock().await;
        assert_eq!(recorded_hooks.len(), 1);
        assert_eq!(recorded_hooks[0].assistant_response, "final answer");
        assert_eq!(recorded_hooks[0].iteration_count, 2);
        assert_eq!(recorded_hooks[0].tool_calls.len(), 1);
        assert_eq!(recorded_hooks[0].tool_calls[0].name, "echo");
        drop(recorded_hooks);

        let requests = provider_impl.requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0][0].role, "system");
        assert!(requests[0][1].content.contains("[Injected]"));
        assert!(requests[0][1].content.contains("hello world"));
        assert!(requests[1]
            .iter()
            .any(|msg| msg.role == "assistant" && msg.content.contains("preface")));
        assert!(requests[1]
            .iter()
            .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]")));
    }

    #[tokio::test]
    async fn turn_uses_cached_transcript_prefix_on_first_iteration() {
        let provider_impl = Arc::new(SequenceProvider {
            responses: AsyncMutex::new(vec![Ok(ChatResponse {
                text: Some("cached-final".into()),
                tool_calls: vec![],
                usage: None,
            })]),
            requests: AsyncMutex::new(Vec::new()),
        });
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut agent = make_agent_with_builder(
            provider,
            vec![Box::new(EchoTool)],
            Box::new(FixedMemoryLoader {
                context: String::new(),
            }),
            vec![],
            crate::openhuman::config::AgentConfig::default(),
            crate::openhuman::config::ContextConfig::default(),
        );
        agent.cached_transcript_messages = Some(vec![
            ChatMessage::system("cached-system"),
            ChatMessage::assistant("cached-assistant"),
        ]);

        let response = agent.turn("fresh").await.expect("turn should succeed");
        assert_eq!(response, "cached-final");
        assert!(agent.cached_transcript_messages.is_none());

        let requests = provider_impl.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].len(), 3);
        assert_eq!(requests[0][0].content, "cached-system");
        assert_eq!(requests[0][1].content, "cached-assistant");
        assert_eq!(requests[0][2].role, "user");
        assert_eq!(requests[0][2].content, "fresh");
    }

    #[tokio::test]
    async fn turn_errors_when_max_tool_iterations_are_exceeded() {
        let provider: Arc<dyn Provider> = Arc::new(SequenceProvider {
            responses: AsyncMutex::new(vec![Ok(ChatResponse {
                text: Some("<tool_call>{\"name\":\"echo\",\"arguments\":{}}</tool_call>".into()),
                tool_calls: vec![],
                usage: None,
            })]),
            requests: AsyncMutex::new(Vec::new()),
        });
        let mut agent = make_agent_with_builder(
            provider,
            vec![Box::new(EchoTool)],
            Box::new(FixedMemoryLoader {
                context: String::new(),
            }),
            vec![],
            crate::openhuman::config::AgentConfig {
                max_tool_iterations: 1,
                ..crate::openhuman::config::AgentConfig::default()
            },
            crate::openhuman::config::ContextConfig::default(),
        );

        let err = agent
            .turn("hello")
            .await
            .expect_err("turn should stop at configured iteration budget");
        assert!(err
            .to_string()
            .contains("Agent exceeded maximum tool iterations (1)"));
        assert!(agent.history.iter().any(|message| matches!(
            message,
            ConversationMessage::AssistantToolCalls { tool_calls, .. } if tool_calls.len() == 1
        )));
    }

    #[tokio::test]
    async fn execute_tool_call_applies_inline_result_budget() {
        let provider: Arc<dyn Provider> = Arc::new(DummyProvider);
        let agent = make_agent_with_builder(
            provider,
            vec![Box::new(LongTool)],
            Box::new(FixedMemoryLoader {
                context: String::new(),
            }),
            vec![],
            crate::openhuman::config::AgentConfig::default(),
            crate::openhuman::config::ContextConfig {
                tool_result_budget_bytes: 300,
                ..crate::openhuman::config::ContextConfig::default()
            },
        );
        let call = ParsedToolCall {
            name: "long".into(),
            arguments: serde_json::json!({}),
            tool_call_id: Some("long-1".into()),
        };

        let (result, record) = agent.execute_tool_call(&call, 0).await;
        assert!(result.success);
        assert!(result.output.contains("truncated by tool_result_budget"));
        assert!(record.output_summary.starts_with("long: ok ("));
    }
}
