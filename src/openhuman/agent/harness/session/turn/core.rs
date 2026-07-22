//! Core turn execution: the main `turn()` method and `inject_agent_experience_context()`.

use super::super::types::Agent;
use super::{
    integration_announcement_note, mcp_announcement_note, newly_connected_slugs,
    skill_announcement_note, skill_retraction_note,
};
use crate::openhuman::agent::harness;
use crate::openhuman::agent::harness::definition::TriggerMemoryAgent;
use crate::openhuman::agent::harness::fork_context::ParentExecutionContext;
use crate::openhuman::agent::hooks::{self, TurnContext};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent_experience::{
    prepend_experience_block, render_experience_hits, AgentExperienceStore, ExperienceQuery,
};
use crate::openhuman::agent_memory::memory_loader::collect_recall_citations;
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage};
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::util::truncate_with_ellipsis;

use anyhow::Result;
use std::hash::{Hash, Hasher};

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
/// - `user_message` — the request is not an obvious context-free greeting or
///   simple local action. Super context is useful when prior memory, profile,
///   integrations, or web facts can change the answer; it is counterproductive
///   for "hi"/"ciao" and straightforward local filesystem commands, where an
///   auto-run scout only adds tool noise before the orchestrator sees intent.
/// - `native_tool_calling` — provider-aware guardrail for #4361. Local
///   providers (Ollama / LM Studio / MLX / llama.cpp) force
///   `native_tool_calling=false`, so the harness serializes the **entire tool +
///   integration catalog as prose** into the system prompt (the
///   `PFormatToolDispatcher` path, `should_send_tool_specs()==false`). A weak
///   local model then over-selects from that text menu and mis-routes a
///   greeting into `composio_list_connections` or a local folder op into the
///   calendar path (the reported v0.58 regression). For these providers we only
///   auto-run the scout when the user *explicitly* asks for prior context or a
///   connected integration; otherwise the extra scout just enlarges the surface
///   the model can trip over. Native-tool-calling providers keep the broader
///   behavior — structured tool specs make spurious tool-routing far rarer.
///
/// Pulled out as a pure function so the gate (in particular the resume and
/// orchestrator guards) is unit-testable without a full agent turn harness.
fn super_context_skip_reason(
    user_message: &str,
    native_tool_calling: bool,
) -> Option<&'static str> {
    let normalized = user_message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| c.is_ascii_punctuation())
        .to_ascii_lowercase();

    if normalized.is_empty() {
        return Some("empty_message");
    }

    if matches!(
        normalized.as_str(),
        "hi" | "hello"
            | "hey"
            | "yo"
            | "ciao"
            | "hola"
            | "bonjour"
            | "thanks"
            | "thank you"
            | "ok"
            | "okay"
            | "good morning"
            | "good afternoon"
            | "good evening"
            | "good night"
    ) {
        return Some("context_free_greeting");
    }

    let local_action_candidate = strip_local_folder_action_lead_in(&normalized);
    let starts_like_local_folder_action = [
        // English
        "create a folder ",
        "create folder ",
        "make a folder ",
        "make folder ",
        "create a directory ",
        "create directory ",
        "make a directory ",
        "make directory ",
        "mkdir ",
        // Italian (#4361 repro: "Crea una cartella sul Desktop e chiamala
        // PROVA"). Gemma-class local models mis-route the Italian phrasing into
        // Calendar/Connections exactly as they do the English one, so the same
        // folder-op suppression must be language-aware for the reported locale.
        "crea una cartella ",
        "crea la cartella ",
        "crea cartella ",
        "creare una cartella ",
        "creare cartella ",
        "crea una directory ",
        "crea directory ",
        "fai una cartella ",
        "fammi una cartella ",
        "nuova cartella ",
    ]
    .iter()
    .any(|prefix| local_action_candidate.starts_with(prefix));

    if starts_like_local_folder_action {
        let context_hints = [
            "discussed",
            "mentioned",
            "previous",
            "earlier",
            "last time",
            "email",
            "gmail",
            "calendar",
            "slack",
            "notion",
            "github",
            "drive",
            // Italian cues, mirroring the Italian folder-op detection above so a
            // folder request that *does* reference prior context or an
            // integration still earns a scout (#4361).
            "discusso",
            "parlato",
            "calendario",
            "precedente",
        ];
        if !context_hints.iter().any(|hint| normalized.contains(hint)) {
            return Some("simple_local_filesystem_action");
        }
    }

    // Provider-aware guardrail (#4361). On providers without native tool calling
    // the whole tool/integration catalog is injected as prose, so an unrequested
    // first-turn scout is exactly what tips a small local model into spurious
    // integration tool-calls. Suppress super context for these providers unless
    // the prompt explicitly references prior context or a connected integration
    // (e.g. "show my connections", "schedule a meeting tomorrow at 3"), where the
    // scout genuinely helps. Native-tool-calling providers are unaffected.
    if !native_tool_calling && !mentions_context_or_integration(&normalized) {
        return Some("non_native_provider_no_explicit_intent");
    }

    None
}

