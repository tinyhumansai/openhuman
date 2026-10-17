use super::{
    observation::{self, Observed},
    store,
    types::{Phase, Work},
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
static SUPPRESSED: once_cell::sync::Lazy<std::sync::Mutex<std::collections::HashSet<String>>> =
    once_cell::sync::Lazy::new(Default::default);
pub fn suppress(thread: &str) {
    SUPPRESSED.lock().unwrap().insert(thread.into());
}
pub fn allow(thread: &str) {
    SUPPRESSED.lock().unwrap().remove(thread);
}
pub fn suppressed(thread: &str) -> bool {
    SUPPRESSED.lock().unwrap().contains(thread)
}
static STARTED: AtomicBool = AtomicBool::new(false);
pub fn now_ms() -> u64 {
    tinyagents_harness::ids::now_ms()
}

/// Desktop-only pump. State is read from the current user's workspace on every tick.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
        async {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(error) = tick().await {
                    tracing::warn!(error = %error, "[autonomous] scheduling tick failed");
                }
            }
        },
    ));
}
async fn tick() -> anyhow::Result<()> {
    let config = crate::config::rpc::load_config_with_timeout()
        .await
        .map_err(anyhow::Error::msg)?;
    let mut tasks = store::list(&config.workspace_dir).await?;
    tasks.retain(|w| !suppressed(&w.thread_id) && w.runnable() && w.next_run_ms <= now_ms());
    if tasks.is_empty() {
        return Ok(());
    }
    tasks.sort_by_key(|w| w.next_run_ms);
    let threads = crate::threads::store::blocking::list_threads(crate::core::runtime::DispatchCtx::ambient(), config.workspace_dir.clone())
        .await
        .map_err(anyhow::Error::msg)?;
    for work in tasks {
        if !threads.iter().any(|t| t.id == work.thread_id) {
            store::pause(&config.workspace_dir, &work.thread_id, None).await?;
            continue;
        }
        let goal = crate::agent::goals::store::get(&config.workspace_dir, &work.thread_id)
            .await
            .map_err(anyhow::Error::msg)?;
        if !goal
            .as_ref()
            .is_some_and(|g| g.goal_id == work.goal_id && g.status.is_active())
        {
            store::update(&config.workspace_dir, &work.thread_id, &work.id, |w| {
                w.phase = Phase::Paused;
                w.last_error =
                    Some("Goal stopped, replaced, or its token budget was reached".into());
                Ok(())
            })
            .await?;
            continue;
        }
        crate::web_chat::start_autonomous_turn(config.workspace_dir.clone(), work).await?;
    }
    Ok(())
}

/// Apply a finished slice only to the generation and lease that actually ran.
pub async fn settle(
    root: &Path,
    job: &Work,
    request: &str,
    observed: &Observed,
    error: Option<(String, bool, Option<u64>)>,
) -> anyhow::Result<Work> {
    let result = store::settle(root, &job.thread_id, &job.id, request, |work| {
        if work.lease.as_deref() != Some(request) || !work.runnable() { return Ok(()); }
        work.lease = None;
        work.attempts = work.attempts.saturating_add(1);
        if let Some((message, retryable, retry_after)) = error {
            work.finish_requested = false;
            work.failures = work.failures.saturating_add(1);
            let protocol_failure = message.contains("tool_call_recovery_exhausted");
            work.last_error = Some(message);
            if protocol_failure && work.failures >= 3 {
                work.phase = Phase::Blocked;
                work.last_error = Some("The model repeatedly failed the tool protocol after automatic repair. No completion is claimed. Select a model capable of the offered tool protocol, then resume.".into());
            } else if retryable {
                let delay = 30_000u64.saturating_mul(1u64 << work.failures.min(6)).min(1_800_000);
                work.next_run_ms = now_ms().saturating_add(delay.max(retry_after.unwrap_or(0)));
            } else { work.phase = Phase::Blocked; }
            return Ok(());
        }
        if job.phase == Phase::Reviewing && work.phase == Phase::Reviewing && work.finish_requested {
            if observation::verified(observed) && work.evidence.len() == super::plan::criteria(work)?.len()
                && work.review_task == job.review_task
                && work.worker_revision == job.worker_revision
                && !crate::agent::orchestration::running_subagents::has_running_for_thread(&job.thread_id)
                && !crate::agent::orchestration::background_completions::has_pending_for_thread(&job.thread_id) {
                super::plan::accept_review(work)?;
                work.failures = 0;
                work.stagnant_turns = 0;
                work.next_run_ms = now_ms().saturating_add(10_000);
                return Ok(());
            }
            work.phase = Phase::Working;
            work.finish_requested = false;
            work.last_error = Some("Review did not produce enough successful verification activity, or a tool failed. Repair and request another review.".into());
        }
        let stagnant = observed.successful == 0 || work.last_fingerprint == Some(observed.fingerprint);
        work.stagnant_turns = if stagnant { work.stagnant_turns.saturating_add(1) } else { 0 };
        work.last_fingerprint = Some(observed.fingerprint);
        if work.stagnant_turns >= 3 {
            work.phase = Phase::Blocked;
            work.last_error = Some("Three continuation turns made no new observable progress. Inspect the transcript and provide the missing input or revise the approach before resuming.".into());
        }
        work.failures = 0;
        work.next_run_ms = now_ms().saturating_add(10_000);
        Ok(())
    }).await?;
    tracing::info!(thread = %job.thread_id, phase = ?result.phase, attempts = result.attempts, "[autonomous] slice settled");
    Ok(result)
}
#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
