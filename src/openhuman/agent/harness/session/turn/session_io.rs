//! Session persistence: transcript loading, checkpointing, and background tasks.

use super::super::transcript;
use super::super::types::Agent;
use crate::openhuman::agent::harness;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::context::ARCHIVIST_EXTRACTION_PROMPT;
use crate::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ProviderDelta, UsageInfo, AGENT_TURN_MAX_OUTPUT_TOKENS,
};

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
    /// Streams text deltas to the progress sink (when attached) so the summary
    /// appears in the UI like any other reply.
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

        // Mirror the main loop's streaming sink so the checkpoint renders
        // incrementally. Only text deltas are relevant here (tools are
        // disabled for this call).
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
                                iteration: iteration_for_stream,
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
                    // Reservation-pricing pre-flight budget cap (TAURI-RUST-C62).
                    max_tokens: Some(AGENT_TURN_MAX_OUTPUT_TOKENS),
                },
                effective_model,
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
                // Strip any stray tool-call XML a text-mode model may have
                // emitted; keep only the prose.
                let (text, calls) = self.tool_dispatcher.parse_response(&resp);
                let checkpoint = if !text.trim().is_empty() {
                    text
                } else if calls.is_empty() {
                    // No tool-call markup was present, so the raw text (if
                    // any) is genuine prose — safe to use.
                    resp.text.unwrap_or_default()
                } else {
                    // `parse_response` stripped tool-call markup and left no
                    // prose. Do NOT re-emit `resp.text` here: it would persist
                    // the raw `<tool_call>…` markup verbatim as the checkpoint.
                    // Return empty so the caller uses the deterministic
                    // fallback instead (bug-report-2026-05-26 A1).
                    String::new()
                };
                (checkpoint, usage)
            }
            Err(e) => {
                log::warn!("[agent_loop] checkpoint summary call failed: {e:#}");
                (String::new(), None)
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

        match transcript::write_transcript(path, messages, &meta, turn_usage) {
            Ok(()) => {
                // Best-effort, non-fatal dual-write into the TinyAgents store.
                // Gated by the default-ON session dual-write flag
                // (`OPENHUMAN_SESSION_DUAL_WRITE` is a kill switch). Only runs
                // after the legacy JSONL append above succeeds; the legacy path
                // is primary and untouched (issue #4249, 04.1).
                self.maybe_dual_write_session_store(path, messages, &meta, turn_usage);
            }
            Err(err) => {
                log::warn!(
                    "[transcript] failed to write transcript {}: {err}",
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
