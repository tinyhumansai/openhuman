//! Subconscious engine — periodic, structured background loop.
//!
//! Each tick is a small, deterministic, three-stage flow:
//!
//!   1. **memory_diff (code)** — diff the agent's connected sources against the
//!      world baseline captured at the end of the previous tick, to see how the
//!      user's world changed (`memory_diff::ops::diff_since_checkpoint`).
//!   2. **prepare_context (code)** — run the read-only `context_scout`
//!      (`agent_prepare_context`) over that diff to gather grounding context
//!      from memory, goals/profile, integrations, and the web.
//!   3. **decide (agent)** — hand `diff + context` to the slim subconscious
//!      agent, which decides what (if anything) to do: record follow-ups on the
//!      user's global to-do board (`update_task`), evolve long-term goals
//!      (`goals_*`), notify the user (`notify_user`), or delegate deeper work
//!      (`spawn_async_subagent`).
//!
//! Continuity across ticks lives in those durable stores (global to-dos +
//! goals), not in a bespoke scratchpad — so quiet ticks (no diff) cost nothing
//! and the loop stays stateless beyond the world baseline.
//!
//! ## Concurrency & timeouts
//!
//! A per-engine `tick_lock` prevents overlapping ticks. Each tick has a hard
//! wall-clock timeout (`TICK_TIMEOUT`) so a stuck LLM call cannot block the loop
//! forever. Individual tool calls within the agent turn are bounded by the agent
//! harness's own iteration cap.

use super::store;
use super::types::{SubconsciousStatus, TickResult};
use crate::openhuman::config::schema::SubconsciousMode;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER};
use crate::openhuman::memory_diff::types::CrossSourceDiff;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Hard timeout for a single subconscious tick (agent run).
const TICK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Per-tool-call timeout injected into the agent config.
const TOOL_CALL_TIMEOUT_SECS: u64 = 5 * 60;

/// Label stamped on the world-baseline checkpoint the tick re-creates each run.
const BASELINE_CHECKPOINT_LABEL: &str = "subconscious_tick";

/// Max changed items listed per source in the rendered world diff, to keep the
/// decision agent's prompt bounded when a source churns a lot.
const MAX_ITEMS_PER_SOURCE: usize = 10;

/// Tool catalogue handed to the `context_scout` so its `recommended_tool_calls`
/// stay grounded in tools the decision agent can actually call. Keep in sync
/// with `agent/agent.toml`'s `[tools].named` (actionable subset).
const SUBCONSCIOUS_TOOL_CATALOG: &str = "\
- notify_user: Send the user a proactive message about something important or time-sensitive.
- update_task: Add or update an actionable item on the user's global to-do board.
- goals_add: Record a new long-term goal that the changed world makes relevant.
- goals_edit: Revise an existing long-term goal.
- spawn_async_subagent: Delegate deeper research or multi-step work.
";

/// Actionable reason surfaced (via `SubconsciousStatus.provider_unavailable_reason`)
/// when a subconscious tick fails because the configured chat model has no
/// tool-use endpoint. The subconscious turn is inherently tool-bearing (it acts
/// through tools), so a tool-incapable model can never satisfy a tick — this
/// tells the user how to recover. See TAURI-RUST-ADC.
const TOOL_UNSUPPORTED_REASON: &str = "The selected chat model has no tool-use endpoint, so Subconscious can't run. Pick a tool-capable model in Settings > AI.";

/// Pick the `TrustedAutomationSource` variant for a subconscious tick.
///
/// Extracted from the engine's `run_agent` body so the origin-escalation
/// contract can be unit-tested without spinning up a real `Agent` + provider.
///
/// Contract: any tick that reacted to third-party sync changes (the memory_diff
/// surfaced added/modified/removed items, all of which originate from external
/// sources like Gmail / Slack / Notion / synced folders) must run with
/// `SubconsciousTainted` so the approval gate refuses external_effect tools.
/// A tick with no external changes keeps the legacy `Subconscious` origin.
pub(crate) fn tick_origin_source(
    has_external_content: bool,
) -> crate::openhuman::agent::turn_origin::TrustedAutomationSource {
    if has_external_content {
        crate::openhuman::agent::turn_origin::TrustedAutomationSource::SubconsciousTainted
    } else {
        crate::openhuman::agent::turn_origin::TrustedAutomationSource::Subconscious
    }
}

