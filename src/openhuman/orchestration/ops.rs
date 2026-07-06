//! Orchestration wake-graph invocation (stage 4).
//!
//! This is the one thing that lives *outside* the graph on the transport side:
//! DMs arrive asynchronously, the stage-3 ingest subscriber persists them and
//! then asks us to wake the graph for that session. We:
//!
//! 1. **debounce** per session so a burst of DMs produces one graph run,
//! 2. **guard idempotence** via a per-session cursor so a re-trigger with no new
//!    messages does no LLM work and sends no DM,
//! 3. **seed** [`OrchestrationState`] from the stage-3 store (windowed messages +
//!    the counterpart to reply to), and
//! 4. drive [`run_orchestration_graph`] with the production nodes: the front-end
//!    agent (`hint:chat`), a stubbed reasoning core, and the Signal DM sender.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::openhuman::config::Config;

use super::graph::compress::{compression_budget, count_tokens, enforce_budget};
use super::graph::{
    run_orchestration_graph, world_diff, CompressedEntry, EvictionOutcome, ExecuteOutcome,
    OrchestrationRuntime, OrchestrationState, WorldDiffEntry,
};
use super::steering::{
    build_steering_prompt, is_explicit_none, parse_steering_output, ParsedSteering,
};
use super::store;
use super::types::{ChatKind, OrchestrationMessage, OrchestrationSession, SessionEnvelopeV1};

/// Assumed model context window (tokens) for the `context_guard` utilization
/// estimate until per-model resolution is wired. Sized to the reasoning tier.
const ASSUMED_CONTEXT_WINDOW: u64 = 200_000;

/// The pinned local "Subconscious" chat window session id (UI only, stage 7).
const SUBCONSCIOUS_SESSION: &str = "subconscious";
/// System prompt for the offline steering-synthesis call (tool-free by design).
const STEERING_SYNTH_SYSTEM: &str =
    "You are an offline subconscious. You never take actions and never contact anyone. Follow the \
     output contract EXACTLY.";
/// Bounded batch of unreviewed compressed rows / world mutations per review.
const REVIEW_BATCH: u32 = 20;

const LOG: &str = "orchestration";

/// The per-session idempotence cursor key: the highest message seq that has been
/// carried through a completed wake cycle.
fn cursor_key(agent_id: &str, session_id: &str) -> String {
    format!("cursor:{agent_id}:{session_id}")
}

/// Per-session debounce generation counter. Each trigger bumps its session's
/// generation; the delayed task only proceeds if it is still the latest.
fn wake_generations() -> &'static Mutex<HashMap<String, u64>> {
    static GENS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    GENS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Bump the generation for `key` and return the new value.
fn bump_generation(key: &str) -> u64 {
    let mut map = wake_generations().lock().unwrap();
    let gen = map.entry(key.to_string()).or_insert(0);
    *gen += 1;
    *gen
}

/// True if `gen` is still the latest recorded generation for `key`.
fn is_latest_generation(key: &str, gen: u64) -> bool {
    wake_generations()
        .lock()
        .unwrap()
        .get(key)
        .is_some_and(|latest| *latest == gen)
}

/// Debounced entry point called by the stage-3 ingest subscriber on
/// `OrchestrationSessionMessage`. Coalesces a DM burst for one session into a
/// single graph run: the last trigger within `debounce_ms` wins.
pub async fn schedule_wake(agent_id: String, session_id: String, chat_kind: String) {
    let config = match Config::load_or_init().await {
        Ok(c) => c,
        Err(e) => {
            log::warn!(target: LOG, "[orchestration] wake.config_load_failed: {e}");
            return;
        }
    };
    if !config.orchestration.enabled {
        return;
    }
    // The subconscious window is not a wake trigger — it feeds steering (stage 6),
    // not the front-end channel loop.
    if ChatKind::from_str(&chat_kind) == ChatKind::Subconscious {
        return;
    }

    let key = format!("{agent_id}:{session_id}");
    let gen = bump_generation(&key);
    let debounce = config.orchestration.debounce_ms;
    log::debug!(
        target: LOG,
        "[orchestration] wake.scheduled agent={agent_id} session={session_id} gen={gen} debounce_ms={debounce}",
    );

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(debounce)).await;
        if !is_latest_generation(&key, gen) {
            log::debug!(target: LOG, "[orchestration] wake.coalesced key={key} gen={gen}");
            return;
        }
        if let Err(e) = invoke_orchestration_graph(&config, &agent_id, &session_id).await {
            log::warn!(target: LOG, "[orchestration] wake.run_failed session={session_id}: {e}");
        }
    });
}

