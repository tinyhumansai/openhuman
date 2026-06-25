//! Core turn execution: the main `turn()` method and `inject_agent_experience_context()`.

use super::super::transcript;
use super::super::turn_engine_adapter::{AgentCheckpoint, AgentObserver, AgentToolSource};
use super::super::types::Agent;
use super::{
    integration_announcement_note, mcp_announcement_note, newly_connected_slugs,
    skill_announcement_note, skill_retraction_note,
};
use crate::openhuman::agent::harness;
use crate::openhuman::agent::harness::definition::TriggerMemoryAgent;
use crate::openhuman::agent::harness::fork_context::ParentExecutionContext;
use crate::openhuman::agent::hooks::{self, TurnContext};
use crate::openhuman::agent::memory_loader::collect_recall_citations;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent_experience::{
    prepend_experience_block, render_experience_hits, AgentExperienceStore, ExperienceQuery,
};
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage};
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::util::truncate_with_ellipsis;

use anyhow::Result;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Decide whether the harness-driven "super context" collection pass should
/// run this turn.
///
/// It runs only on the first turn of a **genuinely new** thread driven by the
/// **user-facing orchestrator**:
/// - `is_orchestrator` — the turn belongs to the `orchestrator` agent (the
///   interactive chat path surfaced by the composer toggle). `Agent::turn` is
///   shared with `run_single()` background/automated flows (goals enrichment,
///   cron/task agents, specialist sub-agents); without this gate those first
///   turns would spawn `context_scout` and prepend a prepared-context block,
///   adding unexpected LLM/tool work and changing automated outputs; AND
/// - `first_turn` — the agent's `history` is empty at turn start; AND
/// - `!has_prior_conversation` — the seeded `cached_transcript_messages`
///   prefix contains no prior **assistant** reply. A thread resumed cold
///   (web-chat task rebuilt for an existing conversation, or a transcript
///   loaded from disk) also has an empty `history`, so the seeded prefix is
///   what distinguishes a *new* thread from a *resumed* one. We key on a prior
///   assistant message rather than "any cached prefix" because an
///   attachment-first new thread can seed a single just-persisted *user* row
///   (the expanded `[IMAGE:…]`/`[FILE:…]` send payload doesn't exact-match the
///   persisted `content`, so `seed_resume_from_messages` can't drop it) — that
///   is still a brand-new conversation and should get super context; AND
/// - `enabled` — the `context.super_context_enabled` config flag is on.
///
/// Pulled out as a pure function so the gate (in particular the resume and
/// orchestrator guards) is unit-testable without a full agent turn harness.
fn should_run_super_context(
    is_orchestrator: bool,
    first_turn: bool,
    has_prior_conversation: bool,
    enabled: bool,
) -> bool {
    is_orchestrator && first_turn && !has_prior_conversation && enabled
}

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
        // Capture before any system-prompt push mutates `history`: this is the
        // signal that gates first-turn-only work (system prompt build, and the
        // "super context" harness-driven context-collection pass below).
        let first_turn = self.history.is_empty();
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
        if self.history.is_empty() && self.cached_transcript_messages.is_none() {
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
            // Seed the per-turn mid-session refresh baseline with the
            // hash of whatever Composio actually returned just now.
            // Subsequent turns short-circuit unless this hash changes.
            self.last_seen_integrations_hash =
                crate::openhuman::composio::connected_set_hash(&self.connected_integrations);
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
                crate::openhuman::mcp_registry::connections::connected_overview()
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
            // Dynamic turn-to-turn context (memory recall, learned snippets)
            // rides on the user message via `memory_loader.load_context()`
            // — that's where the caller should inject anything that varies
            // between turns.
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
            // [`crate::openhuman::composio::INTEGRATIONS_CACHE`], kept
            // current by (a) the desktop UI's 5 s
            // `composio_list_connections` poll, (b) the post-OAuth
            // `ComposioConnectionCreatedSubscriber` invalidation, and
            // (c) the 60 s TTL fallback. We read it via the read-only
            // [`crate::openhuman::composio::cached_active_integrations`]
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
                crate::openhuman::mcp_registry::connections::connected_overview()
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

        if self.auto_save {
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
            log::debug!(
                "[agent_autosave] enqueue user-message store key={autosave_key} chars={chars}"
            );
            tokio::spawn(async move {
                match memory
                    .store(
                        "",
                        &autosave_key,
                        &user_msg,
                        MemoryCategory::Conversation,
                        None,
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

        // ── Phase 3 STM preemptive recall ────────────────────────────
        // On the very first turn only, assemble a bounded cross-thread
        // context block from the FTS5 episodic arm (keyword match) and the
        let mut context = context;

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
                crate::openhuman::memory::preferences::recall_situational_preferences(
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
        let active_goal = {
            let loaded = crate::openhuman::thread_goals::runtime::load_for_current_thread(
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
                        crate::openhuman::thread_goals::ThreadGoalStatus::Paused
                    ) =>
                {
                    crate::openhuman::thread_goals::runtime::resume_for_current_thread(
                        &self.workspace_dir,
                    )
                    .await
                    .unwrap_or(Some(goal))
                }
                other => other,
            }
        };
        if let Some(ref goal) = active_goal {
            if let Some(block) =
                crate::openhuman::thread_goals::runtime::active_goal_context_block(goal)
            {
                log::info!(
                    "[thread_goals] injecting active_goal block status={} budget={:?} ({} chars)",
                    goal.status.as_str(),
                    goal.token_budget,
                    block.chars().count()
                );
                context.push_str(&block);
            }
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
        // Capture before `self` is borrowed by the turn observer below, so it can
        // be installed as the `current_model_vision` task-local around the engine
        // call (read by the image gate for custom/BYOK vision models).
        let model_vision = self.model_vision;
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

        let enriched = self
            .inject_triggered_memory_agent_context(user_message, enriched, &parent_context)
            .await;

        // ── "Super context": harness-driven first-turn context collection ──
        // When enabled (config `context.super_context_enabled`, surfaced as the
        // composer toggle), run the read-only `context_scout` BEFORE the
        // orchestrator LLM gets the turn, and fold its bounded
        // `[context_bundle]` into the user message. This is the harness driving
        // the collection deterministically — unlike the `agent_prepare_context`
        // tool, which the model chooses to call. The tool stays exposed so the
        // model can still scout again mid-turn.
        //
        // Gate on the **first turn of a genuinely new thread**: `first_turn`
        // (empty `history`) is necessary but NOT sufficient, because a thread
        // resumed cold (e.g. a web-chat task rebuilt for an existing
        // conversation after an app restart) seeds prior messages into
        // `cached_transcript_messages` via `seed_resume_from_messages` /
        // `try_load_session_transcript` WITHOUT populating `history`. Without
        // the `cached_transcript_messages.is_none()` guard, super context would
        // re-fire on every cold-started existing conversation, surprising the
        // user with extra scout/tool calls and a stray prepared-context block.
        //
        // Runs inside the parent-context scope because `run_context_scout`
        // reads the parent's visible tool catalogue and runs the scout against
        // the parent's provider via the PARENT_CONTEXT task-local. Best-effort:
        // any failure (scout error, no bundle) leaves the turn to proceed with
        // the un-augmented message rather than blocking the user.
        // A genuinely new thread has no prior assistant reply in its seeded
        // transcript prefix; a cold-resumed thread does. (An attachment-first
        // new thread may seed a lone user row — see `should_run_super_context`.)
        let has_prior_conversation = self
            .cached_transcript_messages
            .as_ref()
            .is_some_and(|msgs| msgs.iter().any(|m| m.role == "assistant"));
        let enriched = if should_run_super_context(
            self.agent_definition_id == "orchestrator",
            first_turn,
            has_prior_conversation,
            self.context.super_context_enabled(),
        ) {
            log::info!(
                "[agent_loop] super_context enabled — running harness-driven context collection (new thread, first turn)"
            );
            let scout = harness::with_parent_context(parent_context.clone(), {
                let user_message = user_message.to_string();
                async move {
                    crate::openhuman::agent_orchestration::tools::run_context_scout(
                        &user_message,
                        None,
                    )
                    .await
                }
            })
            .await;
            match scout {
                Ok(result) if !result.is_error => {
                    let bundle = result.output();
                    log::info!(
                        "[agent_loop] super_context bundle collected bundle_chars={}",
                        bundle.chars().count()
                    );
                    format!(
                        "## Prepared context (super context)\n\nThe following context was \
                         collected up-front by a read-only context scout before this turn. \
                         Use it to ground your response; you may still gather more if needed.\n\n\
                         {bundle}\n\n---\n\n{enriched}"
                    )
                }
                Ok(result) => {
                    log::warn!(
                        "[agent_loop] super_context scout returned an error — proceeding without bundle: {}",
                        result.output()
                    );
                    enriched
                }
                Err(err) => {
                    log::warn!(
                        "[agent_loop] super_context collection failed — proceeding without bundle: {err}"
                    );
                    enriched
                }
            }
        } else {
            enriched
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
            // Capture everything the engine seams need as locals/clones *before*
            // the observer takes `&mut self`, so the borrow checker is happy:
            // the tool source + parser + checkpoint hold clones disjoint from
            // the `Agent`, and the observer alone borrows it mutably.
            let dispatcher = self.tool_dispatcher.clone();
            let provider = self.provider.clone();
            let provider_name = self.event_channel().to_string();
            let temperature = self.temperature;
            let max_iterations = self.config.max_tool_iterations;
            // Source multimodal limits from the session's runtime config when
            // present so [IMAGE:…] / [FILE:…] markers in user messages are
            // resolved with the operator-configured caps (max files, max size,
            // max extracted text). Without this, agents fall back to the
            // crate-default caps and `MultimodalFileConfig::default()`
            // disables file expansion entirely.
            let multimodal = self
                .integration_runtime_config
                .as_ref()
                .map(|c| c.multimodal.clone())
                .unwrap_or_default();
            let multimodal_files = self
                .integration_runtime_config
                .as_ref()
                .map(|c| c.multimodal_files.clone())
                .unwrap_or_default();
            let artifact_store = Some(
                crate::openhuman::agent::harness::tool_result_artifacts::ToolResultArtifactStore::new(
                    self.action_dir.clone(),
                    self.session_key.clone(),
                ),
            );
            let mut tool_source = AgentToolSource {
                tools: self.tools.clone(),
                visible_tool_names: self.visible_tool_names.clone(),
                tool_policy_session: self.tool_policy_session.clone(),
                tool_policy: self.tool_policy.clone(),
                payload_summarizer: self.payload_summarizer.clone(),
                event_session_id: self.event_session_id().to_string(),
                event_channel: self.event_channel().to_string(),
                agent_definition_id: self.agent_definition_id.clone(),
                prefer_markdown: self.context.prefer_markdown_tool_output(),
                budget_bytes: self.context.tool_result_budget_bytes(),
                compaction_enabled: self.context.compaction_enabled(),
                artifact_store: artifact_store.clone(),
                should_send_specs: self.tool_dispatcher.should_send_tool_specs(),
                advertised_specs: self.visible_tool_specs.as_ref().clone(),
                records: Vec::new(),
            };
            let progress = super::super::super::engine::TurnProgress::new(self.on_progress.clone());
            let parser = super::super::super::engine::DispatcherParser {
                dispatcher: dispatcher.as_ref(),
            };
            let checkpoint = AgentCheckpoint {
                provider: self.provider.clone(),
                dispatcher: self.tool_dispatcher.clone(),
                model: effective_model.clone(),
                temperature,
                on_progress: self.on_progress.clone(),
                user_message: user_message.to_string(),
                max_iterations,
            };
            let turn_run_queue = self.run_queue.clone();
            let cached_prefix = self.cached_transcript_messages.take();
            // Resolve the context window once per turn through the provider so
            // local providers (LM Studio) trim to their runtime-loaded n_ctx
            // rather than the trained-max table (#3550 / TAURI-RUST-6V0).
            // Must run before `agent: self` takes the &mut borrow below.
            //
            // For local providers this is always `Some` (a conservative floor
            // backs up any missing profile default), so trimming always engages.
            // `None` means a cloud provider with an unknown model — trimming is
            // intentionally skipped there (large window; over-trimming is worse).
            let turn_context_window = self
                .provider
                .effective_context_window(&effective_model)
                .await;
            match turn_context_window {
                Some(context_window) => tracing::debug!(
                    provider = %provider_name,
                    model = %effective_model,
                    context_window,
                    "[agent_loop] effective context window resolved for turn"
                ),
                None => tracing::debug!(
                    provider = %provider_name,
                    model = %effective_model,
                    "[agent_loop] effective context window unavailable (cloud unknown model); pre-dispatch trimming skipped this turn"
                ),
            }
            let mut observer = AgentObserver {
                agent: self,
                artifact_store,
                effective_model: effective_model.clone(),
                context_window: turn_context_window,
                cumulative_input: 0,
                cumulative_output: 0,
                cumulative_cached: 0,
                cumulative_charged: 0.0,
                last_turn_usage: None,
                cached_prefix,
                pending_results: Vec::new(),
                did_push_final: false,
            };
            let mut buf: Vec<ChatMessage> = Vec::new();

            // Box-pin the parent agent's engine call so its ~600-line
            // generator state lives on the heap. Tools that delegate to
            // sub-agents (orchestrator → researcher / personality /
            // archetype / skill) recurse back into another
            // `run_turn_engine` via `run_subagent`; without the box,
            // both engines' state machines pile up on the same tokio
            // worker stack and overflow the 2 MiB default. The inner
            // boxes inside `run_typed_mode` aren't reached if the
            // overflow happens during the parent's poll on the way in
            // — verified against the `chat-harness-subagent` Playwright
            // lane crash on PR #3151.
            // Carry the current turn's image placeholders so a delegation to the
            // vision sub-agent (analyze_image) can forward the attached image
            // into its prompt — the orchestrator's own non-vision turn keeps the
            // placeholder as text and never rehydrates it.
            let turn_image_placeholders =
                crate::openhuman::agent::multimodal::extract_image_placeholders_in_text(
                    user_message,
                );
            let outcome =
                super::super::super::turn_attachments_context::with_current_turn_image_placeholders(
                    turn_image_placeholders,
                    super::super::super::model_vision_context::with_current_model_vision(
                        model_vision,
                        Box::pin(super::super::super::engine::run_turn_engine(
                    provider.as_ref(),
                    &mut buf,
                    &mut tool_source,
                    &progress,
                    &mut observer,
                    &checkpoint,
                    &parser,
                    &provider_name,
                    &effective_model,
                    temperature,
                    true, // silent — the channel/UI renders via progress + the return value
                    &multimodal,
                    &multimodal_files,
                    max_iterations,
                    None, // the web bridge streams via on_progress deltas, not on_delta
                    &[],
                    turn_run_queue,
                    None, // main agent compacts via its ContextManager in before_dispatch
                        )),
                    ),
                )
                .await?;

            // Pull the observer's accounting out, then drop it to release the
            // `&mut self` borrow so the epilogue can use `self`.
            let did_push_final = observer.did_push_final;
            let cumulative_input = observer.cumulative_input;
            let cumulative_output = observer.cumulative_output;
            let cumulative_cached = observer.cumulative_cached;
            let cumulative_charged = observer.cumulative_charged;
            let last_turn_usage = observer.last_turn_usage.take();
            drop(observer);
            let records = std::mem::take(&mut tool_source.records);

            self.context.record_tool_calls(records.len());

            // Account this turn's tokens (prompt + completion) and elapsed time
            // against the thread's active goal, flipping it to budget_limited
            // when the cap is crossed. Best-effort — never fails the turn.
            crate::openhuman::thread_goals::runtime::account_turn_against_goal(
                &self.workspace_dir,
                cumulative_input,
                cumulative_output,
                turn_started.elapsed().as_secs(),
            )
            .await;

            // For a clean final response the observer already pushed the
            // assistant message + persisted. For a max-iteration checkpoint or
            // circuit-breaker halt the engine returned the text without pushing
            // it, so finish the history + transcript here (mirrors the old
            // final/max-iter branches).
            if !did_push_final {
                self.history
                    .push(ConversationMessage::Chat(ChatMessage::assistant(
                        outcome.text.clone(),
                    )));
                self.trim_history();
                // Note: the engine already emits `TurnCompleted` on the
                // checkpoint exit (and every other terminal path), so we don't
                // re-emit it here — doing so would double-fire for the UI.
                let messages = self.tool_dispatcher.to_provider_messages(&self.history);
                self.persist_session_transcript(
                    &messages,
                    cumulative_input,
                    cumulative_output,
                    cumulative_cached,
                    cumulative_charged,
                    last_turn_usage.as_ref(),
                );
            }

            // Auto-save a short memory of the final reply (not on a capped turn,
            // matching the prior behavior).
            if self.auto_save && !outcome.hit_cap {
                let summary = truncate_with_ellipsis(&outcome.text, 100);
                let _ = self
                    .memory
                    .store("", "assistant_resp", &summary, MemoryCategory::Daily, None)
                    .await;
            }

            // Fire post-turn hooks (non-blocking).
            if !self.post_turn_hooks.is_empty() {
                let ctx = TurnContext {
                    user_message: user_message.to_string(),
                    assistant_response: outcome.text.clone(),
                    tool_calls: records,
                    turn_duration_ms: turn_started.elapsed().as_millis() as u64,
                    session_id: Some(self.event_session_id.clone())
                        .filter(|session_id| !session_id.trim().is_empty()),
                    agent_id: Some(self.agent_definition_id.clone())
                        .filter(|agent_id| !agent_id.trim().is_empty()),
                    entrypoint: Some(self.event_channel.clone())
                        .filter(|entrypoint| !entrypoint.trim().is_empty()),
                    iteration_count: outcome.iterations as usize,
                };
                hooks::fire_hooks(&self.post_turn_hooks, ctx);
            }

            Ok(outcome.text)
        }; // end of `turn_body` async block

        // Run the turn body inside the parent-execution-context scope so
        // that any `spawn_subagent` tool call fired during the loop can
        // read the parent's provider, tools, model, and workspace via
        // the PARENT_CONTEXT task-local.
        // Arm the thread-goal budget stop hook for this turn when an active,
        // budgeted goal exists — it hard-stops the loop the moment running usage
        // would exceed the cap (so an autonomous run can't blow past it between
        // accounting points). Merge with any ambient stop hooks rather than
        // clobbering them. No budgeted active goal → no extra hook, no wrap.
        let mut turn_stop_hooks = crate::openhuman::agent::stop_hooks::current_stop_hooks();
        if let Some(ref goal) = active_goal {
            if let Some(hook) =
                crate::openhuman::thread_goals::runtime::GoalBudgetStopHook::for_goal(
                    &goal_workspace_dir,
                    goal,
                )
            {
                turn_stop_hooks.push(std::sync::Arc::new(hook));
            }
        }
        let result = if turn_stop_hooks.is_empty() {
            harness::with_parent_context(parent_context, turn_body).await
        } else {
            harness::with_parent_context(
                parent_context,
                crate::openhuman::agent::stop_hooks::with_stop_hooks(turn_stop_hooks, turn_body),
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
            self.spawn_session_memory_extraction().await;
            // Sibling pipeline (#1399): heuristic transcript ingestion
            // turns the just-written transcript into durable
            // conversational memory + reflections so a brand-new chat
            // can recover continuity. Background-only, never blocks the
            // user-facing turn return.
            self.spawn_transcript_ingestion();
        }

        result
    }

    pub(super) async fn inject_agent_experience_context(
        &self,
        user_message: &str,
        enriched: String,
    ) -> String {
        const MAX_EXPERIENCE_HITS: usize = 3;
        const MAX_EXPERIENCE_BLOCK_BYTES: usize = 2048;

        if !self.learning_enabled {
            return enriched;
        }

        let tools = self
            .visible_tool_specs
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        let store = AgentExperienceStore::new(self.memory.clone());
        let query = ExperienceQuery {
            query: user_message.to_string(),
            tools,
            tags: Vec::new(),
            agent_id: Some(self.agent_definition_id.clone()).filter(|id| !id.trim().is_empty()),
            entrypoint: Some(self.event_channel.clone())
                .filter(|entrypoint| !entrypoint.trim().is_empty()),
            max_hits: MAX_EXPERIENCE_HITS,
        };

        match store.retrieve(query).await {
            Ok(hits) => {
                let matched_hits: Vec<_> = hits
                    .into_iter()
                    .filter(|hit| !hit.match_reasons.is_empty())
                    .collect();
                let block = render_experience_hits(&matched_hits, MAX_EXPERIENCE_BLOCK_BYTES);
                if block.is_empty() {
                    return enriched;
                }
                log::debug!(
                    "[agent-experience] injected {} experience hit(s) bytes={}",
                    matched_hits.len(),
                    block.len()
                );
                prepend_experience_block(&enriched, &block)
            }
            Err(err) => {
                log::warn!("[agent-experience] retrieval failed (non-fatal): {err}");
                enriched
            }
        }
    }

    async fn inject_triggered_memory_agent_context(
        &self,
        user_message: &str,
        enriched: String,
        parent_context: &ParentExecutionContext,
    ) -> String {
        const MEMORY_AGENT_ID: &str = "agent_memory";
        const MAX_MEMORY_AGENT_BLOCK_CHARS: usize = 8000;

        if self.trigger_memory_agent != TriggerMemoryAgent::Always {
            log::debug!(
                "[agent_memory:trigger] skipped agent_id={} policy={:?}",
                self.agent_definition_id,
                self.trigger_memory_agent
            );
            return enriched;
        }

        if self.agent_definition_id == MEMORY_AGENT_ID {
            log::debug!("[agent_memory:trigger] skipped recursive memory agent invocation");
            return enriched;
        }

        let Some(registry) = harness::AgentDefinitionRegistry::global() else {
            log::warn!(
                "[agent_memory:trigger] AgentDefinitionRegistry unavailable; continuing without memory agent context"
            );
            return enriched;
        };
        let Some(definition) = registry.get(MEMORY_AGENT_ID).cloned() else {
            log::warn!(
                "[agent_memory:trigger] `{MEMORY_AGENT_ID}` definition unavailable; continuing without memory agent context"
            );
            return enriched;
        };

        let task_id = format!("mem-trigger-{}", uuid::Uuid::new_v4());
        let prompt = format!(
            "Search the user's memory tree and return only context relevant to the next agent turn.\n\nUser prompt:\n{user_message}"
        );
        let options = harness::SubagentRunOptions {
            task_id: Some(task_id.clone()),
            model_override: Some(parent_context.model_name.clone()),
            ..Default::default()
        };

        log::debug!(
            "[agent_memory:trigger] starting agent_id={} task_id={} user_message_chars={}",
            self.agent_definition_id,
            task_id,
            user_message.chars().count()
        );

        let started = std::time::Instant::now();
        let result = harness::with_parent_context(parent_context.clone(), async move {
            harness::run_subagent(&definition, &prompt, options).await
        })
        .await;

        match result {
            Ok(outcome) => {
                log::info!(
                    "[agent_memory:trigger] completed agent_id={} task_id={} iterations={} elapsed={:?} status={:?} output_chars={}",
                    self.agent_definition_id,
                    task_id,
                    outcome.iterations,
                    started.elapsed(),
                    outcome.status,
                    outcome.output.chars().count()
                );
                let mut output =
                    truncate_with_ellipsis(&outcome.output, MAX_MEMORY_AGENT_BLOCK_CHARS);
                if let harness::subagent_runner::SubagentRunStatus::AwaitingUser {
                    question, ..
                } = &outcome.status
                {
                    let question = question.trim();
                    if !question.is_empty() {
                        output.push_str("\n\nMemory agent needs clarification: ");
                        output.push_str(question);
                    }
                }
                output = truncate_with_ellipsis(&output, MAX_MEMORY_AGENT_BLOCK_CHARS);
                if output.trim().is_empty() {
                    return enriched;
                }
                format!(
                    "## Memory agent context\n\n{}\n\n---\n\n{}",
                    output.trim(),
                    enriched
                )
            }
            Err(err) => {
                log::warn!(
                    "[agent_memory:trigger] failed agent_id={} task_id={}: {err:#}",
                    self.agent_definition_id,
                    task_id
                );
                enriched
            }
        }
    }
}

#[cfg(test)]
mod super_context_gate_tests {
    use super::should_run_super_context;

    #[test]
    fn runs_only_on_first_turn_of_a_new_orchestrator_thread_when_enabled() {
        // Orchestrator, new thread, first turn, flag on → run.
        assert!(should_run_super_context(true, true, false, true));
    }

    #[test]
    fn skips_when_flag_disabled() {
        assert!(!should_run_super_context(true, true, false, false));
    }

    #[test]
    fn skips_on_later_turns() {
        // history non-empty → not the first turn.
        assert!(!should_run_super_context(true, false, false, true));
    }

    #[test]
    fn skips_on_cold_resumed_thread_even_on_first_turn() {
        // Regression: a thread resumed cold has an empty `history` (so
        // `first_turn` is true) but a seeded prefix that includes a prior
        // assistant reply. Super context must NOT re-fire on these existing
        // conversations.
        assert!(!should_run_super_context(true, true, true, true));
    }

    #[test]
    fn runs_for_attachment_first_new_thread_with_lone_seeded_user_row() {
        // Regression: an attachment-first new thread can seed a single just-
        // persisted *user* row (no assistant reply), so `has_prior_conversation`
        // is false. That is still a brand-new conversation — super context
        // should run.
        assert!(should_run_super_context(true, true, false, true));
    }

    #[test]
    fn skips_for_non_orchestrator_agents() {
        // Regression: `Agent::turn` is shared with background/automated
        // `run_single()` flows (goals enrichment, cron/task agents,
        // specialist sub-agents). Even on a fresh first turn with the flag on,
        // super context must only run for the user-facing orchestrator.
        assert!(!should_run_super_context(false, true, false, true));
    }
}
