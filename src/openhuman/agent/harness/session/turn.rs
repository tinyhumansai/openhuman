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
//!   per-call runners.
//! - [`Agent::build_parent_execution_context`] — snapshot helper for
//!   the parent-context task-local that sub-agents read.
//! - [`Agent::trim_history`], [`Agent::fetch_learned_context`],
//!   [`Agent::build_system_prompt`] — the small helpers `turn()` leans
//!   on every call.
//! - [`Agent::spawn_session_memory_extraction`] — the fire-and-forget
//!   background archivist fork.

use super::transcript;
use super::turn_engine_adapter::{AgentCheckpoint, AgentObserver, AgentToolSource};
use super::types::Agent;
use crate::openhuman::agent::dispatcher::{ParsedToolCall, ToolExecutionResult};
use crate::openhuman::agent::harness;
use crate::openhuman::agent::hooks::{self, ToolCallRecord, TurnContext};
use crate::openhuman::agent::memory_loader::collect_recall_citations;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent_experience::{
    prepend_experience_block, render_experience_hits, AgentExperienceStore, ExperienceQuery,
};
use crate::openhuman::agent_tool_policy::render_tool_policy_boundary;
use crate::openhuman::context::prompt::{
    LearnedContextData, NamespaceSummary, PromptContext, PromptTool,
};
use crate::openhuman::context::ARCHIVIST_EXTRACTION_PROMPT;
use crate::openhuman::inference::provider::{
    ChatMessage, ChatRequest, ConversationMessage, ProviderDelta, UsageInfo,
};
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::tools::Tool;
use crate::openhuman::util::truncate_with_ellipsis;

use anyhow::Result;
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// True when `msg` is an `assistant` ChatMessage whose JSON-encoded content
/// carries a non-empty `tool_calls` array.
///
/// `to_provider_messages` (in `agent/dispatcher.rs`) serialises an
/// `AssistantToolCalls` ConversationMessage as a single `assistant` ChatMessage
/// with a JSON body of the form `{"content": "...", "tool_calls": [...]}`. To
/// detect those at the `ChatMessage` boundary (where `bound_cached_transcript_messages`
/// operates) we have to peek inside the JSON. See TAURI-RUST-7 for the
/// failure mode this guards against.
use super::turn_checkpoint::{assistant_message_has_tool_calls, MAX_ITER_CHECKPOINT_INSTRUCTION};

/// Built-in direct tools that the orchestrator should call by name, not via
/// `run_skill`.
const DIRECT_TOOL_NAMES: &[&str] = &[
    "cron_add",
    "cron_list",
    "cron_remove",
    "cron_update",
    "cron_run",
    "cron_runs",
    "current_time",
];