pub struct SubconsciousEngine {
    workspace_dir: PathBuf,
    mode: SubconsciousMode,
    interval_minutes: u32,
    context_budget_tokens: u32,
    enabled: bool,
    state: Mutex<EngineState>,
    tick_generation: AtomicU64,
    tick_lock: Mutex<()>,
}

struct EngineState {
    last_tick_at: f64,
    total_ticks: u64,
    consecutive_failures: u64,
    provider_unavailable_reason: Option<String>,
}

impl SubconsciousEngine {
    pub fn new(config: &crate::openhuman::config::Config) -> Self {
        Self::from_heartbeat_config(&config.heartbeat, config.workspace_dir.clone())
    }

    pub fn from_heartbeat_config(
        heartbeat: &crate::openhuman::config::HeartbeatConfig,
        workspace_dir: PathBuf,
    ) -> Self {
        let last_tick_at = match store::with_connection(&workspace_dir, store::get_last_tick_at) {
            Ok(v) => {
                if v > 0.0 {
                    info!("[subconscious] resumed last_tick_at={v} from disk");
                }
                v
            }
            Err(e) => {
                warn!("[subconscious] last_tick_at load failed, falling back to 0.0: {e}");
                0.0
            }
        };

        let mode = heartbeat.effective_subconscious_mode();

        Self {
            workspace_dir,
            mode,
            interval_minutes: mode.default_interval_minutes().max(5),
            context_budget_tokens: heartbeat.context_budget_tokens,
            enabled: mode.is_enabled(),
            state: Mutex::new(EngineState {
                last_tick_at,
                total_ticks: 0,
                consecutive_failures: 0,
                provider_unavailable_reason: None,
            }),
            tick_generation: AtomicU64::new(0),
            tick_lock: Mutex::new(()),
        }
    }