/// True when a first-turn prompt explicitly references prior conversation
/// context or a connected integration — the cases where a context scout earns
/// its cost. Used to keep super context ON for these prompts even on local
/// (non-native-tool-calling) providers, where it is otherwise suppressed to
/// avoid tipping weak models into spurious integration tool-calls (#4361).
///
/// `normalized` must already be whitespace-collapsed, punctuation-trimmed, and
/// lowercased by `super_context_skip_reason`.
fn mentions_context_or_integration(normalized: &str) -> bool {
    const INTENT_HINTS: [&str; 37] = [
        // prior-conversation / memory cues
        "discussed",
        "mentioned",
        "previous",
        "earlier",
        "last time",
        "remember",
        "we talked",
        "you said",
        "my profile",
        // connection / integration cues
        "connection",
        "connections",
        "integration",
        "integrations",
        "connected",
        // calendar / scheduling
        "calendar",
        "schedule",
        "meeting",
        "reminder",
        "remind me",
        "event",
        // mail
        "email",
        "gmail",
        "inbox",
        // common connected apps
        "slack",
        "notion",
        "github",
        "drive",
        "linkedin",
        "whatsapp",
        "telegram",
        // Italian cues (#4361) — keep super context ON for an explicit
        // context/integration ask in the reported locale. "calendario"/"email"/
        // "gmail" already match via substring, so only the non-overlapping stems
        // are listed here. Additive only: these can never *cause* a skip.
        "connessione",  // connection
        "connessioni",  // connections
        "integrazione", // integration
        "riunione",     // meeting
        "promemoria",   // reminder
        "ricordami",    // remind me
        "agenda",       // agenda / calendar
    ];
    INTENT_HINTS.iter().any(|hint| normalized.contains(hint))
}

fn strip_local_folder_action_lead_in(message: &str) -> &str {
    let mut candidate = message;
    for _ in 0..3 {
        let trimmed = candidate.trim_start();
        let next = [
            // English
            "can you please ",
            "could you please ",
            "would you please ",
            "can you ",
            "could you ",
            "would you ",
            "please ",
            "hey ",
            "hello ",
            "hi ",
            // Italian (#4361)
            "puoi per favore ",
            "potresti per favore ",
            "puoi per piacere ",
            "puoi ",
            "potresti ",
            "per favore ",
            "per piacere ",
            "ciao ",
            "ehi ",
        ]
        .iter()
        .find_map(|lead_in| trimmed.strip_prefix(lead_in));

        match next {
            Some(rest) => candidate = rest,
            None => return trimmed,
        }
    }
    candidate.trim_start()
}

fn super_context_base_gate(
    is_orchestrator: bool,
    first_turn: bool,
    has_prior_conversation: bool,
    enabled: bool,
) -> bool {
    is_orchestrator && first_turn && !has_prior_conversation && enabled
}

fn should_run_super_context(
    is_orchestrator: bool,
    first_turn: bool,
    has_prior_conversation: bool,
    enabled: bool,
    native_tool_calling: bool,
    user_message: &str,
) -> bool {
    super_context_base_gate(is_orchestrator, first_turn, has_prior_conversation, enabled)
        && super_context_skip_reason(user_message, native_tool_calling).is_none()
}

// `parse_context_bundle_has_enough_context` moved to
// `tinyagents::middleware` alongside the `SuperContextMiddleware` graph node
// that now owns the first-turn context-collection pass (#4249).

