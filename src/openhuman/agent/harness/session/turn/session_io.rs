//! Session persistence: transcript loading, checkpointing, and background tasks.

use super::super::transcript;
use super::super::types::Agent;
use crate::openhuman::agent::harness;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::context::ARCHIVIST_EXTRACTION_PROMPT;
use crate::openhuman::inference::provider::{
    ChatMessage, ChatResponse, UsageInfo, AGENT_TURN_MAX_OUTPUT_TOKENS,
};
use futures::StreamExt;
use tinyagents::harness::model::{ModelRequest, ModelStreamItem};

impl Agent {
    // ─────────────────────────────────────────────────────────────────
    // Session transcript helpers
    // ─────────────────────────────────────────────────────────────────

    /// Try to load a previous session transcript for KV cache resume.
    ///
    /// Best-effort: failures are logged and silently ignored.
    pub(in super::super) fn try_load_session_transcript(&mut self) {
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
                        let loaded_count = session.messages.len();
                        log::info!("[transcript] loaded {} messages for resume", loaded_count);
                        // Best-effort store-backed shadow read (issue #4249,
                        // 04.2 phase 2). Observes + logs divergence only; the
                        // legacy transcript just loaded stays authoritative and
                        // is what feeds the resume below. Gated OFF by default.
                        self.maybe_shadow_read_session_store(&path, &session);
                        let bounded = self.bound_cached_transcript_messages(session.messages);
                        if bounded.len() < loaded_count {
                            log::warn!(
                                "[transcript] resume prefix trimmed from {} to {} messages (max_history_messages={})",
                                loaded_count,
                                bounded.len(),
                                self.config.max_history_messages
                            );
                        }
                        self.cached_transcript_messages = Some(bounded);
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

    /// Ask the provider for a short wrap-up message with native tools
    /// **disabled** so the model returns prose rather than another tool call.
    /// Buffers text deltas and forwards them to the progress sink (when
    /// attached) only after the completed response is validated as prose, so
    /// prompt-formatted tool calls cannot flash in the UI before fallback.
    ///
    /// `instruction` is the synthetic user turn that steers the wrap-up — the
    /// tool-call-cap checkpoint (`MAX_ITER_CHECKPOINT_INSTRUCTION`) or the
    /// no-final-answer close (`FINAL_ANSWER_INSTRUCTION`, issue #4093).
    ///
    /// Returns the summary text (empty when the provider call fails or
    /// yields nothing — the caller then falls back to a deterministic builder
    /// so the turn is never left without a well-formed assistant message,
    /// bug-report-2026-05-26 A1 / issue #4093) **paired with the provider
    /// usage** for this extra call, so the caller can fold it into the turn's
    /// cumulative token/cost accounting instead of silently dropping it.
    pub(super) async fn summarize_turn_wrapup(
        &self,
        base_messages: &[ChatMessage],
        effective_model: &str,
        iteration_for_stream: u32,
        instruction: &str,
    ) -> (String, Option<UsageInfo>) {
        let mut messages = base_messages.to_vec();
        messages.push(ChatMessage::user(instruction));

        let chat_model = match self
            .turn_model_source
            .build_summarizer(effective_model, self.temperature)
        {
            Ok(model) => model,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    model = effective_model,
                    "[agent::session] failed to build wrap-up model"
                );
                return (String::new(), None);
            }
        };
        let request = ModelRequest::new(
            messages
                .iter()
                .map(crate::openhuman::tinyagents::chat_message_to_message)
                .collect(),
        )
        .with_model(effective_model)
        .with_temperature(self.temperature)
        .with_max_tokens(AGENT_TURN_MAX_OUTPUT_TOKENS);
        let mut stream = match chat_model.stream(&(), request).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    model = effective_model,
                    "[agent::session] wrap-up stream failed to start"
                );
                return (String::new(), None);
            }
        };

        let mut streamed_text = String::new();
        let mut completed = None;
        while let Some(item) = stream.next().await {
            match item {
                ModelStreamItem::MessageDelta(delta) if !delta.text.is_empty() => {
                    streamed_text.push_str(&delta.text);
                }
                ModelStreamItem::Completed(response) => completed = Some(response),
                ModelStreamItem::Failed(error) => {
                    tracing::warn!(%error, "[agent::session] wrap-up stream failed");
                    return (String::new(), None);
                }
                ModelStreamItem::ProviderFailed(error) => {
                    tracing::warn!(error = %error.message, "[agent::session] wrap-up provider failed");
                    return (String::new(), None);
                }
                _ => {}
            }
        }
        let Some(response) = completed else {
            tracing::warn!("[agent::session] wrap-up stream ended without completion");
            return (String::new(), None);
        };
        let usage = crate::openhuman::tinyagents::model::usage_info_from_response(&response);
        let text = response.text();
        // Tools are disabled for wrap-up calls, but text-protocol models can
        // still ignore that instruction. Parse through the active dispatcher
        // so XML/JSON and registry-backed P-Format calls are all rejected. The
        // completed response and buffered deltas are checked independently:
        // some providers only preserve one of those representations.
        let parsed_call_count = |candidate: &str| {
            self.tool_dispatcher
                .parse_response(&ChatResponse {
                    text: Some(candidate.to_string()),
                    ..ChatResponse::default()
                })
                .1
                .len()
        };
        let parsed_response_calls = parsed_call_count(&text);
        let parsed_stream_calls = if streamed_text == text {
            parsed_response_calls
        } else {
            parsed_call_count(&streamed_text)
        };
        let native_tool_calls = response.tool_calls().len();
        let attempted_tool_call =
            native_tool_calls > 0 || parsed_response_calls > 0 || parsed_stream_calls > 0;
        let checkpoint = if attempted_tool_call {
            tracing::warn!(
                model = effective_model,
                iteration = iteration_for_stream,
                native_tool_calls,
                parsed_response_calls,
                parsed_stream_calls,
                "[agent::session] wrap-up attempted a tool call; using deterministic fallback"
            );
            String::new()
        } else if !text.trim().is_empty() {
            tracing::debug!(
                model = effective_model,
                iteration = iteration_for_stream,
                text_len = text.len(),
                "[agent::session] wrap-up selected completed response text"
            );
            text
        } else {
            tracing::debug!(
                model = effective_model,
                iteration = iteration_for_stream,
                text_len = streamed_text.len(),
                "[agent::session] wrap-up selected buffered stream text"
            );
            streamed_text
        };
        // Hold wrap-up deltas until protocol validation completes. Otherwise a
        // rejected XML/P-Format tool call briefly renders in chat even though
        // the caller subsequently replaces it with a deterministic fallback.
        if !checkpoint.is_empty() {
            if let Some(sink) = &self.on_progress {
                if let Err(error) = sink
                    .send(AgentProgress::TextDelta {
                        delta: checkpoint.clone(),
                        iteration: iteration_for_stream,
                    })
                    .await
                {
                    tracing::debug!(
                        model = effective_model,
                        iteration = iteration_for_stream,
                        error = %error,
                        "[agent::session] wrap-up progress sink closed"
                    );
                }
            }
        }
        tracing::debug!(
            model = effective_model,
            iteration = iteration_for_stream,
            checkpoint_len = checkpoint.len(),
            used_deterministic_fallback = attempted_tool_call,
            "[agent::session] wrap-up checkpoint selection complete"
        );
        (checkpoint, usage)
    }

    /// Enforce this agent's required structured-output contract on the turn's
    /// final `reply` (issue #4117), reconciling with streaming.
    ///
    /// When the contract is active and `reply` already carries a well-formed
    /// leading block, returns `None` (the caller keeps `reply` unchanged). When
    /// the block is omitted/invalid, the turn is **repaired** so downstream
    /// parsing/routing always receives one, and the repaired text is returned as
    /// `Some((repaired_text, usage))` so the caller can fold the extra call's
    /// usage into the turn accounting and rewrite the trailing assistant message.
    ///
    /// ## Streaming reconciliation (the #4387 / sanil-23 blocker)
    ///
    /// The original reply is streamed to the client as `TextDelta`s *before* this
    /// runs (via the harness event bridge, keyed on `on_progress`). #4387
    /// repaired with a `stream: None` re-prompt whose result then *replaced* the
    /// already-streamed reply — so the client watched one answer stream in and
    /// silently got a different one back. This implementation makes the repair
    /// **append-only**, so the returned/persisted reply is always exactly the
    /// concatenation of deltas the client saw — the live preview is a *prefix* of
    /// the final message, never contradicted:
    ///
    /// * **Streamed case** (`on_progress` attached — interactive/user-visible):
    ///   the corrective re-prompt runs *silently* (its raw output is never
    ///   streamed, so a malformed attempt is never shown), then the chosen
    ///   correction — the model's block if the concatenation validates, else a
    ///   deterministic [`synthesize_block`] — is streamed as a continuation and
    ///   appended after the original prose. Visible text == returned text, and
    ///   the appended block is the first JSON value (prose isn't JSON), so the
    ///   leading-position rule holds for the dominant omitted-block case.
    /// * **Non-streamed case** (`on_progress` absent — background/cron/routing,
    ///   the "non-user-visible" scope sanil-23 offered as the alternative): no
    ///   client saw anything, so there is nothing to stay consistent with. The
    ///   strict #4387 **replace** design applies — recover via re-prompt, else
    ///   prepend a synthesized block to the prose — guaranteeing strict leading
    ///   position.
    ///
    /// `iteration_for_stream` labels the streamed continuation so the UI groups
    /// it with the turn's other deltas.
    ///
    /// [`synthesize_block`]: harness::required_output::synthesize_block
    pub(in super::super) async fn enforce_required_output(
        &self,
        reply: &str,
        contract: &crate::openhuman::config::RequiredOutputContract,
        effective_model: &str,
        iteration_for_stream: u32,
    ) -> Option<(String, Option<UsageInfo>)> {
        use harness::required_output as ro;

        if ro::output_satisfies_contract(reply, contract) {
            return None;
        }
        log::warn!(
            "[agent_loop] required output block `{}` omitted from turn reply — repairing (streamed={})",
            contract.block_key,
            self.on_progress.is_some(),
        );

        // Corrective re-prompt (native tools disabled), seeded with the current
        // history — which already carries the omitting assistant reply, so the
        // model sees exactly what it left out. Run silently: we validate the
        // result before deciding whether/what to show, so a malformed attempt is
        // never streamed to the client.
        let mut base = self.tool_dispatcher.to_provider_messages(&self.history);
        base.push(ChatMessage::user(ro::repair_instruction(contract)));
        let (repair_text, usage) = self
            .reprompt_for_required_block(&base, effective_model)
            .await;
        let repair_text = repair_text.trim().to_string();

        // Non-streamed (replace) path: nothing was shown, so the repaired reply
        // can stand alone with a strictly-leading block.
        if self.on_progress.is_none() {
            if !repair_text.is_empty() && ro::output_satisfies_contract(&repair_text, contract) {
                log::info!(
                    "[agent_loop] required output block `{}` recovered via re-prompt (replace)",
                    contract.block_key
                );
                return Some((repair_text, usage));
            }
            log::warn!(
                "[agent_loop] required output block `{}` still missing after re-prompt — prepending a synthesized block",
                contract.block_key
            );
            let synthesized = format!("{}\n\n{}", ro::synthesize_block(contract), reply);
            return Some((synthesized, usage));
        }

        // Streamed (append) path: the original prose is already on the client, so
        // never replace it — append a streamed correction and return the exact
        // concatenation the client sees. Append ONLY the required block, not the
        // whole re-prompt reply: `repair_instruction` asks the model to re-emit the
        // block *and continue with its answer*, so appending the full reply would
        // duplicate the already-streamed answer after the block (#4900). Prefer the
        // model's own recovered block; fall back to a synthesized one otherwise.
        let correction = match ro::find_required_block(&repair_text, contract) {
            Some(block) => {
                log::info!(
                    "[agent_loop] required output block `{}` recovered via re-prompt (append)",
                    contract.block_key
                );
                serde_json::to_string(&block).unwrap_or_else(|_| ro::synthesize_block(contract))
            }
            None => {
                log::warn!(
                    "[agent_loop] required output block `{}` still missing after re-prompt — appending a synthesized block",
                    contract.block_key
                );
                ro::synthesize_block(contract)
            }
        };
        // Stream only the correction as a continuation so the live preview stays
        // a prefix of the final message (visible == returned).
        let continuation = format!("\n\n{correction}");
        self.stream_text_continuation(&continuation, iteration_for_stream)
            .await;
        let repaired = format!("{reply}{continuation}");
        if !ro::output_satisfies_contract(&repaired, contract) {
            // The only way to reach here is a reply that streamed a *non-conforming
            // JSON object first*: append-only can't restore strict leading
            // position without contradicting what the user already saw, so we
            // accept the trailing valid block and keep stream consistency (the
            // higher-priority invariant). Downstream still finds a conforming
            // block via `extract_json_values`; only strict ordering is relaxed,
            // and only for this pathological already-streamed case.
            log::warn!(
                "[agent_loop] required output block `{}` appended but not in leading position (streamed reply led with JSON) — accepting for stream consistency",
                contract.block_key
            );
        }
        Some((repaired, usage))
    }

    /// Ask the provider once for a reply that includes the required
    /// structured-output block, with native tools **disabled** and **without**
    /// forwarding any delta to the progress sink. Returns the parsed prose paired
    /// with the call's usage (empty text + `None` usage when the call fails or
    /// yields only tool-call markup).
    ///
    /// Unlike [`summarize_turn_wrapup`](Self::summarize_turn_wrapup) this is
    /// deliberately silent: `enforce_required_output` validates the result before
    /// deciding what (if anything) to stream, so a malformed repair attempt is
    /// never shown to the client.
    async fn reprompt_for_required_block(
        &self,
        base_messages: &[ChatMessage],
        effective_model: &str,
    ) -> (String, Option<UsageInfo>) {
        let chat_model = match self
            .turn_model_source
            .build_summarizer(effective_model, self.temperature)
        {
            Ok(model) => model,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    model = effective_model,
                    "[agent::session] failed to build required-output re-prompt model"
                );
                return (String::new(), None);
            }
        };
        let request = ModelRequest::new(
            base_messages
                .iter()
                .map(crate::openhuman::tinyagents::chat_message_to_message)
                .collect(),
        )
        .with_model(effective_model)
        .with_temperature(self.temperature)
        .with_max_tokens(AGENT_TURN_MAX_OUTPUT_TOKENS);
        let mut stream = match chat_model.stream(&(), request).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    model = effective_model,
                    "[agent::session] required-output re-prompt stream failed to start"
                );
                return (String::new(), None);
            }
        };

        let mut streamed_text = String::new();
        let mut completed = None;
        while let Some(item) = stream.next().await {
            match item {
                // Buffer only — deliberately NOT forwarded to `on_progress`.
                ModelStreamItem::MessageDelta(delta) if !delta.text.is_empty() => {
                    streamed_text.push_str(&delta.text);
                }
                ModelStreamItem::Completed(response) => completed = Some(response),
                ModelStreamItem::Failed(error) => {
                    tracing::warn!(%error, "[agent::session] required-output re-prompt stream failed");
                    return (String::new(), None);
                }
                ModelStreamItem::ProviderFailed(error) => {
                    tracing::warn!(error = %error.message, "[agent::session] required-output re-prompt provider failed");
                    return (String::new(), None);
                }
                _ => {}
            }
        }
        let Some(response) = completed else {
            tracing::warn!("[agent::session] required-output re-prompt ended without completion");
            return (String::new(), None);
        };
        let usage = crate::openhuman::tinyagents::model::usage_info_from_response(&response);
        let text = response.text();
        let out = if !text.trim().is_empty() {
            text
        } else if response.tool_calls().is_empty() {
            streamed_text
        } else {
            // Only tool-call markup was present — no usable prose.
            String::new()
        };
        (out, usage)
    }

    /// Emit `text` to the progress sink as a `TextDelta` continuation so a
    /// repaired required-output block appears in the UI appended after the
    /// already-streamed reply (issue #4117). No-op when no sink is attached.
    async fn stream_text_continuation(&self, text: &str, iteration: u32) {
        if text.is_empty() {
            return;
        }
        if let Some(sink) = &self.on_progress {
            let _ = sink
                .send(AgentProgress::TextDelta {
                    delta: text.to_string(),
                    iteration,
                })
                .await;
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
    pub(in super::super) fn persist_session_transcript(
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
            agent_id: Some(self.agent_definition_id.clone()),
            agent_type: Some(if self.session_parent_prefix.is_some() {
                "subagent".to_string()
            } else {
                "root".to_string()
            }),
            dispatcher: if self.tool_dispatcher.should_send_tool_specs() {
                "native".into()
            } else {
                "xml".into()
            },
            provider: turn_usage.map(|usage| usage.provider.clone()),
            model: turn_usage.map(|usage| usage.model.clone()),
            created: now.clone(),
            updated: now,
            turn_count: self.context.stats().session_memory_current_turn as usize,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            charged_amount_usd,
            thread_id: crate::openhuman::inference::provider::thread_context::current_thread_id(),
            task_id: None,
        };

        // Append-only write (Phase A, transcript-derived view): diff this turn's
        // logical messages against the previously-persisted set tracked in
        // memory. A pure extension appends only the new tail; a context
        // reduction appends a `compaction` record. The file is never rewritten,
        // so pre-compaction history survives on disk for the display projection.
        // `request_id` (web-chat only) stamps a turn boundary on each line.
        let prev = std::mem::take(&mut self.persisted_transcript_messages);
        let request_id = crate::openhuman::agent::turn_origin::current_request_id();
        match transcript::append_transcript_turn(
            path,
            &prev,
            messages,
            &meta,
            turn_usage,
            request_id.as_deref(),
        ) {
            Ok(()) => {
                // Track the new persisted logical set for the next turn's diff.
                self.persisted_transcript_messages = messages.to_vec();
                // Best-effort, non-fatal dual-write into the TinyAgents store.
                // Gated by the default-ON session dual-write flag
                // (`OPENHUMAN_SESSION_DUAL_WRITE` is a kill switch). Only runs
                // after the legacy JSONL append above succeeds; the legacy path
                // is primary and untouched (issue #4249, 04.1).
                self.maybe_dual_write_session_store(path, messages, &meta, turn_usage);
            }
            Err(err) => {
                // Restore the tracked state so a transient failure doesn't make
                // the next turn mis-diff (and spuriously emit a compaction).
                self.persisted_transcript_messages = prev;
                log::warn!(
                    "[transcript] failed to append transcript {}: {err}",
                    path.display()
                );
            }
        }
    }

    /// Mirror the just-persisted turn into the TinyAgents session store.
    ///
    /// Additive and gated on the default-ON session dual-write flag
    /// (`OPENHUMAN_SESSION_DUAL_WRITE` is a kill switch): when killed this is a
    /// cheap early return — no store handle is constructed and behavior is
    /// byte-identical to the legacy-only path. When on (the default), the
    /// store write is fired best-effort on a background task and any error is
    /// logged (`[session-store]`) and swallowed, so it can never fail or alter a
    /// chat turn. Records reuse the importer's normalization
    /// ([`crate::openhuman::session_import`]) so live and imported records are
    /// shape-identical. Reads stay 100% legacy until 04.2.
    fn maybe_dual_write_session_store(
        &self,
        path: &std::path::Path,
        messages: &[ChatMessage],
        meta: &transcript::TranscriptMeta,
        turn_usage: Option<&transcript::TurnUsage>,
    ) {
        use crate::openhuman::session_import::live;

        // Config flag (default ON) gates the mirror; the env kill switch can
        // still force it off. `self.config` is the effective per-agent config.
        if !live::dual_write_enabled(self.config.session_dual_write) {
            return;
        }

        // The session key is the transcript stem — the same value the importer
        // reads off the on-disk filename, so `stream_name`/descriptor keys match.
        let Some(stem) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            log::warn!(
                "[session-store] dual-write skipped: no file stem for {}",
                path.display()
            );
            return;
        };

        // Rebuild the exact message shape the importer sees after a JSONL
        // round-trip: attach this turn's usage to the last assistant message so
        // its `openhuman_turn_usage` metadata matches an imported record.
        let mut msgs = messages.to_vec();
        if let Some(usage) = turn_usage {
            if let Some(idx) = msgs.iter().rposition(|m| m.role == "assistant") {
                transcript::attach_turn_usage_metadata(&mut msgs[idx], usage);
            }
        }
        let session_transcript = transcript::SessionTranscript {
            meta: meta.clone(),
            messages: msgs,
        };
        let workspace = self.workspace_dir.clone();

        log::debug!(
            "[session-store] dual-write scheduled stem={stem} workspace={}",
            workspace.display()
        );
        tokio::spawn(async move {
            if let Err(err) = live::write_live_turn(&workspace, &stem, &session_transcript).await {
                log::warn!("[session-store] dual-write failed stem={stem}: {err:#}");
            }
        });
    }

    /// Store-backed **shadow read** of a just-loaded session transcript.
    ///
    /// Beside the legacy authoritative reader (`try_load_session_transcript`),
    /// read the same session back from the TinyAgents journal store, normalize
    /// both sides through the importer's `session_import::convert` machinery,
    /// compare, and log any divergence (`[session_shadow_read]`, issue #4249,
    /// 04.2 phase 2). Additive and gated on the default-**OFF**
    /// `AgentConfig::session_shadow_reads` flag
    /// (`OPENHUMAN_SESSION_SHADOW_READS` is a kill switch): when disabled this
    /// is a cheap early return.
    ///
    /// The legacy transcript stays authoritative — this only observes. The
    /// comparison runs on a spawned background task so it never slows the
    /// authoritative read, and every store-read error is treated as "no shadow
    /// available" (logged at debug), never propagated.
    fn maybe_shadow_read_session_store(
        &self,
        path: &std::path::Path,
        session: &transcript::SessionTranscript,
    ) {
        use crate::openhuman::session_import::live;

        // Config flag (default OFF) gates the shadow read; the env kill switch
        // can still force it off. `self.config` is the effective per-agent config.
        if !live::shadow_reads_enabled(self.config.session_shadow_reads) {
            return;
        }

        // Same session key the write side / importer use: the transcript stem.
        let Some(stem) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            log::debug!(
                "[session_shadow_read] skipped: no file stem for {}",
                path.display()
            );
            return;
        };

        let workspace = self.workspace_dir.clone();
        let transcript = session.clone();
        log::debug!(
            "[session_shadow_read] scheduled stem={stem} workspace={} legacy_messages={}",
            workspace.display(),
            transcript.messages.len()
        );
        tokio::spawn(async move {
            let _ = live::shadow_read_compare(&workspace, &stem, &transcript).await;
        });
    }

    // ─────────────────────────────────────────────────────────────────
    // Session-memory extraction.
    // ─────────────────────────────────────────────────────────────────

    /// Spawn a background archivist sub-agent to extract durable facts
    /// from the recent conversation into `MEMORY.md`. Fire-and-forget.
    ///
    /// Gated by [`context_pipeline::SessionMemoryState::should_extract`]
    /// — see its docs for the threshold invariants. Safe to call from
    /// inside `turn()` after the turn body has settled.
    pub(in super::super) async fn spawn_session_memory_extraction(
        &mut self,
        parent_ctx: harness::ParentExecutionContext,
    ) {
        // ── Flush the trailing open segment before the session winds down ──
        //
        // The ArchivistHook manages per-turn segment lifecycle but cannot
        // force-close the *last* open segment because there is no explicit
        // "session end" event in the turn loop. `spawn_session_memory_extraction`
        // is the closest available signal: it fires when the context manager
        // decides the session has accumulated enough material to archive.
        //
        // GUARANTEE: the flush is *awaited* here (not fire-and-forget) so
        // the trailing segment always receives its recap + embedding + tree
        // ingest before the function returns, even during runtime wind-down.
        // This honours the doc-comment guarantee on `flush_open_segment` in
        // `archivist.rs`. No deadlock risk: no mutex guard is held across
        // this await point.
        if let Some(ref archivist) = self.archivist_hook {
            let session_id = self.event_session_id.clone();
            log::debug!(
                "[archivist] awaiting flush_open_segment for session={session_id} at session wind-down"
            );
            archivist.flush_open_segment(&session_id).await;
        }

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

    /// Spawn a background task that ingests the current session
    /// transcript into the conversational-memory store.
    ///
    /// Issue #1399: complements `spawn_session_memory_extraction`. The
    /// archivist path writes dense bullets into `MEMORY.md`; this path
    /// extracts importance-tagged, provenance-bearing memories via the
    /// heuristic [`crate::openhuman::learning::transcript_ingest`]
    /// pipeline. The two are deliberately independent so the prompt
    /// retrieval layer can pull from `conversation_memory` without
    /// needing the archivist's extraction to have fired this session.
    ///
    /// Fire-and-forget: failures are logged, never propagated.
    pub(in super::super) fn spawn_transcript_ingestion(&self) {
        let Some(path) = self.session_transcript_path.clone() else {
            log::debug!("[transcript_ingest] no session transcript path yet — skipping spawn");
            return;
        };
        let memory = std::sync::Arc::clone(&self.memory);

        tokio::spawn(async move {
            match crate::openhuman::learning::transcript_ingest::ingest_transcript_path(
                memory.as_ref(),
                &path,
            )
            .await
            {
                Ok(report) => tracing::info!(
                    transcript = %path.display(),
                    extracted = report.extracted,
                    stored = report.stored,
                    deduped = report.deduped,
                    reflections_stored = report.reflections_stored,
                    "[transcript_ingest] background ingest complete"
                ),
                Err(err) => tracing::warn!(
                    transcript = %path.display(),
                    error = %err,
                    "[transcript_ingest] background ingest failed — will retry next threshold window"
                ),
            }
        });
    }
}
