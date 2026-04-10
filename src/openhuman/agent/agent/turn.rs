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
//!   [`Agent::build_system_prompt`], [`Agent::classify_model`] — the
//!   small helpers `turn()` leans on every call.
//! - [`Agent::spawn_session_memory_extraction`] — the fire-and-forget
//!   background archivist fork.

use super::types::Agent;
use crate::openhuman::agent::context_pipeline;
use crate::openhuman::agent::dispatcher::{ParsedToolCall, ToolExecutionResult};
use crate::openhuman::agent::harness;
use crate::openhuman::agent::hooks::{self, ToolCallRecord, TurnContext};
use crate::openhuman::agent::prompt::{LearnedContextData, PromptContext};
use crate::openhuman::event_bus::{publish_global, DomainEvent};
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::providers::{ChatMessage, ChatRequest, ConversationMessage};
use crate::openhuman::tools::Tool;
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::sync::Arc;

impl Agent {
    /// Performs a single interaction "turn" with the agent.
    ///
    /// This is the core logic that takes user input, manages the history,
    /// calls the LLM, handles tool calls (up to `max_tool_iterations`),
    /// and returns the final assistant response.
    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
        let turn_started = std::time::Instant::now();
        log::info!(
            "[agent] turn started — awaiting user message processing"
        );
        log::info!(
            "[agent_loop] turn start message_chars={} history_len={} max_tool_iterations={}",
            user_message.chars().count(),
            self.history.len(),
            self.config.max_tool_iterations
        );
        if self.history.is_empty() {
            // Learned context is only baked into the system prompt on the
            // very first turn — once the history is non-empty we reuse the
            // stored prompt verbatim to preserve the KV-cache prefix the
            // inference backend has already tokenised. Fetching it later
            // would just burn memory-store reads on data we throw away.
            let learned = self.fetch_learned_context().await;
            let system_prompt = self.build_system_prompt(learned)?;
            log::info!("[agent] system prompt built — initialising conversation history");
            log::info!(
                "[agent_loop] system prompt built chars={}",
                system_prompt.chars().count()
            );
            log::debug!("[agent_loop] system prompt body:\n{system_prompt}");
            self.history
                .push(ConversationMessage::Chat(ChatMessage::system(
                    system_prompt,
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
        self.context_pipeline.tick_turn();

        // Collect tool call records across all iterations for post-turn hooks
        let mut all_tool_records: Vec<ToolCallRecord> = Vec::new();

        let turn_body = async {
            for iteration in 0..self.config.max_tool_iterations {
                log::info!(
                    "[agent_loop] iteration start i={} history_len={}",
                    iteration + 1,
                    self.history.len()
                );

                // Context pipeline stages 3 & 4: run the reduction
                // chain before every provider hit. Microcompact fires
                // when the guard reports we're above the soft threshold
                // and there are older tool results to clear; otherwise
                // we log an autocompaction signal (openhuman's
                // compactor lives in `loop_/history.rs` and operates on
                // the `ChatMessage` shape, so for now the
                // `ConversationMessage`-shaped Agent path lets the
                // signal bubble up as telemetry until a native
                // summariser lands).
                let outcome = self.context_pipeline.run_before_call(&mut self.history);
                match &outcome {
                    context_pipeline::PipelineOutcome::NoOp => {}
                    context_pipeline::PipelineOutcome::Microcompacted(stats) => {
                        log::info!(
                            "[agent_loop] context_pipeline microcompact i={} envelopes={} entries={} bytes_freed={}",
                            iteration + 1,
                            stats.envelopes_cleared,
                            stats.entries_cleared,
                            stats.bytes_freed
                        );
                    }
                    context_pipeline::PipelineOutcome::AutocompactionRequested {
                        utilisation_pct,
                    } => {
                        log::warn!(
                            "[agent_loop] context_pipeline autocompaction requested i={} utilisation_pct={}",
                            iteration + 1,
                            utilisation_pct
                        );
                    }
                    context_pipeline::PipelineOutcome::ContextExhausted {
                        utilisation_pct,
                        reason,
                    } => {
                        log::error!(
                            "[agent_loop] context_pipeline context exhausted i={} utilisation_pct={} reason={}",
                            iteration + 1,
                            utilisation_pct,
                            reason
                        );
                        return Err(anyhow::anyhow!(
                            "Context window exhausted ({utilisation_pct}% full): {reason}"
                        ));
                    }
                }

                let messages = self.tool_dispatcher.to_provider_messages(&self.history);
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
                            system_prompt_cache_boundary: None,
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
                        // Feed the context pipeline (guard +
                        // session-memory token accounting). No-op when
                        // the provider doesn't return usage.
                        if let Some(ref usage) = resp.usage {
                            self.context_pipeline.record_usage(usage);
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
                    // here and surface it via the pipeline state — the
                    // epilogue (below) reads `should_extract_session_memory()`.
                    self.context_pipeline
                        .record_tool_calls(all_tool_records.len());

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

                let (results, records) = self.execute_tools(&calls).await;
                all_tool_records.extend(records);
                log::info!(
                    "[agent_loop] tool results complete i={} result_count={}",
                    iteration + 1,
                    results.len()
                );
                for r in &results {
                    let preview = truncate_with_ellipsis(&r.output, 300);
                    log::info!(
                        "[agent] tool response name={} success={} output_chars={}\n{}",
                        r.name,
                        r.success,
                        r.output.chars().count(),
                        preview
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
        if result.is_ok() && self.context_pipeline.should_extract_session_memory() {
            self.spawn_session_memory_extraction();
        }

        result
    }

    // ─────────────────────────────────────────────────────────────────
    // Per-call tool execution
    // ─────────────────────────────────────────────────────────────────

    /// Executes a single tool call and returns the result and execution record.
    pub(super) async fn execute_tool_call(
        &self,
        call: &ParsedToolCall,
    ) -> (ToolExecutionResult, ToolCallRecord) {
        let started = std::time::Instant::now();
        publish_global(DomainEvent::ToolExecutionStarted {
            tool_name: call.name.clone(),
            session_id: self.event_session_id().to_string(),
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

        let (raw_result, success) =
            if let Some(tool) = self.tools.iter().find(|t| t.name() == call.name) {
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
        let budget_bytes = self.config.tool_result_budget_bytes;
        let (result, budget_outcome) =
            context_pipeline::apply_tool_result_budget(raw_result, budget_bytes);
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
    pub(super) async fn execute_tools(
        &self,
        calls: &[ParsedToolCall],
    ) -> (Vec<ToolExecutionResult>, Vec<ToolCallRecord>) {
        let mut results = Vec::with_capacity(calls.len());
        let mut records = Vec::with_capacity(calls.len());
        for call in calls {
            let (exec_result, record) = self.execute_tool_call(call).await;
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
            identity_config: self.identity_config.clone(),
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
            tool_specs: Arc::clone(&self.tool_specs),
            message_prefix: Arc::new(messages),
            cache_boundary: None,
            fork_task_prompt,
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // History & prompt helpers
    // ─────────────────────────────────────────────────────────────────

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
        }
    }

    /// Builds the system prompt for the current turn, including tool
    /// instructions and learned context.
    pub(super) fn build_system_prompt(&self, learned: LearnedContextData) -> Result<String> {
        let tools_slice: &[Box<dyn Tool>] = self.tools.as_slice();
        let instructions = self.tool_dispatcher.prompt_instructions(tools_slice);
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            tools: tools_slice,
            skills: &self.skills,
            identity_config: Some(&self.identity_config),
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &self.visible_tool_names,
        };
        self.prompt_builder.build(&ctx)
    }

    /// Classifies the user message to determine if a specific model hint should be used.
    ///
    /// Currently unused by `turn()` — we pin the main agent to its configured
    /// model for KV-cache stability (see the rationale in `turn()` where
    /// `effective_model` is set). Kept around because the classifier config
    /// is still surfaced via `AgentBuilder::classification_config` and
    /// external callers (e.g. eval harnesses) may want to probe it directly.
    #[allow(dead_code)]
    pub(super) fn classify_model(&self, user_message: &str) -> String {
        if let Some(hint) =
            crate::openhuman::agent::classifier::classify(&self.classification_config, user_message)
        {
            if self.available_hints.contains(&hint) {
                tracing::info!(hint = hint.as_str(), "Auto-classified query");
                return format!("hint:{hint}");
            }
        }
        self.model_name.clone()
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
        let extraction_prompt = context_pipeline::ARCHIVIST_EXTRACTION_PROMPT.to_string();

        // Optimistically flip the extraction state to "complete" right
        // away: we don't need a channel back from the background task
        // because a failed extraction is idempotent — it will just be
        // retried after the next threshold crossing. `mark_extraction_complete`
        // also clears the `extraction_in_progress` flag, so calling it
        // alone covers both bookkeeping steps.
        self.context_pipeline
            .session_memory
            .mark_extraction_complete();

        log::info!(
            "[session_memory] spawning background archivist extraction (turn={}, tokens={})",
            self.context_pipeline.session_memory.current_turn,
            self.context_pipeline.session_memory.total_tokens
        );

        tokio::spawn(async move {
            let options = harness::SubagentRunOptions::default();
            let fut = harness::run_subagent(&definition, &extraction_prompt, options);
            let result = harness::with_parent_context(parent_ctx, fut).await;
            match result {
                Ok(outcome) => tracing::info!(
                    agent_id = %outcome.agent_id,
                    task_id = %outcome.task_id,
                    iterations = outcome.iterations,
                    output_chars = outcome.output.chars().count(),
                    "[session_memory] archivist extraction completed"
                ),
                Err(err) => tracing::warn!(
                    error = %err,
                    "[session_memory] archivist extraction failed — will retry after next threshold crossing"
                ),
            }
        });
    }
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
