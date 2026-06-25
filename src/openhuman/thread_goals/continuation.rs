//! Heartbeat-driven autonomous continuation of idle thread goals (Codex's
//! `MaybeContinueIfIdle`).
//!
//! When a thread carries an **active** goal and goes idle — no in-flight turn
//! and no activity for `goal_idle_minutes` — the heartbeat injects **one**
//! continuation turn that keeps working the objective. The guards mirror Codex:
//!
//! - **Opt-in.** Off unless `heartbeat.goal_continuation_enabled` is set — an
//!   autonomous turn spends budget with no user present, so it defaults off
//!   (matching the project's stance on background automation).
//! - **One-shot per idle period.** After a continuation fires, the goal is
//!   marked `continuation_suppressed`; a later user-initiated turn clears it
//!   (see [`super::runtime::account_turn_against_goal`]), so the next idle
//!   period can fire again. Prevents a tight self-driving loop.
//! - **Serialised.** A process-wide `Semaphore(1)` ensures at most one
//!   continuation runs at a time.
//! - **No in-flight turn.** Skipped while the thread has a live turn.
//! - **Per-tick cap.** Bounds how many goals continue in a single tick.
//!
//! The continuation turn runs the orchestrator on the thread (resuming its
//! transcript) under a `TrustedAutomation { GoalContinuation }` origin, so the
//! approval gate parks irreversible external actions (no present user to
//! authorize) while read/compute work proceeds.

use std::path::Path;
use std::sync::OnceLock;

use tokio::sync::Semaphore;

use super::store;
use super::types::{ThreadGoal, ThreadGoalStatus};
use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::thread_context::with_thread_id;
use crate::openhuman::threads::turn_state::{TurnLifecycle, TurnStateStore};

/// Serialise continuation dispatches so at most one autonomous goal turn runs at
/// a time (Codex's `Semaphore(1)` guard).
fn continuation_gate() -> &'static Semaphore {
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    GATE.get_or_init(|| Semaphore::new(1))
}

/// Max continuation turns dispatched per heartbeat tick (bounds spend).
const MAX_CONTINUATIONS_PER_TICK: usize = 2;

/// Current unix time in milliseconds.
fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// One heartbeat pass: find idle active goals and continue them. No-op unless
/// `heartbeat.goal_continuation_enabled`.
pub async fn run_continuation_tick(config: &Config) {
    if !config.heartbeat.goal_continuation_enabled {
        return;
    }
    let workspace_dir = config.workspace_dir.clone();
    let idle_ms = u64::from(config.heartbeat.goal_idle_minutes.max(1)) * 60_000;
    let now = now_ms();

    let goals = match store::list_all(&workspace_dir).await {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!("[thread_goals] continuation tick: list_all failed: {e}");
            return;
        }
    };

    let mut candidates: Vec<ThreadGoal> = goals
        .into_iter()
        .filter(|g| g.status == ThreadGoalStatus::Active)
        .filter(|g| !g.continuation_suppressed)
        .filter(|g| now.saturating_sub(g.updated_at_ms) >= idle_ms)
        .collect();
    // Oldest-idle first so the most-neglected goal advances under the per-tick cap.
    candidates.sort_by_key(|g| g.updated_at_ms);
    candidates.truncate(MAX_CONTINUATIONS_PER_TICK);

    if candidates.is_empty() {
        tracing::debug!("[thread_goals] continuation tick: no idle active goals");
        return;
    }
    tracing::info!(
        "[thread_goals] continuation tick: {} idle candidate(s) (idle_min={})",
        candidates.len(),
        config.heartbeat.goal_idle_minutes.max(1)
    );

    for goal in candidates {
        if has_in_flight_turn(&workspace_dir, &goal.thread_id) {
            tracing::debug!(
                thread_id = %goal.thread_id,
                "[thread_goals] continuation skip: turn in flight"
            );
            continue;
        }

        // One continuation at a time. If the gate is busy, another continuation
        // is mid-flight — defer the rest to the next tick.
        let permit = match continuation_gate().try_acquire() {
            Ok(p) => p,
            Err(_) => {
                tracing::debug!("[thread_goals] continuation gate busy; deferring to next tick");
                return;
            }
        };

        let ran = dispatch_continuation(config, &goal).await;

        // One-shot: suppress further continuations on this goal until a
        // user-initiated turn clears the flag. Only when the turn actually ran,
        // and only if THIS goal is still current (compare-and-set on goal_id) —
        // a goal completed or replaced during the turn must not be suppressed.
        if ran {
            if let Err(e) = store::set_continuation_suppressed_if(
                &workspace_dir,
                &goal.thread_id,
                &goal.goal_id,
                true,
            )
            .await
            {
                tracing::debug!(
                    thread_id = %goal.thread_id,
                    error = %e,
                    "[thread_goals] failed to set continuation_suppressed"
                );
            }
        }
        drop(permit);
    }
}

