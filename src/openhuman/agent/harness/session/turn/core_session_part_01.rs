impl Agent {
    /// Drive a full chat turn through the `tinyagents` harness (issue #4249).
    ///
    /// The frozen system+prior history is converted to provider messages, the
    /// user turn appended, and the loop run over the agent's resolved tools. The
    /// final reply + the user turn are recorded into `history`, the transcript
    /// is persisted, and `TurnCompleted` is emitted so the UI stops spinning.
    ///
    /// Full-fidelity with the legacy `run_turn_engine`: live tool-timeline /
    /// text-delta progress and the cost/token footer are mirrored from the
    /// harness event stream via `OpenhumanEventBridge` (tinyagents harness),
    /// `[IMAGE:…]`/`[FILE:…]` markers are expanded for the provider, and history
    /// is trimmed to the provider's context window.
    async fn run_turn_via_tinyagents_session(
        &mut self,
        user_message: &str,
        effective_model: &str,
        temperature: f64,
        max_iterations: usize,
        artifact_store: Option<
            crate::openhuman::agent::harness::tool_result_artifacts::ToolResultArtifactStore,
        >,
        suppress_tools: bool,
    ) -> Result<String> {
        let turn_started = std::time::Instant::now();
        // This turn's stamped user message is already the last entry in
        // `self.history` (pushed by `turn()` before the engine branch), so build
        // the provider messages straight from history — do NOT push the user
        // again. A resumed session's replayed prefix is folded into that same
        // history first (see below), so there is exactly one sequence to read.
        self.absorb_resumed_transcript_prefix();
        let mut messages = self.tool_dispatcher.to_provider_messages(&self.history);

        // Multimodal prep (parity with the legacy engine): rehydrate image
        // placeholders for vision-capable providers, then expand `[IMAGE:…]` /
        // `[FILE:…]` markers into provider-ready content before dispatch. The
        // expanded copy is provider-only and never persisted to `history`.
        let multimodal = self
            .runtime_config
            .as_ref()
            .map(|c| c.multimodal.clone())
            .unwrap_or_default();
        let multimodal_files = self
            .runtime_config
            .as_ref()
            .map(|c| c.multimodal_files.clone())
            .unwrap_or_default();
        // Resolve the effective context window and build the turn's tiered crate
        // `ChatModel` set from the session source up front (issue #4249, Phase 3 /
        // Motion A) — the harness holds crate model types, and the vision read
        // below comes off the built models, not a raw provider.
        let context_window = self
            .turn_model_source
            .effective_context_window(effective_model)
            .await;
        let turn_models =
            self.turn_model_source
                .build(effective_model, temperature, context_window)?;

        // Honor custom/BYOK vision models too: they can set `model_vision` even
        // when the provider capability bit is false, and must still rehydrate
        // `[IMAGE:…]` placeholders (else image chat silently degrades to text).
        if (turn_models.supports_vision() || self.model_vision)
            && crate::openhuman::agent::multimodal::has_image_placeholders(&messages)
        {
            messages = crate::openhuman::agent::multimodal::rehydrate_image_placeholders(&messages);
        }
        let messages = crate::openhuman::agent::multimodal::prepare_messages_for_provider(
            &messages,
            &multimodal,
            &multimodal_files,
        )
        .await
        .map(|prepared| prepared.messages)
        .unwrap_or(messages);

        // Per-turn tool scope (#1725). A chat / small-talk turn runs with an
        // EMPTY tool set: the provider request carries no tool schema, so the
        // model cannot enter the tool loop and answers in a single call. The
        // agent's durable `self.tools` / `self.visible_tool_names` are left
        // untouched — the next un-overridden turn gets the full toolbelt back.
        let (turn_tools, turn_synthesized_tools, turn_visible_tool_names) =
            self.turn_tool_sets(suppress_tools);

        tracing::info!(
            model = %effective_model,
            max_iterations,
            tools = turn_tools.len() + turn_synthesized_tools.len(),
            suppress_tools,
            "[agent_loop] routing chat turn through the tinyagents harness"
        );

        // Dispatch through the chat turn graph (this folder's `graph.rs`): a thin
        // wrapper over the shared tinyagents seam that pins the chat path's fixed
        // arguments (no child scope, no early-exit tools, graceful cap pause,
        // per-turn output cap) and runs the context-window summarization step.
        // Context middlewares sourced from this session's ContextManager: the
        // per-tool-result byte cap + payload summarizer (after_tool) and
        // microcompact tool-body clearing (before_model). KV-cache-prefix drift
        // detection is owned by the crate `PromptCacheGuardMiddleware` (fed by
        // `PromptCacheSegmentMiddleware`); the warn-only `CacheAlignMiddleware`
        // was deleted in C3.
        let context_mw = crate::openhuman::agent::tinyagents::TurnContextMiddleware {
            tool_result_budget_bytes: self.context.tool_result_budget_bytes(),
            payload_summarizer: self.payload_summarizer.clone(),
            artifact_store,
            tokenjuice_compaction_enabled: self.context.compaction_enabled(),
            tokenjuice_compression: self.tokenjuice_compression,
            microcompact_keep_recent: self.context.microcompact_keep_recent(),
            // Honor the [context].enabled / autocompact_enabled opt-outs: when off,
            // the summarization middleware is not installed (no summarizer tokens,
            // no history rewrite).
            autocompact_enabled: self.context.autocompact_enabled(),
            // Progressive-disclosure handoff is a sub-agent (integrations_agent)
            // concern; the top-level chat turn never sets it.
            handoff: None,
            // Live transcript snapshotting is a sub-agent error-recovery concern
            // (#4466); the chat path persists its transcript post-run.
            transcript_snapshot: None,
        };

        // Gather any sub-agent spend delegated during this turn (synchronous
        // `spawn_subagent` runs inline on this task and records into the collector)
        // so the turn's usage meters + the `chat_done` per-child breakdown include
        // it — the collector scope the legacy engine installed.
        // Install the turn's sub-agent dispatch guard around the same future
        // (#5804). It records two facts the turn already produces but never
        // wrote down — that a graceful pause has been requested at the
        // model-call cap, and how long this turn's sub-agents actually take —
        // so `run_subagent` can refuse a dispatch that cannot finish inside the
        // remaining wall-clock budget instead of taking the whole turn down
        // with it. Boxed at the call site: `with_dispatch_guard` takes its
        // future by value, and the collector future wraps the entire turn
        // generator, so passing it unboxed would move hundreds of KiB through
        // this frame — the same hazard `with_turn_collector`'s own comment
        // documents, with the gdb measurements behind it.
        let turn_future = Box::pin(
            crate::openhuman::agent::harness::turn_subagent_usage::with_turn_collector(
                super::graph::run_chat_turn_graph(super::graph::ChatTurnGraph {
                    turn_models,
                    model: effective_model.to_string(),
                    messages,
                    tools: turn_tools,
                    synthesized_tools: turn_synthesized_tools,
                    visible_tool_names: turn_visible_tool_names,
                    max_iterations,
                    on_progress: self.on_progress.clone(),
                    context_window,
                    run_queue: self.run_queue.clone(),
                    context_mw,
                    // Enforce the builder-configured tool policy at the tool
                    // boundary (the tinyagents path otherwise bypasses it).
                    tool_policy: Some(crate::openhuman::agent::tinyagents::ToolPolicyEnforcement {
                        policy: self.tool_policy.clone(),
                        session: self.tool_policy_session.clone(),
                        session_id: self.event_session_id.clone(),
                        channel: self.event_channel().to_string(),
                        agent_definition_id: self.agent_definition_id.clone(),
                    }),
                    // Section D: forward the session's per-profile workspace
                    // descriptor (if any) so the top-level chat turn's acting
                    // tools default their cwd to the profile's dedicated dir.
                    workspace_descriptor: self.workspace_descriptor.clone(),
                    // Scope direct Master-Agent calls under its declared
                    // sandbox. `agent_definition_name` can carry a thread
                    // suffix, so resolve with the stable definition id.
                    sandbox_mode: crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
                        .and_then(|registry| registry.get(&self.agent_definition_id))
                        .map(|definition| definition.sandbox_mode)
                        .unwrap_or(crate::openhuman::agent::harness::definition::SandboxMode::None),
                }),
            ),
        );
        let (outcome, subagent_usage_entries) =
            crate::openhuman::agent::harness::turn_dispatch_guard::with_dispatch_guard(
                crate::openhuman::agent::tinyagents::agent_turn_wall_clock_ms()
                    .map(std::time::Duration::from_millis),
                turn_future,
            )
            .await;
        let outcome = outcome?;

        // Record whether this turn paused at the tool-call cap (vs. finishing
        // naturally) BEFORE anything below can early-return, so a caller
        // inspecting `last_turn_hit_cap()` after `run_single` always reflects
        // this turn, never a stale value from a prior one.
        self.last_turn_hit_cap = outcome.hit_cap;

        // The stamped user turn is already in `self.history` (pushed by `turn()`),
        // so append only the structured messages this turn produced — assistant
        // tool calls + tool results + (for a clean finish) the final assistant —
        // preserving tool-call history fidelity for the UI, persisted transcript,
        // and the next turn's KV-cache prefix.
        self.history.extend(outcome.conversation.iter().cloned());

        // Token accounting for the turn (the cap checkpoint call below folds in
        // its own usage).
        // Seed from the turn outcome (the harness observed real usage incl. cached
        // tokens and an estimated cost) rather than zero, so a normal non-cap turn
        // persists real cost instead of $0. The cap-checkpoint branch below folds
        // in its extra call's usage on top.
        let mut input_tokens = outcome.input_tokens;
        let mut output_tokens = outcome.output_tokens;
        let mut cached_input_tokens = outcome.cached_input_tokens;
        let mut charged_amount_usd = outcome.charged_amount_usd;

        let reply = if outcome.hit_cap && outcome.wrap_up_injected && !outcome.text.trim().is_empty()
        {
            // The conclusion was produced INSIDE the loop (issue #6014):
            // `FinalCallWrapUpMiddleware` withdrew the tools on the last
            // permitted model call and appended the wrap-up instruction, so this
            // turn's own final response already is the capped turn's answer.
            // There is nothing left to ask for, and asking anyway would spend a
            // second call to re-summarise text the model has just written.
            //
            // No `history.push` either: unlike the out-of-band path below, this
            // response came from the loop, so it was already folded into
            // `self.history` by the `extend(outcome.conversation)` above. Pushing
            // it again would duplicate the assistant turn in the transcript and
            // in the next request's prefix.
            log::info!(
                "[agent_loop] turn capped at {} model call(s); the loop's final call produced the \
                 conclusion ({} chars) — no out-of-band wrap-up needed",
                outcome.model_calls,
                outcome.text.chars().count()
            );
            outcome.text.clone()
        } else if outcome.hit_cap {
            // The loop paused at the tool-call cap. Ask the model for a resumable
            // checkpoint (tools disabled), falling back to a deterministic
            // done/next summary so the thread never ends on a dangling tool
            // cycle. Fold the extra call's usage into the turn accounting.
            //
            // Reached when the in-loop conclusion above did not happen or came
            // back empty: a run with the middleware uninstalled, a budget of one
            // model call, or a concluding call that answered with nothing. Kept
            // intact so no path loses the behaviour it had before #6014.
            // The concluding call answered with nothing, and the `extend` above
            // folded that empty assistant message into `self.history`. Anthropic
            // rejects empty content, so sending it would turn "the model said
            // nothing" into a failed turn (CodeRabbit on #6068).
            if self.history.last().is_some_and(is_empty_assistant_chat) {
                self.history.pop();
            }
            let base = self.tool_dispatcher.to_provider_messages(&self.history);
            let (summary, summary_usage) = self
                .summarize_turn_wrapup(
                    &base,
                    effective_model,
                    outcome.model_calls as u32 + 1,
                    super::super::turn_checkpoint::MAX_ITER_CHECKPOINT_INSTRUCTION,
                )
                .await;
            if let Some(u) = summary_usage {
                input_tokens += u.input_tokens;
                output_tokens += u.output_tokens;
                cached_input_tokens += u.cached_input_tokens;
                charged_amount_usd += u.charged_amount_usd;
            }
            let checkpoint = if summary.trim().is_empty() {
                super::super::turn_checkpoint::build_deterministic_checkpoint(
                    &checkpoint_results_from_conversation(
                        &outcome.conversation,
                        &outcome.tool_outcomes,
                    ),
                    max_iterations,
                )
            } else {
                summary
            };
            self.history
                .push(ConversationMessage::Chat(ChatMessage::assistant(
                    checkpoint.clone(),
                )));
            checkpoint
        } else if outcome.text.trim().is_empty() && outcome.tool_calls == 0 {
            // A completion with no text and no tool calls is never a valid final
            // answer — surface it as an error instead of wedging the thread on a
            // blank reply (bug-report-2026-05-26 A1, defect B).
            //
            // #4457 (defect A): the empty terminal assistant response was already
            // folded into `self.history` via `outcome.conversation` at the
            // `history.extend` above (an empty `Chat(assistant(""))`). The #4093
            // branch below pops that dangling blank row before re-prompting, but
            // this `tool_calls == 0` path returned the error with the empty row
            // still in history — so the *next* request carried an empty-content
            // assistant message and strict providers (Anthropic: "text content
            // blocks must be non-empty") 400 the whole thread, not just this turn.
            // Pop the trailing empty assistant row before returning so a retry
            // sends a clean transcript.
            if matches!(
                self.history.last(),
                Some(ConversationMessage::Chat(msg))
                    if msg.role == "assistant" && msg.content.trim().is_empty()
            ) {
                log::debug!(
                    "[agent_loop] EmptyProviderResponse at iteration {}: popping dangling empty assistant row before returning — #4457 defect A",
                    outcome.model_calls
                );
                self.history.pop();
            }
            return Err(anyhow::Error::new(
                crate::openhuman::agent::error::AgentError::EmptyProviderResponse {
                    iteration: outcome.model_calls,
                },
            ));
        } else if outcome.text.trim().is_empty() {
            // #4093: the loop ran tool calls (tool_calls > 0, so the branch
            // above did not fire) and then yielded a terminating response with
            // no final text — the turn did work but would otherwise end
            // silently, leaving the user with nothing. Enforce the
            // "must produce a final response" terminal step: re-prompt the
            // model (tools disabled) for a closing summary of what it did,
            // falling back to a deterministic summary of the tool calls so the
            // synthesized message is never itself empty. Fold the extra call's
            // usage into the turn accounting, exactly like the cap path above.
            let base = self.tool_dispatcher.to_provider_messages(&self.history);
            let (summary, summary_usage) = self
                .summarize_turn_wrapup(
                    &base,
                    effective_model,
                    outcome.model_calls as u32 + 1,
                    super::super::turn_checkpoint::FINAL_ANSWER_INSTRUCTION,
                )
                .await;
            if let Some(u) = summary_usage {
                input_tokens += u.input_tokens;
                output_tokens += u.output_tokens;
                cached_input_tokens += u.cached_input_tokens;
                charged_amount_usd += u.charged_amount_usd;
            }
            let final_answer = if summary.trim().is_empty() {
                super::super::turn_checkpoint::build_deterministic_final_summary(
                    &tool_records_from_conversation(&outcome.conversation, &outcome.tool_outcomes),
                )
            } else {
                summary
            };
            log::info!(
                "[agent_loop] turn produced no final text after {} tool call(s); synthesized a closing summary ({} chars) — #4093",
                outcome.tool_calls,
                final_answer.chars().count()
            );
            // The empty terminal assistant response was already folded into
            // `self.history` via `outcome.conversation` above (an empty
            // `Chat(assistant(""))` — see `messages_to_conversation`). Drop that
            // blank turn before appending the synthesized answer so the
            // transcript and the next prompt don't carry a dangling empty
            // assistant message immediately before the real reply (Codex review).
            if matches!(
                self.history.last(),
                Some(ConversationMessage::Chat(msg))
                    if msg.role == "assistant" && msg.content.trim().is_empty()
            ) {
                self.history.pop();
            }
            self.history
                .push(ConversationMessage::Chat(ChatMessage::assistant(
                    final_answer.clone(),
                )));
            final_answer
        } else if outcome.early_exit_tool.is_some() {
            // Paused on `ask_user_clarification`. The run stopped right after the
            // tool round, so `outcome.conversation` ends on a tool result and
            // carries no final assistant turn — `outcome.text` (the question) is
            // standing in for one. Push it, or the reply the user is looking at
            // would be missing from the next request's prefix and the model would
            // have to infer that it asked anything from its own tool result.
            // The sub-agent path makes the same substitution when it persists a
            // paused child's transcript (`subagent_runner/ops/graph_part_01.rs`).
            self.history
                .push(ConversationMessage::Chat(ChatMessage::assistant(
                    outcome.text.clone(),
                )));
            outcome.text.clone()
        } else {
            outcome.text.clone()
        };

        // Enforce the required structured-output contract (issue #4117) on the
        // accepted reply — for the branches above that actually deliver one
        // (normal finish, cap checkpoint, #4093 synthesized close), since each
        // is a reply downstream parsing depends on. When this agent must emit a JSON block
        // every turn and the reply omitted it, validate-and-repair before the
        // turn is accepted, reconciling with streaming (append-only when a live
        // stream is attached, replace otherwise — see `enforce_required_output`).
        // The trailing assistant message is rewritten to match, and the repair
        // call's usage is folded into the turn accounting. `required_output`
        // defaults to `None`, so existing agents are entirely unaffected.
        // Converted to the crate contract at the read site: the enforcement
        // helpers below are part of the runtime slated to move into TinyAgents
        // and so speak the crate type, while the session still holds the host's
        // `AgentConfig`. See `tinyagents::config::required_output_from`.
        //
        // NOT for a turn paused on `ask_user_clarification`. The reply there is
        // the question itself, which will not carry the required block, so
        // enforcement would spend a repair model call rewriting the very text
        // the user is being asked to answer — and `replace_last_assistant_reply`
        // would persist the rewrite, so the question in the transcript would
        // stop matching the one on screen. A paused turn delivers no reply for
        // downstream parsing to depend on; the contract applies to the answer
        // this agent gives after the user replies, not to the asking (#6213
        // review).
        let required_output = if outcome.early_exit_tool.is_some() {
            None
        } else {
            self.config
                .required_output
                .as_ref()
                .map(crate::openhuman::agent::tinyagents::config::required_output_from)
        };
        let reply = if let Some(contract) = required_output {
            match self
                .enforce_required_output(
                    &reply,
                    &contract,
                    effective_model,
                    outcome.model_calls as u32 + 1,
                )
                .await
            {
                Some((repaired, repair_usage)) => {
                    if let Some(u) = repair_usage {
                        input_tokens += u.input_tokens;
                        output_tokens += u.output_tokens;
                        cached_input_tokens += u.cached_input_tokens;
                        charged_amount_usd += u.charged_amount_usd;
                    }
                    replace_last_assistant_reply(&mut self.history, &repaired);
                    repaired
                }
                None => reply,
            }
        } else {
            reply
        };
        self.trim_history();

        // Fold this turn's sub-agent spend into the cumulative meters and capture
        // the holistic per-turn usage the web channel surfaces on `chat_done` (it
        // calls `take_last_turn_usage_totals()` right after the turn). Without this
        // the event reported `usage: None` despite the transcript being persisted
        // with real numbers.
        for entry in &subagent_usage_entries {
            input_tokens = input_tokens.saturating_add(entry.usage.input_tokens);
            output_tokens = output_tokens.saturating_add(entry.usage.output_tokens);
            cached_input_tokens =
                cached_input_tokens.saturating_add(entry.usage.cached_input_tokens);
            charged_amount_usd += entry.usage.charged_amount_usd;
        }
        self.last_turn_usage_totals = Some(
            crate::openhuman::agent::harness::turn_subagent_usage::LastTurnUsage {
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cost_usd: charged_amount_usd,
                context_window: context_window.unwrap_or(0),
                subagents: subagent_usage_entries,
            },
        );

        let mut persisted = self.tool_dispatcher.to_provider_messages(&self.history);
        // Re-attach per-call failure outcomes (dropped when the engine folded
        // each tool result into a `role:"tool"` message) so the derived
        // transcript view renders failed tools as errors, not successes.
        stamp_tool_failures(&mut persisted, &outcome.tool_outcomes);
        // Carry the turn's provider (event channel) + effective model and usage
        // into the persisted transcript meta. Passing `None` here dropped
        // `provider`/`model` from every transcript (they are `TranscriptMeta`
        // fields sourced from the turn usage) — parity with the legacy engine,
        // which handed `self.last_turn_usage.as_ref()` to this call.
        let turn_usage = crate::openhuman::agent::harness::session::transcript::TurnUsage {
            provider: self.event_channel().to_string(),
            // The model that actually ran this turn (a per-turn override can
            // diverge from `self.model_name`); attribute usage to it.
            model: effective_model.to_string(),
            usage: crate::openhuman::agent::harness::session::transcript::MessageUsage {
                input: input_tokens,
                output: output_tokens,
                cached_input: cached_input_tokens,
                context_window: context_window.unwrap_or(0),
                cost_usd: charged_amount_usd,
            },
            ts: chrono::Utc::now().to_rfc3339(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            iteration: outcome.model_calls as u32,
        };
        self.persist_session_transcript(
            &persisted,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            charged_amount_usd,
            Some(&turn_usage),
        );

        // Charge this turn's usage against the thread's active goal (parity with
        // the legacy engine) so budgeted goals progress to `budget_limited` and
        // continuation scheduling reads a live budget. Self-guarding + best-effort
        // — a no-op when there is no active goal for the ambient thread.
        crate::openhuman::threads::goals::runtime::account_turn_against_goal(
            &self.workspace_dir,
            input_tokens,
            output_tokens,
            turn_started.elapsed().as_secs(),
        )
        .await;

        // Content (prompt + reply) rides its own event so a tracing consumer can
        // attach it to the turn span. Gated on the opt-in
        // `observability.agent_tracing.capture_content` flag (#4454): with the
        // default off, we don't even emit the content event, so prompt/reply text
        // never reaches the span store or any exporter. The collector applies the
        // same storage-level gate as defense in depth.
        let capture_content = self
            .runtime_config
            .as_ref()
            .map(|c| c.observability.agent_tracing.capture_content)
            .unwrap_or(false);
        if capture_content {
            log::debug!(
                target: "agent-tracing",
                "[agent-tracing] emitting TurnContent (capture_content=true)"
            );
            self.emit_progress(AgentProgress::TurnContent {
                input: Some(user_message.to_string()),
                output: Some(reply.clone()),
            })
            .await;
        } else {
            log::debug!(
                target: "agent-tracing",
                "[agent-tracing] skipping TurnContent emit (capture_content=false)"
            );
        }

        self.emit_progress(AgentProgress::TurnCompleted {
            iterations: outcome.model_calls as u32,
        })
        .await;

        if self.auto_save {
            let summary = truncate_with_ellipsis(&reply, 100);
            let autosave_key = format!("assistant_resp:{}", uuid::Uuid::new_v4());
            let _ = self
                .memory
                .store(
                    crate::openhuman::agent::learning::transcript_ingest::CONVERSATION_RAW_NAMESPACE,
                    &autosave_key,
                    &summary,
                    MemoryCategory::Daily,
                    None,
                )
                .await;
        }

        // Fire post-turn hooks (non-blocking), matching the legacy engine.
        if !self.post_turn_hooks.is_empty() {
            let ctx = TurnContext {
                user_message: user_message.to_string(),
                assistant_response: reply.clone(),
                tool_calls: tool_records_from_conversation(
                    &outcome.conversation,
                    &outcome.tool_outcomes,
                ),
                turn_duration_ms: turn_started.elapsed().as_millis() as u64,
                session_id: Some(self.event_session_id.clone())
                    .filter(|session_id| !session_id.trim().is_empty()),
                agent_id: Some(self.agent_definition_id.clone())
                    .filter(|agent_id| !agent_id.trim().is_empty()),
                entrypoint: Some(self.event_channel.clone())
                    .filter(|entrypoint| !entrypoint.trim().is_empty()),
                iteration_count: outcome.model_calls,
            };
            hooks::fire_hooks(&self.post_turn_hooks, ctx);
        }

        Ok(reply)
    }
}

impl Agent {
    /// Fold a resumed session's replayed transcript prefix into [`Agent::history`].
    ///
    /// A no-op unless `cached_transcript_messages` is set, which happens only on
    /// the first turn after a resume.
    pub(in super::super) fn absorb_resumed_transcript_prefix(&mut self) {
        let Some(cached) = self.cached_transcript_messages.take() else {
            return;
        };
        // A resumed session's replayed prefix is **absorbed into
        // `self.history`**, not merely prepended to this one request.
        //
        // It used to be prepended: the request was built from `history`,
        // the cached prefix was `take()`n and spliced in front, and
        // `history` was left holding only the new turn. That made the
        // replayed conversation visible to exactly one request, and it cost
        // the conversation twice over — both observed on the wire, not
        // reasoned about:
        //
        //  1. **The next turn lost it.** `take()` empties the field, and
        //     nothing else ever re-read the transcript, so turn 2 after a
        //     restart went out with only its own messages. A ten-message
        //     thread answered its next question from four. That is
        //     conversational amnesia first and a cache miss second.
        //  2. **The transcript written afterwards lost it.** Persistence
        //     serializes `history` (see `run_turn_impl`'s
        //     `persist_session_transcript` call), so the file produced after
        //     a resume held only the new turn — and `latest_for_agent` hands
        //     that file to the *next* resume. Each restart truncated the
        //     thread a little further.
        //
        // Absorbing it makes `history` the single source of truth again, so
        // the request, the following turn and the durable record cannot
        // disagree. The bytes on the wire for *this* request are unchanged:
        // the prefix's entries are already provider-shaped `ChatMessage`s
        // and `Chat` is rendered verbatim, exactly as the splice did.
        //
        // The leading system message(s) built for this turn are dropped in
        // favour of the prefix's own, which is the whole point of resuming —
        // the stored prompt is the KV-cache prefix the backend has already
        // tokenised, and re-rendering it is what this path exists to avoid.
        //
        // `Chat` rather than the structured `AssistantToolCalls` /
        // `ToolResults` variants is deliberate: these entries have already
        // been through `bound_cached_transcript_messages`, which snaps the
        // window past an orphaned lead and strips a trailing unpaired
        // opener, so the cycle-pairing guard has nothing left to do. Lifting
        // them into the structured variants would mean re-deriving a pairing
        // that was already decided, with a second chance to get it wrong;
        // the native envelope they carry is parsed back into real tool calls
        // at the provider seam either way (see
        // `message_convert::parse_native_assistant_envelope`, which exists
        // for precisely this seeded-transcript case).
        // `bound_cached_transcript_messages` preserves a leading system
        // message only "when present" — a resumed transcript that was never
        // seeded with one (or was truncated ahead of it) hands back a
        // `cached` prefix with no system entry at all. Dropping this turn's
        // freshly-built system message(s) unconditionally would then leave
        // the absorbed history with none, not the cached one's — so only
        // drop them when the cached prefix actually supplies a replacement.
        let cached_has_system = matches!(cached.first(), Some(msg) if msg.role == "system");
        let tail: Vec<ConversationMessage> = self
            .history
            .drain(..)
            .skip_while(|entry| {
                cached_has_system
                    && matches!(entry, ConversationMessage::Chat(chat) if chat.role == "system")
            })
            .collect();
        self.history = cached
            .into_iter()
            .map(ConversationMessage::Chat)
            .chain(tail)
            .collect();
    }
}