/// Periodically drain the relay mailbox through orchestration ingest.
///
/// tiny.place relay DMs are delivered to `/messages` (poll-only) and are NOT
/// published to the `/inbox/stream` WebSocket — the backend only streams inbox
/// items for payments/notifications, never `PUT /messages`. So a poller is the
/// actual delivery path: it lists `/messages` and feeds each envelope through
/// the same decrypt → classify → persist → acknowledge pipeline the wake graph
/// consumes. Unlinked senders are skipped without being consumed, so their DMs
/// remain readable by the Messaging UI.
pub fn start_message_drain_supervisor() {
    tokio::spawn(async {
        loop {
            match Config::load_or_init().await {
                Ok(config) => match super::ingest::drain_mailbox_once(&config).await {
                    Ok(n) if n > 0 => {
                        log::debug!(target: LOG, "[orchestration] drain: examined {n} envelope(s)")
                    }
                    Ok(_) => {}
                    Err(e) => log::debug!(target: LOG, "[orchestration] drain error: {e}"),
                },
                Err(e) => log::debug!(target: LOG, "[orchestration] drain config load: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });
}

/// Seed a wake-cycle [`OrchestrationState`] from the store: the counterpart to
/// reply to plus the recent-message window. Returns `None` when the session has
/// no persisted messages (nothing to wake for).
pub fn seed_state(
    config: &Config,
    agent_id: &str,
    session_id: &str,
) -> Result<Option<OrchestrationState>, String> {
    let window = config.orchestration.message_window;
    store::with_connection(&config.workspace_dir, |conn| {
        let messages = store::list_recent_messages(conn, agent_id, session_id, window)?;
        if messages.is_empty() {
            return Ok(None);
        }
        let state =
            OrchestrationState::seed(session_id.to_string(), agent_id.to_string(), messages);
        Ok(Some(state))
    })
    .map_err(|e| format!("seed_state: {e}"))
}

/// Bump the global reasoning-cycle counter and load the current (non-expired)
/// subconscious steering directive into `state` — the reasoning `execute` node
/// then weaves it into its prompt (out-of-band writer pattern, spec §6; the
/// subconscious never edges into the graph).
///
/// Called only *after* the idempotence check confirms the wake will proceed, so
/// no-op triggers (retries, duplicate ingest events, debounce edge cases) don't
/// consume a cycle tick and prematurely expire an active steering directive.
fn apply_cycle_steering(config: &Config, state: &mut OrchestrationState) -> Result<(), String> {
    store::with_connection(&config.workspace_dir, |conn| {
        let cycle = store::bump_cycle_counter(conn)?;
        state.subconscious_steering = store::current_steering_directive(conn, cycle)?
            .map(|d| d.text)
            .filter(|t| !t.trim().is_empty());
        Ok(())
    })
    .map_err(|e| format!("apply_cycle_steering: {e}"))
}

/// The highest message seq currently persisted for the session.
fn latest_seq(state: &OrchestrationState) -> i64 {
    state.messages.iter().map(|m| m.seq).max().unwrap_or(0)
}

/// Idempotence guard: has anything newer than the recorded cursor arrived?
fn has_new_work(config: &Config, agent_id: &str, session_id: &str, latest: i64) -> bool {
    let key = cursor_key(agent_id, session_id);
    let cursor = store::with_connection(&config.workspace_dir, |conn| store::kv_get(conn, &key))
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(i64::MIN);
    latest > cursor
}

/// Advance the idempotence cursor after a completed cycle.
fn advance_cursor(config: &Config, agent_id: &str, session_id: &str, latest: i64) {
    let key = cursor_key(agent_id, session_id);
    if let Err(e) = store::with_connection(&config.workspace_dir, |conn| {
        store::kv_set(conn, &key, &latest.to_string())
    }) {
        log::warn!(target: LOG, "[orchestration] cursor.advance_failed session={session_id}: {e}");
    }
}

// ── Stage 6: subconscious orchestration review ──────────────────────────────
//
// The review is driven by the dedicated **`tinyplace` subconscious instance**
// (`subconscious::profiles::tinyplace`), which ticks on its own cadence via the
// heartbeat fan-out — it no longer piggybacks on the memory tick. That profile
// calls [`load_review_window`] (observe) + [`synthesize_and_persist`] (reflect)
// and advances the review cursor from its own `commit`. The all-in-one
// [`run_orchestration_review`] wrapper below is retained for its unit tests.

/// Reflect over the orchestration layer's unreviewed compressed history +
/// cumulative world-diff timeline and, if a macro-trend warrants it, emit **one**
/// steering directive that later reasoning cycles inject into their prompt.
///
/// Fully offline: a single **tool-free** provider chat on the `subconscious`
/// route (structurally isolated — no channel/effect tools reachable). Self-gating
/// (no-op when orchestration is disabled or there is nothing new to review) and
/// idempotent (advances a review cursor after the persist). Returns `Ok(true)`
/// when a directive was emitted. The live tick path uses the split
/// `load_review_window` + `synthesize_and_persist` instead (see the stage note).
pub async fn run_orchestration_review(
    config: &Config,
    source_tick_id: &str,
) -> Result<bool, String> {
    let Some(window) = load_review_window(config).await? else {
        return Ok(false);
    };
    let emitted = synthesize_and_persist(config, &window, source_tick_id).await?;
    // The all-in-one wrapper advances the cursor itself (idle or emitted) to
    // preserve the pre-split behaviour; the tinyplace profile instead advances
    // it from its `commit`, so a superseded tick can't skip rows.
    if let Some(newest) = &window.newest_reviewed {
        let _ = store::with_connection(&config.workspace_dir, |conn| {
            store::set_review_cursor(conn, newest)
        });
    }
    Ok(emitted.is_some())
}

/// The unreviewed slice of orchestration history a steering review reflects
/// over, plus the cursor token that pins exactly the window observed. Serde so
/// it can ride the subconscious tick graph's checkpointed state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReviewWindow {
    /// Compressed-history summaries new since the review cursor (oldest-first).
    pub summaries: Vec<String>,
    /// The cumulative world-diff mutation timeline (context, not the trigger).
    pub mutations: Vec<String>,
    /// Reasoning-cycle counter stamped on an emitted directive.
    pub current_cycle: i64,
    /// How many compressed rows this window folded (for `derived_from`).
    pub compressed_count: usize,
    /// Newest reviewed compressed-row `created_at` — the commit cursor. `None`
    /// only on an empty window (which never produces a `Some(window)`).
    pub newest_reviewed: Option<String>,
}

/// Stage 6 load half: the unreviewed compressed history + cumulative world-diff
/// timeline. Self-gating — returns `None` (a clean quiet tick) when orchestration
/// is disabled or there is nothing new since the review cursor.
pub async fn load_review_window(config: &Config) -> Result<Option<ReviewWindow>, String> {
    if !config.orchestration.enabled {
        return Ok(None);
    }

    let (compressed, mutations, current_cycle) =
        store::with_connection(&config.workspace_dir, |conn| {
            let cursor = store::review_cursor(conn)?;
            let compressed = store::list_unreviewed_compressed(conn, &cursor, REVIEW_BATCH)?;
            let mutations = store::list_recent_world_mutations(conn, REVIEW_BATCH)?;
            let cycle = store::current_cycle_counter(conn)?;
            Ok((compressed, mutations, cycle))
        })
        .map_err(|e| format!("review load: {e}"))?;

    // Idempotence trigger: a review fires only on **new** compressed history
    // since the cursor. Compressed rows are written every cycle alongside the
    // world diff, so a re-tick with no new data is a clean no-op while still
    // handing the model the full cumulative world timeline for context.
    if compressed.is_empty() {
        log::debug!(target: LOG, "[orchestration] review.idle — no new compressed history");
        return Ok(None);
    }

    let newest_reviewed = compressed.iter().map(|(c, _)| c.clone()).max();
    Ok(Some(ReviewWindow {
        summaries: compressed.iter().map(|(_, t)| t.clone()).collect(),
        compressed_count: compressed.len(),
        mutations,
        current_cycle,
        newest_reviewed,
    }))
}

/// Stage 6 synthesize half: reflect over `window` offline (tool-free chat,
/// tainted origin) and, when a macro-trend warrants it, persist **one** steering
/// directive (superseding the prior) + surface it in the local Subconscious
/// window. Returns the new directive id, or `None` on a clean NONE / twice-failed.
///
/// Deliberately does **not** advance the review cursor — the caller owns that so
/// a superseded tick cannot skip rows (the tinyplace profile advances it from
/// `commit`).
pub async fn synthesize_and_persist(
    config: &Config,
    window: &ReviewWindow,
    source_tick_id: &str,
) -> Result<Option<i64>, String> {
    let prompt = build_steering_prompt(&window.summaries, &window.mutations);
    let Some(parsed) = synthesize_steering(config, &prompt, source_tick_id).await else {
        return Ok(None);
    };

    let now = chrono::Utc::now().to_rfc3339();
    let derived_from = format!(
        "compressed_rows:{} world_mutations:{}",
        window.compressed_count,
        window.mutations.len()
    );
    let directive_id = store::with_connection(&config.workspace_dir, |conn| {
        store::insert_steering_directive(
            conn,
            &parsed.text,
            &now,
            source_tick_id,
            parsed.expires_after_cycles,
            window.current_cycle,
            &derived_from,
        )
    })
    .map_err(|e| format!("review persist: {e}"))?;

    record_subconscious_directive(config, directive_id, &parsed.text).await;
    log::info!(
        target: LOG,
        "[orchestration] review.directive_emitted id={directive_id} expires_after={} derived={derived_from}",
        parsed.expires_after_cycles,
    );
    Ok(Some(directive_id))
}

/// Run the offline steering synthesis: a single tool-free chat on the
/// `subconscious` provider route under the `SubconsciousTainted` origin. Retries
/// once on a contract violation; returns `None` on a clean NONE or twice-failed.
async fn synthesize_steering(
    config: &Config,
    prompt: &str,
    tick_id: &str,
) -> Option<ParsedSteering> {
    use crate::openhuman::agent::turn_origin::{
        with_origin, AgentTurnOrigin, TrustedAutomationSource,
    };
    use crate::openhuman::inference::provider::create_chat_provider;

    for attempt in 1..=2 {
        let (provider, model) = match create_chat_provider("subconscious", config) {
            Ok(pm) => pm,
            Err(e) => {
                log::warn!(target: LOG, "[orchestration] review.provider_unavailable: {e}");
                return None;
            }
        };
        let origin = AgentTurnOrigin::TrustedAutomation {
            job_id: tick_id.to_string(),
            source: TrustedAutomationSource::SubconsciousTainted,
        };
        match with_origin(
            origin,
            provider.chat_with_system(Some(STEERING_SYNTH_SYSTEM), prompt, &model, 0.3),
        )
        .await
        {
            Ok(text) => {
                if let Some(parsed) = parse_steering_output(&text) {
                    return Some(parsed);
                }
                if is_explicit_none(&text) {
                    return None; // valid idle response — do not retry
                }
                log::warn!(
                    target: LOG,
                    "[orchestration] review.contract_violation attempt={attempt}",
                );
                if attempt == 2 {
                    return None;
                }
            }
            Err(e) => {
                log::warn!(target: LOG, "[orchestration] review.synth_failed attempt={attempt}: {e}");
                if attempt == 2 {
                    return None;
                }
            }
        }
    }
    None
}

/// Persist an emitted directive into the local Subconscious chat window and
/// publish it for the live UI (stage 7). No outbound tiny.place effect: the wake
/// subscriber ignores `Subconscious` chat-kind events.
pub async fn record_subconscious_directive(config: &Config, directive_id: i64, text: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) = store::with_connection(&config.workspace_dir, |conn| {
        store::upsert_session(
            conn,
            &OrchestrationSession {
                session_id: SUBCONSCIOUS_SESSION.to_string(),
                agent_id: SUBCONSCIOUS_SESSION.to_string(),
                source: "subconscious".to_string(),
                label: None,
                workspace: None,
                last_seq: directive_id,
                created_at: now.clone(),
                last_message_at: now.clone(),
            },
        )?;
        store::insert_message(
            conn,
            &OrchestrationMessage {
                id: format!("steering:{directive_id}"),
                agent_id: SUBCONSCIOUS_SESSION.to_string(),
                session_id: SUBCONSCIOUS_SESSION.to_string(),
                chat_kind: ChatKind::Subconscious,
                role: "subconscious".to_string(),
                body: text.to_string(),
                timestamp: now.clone(),
                seq: directive_id,
            },
        )
    }) {
        log::warn!(target: LOG, "[orchestration] review.window_persist_failed: {e}");
    }

    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::OrchestrationSessionMessage {
            agent_id: SUBCONSCIOUS_SESSION.to_string(),
            session_id: SUBCONSCIOUS_SESSION.to_string(),
            chat_kind: ChatKind::Subconscious.as_str().to_string(),
        },
    );
}

/// Build the production node set and drive one wake cycle. Skips (no LLM, no DM)
/// when the idempotence cursor shows no new messages since the last cycle.
pub async fn invoke_orchestration_graph(
    config: &Config,
    agent_id: &str,
    session_id: &str,
) -> Result<(), String> {
    let config_arc = Arc::new(config.clone());
    let runtime: Arc<dyn OrchestrationRuntime> = Arc::new(ProductionRuntime {
        config: config_arc.clone(),
        agent_id: agent_id.to_string(),
        session_id: session_id.to_string(),
    });
    invoke_with_runtime(config, agent_id, session_id, runtime).await
}

/// Drive one wake cycle with an injected runtime (the production nodes, or a stub
/// in tests). Hardening (stage 8):
/// - **scheduler_gate**: awaits `wait_for_capacity()` so a `Paused`/`Throttled`
///   gate defers the cycle instead of running — the message stays in the store
///   and the cursor is untouched, so nothing is dropped.
/// - **no duplicate DM on failure**: the idempotence cursor advances *only* when
///   the cycle completed and sent its DM; a provider error mid-graph leaves the
///   cursor unmoved so the next trigger resumes (the `dm_sent` latch + the
///   deterministic `cycle_id` keep store writes idempotent).
/// - **last-error observability**: a failed cycle records `orchestration:last_error`
///   for `orchestration.status`.
pub async fn invoke_with_runtime(
    config: &Config,
    agent_id: &str,
    session_id: &str,
    runtime: Arc<dyn OrchestrationRuntime>,
) -> Result<(), String> {
    let Some(mut state) = seed_state(config, agent_id, session_id)? else {
        log::debug!(target: LOG, "[orchestration] wake.skip_empty session={session_id}");
        return Ok(());
    };
    let latest = latest_seq(&state);
    if !has_new_work(config, agent_id, session_id, latest) {
        log::debug!(
            target: LOG,
            "[orchestration] wake.skip_idempotent session={session_id} latest_seq={latest}",
        );
        return Ok(());
    }
    // Only now that the cycle is confirmed to proceed: advance the reasoning-cycle
    // counter and inject the current steering directive. Keeping this after the
    // idempotence guard prevents no-op wakes from expiring steering early.
    apply_cycle_steering(config, &mut state)?;

    // Defer under a paused/throttled scheduler gate — the permit is held for the
    // whole cycle so background pressure backs off without dropping the message.
    let _gate = crate::openhuman::scheduler_gate::wait_for_capacity().await;

    let config_arc = Arc::new(config.clone());
    let out = match run_orchestration_graph(config_arc.clone(), runtime, state).await {
        Ok(out) => out,
        Err(e) => {
            let msg = format!("graph run: {e}");
            record_last_error(config, &msg);
            return Err(msg);
        }
    };

    // Advance the cursor only on a completed, DM-sent cycle (no double-send on
    // resume; a crash before this leaves the cursor for a clean retry).
    if out.dm_sent {
        advance_cursor(config, agent_id, session_id, latest);
    }
    Ok(())
}

/// Record the most recent orchestration error for `orchestration.status` health.
/// Never includes message bodies — just a short cause string.
fn record_last_error(config: &Config, message: &str) {
    let stamped = format!("{} · {}", chrono::Utc::now().to_rfc3339(), message);
    let _ = store::with_connection(&config.workspace_dir, |conn| {
        store::kv_set(conn, "orchestration:last_error", &stamped)
    });
}

// ── Production runtime ──────────────────────────────────────────────────────

/// Render the windowed transcript for a node prompt. Roles are the harness roles
/// (`user` / `agent`); the agents read them like a chat log.
fn render_transcript(state: &OrchestrationState) -> String {
    let mut out = String::with_capacity(1024);
    for m in &state.messages {
        out.push_str(&format!("[{}] {}\n", m.role, m.body));
    }
    out
}

/// The production wiring for every wake-graph node: the front-end + reasoning
/// agents, the compression summarizer, the world-diff + compressed-history store
/// writes, the memory-RAG eviction, and the Signal DM reply.
struct ProductionRuntime {
    config: Arc<Config>,
    agent_id: String,
    session_id: String,
}

impl ProductionRuntime {
    /// Run a built-in agent for one turn under a background origin, forcing the
    /// given model hint (`hint:chat` for the front end, `hint:reasoning` for the
    /// core). Returns the final assistant text.
    async fn run_agent_turn(
        &self,
        agent_id: &str,
        model_hint: &str,
        channel: &str,
        user_message: String,
    ) -> anyhow::Result<String> {
        use crate::openhuman::agent::turn_origin::{
            with_origin, AgentTurnOrigin, TrustedAutomationSource,
        };
        use crate::openhuman::agent::Agent;

        let mut effective = (*self.config).clone();
        effective.default_model = Some(model_hint.to_string());

        let mut agent = Agent::from_config_for_agent(&effective, agent_id)
            .map_err(|e| anyhow::anyhow!("{agent_id} init: {e}"))?;
        agent.set_event_context(
            format!("orchestration:{channel}:{}", self.session_id),
            "orchestration",
        );

        // Background origin: no interactive approval parking.
        let origin = AgentTurnOrigin::TrustedAutomation {
            job_id: format!("orchestration:{channel}:{}", self.session_id),
            source: TrustedAutomationSource::Cron,
        };
        with_origin(origin, agent.run_single(&user_message))
            .await
            .map_err(|e| anyhow::anyhow!("{agent_id} run: {e}"))
    }
}

#[async_trait]
impl OrchestrationRuntime for ProductionRuntime {
    async fn frontend_instruct(&self, state: &OrchestrationState) -> anyhow::Result<String> {
        let prompt = format!(
            "Session transcript:\n\n{}\n\n## Pass 1\n\nTriage this. If a complete answer is \
             obvious, call `reply_to_channel`. Otherwise call `defer_to_orchestrator` with concise \
             macro-instructions for the reasoning core.",
            render_transcript(state),
        );
        self.run_agent_turn("frontend_agent", "hint:chat", "frontend", prompt)
            .await
    }

    async fn frontend_compile(&self, state: &OrchestrationState) -> anyhow::Result<String> {
        let reply = state.agent_reply.clone().unwrap_or_default();
        let prompt = format!(
            "Session transcript:\n\n{}\n\n## Pass 2\n\nThe reasoning core produced this result:\n\n\
             {}\n\nCompile it into the finished message to send back to the session, then call \
             `reply_to_channel` with that text.",
            render_transcript(state),
            reply,
        );
        self.run_agent_turn("frontend_agent", "hint:chat", "frontend", prompt)
            .await
    }

    async fn execute(&self, state: &OrchestrationState) -> anyhow::Result<ExecuteOutcome> {
        let instructions = state.agent_instructions.as_deref().unwrap_or("(none)");
        let prompt = format!(
            "Macro-instructions from the front end:\n\n{instructions}\n\nSession transcript:\n\n{}\n\n\
             Do the work (delegating to worker sub-agents where appropriate) and return the result.",
            render_transcript(state),
        );
        // Scope the current steering directive so the reasoning agent's prompt
        // builder weaves it into the system prompt (spec §3.2).
        let steering = state.subconscious_steering.clone().unwrap_or_default();
        let reply = super::reasoning_agent::with_steering(
            steering,
            self.run_agent_turn("reasoning_agent", "hint:reasoning", "reasoning", prompt),
        )
        .await?;
        // The trace the compression node condenses. `run_single` surfaces the
        // final assistant text; the richer per-tool/sub-agent trace lands when
        // the lower-level runner is wired (follow-up). Frame it with the
        // instructions so the compressed record is self-describing.
        let trace = format!("Instructions: {instructions}\n\nResult:\n{reply}");
        Ok(ExecuteOutcome { reply, trace })
    }

    async fn compress(&self, state: &OrchestrationState) -> anyhow::Result<CompressedEntry> {
        let trace = &state.execution_trace;
        let input_tokens = count_tokens(trace);
        if input_tokens == 0 {
            return Ok(CompressedEntry::default());
        }
        let budget = compression_budget(input_tokens);

        // Summarize via a cheap tier, then enforce the 20:1 budget: retry once if
        // the summary exceeds 1.5× budget, then hard-truncate.
        let summarize_prompt = format!(
            "Compress the following execution trace into at most ~{budget} tokens. Keep only the \
             decisions, outcomes, and facts needed to continue. No preamble.\n\n{trace}",
        );
        let raw = self
            .run_agent_turn(
                "summarizer",
                "hint:burst",
                "compress",
                summarize_prompt.clone(),
            )
            .await
            .unwrap_or_else(|_| trace.clone());
        let (mut summary, mut truncated) = enforce_budget(&raw, budget);
        if truncated {
            if let Ok(retry) = self
                .run_agent_turn("summarizer", "hint:burst", "compress", summarize_prompt)
                .await
            {
                let (s2, t2) = enforce_budget(&retry, budget);
                summary = s2;
                truncated = t2;
            }
        }
        let output_tokens = count_tokens(&summary);
        let now = chrono::Utc::now().to_rfc3339();

        // Persist idempotently by cycle_id (a resumed cycle re-writes the same row).
        let cycle_id = state.cycle_id.clone();
        let session_id = state.session_id.clone();
        let agent_id = self.agent_id.clone();
        let text = summary.clone();
        if let Err(e) = store::with_connection(&self.config.workspace_dir, |conn| {
            store::insert_compressed(
                conn,
                &cycle_id,
                &session_id,
                &agent_id,
                input_tokens as i64,
                output_tokens as i64,
                &text,
                &now,
            )
        }) {
            log::warn!(target: LOG, "[orchestration] compress.persist_failed cycle={cycle_id}: {e}");
        }
        log::debug!(
            target: LOG,
            "[orchestration] compress cycle={} input={input_tokens} output={output_tokens} budget={budget} truncated={truncated}",
            state.cycle_id,
        );
        Ok(CompressedEntry {
            summary,
            covered_messages: state.messages.len() as u32,
        })
    }

    async fn world_diff(&self, state: &OrchestrationState) -> anyhow::Result<WorldDiffEntry> {
        let signature = world_diff::event_signature(state);
        let mutation = world_diff::world_mutation(state);
        let delta = world_diff::delta(state);
        let now = chrono::Utc::now().to_rfc3339();

        let cycle_id = state.cycle_id.clone();
        let session_id = state.session_id.clone();
        let agent_id = self.agent_id.clone();
        let seq = store::with_connection(&self.config.workspace_dir, |conn| {
            store::append_world_diff(
                conn,
                &cycle_id,
                &session_id,
                &agent_id,
                &signature,
                &mutation,
                &delta,
                &now,
            )
        })
        .map_err(|e| anyhow::anyhow!("world_diff persist: {e}"))?;

        Ok(WorldDiffEntry {
            seq: seq as u64,
            note: mutation,
        })
    }

    async fn context_utilization(&self, state: &OrchestrationState) -> anyhow::Result<f32> {
        // Estimate accumulated tokens: the message window + execution trace +
        // retained compressed-history summaries, over the assumed window.
        let mut tokens = count_tokens(&render_transcript(state));
        tokens += count_tokens(&state.execution_trace);
        for entry in &state.compressed_history {
            tokens += count_tokens(&entry.summary);
        }
        let util = (tokens as f32 / ASSUMED_CONTEXT_WINDOW as f32).min(1.0);
        Ok(util)
    }

    async fn evict(&self, state: &OrchestrationState) -> anyhow::Result<EvictionOutcome> {
        // Keep the most recent two compressed entries live; evict the older head
        // to memory RAG under a session-scoped path so it stays retrievable.
        let total = state.compressed_history.len();
        let keep = 2usize.min(total);
        let evict_count = total.saturating_sub(keep);
        let path_scope = format!("orchestration/{}", state.session_id);

        for (i, entry) in state
            .compressed_history
            .iter()
            .take(evict_count)
            .enumerate()
        {
            let doc = crate::openhuman::memory_sync::canonicalize::document::DocumentInput {
                provider: "orchestration".to_string(),
                title: format!("orchestration session {} — cycle summary", state.session_id),
                body: entry.summary.clone(),
                modified_at: chrono::Utc::now(),
                source_ref: None,
            };
            let source_id = format!("orchestration/{}/{}#{i}", state.session_id, state.cycle_id);
            if let Err(e) = crate::openhuman::memory::ingest_pipeline::ingest_document_with_scope(
                &self.config,
                &source_id,
                &self.agent_id,
                vec!["orchestration".to_string()],
                doc,
                Some(path_scope.clone()),
            )
            .await
            {
                log::warn!(target: LOG, "[orchestration] evict.memory_write_failed: {e}");
            }
        }

        // Utilization after dropping the evicted head from live state.
        let mut retained_tokens = count_tokens(&render_transcript(state));
        retained_tokens += count_tokens(&state.execution_trace);
        for entry in state.compressed_history.iter().skip(evict_count) {
            retained_tokens += count_tokens(&entry.summary);
        }
        let new_utilization = (retained_tokens as f32 / ASSUMED_CONTEXT_WINDOW as f32).min(1.0);
        log::debug!(
            target: LOG,
            "[orchestration] evict session={} evicted={evict_count} new_util={new_utilization}",
            state.session_id,
        );
        Ok(EvictionOutcome {
            evicted: evict_count,
            new_utilization,
        })
    }

    async fn send_dm(&self, counterpart_agent_id: &str, body: &str) -> anyhow::Result<()> {
        // A reply into a real harness session is stamped with a v1 session
        // envelope so the peer threads it under the same session id; Master and
        // subconscious replies stay plain.
        let plaintext = session_send_plaintext(&self.session_id, body)?;
        let mut params = Map::new();
        params.insert("recipient".to_string(), Value::from(counterpart_agent_id));
        params.insert("plaintext".to_string(), Value::from(plaintext));
        crate::openhuman::tinyplace::handle_tinyplace_signal_send_message(params)
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("signal send: {e}"))
    }
}

/// Wire body for an agent reply into `session_id`: a v1 session envelope for a
/// real harness session (so the peer threads its reply under the same id), or
/// the plain body for the pinned Master / subconscious windows.
fn session_send_plaintext(session_id: &str, body: &str) -> anyhow::Result<String> {
    if session_id == "master" || session_id == "subconscious" {
        return Ok(body.to_string());
    }
    let message_id = format!("session-out:{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();
    serde_json::to_string(&SessionEnvelopeV1::outgoing(
        session_id,
        body,
        &message_id,
        &now,
    ))
    .map_err(|e| anyhow::anyhow!("envelope encode: {e}"))
}

// ── Self-identity composition (orchestration_self_identity read model) ────────

/// One @handle this agent's wallet holds (reverse-resolved from the directory).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HandleEntry {
    pub(crate) username: String,
    pub(crate) primary: bool,
}

/// This agent's own tiny.place identity and whether peers can reach it.
///
/// `discoverable` is the bottom line the UI cares about: a peer can DM this
/// agent only if both its directory card AND its Signal encryption key are
/// published. A fresh identity can accept contacts yet still be un-messageable
/// until it registers a @handle (which is what publishes both), so the
/// `SelfIdentityCard` surfaces the gap instead of leaving it a mystery 404.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelfIdentity {
    pub(crate) agent_id: String,
    pub(crate) handles: Vec<HandleEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) primary_handle: Option<String>,
    pub(crate) card_published: bool,
    pub(crate) key_published: bool,
    pub(crate) discoverable: bool,
}