/// Whether `thread_id` currently has a live (started/streaming) turn.
fn has_in_flight_turn(workspace_dir: &Path, thread_id: &str) -> bool {
    match TurnStateStore::new(workspace_dir.to_path_buf()).get(thread_id) {
        Ok(Some(ts)) => matches!(
            ts.lifecycle,
            TurnLifecycle::Started | TurnLifecycle::Streaming
        ),
        _ => false,
    }
}

/// The continuation message handed to the orchestrator.
fn continuation_prompt(objective: &str) -> String {
    format!(
        "[goal continuation] You are resuming autonomous work toward this thread's goal — \
         no user is currently present.\n\nGoal: {objective}\n\n\
         Assess progress against concrete evidence, then take the next useful step. \
         If the goal is already satisfied, call `goal_complete`. If you are blocked or \
         need the user (e.g. an irreversible external action, missing input), stop and \
         summarise the blocker and the next step rather than guessing — external actions \
         are not auto-approved while you run unattended."
    )
}

/// Build and run a single continuation turn for `goal`. Best-effort: failures
/// are logged, never propagated (the heartbeat must keep ticking).
///
/// Returns `true` when a turn was actually attempted (agent built + run_single
/// invoked), `false` when the agent couldn't even be built — the caller only
/// suppresses further continuations when a turn actually ran.
async fn dispatch_continuation(config: &Config, goal: &ThreadGoal) -> bool {
    let thread_id = goal.thread_id.clone();
    tracing::info!(
        thread_id = %thread_id,
        goal_id = %goal.goal_id,
        "[thread_goals] dispatching continuation turn"
    );

    let mut agent = match Agent::from_config_for_agent(config, "orchestrator") {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(
                thread_id = %thread_id,
                error = %e,
                "[thread_goals] continuation: failed to build orchestrator agent"
            );
            return false;
        }
    };
    // Tag events so subscribers can correlate goal-continuation turns and filter
    // them from user-driven flows.
    agent.set_event_context(format!("goal:{thread_id}"), "goal_continuation");

    let prompt = continuation_prompt(&goal.objective);
    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: format!("goal:{thread_id}"),
        source: TrustedAutomationSource::GoalContinuation,
    };

    // Scope the ambient thread id (so the goal tools + per-turn injection target
    // this thread) and the trusted-automation origin (so the approval gate parks
    // unattended external actions). `run_single` resumes the thread transcript.
    let result = with_thread_id(
        thread_id.clone(),
        with_origin(origin, agent.run_single(&prompt)),
    )
    .await;

    match result {
        Ok(text) => tracing::info!(
            thread_id = %thread_id,
            response_chars = text.chars().count(),
            "[thread_goals] continuation turn complete"
        ),
        Err(e) => tracing::warn!(
            thread_id = %thread_id,
            error = %e,
            "[thread_goals] continuation turn failed"
        ),
    }
    // A turn was attempted (built + run) regardless of Ok/Err — suppress so we
    // don't re-fire it every tick until the user re-engages.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(enabled: bool, idle_minutes: u32, workspace: &Path) -> Config {
        let mut config = Config::default();
        config.workspace_dir = workspace.to_path_buf();
        config.heartbeat.goal_continuation_enabled = enabled;
        config.heartbeat.goal_idle_minutes = idle_minutes;
        config
    }

    #[tokio::test]
    async fn tick_is_noop_when_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        // Active, long-idle goal present — but the feature is off.
        store::set(tmp.path(), "t", "obj", None).await.unwrap();
        let config = config_with(false, 1, tmp.path());
        // Must not panic / must return promptly; nothing to assert beyond no-op.
        run_continuation_tick(&config).await;
        // Goal untouched (not suppressed) since the feature is disabled.
        let g = store::get(tmp.path(), "t").await.unwrap().unwrap();
        assert!(!g.continuation_suppressed);
    }

    #[tokio::test]
    async fn candidate_filter_respects_status_idle_and_suppression() {
        // This exercises the selection predicate without dispatching a turn by
        // keeping the feature enabled but pointing at a fresh (non-idle) goal.
        let tmp = tempfile::tempdir().unwrap();
        // Fresh goal: updated_at is "now", so with a 60-min idle window it is
        // NOT a candidate and the tick stays a no-op (no agent build attempted).
        store::set(tmp.path(), "fresh", "obj", None).await.unwrap();
        let config = config_with(true, 60, tmp.path());
        run_continuation_tick(&config).await;
        let g = store::get(tmp.path(), "fresh").await.unwrap().unwrap();
        assert!(
            !g.continuation_suppressed,
            "fresh goal must not be continued/suppressed"
        );
    }

    #[test]
    fn continuation_prompt_names_objective_and_guards() {
        let p = continuation_prompt("ship the release");
        assert!(p.contains("ship the release"));
        assert!(p.contains("goal_complete"));
        assert!(p.contains("not auto-approved"));
    }
}