/// Flatten the assistant tool calls a turn produced into [`ToolCallRecord`]s for
/// post-turn hooks + the deterministic cap checkpoint. Per-call success +
/// sanitized output summary are recovered from the turn's captured
/// [`ToolCallOutcome`]s (correlated by provider call id), since the harness folds
/// a tool result into a `Message::tool` that drops its failure flag — matching the
/// engine's honest per-call accounting instead of recording every call as ok.
fn tool_records_from_conversation(
    conversation: &[ConversationMessage],
    tool_outcomes: &[crate::openhuman::tinyagents::ToolCallOutcome],
) -> Vec<hooks::ToolCallRecord> {
    let mut records = Vec::new();
    for msg in conversation {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = msg {
            for call in tool_calls {
                let outcome = tool_outcomes.iter().find(|o| o.call_id == call.id);
                // Default a MISSING outcome to `false` (#4467, item 7): a call
                // with no captured outcome is a hallucinated/unknown tool the
                // crate recovered via `ReturnToolError` without running
                // `after_tool` (so the capture sink never saw it). Recording it as
                // succeeded misreports the timeline; real executed tools always
                // have an outcome, so this only flips the genuinely-unknown case.
                let success = outcome.map(|o| o.success).unwrap_or(false);
                let output_summary = outcome
                    .map(|o| hooks::sanitize_tool_output(&o.content, &call.name, success))
                    .unwrap_or_default();
                records.push(hooks::ToolCallRecord {
                    name: call.name.clone(),
                    arguments: serde_json::from_str(&call.arguments)
                        .unwrap_or(serde_json::Value::Null),
                    success,
                    output_summary,
                    duration_ms: 0,
                });
            }
        }
    }
    records
}

/// Stamp each **failed** tool-result [`ChatMessage`] with its failure outcome
/// before persistence, so the derived transcript view can render an error tool
/// row instead of a false success.
///
/// The harness folds a tool result into a `role:"tool"` message whose native
/// content envelope (`{"tool_call_id":…,"content":…}`) has already dropped
/// `ToolResult::is_error`. The only structured per-call success signal is the
/// captured [`ToolCallOutcome`] side-channel; correlate by provider call id and
/// re-attach an additive failure marker (see
/// `transcript::attach_tool_failure_metadata`). Non-tool messages, tool messages
/// with no matching outcome, and successful calls are left untouched.
fn stamp_tool_failures(
    messages: &mut [ChatMessage],
    tool_outcomes: &[crate::openhuman::tinyagents::ToolCallOutcome],
) {
    use crate::openhuman::agent::harness::session::transcript;
    if tool_outcomes.is_empty() {
        return;
    }
    for msg in messages.iter_mut() {
        if msg.role != "tool" {
            continue;
        }
        let Some(call_id) = parse_tool_call_id(&msg.content) else {
            continue;
        };
        let Some(outcome) = tool_outcomes.iter().find(|o| o.call_id == call_id) else {
            continue;
        };
        if outcome.success {
            continue;
        }
        let detail = short_failure_detail(&outcome.content);
        log::debug!(
            "[transcript] stamping tool failure call_id={call_id} name={}",
            outcome.name
        );
        transcript::attach_tool_failure_metadata(msg, detail.as_deref());
    }
}

/// Extract the `tool_call_id` from a native tool-result content envelope
/// (`{"tool_call_id":…,"content":…}`). `None` for non-envelope content (XML /
/// P-Format dispatchers, which don't emit `role:"tool"` messages anyway).
fn parse_tool_call_id(content: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    value.get("tool_call_id")?.as_str().map(str::to_string)
}

/// Reduce a tool's error output to a short, single-line reason for display.
fn short_failure_detail(content: &str) -> Option<String> {
    const MAX: usize = 160;
    let line = content.lines().map(str::trim).find(|l| !l.is_empty())?;
    let short: String = line.chars().take(MAX).collect();
    if short.is_empty() {
        None
    } else {
        Some(short)
    }
}

/// Rewrite the **trailing** assistant `Chat` message in `history` to `text`,
/// keeping the persisted transcript and the next turn's KV-cache prefix
/// consistent with a repaired required-output reply (issue #4117). Only the last
/// row is touched — when the tail is not an assistant `Chat` (defensive; a clean
/// finish, a cap checkpoint, and the #4093 close all end on one) a fresh
/// assistant message is appended rather than mutating an older entry.
fn replace_last_assistant_reply(history: &mut Vec<ConversationMessage>, text: &str) {
    match history.last_mut() {
        Some(ConversationMessage::Chat(chat)) if chat.role == "assistant" => {
            chat.content = text.to_string();
        }
        _ => history.push(ConversationMessage::Chat(ChatMessage::assistant(
            text.to_string(),
        ))),
    }
}

