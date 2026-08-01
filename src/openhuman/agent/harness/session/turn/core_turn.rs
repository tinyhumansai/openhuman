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
    /// 3. **Context Injection**: Enriches the user message with per-turn context
    ///    such as situational preferences, the thread goal, and active sub-agents.
    ///    Broad memory recall is available to the model on demand instead.
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
        // Consume any per-turn overrides the caller set for THIS turn (#1725).
        // Taking them resets the field to the default, so they apply to exactly
        // one turn — a chat/small-talk turn runs tool-less, memory-less and
        // goal-less without permanently altering the agent, and the following
        // turn is back to full-agentic behaviour. Callers that never opt in get
        // `TurnOverrides::default()` (all-false) and the unchanged path.
        let turn_overrides = std::mem::take(&mut self.pending_turn_overrides);
        if turn_overrides != super::super::types::TurnOverrides::default() {
            log::info!(
                "[agent_loop] per-turn overrides active: suppress_active_goal={} suppress_tools={} suppress_memory_agent={}",
                turn_overrides.suppress_active_goal,
                turn_overrides.suppress_tools,
                turn_overrides.suppress_memory_agent
            );
        }
        self.emit_progress(AgentProgress::TurnStarted).await;
        log::info!("[agent] turn started — awaiting user message processing");
        log::info!(
            "[agent_loop] turn start message_chars={} history_len={} max_tool_iterations={}",
            user_message.chars().count(),
            self.history.len(),
            self.config.max_tool_iterations
        );
        self.ensure_composio_integrations_listener();
        // Arm the installed-skills listener at turn start (not lazily inside
        // `drain_skill_events`, which is only reached after the first turn) —
        // broadcast subscriptions are not retroactive, so a skill installed
        // during turn 1 would otherwise be missed until a later subscribe.
        self.ensure_skill_events_listener();
        // ── Session transcript resume ─────────────────────────────────
        // On a fresh session (empty history), look for a previous
        // transcript to pre-populate the exact provider messages for
        // KV cache prefix reuse.
        if self.history.is_empty()
            && self.cached_transcript_messages.is_none()
            && !turn_overrides.suppress_transcript_autoload
        {
            self.try_load_session_transcript();
        }

        if self.history.is_empty() {
            // Learned context is only baked into the system prompt on the
            // very first turn — once the history is non-empty we reuse the
            // stored prompt verbatim to preserve the KV-cache prefix the
            // inference backend has already tokenised. Fetching it later
            // would just burn memory-store reads on data we throw away.
            if !self.connected_integrations_initialized {
                self.fetch_connected_integrations().await;
                // Sessions born without a cached Composio view still need
                // a one-shot delegation-surface reconcile before the system
                // prompt is frozen. The shared-Arc failure path returns
                // `false`, but on turn 1 the Arc should still be uniquely
                // owned; a `false` return here indicates a programmer error
                // and the warn-level log inside the helper already surfaces
                // it, so we keep the existing best-effort contract.
                let _ = self.refresh_delegation_tools();
            }
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
            //
            // AGENTS.md instruction layers are also user/project-controlled and
            // can land in the prompt even when PROFILE/MEMORY are both omitted
            // (common for narrow specialists), so treat their presence as a
            // redaction trigger too — otherwise the full-body path would print
            // raw AGENTS.md contents verbatim.
            let contains_agents_md =
                rendered_prompt.contains("## Project instructions (AGENTS.md)");
            if self.omit_profile && self.omit_memory_md && !contains_agents_md {
                log::debug!("[agent_loop] system prompt body:\n{}", rendered_prompt);
            } else {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                rendered_prompt.hash(&mut hasher);
                log::debug!(
                    "[agent_loop] system prompt body redacted (contains PROFILE/MEMORY/AGENTS.md): chars={} hash={:016x}",
                    rendered_prompt.chars().count(),
                    hasher.finish()
                );
            }
            self.history
                .push(ConversationMessage::Chat(ChatMessage::system(
                    rendered_prompt,
                )));
            // Seed the per-turn mid-session refresh baseline with the
            // hash of whatever Composio actually returned just now.
            // Subsequent turns short-circuit unless this hash changes.
            self.last_seen_integrations_hash =
                crate::openhuman::integrations::composio::connected_set_hash(
                    &self.connected_integrations,
                );
            // Seed the announced set with the startup connected toolkits so
            // only genuinely-new mid-session connects get announced later.
            self.announced_integrations = self
                .connected_integrations
                .iter()
                .map(|i| i.toolkit.clone())
                .collect();
            // MCP analogue: seed the announced MCP set with the servers already
            // connected at startup. Those are already in the (turn-1) system
            // prompt's `## Connected MCP Servers` block, so only servers that
            // connect *mid-session* should later be announced on the user turn.
            self.announced_mcp_servers =
                crate::openhuman::mcp::registry::connections::connected_overview()
                    .await
                    .into_iter()
                    .map(|s| s.qualified_name)
                    .collect();
        } else {
            // Deliberately do NOT rebuild the system prompt on subsequent
            // turns. The rendered prompt is the KV-cache prefix the inference
            // backend has already tokenised; replacing its bytes (even
            // cosmetically) forces the backend to re-prefill from scratch.
            //
            // Dynamic turn-to-turn context rides on the user message assembled
            // below (`context`) — that is where anything varying between turns
            // belongs. Broad memory recall is not injected; the model calls the
            // memory tools when it needs stored context.
            //
            // *** Mid-session schema-only refresh ***
            //
            // The system prompt stays frozen, but the function-calling
            // schema (the `tools` field in the provider request) is sent
            // fresh on every API call — it's not part of the KV-cache
            // prefix. So we *can* react to Composio connect/disconnect
            // events mid-session by re-synthesising the `delegate_<toolkit>`
            // surface on `self.tools` / `self.tool_specs` and letting
            // the next provider call carry the new schema. KV cache stays
            // intact; the system prompt's `## Connected Integrations`
            // block goes mildly stale until the next session, but the
            // schema is the source of truth the model actually routes
            // against.
            //
            // The signal we react to is the process-wide
            // [`crate::openhuman::integrations::composio::INTEGRATIONS_CACHE`], kept
            // current by (a) the desktop UI's 5 s
            // `composio_list_connections` poll, (b) the post-OAuth
            // `ComposioConnectionCreatedSubscriber` invalidation, and
            // (c) the 60 s TTL fallback. We read it via the read-only
            // [`crate::openhuman::integrations::composio::cached_active_integrations`]
            // helper — never trigger a backend fetch ourselves, never
            // block on a writer.
            // Session agents built through `from_config_*` carry their
            // runtime `Config` snapshot directly, so this read avoids the
            // old `Config::load_or_init()` round-trip on every turn.
            //
            let _ = self.refresh_delegation_tools_from_cached_integrations("turn-boundary");
            // Same idea for installed skills. The system-prompt
            // `## Installed Skills` block is frozen at turn 1 for KV-cache
            // stability (history is non-empty here, so it is never rebuilt
            // mid-session), so — exactly like the MCP mechanism — the
            // user-turn announcement below is what surfaces a mid-session
            // install to the model. `refresh_workflows` updates the tracked
            // set (so the next refresh diffs correctly and a future fresh
            // session renders the new catalogue) and parks the announcement.
            // Event-driven (mirror of the composio path): only re-scan disk
            // when a `WorkflowsChanged` event was published since the last
            // turn — no per-turn filesystem walk on the steady-state hot path.
            if self.drain_skill_events() {
                let _ = self.refresh_workflows("event");
            }
            // Cache empty/expired or config unavailable => no signal.
            // We leave the current tool surface alone and pick up any
            // real change on the next turn after the UI's 5 s poll has
            // repopulated [`INTEGRATIONS_CACHE`].

            // MCP mid-session connect surfacing — the analogue of the Composio
            // path above. `use_mcp_server` is a single static delegate (no
            // per-server schema to refresh), so the whole mechanism is: diff
            // the live in-process connection map against what we've already
            // announced and queue a one-shot note for any newly-connected
            // server onto the next user message. The map is in-process (no
            // network, unlike Composio's cache), so reading it every turn is
            // cheap. Like the Composio block, the frozen `## Connected MCP
            // Servers` system-prompt section stays as the turn-1 snapshot.
            let connected_mcp: Vec<String> =
                crate::openhuman::mcp::registry::connections::connected_overview()
                    .await
                    .into_iter()
                    .map(|s| s.qualified_name)
                    .collect();
            for qn in newly_connected_slugs(&connected_mcp, &mut self.announced_mcp_servers) {
                if !self.pending_mcp_announcement.contains(&qn) {
                    self.pending_mcp_announcement.push(qn);
                }
            }

            log::trace!(
                "[agent_loop] system prompt reused (history_len={}) — KV cache prefix preserved",
                self.history.len()
            );
        }

        // `auto_save` says the workspace keeps its chat in memory; the origin says
        // whether this turn is chat at all. An internal agent is built from the
        // same config (`Agent::from_config_for_agent`), so it inherits the flag —
        // and its "user message" is the prompt the host wrote for it, not
        // anything the user said. Live, that stored `memory_goals::enrich`'s
        // prompt as a `Conversation` document keyed `user_msg:…`, where it then
        // competed for slots in every later recall (#5312). Gating here rather
        // than at each caller keeps a new internal agent from having to remember
        // to opt out, which is a thing nobody notices forgetting.
        //
        // Upstream of the same-session exclusion filter `main` added alongside
        // `CONVERSATION_RAW_NAMESPACE`: that filter stops this document echoing
        // back inside the turn that wrote it, but a host-written prompt stored
        // here still surfaces in a *later* session's recall. This gate is what
        // keeps it from being written at all.
        if self.auto_save && crate::openhuman::agent::turn_origin::current_is_user_authored() {
            // Fire-and-forget: persisting the user message to the memory store
            // does an embedding round-trip (Voyage) + memory-tree write that the
            // in-flight turn never reads back. Awaiting it delayed the start of
            // *every* turn before recall/LLM began, so spawn it and let the chat
            // continue immediately.
            //
            // Use a UNIQUE per-message key: the old fixed `"user_msg"` key
            // upserts a single document (`upsert_document` keys by namespace+key),
            // so concurrent turns would race on — and overwrite — one shared slot.
            // A unique key makes each user message its own conversation document,
            // which both removes the race and stops the autosave from only ever
            // retaining the latest message.
            let memory = self.memory.clone();
            let user_msg = user_message.to_string();
            let autosave_key = format!("user_msg:{}", uuid::Uuid::new_v4());
            let chars = user_msg.chars().count();
            // Captured *before* `tokio::spawn` — the ambient thread id is a
            // `tokio::task_local` (see `tinyagents::thread_context`)
            // and does not propagate into a spawned task, so it must be read
            // on this (still-scoped) task and moved in explicitly. Tagging
            // this document with the live chat thread id is what lets the
            // same-session exclusion filter (`UnifiedMemory::recall` /
            // `memory_hybrid_search`) recognize and drop it later this same
            // turn, so the agent's own on-demand memory search doesn't echo
            // its own triggering request back as a "relevant" result.
            let session_id_for_autosave =
                crate::openhuman::agent::tinyagents::thread_context::current_thread_id();
            log::debug!(
                "[agent_autosave] enqueue user-message store key={autosave_key} chars={chars} \
                 session_id={}",
                session_id_for_autosave.as_deref().unwrap_or("<none>")
            );
            tokio::spawn(async move {
                match memory
                    .store(
                        crate::openhuman::agent::learning::transcript_ingest::CONVERSATION_RAW_NAMESPACE,
                        &autosave_key,
                        &user_msg,
                        MemoryCategory::Conversation,
                        session_id_for_autosave.as_deref(),
                    )
                    .await
                {
                    Ok(()) => log::debug!(
                        "[agent_autosave] stored user-message key={autosave_key} chars={chars}"
                    ),
                    Err(err) => log::warn!(
                        "[agent_autosave] user-message memory autosave failed key={autosave_key} err={err}"
                    ),
                }
            });
        }

        log::info!("[agent] spawning UI-only citation collection for user message");
        const MEMORY_CITATION_LIMIT: usize = 5;
        const MEMORY_CITATION_MIN_RELEVANCE: f64 = 0.4;
        // Spawned, not awaited: see `Agent::pending_citations`. The result is
        // UI-only, so the turn must not wait for it before calling the model.
        self.last_turn_citations.clear();
        if let Some(previous) = self.pending_citations.take() {
            // A turn that never had its citations collected leaves a task
            // behind; abort it rather than letting a stale recall outlive the
            // turn it belonged to.
            previous.abort();
        }
        let citation_memory = self.memory.clone();
        let citation_query = user_message.to_string();
        self.pending_citations = Some(tokio::spawn(async move {
            match collect_recall_citations(
                citation_memory.as_ref(),
                &citation_query,
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
                    citations
                }
                Err(_err) => {
                    // Recall errors may include the user-authored query. Keep
                    // warning logs free of raw external content.
                    log::warn!("[agent_loop] memory citation collection failed");
                    Vec::new()
                }
            }
        }));
        // No per-turn memory-context block is assembled here any more.
        //
        // `memory_loader.load_context()` used to prepend `[User working
        // memory]`, `[Prior conversations]` and `[Cross-chat context]` to every
        // user message. It cost two full scans of the `global` namespace per
        // turn — every document and every vector chunk, decoded and scored — to
        // contribute at most nine lines, and the cost grew with everything the
        // user had ever said. Benchmarked at ~10k memories it was the dominant
        // per-turn cost by a wide margin, and the `[User working memory]` arm
        // in particular scanned the whole namespace only to filter the results
        // down to a `working.user.` key prefix, so it returned nothing at all
        // once ordinary chat crowded the ranking.
        //
        // Memory is still available to the agent — `memory_recall` and the rest
        // of the memory tools are unchanged, so the model fetches what it needs
        // when it needs it, rather than every turn paying for a broad guess.
        let mut context = String::new();

        // ── Lane B: situational preferences (every turn) ─────────────────────
        // Recall topic-scoped preferences semantically relevant to THIS message
        // (model-aware embeddings, gated by vector similarity) and inject them
        // under a banner. Runs every turn — unlike the first-turn-gated tree/STM
        // blocks above — because the query changes per message; it rides the
        // per-turn context that's prepended to the user message (no KV-cache
        // cost). An unrelated message clears the similarity gate to nothing, so
        // no block is injected.
        {
            let situational =
                crate::openhuman::memory::preferences::recall_situational_preferences_on(
                    &self.memory,
                    user_message,
                )
                .await;
            if !situational.is_empty() {
                log::info!(
                    "[pref_recall] situational block injected: {} item(s)",
                    situational.len()
                );
                context.push_str("## Relevant preferences for this message\n\n");
                for pref in &situational {
                    context.push_str("- ");
                    context.push_str(pref.trim());
                    context.push('\n');
                }
                context.push('\n');
            } else {
                log::debug!("[pref_recall] no situational preference relevant to this message");
            }
        }

        // ── Thread goal (Codex-style per-thread completion contract) ─────────
        // Load this thread's durable goal once per turn and prepend a compact
        // [active_goal] block so the objective + live status/budget steer the
        // turn. Rides the per-turn context (NOT the cached system-prompt prefix)
        // so edits take effect immediately. `active_goal` is reused below to arm
        // the budget stop hook around the engine call.
        // Capture the workspace path for the budget stop hook built after the
        // `turn_body` coroutine (which borrows `&mut self`) is constructed.
        let goal_workspace_dir = self.workspace_dir.clone();
        let active_goal = if turn_overrides.suppress_active_goal {
            // Chat / small-talk turn: do NOT load, auto-resume, inject, or
            // budget-arm this thread's goal. A goal an earlier task left
            // uncompleted must not steer an unrelated greeting (#1725, the
            // "stale goal replays every turn" half of the context leak).
            log::debug!(
                "[thread_goals] active_goal suppressed for this turn (chat/small-talk override)"
            );
            None
        } else {
            let loaded = crate::openhuman::threads::goals::runtime::load_for_current_thread(
                &self.workspace_dir,
            )
            .await;
            // Thread-resume semantics: the user re-engaging a thread reactivates a
            // paused goal (Codex's ThreadResumed). Best-effort; on failure keep
            // the loaded (paused) goal so we still surface it.
            match loaded {
                Some(goal)
                    if matches!(
                        goal.status,
                        crate::openhuman::threads::goals::ThreadGoalStatus::Paused
                    ) =>
                {
                    crate::openhuman::threads::goals::runtime::resume_for_current_thread(
                        &self.workspace_dir,
                    )
                    .await
                    .unwrap_or(Some(goal))
                }
                other => other,
            }
        };
        if let Some(ref goal) = active_goal {
            if let Some(block) = tinyagents_graph::goals::active_goal_context_block(goal) {
                log::info!(
                    "[thread_goals] injecting active_goal block status={} budget={:?} ({} chars)",
                    goal.status.as_str(),
                    goal.token_budget,
                    block.chars().count()
                );
                context.push_str(&block);
            }
        }

        // ── Active sub-agents (ambient fleet awareness) ──────────────────────
        // When this agent has async/parallel workers registered under its own
        // session, prepend a compact `[active_subagents]` roster (agent type,
        // subagent_session_id, live status) so it tracks the fleet from the turn
        // context instead of relying on remembered `[async_subagent_ref]` blocks
        // that may have scrolled away. Children register under the parent's
        // `session_id`, which is this agent's `event_session_id` (see
        // `build_parent_execution_context`). Gated on presence: agents that never
        // spawn get an empty block and no injection. Rides per-turn context (like
        // the goal block) so status is always live.
        if let Some(block) =
            crate::openhuman::agent::orchestration::running_subagents::active_subagents_context_block(
                &self.event_session_id,
                &self.workspace_dir,
            )
        {
            log::info!(
                "[running_subagents] injecting active_subagents block session={} ({} chars)",
                self.event_session_id,
                block.chars().count()
            );
            context.push_str(&block);
        }

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

        let enriched = self
            .inject_agent_experience_context(user_message, enriched)
            .await;

        // ── SKILL.md body injection: REMOVED (was #781) ──────────────
        // We used to keyword-match installed skills against the user message
        // and prepend their full SKILL.md bodies onto the user turn. That
        // brittle name/description/tag match fired unintentionally and — by
        // baking the body into the stored user message — left full skill text
        // permanently in chat history (microcompact only clears tool results,
        // not user messages).
        //
        // Skills are now surfaced via the compact `## Installed Skills`
        // catalog in the orchestrator prompt and executed via `run_skill`,
        // which loads and follows the SKILL.md inside an isolated worker, so
        // the full body never enters this conversation. `self.workflows` still
        // feeds the catalog through `PromptContext`.

        // Consume any one-shot mid-session connect announcement parked by
        // `refresh_delegation_tools_from_cached_integrations`. It rides on the
        // user turn (NOT a system message — `trim_history` hoists system
        // messages to the front and would bust the KV-cache prefix) and
        // `.take()` clears it so it fires exactly once.
        let pending_slugs = std::mem::take(&mut self.pending_integration_announcement);
        let enriched = match integration_announcement_note(&pending_slugs) {
            Some(note) => format!("{note}\n\n{enriched}"),
            None => enriched,
        };

        // Same one-shot treatment for MCP servers connected mid-session
        // (queued above). `.take()` clears it so it fires exactly once.
        let pending_mcp = std::mem::take(&mut self.pending_mcp_announcement);
        let enriched = match mcp_announcement_note(&pending_mcp) {
            Some(note) => format!("{note}\n\n{enriched}"),
            None => enriched,
        };

        // Same one-shot pattern for skills installed mid-session (parked by
        // `refresh_workflows` above). Rides the user turn so the KV-cache
        // prefix stays stable; `.take()` fires it exactly once.
        let pending_skills = std::mem::take(&mut self.pending_skill_announcement);
        let enriched = match skill_announcement_note(&pending_skills) {
            Some(note) => format!("{note}\n\n{enriched}"),
            None => enriched,
        };

        // Same one-shot treatment for skills uninstalled mid-session (parked by
        // `refresh_workflows`). The model must know the skill is gone so it does
        // not attempt `run_skill` on a removed entry. Rides the user turn for
        // the same KV-cache reason as the install note above.
        let pending_retracted = std::mem::take(&mut self.pending_skill_retraction);
        let enriched = match skill_retraction_note(&pending_retracted) {
            Some(note) => format!("{note}\n\n{enriched}"),
            None => enriched,
        };

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
        let session_memory_parent_context = parent_context.clone();

        let mut agent_context_prepared_sources: Vec<harness::AgentContextPreparedSource> =
            Vec::new();
        // Triggered memory-agent recall runs on EVERY channel, voice included:
        // dropping it on voice would strip the user's remembered context
        // (preferences, people, prior facts) from spoken answers — a real quality
        // loss the transcript alone can't replace. Recall adds a few seconds of
        // embedding + retrieval before the first model token, but on realtime
        // voice that latency is already covered end-to-end: the backend relay
        // streams an audible keepalive filler from t=0 so the cloud session never
        // sees a silent stall, and the desktop's ~8s ack-defer closes the spoken
        // turn and finishes in the background if the work runs long. So the recall
        // path is byte-for-byte identical across voice and chat.
        let (enriched, memory_agent_context_injected) = self
            .inject_triggered_memory_agent_context(
                user_message,
                enriched,
                &parent_context,
                turn_overrides.suppress_memory_agent,
            )
            .await;
        if memory_agent_context_injected {
            agent_context_prepared_sources.push(harness::AgentContextPreparedSource {
                source: "memory agent context retrieval".to_string(),
                has_enough_context: None,
            });
        }

        let enriched = if agent_context_prepared_sources.is_empty() {
            enriched
        } else {
            log::debug!(
                "[agent_loop] agent context already prepared sources={:?}",
                agent_context_prepared_sources
            );
            format!(
                "{}\n\n{enriched}",
                render_agent_context_status_note(&agent_context_prepared_sources)
            )
        };

        // #3602: stamp every turn's user message with the live local time
        // so time-relative phrasing (greetings, "today"/"tonight") is
        // grounded on the real clock. Rides the user message — not the
        // frozen system-prompt prefix (see core.rs KV-cache note above) — so
        // it stays fresh across a long-lived session without busting the
        // cached prefix. This path runs for every `turn()` caller, including
        // one-shot `run_single` flows (cron/morning-briefing/meet), so those
        // get a fresh stamp too. The grounding *rule* lives in the system
        // prompt's `## Current Date & Time` section.
        let enriched = format!(
            "{}\n\n{enriched}",
            crate::openhuman::agent::prompts::current_datetime_line()
        );

        self.history
            .push(ConversationMessage::Chat(ChatMessage::user(enriched)));

        // Bump the session-memory turn counter. Used later by
        // `should_extract_session_memory` to decide whether to spawn a
        // background archivist fork at end-of-turn.
        self.context.tick_turn();

        let turn_body = async {
            // Keep the scalar turn settings outside the pinned future arguments;
            // the TinyAgents session path reads provider/tool/multimodal state
            // directly from `self` when preparing the request.
            let temperature = self.temperature;
            let max_iterations = self.config.max_tool_iterations;
            let artifact_store = Some(
                crate::openhuman::agent::harness::tool_result_artifacts::ToolResultArtifactStore::new(
                    self.action_dir.clone(),
                    self.session_key.clone(),
                ),
            );
            // The whole turn runs through the tinyagents harness (issue #4249);
            // the legacy `run_turn_engine` has been removed. Heap-allocate the
            // (large) session-turn future so it isn't held inline on `turn()`'s
            // already-large frame — `run_single` and the cron wrappers nest more
            // layers on top, which would otherwise overflow the stack.
            Box::pin(self.run_turn_via_tinyagents_session(
                user_message,
                &effective_model,
                temperature,
                max_iterations,
                artifact_store,
                turn_overrides.suppress_tools,
            ))
            .await
        }; // end of `turn_body` async block

        // Run the turn body inside the parent-execution-context scope so
        // that any `spawn_subagent` tool call fired during the loop can
        // read the parent's provider, tools, model, and workspace via
        // the PARENT_CONTEXT task-local.
        // Arm the thread-goal budget stop hook for this turn when an active,
        // budgeted goal exists — it votes to stop the loop as soon as running
        // usage would exceed the cap. #4469 item 1: the stop is a graceful pause
        // drained at the next iteration boundary, not an instantaneous abort, so
        // the current tool round + one wrap-up summary call can still run past the
        // cap (a small, bounded overshoot) before the partial transcript returns.
        // Merge with any ambient stop hooks rather than clobbering them. No
        // budgeted active goal → no extra hook, no wrap.
        let mut turn_stop_hooks = crate::openhuman::agent::stop_hooks::current_stop_hooks();
        if let Some(ref goal) = active_goal {
            if let Some(hook) =
                crate::openhuman::threads::goals::runtime::GoalBudgetStopHook::for_goal(
                    &goal_workspace_dir,
                    goal,
                )
            {
                turn_stop_hooks.push(std::sync::Arc::new(hook));
            }
        }
        // Surface this turn's image-attachment placeholders so a delegation to a
        // vision sub-agent (which reads `current_turn_image_placeholders()` in
        // `agent_orchestration::tools::dispatch`) can forward the user's attached
        // image — the orchestrator itself keeps it as a text placeholder. Scoped
        // around the harness turn (the delegating tool fires inside it).
        let image_placeholders =
            crate::openhuman::agent::multimodal::extract_image_placeholders_in_text(user_message);
        let result = if turn_stop_hooks.is_empty() {
            harness::with_parent_context(
                parent_context,
                harness::with_agent_context_prepared_sources(
                    agent_context_prepared_sources.clone(),
                    harness::turn_attachments_context::with_current_turn_image_placeholders(
                        image_placeholders,
                        turn_body,
                    ),
                ),
            )
            .await
        } else {
            harness::with_parent_context(
                parent_context,
                harness::with_agent_context_prepared_sources(
                    agent_context_prepared_sources.clone(),
                    harness::turn_attachments_context::with_current_turn_image_placeholders(
                        image_placeholders,
                        crate::openhuman::agent::stop_hooks::with_stop_hooks(
                            turn_stop_hooks,
                            turn_body,
                        ),
                    ),
                ),
            )
            .await
        };

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
            self.spawn_session_memory_extraction(session_memory_parent_context)
                .await;
            // Sibling pipeline (#1399): heuristic transcript ingestion
            // turns the just-written transcript into durable
            // conversational memory + reflections so a brand-new chat
            // can recover continuity. Background-only, never blocks the
            // user-facing turn return.
            self.spawn_transcript_ingestion();
        }

        result
    }
}