    pub async fn run(&self) -> Result<()> {
        if !self.enabled {
            info!("[subconscious] disabled, exiting");
            return Ok(());
        }

        let interval_secs = u64::from(self.interval_minutes) * 60;
        info!(
            "[subconscious] started: every {} minutes, budget {} tokens",
            self.interval_minutes, self.context_budget_tokens
        );

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
            match self.tick().await {
                Ok(result) => {
                    info!(
                        "[subconscious] tick: duration={}ms response_chars={}",
                        result.duration_ms, result.response_chars
                    );
                }
                Err(e) => {
                    warn!("[subconscious] tick error: {e}");
                }
            }
        }
    }

    pub async fn tick(&self) -> Result<TickResult> {
        let _tick_guard =
            match tokio::time::timeout(std::time::Duration::from_secs(5), self.tick_lock.lock())
                .await
            {
                Ok(guard) => guard,
                Err(_) => {
                    warn!("[subconscious] tick skipped — another tick is still running");
                    return Ok(TickResult {
                        tick_at: now_secs(),
                        duration_ms: 0,
                        response_chars: 0,
                    });
                }
            };

        match tokio::time::timeout(TICK_TIMEOUT, self.tick_inner()).await {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    "[subconscious] tick timed out after {}s",
                    TICK_TIMEOUT.as_secs()
                );
                let mut state = self.state.lock().await;
                state.consecutive_failures += 1;
                state.total_ticks += 1;
                Ok(TickResult {
                    tick_at: now_secs(),
                    duration_ms: TICK_TIMEOUT.as_millis() as u64,
                    response_chars: 0,
                })
            }
        }
    }

    async fn tick_inner(&self) -> Result<TickResult> {
        let started = std::time::Instant::now();
        let tick_at = now_secs();

        let my_generation = self.tick_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let config = match Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                warn!("[subconscious] config load failed: {e}");
                let mut state = self.state.lock().await;
                state.provider_unavailable_reason = Some(format!("Config unavailable: {e}"));
                state.consecutive_failures += 1;
                state.total_ticks += 1;
                return Ok(TickResult {
                    tick_at,
                    duration_ms: started.elapsed().as_millis() as u64,
                    response_chars: 0,
                });
            }
        };

        if let Some(reason) = subconscious_provider_unavailable_reason(&config) {
            info!("[subconscious] provider unavailable, skipping tick: {reason}");
            let mut state = self.state.lock().await;
            state.provider_unavailable_reason = Some(reason);
            state.consecutive_failures += 1;
            state.total_ticks += 1;
            return Ok(TickResult {
                tick_at,
                duration_ms: started.elapsed().as_millis() as u64,
                response_chars: 0,
            });
        }

        {
            let mut state = self.state.lock().await;
            state.provider_unavailable_reason = None;
        }

        // ── Stage 1: memory_diff — how did the agent's world change? ──────────
        let baseline =
            store::with_connection(&self.workspace_dir, store::get_baseline_checkpoint_id)
                .unwrap_or_else(|e| {
                    warn!("[subconscious] baseline load failed: {e}");
                    None
                });

        let diff: Option<CrossSourceDiff> = match &baseline {
            Some(checkpoint_id) => match crate::openhuman::memory_diff::ops::diff_since_checkpoint(
                checkpoint_id,
                &config,
                false,
            )
            .await
            {
                Ok(d) => Some(d),
                Err(e) => {
                    warn!("[subconscious] memory_diff failed (baseline={checkpoint_id}): {e}");
                    None
                }
            },
            None => {
                debug!("[subconscious] no world baseline yet — first tick establishes one");
                None
            }
        };

        let has_changes = diff
            .as_ref()
            .map(|d| world_diff_change_count(d) > 0)
            .unwrap_or(false);

        if !has_changes {
            // Quiet window, first tick, or a diff error: nothing to react to.
            // Refresh the baseline and return without spending a decision turn.
            info!("[subconscious] no world changes this tick — refreshing baseline, no agent run");
            self.refresh_baseline(&config).await;
            let mut state = self.state.lock().await;
            state.total_ticks += 1;
            if self.tick_generation.load(Ordering::SeqCst) == my_generation {
                state.consecutive_failures = 0;
                state.last_tick_at = tick_at;
                persist_last_tick_at(&self.workspace_dir, tick_at);
            }
            return Ok(TickResult {
                tick_at,
                duration_ms: started.elapsed().as_millis() as u64,
                response_chars: 0,
            });
        }

        let diff = diff.expect("has_changes implies diff is Some");
        let world_diff = render_world_diff(&diff);
        // Every change originates from an external source sync, so the decision
        // turn runs tainted: the approval gate refuses external_effect tools.
        let has_external_content = true;

        // ── Stage 2: prepare_context — ground the diff before deciding ───────
        let prepared_context = self.prepare_context(&world_diff).await;

        // ── Stage 3: decide — slim agent acts on diff + prepared context ─────
        let mut agent_prompt =
            String::with_capacity(world_diff.len() + prepared_context.len() + 256);
        agent_prompt.push_str("## What changed in your world since the last check\n\n");
        agent_prompt.push_str(&world_diff);
        agent_prompt.push_str("\n\n");
        if !prepared_context.is_empty() {
            agent_prompt.push_str("## Prepared context\n\n");
            agent_prompt.push_str(&prepared_context);
            agent_prompt.push_str("\n\n");
        }

        let agent_result = self
            .run_agent(&config, &agent_prompt, has_external_content)
            .await;
        let agent_failed = agent_result.is_err();
        let response_chars = *agent_result.as_ref().unwrap_or(&0);

        // Check if superseded by a newer tick.
        if self.tick_generation.load(Ordering::SeqCst) != my_generation {
            info!("[subconscious] tick superseded by newer tick, discarding");
            let mut state = self.state.lock().await;
            state.total_ticks += 1;
            return Ok(TickResult {
                tick_at,
                duration_ms: started.elapsed().as_millis() as u64,
                response_chars: 0,
            });
        }

        // Advance the world baseline only on a successful, current tick, so a
        // failed tick re-diffs the same window next time instead of losing it.
        if !agent_failed {
            self.refresh_baseline(&config).await;
        }

        let mut state = self.state.lock().await;
        state.total_ticks += 1;
        if agent_failed {
            state.consecutive_failures += 1;
            // Surface an actionable reason when the failure is a permanent
            // tool-capability error (TAURI-RUST-ADC): the subconscious turn is
            // inherently tool-bearing, so a tool-incapable chat model can never
            // satisfy it and the user must pick a tool-capable model.
            if let Err(e) = &agent_result {
                if is_tool_capability_error(e) {
                    info!(
                        "[subconscious] configured chat model has no tool-use endpoint — Subconscious can't run until the model changes (TAURI-RUST-ADC)"
                    );
                    state.provider_unavailable_reason = Some(TOOL_UNSUPPORTED_REASON.to_string());
                }
            }
        } else {
            state.consecutive_failures = 0;
            state.last_tick_at = tick_at;
            persist_last_tick_at(&self.workspace_dir, tick_at);
        }

        Ok(TickResult {
            tick_at,
            duration_ms: started.elapsed().as_millis() as u64,
            response_chars,
        })
    }

    /// Stage 2: run the read-only `context_scout` over the world diff to gather
    /// grounding context. Best-effort — on any error the decision agent simply
    /// runs without a prepared-context section.
    async fn prepare_context(&self, world_diff: &str) -> String {
        let question = format!(
            "Background awareness check. Here is what changed in the user's connected sources \
             since the last check:\n\n{world_diff}\n\nSurface what the user should be aware of or \
             act on, and the context that grounds a good decision.",
        );

        match crate::openhuman::agent_orchestration::tools::run_context_scout_with_catalog(
            &question,
            None,
            SUBCONSCIOUS_TOOL_CATALOG,
        )
        .await
        {
            Ok(result) if !result.is_error => {
                debug!(
                    "[subconscious] prepared context bundle ({} chars)",
                    result.output().chars().count()
                );
                result.output().to_string()
            }
            Ok(result) => {
                warn!(
                    "[subconscious] prepare_context returned an error result: {}",
                    result.output()
                );
                String::new()
            }
            Err(e) => {
                warn!("[subconscious] prepare_context failed: {e}");
                String::new()
            }
        }
    }

    /// Re-snapshot the world and persist the new checkpoint as the baseline the
    /// next tick diffs against. Best-effort — a failure leaves the old baseline
    /// in place (the next tick diffs against a slightly older window).
    async fn refresh_baseline(&self, config: &Config) {
        match crate::openhuman::memory_diff::ops::create_checkpoint(
            BASELINE_CHECKPOINT_LABEL,
            config,
        )
        .await
        {
            Ok(ckpt) => {
                if let Err(e) = store::with_connection(&self.workspace_dir, |conn| {
                    store::set_baseline_checkpoint_id(conn, &ckpt.id)
                }) {
                    warn!("[subconscious] failed to persist baseline checkpoint id: {e}");
                } else {
                    debug!("[subconscious] world baseline advanced to {}", ckpt.id);
                }
            }
            Err(e) => warn!("[subconscious] failed to create world baseline checkpoint: {e}"),
        }
    }

    pub async fn status(&self) -> SubconsciousStatus {
        let state = self.state.lock().await;

        SubconsciousStatus {
            enabled: self.enabled,
            mode: self.mode.as_str().to_string(),
            provider_available: state.provider_unavailable_reason.is_none(),
            provider_unavailable_reason: state.provider_unavailable_reason.clone(),
            interval_minutes: self.interval_minutes,
            last_tick_at: if state.last_tick_at > 0.0 {
                Some(state.last_tick_at)
            } else {
                None
            },
            total_ticks: state.total_ticks,
            consecutive_failures: state.consecutive_failures,
        }
    }

    /// Run the slim subconscious agent over `prompt_text` (diff + prepared
    /// context). The agent decides and acts through its tools. Returns
    /// `response_chars` on success, or `Err` on agent init/run failure.
    async fn run_agent(
        &self,
        config: &Config,
        prompt_text: &str,
        has_external_content: bool,
    ) -> Result<usize, String> {
        use crate::openhuman::agent::Agent;

        let mut effective = config.clone();
        effective.agent.agent_timeout_secs = TOOL_CALL_TIMEOUT_SECS;
        // Route the tick build through the `subconscious` background workload so
        // Settings → AI → Advanced "Subconscious" governs the cloud tick
        // provider, instead of riding the `chat` role. The session builder maps
        // `hint:subconscious` → the `subconscious` provider role; on the managed
        // backend the model still resolves to `chat-v1` (no regression).
        effective.default_model = Some("hint:subconscious".to_string());
        debug!(
            "[subconscious] tick provider routed via hint:subconscious (subconscious_provider={:?})",
            effective.subconscious_provider
        );

        // The decision agent must write internal continuity (global to-dos,
        // goals) and surface proactive messages — all app-internal writes, not
        // external effects. So it runs with Full autonomy; genuinely external
        // effects are still gated by the tainted origin + approval gate. Mode
        // only scales how much delegation depth the tick gets.
        effective.autonomy.level = crate::openhuman::security::AutonomyLevel::Full;
        match self.mode {
            SubconsciousMode::Simple => {
                effective.agent.max_tool_iterations = 15;
            }
            SubconsciousMode::Aggressive | SubconsciousMode::EventDriven => {
                effective.agent.max_tool_iterations = 30;
            }
            SubconsciousMode::Off => return Ok(0),
        }

        let mut agent = Agent::from_config(&effective).map_err(|e| {
            warn!("[subconscious] agent init failed: {e}");
            format!("agent init: {e}")
        })?;

        agent.set_event_context(
            format!("subconscious:tick:{}", now_secs() as u64),
            "subconscious",
        );

        let mode_guidance = match self.mode {
            SubconsciousMode::Aggressive | SubconsciousMode::EventDriven => {
                "\n\nYou may delegate deeper work with `spawn_async_subagent` (e.g. research \
                 or multi-step execution) when you spot something genuinely actionable."
            }
            _ => "",
        };

        let user_message = format!(
            "{prompt_text}\
             ## Your job\n\n\
             The diff above is how the user's world changed since the last check; the prepared \
             context grounds it. Decide what (if anything) deserves action:\n\
             - Record or update actionable follow-ups on the user's to-do board with `update_task` \
               (pass `threadId: \"user-tasks\"`).\n\
             - Evolve the user's long-term goals with `goals_add` / `goals_edit` when the world \
               shifts what matters to them.\n\
             - Surface anything time-sensitive or important with `notify_user`.\n\n\
             If nothing meaningful changed, do nothing — staying silent is the right call most \
             ticks. Do not invent busywork.{mode_guidance}",
        );

        debug!("[subconscious] spawning decision agent");
        let source = tick_origin_source(has_external_content);
        debug!(
            "[subconscious] tick origin source={:?} has_external_content={has_external_content}",
            source
        );
        let origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
            job_id: format!("subconscious:tick:{}", now_secs() as u64),
            source,
        };
        let response = crate::openhuman::agent::turn_origin::with_origin(
            origin,
            agent.run_single(&user_message),
        )
        .await
        .map_err(|e| {
            warn!("[subconscious] agent run failed: {e}");
            format!("agent run: {e}")
        })?;

        let response_chars = response.chars().count();
        info!(
            "[subconscious] decision agent completed (response {} chars)",
            response_chars
        );
        Ok(response_chars)
    }
}