/// Recovery shim for legacy/wrong-model calls of the form:
/// `run_skill({skill_id: "<built-in tool>", inputs: {...}})`.
///
/// When this pattern appears, rewrite it into a direct tool call so the turn
/// can proceed without a manual retry.
fn normalize_tool_call<'a>(call: &'a ParsedToolCall) -> Cow<'a, ParsedToolCall> {
    if call.name != "run_skill" {
        return Cow::Borrowed(call);
    }
    let Some(skill_id) = call.arguments.get("skill_id").and_then(|v| v.as_str()) else {
        return Cow::Borrowed(call);
    };
    if !DIRECT_TOOL_NAMES.contains(&skill_id) {
        return Cow::Borrowed(call);
    }
    let Some(inputs) = call.arguments.get("inputs").and_then(|v| v.as_object()) else {
        return Cow::Borrowed(call);
    };

    log::warn!(
        "[agent_loop] rewrote legacy run_skill->{} call into direct tool invocation",
        skill_id
    );
    Cow::Owned(ParsedToolCall {
        name: skill_id.to_string(),
        arguments: serde_json::Value::Object(inputs.clone()),
        tool_call_id: call.tool_call_id.clone(),
    })
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
            if let Some(cfg) = self.integration_runtime_config.as_ref() {
                if let Some(cache_view) =
                    crate::openhuman::composio::cached_active_integrations(cfg)
                {
                    let new_hash = crate::openhuman::composio::connected_set_hash(&cache_view);
                    if new_hash != self.last_seen_integrations_hash {
                        log::info!(
                            "[agent_loop] connection set changed mid-session (hash {:x} -> {:x}); refreshing tool schema (system prompt left intact for KV cache)",
                            self.last_seen_integrations_hash,
                            new_hash
                        );
                        // Snapshot the previous integration list so we
                        // can roll back if `refresh_delegation_tools`
                        // bails on a shared `Arc` — otherwise the
                        // agent's `connected_integrations` and its
                        // schema would disagree until the next
                        // event-driven refresh, and the
                        // `last_seen_integrations_hash` advance below
                        // would suppress retries.
                        let prev_integrations =
                            std::mem::replace(&mut self.connected_integrations, cache_view);
                        if self.refresh_delegation_tools() {
                            self.last_seen_integrations_hash = new_hash;
                            self.connected_integrations_initialized = true;
                        } else {
                            // Reconcile aborted (shared Arc) — restore
                            // the previous integration list so the
                            // next turn re-detects the same change and
                            // retries cleanly. We deliberately do NOT
                            // advance `last_seen_integrations_hash`.
                            self.connected_integrations = prev_integrations;
                        }
                    }
                }
            }
            // Cache empty/expired or config unavailable => no signal.
            // We leave the current tool surface alone and pick up any
            // real change on the next turn after the UI's 5 s poll has
            // repopulated [`INTEGRATIONS_CACHE`].

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
        //
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
            let multimodal = crate::openhuman::config::MultimodalConfig::default();
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
                should_send_specs: self.tool_dispatcher.should_send_tool_specs(),
                advertised_specs: self.visible_tool_specs.as_ref().clone(),
                records: Vec::new(),
            };
            let progress = super::super::engine::TurnProgress::new(self.on_progress.clone());
            let parser = super::super::engine::DispatcherParser {
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
            let cached_prefix = self.cached_transcript_messages.take();
            let mut observer = AgentObserver {
                agent: self,
                effective_model: effective_model.clone(),
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

            let outcome = super::super::engine::run_turn_engine(
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
                max_iterations,
                None, // the web bridge streams via on_progress deltas, not on_delta
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

    async fn inject_agent_experience_context(
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
        let normalized_call = normalize_tool_call(call);
        let call: &ParsedToolCall = &normalized_call;
        // The per-call execution path lives in the shared
        // [`super::agent_tool_exec::run_agent_tool_call`] so `Agent::turn`
        // (when migrated to the turn engine, via `AgentToolSource`) and any
        // direct caller run the identical logic. Progress is emitted through a
        // `TurnProgress` over this agent's sink. Legacy `run_skill`-wrapped
        // built-in cron tool calls are normalized to direct calls first.
        let progress = super::super::engine::TurnProgress::new(self.on_progress.clone());
        let ctx = super::agent_tool_exec::AgentToolExecCtx {
            tools: &self.tools,
            visible_tool_names: &self.visible_tool_names,
            tool_policy_session: &self.tool_policy_session,
            tool_policy: self.tool_policy.as_ref(),
            payload_summarizer: self.payload_summarizer.as_deref(),
            event_session_id: self.event_session_id(),
            event_channel: self.event_channel(),
            agent_definition_id: &self.agent_definition_id,
            prefer_markdown: self.context.prefer_markdown_tool_output(),
            budget_bytes: self.context.tool_result_budget_bytes(),
        };
        super::agent_tool_exec::run_agent_tool_call(&ctx, &progress, call, iteration).await
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
            memory_context: Arc::new(self.last_memory_context.clone()),
            session_id: self.event_session_id().to_string(),
            channel: self.event_channel().to_string(),
            connected_integrations: self.connected_integrations.clone(),
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            session_key: self.session_key.clone(),
            session_parent_prefix: self.session_parent_prefix.clone(),
            on_progress: self.on_progress.clone(),
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

        // A cut that lands *between* an `AssistantToolCalls` and its
        // `ToolResults` leaves the window opening on an orphaned `ToolResults`.
        // Serialized, that is a `tool` message with no preceding `tool_calls`,
        // which the provider rejects with a 400 (the response streams back
        // empty and surfaces to the user as "Something went wrong"). Snap the
        // boundary forward past any leading orphaned results so the window
        // always starts on a clean turn (a `Chat` or an `AssistantToolCalls`).
        let orphan_lead = other_messages
            .iter()
            .take_while(|m| matches!(m, ConversationMessage::ToolResults(_)))
            .count();
        if orphan_lead > 0 {
            log::debug!(
                "[agent] trim_history snapped window past {orphan_lead} orphaned ToolResults \
                 (tool-cycle bisected by the {max}-message cap)"
            );
            other_messages.drain(0..orphan_lead);
        }

        self.history = system_messages;
        self.history.extend(other_messages);
    }

    /// Bound a resumed transcript prefix to the agent history window.
    ///
    /// Resume paths may load a long prior transcript directly into
    /// `cached_transcript_messages` (provider-ready `ChatMessage`s), which
    /// bypasses `self.history`-based trimming/reduction. Keep at most
    /// `max_history_messages` entries while preserving the leading system
    /// message when present.
    pub(super) fn bound_cached_transcript_messages(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Vec<ChatMessage> {
        let max = self.config.max_history_messages.max(1);
        if messages.len() <= max {
            return messages;
        }

        let has_system = matches!(messages.first(), Some(msg) if msg.role == "system");
        let keep_tail = if has_system {
            max.saturating_sub(1)
        } else {
            max
        };
        let start = messages.len().saturating_sub(keep_tail);

        // Same hazard as `trim_history`: the tail slice can open on a `tool`
        // message whose `tool_calls` opener fell outside the window, which the
        // provider rejects. Advance past any leading orphaned `tool` results so
        // the window starts on a clean turn.
        let tail = &messages[start..];
        let orphan_lead = tail.iter().take_while(|m| m.role == "tool").count();
        if orphan_lead > 0 {
            log::debug!(
                "[agent] bound_cached_transcript_messages snapped window past {orphan_lead} \
                 orphaned tool result(s) (tool-cycle bisected by the {max}-message cap)"
            );
        }
        let tail = &tail[orphan_lead..];

        let mut bounded = Vec::with_capacity(tail.len() + usize::from(has_system));
        if has_system {
            bounded.push(messages[0].clone());
        }
        bounded.extend(tail.iter().cloned());

        // TAURI-RUST-7: symmetric guard to the leading-orphan strip above. A
        // resumed transcript that ends on an `assistant` message containing
        // `tool_calls` (because the cached transcript was captured mid-cycle,
        // before the tool responses were persisted) is rejected by the
        // provider with `400 An assistant message with 'tool_calls' must be
        // followed by tool messages`. Pop any such trailing assistant
        // tool_calls so the bounded transcript ends on a clean turn boundary.
        let mut dropped_tail = 0usize;
        while bounded
            .last()
            .map(assistant_message_has_tool_calls)
            .unwrap_or(false)
        {
            bounded.pop();
            dropped_tail += 1;
        }
        if dropped_tail > 0 {
            log::debug!(
                "[agent] bound_cached_transcript_messages stripped {dropped_tail} trailing \
                 assistant tool_calls message(s) without paired tool responses"
            );
        }

        bounded
    }

    /// Pre-fetches learned context data from memory (observations, patterns, user profile).
    ///
    /// This is an async, non-blocking operation that populates the context
    /// for the system prompt.
    ///
    /// # Explicit-preferences narrow path
    ///
    /// When `learning_enabled` is `false` but `explicit_preferences_enabled`
    /// is `true`, only the `user_profile` namespace (pinned preferences from
    /// the `remember_preference` tool) is fetched and returned.  All other
    /// inference-derived data (observations, patterns, reflections, tree
    /// summaries) remains empty — the inference stack is not touched.
    pub(super) async fn fetch_learned_context(&self) -> LearnedContextData {
        // Fast path: neither the full learning subsystem nor the explicit
        // preferences path is active — skip all memory reads.
        if !self.learning_enabled && !self.explicit_preferences_enabled {
            tracing::debug!(
                "[learning] fetch_learned_context: both learning_enabled and \
                 explicit_preferences_enabled are false — returning empty context"
            );
            return LearnedContextData::default();
        }

        // Narrow explicit-preferences path (Lane A): inject the latest-N general
        // (always-on) preferences written via `save_preference`. Topic-scoped
        // (situational) prefs are NOT injected here — they ride the user message
        // via per-turn recall (Lane B). The legacy `user_profile` pinned namespace
        // is no longer read here; explicit prefs now live in `user_pref_general`.
        if !self.learning_enabled && self.explicit_preferences_enabled {
            let general = crate::openhuman::memory::preferences::load_general_preferences(
                &self.memory,
                crate::openhuman::memory::preferences::STANDING_PREFS_LIMIT,
            )
            .await;
            tracing::debug!(
                "[learning] fetch_learned_context: explicit_preferences_enabled — loaded {} general preference(s) for the system prompt",
                general.len()
            );
            return LearnedContextData {
                user_profile: general,
                ..LearnedContextData::default()
            };
        }

        // Full learning path: fetch all inference-derived data.
        tracing::debug!(
            "[learning] fetch_learned_context: learning_enabled=true — fetching full context"
        );

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

        // Standing preferences come from the explicit two-lane store (Lane A),
        // not the inferred `user_profile` facets — those are demoted: no longer
        // injected as ground truth. A high-confidence inferred facet should be
        // *proposed* to the user (and pinned via `save_preference` on
        // confirmation), not silently treated as a standing preference.
        let general = crate::openhuman::memory::preferences::load_general_preferences(
            &self.memory,
            crate::openhuman::memory::preferences::STANDING_PREFS_LIMIT,
        )
        .await;

        // Explicit user reflections — privileged memory class. Pulled
        // separately from observations/patterns so the prompt assembly
        // can render them ahead of generic tree summaries.
        let reflection_entries = self
            .memory
            .list(
                Some(crate::openhuman::learning::reflection::REFLECTIONS_NAMESPACE),
                Some(&MemoryCategory::Custom(
                    crate::openhuman::learning::reflection::REFLECTIONS_NAMESPACE.into(),
                )),
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
        //
        // Per-namespace + total caps come from the user-facing memory
        // window preset on `AgentConfig` so changing the slider in the
        // UI takes effect on the very next session-start.
        let limits = self.config.resolved_memory_limits();
        let tree_root_summaries = collect_tree_root_summaries(
            &self.workspace_dir,
            limits.per_namespace_max_chars,
            limits.total_tree_max_chars,
        );

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
            user_profile: general,
            // Cap reflections at 10 to keep the privileged section
            // bounded — the issue requires reflections improve context
            // rather than flood it. Newest first.
            reflections: reflection_entries
                .iter()
                .rev()
                .take(10)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            tree_root_summaries,
        }
    }

    /// Fetches the user's active Composio connections and populates
    /// `self.connected_integrations` so the system prompt can surface them.
    ///
    /// Delegates to the shared [`crate::openhuman::composio::fetch_connected_integrations`]
    /// which is the single source of truth for integration discovery.
    ///
    /// **No session-scoped Composio client is cached on the agent any
    /// more (#1710 Wave 2)**. Every downstream caller that needs to
    /// dispatch a Composio action now resolves a fresh client via
    /// [`crate::openhuman::composio::client::create_composio_client`]
    /// at call time so the live `composio.mode` toggle is honoured
    /// without rebuilding the session — see `ComposioActionTool`,
    /// `ProviderContext::execute`, the 5 migrated agent tools in
    /// `composio/tools.rs`, and the spawn-time per-action tool build
    /// path in `subagent_runner/ops.rs`.
    pub async fn fetch_connected_integrations(&mut self) {
        let config = match self.integration_runtime_config.clone() {
            Some(config) => config,
            None => match crate::openhuman::config::Config::load_or_init().await {
                Ok(config) => config,
                Err(e) => {
                    log::debug!(
                        "[agent] skipping connected integrations fetch: config load failed: {e}"
                    );
                    return;
                }
            },
        };
        self.connected_integrations =
            crate::openhuman::composio::fetch_connected_integrations(&config).await;
        self.connected_integrations_initialized = true;
    }

    /// Re-synthesise `delegate_*` tools for the orchestrator's `subagents`
    /// declaration using the live `connected_integrations` slice, and
    /// reconcile the resulting set into `self.tools` / `self.tool_specs` /
    /// `self.visible_tool_specs` / `self.visible_tool_names`.
    ///
    /// **Reconciliation strategy** — full rebuild of the synthesised
    /// subset:
    ///
    ///   1. Drop every tool whose name was in [`Self::synthesized_tool_names`]
    ///      from the previous synthesis. Direct tools (`query_memory`,
    ///      `cron_add`, …) are untouched because their names are not in
    ///      that set.
    ///   2. Append the freshly collected synthesis output verbatim.
    ///   3. Replace `synthesized_tool_names` with the new set so the
    ///      next refresh has a clean mask to undo.
    ///
    /// This is safer than appending-only or strict-diff reconcile:
    ///
    ///   * Stale tools after a revoke can never leak — anything from the
    ///     previous synthesis is unconditionally dropped, the new set is
    ///     authoritative.
    ///   * Direct tools can never be accidentally removed — only names
    ///     in `synthesized_tool_names` are touched.
    ///   * Duplicate registration is impossible — retain+extend
    ///     guarantees every final entry is either a non-synthesised
    ///     direct tool or a member of the fresh `synthed` set.
    ///
    /// **When to call**: on turn 1 only when the session was built
    /// without a prewarmed Composio cache snapshot, and on any
    /// subsequent turn where the connection set has changed since the
    /// last reconcile (detected via
    /// [`Self::last_seen_integrations_hash`] vs.
    /// [`crate::openhuman::composio::cached_active_integrations`]).
    ///
    /// **Atomicity**: when `Arc::get_mut` fails (a sub-agent or other
    /// caller has already captured a clone of the tool list), we restore
    /// the previous `synthesized_tool_names` and bail. The next refresh
    /// attempt will re-apply the full transition cleanly rather than
    /// resuming from a partial state. This should never happen on a turn
    /// boundary in production — sub-agents always drop their snapshots
    /// before the parent's next turn — but it's defended against anyway.
    ///
    /// **Return value** — `true` when the agent's tool surface is now
    /// consistent with `self.connected_integrations` (either because a
    /// successful reconcile applied, or because no reconcile was needed).
    /// `false` only when the Arc was shared and the reconcile was
    /// aborted; callers should treat this as "retry next turn" and
    /// **not** advance any signal they use to gate future refreshes
    /// (e.g. `last_seen_integrations_hash`) — otherwise a one-shot
    /// shared-Arc collision could suppress further reconciliation until
    /// another integration event happened to bump the hash again.
    pub fn refresh_delegation_tools(&mut self) -> bool {
        use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
        use crate::openhuman::tools::orchestrator_tools::collect_orchestrator_tools;

        let Some(reg) = AgentDefinitionRegistry::global() else {
            // No registry — there's nothing we can do until the
            // registry is initialised. The agent's surface stays at
            // whatever the builder produced; callers can safely treat
            // this as "no reconcile needed right now".
            return true;
        };
        let Some(def) = reg.get(&self.agent_definition_id) else {
            log::debug!(
                "[agent] refresh_delegation_tools: definition '{}' not in registry — skipping",
                self.agent_definition_id
            );
            return true;
        };
        if def.subagents.is_empty() {
            return true;
        }

        let synthed = collect_orchestrator_tools(def, reg, &self.connected_integrations);
        let synthed_names: std::collections::HashSet<String> =
            synthed.iter().map(|t| t.name().to_string()).collect();
        let synthed_specs: Vec<crate::openhuman::tools::ToolSpec> =
            synthed.iter().map(|t| t.spec()).collect();

        // Skip the Arc mutation entirely when neither the previous nor
        // the next synthesis produced any names — saves an
        // `Arc::get_mut` probe on agents that don't declare `subagents`.
        if self.synthesized_tool_names.is_empty() && synthed_names.is_empty() {
            return true;
        }

        // Mask used to drop the previous synthesis. We `take` it now so
        // the restore-on-failure path below can put it back if the Arc
        // turns out to be shared.
        let old_synth = std::mem::take(&mut self.synthesized_tool_names);

        match (
            Arc::get_mut(&mut self.tools),
            Arc::get_mut(&mut self.tool_specs),
        ) {
            (Some(tools_vec), Some(specs_vec)) => {
                tools_vec.retain(|t| !old_synth.contains(t.name()));
                specs_vec.retain(|s| !old_synth.contains(&s.name));
                tools_vec.extend(synthed);
                specs_vec.extend(synthed_specs);
            }
            _ => {
                log::warn!(
                    "[agent] refresh_delegation_tools: tools/tool_specs Arc is shared — \
                     cannot reconcile delegation surface (would have produced {} synthesised tool(s)). \
                     Restoring previous synthesized_tool_names so the next refresh retries cleanly.",
                    synthed_names.len()
                );
                self.synthesized_tool_names = old_synth;
                return false;
            }
        }

        // `visible_tool_names` carries an explicit allowlist for
        // [`ToolScope::Named`] agents. Drop the previously-synthesised
        // names and add the new ones so the visible set tracks the
        // tool list. Wildcard-scope agents keep this empty ("no
        // filter") and never need touching.
        if !self.visible_tool_names.is_empty() {
            for name in &old_synth {
                self.visible_tool_names.remove(name);
            }
            for name in &synthed_names {
                self.visible_tool_names.insert(name.clone());
            }
        }

        // Rebuild the visible-spec cache from the new tool_specs so the
        // next provider call carries the reconciled schema. Dedup
        // afterward so a delegate synthesised here (e.g.
        // `delegate_name = "research"`) doesn't collide with a
        // same-named skill tool on the wire — Anthropic 400s on dup
        // tool names where OpenHuman's backend silently accepts.
        self.rebuild_tool_policy_session();

        // Compute add/remove deltas for the log line — useful when
        // diagnosing a Composio connect/revoke that should have rebuilt
        // the surface but didn't. Materialise to owned `Vec<String>`
        // so we can move `synthed_names` into `self.synthesized_tool_names`
        // below without the log-statement reborrow blocking the move.
        let added: Vec<String> = synthed_names
            .iter()
            .filter(|n| !old_synth.contains(n.as_str()))
            .cloned()
            .collect();
        let removed: Vec<String> = old_synth
            .iter()
            .filter(|n| !synthed_names.contains(n.as_str()))
            .cloned()
            .collect();

        self.synthesized_tool_names = synthed_names;

        log::info!(
            "[agent] refresh_delegation_tools: reconciled delegation surface for agent '{}' (display='{}'); now {} synthesised tool(s); added={:?} removed={:?}",
            self.agent_definition_id,
            self.agent_definition_name,
            self.synthesized_tool_names.len(),
            added,
            removed
        );
        true
    }

    /// Builds the system prompt for the current turn, including tool
    /// instructions and learned context.
    pub fn build_system_prompt(&self, learned: LearnedContextData) -> Result<String> {
        let tools_slice: &[Box<dyn Tool>] = self.tools.as_slice();
        let instructions = self
            .tool_dispatcher
            .prompt_instructions_for_specs(self.visible_tool_specs.as_slice())
            .unwrap_or_else(|| self.tool_dispatcher.prompt_instructions(tools_slice));
        // Adapt the owned Box<dyn Tool> slice into the shared PromptTool
        // shape that every prompt-building call-site uses. Temporary vec
        // borrows from `tools_slice` and lives for the duration of the
        // prompt build.
        let prompt_tools = PromptTool::from_tools(tools_slice);
        let prompt_visible_tool_names = self.tool_policy_session.visible_tool_names_for_prompt();
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            agent_id: &self.agent_definition_name,
            tools: &prompt_tools,
            skills: &self.skills,
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &prompt_visible_tool_names,
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            connected_integrations: &self.connected_integrations,
            connected_identities_md: crate::openhuman::agent::prompts::render_connected_identities(
            ),
            include_profile: !self.omit_profile,
            include_memory_md: !self.omit_memory_md,
            curated_snapshot: None,
            user_identity: crate::openhuman::app_state::peek_cached_current_user_identity(),
            // TODO(phase-2): Wire personality context into the live agent turn.
            // Currently personalities only take effect during delegate_to_personality sub-agent runs.
            // To activate: load the active profile via AgentProfileStore::resolve(), build
            // PersonalityContext::from_profile(), and populate these fields.
            personality_soul_md: None, // TODO: personality_ctx.soul_md_override
            personality_memory_md: None, // TODO: personality_ctx.memory_md_override
            personality_roster: vec![], // TODO: build_personality_roster(&workspace_dir)
            workflows: &self.workflows,
        };
        // Route through the global context manager so every
        // prompt-building call-site — main agent, sub-agent runner,
        // channel runtimes — shares one builder configuration.
        let mut prompt = self.context.build_system_prompt(&ctx)?;
        if let Some(boundary) = render_tool_policy_boundary(&self.tool_policy_session, 2048) {
            prompt = format!("{boundary}\n\n{prompt}");
        }
        Ok(prompt)
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
                        let loaded_count = session.messages.len();
                        log::info!("[transcript] loaded {} messages for resume", loaded_count);
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

    /// Ask the provider for a resumable checkpoint summary when a turn
    /// hits the tool-call iteration cap, with native tools **disabled** so
    /// the model returns prose rather than another tool call. Streams text
    /// deltas to the progress sink (when attached) so the checkpoint
    /// appears in the UI like any other reply.
    ///
    /// Returns the summary text (empty when the provider call fails or
    /// yields nothing — the caller then falls back to
    /// [`build_deterministic_checkpoint`] so the thread is never left on an
    /// unterminated tool cycle, bug-report-2026-05-26 A1) **paired with the
    /// provider usage** for this extra call, so the caller can fold it into
    /// the turn's cumulative token/cost accounting instead of silently
    /// dropping it.
    async fn summarize_iteration_checkpoint(
        &self,
        base_messages: &[ChatMessage],
        effective_model: &str,
        iteration_for_stream: u32,
    ) -> (String, Option<UsageInfo>) {
        let mut messages = base_messages.to_vec();
        messages.push(ChatMessage::user(MAX_ITER_CHECKPOINT_INSTRUCTION));

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
            thread_id: crate::openhuman::inference::provider::thread_context::current_thread_id(),
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
    pub(super) async fn spawn_session_memory_extraction(&mut self) {
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
    pub(super) fn spawn_transcript_ingestion(&self) {
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

/// Wrapper around
/// [`crate::openhuman::memory_tree::tree_runtime::store::collect_root_summaries_with_caps`]
/// that takes user-resolved per-namespace and total caps. The actual
/// limits are derived from the active
/// [`crate::openhuman::config::schema::agent::MemoryContextWindow`]
/// preset by [`crate::openhuman::config::schema::agent::AgentConfig::resolved_memory_limits`].
fn collect_tree_root_summaries(
    workspace_dir: &std::path::Path,
    per_namespace_cap: usize,
    total_cap: usize,
) -> Vec<NamespaceSummary> {
    crate::openhuman::memory_tree::tree_runtime::store::collect_root_summaries_with_caps(
        workspace_dir,
        per_namespace_cap,
        total_cap,
    )
    .into_iter()
    .map(|(namespace, body, updated_at)| NamespaceSummary {
        namespace,
        body,
        updated_at,
    })
    .collect()
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
