
impl Agent {

    // ─────────────────────────────────────────────────────────────────
    // Session transcript helpers
    // ─────────────────────────────────────────────────────────────────

    /// Try to load a previous session transcript for KV cache resume.
    ///
    /// Best-effort: failures are logged and silently ignored.
    ///
    /// # How this reaches the transcript (S4)
    ///
    /// Both halves of the turn path now go through the seam: writes through
    /// [`SessionHistory::append_turn`][super::super::transcript_history::SessionHistory::append_turn],
    /// reads through
    /// [`SessionHistoryLocator`][super::super::transcript_history::SessionHistoryLocator]
    /// + [`SessionTranscriptRead::read_session`][super::super::transcript_history::SessionTranscriptRead::read_session].
    ///
    /// The read is **not** `ChatHistory::messages()`, and that is settled, not
    /// pending: `messages()` returns `Vec<Message>`, and converting back with
    /// `message_to_chat_message` flattens `Assistant.tool_calls` into plain
    /// text. That is precisely what
    /// [`bound_cached_transcript_messages`][Agent::bound_cached_transcript_messages]'
    /// TAURI-RUST-7 trailing strip inspects, and re-sending a flattened prefix
    /// to a native provider is the `400 assistant message with 'tool_calls'
    /// must be followed by tool messages` failure that strip exists to prevent.
    /// `read_session` returns the whole [`SessionTranscript`][super::super::transcript::SessionTranscript]
    /// instead — the same struct the free function returns, from the same
    /// `read_transcript` call — so compaction replay, `interrupted: true`
    /// partial skipping and the `_meta` header
    /// [`maybe_shadow_read_session_store`][Agent::maybe_shadow_read_session_store]
    /// needs all survive by construction.
    ///
    /// Discovery lives on the locator because it is a *lookup*, not a read:
    /// this function's key is `(workspace, session_raw_subdir, agent name)`
    /// (newest match, with a legacy `session_raw/DDMMYYYY/` fallback) and the
    /// cold-boot sibling
    /// [`seed_resume_from_thread_transcript`][Agent::seed_resume_from_thread_transcript]
    /// keys off `_meta.thread_id`. Neither is a stem, and `ChatHistory` has no
    /// discovery concept at all.
    pub(in super::super) fn try_load_session_transcript(&mut self) {
        let Some(handle) = self
            .session_locator()
            .latest_for_agent(&self.agent_definition_name)
        else {
            log::debug!(
                "[transcript] no previous transcript found for agent={}",
                self.agent_definition_name
            );
            return;
        };
        let path = handle.path().to_path_buf();
        log::info!(
            "[transcript] found previous transcript path={}",
            path.display()
        );
        match handle.read_session() {
            // `Ok(None)` (file vanished between discovery and read) folds into
            // the same "nothing to resume from" branch as an empty transcript,
            // so the caller's behaviour is unchanged either way.
            Ok(None) => {
                log::debug!("[transcript] previous transcript is empty — skipping resume");
            }
            Ok(Some(session)) => {
                if session.messages.is_empty() {
                    log::debug!("[transcript] previous transcript is empty — skipping resume");
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

    /// The transcript locator for this session — the injected one, or a
    /// [`FileTranscriptLocator`] built from the agent's **current** workspace.
    ///
    /// Built per call rather than cached: `workspace_dir` and
    /// `session_raw_subdir` are reassignable after `build()` (tests do exactly
    /// that), and a locator frozen at build time would silently keep resolving
    /// against the directory the agent no longer uses. The construction is two
    /// clones of small strings — cheaper than the `read_dir` it precedes.
    pub(in super::super) fn session_locator(&self) -> std::sync::Arc<dyn SessionHistoryLocator> {
        match &self.session_history_locator {
            Some(locator) => locator.clone(),
            None => std::sync::Arc::new(FileTranscriptLocator::new(
                self.workspace_dir.clone(),
                self.session_raw_subdir.clone(),
            )),
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
                .map(crate::openhuman::agent::tinyagents::chat_message_to_message)
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
        let usage = crate::openhuman::agent::tinyagents::model::usage_info_from_response(&response);
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
        contract: &tinyagents_harness::config::RequiredOutput,
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
                .map(crate::openhuman::agent::tinyagents::chat_message_to_message)
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
        let usage = crate::openhuman::agent::tinyagents::model::usage_info_from_response(&response);
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
}