// ── World-diff rendering ─────────────────────────────────────────────────────

/// Total added + modified + removed across all sources in a cross-source diff.
fn world_diff_change_count(diff: &CrossSourceDiff) -> u32 {
    diff.summary.added + diff.summary.modified + diff.summary.removed
}

/// Render a [`CrossSourceDiff`] into a compact markdown summary for the decision
/// agent's prompt. Per-source change lists are capped at [`MAX_ITEMS_PER_SOURCE`]
/// so a churny source can't blow out the context window.
fn render_world_diff(diff: &CrossSourceDiff) -> String {
    let s = &diff.summary;
    let total = s.added + s.modified + s.removed;
    if total == 0 {
        return "Nothing changed across your connected sources since the last check.".to_string();
    }

    let mut out = format!(
        "{total} item(s) changed across your sources since the last check \
         ({} added, {} modified, {} removed).\n",
        s.added, s.modified, s.removed
    );

    for source in &diff.per_source {
        let ss = &source.summary;
        if ss.added + ss.modified + ss.removed == 0 {
            continue;
        }
        out.push_str(&format!(
            "\n### {} ({})\n- {} added, {} modified, {} removed\n",
            source.source_label, source.source_kind, ss.added, ss.modified, ss.removed
        ));
        for change in source.changes.iter().take(MAX_ITEMS_PER_SOURCE) {
            let verb = match change.kind {
                crate::openhuman::memory_diff::types::ChangeKind::Added => "added",
                crate::openhuman::memory_diff::types::ChangeKind::Removed => "removed",
                crate::openhuman::memory_diff::types::ChangeKind::Modified => "modified",
            };
            let label = if change.title.trim().is_empty() {
                change.item_id.as_str()
            } else {
                change.title.as_str()
            };
            out.push_str(&format!("  - [{verb}] {label}\n"));
        }
        if source.changes.len() > MAX_ITEMS_PER_SOURCE {
            out.push_str(&format!(
                "  - …and {} more\n",
                source.changes.len() - MAX_ITEMS_PER_SOURCE
            ));
        }
    }
    out
}

