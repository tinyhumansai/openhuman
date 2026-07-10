//! The `memory` subconscious world — OpenHuman's internal high-level world of
//! the user's connected memory sources (Gmail / Slack / Notion / folders).
//!
//! Extracted verbatim from the old monolithic engine (stages 1–3):
//!   1. **observe** — diff the connected sources against the world baseline
//!      (`memory_diff::diff_since_checkpoint`) to see how the user's world changed.
//!   2. **prepare_context** — run the read-only `context_scout` over that diff to
//!      gather grounding context.
//!   3. **reflect** — hand `diff + context` to the slim decision agent, which
//!      records to-dos (`update_task`), evolves goals (`goals_*`), notifies the
//!      user (`notify_user`), or delegates (`spawn_async_subagent`).
//!
//! The generic [`SubconsciousInstance`] runner owns the scheduler + circuit
//! breaker; this profile owns only what is memory-specific.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, info, warn};

use super::super::instance::SubconsciousInstance;
use super::super::profile::{Observation, Reflection, SubconsciousProfile};
use super::super::store;
use crate::openhuman::agent::turn_origin::TrustedAutomationSource;
use crate::openhuman::agent_orchestration::parent_context::with_root_parent;
use crate::openhuman::config::schema::SubconsciousMode;
use crate::openhuman::config::Config;
use crate::openhuman::memory_diff::types::CrossSourceDiff;

/// Per-tool-call timeout injected into the decision agent config.
const TOOL_CALL_TIMEOUT_SECS: u64 = 5 * 60;

/// Label stamped on the world-baseline checkpoint `commit` re-creates each tick.
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

/// Construct the live `memory` instance from config (used by the registry /
/// bootstrap). The only place `MemoryProfile` is wired into a runner.
pub fn memory_instance(config: &Config) -> SubconsciousInstance {
    let mode = config.heartbeat.effective_subconscious_mode();
    SubconsciousInstance::new(
        Arc::new(MemoryProfile::new(config)),
        config.workspace_dir.clone(),
        mode.is_enabled(),
        mode.default_interval_minutes().max(5),
        mode.as_str(),
    )
}

/// The `memory` world profile. Holds only the subconscious mode (drives cadence
/// and the decision agent's delegation depth); everything else it reads live
/// from the per-tick `Config`.
pub struct MemoryProfile {
    mode: SubconsciousMode,
}

impl MemoryProfile {
    pub fn new(config: &Config) -> Self {
        Self {
            mode: config.heartbeat.effective_subconscious_mode(),
        }
    }

    /// Stage 2: run the read-only `context_scout` over the world diff to gather
    /// grounding context. Best-effort — on any error the decision agent simply
    /// runs without a prepared-context section.
    ///
    /// The tick is a controller-spawned background surface with **no enclosing
    /// agent turn**, so the scout spawn has no ambient `current_parent()`. We
    /// establish a root parent via [`with_root_parent`] — without it every
    /// tick's scout died with `NoParentContext` (Sentry TAURI-RUST-HMW; #4337).
    async fn run_scout(&self, config: &Config, world_diff: &str) -> String {
        let question = format!(
            "Background awareness check. Here is what changed in the user's connected sources \
             since the last check:\n\n{world_diff}\n\nSurface what the user should be aware of or \
             act on, and the context that grounds a good decision.",
        );

        // Flatten: outer Err = root-parent build failure, inner = scout result.
        let scout = with_root_parent(config, "subconscious", "subconscious", "subconscious", {
            crate::openhuman::agent_orchestration::tools::run_context_scout_with_catalog(
                &question,
                None,
                SUBCONSCIOUS_TOOL_CATALOG,
            )
        })
        .await
        .and_then(|inner| inner);

        match scout {
            Ok(result) if !result.is_error => {
                debug!(
                    "[subconscious:memory] prepared context bundle ({} chars)",
                    result.output().chars().count()
                );
                result.output().to_string()
            }
            Ok(result) => {
                warn!(
                    "[subconscious:memory] prepare_context returned an error result: {}",
                    result.output()
                );
                String::new()
            }
            Err(e) => {
                warn!("[subconscious:memory] prepare_context failed: {e}");
                String::new()
            }
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
        // Connections → API keys → LLM "Subconscious" governs the cloud tick
        // provider, instead of riding the `chat` role.
        effective.default_model = Some("hint:subconscious".to_string());
        debug!(
            "[subconscious:memory] tick provider routed via hint:subconscious (subconscious_provider={:?})",
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
            warn!("[subconscious:memory] agent init failed: {e}");
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

        debug!("[subconscious:memory] spawning decision agent");
        let source = tick_origin_source(has_external_content);
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
            warn!("[subconscious:memory] agent run failed: {e}");
            format!("agent run: {e}")
        })?;

        let response_chars = response.chars().count();
        info!(
            "[subconscious:memory] decision agent completed (response {} chars)",
            response_chars
        );
        Ok(response_chars)
    }
}

