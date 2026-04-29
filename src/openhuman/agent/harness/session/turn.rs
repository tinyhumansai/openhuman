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
use crate::openhuman::agent::memory_loader::collect_recall_citations;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::context::prompt::{LearnedContextData, PromptContext, PromptTool};
use crate::openhuman::context::{ReductionOutcome, ARCHIVIST_EXTRACTION_PROMPT};
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::providers::{ChatMessage, ChatRequest, ConversationMessage, ProviderDelta};
use crate::openhuman::tools::Tool;
use crate::openhuman::util::truncate_with_ellipsis;
use anyhow::Result;
use std::hash::{Hash, Hasher};
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
        self.emit_progress(AgentProgress::TurnStarted).await;
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
                rendered_prompt.chars().count()
            );
            // User-file injection (PROFILE.md, MEMORY.md) puts
            // potentially-sensitive content (LinkedIn scrape output,
            // archivist-curated memories) into the system prompt. Avoid
            // leaking that to debug logs — log a length + content hash
            // instead. Narrow specialists (both flags off) keep the
            // full-body log so prompt-engineering iteration on
            // tools/safety sections stays easy.
            if self.omit_profile && self.omit_memory_md {
                log::debug!("[agent_loop] system prompt body:\n{}", rendered_prompt);
            } else {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                rendered_prompt.hash(&mut hasher);
                log::debug!(
                    "[agent_loop] system prompt body redacted (contains PROFILE/MEMORY): chars={} hash={:016x}",
                    rendered_prompt.chars().count(),
                    hasher.finish()
                );
            }
            self.history
                .push(ConversationMessage::Chat(ChatMessage::system(
                    rendered_prompt,
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
                .store(
                    "",
                    "user_msg",
                    user_message,
                    MemoryCategory::Conversation,
                    None,
                )
                .await;
        }

        log::info!("[agent] loading memory context for user message");
        const MEMORY_CITATION_LIMIT: usize = 5;
        const MEMORY_CITATION_MIN_RELEVANCE: f64 = 0.4;
        match collect_recall_citations(
            self.memory.as_ref(),
            user_message,
            MEMORY_CITATION_LIMIT,
            MEMORY_CITATION_MIN_RELEVANCE,
        )
        .await
        {
            Ok(citations) => {
                log::debug!(
                    "[agent_loop] memory citations collected count={}",
                    citations.len()
                );
                self.last_turn_citations = citations;
            }
            Err(err) => {
                log::warn!("[agent_loop] memory citation collection failed: {err}");
                self.last_turn_citations.clear();
            }
        }
        let context = self
            .memory_loader
            .load_context(self.memory.as_ref(), user_message)
            .await
            .unwrap_or_default();

        // ── Memory-tree eager prefetch (#710 wiring) ──────────────────
        // The orchestrator session injects a cross-source digest on the
        // first turn AND every `tree_loader::REFRESH_INTERVAL` (30 min by
        // default) thereafter, so long-running conversations stay current
        // with newly-ingested memory. Each injection still rides on the
        // user message (NOT the system prompt) to keep the KV-cache prefix
        // stable. Failure is non-fatal — bare `context` is returned on any
        // error. The timestamp is bumped on every successful `load` (even
        // when the digest is empty) so an empty workspace doesn't get
        // re-queried every turn.
        let now = std::time::Instant::now();
        let context = if crate::openhuman::agent::tree_loader::should_prefetch(
            self.last_tree_prefetch_at,
            now,
            crate::openhuman::agent::tree_loader::REFRESH_INTERVAL,
        ) {
            match crate::openhuman::config::rpc::load_config_with_timeout().await {
                Ok(cfg) => {
                    match crate::openhuman::agent::tree_loader::TreeContextLoader::load(&cfg).await
                    {
                        Ok(tree_ctx) => {
                            let was_first = self.last_tree_prefetch_at.is_none();
                            self.last_tree_prefetch_at = Some(now);
                            if !tree_ctx.is_empty() {
                                log::info!(
                                    "[memory_tree] tree context injected first_turn={} chars={}",
                                    was_first,
                                    tree_ctx.chars().count()
                                );
                                format!("{context}{tree_ctx}")
                            } else {
                                context
                            }
                        }
                        Err(e) => {
                            log::warn!("[memory_tree] tree_loader.load failed (non-fatal): {e}");
                            context
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[memory_tree] tree_loader skipped — config load failed (non-fatal): {e}"
                    );
                    context
                }
            }
        } else {
            log::trace!("[memory_tree] tree_loader skipped — within refresh interval");
            context
        };

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

        // ── SKILL.md body injection (#781) ───────────────────────────
        // Match installed SKILL.md skills against the user message and
        // prepend their bodies ahead of the memory-context block so the
        // LLM sees them at the top of the user turn. See the module
        // docs on [`crate::openhuman::skills::inject`] for the matching
        // heuristic and size cap rationale.
        let enriched = {
            use crate::openhuman::skills::inject;
            let matches = inject::match_skills(&self.skills, user_message);
            if matches.is_empty() {
                log::debug!(
                    "[skills:inject] no skill matches for user message (skill_catalog_len={})",
                    self.skills.len()
                );
                enriched
            } else {
                let injection = inject::render_injection(
                    &matches,
                    inject::DEFAULT_MAX_INJECTION_BYTES,
                    |skill| skill.read_body(),
                );
                let matched_count = injection.decisions.iter().filter(|d| d.matched).count();
                log::info!(
                    "[skills:inject] summary candidates={} matched={} injected_bytes={} truncated_any={}",
                    injection.decisions.len(),
                    matched_count,
                    injection.injected_bytes,
                    injection.truncated
                );
                if injection.rendered.is_empty() {
                    enriched
                } else {
                    format!("{}\n{}", injection.rendered, enriched)
                }
            }
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

        // Per-turn usage from the final provider response, attached to the
        // last assistant message in the persisted transcript.
        let mut last_turn_usage: Option<transcript::TurnUsage> = None;

        let turn_body = async {
            for iteration in 0..self.config.max_tool_iterations {
                self.emit_progress(AgentProgress::IterationStarted {
                    iteration: (iteration + 1) as u32,
                    max_iterations: self.config.max_tool_iterations as u32,
                })
                .await;
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
                // Only set up the streaming sink when someone is
                // listening for progress events. Without a listener the
                // channel buffer would fill up and back-pressure the
                // provider; skipping it also keeps the non-streaming
                // HTTP path alive for providers that don't implement
                // SSE.
                let iteration_for_stream = (iteration + 1) as u32;
                let (delta_tx_opt, delta_forwarder) = if self.on_progress.is_some() {
                    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(128);
                    let progress_tx = self.on_progress.clone();
                    let forwarder = tokio::spawn(async move {
                        while let Some(event) = rx.recv().await {
                            let Some(ref sink) = progress_tx else {
                                continue;
                            };
                            let mapped = match event {
                                ProviderDelta::TextDelta { delta } => AgentProgress::TextDelta {
                                    delta,
                                    iteration: iteration_for_stream,
                                },
                                ProviderDelta::ThinkingDelta { delta } => {
                                    AgentProgress::ThinkingDelta {
                                        delta,
                                        iteration: iteration_for_stream,
                                    }
                                }
                                ProviderDelta::ToolCallStart { call_id, tool_name } => {
                                    AgentProgress::ToolCallArgsDelta {
                                        call_id,
                                        tool_name,
                                        delta: String::new(),
                                        iteration: iteration_for_stream,
                                    }
                                }
                                ProviderDelta::ToolCallArgsDelta { call_id, delta } => {
                                    AgentProgress::ToolCallArgsDelta {
                                        call_id,
                                        tool_name: String::new(),
                                        delta,
                                        iteration: iteration_for_stream,
                                    }
                                }
                            };
                            // Await backpressure so streamed deltas arrive
                            // in order and aren't silently dropped when the
                            // downstream progress bridge is slow.
                            if sink.send(mapped).await.is_err() {
                                break;
                            }
                        }
                    });
                    (Some(tx), Some(forwarder))
                } else {
                    (None, None)
                };
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
                            stream: delta_tx_opt.as_ref(),
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
                            // Snapshot this turn's usage so the transcript
                            // writer can attribute it to the last assistant
                            // message.
                            last_turn_usage = Some(transcript::TurnUsage {
                                model: effective_model.clone(),
                                usage: transcript::MessageUsage {
                                    input: usage.input_tokens,
                                    output: usage.output_tokens,
                                    cached_input: usage.cached_input_tokens,
                                    cost_usd: usage.charged_amount_usd,
                                },
                                ts: chrono::Utc::now().to_rfc3339(),
                            });
                        } else {
                            // Missing usage on this iteration: clear any
                            // snapshot carried from a prior iteration so
                            // the transcript doesn't attribute stale
                            // numbers to the final assistant message.
                            last_turn_usage = None;
                        }
                        resp
                    }
                    Err(err) => {
                        drop(delta_tx_opt);
                        if let Some(handle) = delta_forwarder {
                            let _ = handle.await;
                        }
                        return Err(err);
                    }
                };
                drop(delta_tx_opt);
                if let Some(handle) = delta_forwarder {
                    let _ = handle.await;
                }

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
                    })
                    .await;

                    self.history
                        .push(ConversationMessage::Chat(ChatMessage::assistant(
                            final_text.clone(),
                        )));
                    self.trim_history();

                    // Mirror the final assistant reply into the transcript
                    // snapshot so the JSONL persisted below captures the
                    // response (not just the prompt that was sent).
                    if let Some(ref mut msgs) = last_provider_messages {
                        msgs.push(ChatMessage::assistant(final_text.clone()));
                    }

                    // Persist the transcript **now** — right after the
                    // provider response lands — so a crash during hooks
                    // / memory-extraction / the outer epilogue can't
                    // lose the assistant's reply.
                    if let Some(ref messages) = last_provider_messages {
                        self.persist_session_transcript(
                            messages,
                            cumulative_input_tokens,
                            cumulative_output_tokens,
                            cumulative_cached_input_tokens,
                            cumulative_charged_usd,
                            last_turn_usage.as_ref(),
                        );
                    }

                    if self.auto_save {
                        let summary = truncate_with_ellipsis(&final_text, 100);
                        let _ = self
                            .memory
                            .store("", "assistant_resp", &summary, MemoryCategory::Daily, None)
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

                // Persist the transcript **right after** the provider
                // response lands — before executing tools — so if the
                // session crashes mid-tool-call we still have the
                // assistant's response + tool-call intents on disk.
                // Rebuild `last_provider_messages` from the current
                // history so the snapshot includes whatever the
                // assistant just emitted (plain text + tool calls).
                last_provider_messages =
                    Some(self.tool_dispatcher.to_provider_messages(&self.history));
                if let Some(ref messages) = last_provider_messages {
                    self.persist_session_transcript(
                        messages,
                        cumulative_input_tokens,
                        cumulative_output_tokens,
                        cumulative_cached_input_tokens,
                        cumulative_charged_usd,
                        last_turn_usage.as_ref(),
                    );
                }

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
                // Flush the transcript again now that tool results have
                // been appended — the pre-tool persist above only
                // captured the assistant's tool-call intents. A crash
                // or early-exit between iterations would otherwise lose
                // the tool output from the on-disk session record.
                let post_tool_messages = self.tool_dispatcher.to_provider_messages(&self.history);
                self.persist_session_transcript(
                    &post_tool_messages,
                    cumulative_input_tokens,
                    cumulative_output_tokens,
                    cumulative_cached_input_tokens,
                    cumulative_charged_usd,
                    last_turn_usage.as_ref(),
                );
                last_provider_messages = Some(post_tool_messages);
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

        // Session transcript persistence lives INSIDE the turn body —
        // one write per provider response, fired right after the
        // response lands (see the tool-call and terminal branches in
        // `turn_body`). A crash during tool execution no longer drops
        // the assistant's reply because it was already flushed to
        // disk before tool dispatch started. No outer-loop save is
        // needed here.

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
        // Synthesise a fallback id for prompt-guided (non-native) tool
        // calls so downstream consumers always have a stable key to
        // reconcile tool_call / tool_args_delta / tool_result rows by.
        // A random uuid guarantees uniqueness even when the same tool
        // name appears multiple times in the same iteration's parsed
        // calls.
        let call_id = call.tool_call_id.clone().unwrap_or_else(|| {
            format!(
                "turn-{iteration}-{}-{}",
                call.name,
                uuid::Uuid::new_v4().simple()
            )
        });
        self.emit_progress(AgentProgress::ToolCallStarted {
            call_id: call_id.clone(),
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
            iteration: (iteration + 1) as u32,
        })
        .await;
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
                        let mut output = r.output();
                        // Issue #574 — if a payload summarizer is wired
                        // in (orchestrator session only) and the output
                        // exceeds the configured threshold, hand it to
                        // the summarizer sub-agent before it enters
                        // history. On any failure or below-threshold
                        // payload, leave `output` untouched and let the
                        // existing tool_result_budget_bytes truncation
                        // pipeline handle it downstream.
                        if let Some(ps) = self.payload_summarizer.as_ref() {
                            log::debug!(
                                "[agent_loop] payload_summarizer intercepting tool={} bytes={}",
                                call.name,
                                output.len()
                            );
                            match ps.maybe_summarize(&call.name, None, &output).await {
                                Ok(Some(payload)) => {
                                    log::info!(
                                        "[agent_loop] payload_summarizer compressed tool={} {}->{} bytes",
                                        call.name,
                                        payload.original_bytes,
                                        payload.summary_bytes
                                    );
                                    output = payload.summary;
                                }
                                Ok(None) => {
                                    log::debug!(
                                        "[agent_loop] payload_summarizer pass-through tool={} bytes={}",
                                        call.name,
                                        output.len()
                                    );
                                }
                                Err(e) => {
                                    log::warn!(
                                        "[agent_loop] payload_summarizer error tool={} err={} (passing raw payload through)",
                                        call.name,
                                        e
                                    );
                                }
                            }
                        }
                        (output, true)
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
            call_id: call_id.clone(),
            tool_name: call.name.clone(),
            success,
            output_chars: result.chars().count(),
            elapsed_ms,
            iteration: (iteration + 1) as u32,
        })
        .await;
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
            connected_integrations: self.connected_integrations.clone(),
            composio_client: self.composio_client.clone(),
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            session_key: self.session_key.clone(),
            session_parent_prefix: self.session_parent_prefix.clone(),
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
            fork_task_prompt,
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // History & prompt helpers
    // ─────────────────────────────────────────────────────────────────

    /// Emit a lifecycle progress event. Uses `send().await` so control
    /// events (turn/iteration boundaries, tool_call_started/completed,
    /// turn_completed) survive downstream backpressure from the
    /// higher-frequency streamed deltas that share the same `on_progress`
    /// channel — dropping one of these would desync the web-channel
    /// progress bridge (e.g. a tool row stuck in `running` forever).
    /// A closed sink is logged and ignored; no progress subscriber is
    /// equivalent to success.
    async fn emit_progress(&self, event: AgentProgress) {
        if let Some(ref tx) = self.on_progress {
            if let Err(e) = tx.send(event).await {
                log::warn!("[agent] progress sink closed while emitting lifecycle event: {e}");
            }
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
                Some("learning_observations"),
                Some(&MemoryCategory::Custom("learning_observations".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let pat_entries = self
            .memory
            .list(
                Some("learning_patterns"),
                Some(&MemoryCategory::Custom("learning_patterns".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let profile_entries = self
            .memory
            .list(
                Some("user_profile"),
                Some(&MemoryCategory::Custom("user_profile".into())),
                None,
            )
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
    /// Also caches a [`ComposioClient`] on the session so the sub-agent
    /// runner can construct per-action tools for `integrations_agent` spawns
    /// without rebuilding the client on every call.
    ///
    /// Delegates to the shared [`crate::openhuman::composio::fetch_connected_integrations`]
    /// which is the single source of truth for integration discovery.
    pub async fn fetch_connected_integrations(&mut self) {
        let config = match crate::openhuman::config::Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                log::debug!(
                    "[agent] skipping connected integrations fetch: config load failed: {e}"
                );
                return;
            }
        };
        self.connected_integrations =
            crate::openhuman::composio::fetch_connected_integrations(&config).await;
        self.composio_client = crate::openhuman::composio::build_composio_client(&config);
    }

    /// Builds the system prompt for the current turn, including tool
    /// instructions and learned context.
    pub fn build_system_prompt(&self, learned: LearnedContextData) -> Result<String> {
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
            agent_id: &self.agent_definition_name,
            tools: &prompt_tools,
            skills: &self.skills,
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &self.visible_tool_names,
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            connected_integrations: &self.connected_integrations,
            connected_identities_md: crate::openhuman::agent::prompts::render_connected_identities(
            ),
            include_profile: !self.omit_profile,
            include_memory_md: !self.omit_memory_md,
        };
        // Route through the global context manager so every
        // prompt-building call-site — main agent, sub-agent runner,
        // channel runtimes — shares one builder configuration.
        self.context.build_system_prompt(&ctx)
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
                        log::info!(
                            "[transcript] loaded {} messages for resume",
                            session.messages.len()
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
    /// Writes JSONL as source of truth and re-renders the companion `.md`
    /// for human readability. Best-effort: failures are logged and silently
    /// ignored. The JSONL conversation store remains the authoritative
    /// persistence layer; session transcripts are an optimization for KV
    /// cache stability.
    ///
    /// `turn_usage` — when `Some`, attributes per-message token/cost figures
    /// to the last assistant message in the written transcript.
    pub(super) fn persist_session_transcript(
        &mut self,
        messages: &[ChatMessage],
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
        charged_amount_usd: f64,
        turn_usage: Option<&transcript::TurnUsage>,
    ) {
        // Resolve the transcript path on first write. The stem is
        // `{parent_prefix}__{session_key}` for sub-agents (producing a
        // flat hierarchical filename) or just `{session_key}` for a
        // root session. Prefix chaining is already done by the
        // sub-agent runner when it populates `session_parent_prefix`.
        if self.session_transcript_path.is_none() {
            let stem = match &self.session_parent_prefix {
                Some(prefix) => format!("{}__{}", prefix, self.session_key),
                None => self.session_key.clone(),
            };
            match transcript::resolve_keyed_transcript_path(&self.workspace_dir, &stem) {
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
            created: now.clone(),
            updated: now,
            turn_count: self.context.stats().session_memory_current_turn as usize,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            charged_amount_usd,
        };

        if let Err(err) = transcript::write_transcript(path, messages, &meta, turn_usage) {
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
#[path = "turn_tests.rs"]
mod tests;