// ── Provider routing ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
enum SubconsciousProviderRoute {
    LocalOllama { model: String },
    OpenHumanCloud,
    Other(String),
}

pub(crate) fn subconscious_provider_unavailable_reason(config: &Config) -> Option<String> {
    match resolve_subconscious_route(config) {
        SubconsciousProviderRoute::LocalOllama { .. } => None,
        SubconsciousProviderRoute::OpenHumanCloud => {
            if crate::openhuman::scheduler_gate::is_signed_out() {
                return Some(
                    "Sign in to use the OpenHuman cloud Subconscious provider.".to_string(),
                );
            }

            let state_dir = config
                .config_path
                .parent()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| config.workspace_dir.clone());
            let auth = AuthService::new(&state_dir, config.secrets.encrypt);
            match auth.get_provider_bearer_token(APP_SESSION_PROVIDER, None) {
                Ok(Some(token)) if !token.trim().is_empty() => None,
                Ok(_) => Some(
                    "Sign in or configure a local Subconscious provider in Settings > AI."
                        .to_string(),
                ),
                Err(e) => Some(format!("Unable to read the OpenHuman session: {e}")),
            }
        }
        SubconsciousProviderRoute::Other(_) => None,
    }
}

fn resolve_subconscious_route(config: &Config) -> SubconsciousProviderRoute {
    if let Some(model) = config.workload_local_model("subconscious") {
        return SubconsciousProviderRoute::LocalOllama { model };
    }

    let raw = config
        .subconscious_provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("cloud");
    let is_openhuman_cloud = raw.eq_ignore_ascii_case("cloud")
        || raw.eq_ignore_ascii_case("openhuman")
        || raw.to_ascii_lowercase().starts_with("openhuman:");
    if is_openhuman_cloud {
        SubconsciousProviderRoute::OpenHumanCloud
    } else {
        SubconsciousProviderRoute::Other(raw.to_string())
    }
}

/// True when an agent-run error means the configured chat model can't do tool
/// calls at all — a permanent, user-actionable condition (pick a tool-capable
/// model). Matches both the direct-provider body (`<model> does not support
/// tools`) and OpenRouter's router-level phrasing (`No endpoints found that
/// support tool use`, TAURI-RUST-ADC). Kept narrow to tool capability so an
/// unrelated provider error (auth, billing, rate-limit) is not misread as one.
fn is_tool_capability_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("no endpoints found that support tool use")
        || lower.contains("does not support tools")
}

fn persist_last_tick_at(workspace_dir: &std::path::Path, tick_at: f64) {
    if let Err(e) =
        store::with_connection(workspace_dir, |conn| store::set_last_tick_at(conn, tick_at))
    {
        warn!("[subconscious] failed to persist last_tick_at={tick_at}: {e}");
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