/// Pure composition of the three tinyplace reads into the renderer shape. Kept
/// here (business logic) so the parsing/discoverability rules are unit-testable
/// without a live tiny.place client; the `schemas` handler supplies the reads.
///
/// `reverse` is the raw `directory_reverse` JSON (`{ identities: [...] }`), or
/// `None` on a reverse miss. Discoverable = card live AND encryption key
/// published + current — either gap leaves the agent un-messageable.
pub(crate) fn build_self_identity(
    agent_id: String,
    key_published: bool,
    reverse: Option<&Value>,
    card_published: bool,
) -> SelfIdentity {
    let mut handles: Vec<HandleEntry> = Vec::new();
    let mut primary_handle: Option<String> = None;
    if let Some(idents) = reverse
        .and_then(|r| r.get("identities"))
        .and_then(Value::as_array)
    {
        for ident in idents {
            let username = ident
                .get("username")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(username) = username else { continue };
            let primary = ident
                .get("primary")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if primary && primary_handle.is_none() {
                primary_handle = Some(username.to_string());
            }
            handles.push(HandleEntry {
                username: username.to_string(),
                primary,
            });
        }
    }
    // Fall back to the first handle when none is flagged primary.
    if primary_handle.is_none() {
        primary_handle = handles.first().map(|h| h.username.clone());
    }
    SelfIdentity {
        agent_id,
        handles,
        primary_handle,
        card_published,
        key_published,
        discoverable: card_published && key_published,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::orchestration::types::OrchestrationMessage;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tinyagents::graph::checkpoint::Checkpointer;

    #[test]
    fn self_identity_marks_published_identity_discoverable() {
        let reverse = serde_json::json!({
            "identities": [
                { "username": "  ", "primary": false },   // blank → skipped
                { "username": "openhuman", "primary": false },
                { "username": "oh_primary", "primary": true },
            ]
        });
        let id = build_self_identity("addr123".to_string(), true, Some(&reverse), true);
        assert_eq!(id.agent_id, "addr123");
        assert_eq!(id.handles.len(), 2, "blank username skipped");
        assert_eq!(id.primary_handle.as_deref(), Some("oh_primary"));
        assert!(id.card_published && id.key_published && id.discoverable);
    }

    #[test]
    fn self_identity_primary_falls_back_to_first_handle() {
        let reverse = serde_json::json!({
            "identities": [ { "username": "solo" } ] // no primary flag
        });
        let id = build_self_identity("addr".to_string(), true, Some(&reverse), true);
        assert_eq!(id.primary_handle.as_deref(), Some("solo"));
    }

    #[test]
    fn self_identity_undiscoverable_when_card_or_key_missing() {
        // No reverse (handle-less), card present but key not published → the
        // exact un-messageable case the SelfIdentityCard must flag.
        let no_key = build_self_identity("addr".to_string(), false, None, true);
        assert!(no_key.handles.is_empty());
        assert!(no_key.primary_handle.is_none());
        assert!(!no_key.discoverable, "key not published → not discoverable");

        let no_card = build_self_identity("addr".to_string(), true, None, false);
        assert!(
            !no_card.discoverable,
            "card not published → not discoverable"
        );
    }
    use tinyagents::graph::SqliteCheckpointer;

    fn test_config(tmp: &tempfile::TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().to_path_buf(),
            ..Config::default()
        }
    }

    #[test]
    fn session_reply_is_wrapped_but_master_reply_stays_plain() {
        // A real session id → v1 envelope threaded under that id.
        let wire = session_send_plaintext("h-42", "on it").expect("encode");
        let env = SessionEnvelopeV1::parse(&wire).expect("valid v1 envelope");
        assert_eq!(env.scope.harness_session_id, "h-42");
        assert_eq!(env.message.text, "on it");
        // The pinned windows stay plain (no envelope).
        assert_eq!(session_send_plaintext("master", "hi").unwrap(), "hi");
        assert_eq!(session_send_plaintext("subconscious", "hi").unwrap(), "hi");
    }

    fn msg(session: &str, seq: i64) -> OrchestrationMessage {
        OrchestrationMessage {
            id: format!("m{seq}"),
            agent_id: "@peer".into(),
            session_id: session.into(),
            chat_kind: ChatKind::Session,
            role: "user".into(),
            body: "hello".into(),
            timestamp: format!("2026-07-02T00:00:{seq:02}Z"),
            seq,
        }
    }

    #[test]
    fn cursor_gates_reprocessing() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        // No cursor yet → any message is new work.
        assert!(has_new_work(&config, "@peer", "h1", 3));
        advance_cursor(&config, "@peer", "h1", 3);
        // Nothing newer than seq 3 → no work (idempotent re-trigger).
        assert!(!has_new_work(&config, "@peer", "h1", 3));
        // A newer message reopens work.
        assert!(has_new_work(&config, "@peer", "h1", 4));
    }

    #[test]
    fn debounce_generation_coalesces_bursts() {
        let key = "@peer:burst-session";
        let g1 = bump_generation(key);
        let g2 = bump_generation(key);
        let g3 = bump_generation(key);
        assert!(g2 > g1 && g3 > g2);
        // Only the latest trigger survives the debounce window.
        assert!(!is_latest_generation(key, g1));
        assert!(!is_latest_generation(key, g2));
        assert!(is_latest_generation(key, g3));
    }

    #[test]
    fn seed_state_windows_messages_and_skips_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        // Empty session → nothing to wake for.
        assert!(seed_state(&config, "@peer", "h1").unwrap().is_none());

        // Persist two messages, then seed reads them in order.
        store::with_connection(&config.workspace_dir, |conn| {
            store::insert_message(conn, &msg("h1", 1))?;
            store::insert_message(conn, &msg("h1", 2))?;
            Ok(())
        })
        .unwrap();
        let state = seed_state(&config, "@peer", "h1").unwrap().expect("seeded");
        assert_eq!(state.session_id, "h1");
        assert_eq!(state.counterpart_agent_id, "@peer");
        assert_eq!(state.messages.len(), 2);
        assert_eq!(latest_seq(&state), 2);
    }

    // A hermetic stub runtime for the integration run (no LLM, no real Signal,
    // no memory writes) that records DMs + world-diff/compress store rows.
    // (`CompressedEntry`, `ExecuteOutcome`, etc. are in scope via `use super::*`.)
    struct StubRuntime {
        config: Arc<Config>,
        agent_id: String,
        sends: Arc<AtomicUsize>,
        /// Stage-8 failure injection: when true, the reasoning node errors mid-graph.
        fail_execute: bool,
    }

    #[async_trait]
    impl OrchestrationRuntime for StubRuntime {
        async fn frontend_instruct(&self, _s: &OrchestrationState) -> anyhow::Result<String> {
            Ok("instructions".into())
        }
        async fn frontend_compile(&self, _s: &OrchestrationState) -> anyhow::Result<String> {
            Ok("compiled reply".into())
        }
        async fn execute(&self, _s: &OrchestrationState) -> anyhow::Result<ExecuteOutcome> {
            if self.fail_execute {
                anyhow::bail!("provider error mid-graph (injected)");
            }
            Ok(ExecuteOutcome {
                reply: "reasoning reply".into(),
                trace: "trace line one\ntrace line two".into(),
            })
        }
        async fn compress(&self, s: &OrchestrationState) -> anyhow::Result<CompressedEntry> {
            // Persist a real compressed row so the e2e can assert exactly one.
            store::with_connection(&self.config.workspace_dir, |conn| {
                store::insert_compressed(
                    conn,
                    &s.cycle_id,
                    &s.session_id,
                    &self.agent_id,
                    100,
                    5,
                    "compact",
                    "now",
                )
            })
            .ok();
            Ok(CompressedEntry {
                summary: "compact".into(),
                covered_messages: s.messages.len() as u32,
            })
        }
        async fn world_diff(&self, s: &OrchestrationState) -> anyhow::Result<WorldDiffEntry> {
            let seq = store::with_connection(&self.config.workspace_dir, |conn| {
                store::append_world_diff(
                    conn,
                    &s.cycle_id,
                    &s.session_id,
                    &self.agent_id,
                    "sig",
                    "mutation",
                    "delta",
                    "now",
                )
            })
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(WorldDiffEntry {
                seq: seq as u64,
                note: "mutation".into(),
            })
        }
        async fn context_utilization(&self, _s: &OrchestrationState) -> anyhow::Result<f32> {
            Ok(0.1)
        }
        async fn evict(&self, _s: &OrchestrationState) -> anyhow::Result<EvictionOutcome> {
            Ok(EvictionOutcome {
                evicted: 0,
                new_utilization: 0.1,
            })
        }
        async fn send_dm(&self, _c: &str, _b: &str) -> anyhow::Result<()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn full_cycle_persists_one_dm_one_compressed_one_diff_and_checkpoints() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Arc::new(test_config(&tmp));
        let sends = Arc::new(AtomicUsize::new(0));

        let state = OrchestrationState::seed("h1", "@peer", vec![msg("h1", 1)]);
        let runtime = Arc::new(StubRuntime {
            config: config.clone(),
            agent_id: "@me".into(),
            sends: sends.clone(),
            fail_execute: false,
        });
        let out = run_orchestration_graph(config.clone(), runtime, state)
            .await
            .expect("graph runs");

        assert!(out.dm_sent, "cycle latches dm_sent");
        assert_eq!(sends.load(Ordering::SeqCst), 1, "exactly one DM");
        assert_eq!(out.channel_response.as_deref(), Some("compiled reply"));

        // Exactly one compressed row + one world-diff entry landed in the store.
        store::with_connection(&config.workspace_dir, |conn| {
            assert_eq!(store::count_compressed(conn, "@me", "h1")?, 1);
            assert_eq!(store::world_diff_seqs(conn, "@me", "h1")?, vec![1]);
            Ok(())
        })
        .unwrap();

        // Checkpoints persisted → kill/restart could resume without re-sending.
        // Same `orchestration_graph_checkpoints.db` path `run_orchestration_graph`
        // opens (see `orchestration/graph/mod.rs`).
        let checkpoint_db = config
            .workspace_dir
            .join("orchestration_graph_checkpoints.db");
        let cp = SqliteCheckpointer::<OrchestrationState>::open(&checkpoint_db)
            .expect("open checkpoint store");
        let list = cp.list("orchestration:h1").await.expect("list checkpoints");
        assert!(!list.is_empty(), "wake cycle persisted checkpoints");
    }

    // ── Stage 6: subconscious steering ──────────────────────────────────────

    /// The factory test override (`test_provider_override`) is process-global, so
    /// the two tests that install a scripted provider must not run concurrently.
    /// This lock serializes them (poison-tolerant).
    static PROVIDER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A scripted provider so `create_chat_provider` returns a canned steering
    /// synthesis without any network (the factory test override).
    struct ScriptedProvider {
        reply: String,
    }
    #[async_trait]
    impl crate::openhuman::inference::provider::Provider for ScriptedProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(self.reply.clone())
        }
    }

    /// Seed one compressed-history row + one world-diff entry so a review has data.
    fn seed_orchestration_activity(config: &Config, cycle_tag: &str) {
        store::with_connection(&config.workspace_dir, |conn| {
            store::insert_compressed(
                conn,
                &format!("h1#{cycle_tag}"),
                "h1",
                "@me",
                400,
                20,
                &format!("did work {cycle_tag}"),
                &format!("2026-07-02T00:0{cycle_tag}:00Z"),
            )?;
            store::append_world_diff(
                conn,
                &format!("h1#{cycle_tag}"),
                "h1",
                "@me",
                "sig",
                &format!("world moved {cycle_tag}"),
                "delta",
                &format!("2026-07-02T00:0{cycle_tag}:00Z"),
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn review_emits_directive_and_next_cycle_seeds_it_into_state() {
        let _serial = PROVIDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        seed_orchestration_activity(&config, "1");

        let _guard =
            crate::openhuman::inference::provider::factory::test_provider_override::install(
                Arc::new(ScriptedProvider {
                    reply: "STEERING_DIRECTIVE: prioritize the billing migration\n\
                            expires_after_cycles: 12"
                        .to_string(),
                }),
            );

        // One review over seeded data → exactly one current directive.
        let emitted = run_orchestration_review(&config, "tick1").await.unwrap();
        assert!(emitted, "a directive was emitted");
        store::with_connection(&config.workspace_dir, |conn| {
            let cur = store::current_steering_directive(conn, 0)?.expect("current directive");
            assert_eq!(cur.text, "prioritize the billing migration");
            assert_eq!(cur.expires_after_cycles, 12);
            Ok(())
        })
        .unwrap();

        // The next reasoning cycle loads it into state at cycle start (the seam the
        // `execute` node reads → reasoning prompt weaves it in, per stage 5).
        store::with_connection(&config.workspace_dir, |conn| {
            store::insert_message(conn, &msg("h1", 1))?;
            Ok(())
        })
        .unwrap();
        let mut state = seed_state(&config, "@peer", "h1").unwrap().expect("seeded");
        // Steering is injected once the wake is confirmed to proceed (mirrors
        // `invoke_with_runtime` calling `apply_cycle_steering` after the
        // idempotence guard), not by `seed_state` itself.
        apply_cycle_steering(&config, &mut state).unwrap();
        assert_eq!(
            state.subconscious_steering.as_deref(),
            Some("prioritize the billing migration"),
            "the directive is injected into the next cycle's state"
        );

        // It also surfaced in the local Subconscious chat window.
        store::with_connection(&config.workspace_dir, |conn| {
            assert_eq!(
                store::count_messages(conn, "subconscious", "subconscious")?,
                1
            );
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn review_is_idempotent_and_idle_without_new_data() {
        let _serial = PROVIDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);

        // Empty orchestration store → clean no-op, no provider call needed.
        assert!(!run_orchestration_review(&config, "t0").await.unwrap());

        seed_orchestration_activity(&config, "1");
        let _guard =
            crate::openhuman::inference::provider::factory::test_provider_override::install(
                Arc::new(ScriptedProvider {
                    reply: "STEERING_DIRECTIVE: do the thing\nexpires_after_cycles: 20".to_string(),
                }),
            );
        assert!(
            run_orchestration_review(&config, "t1").await.unwrap(),
            "first emits"
        );
        // Re-tick with no new compressed history → idempotent no-op (cursor past it).
        assert!(
            !run_orchestration_review(&config, "t2").await.unwrap(),
            "re-tick without new data emits nothing"
        );
        // Still exactly one directive total.
        store::with_connection(&config.workspace_dir, |conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM steering_directives", [], |r| r.get(0))?;
            assert_eq!(count, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn subconscious_agent_tool_surface_has_no_channel_or_effect_tools() {
        // Isolation invariant (stage 6): the subconscious never contacts anyone.
        // Its decide-stage agent must expose no tiny.place / channel outbound
        // tools; the orchestration_review synthesis is a tool-free provider chat
        // by construction. Assert the shipped agent definition stays clean.
        const SUBCONSCIOUS_TOML: &str = include_str!("../subconscious/agent/agent.toml");
        let def: toml::Value = toml::from_str(SUBCONSCIOUS_TOML).expect("subconscious agent.toml");
        let tools = def
            .get("tools")
            .and_then(|t| t.get("named"))
            .and_then(|n| n.as_array())
            .expect("subconscious [tools].named");
        for t in tools {
            let name = t.as_str().unwrap_or_default();
            assert!(
                !name.starts_with("tinyplace_") && !name.contains("send_message"),
                "subconscious must not expose channel/outbound tool `{name}`"
            );
        }
    }

    // ── Stage 8: failure-mode hardening + observability ─────────────────────

    #[tokio::test]
    async fn provider_error_mid_graph_sends_no_dm_and_a_later_cycle_does_not_double_send() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        store::with_connection(&config.workspace_dir, |conn| {
            store::upsert_session(
                conn,
                &OrchestrationSession {
                    session_id: "h1".into(),
                    agent_id: "@peer".into(),
                    source: "codex".into(),
                    label: None,
                    workspace: None,
                    last_seq: 1,
                    created_at: "now".into(),
                    last_message_at: "now".into(),
                },
            )?;
            store::insert_message(conn, &msg("h1", 1))?;
            Ok(())
        })
        .unwrap();

        let sends = Arc::new(AtomicUsize::new(0));
        // Cycle 1: the reasoning node errors → the run fails, no DM, and the
        // idempotence cursor is NOT advanced (so the message is not lost).
        let failing = Arc::new(StubRuntime {
            config: Arc::new(config.clone()),
            agent_id: "@me".into(),
            sends: sends.clone(),
            fail_execute: true,
        });
        let err = invoke_with_runtime(&config, "@peer", "h1", failing)
            .await
            .expect_err("cycle fails on the injected provider error");
        assert!(err.contains("graph run"));
        assert_eq!(sends.load(Ordering::SeqCst), 0, "no DM on a failed cycle");
        // last_error surfaced for orchestration.status.
        let last_error = store::with_connection(&config.workspace_dir, |conn| {
            store::kv_get(conn, "orchestration:last_error")
        })
        .unwrap();
        assert!(last_error.is_some(), "failed cycle records last_error");

        // Cycle 2 (recovery): a healthy runtime sends exactly one DM — the earlier
        // failure did not consume the message or leave a duplicate.
        let healthy = Arc::new(StubRuntime {
            config: Arc::new(config.clone()),
            agent_id: "@me".into(),
            sends: sends.clone(),
            fail_execute: false,
        });
        invoke_with_runtime(&config, "@peer", "h1", healthy)
            .await
            .expect("recovery cycle runs");
        assert_eq!(
            sends.load(Ordering::SeqCst),
            1,
            "recovery sends exactly one DM"
        );

        // A third trigger with no new messages is idempotent (cursor advanced).
        let healthy2 = Arc::new(StubRuntime {
            config: Arc::new(config.clone()),
            agent_id: "@me".into(),
            sends: sends.clone(),
            fail_execute: false,
        });
        invoke_with_runtime(&config, "@peer", "h1", healthy2)
            .await
            .expect("idempotent re-trigger");
        assert_eq!(
            sends.load(Ordering::SeqCst),
            1,
            "no duplicate DM on re-trigger"
        );
    }

    #[test]
    fn malformed_envelope_flood_all_fall_back_to_master_without_panic() {
        // A flood of non-envelope / malformed DM bodies must each classify as a
        // Master message (never a crash, never a Session mis-route). Uses the
        // ingest classifier indirectly through persist.
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        for i in 0..200 {
            // Deliberately malformed: truncated JSON, wrong version, junk.
            let body = match i % 3 {
                0 => "{ not json".to_string(),
                1 => r#"{"envelope_version":"bogus","scope":{}}"#.to_string(),
                _ => format!("plain chatter {i}"),
            };
            // classify_message is private to ingest; assert the envelope parser
            // rejects each (→ Master fallback) without panicking.
            assert!(
                super::super::types::SessionEnvelopeV1::parse(&body).is_none(),
                "malformed body #{i} must not parse as a session envelope"
            );
        }
        let _ = config; // tempdir kept alive
    }

    /// Stage-8 leak guard: no orchestration log line may emit a message body /
    /// decrypted plaintext / seed. Scans the domain source for logging macros
    /// that reference a body-bearing field. The project rule is "never log
    /// secrets or full PII" — message bodies are decrypted plaintext.
    #[test]
    fn orchestration_logs_never_reference_message_bodies() {
        const SOURCES: &[(&str, &str)] = &[
            ("ingest.rs", include_str!("ingest.rs")),
            ("ops.rs", include_str!("ops.rs")),
            ("bus.rs", include_str!("bus.rs")),
            ("schemas.rs", include_str!("schemas.rs")),
            ("graph/mod.rs", include_str!("graph/mod.rs")),
        ];
        // Forbidden substrings that would interpolate secret content into a log.
        const FORBIDDEN: &[&str] = &["plaintext", ".body", "message.text", "signer_seed", "seed="];
        for (name, src) in SOURCES {
            for (lineno, line) in src.lines().enumerate() {
                let is_log = line.contains("log::")
                    || line.contains("tracing::debug!")
                    || line.contains("tracing::info!")
                    || line.contains("tracing::warn!")
                    || line.contains("tracing::error!");
                if !is_log {
                    continue;
                }
                for needle in FORBIDDEN {
                    assert!(
                        !line.contains(needle),
                        "{name}:{}: log line may leak secret/body content (`{needle}`): {}",
                        lineno + 1,
                        line.trim(),
                    );
                }
            }
        }
    }
}
