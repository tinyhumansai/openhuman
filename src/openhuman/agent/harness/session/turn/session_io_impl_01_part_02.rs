
impl Agent {
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
        let now = chrono::Utc::now().to_rfc3339();

        // This turn's `_meta`. Built before the path/handle binding below so it
        // can double as the handle's `seed_meta`; it depends only on agent
        // state and this turn's figures, never on the resolved path.
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
            thread_id: crate::openhuman::agent::tinyagents::thread_context::current_thread_id(),
            task_id: None,
        };

        // Bind the write seam on first write. The stem is
        // `{parent_prefix}__{session_key}` for sub-agents (producing a
        // flat hierarchical filename) or just `{session_key}` for a
        // root session. Prefix chaining is already done by the
        // sub-agent runner when it populates `session_parent_prefix`.
        //
        // Path resolution is the locator's job, not this function's: it used to
        // be duplicated here (a `resolve_keyed_transcript_path_in_dir` call
        // that had to stay in lockstep with the identical one inside the
        // handle's constructor). One call now yields both, and
        // `session_transcript_path` is simply the bound handle's own path — so
        // they cannot drift. The seed meta only matters when the file is absent
        // and the caller supplies none; the turn path always passes its own
        // freshly-computed meta below, so it never takes effect.
        if self.session_transcript_path.is_none() {
            let stem = match &self.session_parent_prefix {
                Some(prefix) => format!("{}__{}", prefix, self.session_key),
                None => self.session_key.clone(),
            };
            match self.session_locator().open_stem(&stem, meta.clone()) {
                Ok(history) => {
                    log::info!(
                        "[transcript] new session transcript path={}",
                        history.path().display()
                    );
                    self.session_transcript_path = Some(history.path().to_path_buf());
                    self.session_history = Some(history);
                }
                Err(err) => {
                    log::warn!("[transcript] failed to bind session history: {err:#}");
                    self.session_transcript_path = None;
                    return;
                }
            }
        }

        let path = self.session_transcript_path.clone().unwrap();
        // Cloned out of `self` before the write so the later `&mut self`
        // dual-write does not conflict with a live borrow of the handle.
        let Some(history) = self.session_history.clone() else {
            log::warn!("[transcript] no session history bound; skipping append");
            return;
        };

        // Append-only write (Phase A, transcript-derived view): diff this turn's
        // logical messages against the previously-persisted set tracked in
        // memory. A pure extension appends only the new tail; a context
        // reduction appends a `compaction` record. The file is never rewritten,
        // so pre-compaction history survives on disk for the display projection.
        // `request_id` (web-chat only) stamps a turn boundary on each line.
        //
        // This goes through `SessionHistory::append_turn` rather than
        // `transcript::append_transcript_turn` directly (S4). The handle is a
        // pure forwarder of exactly these six values — it must be, because the
        // crate's `ChatHistory` methods carry no channel for `request_id`,
        // `turn_usage` or a caller-computed `TranscriptMeta`, and dropping any
        // of them silently guts the transcript-view projection. See the header
        // of `transcript_history.rs` for the full argument.
        let prev = std::mem::take(&mut self.persisted_transcript_messages);
        let request_id = crate::openhuman::agent::turn_origin::current_request_id();
        match history.append_turn(TranscriptTurn {
            prev: &prev,
            next: messages,
            meta: &meta,
            turn_usage,
            request_id: request_id.as_deref(),
        }) {
            Ok(()) => {
                // Track the new persisted logical set for the next turn's diff.
                self.persisted_transcript_messages = messages.to_vec();
                // Best-effort, non-fatal dual-write into the TinyAgents store.
                // Gated by the default-ON session dual-write flag
                // (`OPENHUMAN_SESSION_DUAL_WRITE` is a kill switch). Only runs
                // after the legacy JSONL append above succeeds; the legacy path
                // is primary and untouched (issue #4249, 04.1).
                self.maybe_dual_write_session_store(&path);
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
    /// ([`crate::openhuman::agent::session_import`]) so live and imported records are
    /// shape-identical. Reads stay 100% legacy until 04.2.
    fn maybe_dual_write_session_store(&self, path: &std::path::Path) {
        use crate::openhuman::agent::session_import::live;

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

        let workspace = self.workspace_dir.clone();
        let path = path.to_path_buf();

        log::debug!(
            "[session-store] dual-write scheduled stem={stem} workspace={}",
            workspace.display()
        );
        tokio::spawn(async move {
            // Mirror the exact transcript the shadow reader compares against.
            // `shadow_read_compare` normalizes `read_transcript(path)` — the
            // legacy JSONL after its write→read round-trip — so build the store
            // record from that same read rather than from the in-memory turn.
            // Reconstructing it by hand let sidecar `extra_metadata` (turn-usage,
            // tool-failure marker, reasoning) diverge on resumed sessions even
            // though every message body, id and role matched, which is what the
            // parity soak flagged as `[session_shadow_read] DIVERGENCE` (#6149).
            // Read-and-mirror runs here on the same best-effort background task
            // that already fully rewrites the journal stream. The read itself is
            // synchronous — `fs::read_to_string` plus a JSON parse over the whole
            // append-only JSONL, which grows for the life of the session — so it
            // goes to the blocking pool instead of stalling a Tokio worker for
            // the length of the file. On a read error, skip the mirror for this
            // turn (legacy stays authoritative).
            let read_path = path.clone();
            let read_back =
                tokio::task::spawn_blocking(move || transcript::read_transcript(&read_path)).await;
            let session_transcript = match read_back {
                Ok(Ok(t)) => t,
                Ok(Err(err)) => {
                    log::debug!(
                        "[session-store] dual-write skipped: transcript read-back failed for {}: {err:#}",
                        path.display()
                    );
                    return;
                }
                Err(err) => {
                    log::debug!(
                        "[session-store] dual-write skipped: transcript read-back task failed for {}: {err}",
                        path.display()
                    );
                    return;
                }
            };
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
        use crate::openhuman::agent::session_import::live;

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
    /// heuristic [`crate::openhuman::agent::learning::transcript_ingest`]
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
            match crate::openhuman::agent::learning::transcript_ingest::ingest_transcript_path(
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