#[async_trait]
impl SubconsciousProfile for MemoryProfile {
    fn id(&self) -> &'static str {
        "memory"
    }

    fn cadence(&self, _config: &Config) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.mode.default_interval_minutes().max(5)) * 60)
    }

    async fn observe(&self, config: &Config) -> Observation {
        // ── Stage 1: memory_diff — how did the agent's world change? ──────────
        // (The tiny.place orchestration review is now its own `tinyplace`
        // instance, ticked independently by the heartbeat fan-out — it no longer
        // piggybacks here.)
        let baseline = store::with_connection(&config.workspace_dir, |conn| {
            store::get_baseline_checkpoint_id(conn, "memory")
        })
        .unwrap_or_else(|e| {
            warn!("[subconscious:memory] baseline load failed: {e}");
            None
        });

        let diff: Option<CrossSourceDiff> = match &baseline {
            Some(checkpoint_id) => match crate::openhuman::memory_diff::ops::diff_since_checkpoint(
                checkpoint_id,
                config,
                false,
            )
            .await
            {
                Ok(d) => Some(d),
                Err(e) => {
                    warn!(
                        "[subconscious:memory] memory_diff failed (baseline={checkpoint_id}): {e}"
                    );
                    None
                }
            },
            None => {
                debug!("[subconscious:memory] no world baseline yet — first tick establishes one");
                None
            }
        };

        let has_changes = diff
            .as_ref()
            .map(|d| world_diff_change_count(d) > 0)
            .unwrap_or(false);

        if !has_changes {
            // Quiet window, first tick, or a diff error: nothing to react to.
            // The runner routes straight to commit, which refreshes the baseline.
            info!("[subconscious:memory] no world changes this tick — refreshing baseline, no agent run");
            return Observation::default();
        }

        let diff = diff.expect("has_changes implies diff is Some");
        Observation {
            rendered: render_world_diff(&diff),
            has_changes: true,
            // Every change originates from an external source sync, so the
            // decision turn runs tainted: the approval gate refuses
            // external_effect tools.
            has_external_content: true,
            commit_token: None,
        }
    }

    async fn prepare_context(&self, config: &Config, obs: &Observation) -> String {
        self.run_scout(config, &obs.rendered).await
    }

    async fn reflect(
        &self,
        config: &Config,
        obs: &Observation,
        prepared_context: &str,
    ) -> Result<Reflection, String> {
        let mut agent_prompt =
            String::with_capacity(obs.rendered.len() + prepared_context.len() + 256);
        agent_prompt.push_str("## What changed in your world since the last check\n\n");
        agent_prompt.push_str(&obs.rendered);
        agent_prompt.push_str("\n\n");
        if !prepared_context.is_empty() {
            agent_prompt.push_str("## Prepared context\n\n");
            agent_prompt.push_str(prepared_context);
            agent_prompt.push_str("\n\n");
        }

        let response_chars = self
            .run_agent(config, &agent_prompt, obs.has_external_content)
            .await?;
        Ok(Reflection::Acted { response_chars })
    }

    async fn commit(&self, config: &Config, _obs: &Observation) {
        // Re-snapshot the world and persist the new checkpoint as the baseline
        // the next tick diffs against. Best-effort — a failure leaves the old
        // baseline in place (the next tick diffs against a slightly older window).
        match crate::openhuman::memory_diff::ops::create_checkpoint(
            BASELINE_CHECKPOINT_LABEL,
            config,
        )
        .await
        {
            Ok(ckpt) => {
                if let Err(e) = store::with_connection(&config.workspace_dir, |conn| {
                    store::set_baseline_checkpoint_id(conn, "memory", &ckpt.id)
                }) {
                    warn!("[subconscious:memory] failed to persist baseline checkpoint id: {e}");
                } else {
                    debug!(
                        "[subconscious:memory] world baseline advanced to {}",
                        ckpt.id
                    );
                }
            }
            Err(e) => {
                warn!("[subconscious:memory] failed to create world baseline checkpoint: {e}")
            }
        }
    }

    fn origin(&self, obs: &Observation) -> TrustedAutomationSource {
        tick_origin_source(obs.has_external_content)
    }
}

// ── World-diff rendering ─────────────────────────────────────────────────────

/// Pick the `TrustedAutomationSource` variant for a memory tick.
///
/// Contract: any tick that reacted to third-party sync changes (added/modified/
/// removed items, all originating from external sources like Gmail / Slack /
/// Notion / synced folders) must run with `SubconsciousTainted` so the approval
/// gate refuses external_effect tools. A tick with no external changes keeps the
/// legacy `Subconscious` origin.
pub(crate) fn tick_origin_source(has_external_content: bool) -> TrustedAutomationSource {
    if has_external_content {
        TrustedAutomationSource::SubconsciousTainted
    } else {
        TrustedAutomationSource::Subconscious
    }
}

/// Total added + modified + removed across all sources in a cross-source diff.
pub(crate) fn world_diff_change_count(diff: &CrossSourceDiff) -> u32 {
    diff.summary.added + diff.summary.modified + diff.summary.removed
}

/// Render a [`CrossSourceDiff`] into a compact markdown summary for the decision
/// agent's prompt. Per-source change lists are capped at [`MAX_ITEMS_PER_SOURCE`]
/// so a churny source can't blow out the context window.
pub(crate) fn render_world_diff(diff: &CrossSourceDiff) -> String {
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

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