fn render_agent_context_status_note(sources: &[harness::AgentContextPreparedSource]) -> String {
    let sources = if sources.is_empty() {
        "the OpenHuman harness".to_string()
    } else {
        sources
            .iter()
            .map(|source| source.source.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "## Agent context status\n\nAgent context retrieval/preparation has already run once \
         for this turn in code via {sources}. Do not call `agent_prepare_context` again for \
         general context preparation. Use the prepared context below, and call only specific \
         follow-up tools if a concrete missing detail is required."
    )
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
            // Captured *before* `tokio::spawn` — the ambient thread id is a
            // `tokio::task_local` (see `inference::provider::thread_context`)
            // and does not propagate into a spawned task, so it must be read
            // on this (still-scoped) task and moved in explicitly. Tagging
            // this document with the live chat thread id is what lets the
            // same-session exclusion filter (`UnifiedMemory::recall` /
            // `memory_hybrid_search`) recognize and drop it later this same
            // turn, so the agent's own on-demand memory search doesn't echo
            // its own triggering request back as a "relevant" result.
            let session_id_for_autosave =
                crate::openhuman::inference::provider::thread_context::current_thread_id();
            log::debug!(
                "[agent_autosave] enqueue user-message store key={autosave_key} chars={chars} \
                 session_id={}",
                session_id_for_autosave.as_deref().unwrap_or("<none>")
            );
            tokio::spawn(async move {
                match memory
                    .store(
                        "",
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
            crate::openhuman::agent_orchestration::running_subagents::active_subagents_context_block(
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
        let (enriched, memory_agent_context_injected) = self
            .inject_triggered_memory_agent_context(user_message, enriched, &parent_context)
            .await;
        if memory_agent_context_injected {
            agent_context_prepared_sources.push(harness::AgentContextPreparedSource {
                source: "memory agent context retrieval".to_string(),
                has_enough_context: None,
            });
        }

        // ── "Super context": harness-driven first-turn context collection ──
        // When enabled (config `context.super_context_enabled`, surfaced as the
        // composer toggle), run the read-only `context_scout` BEFORE the
        // orchestrator LLM gets the turn, and fold its bounded
        // `[context_bundle]` into the user message. This is the harness driving
        // the collection deterministically — unlike the `agent_prepare_context`
        // tool, which the model chooses to call. If this path succeeds, the
        // turn prompt and task-local marker tell `agent_prepare_context` not
        // to run another generic scout pass in the same turn.
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
        // `should_send_tool_specs()` is true only when the provider receives a
        // structured tool schema (native tool calling). For local providers it
        // is false — the whole tool/integration catalog is prose in the prompt,
        // which is the surface a weak model mis-routes on (#4361).
        let native_tool_calling = self.tool_dispatcher.should_send_tool_specs();
        let skip_reason_for_logging = super_context_skip_reason(user_message, native_tool_calling);
        let base_gate = super_context_base_gate(
            self.agent_definition_id == "orchestrator",
            first_turn,
            has_prior_conversation,
            self.context.super_context_enabled(),
        );
        if base_gate {
            if let Some(reason) = skip_reason_for_logging {
                log::info!(
                    "[agent_loop] super_context skipped for context-free first turn reason={reason} native_tool_calling={native_tool_calling}"
                );
            }
        }
        // The scout no longer runs here imperatively: super context is now a
        // before_model **graph node** (`SuperContextMiddleware`, installed via
        // `context_mw.super_context` below). It runs the read-only `context_scout`
        // on the first model call, folds the `[context_bundle]` into the user
        // message, and registers its prepared-context source live so a later
        // `agent_prepare_context` call self-suppresses. We only decide *whether*
        // to enable the node here (the gate is unchanged).
        let run_super_context = should_run_super_context(
            self.agent_definition_id == "orchestrator",
            first_turn,
            has_prior_conversation,
            self.context.super_context_enabled(),
            native_tool_calling,
            user_message,
        );
        if run_super_context {
            log::info!(
                "[agent_loop] super_context enabled — installing the SuperContextMiddleware graph node (new thread, first turn)"
            );
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
                run_super_context,
                artifact_store,
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
                crate::openhuman::thread_goals::runtime::GoalBudgetStopHook::for_goal(
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
        // Whether the super-context graph node should run this turn (gate decided
        // by `should_run_super_context` in `turn()`, before the user row was
        // pushed to history — so it can't be recomputed here).
        run_super_context: bool,
        artifact_store: Option<
            crate::openhuman::agent::harness::tool_result_artifacts::ToolResultArtifactStore,
        >,
    ) -> Result<String> {
        let turn_started = std::time::Instant::now();
        // This turn's stamped user message is already the last entry in
        // `self.history` (pushed by `turn()` before the engine branch), so build
        // the provider messages straight from history — do NOT push the user
        // again. When a cached transcript prefix is present (a resumed session's
        // KV-cache warm-up), prepend it and clear it so the first request reuses
        // the cached prefix exactly once.
        let mut messages = self.tool_dispatcher.to_provider_messages(&self.history);
        if let Some(cached) = self.cached_transcript_messages.take() {
            // The cached prefix already carries the system prompt + prior
            // conversation, so drop the freshly-rendered leading system
            // message(s) and append only this turn's new (user) messages.
            let tail = messages
                .into_iter()
                .skip_while(|m| m.role == "system")
                .collect::<Vec<_>>();
            let mut combined = cached;
            combined.extend(tail);
            messages = combined;
        }

        // Multimodal prep (parity with the legacy engine): rehydrate image
        // placeholders for vision-capable providers, then expand `[IMAGE:…]` /
        // `[FILE:…]` markers into provider-ready content before dispatch. The
        // expanded copy is provider-only and never persisted to `history`.
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

        tracing::info!(
            model = %effective_model,
            max_iterations,
            tools = self.tools.len(),
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
        let context_mw = crate::openhuman::tinyagents::TurnContextMiddleware {
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
            // Super context (first-turn read-only context collection) as a graph
            // node — enabled only when its gate passed above. The node runs the
            // scout on the first model call and folds the bundle into the message.
            super_context: run_super_context.then(|| {
                crate::openhuman::tinyagents::SuperContextConfig {
                    user_message: user_message.to_string(),
                }
            }),
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
        let (outcome, subagent_usage_entries) =
            crate::openhuman::agent::harness::turn_subagent_usage::with_turn_collector(
                super::graph::run_chat_turn_graph(super::graph::ChatTurnGraph {
                    turn_models,
                    model: effective_model.to_string(),
                    messages,
                    tools: self.tools.clone(),
                    visible_tool_names: self.visible_tool_names.clone(),
                    max_iterations,
                    on_progress: self.on_progress.clone(),
                    context_window,
                    run_queue: self.run_queue.clone(),
                    context_mw,
                    // Enforce the builder-configured tool policy at the tool
                    // boundary (the tinyagents path otherwise bypasses it).
                    tool_policy: Some(crate::openhuman::tinyagents::ToolPolicyEnforcement {
                        policy: self.tool_policy.clone(),
                        session: self.tool_policy_session.clone(),
                        session_id: self.event_session_id.clone(),
                        channel: self.event_channel().to_string(),
                        agent_definition_id: self.agent_definition_id.clone(),
                    }),
                }),
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

        let reply = if outcome.hit_cap {
            // The loop paused at the tool-call cap. Ask the model for a resumable
            // checkpoint (tools disabled), falling back to a deterministic
            // done/next summary so the thread never ends on a dangling tool
            // cycle. Fold the extra call's usage into the turn accounting.
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
                    &tool_records_from_conversation(&outcome.conversation, &outcome.tool_outcomes),
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
        } else {
            outcome.text.clone()
        };

        // Enforce the required structured-output contract (issue #4117) on the
        // accepted reply — for ALL of the branches above (normal finish, cap
        // checkpoint, #4093 synthesized close), since each delivers a reply
        // downstream parsing depends on. When this agent must emit a JSON block
        // every turn and the reply omitted it, validate-and-repair before the
        // turn is accepted, reconciling with streaming (append-only when a live
        // stream is attached, replace otherwise — see `enforce_required_output`).
        // The trailing assistant message is rewritten to match, and the repair
        // call's usage is folded into the turn accounting. `required_output`
        // defaults to `None`, so existing agents are entirely unaffected.
        let reply = if let Some(contract) = self.config.required_output.clone() {
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
        crate::openhuman::thread_goals::runtime::account_turn_against_goal(
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
            .integration_runtime_config
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
            let _ = self
                .memory
                .store("", "assistant_resp", &summary, MemoryCategory::Daily, None)
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
    ) -> (String, bool) {
        const MEMORY_AGENT_ID: &str = "agent_memory";
        const MAX_MEMORY_AGENT_BLOCK_CHARS: usize = 8000;

        if self.trigger_memory_agent != TriggerMemoryAgent::Always {
            log::debug!(
                "[agent_memory:trigger] skipped agent_id={} policy={:?}",
                self.agent_definition_id,
                self.trigger_memory_agent
            );
            return (enriched, false);
        }

        if self.agent_definition_id == MEMORY_AGENT_ID {
            log::debug!("[agent_memory:trigger] skipped recursive memory agent invocation");
            return (enriched, false);
        }

        let Some(registry) = harness::AgentDefinitionRegistry::global() else {
            log::warn!(
                "[agent_memory:trigger] AgentDefinitionRegistry unavailable; continuing without memory agent context"
            );
            return (enriched, false);
        };
        let Some(definition) = registry.get(MEMORY_AGENT_ID).cloned() else {
            log::warn!(
                "[agent_memory:trigger] `{MEMORY_AGENT_ID}` definition unavailable; continuing without memory agent context"
            );
            return (enriched, false);
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
                    return (enriched, false);
                }
                (
                    format!(
                        "## Memory agent context\n\n{}\n\n---\n\n{}",
                        output.trim(),
                        enriched
                    ),
                    true,
                )
            }
            Err(err) => {
                log::warn!(
                    "[agent_memory:trigger] failed agent_id={} task_id={}: {err:#}",
                    self.agent_definition_id,
                    task_id
                );
                (enriched, false)
            }
        }
    }
}

#[cfg(test)]
mod super_context_gate_tests {
    use super::{
        mentions_context_or_integration, render_agent_context_status_note,
        should_run_super_context, super_context_skip_reason,
    };
    use crate::openhuman::agent::harness::AgentContextPreparedSource;

    // Provider-aware convenience: `native_tool_calling` (5th arg) is `true` for
    // these tests unless a case is specifically exercising the local
    // (non-native-tool-calling) provider path (#4361). Native providers keep the
    // original #4361/#4403 gate semantics.
    const NATIVE: bool = true;
    const LOCAL: bool = false;

    #[test]
    fn runs_only_on_first_turn_of_a_new_orchestrator_thread_when_enabled() {
        // Orchestrator, new thread, first turn, flag on → run.
        assert!(should_run_super_context(
            true,
            true,
            false,
            true,
            NATIVE,
            "find the project we discussed yesterday"
        ));
    }

    #[test]
    fn skips_when_flag_disabled() {
        assert!(!should_run_super_context(
            true,
            true,
            false,
            false,
            NATIVE,
            "find the project we discussed yesterday"
        ));
    }

    #[test]
    fn skips_on_later_turns() {
        // history non-empty → not the first turn.
        assert!(!should_run_super_context(
            true,
            false,
            false,
            true,
            NATIVE,
            "find the project we discussed yesterday"
        ));
    }

    #[test]
    fn skips_on_cold_resumed_thread_even_on_first_turn() {
        // Regression: a thread resumed cold has an empty `history` (so
        // `first_turn` is true) but a seeded prefix that includes a prior
        // assistant reply. Super context must NOT re-fire on these existing
        // conversations.
        assert!(!should_run_super_context(
            true,
            true,
            true,
            true,
            NATIVE,
            "find the project we discussed yesterday"
        ));
    }

    #[test]
    fn runs_for_attachment_first_new_thread_with_lone_seeded_user_row() {
        // Regression: an attachment-first new thread can seed a single just-
        // persisted *user* row (no assistant reply), so `has_prior_conversation`
        // is false. That is still a brand-new conversation — super context
        // should run.
        assert!(should_run_super_context(
            true,
            true,
            false,
            true,
            NATIVE,
            "[IMAGE: screenshot.png]\nwhat is this?"
        ));
    }

    #[test]
    fn skips_for_non_orchestrator_agents() {
        // Regression: `Agent::turn` is shared with background/automated
        // `run_single()` flows (goals enrichment, cron/task agents,
        // specialist sub-agents). Even on a fresh first turn with the flag on,
        // super context must only run for the user-facing orchestrator.
        assert!(!should_run_super_context(
            false,
            true,
            false,
            true,
            NATIVE,
            "find the project we discussed yesterday"
        ));
    }

    #[test]
    fn skips_context_free_greeting() {
        // #4361 case: a bare greeting must never trigger a scout — on any
        // provider. "Ciao" is the exact prompt from the issue report.
        for native in [NATIVE, LOCAL] {
            assert!(!should_run_super_context(
                true, true, false, true, native, "Ciao"
            ));
            assert!(!should_run_super_context(
                true, true, false, true, native, "hello!"
            ));
            assert_eq!(
                super_context_skip_reason("Ciao", native),
                Some("context_free_greeting")
            );
        }
    }

    #[test]
    fn skips_simple_local_folder_creation() {
        // #4361 case: "create a folder on Desktop called test" must not browse
        // Calendar/Connections — on any provider.
        for native in [NATIVE, LOCAL] {
            assert!(
                !should_run_super_context(
                    true,
                    true,
                    false,
                    true,
                    native,
                    "Create a folder on Desktop called test"
                ),
                "expected skip for local folder creation (native={native})"
            );
            assert!(!should_run_super_context(
                true,
                true,
                false,
                true,
                native,
                "Create a folder on Desktop named PROVA"
            ));
        }
    }

    #[test]
    fn skips_simple_local_folder_creation_with_polite_lead_in() {
        for message in [
            "Please create a folder on Desktop named PROVA",
            "Can you make a directory named invoices",
            "Hey please create folder screenshots",
        ] {
            assert!(
                !should_run_super_context(true, true, false, true, NATIVE, message),
                "expected super context to skip for {message:?}"
            );
        }
    }

    #[test]
    fn keeps_super_context_for_local_action_with_context_hint() {
        assert!(should_run_super_context(
            true,
            true,
            false,
            true,
            NATIVE,
            "Create a folder for the project we discussed yesterday"
        ));
        assert!(should_run_super_context(
            true,
            true,
            false,
            true,
            NATIVE,
            "Please create a folder for the project we discussed yesterday"
        ));
    }

    #[test]
    fn skips_italian_local_folder_creation() {
        // #4361 exact repro: the Italian folder op must be recognized as a
        // simple local filesystem action and NOT trigger a Calendar/Connections
        // scout — on any provider. This is the string from the issue report.
        for native in [NATIVE, LOCAL] {
            assert!(
                !should_run_super_context(
                    true,
                    true,
                    false,
                    true,
                    native,
                    "Crea una cartella sul Desktop e chiamala PROVA"
                ),
                "expected skip for Italian folder creation (native={native})"
            );
            // The reason must be the language-agnostic filesystem classification,
            // not the provider fallback — so it holds even for native providers.
            assert_eq!(
                super_context_skip_reason("Crea una cartella sul Desktop e chiamala PROVA", NATIVE),
                Some("simple_local_filesystem_action")
            );
        }
    }

    #[test]
    fn skips_italian_local_folder_creation_variants_and_polite_lead_ins() {
        for message in [
            "Crea una cartella chiamata PROVA",
            "Crea cartella screenshots",
            "Creare una cartella sul desktop",
            "Fai una cartella per le fatture",
            "Nuova cartella documenti",
            "Puoi creare una cartella chiamata PROVA",
            "Per favore crea una cartella sul Desktop",
            "Ciao puoi creare una cartella chiamata test",
        ] {
            assert_eq!(
                super_context_skip_reason(message, NATIVE),
                Some("simple_local_filesystem_action"),
                "expected filesystem skip for {message:?}"
            );
        }
    }

    #[test]
    fn keeps_super_context_for_italian_local_action_with_context_hint() {
        // An Italian folder op that references prior context / an integration
        // should still earn a scout — the filesystem shortcut must not swallow
        // it. Verified on a native provider (local providers add the separate
        // provider gate, exercised elsewhere).
        assert!(should_run_super_context(
            true,
            true,
            false,
            true,
            NATIVE,
            "Crea una cartella per il progetto di cui abbiamo parlato"
        ));
        assert_eq!(
            super_context_skip_reason("Crea una cartella per la riunione in calendario", NATIVE),
            None
        );
    }

    // ── Provider-aware guardrail (#4361) ────────────────────────────────────
    // On providers without native tool calling (Ollama / LM Studio / MLX /
    // llama.cpp) the whole tool + integration catalog is injected as prose, so
    // an unrequested first-turn scout is what tips weak models into spurious
    // integration tool-calls. For these providers super context runs ONLY when
    // the prompt explicitly references prior context or a connected integration.

    #[test]
    fn local_provider_skips_super_context_for_generic_prompt() {
        // A generic first-turn ask with no context/integration cue: a native
        // provider still scouts (broad behavior preserved), but a local provider
        // suppresses it to keep the prose tool menu from mis-routing.
        let msg = "write me a short poem about the sea";
        assert!(should_run_super_context(
            true, true, false, true, NATIVE, msg
        ));
        assert!(!should_run_super_context(
            true, true, false, true, LOCAL, msg
        ));
        assert_eq!(
            super_context_skip_reason(msg, LOCAL),
            Some("non_native_provider_no_explicit_intent")
        );
        assert_eq!(super_context_skip_reason(msg, NATIVE), None);
    }

    #[test]
    fn keeps_super_context_for_explicit_connection_intent() {
        // #4361 case: "show my connections" is an explicit integration ask →
        // super context is allowed on BOTH provider kinds.
        let msg = "show my connections";
        assert!(should_run_super_context(
            true, true, false, true, NATIVE, msg
        ));
        assert!(should_run_super_context(
            true, true, false, true, LOCAL, msg
        ));
        assert_eq!(super_context_skip_reason(msg, LOCAL), None);
    }

    #[test]
    fn keeps_super_context_for_explicit_scheduling_intent() {
        // #4361 case: "schedule a meeting tomorrow at 3" is an explicit
        // integration ask → super context is allowed when integrations are
        // enabled (the `enabled` flag), on BOTH provider kinds.
        let msg = "schedule a meeting tomorrow at 3";
        assert!(should_run_super_context(
            true, true, false, true, NATIVE, msg
        ));
        assert!(should_run_super_context(
            true, true, false, true, LOCAL, msg
        ));
        // Still gated by the config flag: disabled ⇒ no scout regardless.
        assert!(!should_run_super_context(
            true, true, false, false, LOCAL, msg
        ));
    }

    #[test]
    fn local_provider_keeps_super_context_for_explicit_italian_integration_intent() {
        // Fix B, locale-aware: an explicit Italian integration ask must keep the
        // scout ON even on a local (non-native-tool-calling) provider — the
        // provider gate only suppresses *context-free* first turns. A generic
        // Italian prompt with no cue is still suppressed on local providers.
        for msg in [
            "mostra le mie connessioni",    // show my connections
            "fissa una riunione domani",    // schedule a meeting tomorrow
            "aggiungilo al mio calendario", // add it to my calendar
            "controlla la mia email",       // check my email
        ] {
            assert!(
                should_run_super_context(true, true, false, true, LOCAL, msg),
                "expected local provider to keep super context for {msg:?}"
            );
            assert_eq!(super_context_skip_reason(msg, LOCAL), None);
        }

        // A generic Italian ask (a poem) carries no integration cue → the local
        // provider still suppresses it, while a native provider scouts.
        let generic = "scrivimi una breve poesia sul mare";
        assert!(should_run_super_context(
            true, true, false, true, NATIVE, generic
        ));
        assert_eq!(
            super_context_skip_reason(generic, LOCAL),
            Some("non_native_provider_no_explicit_intent")
        );
    }

    #[test]
    fn mentions_context_or_integration_matches_expected_cues() {
        for hit in [
            "show my connections",
            "schedule a meeting tomorrow at 3",
            "check my gmail inbox",
            "what did we discuss earlier",
            "add it to my calendar",
            "post this to slack",
            // Italian cues (#4361)
            "mostra le mie connessioni",
            "fissa una riunione domani",
            "aggiungilo al mio calendario",
            "impostami un promemoria",
            "controlla la mia email",
        ] {
            assert!(
                mentions_context_or_integration(hit),
                "expected context/integration cue in {hit:?}"
            );
        }
        for miss in [
            "write me a short poem about the sea",
            "what is 2 plus 2",
            "translate hola to english",
            "scrivimi una breve poesia sul mare",
        ] {
            assert!(
                !mentions_context_or_integration(miss),
                "did not expect a cue in {miss:?}"
            );
        }
    }

    #[test]
    fn context_status_note_tells_model_not_to_prepare_context_again() {
        let note = render_agent_context_status_note(&[
            AgentContextPreparedSource {
                source: "memory agent context retrieval".to_string(),
                has_enough_context: None,
            },
            AgentContextPreparedSource {
                source: "super context preparation".to_string(),
                has_enough_context: Some(true),
            },
        ]);

        assert!(note.contains("## Agent context status"));
        assert!(note.contains("already run once"));
        assert!(note.contains("memory agent context retrieval"));
        assert!(note.contains("super context preparation"));
        assert!(note.contains("Do not call `agent_prepare_context` again"));
    }
}
