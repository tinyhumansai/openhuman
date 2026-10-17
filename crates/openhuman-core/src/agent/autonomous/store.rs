use super::types::Work;
use once_cell::sync::Lazy;
use std::{path::Path, sync::Arc};
use tinyagents_harness::store::Store;
use tokio::sync::Mutex;

static LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
const NS: &str = "desktop-work";
fn db(root: &Path) -> Arc<dyn Store> {
    crate::agent::session_store::stores_or_ephemeral(root).kv
}
fn key(thread: &str) -> String {
    thread.bytes().map(|b| format!("{b:02x}")).collect()
}
pub async fn get(root: &Path, thread: &str) -> anyhow::Result<Option<Work>> {
    db(root)
        .get(NS, &key(thread))
        .await?
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}
pub async fn list(root: &Path) -> anyhow::Result<Vec<Work>> {
    let db = db(root);
    let mut rows = Vec::new();
    for key in db.list(NS).await? {
        if let Some(value) = db.get(NS, &key).await? {
            match serde_json::from_value(value) {
                Ok(work) => rows.push(work),
                Err(error) => tracing::warn!(%key, %error, "[autonomous] skipping unreadable work record"),
            }
        }
    }
    Ok(rows)
}
pub async fn start(
    root: &Path,
    thread: &str,
    objective: String,
    criteria: Vec<String>,
    budget: Option<u64>,
) -> anyhow::Result<Work> {
    let _lock = LOCK.lock().await;
    if let Some(old) = get(root, thread).await? {
        anyhow::ensure!(
            !old.runnable(),
            "An unfinished task already exists; use checkpoint or review"
        );
    }
    let goal = crate::agent::goals::store::set(root, thread, &objective, budget)
        .await
        .map_err(anyhow::Error::msg)?;
    let work = Work {
        id: uuid::Uuid::new_v4().to_string(),
        thread_id: thread.into(),
        goal_id: goal.goal_id,
        objective,
        acceptance_criteria: criteria,
        phase: super::types::Phase::Working,
        checkpoint: String::new(),
        evidence: vec![],
        finish_requested: false,
        lease: None,
        next_run_ms: 0,
        attempts: 0,
        failures: 0,
        stagnant_turns: 0,
        last_fingerprint: None,
        last_error: None,
        worker_reports: vec![],
        worker_revision: 0,
        plan: vec![],
        review_task: None,
    };
    db(root)
        .put(NS, &key(thread), serde_json::to_value(&work)?)
        .await?;
    super::service::allow(thread);
    Ok(work)
}
/// Serialize host goal writes with autonomous task transitions.
pub(crate) async fn lock_writes() -> tokio::sync::MutexGuard<'static, ()> {
    LOCK.lock().await
}

pub async fn settle(
    root: &Path,
    thread: &str,
    expected: &str,
    request: &str,
    f: impl FnOnce(&mut Work) -> anyhow::Result<()>,
) -> anyhow::Result<Work> {
    let _lock = LOCK.lock().await;
    let mut work = get(root, thread)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No autonomous task on this thread"))?;
    anyhow::ensure!(
        work.id == expected,
        "Task was replaced; stale update rejected"
    );
    if !work.runnable() || work.lease.as_deref() != Some(request) {
        return Ok(work);
    }
    let goal = crate::agent::goals::store::get(root, thread)
        .await
        .map_err(anyhow::Error::msg)?;
    if !goal.is_some_and(|g| g.goal_id == work.goal_id && g.status.is_active()) {
        work.phase = super::types::Phase::Paused;
        work.finish_requested = false;
        work.lease = None;
        work.last_error = Some("Goal stopped, replaced, or its token budget was reached".into());
    } else {
        f(&mut work)?;
    }
    db(root)
        .put(NS, &key(thread), serde_json::to_value(&work)?)
        .await?;
    if work.phase == super::types::Phase::Complete {
        crate::agent::goals::runtime::complete_for_thread(root, Some(thread)).await;
    }
    Ok(work)
}

pub async fn update(
    root: &Path,
    thread: &str,
    expected: &str,
    f: impl FnOnce(&mut Work) -> anyhow::Result<()>,
) -> anyhow::Result<Work> {
    let _lock = LOCK.lock().await;
    let mut work = get(root, thread)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No autonomous task on this thread"))?;
    anyhow::ensure!(
        work.id == expected,
        "Task was replaced; stale update rejected"
    );
    f(&mut work)?;
    db(root)
        .put(NS, &key(thread), serde_json::to_value(&work)?)
        .await?;
    Ok(work)
}
pub async fn pause(root: &Path, thread: &str, request: Option<&str>) -> anyhow::Result<()> {
    let _lock = LOCK.lock().await;
    if let Some(mut work) = get(root, thread).await? {
        if work.runnable() && (request.is_none() || work.lease.as_deref() == request) {
            work.phase = super::types::Phase::Paused;
            work.finish_requested = false;
            work.last_error = Some("Stopped by the user".into());
            db(root)
                .put(NS, &key(thread), serde_json::to_value(&work)?)
                .await?;
            crate::agent::goals::runtime::pause_for_thread(root, Some(thread)).await;
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

/// Persist detached worker claims for the next managed turn. Never let a
/// separate, uninterruptible delivery-model turn act on a managed task.
pub async fn worker_report(root: &Path, thread: &str, report: &str) -> anyhow::Result<bool> {
    let Some(work) = get(root, thread).await? else {
        return Ok(false);
    };
    update(root, thread, &work.id, |w| {
        w.worker_reports.push(report.chars().take(12000).collect());
        if w.worker_reports.len() > 10 {
            w.worker_reports.remove(0);
        }
        w.worker_revision = w.worker_revision.saturating_add(1);
        if w.runnable() {
            w.phase = super::types::Phase::Working;
            w.finish_requested = false;
        }
        Ok(())
    })
    .await?;
    Ok(true)
}
